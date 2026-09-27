package dev.mobileagent.agent

import android.net.LocalServerSocket
import android.net.LocalSocket
import dev.mobileagent.agent.codec.Capability
import dev.mobileagent.agent.codec.EmptyReply
import dev.mobileagent.agent.codec.ErrorCode
import dev.mobileagent.agent.codec.ImageFormat
import dev.mobileagent.agent.codec.InputAck
import dev.mobileagent.agent.codec.KeyCall
import dev.mobileagent.agent.codec.Lifecycle
import dev.mobileagent.agent.codec.LifecycleKind
import dev.mobileagent.agent.codec.Protocol
import dev.mobileagent.agent.codec.ResyncCall
import dev.mobileagent.agent.codec.ScreenshotCall
import dev.mobileagent.agent.codec.ShutdownCall
import dev.mobileagent.agent.codec.SnapshotCall
import dev.mobileagent.agent.codec.Stream
import dev.mobileagent.agent.codec.SwipeCall
import dev.mobileagent.agent.codec.TapCall
import dev.mobileagent.agent.codec.TextAck
import dev.mobileagent.agent.codec.TextMethod
import dev.mobileagent.agent.codec.TextSetCall
import dev.mobileagent.agent.codec.WireError
import java.io.IOException
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.ThreadFactory
import kotlin.system.exitProcess

/**
 * The resident agent: listens on the abstract socket named at launch,
 * accepts connections from the shell user only (the uid adbd runs as, design
 * D3), and serves one host session at a time; a new connection replaces the
 * previous one, so a restarted daemon can reuse the running agent.
 */
class Server(private val options: Options) : RequestHandler, SessionListener {
    companion object {
        /** The uid adbd and `adb shell` run as. */
        const val SHELL_UID = 2000

        /** The name `ps` lists the agent under, with its socket name after it. */
        const val PROCESS_NAME = "mobile-agent"

        /** Exit status when another process holds UI automation. */
        const val EXIT_CONFLICT = 3

        /** How long a refused attach retries, so a short-lived holder such as `uiautomator dump` passes. */
        private const val CONFLICT_WAIT_MS = 2_000L

        /** How long the role may take to come free after --force stopped its holder. */
        private const val RELEASE_WAIT_MS = 3_000L

        fun daemonThreads(name: String) = ThreadFactory { r -> Thread(r, name).apply { isDaemon = true } }

        /** Renames the process as `ps` shows it; a hidden call, skipped where absent. */
        fun setProcessName(name: String) {
            try {
                android.os.Process::class.java.getDeclaredMethod("setArgV0", String::class.java).invoke(null, name)
            } catch (e: ReflectiveOperationException) {
                Out.line("warning cannot rename the process: $e")
            }
        }
    }

    private val timers: ScheduledExecutorService = Executors.newSingleThreadScheduledExecutor(daemonThreads("timers"))
    /** What this device lacks of the hidden members the agent uses. */
    private val warnings = ArrayList<String>()
    /** Input injection; null where the hidden injection method is missing. */
    private val input: Input? = try {
        Input()
    } catch (e: ReflectiveOperationException) {
        warnings.add("input injection is unavailable: ${e.javaClass.simpleName}: ${e.message}")
        null
    }
    private val capabilities = Capability.TREE or Capability.EVENTS or Capability.TEXT_SET or
        Capability.SCREENSHOT or (if (input != null) Capability.INPUT else 0)
    /** Screenshots run one at a time, beside tree reads and input. */
    private val captureWorker = Executors.newSingleThreadExecutor(daemonThreads("capture"))
    private lateinit var screenshots: Screenshots
    /** Input runs one action at a time, beside tree reads. */
    private val inputWorker = Executors.newSingleThreadExecutor(daemonThreads("input"))
    private lateinit var textSetter: TextSetter
    /** Tree reads run one at a time, in arrival order. */
    private val treeWorker = Executors.newSingleThreadExecutor(daemonThreads("tree"))
    private lateinit var reader: TreeReader
    private lateinit var events: EventHub
    private lateinit var compose: ComposeTracker
    /** A Compose re-read is queued on the tree worker. */
    private val pollQueued = java.util.concurrent.atomic.AtomicBoolean(false)
    @Volatile private var nextPoll = 0L
    /** When the agent last had no host: its start, then each disconnect. */
    @Volatile private var hostlessSince = android.os.SystemClock.uptimeMillis()
    /** The last tree read, whose node ids requests may name. */
    @Volatile private var lastRead: TreeReader.Result? = null
    @Volatile private var current: Session? = null
    @Volatile private var automation: Automation? = null
    /** The holder --force stopped, as `name:pid`. */
    private var forced: Holders.Holder? = null

    fun run(): Int {
        setProcessName("$PROCESS_NAME ${options.socket}")
        val a = takeOwnership() ?: return EXIT_CONFLICT
        automation = a
        reader = TreeReader(a)
        textSetter = TextSetter(a, input)
        screenshots = Screenshots(a)
        events = EventHub(a)
        events.sink = { payload, timeUs -> current?.emit(payload, timeUs) }
        compose = ComposeTracker(a, reader, { payload -> current?.emit(payload) }, options.trace)
        a.enableInteractiveWindows()
        events.start()
        if (options.idleExitMs > 0) {
            timers.scheduleWithFixedDelay({ exitWhenIdle() }, 100, 100, java.util.concurrent.TimeUnit.MILLISECONDS)
        }
        if (options.composePollMs > 0) {
            timers.scheduleWithFixedDelay({ pollCompose() }, options.composePollMs, 100, java.util.concurrent.TimeUnit.MILLISECONDS)
        }
        val windowWait = a.awaitActiveWindow(options.windowWaitMs)
        val windows = a.windows()
        val active = windows.firstOrNull { it.isActive }?.root?.packageName ?: "-"
        val server = try {
            LocalServerSocket(options.socket)
        } catch (e: IOException) {
            System.err.println("cannot listen on @${options.socket}: ${e.message}")
            a.disconnect()
            return 1
        }
        Out.line(
            "ready socket=${options.socket} pid=${android.os.Process.myPid()} uid=${android.os.Process.myUid()} " +
                "protocol=${Protocol.VERSION} capabilities=0x${Integer.toHexString(capabilities)} ownership=held " +
                "connect_ms=${a.connectMs} window_wait_ms=$windowWait windows=${windows.size} active=$active " +
                "forced=${forced?.let { "${it.name}:${it.pid}" } ?: "-"}",
        )
        while (true) {
            val socket = try {
                server.accept()
            } catch (e: IOException) {
                System.err.println("accept on @${options.socket} failed: ${e.message}")
                return 1
            }
            val peer = try {
                socket.peerCredentials
            } catch (e: IOException) {
                null
            }
            if (peer == null || (peer.uid != SHELL_UID && peer.uid != android.os.Process.myUid())) {
                Out.line("refused peer uid=${peer?.uid ?: -1} pid=${peer?.pid ?: -1}")
                closeQuietly(socket)
                continue
            }
            val session = Session(socket, capabilities, options.token, timers, this, this)
            val previous = synchronized(this) {
                val p = current
                current = session
                p
            }
            previous?.close("another connection replaced this one")
            session.start()
        }
    }

    override fun handle(work: Work) {
        when (val call = work.call) {
            is SnapshotCall -> treeWorker.execute { snapshot(work, call) }
            is ResyncCall -> treeWorker.execute { resync(work, call) }
            is TapCall -> inputWorker.execute {
                act(work) { input!!.tap(call.x, call.y, call.durationMs?.let { unsigned(it) } ?: 0L) { work.cancelled } }
            }
            is SwipeCall -> inputWorker.execute {
                act(work) { input!!.swipe(call.x1, call.y1, call.x2, call.y2, unsigned(call.durationMs)) { work.cancelled } }
            }
            is KeyCall -> inputWorker.execute { act(work) { input!!.key(call.code, call.meta ?: 0) } }
            is TextSetCall -> inputWorker.execute { setText(work, call) }
            is ScreenshotCall -> captureWorker.execute { screenshot(work, call) }
            ShutdownCall -> shutdown(work)
            else -> work.session.respond(work, null, Errors.unknownMethod(work.method))
        }
    }

    /**
     * Takes UI automation, retrying a refusal for [CONFLICT_WAIT_MS]; with
     * --force, stops each known holder in turn until the role comes free.
     * Reports a conflict and returns null when it cannot.
     */
    private fun takeOwnership(): Automation? {
        val until = android.os.SystemClock.uptimeMillis() + CONFLICT_WAIT_MS
        while (true) {
            try {
                return Automation.connect()
            } catch (e: OwnershipConflict) {
                if (!options.force && android.os.SystemClock.uptimeMillis() < until) {
                    Thread.sleep(100)
                    continue
                }
            }
            val candidates = Holders.find(android.os.Process.myPid(), options.socket)
            if (options.force) {
                for (holder in candidates.filter { it.known }) {
                    if (!Holders.terminate(holder)) continue
                    val released = android.os.SystemClock.uptimeMillis() + RELEASE_WAIT_MS
                    while (android.os.SystemClock.uptimeMillis() < released) {
                        try {
                            val a = Automation.connect()
                            forced = holder
                            return a
                        } catch (e: OwnershipConflict) {
                            Thread.sleep(50)
                        }
                    }
                }
            }
            val likely = candidates.firstOrNull()
            Out.line(
                "conflict pid=${android.os.Process.myPid()} holder=${likely?.name ?: "-"} " +
                    "holder_pid=${likely?.pid?.takeIf { it > 0 } ?: "-"} message=${Holders.message(candidates, options.force)}",
            )
            return null
        }
    }

    override fun onReady(session: Session) {
        val role = forced?.let { "took UI automation over from ${it.describe()}" } ?: "UI automation held"
        session.emit(Lifecycle(LifecycleKind.READY, "${Main.AGENT} serving @${options.socket}; $role"))
        // A hidden member this device does not have costs a capability rather
        // than the attach (design, Risks).
        for (warning in warnings) session.emit(Lifecycle(LifecycleKind.WARNING, warning))
    }

    override fun onClosed(session: Session, reason: String, refusal: WireError?) {
        synchronized(this) {
            if (current === session) {
                current = null
                hostlessSince = android.os.SystemClock.uptimeMillis()
            }
        }
        if (refusal?.code == ErrorCode.VERSION_MISMATCH) {
            // A host that needs another protocol version replaces this agent.
            Out.line("exiting reason=version_mismatch message=${refusal.message}")
            exit(0)
        }
    }

    private fun snapshot(work: Work, call: SnapshotCall) {
        if (work.cancelled) return
        val maxDepth = call.maxDepth?.let { unsigned(it) } ?: Long.MAX_VALUE
        val maxNodes = call.maxNodes?.let { unsigned(it) } ?: TreeReader.DEFAULT_MAX_NODES.toLong()
        val bound = call.idleMs?.let { unsigned(it) } ?: EventHub.DEFAULT_IDLE_MS
        try {
            val idle = events.waitIdle(bound) { work.cancelled }
            // After an injected action the Compose windows' cache is stale.
            if (compose.takeDirty()) for (id in compose.composeWindowsOnScreen()) automation?.evictWindow(id)
            val seq = work.session.lastSeq(Stream.TREE)
            val result = reader.read(
                minOf(maxDepth, Int.MAX_VALUE.toLong()).toInt(),
                minOf(maxNodes, Int.MAX_VALUE.toLong()).toInt(),
                seq,
            ) { work.cancelled }
            lastRead = result
            compose.observe(result.snapshot)
            work.session.respond(work, result.snapshot, null)
            if (options.trace) {
                val s = result.snapshot
                Out.line(
                    "trace snapshot id=${unsigned(work.id)} windows=${s.windows.size} nodes=${s.nodeCount} " +
                        "visible=${result.visible} serialized=${result.nodes.size} truncated=0x${Integer.toHexString(s.truncated)} " +
                        "idle_ms=$idle read_ms=${result.readMs} round_trips=${result.roundTrips}",
                )
            }
        } catch (e: Cancelled) {
            // The deadline was answered already.
        } catch (t: Throwable) {
            work.session.respond(work, null, Errors.internal(work.method, t))
        }
    }

    private fun unsigned(v: Int) = v.toLong() and 0xFFFFFFFFL

    private fun screenshot(work: Work, call: ScreenshotCall) {
        if (work.cancelled) return
        val format = call.format ?: ImageFormat.JPEG
        val quality = call.quality ?: Screenshots.DEFAULT_QUALITY
        if (format != ImageFormat.PNG && format != ImageFormat.JPEG) {
            work.session.respond(work, null, Errors.invalidParams("image format $format is neither 0 (png) nor 1 (jpeg)"))
            return
        }
        if (quality !in 1..100) {
            work.session.respond(work, null, Errors.invalidParams("quality $quality is not between 1 and 100"))
            return
        }
        val t0 = android.os.SystemClock.uptimeMillis()
        try {
            val max = call.maxDimension?.let { unsigned(it) }?.let { minOf(it, Int.MAX_VALUE.toLong()).toInt() }
            val shot = screenshots.capture(format, quality, max)
            work.session.respond(work, shot, null)
            if (options.trace) {
                Out.line(
                    "trace screenshot width=${shot.width} height=${shot.height} rotation=${shot.rotation} " +
                        "bytes=${shot.data.size} ms=${android.os.SystemClock.uptimeMillis() - t0}",
                )
            }
        } catch (t: Throwable) {
            work.session.respond(work, null, Errors.internal(work.method, t))
        }
    }

    /** Sets text directly or types it, answering with the method used. */
    private fun setText(work: Work, call: TextSetCall) {
        if (work.cancelled) return
        val t0 = android.os.SystemClock.uptimeMillis()
        try {
            val node = call.node?.let { id ->
                lastRead?.nodes?.getOrNull(id)
                    ?: throw TextRefused("node ${unsigned(id)} is not in the last snapshot; read the tree again")
            }
            val method = textSetter.set(call.text, node, call.mode ?: TextMethod.AUTO)
            events.noteAction()
            compose.markDirty()
            nextPoll = minOf(nextPoll, android.os.SystemClock.uptimeMillis() + EventHub.QUIET_MS)
            work.session.respond(work, TextAck(method), null)
            if (options.trace) {
                val name = if (method == TextMethod.DIRECT) "direct" else "keys"
                Out.line("trace text method=$name chars=${call.text.length} ms=${android.os.SystemClock.uptimeMillis() - t0}")
            }
        } catch (e: TextRefused) {
            work.session.respond(work, null, Errors.invalidParams(e.message ?: "text refused"))
        } catch (t: Throwable) {
            work.session.respond(work, null, Errors.internal(work.method, t))
        }
    }

    /**
     * Runs an injection and answers with the platform's verdict. Afterwards
     * the idle wait counts from the action, the next read evicts the Compose
     * windows' cache, and the Compose poll comes forward to one quiet window
     * from now, since those windows will not report what the action changed.
     */
    private fun act(work: Work, action: () -> Boolean) {
        if (work.cancelled) return
        val t0 = android.os.SystemClock.uptimeMillis()
        try {
            val accepted = action()
            events.noteAction()
            compose.markDirty()
            nextPoll = minOf(nextPoll, android.os.SystemClock.uptimeMillis() + EventHub.QUIET_MS)
            work.session.respond(work, InputAck(accepted), null)
            if (options.trace) {
                Out.line("trace input method=${Errors.methodName(work.method)} accepted=$accepted inject_ms=${android.os.SystemClock.uptimeMillis() - t0}")
            }
        } catch (e: Cancelled) {
            events.noteAction()
            compose.markDirty()
        } catch (t: Throwable) {
            work.session.respond(work, null, Errors.internal(work.method, t))
        }
    }

    /**
     * Resynchronizes a stream: for the tree stream the node cache is emptied
     * first, so the host's next read is a full one, as after a sequence gap
     * the host cannot trust what it holds.
     */
    private fun resync(work: Work, call: ResyncCall) {
        if (call.stream == Stream.TREE) automation?.clearCache()
        work.session.respondResync(work, call.stream)
    }

    /**
     * Queues a re-read of the windows hosting Compose while a host is
     * connected, every --compose-poll-ms and never more often than four times
     * the last re-read took, so a wide screen does not keep its UI thread busy.
     */
    private fun pollCompose() {
        val now = android.os.SystemClock.uptimeMillis()
        if (now < nextPoll || current?.isReady != true || !pollQueued.compareAndSet(false, true)) return
        treeWorker.execute {
            try {
                compose.reread()
            } catch (t: Throwable) {
                if (options.trace) Out.line("trace compose error=${t.javaClass.simpleName}:${t.message}")
            } finally {
                nextPoll = android.os.SystemClock.uptimeMillis() + maxOf(options.composePollMs, 4 * compose.lastRereadMs)
                pollQueued.set(false)
            }
        }
    }

    /**
     * Exits once no host has been connected for --idle-exit-ms, so a daemon
     * that went away without stopping the agent leaves nothing running,
     * while one that reconnects within the bound reuses it.
     */
    private fun exitWhenIdle() {
        if (current != null) return
        if (android.os.SystemClock.uptimeMillis() - hostlessSince < options.idleExitMs) return
        Out.line("idle no host for ${options.idleExitMs} ms")
        exit(0)
    }

    private fun shutdown(work: Work) {
        val session = work.session
        session.emit(Lifecycle(LifecycleKind.SHUTTING_DOWN, "the host asked the agent to stop"))
        session.respond(work, EmptyReply, null)
        session.close("the agent is shutting down")
        exit(0)
    }

    @Synchronized
    private fun exit(code: Int) {
        automation?.disconnect()
        automation = null
        Out.line("exiting code=$code")
        System.err.flush()
        exitProcess(code)
    }

    private fun closeQuietly(socket: LocalSocket) {
        try {
            socket.close()
        } catch (e: IOException) {
        }
    }
}
