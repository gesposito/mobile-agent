package dev.mobileagent.agent

import android.net.LocalServerSocket
import android.net.LocalSocket
import dev.mobileagent.agent.codec.EmptyReply
import dev.mobileagent.agent.codec.ErrorCode
import dev.mobileagent.agent.codec.Lifecycle
import dev.mobileagent.agent.codec.LifecycleKind
import dev.mobileagent.agent.codec.Protocol
import dev.mobileagent.agent.codec.ShutdownCall
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
    private val capabilities = 0
    @Volatile private var current: Session? = null

    fun run(): Int {
        setProcessName("$PROCESS_NAME ${options.socket}")
        val server = try {
            LocalServerSocket(options.socket)
        } catch (e: IOException) {
            System.err.println("cannot listen on @${options.socket}: ${e.message}")
            return 1
        }
        Out.line(
            "ready socket=${options.socket} pid=${android.os.Process.myPid()} uid=${android.os.Process.myUid()} " +
                "protocol=${Protocol.VERSION} capabilities=0x${Integer.toHexString(capabilities)}",
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
        when (work.call) {
            ShutdownCall -> shutdown(work)
            else -> work.session.respond(work, null, Errors.unknownMethod(work.method))
        }
    }

    override fun onReady(session: Session) {
        session.emit(Lifecycle(LifecycleKind.READY, "${Main.AGENT} serving @${options.socket}"))
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

    private fun shutdown(work: Work) {
        val session = work.session
        session.emit(Lifecycle(LifecycleKind.SHUTTING_DOWN, "the host asked the agent to stop"))
        session.respond(work, EmptyReply, null)
        session.close("the agent is shutting down")
        exit(0)
    }

    private fun exit(code: Int) {
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
