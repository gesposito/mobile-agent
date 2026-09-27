package dev.mobileagent.agent

import android.view.accessibility.AccessibilityNodeInfo

/**
 * Faults the host tests inject into a running agent. `serve --test-hooks`
 * arms them from the environment; without that flag the environment arms
 * nothing, so no other launch can inject a fault.
 *
 * `MOBILE_AGENT_TEST_GET_CHILD_NPE=<times>:<resource id>` makes a tree read's
 * `getChild` throw what the platform's integrity check throws on a
 * debuggable build ([integrityCheckNpe]) instead of returning the child with
 * that resource id, `<times>` times in all. Each throw prints
 * `test-hook get_child_npe id=<resource id> index=<child index> left=<n>`.
 *
 * Tree reads run on the tree worker only, so the count needs no lock.
 */
class TestHooks private constructor(
    private val getChildNpeId: String?,
    private var getChildNpeLeft: Int,
) {
    companion object {
        /** The environment variable that arms the getChild fault. */
        const val GET_CHILD_NPE = "MOBILE_AGENT_TEST_GET_CHILD_NPE"

        /**
         * The hooks armed by `getChildNpe`, the value of [GET_CHILD_NPE];
         * throws [UsageException] when it is not `<times>:<resource id>`.
         */
        fun parse(getChildNpe: String?): TestHooks {
            if (getChildNpe.isNullOrEmpty()) return TestHooks(null, 0)
            val colon = getChildNpe.indexOf(':')
            val times = if (colon > 0) getChildNpe.substring(0, colon).toIntOrNull() else null
            val id = getChildNpe.substring(colon + 1)
            if (times == null || times < 1 || id.isEmpty()) {
                throw UsageException("$GET_CHILD_NPE takes <times>:<resource id>, such as 2:cell-7")
            }
            return TestHooks(id, times)
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
    }

    /**
     * Sees child `index` as `getChild` returned it, before the walk takes
     * it, and throws [integrityCheckNpe] instead while the getChild fault is
     * armed for its resource id.
     */
    fun onChild(index: Int, child: AccessibilityNodeInfo?) {
        if (getChildNpeLeft <= 0 || child == null || child.viewIdResourceName != getChildNpeId) return
        getChildNpeLeft--
        Out.line("test-hook get_child_npe id=$getChildNpeId index=$index left=$getChildNpeLeft")
        throw integrityCheckNpe()
    }
}
