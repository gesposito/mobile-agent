package dev.mobileagent.agent

import android.net.LocalServerSocket
import android.net.LocalSocket
import dev.mobileagent.agent.codec.Capability
import dev.mobileagent.agent.codec.EmptyReply
import dev.mobileagent.agent.codec.ErrorCode
import dev.mobileagent.agent.codec.Lifecycle
import dev.mobileagent.agent.codec.LifecycleKind
import dev.mobileagent.agent.codec.Protocol
import dev.mobileagent.agent.codec.ShutdownCall
import dev.mobileagent.agent.codec.SnapshotCall
import dev.mobileagent.agent.codec.Stream
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
    private val capabilities = Capability.TREE
    /** Tree reads run one at a time, in arrival order. */
    private val treeWorker = Executors.newSingleThreadExecutor(daemonThreads("tree"))
    private lateinit var reader: TreeReader
    /** The last tree read, whose node ids requests may name. */
    @Volatile private var lastRead: TreeReader.Result? = null
    @Volatile private var current: Session? = null
    private var automation: Automation? = null
    /** The holder --force stopped, as `name:pid`. */
    private var forced: Holders.Holder? = null

    fun run(): Int {
        setProcessName("$PROCESS_NAME ${options.socket}")
        val a = takeOwnership() ?: return EXIT_CONFLICT
        automation = a
        reader = TreeReader(a)
        a.enableInteractiveWindows()
        val windowWait = a.awaitActiveWindow(options.windowWaitMs)
        val windows = a.uia.windows
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
    }

    override fun onClosed(session: Session, reason: String, refusal: WireError?) {
        synchronized(this) {
            if (current === session) current = null
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
        try {
            val seq = work.session.lastSeq(Stream.TREE)
            val result = reader.read(
                minOf(maxDepth, Int.MAX_VALUE.toLong()).toInt(),
                minOf(maxNodes, Int.MAX_VALUE.toLong()).toInt(),
                seq,
            ) { work.cancelled }
            lastRead = result
            work.session.respond(work, result.snapshot, null)
            if (options.trace) {
                val s = result.snapshot
                Out.line(
                    "trace snapshot id=${unsigned(work.id)} windows=${s.windows.size} nodes=${s.nodeCount} " +
                        "visible=${result.visible} serialized=${result.nodes.size} truncated=0x${Integer.toHexString(s.truncated)} " +
                        "read_ms=${result.readMs} round_trips=${result.roundTrips}",
                )
            }
        } catch (e: Cancelled) {
            // The deadline was answered already.
        } catch (t: Throwable) {
            work.session.respond(work, null, Errors.internal(work.method, t))
        }
    }

    private fun unsigned(v: Int) = v.toLong() and 0xFFFFFFFFL

    private fun shutdown(work: Work) {
        val session = work.session
        session.emit(Lifecycle(LifecycleKind.SHUTTING_DOWN, "the host asked the agent to stop"))
        session.respond(work, EmptyReply, null)
        session.close("the agent is shutting down")
        exit(0)
    }

    private fun exit(code: Int) {
        automation?.disconnect()
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
