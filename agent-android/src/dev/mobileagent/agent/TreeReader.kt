package dev.mobileagent.agent

import android.graphics.Rect
import android.os.Build
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import dev.mobileagent.agent.codec.Bounds
import dev.mobileagent.agent.codec.Node
import dev.mobileagent.agent.codec.NodeFlag
import dev.mobileagent.agent.codec.Protocol
import dev.mobileagent.agent.codec.Scroll
import dev.mobileagent.agent.codec.Snapshot
import dev.mobileagent.agent.codec.Truncation
import dev.mobileagent.agent.codec.Window
import dev.mobileagent.agent.codec.WindowFlag
import dev.mobileagent.agent.codec.WindowType

/** Thrown out of a walk whose request was abandoned; carries no stack trace. */
class Cancelled : RuntimeException() {
    override fun fillInStackTrace(): Throwable = this
}

/**
 * Reads the tree of every interactive window into the protocol's snapshot
 * payload: nodes breadth first with descendant prefetch (spike S1's `bfs-u`),
 * class names, resource-id prefixes and packages in a string table, node ids
 * in walk order.
 *
 * The caps bound what is serialized, not what is counted: past the node cap
 * or below the depth cap the walk goes on counting, up to [COUNT_LIMIT]
 * nodes, so `node_count` is the whole screen's count and the host learns how
 * many nodes a truncated snapshot omitted.
 *
 * Each window also carries its metadata and each scrolling node its scroll
 * semantics ([Semantics]): a window whose walk meets Compose content sends
 * no change events, one the walk completed without meeting it does.
 */
class TreeReader(private val automation: Automation) {
    companion object {
        /** Nodes serialized when the request names no cap. */
        const val DEFAULT_MAX_NODES = 10_000

        /** Nodes counted at most, serialized or not. */
        const val COUNT_LIMIT = 100_000

        /** Descendants breadth first, uninterrupted; at most 50 nodes per request. */
        private const val PREFETCH = AccessibilityNodeInfo.FLAG_PREFETCH_DESCENDANTS_BREADTH_FIRST or
            AccessibilityNodeInfo.FLAG_PREFETCH_UNINTERRUPTIBLE

        private val PREFETCH_API = Build.VERSION.SDK_INT >= 33

        fun root(w: AccessibilityWindowInfo): AccessibilityNodeInfo? =
            if (PREFETCH_API) w.getRoot(PREFETCH) else w.root

        fun child(n: AccessibilityNodeInfo, i: Int): AccessibilityNodeInfo? =
            if (PREFETCH_API) n.getChild(i, PREFETCH) else n.getChild(i)

        /** Splits `pkg:id/name` into the prefix `pkg:id/` and `name`. */
        fun splitResourceId(id: String): Pair<String?, String> {
            val i = id.indexOf(":id/")
            return if (i >= 0) id.substring(0, i + 4) to id.substring(i + 4) else null to id
        }

        @Suppress("DEPRECATION")
        fun nodeFlags(n: AccessibilityNodeInfo): Int {
            var f = 0
            if (n.isClickable) f = f or NodeFlag.CLICKABLE
            if (n.isLongClickable) f = f or NodeFlag.LONG_CLICKABLE
            if (n.isFocusable) f = f or NodeFlag.FOCUSABLE
            if (n.isFocused) f = f or NodeFlag.FOCUSED
            if (n.isEnabled) f = f or NodeFlag.ENABLED
            if (n.isCheckable) f = f or NodeFlag.CHECKABLE
            if (n.isChecked) f = f or NodeFlag.CHECKED
            if (n.isScrollable) f = f or NodeFlag.SCROLLABLE
            if (n.isSelected) f = f or NodeFlag.SELECTED
            if (n.isEditable) f = f or NodeFlag.EDITABLE
            if (n.isPassword) f = f or NodeFlag.PASSWORD
            if (n.isVisibleToUser) f = f or NodeFlag.VISIBLE
            if (n.isAccessibilityFocused) f = f or NodeFlag.ACCESSIBILITY_FOCUSED
            return f
        }

        fun windowType(type: Int): Int =
            if (type in WindowType.APPLICATION..WindowType.MAGNIFICATION_OVERLAY) type else WindowType.UNKNOWN

        fun windowFlags(w: AccessibilityWindowInfo): Int {
            var f = 0
            if (w.isActive) f = f or WindowFlag.ACTIVE
            if (w.isFocused) f = f or WindowFlag.FOCUSED
            if (w.isAccessibilityFocused) f = f or WindowFlag.ACCESSIBILITY_FOCUSED
            if (w.isInPictureInPictureMode) f = f or WindowFlag.PICTURE_IN_PICTURE
            return f
        }
    }

    /** One read: the payload, the platform nodes by snapshot node id, and what it cost. */
    class Result(
        @JvmField val snapshot: Snapshot,
        @JvmField val nodes: Array<AccessibilityNodeInfo>,
        /** Window id of each serialized node, by node id. */
        @JvmField val nodeWindows: IntArray,
        @JvmField val visible: Int,
        @JvmField val readMs: Long,
        /** Remote requests the walk made, or -1 when they cannot be counted. */
        @JvmField val roundTrips: Int,
    )

    /** A node waiting in the breadth-first queue. */
    private class Item(
        @JvmField val info: AccessibilityNodeInfo,
        @JvmField val depth: Int,
        /** The parent's snapshot id, or -1 for a root or a node that is only counted. */
        @JvmField val parent: Int,
        @JvmField val window: Int,
        @JvmField val serialize: Boolean,
    )

    /** A serialized node before the tree is assembled. */
    private class Built(
        @JvmField val cls: Int,
        @JvmField val resPkg: Int?,
        @JvmField val resId: String?,
        @JvmField val text: String?,
        @JvmField val desc: String?,
        @JvmField val bounds: Bounds,
        @JvmField val flags: Int,
    ) {
        @JvmField var children: IntArray = EMPTY
        @JvmField var childCount = 0
        @JvmField var scroll: Scroll? = null

        fun add(id: Int) {
            if (childCount == children.size) children = children.copyOf(maxOf(4, childCount * 2))
            children[childCount++] = id
        }

        companion object {
            val EMPTY = IntArray(0)
        }
    }

    /**
     * Reads every interactive window, or those whose id is in `only`.
     * `maxDepth` levels below each root and `maxNodes` nodes are serialized;
     * `seq` is the last tree event sent. Throws [Cancelled] as soon as
     * `cancelled` turns true.
     */
    fun read(maxDepth: Int, maxNodes: Int, seq: Long, only: Set<Int>? = null, cancelled: () -> Boolean): Result {
        val t0 = SystemClock.uptimeMillis()
        val rt0 = RoundTrips.count()
        val depthCap = minOf(maxDepth, Protocol.MAX_NODE_DEPTH)
        val strings = LinkedHashMap<String, Int>()
        fun intern(s: String): Int = strings.getOrPut(s) { strings.size }

        val all = automation.windows()
        val windows = all.filter { only == null || it.id in only }.sortedByDescending { it.layer }
        val queue = ArrayDeque<Item>()
        val roots = IntArray(windows.size) { -1 }
        val packages = arrayOfNulls<String>(windows.size)
        val rooted = BooleanArray(windows.size)
        /** Whether the walk met Compose content in each window. */
        val compose = BooleanArray(windows.size)
        var truncated = 0
        windows.forEachIndexed { i, w ->
            val r = root(w)
            if (r == null) {
                truncated = truncated or Truncation.WINDOWS
            } else {
                rooted[i] = true
                packages[i] = r.packageName?.toString()
                queue.addLast(Item(r, 0, -1, i, true))
            }
        }
        val packageIndices = Array(windows.size) { i -> packages[i]?.let { intern(it) } }

        val built = ArrayList<Built>()
        val infos = ArrayList<AccessibilityNodeInfo>()
        val nodeWindows = ArrayList<Int>()
        val rect = Rect()
        var total = 0
        var visible = 0
        while (queue.isNotEmpty() && total < COUNT_LIMIT) {
            if (cancelled()) throw Cancelled()
            val item = queue.removeFirst()
            val n = item.info
            total++
            if (n.isVisibleToUser) visible++
            if (!compose[item.window] && Semantics.hostsCompose(n)) compose[item.window] = true
            var id = -1
            if (item.serialize) {
                if (built.size < maxNodes) {
                    id = built.size
                    n.getBoundsInScreen(rect)
                    val (prefix, rest) = n.viewIdResourceName?.let { splitResourceId(it) } ?: (null to null)
                    built.add(
                        Built(
                            intern(n.className?.toString() ?: ""),
                            prefix?.let { intern(it) },
                            rest,
                            n.text?.toString(),
                            n.contentDescription?.toString(),
                            Bounds(rect.left, rect.top, rect.right, rect.bottom),
                            nodeFlags(n),
                        ),
                    )
                    infos.add(n)
                    nodeWindows.add(windows[item.window].id)
                    if (item.parent >= 0) built[item.parent].add(id) else roots[item.window] = id
                } else {
                    truncated = truncated or Truncation.NODES
                }
            }
            // Scroll semantics look at the children, which the walk fetches anyway.
            val scrolling = if (id >= 0 && Semantics.scrolls(n)) ArrayList<AccessibilityNodeInfo>() else null
            val count = if (item.depth >= Protocol.MAX_NODE_DEPTH) 0 else n.childCount
            if (count > 0) {
                val childSerialized = id >= 0 && item.depth + 1 <= depthCap
                if (id >= 0 && !childSerialized) truncated = truncated or Truncation.DEPTH
                for (i in 0 until count) {
                    val c = child(n, i) ?: continue
                    scrolling?.add(c)
                    queue.addLast(Item(c, item.depth + 1, if (childSerialized) id else -1, item.window, childSerialized))
                }
            }
            if (scrolling != null) built[id].scroll = Semantics.scroll(n, scrolling)
        }
        val complete = queue.isEmpty()
        if (!complete) truncated = truncated or Truncation.NODES

        // Children have larger ids than their parent, so assemble from the end.
        val nodes = arrayOfNulls<Node>(built.size)
        for (id in built.indices.reversed()) {
            val b = built[id]
            val children = if (b.childCount == 0) Node.NO_CHILDREN else Array(b.childCount) { nodes[b.children[it]]!! }
            nodes[id] = Node(id, b.cls, b.resPkg, b.resId, b.text, b.desc, b.bounds, b.flags, children, b.scroll)
        }
        val display = Semantics.displaySize()
        val wire = Array(windows.size) { i ->
            val w = windows[i]
            val type = windowType(w.type)
            val modal = Semantics.modal(w, all, display, rect)
            w.getBoundsInScreen(rect)
            Window(
                w.id,
                type,
                w.layer,
                packageIndices[i],
                w.title?.toString(),
                Bounds(rect.left, rect.top, rect.right, rect.bottom),
                windowFlags(w),
                roots[i].takeIf { it >= 0 }?.let { nodes[it] },
                emitsEvents = when {
                    compose[i] -> false
                    !rooted[i] || !complete -> null
                    else -> true
                },
                ownerIndex = Semantics.systemOwner(type, packages[i])?.let { intern(it) },
                modal = modal,
            )
        }
        val snapshot = Snapshot(strings.keys.toTypedArray(), wire, total, truncated, seq)
        val rt = RoundTrips.count()
        return Result(
            snapshot,
            infos.toTypedArray(),
            nodeWindows.toIntArray(),
            visible,
            SystemClock.uptimeMillis() - t0,
            if (rt0 < 0 || rt < 0) -1 else rt - rt0,
        )
    }
}

/**
 * Remote requests made by the calling thread's accessibility client: its
 * interaction id counter, a hidden field read for measurements only. Spike
 * S1 found it equal to a proxy's count of binder calls on every run.
 */
object RoundTrips {
    private val counter: java.lang.reflect.Field? = try {
        Class.forName("android.view.accessibility.AccessibilityInteractionClient")
            .getDeclaredField("mInteractionIdCounter").apply { isAccessible = true }
    } catch (e: ReflectiveOperationException) {
        null
    }
    private val instance: java.lang.reflect.Method? = try {
        Class.forName("android.view.accessibility.AccessibilityInteractionClient").getDeclaredMethod("getInstance")
    } catch (e: ReflectiveOperationException) {
        null
    }

    /** The counter of the calling thread's client, or -1. */
    fun count(): Int = try {
        val client = instance?.invoke(null)
        (counter?.get(client) as? java.util.concurrent.atomic.AtomicInteger)?.get() ?: -1
    } catch (e: Exception) {
        -1
    }
}
