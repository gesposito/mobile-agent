package dev.mobileagent.agent

import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo

/**
 * Faults and pauses the host tests inject into a running agent. `serve
 * --test-hooks` arms them from the environment; without that flag the
 * environment arms nothing, so no other launch can inject a fault.
 *
 * - `MOBILE_AGENT_TEST_GET_CHILD_NPE=<times>:<resource id>` makes a tree
 *   read's `getChild` throw what the platform's integrity check throws on a
 *   debuggable build ([integrityCheckNpe]) instead of returning the child
 *   with that resource id, `<times>` times in all. A resource id ending in
 *   `*` names every child whose resource id starts with what precedes the
 *   `*`. Each throw prints `test-hook get_child_npe id=<resource id>
 *   index=<child index> left=<n>`.
 * - `MOBILE_AGENT_TEST_PAUSE_AT_CHILD=<nth>:<ms>:<resource id>` makes the
 *   walk wait `<ms>` ms before it takes the child with that resource id, the
 *   `<nth>` time `getChild` returns it, or until the read is abandoned, so a
 *   test can change the screen under a walk. It prints `test-hook pause
 *   id=<resource id> index=<child index> ms=<ms>` as the wait starts and
 *   `test-hook resume id=<resource id> waited_ms=<n> abandoned=<true|false>`
 *   as it ends.
 * - `MOBILE_AGENT_TEST_GET_CHILD_AS_RELEASE=1` makes `getChild` return null
 *   where the platform's call throws, as it does on a release build, which
 *   runs no integrity check.
 *
 * Tree reads run on the tree worker only, so the counts need no lock.
 */
class TestHooks private constructor(
    private val getChildNpeId: String?,
    private var getChildNpeLeft: Int,
    private val pauseId: String?,
    /** Fetches of the pause's child until the one the walk waits at; 0 once it waited. */
    private var pauseLeft: Int,
    private val pauseMs: Long,
    /** `getChild` returns null where the platform's call throws. */
    @JvmField val getChildAsRelease: Boolean,
) {
    companion object {
        /** The environment variable that arms the getChild fault. */
        const val GET_CHILD_NPE = "MOBILE_AGENT_TEST_GET_CHILD_NPE"

        /** The environment variable that arms the pause at a child. */
        const val PAUSE_AT_CHILD = "MOBILE_AGENT_TEST_PAUSE_AT_CHILD"

        /** The environment variable that makes getChild answer as on a release build. */
        const val GET_CHILD_AS_RELEASE = "MOBILE_AGENT_TEST_GET_CHILD_AS_RELEASE"

        /**
         * The hooks the environment `env` arms; throws [UsageException] when
         * a value is malformed.
         */
        fun parse(env: (String) -> String?): TestHooks {
            var npeId: String? = null
            var npeTimes = 0
            val npe = env(GET_CHILD_NPE)
            if (!npe.isNullOrEmpty()) {
                val colon = npe.indexOf(':')
                val times = if (colon > 0) npe.substring(0, colon).toIntOrNull() else null
                val id = npe.substring(colon + 1)
                if (times == null || times < 1 || id.isEmpty()) {
                    throw UsageException("$GET_CHILD_NPE takes <times>:<resource id>, such as 2:cell-7 or 500:cell-*")
                }
                npeId = id
                npeTimes = times
            }
            var pauseId: String? = null
            var pauseNth = 0
            var pauseMs = 0L
            val pause = env(PAUSE_AT_CHILD)
            if (!pause.isNullOrEmpty()) {
                val parts = pause.split(':', limit = 3)
                val nth = parts.getOrNull(0)?.toIntOrNull()
                val ms = parts.getOrNull(1)?.toLongOrNull()
                val id = parts.getOrNull(2)
                if (nth == null || nth < 1 || ms == null || ms < 0 || id.isNullOrEmpty()) {
                    throw UsageException("$PAUSE_AT_CHILD takes <nth>:<ms>:<resource id>, such as 2:1000:cell-100")
                }
                pauseId = id
                pauseNth = nth
                pauseMs = ms
            }
            val release = when (val value = env(GET_CHILD_AS_RELEASE)) {
                null, "", "0" -> false
                "1" -> true
                else -> throw UsageException("$GET_CHILD_AS_RELEASE takes 1 or 0, not $value")
            }
            return TestHooks(npeId, npeTimes, pauseId, pauseNth, pauseMs, release)
        }

        /**
         * What `AccessibilityNodeInfo.getChild` throws on a debuggable build
         * when the app answers its prefetching request with a single node
         * instead of a list: the accessibility client's integrity check calls
         * `size()` on the missing list. Type, message and top frames are those
         * the API 36 emulator threw; the frames below them are the agent's.
         */
        fun integrityCheckNpe(): NullPointerException {
            val e = NullPointerException(
                "Attempt to invoke interface method 'int java.util.List.size()' on a null object reference",
            )
            val client = "android.view.accessibility.AccessibilityInteractionClient"
            val file = "AccessibilityInteractionClient.java"
            e.stackTrace = arrayOf(
                StackTraceElement(client, "checkFindAccessibilityNodeInfoResultIntegrity", file, 1520),
                StackTraceElement(client, "getFindAccessibilityNodeInfosResultAndClear", file, 1228),
                StackTraceElement(client, "findAccessibilityNodeInfoByAccessibilityId", file, 681),
                StackTraceElement("android.view.accessibility.AccessibilityNodeInfo", "getChild", "AccessibilityNodeInfo.java", 1467),
            ) + e.stackTrace
            return e
        }

        /** Whether resource id `id` is the one `pattern` names, or starts with a `prefix*` pattern's prefix. */
        private fun matches(id: String, pattern: String?): Boolean {
            if (pattern == null) return false
            return if (pattern.endsWith('*')) id.startsWith(pattern.substring(0, pattern.length - 1)) else id == pattern
        }
    }

    /**
     * Sees child `index` as `getChild` returned it, before the walk takes
     * it: waits there while the pause is due for its resource id, throwing
     * [Cancelled] as soon as `cancelled` turns true, and throws
     * [integrityCheckNpe] instead of returning it while the getChild fault
     * is armed for its resource id.
     */
    fun onChild(index: Int, child: AccessibilityNodeInfo?, cancelled: () -> Boolean) {
        val id = child?.viewIdResourceName ?: return
        if (pauseLeft > 0 && matches(id, pauseId)) {
            pauseLeft--
            if (pauseLeft == 0) pause(id, index, cancelled)
        }
        if (getChildNpeLeft > 0 && matches(id, getChildNpeId)) {
            getChildNpeLeft--
            Out.line("test-hook get_child_npe id=$id index=$index left=$getChildNpeLeft")
            throw integrityCheckNpe()
        }
    }

    private fun pause(id: String, index: Int, cancelled: () -> Boolean) {
        Out.line("test-hook pause id=$id index=$index ms=$pauseMs")
        val t0 = SystemClock.uptimeMillis()
        val end = t0 + pauseMs
        while (true) {
            val now = SystemClock.uptimeMillis()
            if (cancelled()) {
                Out.line("test-hook resume id=$id waited_ms=${now - t0} abandoned=true")
                throw Cancelled()
            }
            if (now >= end) break
            Thread.sleep(minOf(10L, end - now))
        }
        Out.line("test-hook resume id=$id waited_ms=${SystemClock.uptimeMillis() - t0} abandoned=false")
    }
}
