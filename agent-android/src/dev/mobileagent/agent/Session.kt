package dev.mobileagent.agent

import android.net.LocalSocket
import android.os.SystemClock
import dev.mobileagent.agent.codec.Call
import dev.mobileagent.agent.codec.CborReader
import dev.mobileagent.agent.codec.CborWriter
import dev.mobileagent.agent.codec.Codec
import dev.mobileagent.agent.codec.CodecException
import dev.mobileagent.agent.codec.Event
import dev.mobileagent.agent.codec.EventPayload
import dev.mobileagent.agent.codec.FrameTooLargeException
import dev.mobileagent.agent.codec.Frames
import dev.mobileagent.agent.codec.Hello
import dev.mobileagent.agent.codec.Kind
import dev.mobileagent.agent.codec.Message
import dev.mobileagent.agent.codec.Ping
import dev.mobileagent.agent.codec.Pong
import dev.mobileagent.agent.codec.Protocol
import dev.mobileagent.agent.codec.Reply
import dev.mobileagent.agent.codec.Request
import dev.mobileagent.agent.codec.Response
import dev.mobileagent.agent.codec.ResyncAck
import dev.mobileagent.agent.codec.UnknownCall
import dev.mobileagent.agent.codec.UnknownKindException
import dev.mobileagent.agent.codec.Welcome
import dev.mobileagent.agent.codec.WireError
import java.io.IOException
import java.io.InputStream
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit

/** A request the agent works on, answered once through [Session.respond]. */
class Work(
    @JvmField val session: Session,
    @JvmField val id: Int,
    @JvmField val method: Int,
    @JvmField val call: Call,
    /** Relative deadline in ms as the request carried it, or null. */
    @JvmField val deadlineMs: Int?,
    /** [SystemClock.uptimeMillis] when the request arrived. */
    @JvmField val arrived: Long,
) {
    /** Set once the answer no longer matters: the deadline passed or the session closed. */
    @Volatile @JvmField var cancelled = false
    @JvmField var deadlineTask: ScheduledFuture<*>? = null

    /** Uptime ms by which the work must finish, or [Long.MAX_VALUE]. */
    val deadline: Long
        get() = deadlineMs?.let { arrived + (it.toLong() and 0xFFFFFFFFL) } ?: Long.MAX_VALUE
}

/** What the agent does with a request that passed the session's checks. It must not block. */
fun interface RequestHandler {
    fun handle(work: Work)
}

interface SessionListener {
    /** The host accepted the hello. */
    fun onReady(session: Session)

    /** The session closed; `refusal` is the host's error when it refused the hello. */
    fun onClosed(session: Session, reason: String, refusal: WireError?)
}

/**
 * The agent's end of one connection, the counterpart of crates/protocol's
 * AgentSession: it sends the hello first, accepts the welcome, checks each
 * request's id, method and capability, answers requests whose deadline
 * passed, numbers events per stream, answers pings, pings a host silent for
 * [PING_INTERVAL_MS] and closes after [PEER_TIMEOUT_MS] of silence.
 *
 * One thread reads; responses and events are written whole under one lock,
 * so frames never interleave.
 */
class Session(
    private val socket: LocalSocket,
    private val capabilities: Int,
    private val token: String?,
    private val timers: ScheduledExecutorService,
    private val handler: RequestHandler,
    private val listener: SessionListener,
) {
    companion object {
        const val PING_INTERVAL_MS = 1000L
        const val PEER_TIMEOUT_MS = 3000L
        private const val TICK_MS = 100L
        /** Payload bytes of an oversized frame read to name its request. */
        private const val PEEK_LEN = 32
        private const val AWAITING_WELCOME = 0
        private const val READY = 1
        private const val CLOSED = 2
        private const val EOF = "the other side closed the connection"

        private fun now() = SystemClock.uptimeMillis()

        private fun unsigned(v: Int) = v.toLong() and 0xFFFFFFFFL

        /** Monotonic time in microseconds for event stamps: uptime, the clock of event times. */
        fun nowUs(): Long = SystemClock.uptimeMillis() * 1000
    }

    private val output = socket.outputStream
    private val lock = Any()
    @Volatile private var state = AWAITING_WELCOME
    /** The frame limit in both directions: this side's maximum until the welcome, then the negotiated one. */
    private var limit = unsigned(Protocol.DEFAULT_MAX_FRAME)
    private val inFlight = HashMap<Int, Work>()
    private val seqs = LongArray(8)
    private val eventWriter = CborWriter(1024)
    @Volatile private var lastHeard = now()
    private var lastPing = 0L
    private var nonce = 0L
    private var tick: ScheduledFuture<*>? = null

    val isReady: Boolean get() = state == READY
    val isClosed: Boolean get() = state == CLOSED

    /** Sends the hello and starts reading. */
    fun start() {
        val hello = Hello(Protocol.VERSIONS, capabilities, Protocol.DEFAULT_MAX_FRAME, Main.AGENT, token)
        val out = CborWriter(256)
        Frames.write(hello, -1, out)
        if (!write(out)) return
        tick = timers.scheduleAtFixedRate({ tick() }, TICK_MS, TICK_MS, TimeUnit.MILLISECONDS)
        Thread({ readLoop() }, "session-reader").apply {
            isDaemon = true
            start()
        }
    }

    /** The last sequence number used on `stream`; 0 before the first event. */
    fun lastSeq(stream: Int): Long = synchronized(lock) { seqs[stream] }

    /**
     * Sends an event with the stream's next sequence number and returns the
     * number, or 0 when the session is not ready. An event over the frame
     * limit uses its number up, so the host sees the gap and resynchronizes.
     */
    fun emit(payload: EventPayload, timeUs: Long = nowUs()): Long {
        synchronized(lock) {
            if (state != READY) return 0
            val seq = ++seqs[payload.stream]
            eventWriter.reset()
            try {
                Frames.write(Event(seq, timeUs, payload), limit.toInt(), eventWriter)
            } catch (e: FrameTooLargeException) {
                return seq
            }
            writeLocked(eventWriter)
            return seq
        }
    }

    /** Answers `work` with a result or an error; false when it was answered already. */
    fun respond(work: Work, reply: Reply?, error: WireError?): Boolean {
        synchronized(lock) {
            if (inFlight[work.id] !== work) return false
            inFlight.remove(work.id)
        }
        work.deadlineTask?.cancel(false)
        sendResponse(work.id, work.method, reply, error)
        return true
    }

    /**
     * Answers a resync of `stream` with the last sequence number sent on it,
     * read and written under the lock events take, so no event slips
     * between the number and the answer.
     */
    fun respondResync(work: Work, stream: Int) {
        synchronized(lock) {
            if (inFlight[work.id] !== work) return
            inFlight.remove(work.id)
            val seq = if (stream in seqs.indices) seqs[stream] else 0
            val out = CborWriter(64)
            Frames.write(Response(work.id, work.method, ResyncAck(stream, seq), null), limit.toInt(), out)
            if (state != CLOSED) writeLocked(out)
        }
        work.deadlineTask?.cancel(false)
    }

    fun close(reason: String, refusal: WireError? = null) {
        val pending: List<Work>
        synchronized(lock) {
            if (state == CLOSED) return
            state = CLOSED
            pending = inFlight.values.toList()
            inFlight.clear()
        }
        tick?.cancel(false)
        for (w in pending) {
            w.cancelled = true
            w.deadlineTask?.cancel(false)
        }
        try {
            socket.shutdownInput()
        } catch (e: IOException) {
        }
        try {
            socket.shutdownOutput()
        } catch (e: IOException) {
        }
        try {
            socket.close()
        } catch (e: IOException) {
        }
        listener.onClosed(this, reason, refusal)
    }

    private fun readLoop() {
        val input = socket.inputStream
        val header = ByteArray(Frames.HEADER_LEN)
        try {
            while (state != CLOSED) {
                if (!readFully(input, header, header.size)) return close(EOF)
                val len = Frames.payloadLength(header, 0)
                val max = synchronized(lock) { limit }
                if (len > max) {
                    val peek = ByteArray(minOf(len, PEEK_LEN.toLong()).toInt())
                    if (!readFully(input, peek, peek.size)) return close(EOF)
                    lastHeard = now()
                    oversized(len, max, peek)
                    // The rest is dropped as it arrives and is no sign of life.
                    if (!skipFully(input, len - peek.size)) return close(EOF)
                    continue
                }
                val payload = ByteArray(len.toInt())
                if (!readFully(input, payload, payload.size)) return close(EOF)
                lastHeard = now()
                val msg = try {
                    Codec.decode(payload, 0, payload.size)
                } catch (e: UnknownKindException) {
                    continue
                } catch (e: CodecException) {
                    return close("protocol error: invalid message: ${e.message}")
                }
                handle(msg)
            }
        } catch (e: IOException) {
            close(EOF)
        }
    }

    private fun handle(msg: Message) {
        when (state) {
            AWAITING_WELCOME ->
                if (msg is Welcome) {
                    acceptWelcome(msg)
                } else {
                    close("protocol error: expected a welcome from the host, received a ${Errors.kindName(msg.kind)}")
                }
            READY -> when (msg) {
                is Request -> acceptRequest(msg)
                is Ping -> {
                    val out = CborWriter(32)
                    Frames.write(Pong(msg.nonce), -1, out)
                    write(out)
                }
                is Pong -> {}
                else -> close(
                    "protocol error: expected a request, a ping or a pong from the host, received a ${Errors.kindName(msg.kind)}",
                )
            }
        }
    }

    private fun acceptWelcome(w: Welcome) {
        val error = w.error
        if (error != null) return close("handshake failed: the daemon refused the connection: ${error.code}: ${error.message}", error)
        val version = w.version ?: return close("protocol error: a welcome with neither a version nor an error")
        if (Protocol.VERSIONS.none { it == version }) {
            return close("handshake failed: the daemon selected protocol version $version, which the agent does not speak")
        }
        val maxFrame = minOf(unsigned(w.maxFrame), unsigned(Protocol.DEFAULT_MAX_FRAME))
        if (maxFrame < Protocol.MIN_MAX_FRAME) {
            return close("handshake failed: the negotiated maximum frame size of $maxFrame bytes is below the protocol minimum")
        }
        synchronized(lock) {
            if (state != AWAITING_WELCOME) return
            limit = maxFrame
            state = READY
        }
        listener.onReady(this)
    }

    private fun acceptRequest(r: Request) {
        val method = r.call.method
        val work = Work(this, r.id, method, r.call, r.deadlineMs, now())
        synchronized(lock) {
            if (inFlight.containsKey(r.id)) {
                return close("protocol error: request id ${unsigned(r.id)} reused while in flight")
            }
        }
        if (r.call is UnknownCall) return sendResponse(r.id, method, null, Errors.unknownMethod(method))
        val needed = Errors.requiredCapability(method)
        if (capabilities and needed != needed) {
            return sendResponse(r.id, method, null, Errors.capabilityMissing(method, needed and capabilities.inv()))
        }
        synchronized(lock) {
            if (state != READY) return
            inFlight[r.id] = work
        }
        if (r.deadlineMs != null) {
            work.deadlineTask = timers.schedule({ expire(work) }, unsigned(r.deadlineMs), TimeUnit.MILLISECONDS)
        }
        try {
            handler.handle(work)
        } catch (t: Throwable) {
            respond(work, null, Errors.internal(method, t))
        }
    }

    /** The deadline of `work` passed before it was answered. */
    private fun expire(work: Work) {
        synchronized(lock) {
            if (inFlight[work.id] !== work) return
            inFlight.remove(work.id)
        }
        work.cancelled = true
        sendResponse(work.id, work.method, null, Errors.deadlineExceeded(work.method, work.id, work.deadlineMs ?: 0))
    }

    /** Answers a request the host sent over the limit, whose payload is being dropped. */
    private fun oversized(size: Long, max: Long, peek: ByteArray) {
        var kind = -1
        var id: Int? = null
        var method: Int? = null
        try {
            val r = CborReader(peek, 0, peek.size)
            if (r.peekMajor() == 5) {
                val entries = r.map()
                for (i in 0 until minOf(entries, 3)) {
                    when (r.key()) {
                        0L -> kind = r.uint8()
                        1L -> id = r.uint32()
                        2L -> method = r.uint16()
                        else -> r.skip()
                    }
                }
            }
        } catch (e: CodecException) {
            // Whatever the first bytes named is all there is to go on.
        }
        val requestId = id
        val requestMethod = method
        if (kind == Kind.REQUEST && requestId != null && requestMethod != null) {
            sendResponse(requestId, requestMethod, null, Errors.frameTooLarge(requestMethod, requestId, size, max))
        }
    }

    /** Writes a response, or a frame_too_large error in its place when it exceeds the limit. */
    private fun sendResponse(id: Int, method: Int, reply: Reply?, error: WireError?) {
        val max = synchronized(lock) { limit }
        val out = CborWriter(if (reply == null) 256 else 16 * 1024)
        try {
            Frames.write(Response(id, method, reply, error), max.toInt(), out)
        } catch (e: FrameTooLargeException) {
            out.reset()
            Frames.write(Response(id, method, null, Errors.frameTooLarge(method, id, e.size, max)), max.toInt(), out)
        }
        write(out)
    }

    private fun tick() {
        if (state == CLOSED) return
        val now = now()
        val silence = now - lastHeard
        if (silence >= PEER_TIMEOUT_MS) {
            close(String.format(java.util.Locale.ROOT, "nothing arrived from the other side for %.1f s", silence / 1000.0))
            return
        }
        if (state != READY || silence < PING_INTERVAL_MS) return
        val since = if (lastPing > lastHeard) lastPing else lastHeard
        if (now < since + PING_INTERVAL_MS) return
        lastPing = now
        val out = CborWriter(32)
        Frames.write(Ping(nonce++), -1, out)
        write(out)
    }

    private fun write(out: CborWriter): Boolean {
        synchronized(lock) {
            if (state == CLOSED) return false
            return writeLocked(out)
        }
    }

    private fun writeLocked(out: CborWriter): Boolean {
        return try {
            output.write(out.buf, 0, out.pos)
            true
        } catch (e: IOException) {
            // Closing runs the listener, which may write; do it off this lock's holder.
            timers.execute { close(EOF) }
            false
        }
    }

    private fun readFully(input: InputStream, buf: ByteArray, len: Int): Boolean {
        var off = 0
        while (off < len) {
            val n = input.read(buf, off, len - off)
            if (n < 0) return false
            off += n
        }
        return true
    }

    private fun skipFully(input: InputStream, count: Long): Boolean {
        val buf = ByteArray(64 * 1024)
        var left = count
        while (left > 0) {
            val n = input.read(buf, 0, minOf(left, buf.size.toLong()).toInt())
            if (n < 0) return false
            left -= n
        }
        return true
    }
}
