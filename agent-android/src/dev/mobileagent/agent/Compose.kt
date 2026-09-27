package dev.mobileagent.agent

import android.os.SystemClock
import dev.mobileagent.agent.codec.EventPayload
import dev.mobileagent.agent.codec.Node
import dev.mobileagent.agent.codec.Snapshot
import dev.mobileagent.agent.codec.TreeChange
import dev.mobileagent.agent.codec.TreeChangeKind

/**
 * Windows hosting Jetpack Compose (an AndroidComposeView, under a
 * `androidx.compose.ui.platform.ComposeView`) send no accessibility events to
 * a UI automation client (spike S1, design D5), so their platform cache goes
 * stale after most changes. The tracker re-reads them in full with their
 * cache evicted, on a poll, diffs each read against the last read the host
 * could have seen, and reports the smallest subtree holding every difference
 * as a tree event.
 */
class ComposeTracker(
    private val automation: Automation,
    private val reader: TreeReader,
    private val emit: (EventPayload) -> Unit,
    private val trace: Boolean,
) {
    companion object {
        /** Class names of Compose's hosting views start with this. */
        const val COMPOSE_PREFIX = "androidx.compose."

        fun hasCompose(s: Snapshot, root: Node): Boolean {
            val stack = ArrayList<Node>()
            stack.add(root)
            while (stack.isNotEmpty()) {
                val n = stack.removeAt(stack.size - 1)
                if (s.strings[n.cls].startsWith(COMPOSE_PREFIX)) return true
                stack.addAll(n.children)
            }
            return false
        }

        /** Every node of the tree under `root`, keyed by its child-index path, hashed with what the payload carries. */
        fun signatures(s: Snapshot, root: Node): HashMap<String, Long> {
            val out = HashMap<String, Long>()
            fun visit(n: Node, path: String) {
                var h = s.strings[n.cls].hashCode().toLong()
                h = h * 31 + (n.resPkg?.let { s.strings[it].hashCode() } ?: 0)
                h = h * 31 + (n.resId?.hashCode() ?: 0)
                h = h * 31 + (n.text?.hashCode() ?: 0)
                h = h * 31 + (n.desc?.hashCode() ?: 0)
                h = h * 31 + n.bounds.left
                h = h * 31 + n.bounds.top
                h = h * 31 + n.bounds.right
                h = h * 31 + n.bounds.bottom
                h = h * 31 + n.flags
                h = h * 31 + n.children.size
                out[path] = h
                n.children.forEachIndexed { i, c -> visit(c, if (path.isEmpty()) "$i" else "$path.$i") }
            }
            visit(root, "")
            return out
        }

        /** The deepest path above every difference between two signature maps, or null when equal. */
        fun changedSubtree(before: Map<String, Long>, after: Map<String, Long>): IntArray? {
            var common: List<String>? = null
            fun merge(path: String) {
                val parts = if (path.isEmpty()) emptyList() else path.split('.')
                val c = common
                common = if (c == null) parts else c.zip(parts).takeWhile { it.first == it.second }.map { it.first }
            }
            for ((k, v) in after) if (before[k] != v) merge(k)
            for (k in before.keys) if (!after.containsKey(k)) merge(k)
            return common?.map { it.toInt() }?.toIntArray()
        }
    }

    /** Window ids known to host Compose. */
    private val compose = HashSet<Int>()

    /** Per Compose window, the content of the last read the host could have seen. */
    private val seen = HashMap<Int, HashMap<String, Long>>()

    /** An injected action happened since the Compose windows were last read fresh. */
    @Volatile private var dirty = false

    fun markDirty() {
        dirty = true
    }

    /** Whether an action happened since the last fresh read; clears the mark. */
    fun takeDirty(): Boolean {
        val was = dirty
        dirty = false
        return was
    }

    /** Duration of the last re-read, which paces the poll. */
    @Volatile var lastRereadMs = 0L
        private set

    /** Classifies the windows of a read the host receives and records what it saw of them. */
    @Synchronized
    fun observe(s: Snapshot) {
        for (w in s.windows) {
            val root = w.root ?: continue
            if (hasCompose(s, root)) {
                compose.add(w.id)
                seen[w.id] = signatures(s, root)
            } else {
                compose.remove(w.id)
                seen.remove(w.id)
            }
        }
    }

    /** Ids of the windows on screen now that host Compose. */
    fun composeWindowsOnScreen(): List<Int> {
        val ids = automation.windows().map { it.id }
        synchronized(this) { return ids.filter { it in compose } }
    }

    /**
     * Evicts every cached node of the Compose windows on screen that a
     * snapshot has shown the host, re-reads them, and reports a tree event for each whose
     * content differs from what the host last saw. Windows the host has not
     * read are left alone: it holds nothing of them that could be stale, and
     * the window events announce them. Runs on the tree worker.
     */
    fun reread() {
        val ids = automation.windows().map { it.id }.toSet()
        val targets: List<Int>
        synchronized(this) {
            compose.retainAll(ids)
            seen.keys.retainAll(ids)
            targets = ids.filter { it in compose }
        }
        if (targets.isEmpty()) return
        val t0 = SystemClock.uptimeMillis()
        dirty = false
        for (id in targets) automation.evictWindow(id)
        val result = reader.read(Int.MAX_VALUE, TreeReader.DEFAULT_MAX_NODES, 0, targets.toSet()) { false }
        lastRereadMs = SystemClock.uptimeMillis() - t0
        val s = result.snapshot
        val changed = ArrayList<String>()
        synchronized(this) {
            for (w in s.windows) {
                val root = w.root ?: continue
                if (!hasCompose(s, root)) {
                    compose.remove(w.id)
                    continue
                }
                val now = signatures(s, root)
                val before = seen.put(w.id, now) ?: continue
                val path = changedSubtree(before, now) ?: continue
                emit(TreeChange(TreeChangeKind.CONTENT, w.id, path))
                changed.add("${w.id}:${path.joinToString(".")}")
            }
        }
        if (trace) {
            Out.line(
                "trace compose windows=${targets.joinToString(",")} nodes=${s.nodeCount} read_ms=$lastRereadMs " +
                    "round_trips=${result.roundTrips} changed=${changed.joinToString(",").ifEmpty { "-" }}",
            )
        }
    }
}
