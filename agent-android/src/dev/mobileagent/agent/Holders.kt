package dev.mobileagent.agent

/**
 * Infers which process holds UI automation when the platform refuses the
 * role. The refusal names only the holder's binder proxy (spike S1), so the
 * candidates come from the process list, where `app_process` programs show
 * the names they gave themselves, and the activity manager's instrumentation
 * list.
 */
object Holders {
    /** A process that may hold the role. */
    class Holder(
        /** Short name without spaces: `uiautomator`, `mobile-agent`, `am-instrument`, ... */
        @JvmField val name: String,
        @JvmField val pid: Int,
        @JvmField val args: String,
        /** A known automation client, which --force may stop. */
        @JvmField val known: Boolean,
        /** For an app process or an instrumentation, the package `am force-stop` stops. */
        @JvmField val pkg: String?,
    ) {
        fun describe(): String = if (pid > 0) "$name (pid $pid: $args)" else "$name ($args)"
    }

    /** App packages of automation tools whose instrumentation or server holds the role. */
    private val KNOWN_PACKAGES = listOf(
        "io.appium.uiautomator2" to "appium-uiautomator2",
        "com.wetest.uia2" to "openatx-u2",
        "com.github.uiautomator" to "openatx-uiautomator",
        "dev.mobile.maestro" to "maestro",
        "com.callstack.agentdevice" to "agent-device",
    )

    /** Candidates, most likely first, leaving out this agent and its launcher. */
    fun find(selfPid: Int, socket: String): List<Holder> {
        val known = ArrayList<Holder>()
        val possible = ArrayList<Holder>()
        for (line in run("ps", "-A", "-o", "PID,PPID,USER,ARGS").lines().drop(1)) {
            val cols = line.trim().split(Regex("\\s+"), limit = 4)
            if (cols.size < 4) continue
            val pid = cols[0].toIntOrNull() ?: continue
            val user = cols[2]
            val args = cols[3]
            if (pid == selfPid || args.contains(socket)) continue
            val first = args.substringBefore(' ')
            val holder = when {
                first == "uiautomator" -> Holder("uiautomator", pid, args, true, null)
                first == Server.PROCESS_NAME -> Holder(Server.PROCESS_NAME, pid, args, true, null)
                args.contains("com.android.commands.am.Am") && args.contains(" instrument") ->
                    Holder("am-instrument", pid, args, true, null)
                else -> KNOWN_PACKAGES.firstOrNull { args.contains(it.first) }?.let { (pkg, name) ->
                    Holder(name, pid, args, true, if (user == "shell" || user == "root") null else first)
                } ?: if (first.endsWith("app_process") || first.endsWith("app_process64") || first.endsWith("app_process32")) {
                    Holder("app_process", pid, args, false, null)
                } else {
                    null
                }
            }
            if (holder == null) continue
            if (holder.known) known.add(holder) else possible.add(holder)
        }
        for (component in instrumentations()) {
            known.add(Holder("instrumentation", -1, component, true, component.substringBefore('/')))
        }
        return known + possible
    }

    /** Components of running instrumentations, from `dumpsys activity processes`. */
    private fun instrumentations(): List<String> {
        val pattern = Regex("ActiveInstrumentation\\{\\S+ \\{([^}]+)\\}")
        return pattern.findAll(run("dumpsys", "activity", "processes"))
            .map { it.groupValues[1] }
            .filter { !it.contains(" FINISHED") }
            .distinct()
            .toList()
    }

    /** The message of a conflict report. */
    fun message(candidates: List<Holder>, forced: Boolean): String {
        val likely = candidates.firstOrNull()
        val who = when {
            likely == null -> "a process that neither the process list nor the instrumentation list names"
            likely.known -> "another process, likely ${likely.describe()}"
            else -> "another process, possibly ${likely.describe()}"
        }
        val others = candidates.drop(1).take(2)
        val also = if (others.isEmpty()) "" else "; other candidates: " + others.joinToString(", ") { it.describe() }
        val hint = if (forced) {
            "--force could not take it over, because no known automation client could be stopped"
        } else {
            "stop it, or run the agent with --force to take the role over"
        }
        return "UI automation is held by $who$also; $hint"
    }

    /** Stops a known holder: SIGKILL for a process this user may signal, `am force-stop` for an app. */
    fun terminate(holder: Holder): Boolean {
        val pkg = holder.pkg
        return try {
            if (pkg != null) {
                run("am", "force-stop", pkg)
                true
            } else if (holder.pid > 0) {
                android.os.Process.killProcess(holder.pid)
                true
            } else {
                false
            }
        } catch (e: Exception) {
            false
        }
    }

    private fun run(vararg command: String): String {
        val process = ProcessBuilder(*command).redirectErrorStream(true).start()
        val text = process.inputStream.bufferedReader().readText()
        process.waitFor()
        return text
    }
}
