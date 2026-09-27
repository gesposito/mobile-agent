package dev.mobileagent.agent

import android.graphics.Rect
import android.os.Build
import android.util.Log
import android.view.accessibility.AccessibilityNodeInfo
import dev.mobileagent.agent.codec.Bounds
import dev.mobileagent.agent.codec.NodeBounds
import dev.mobileagent.agent.codec.Probed

/**
 * Bounds probes (task 10.11): the bounds of nodes of the last snapshot, read
 * again past the platform's node cache. A translation, such as a screen
 * sliding in when an app starts, moves nodes without a change event, so the
 * cache keeps the bounds it read before the move, and a host with no frames
 * to watch would take the screen for settled while it still moves.
 * `AccessibilityNodeInfo.refresh()` asks the app for the node again, past
 * the cache and without prefetch, so each node costs one round trip to the
 * app's UI thread.
 */
object BoundsProbe {
    /**
     * The nodes `ids` names in `read`, the last snapshot's nodes by id, each
     * with its bounds now, in the order of `ids`. A node that is gone, whose
     * refresh fails, or that `read` does not hold is left out. Throws
     * [Cancelled] once `cancelled` turns true.
     */
    fun read(read: TreeReader.Result?, ids: IntArray, cancelled: () -> Boolean): Probed {
        val nodes = read?.nodes ?: return Probed(arrayOf())
        val found = ArrayList<NodeBounds>(ids.size)
        val rect = Rect()
        for (id in ids) {
            if (cancelled()) throw Cancelled()
            // Ids are unsigned: one past Int.MAX_VALUE indexes no snapshot.
            val node = if (id >= 0) nodes.getOrNull(id) else null
            val fresh = node?.let { refreshed(it) } ?: continue
            fresh.getBoundsInScreen(rect)
            found.add(NodeBounds(id, Bounds(rect.left, rect.top, rect.right, rect.bottom)))
        }
        return Probed(found.toTypedArray())
    }

    /**
     * A copy of `node` read again from the app, or null when the node is
     * gone. The snapshot's own node stays as the walk read it for the other
     * requests that name it, such as text_set on the input thread.
     */
    private fun refreshed(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
        val copy = copyOf(node)
        return try {
            if (copy.refresh()) copy else null
        } catch (e: RuntimeException) {
            Log.d(TreeReader.LOG_TAG, "refreshing a node for its bounds threw; taken as gone", e)
            null
        }
    }

    @Suppress("DEPRECATION")
    private fun copyOf(node: AccessibilityNodeInfo): AccessibilityNodeInfo =
        if (Build.VERSION.SDK_INT >= 33) AccessibilityNodeInfo(node) else AccessibilityNodeInfo.obtain(node)
}
