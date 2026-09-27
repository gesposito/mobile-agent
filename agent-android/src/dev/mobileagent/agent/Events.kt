package dev.mobileagent.agent

import android.os.SystemClock
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import dev.mobileagent.agent.codec.EventPayload
import dev.mobileagent.agent.codec.Protocol
import dev.mobileagent.agent.codec.TreeChange
import dev.mobileagent.agent.codec.TreeChangeKind
import dev.mobileagent.agent.codec.WindowChange
import dev.mobileagent.agent.codec.WindowChangeKind

/**
 * Accessibility events from the UI automation connection, turned into the
 * protocol's tree and window events. They arrive on the connection's
 * callback thread after the platform's node cache has processed them, so a
 * tree event's subtree is already fresh in the cache when the host reads it.
 */
class EventHub(private val automation: Automation) {
    companion object {
        /**
         * The quiet window of the idle wait: longer than the gaps between
         * the batches of one continuing change, which spike S1 measured at
         * 101 to 116 ms (p50) and 118 to 147 ms (p95), as design D8 sets it.
         */
        const val QUIET_MS = 200L

        /** The idle bound when a request names none; the spec requires under a second. */
        const val DEFAULT_IDLE_MS = 500L

        /** Hidden members for child-index paths; present from API 21 to 37. */
        private val sourceNodeId = AccessibilityNodeInfo::class.java.getDeclaredMethod("getSourceNodeId")
        private val childId =
            AccessibilityNodeInfo::class.java.getDeclaredMethod("getChildId", Int::class.javaPrimitiveType)

        private val EMPTY = IntArray(0)
    }

    /** Where events go: the current session, if one is ready. */
    @Volatile var sink: ((EventPayload, Long) -> Unit)? = null

    /** [SystemClock.uptimeMillis] of the last event, 0 before the first. */
    @Volatile var lastEvent = 0L
        private set

    fun start() {
        automation.uia.setOnAccessibilityEventListener { e -> onEvent(e) }
    }

    /**
     * Waits until no event has arrived for [QUIET_MS], or until `boundMs`
     * passed, whichever comes first, and returns the ms waited. An endless
     * animation keeps sending events, so the wait then lasts the bound and
     * never longer.
     */
    fun waitIdle(boundMs: Long, cancelled: () -> Boolean): Long {
        val start = SystemClock.uptimeMillis()
        if (boundMs <= 0) return 0
        val end = start + boundMs
        while (true) {
            val now = SystemClock.uptimeMillis()
            val quiet = now - lastEvent
            if (quiet >= QUIET_MS || now >= end) return now - start
            if (cancelled()) throw Cancelled()
            Thread.sleep(minOf(QUIET_MS - quiet, end - now, 20L))
        }
    }

    private fun onEvent(e: AccessibilityEvent) {
        lastEvent = SystemClock.uptimeMillis()
        val out = sink ?: return
        // Event times are uptime milliseconds, the clock of Session.nowUs.
        val timeUs = e.eventTime * 1000
        try {
            for (payload in payloads(e)) out(payload, timeUs)
        } catch (t: RuntimeException) {
            // A node that vanished mid-lookup: the next event or read covers it.
        }
    }

    /** The protocol events one accessibility event stands for; empty for input and feedback events. */
    private fun payloads(e: AccessibilityEvent): List<EventPayload> {
        val window = e.windowId
        return when (e.eventType) {
            AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED -> {
                val types = e.contentChangeTypes
                val kind = if (types and AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE == 0 &&
                    types and AccessibilityEvent.CONTENT_CHANGE_TYPE_TEXT != 0
                ) {
                    TreeChangeKind.TEXT
                } else {
                    TreeChangeKind.CONTENT
                }
                listOf(TreeChange(kind, window, path(e)))
            }
            AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED -> listOf(TreeChange(TreeChangeKind.TEXT, window, path(e)))
            AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED, AccessibilityEvent.TYPE_VIEW_SELECTED ->
                listOf(TreeChange(TreeChangeKind.SELECTION, window, path(e)))
            AccessibilityEvent.TYPE_VIEW_SCROLLED -> listOf(TreeChange(TreeChangeKind.SCROLL, window, path(e)))
            AccessibilityEvent.TYPE_VIEW_FOCUSED,
            AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED,
            AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
            -> listOf(TreeChange(TreeChangeKind.FOCUS, window, path(e)))
            AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED -> listOf(WindowChange(WindowChangeKind.STATE, window))
            AccessibilityEvent.TYPE_WINDOWS_CHANGED -> windowChanges(e.windowChanges).map { WindowChange(it, window) }
            else -> emptyList()
        }
    }

    private fun windowChanges(bits: Int): List<Int> {
        val kinds = ArrayList<Int>(2)
        if (bits and AccessibilityEvent.WINDOWS_CHANGE_ADDED != 0) kinds.add(WindowChangeKind.ADDED)
        if (bits and AccessibilityEvent.WINDOWS_CHANGE_REMOVED != 0) kinds.add(WindowChangeKind.REMOVED)
        if (bits and AccessibilityEvent.WINDOWS_CHANGE_BOUNDS != 0) kinds.add(WindowChangeKind.BOUNDS)
        val focus = AccessibilityEvent.WINDOWS_CHANGE_ACTIVE or AccessibilityEvent.WINDOWS_CHANGE_FOCUSED or
            AccessibilityEvent.WINDOWS_CHANGE_ACCESSIBILITY_FOCUSED
        if (bits and focus != 0) kinds.add(WindowChangeKind.FOCUS)
        val state = AccessibilityEvent.WINDOWS_CHANGE_TITLE or AccessibilityEvent.WINDOWS_CHANGE_LAYER or
            AccessibilityEvent.WINDOWS_CHANGE_PARENT or AccessibilityEvent.WINDOWS_CHANGE_CHILDREN or
            AccessibilityEvent.WINDOWS_CHANGE_PIP
        if (bits and state != 0 || kinds.isEmpty()) kinds.add(WindowChangeKind.STATE)
        return kinds
    }

    /**
     * Child indices from the window root down to the event's source, found
     * by climbing parents, which the platform cache holds; the whole window
     * (an empty path) when the source cannot be followed.
     */
    private fun path(e: AccessibilityEvent): IntArray {
        val source = e.source ?: return EMPTY
        val reversed = ArrayList<Int>(16)
        var node: AccessibilityNodeInfo = source
        for (depth in 0 until Protocol.MAX_NODE_DEPTH) {
            val parent = node.parent ?: break
            val id = sourceNodeId.invoke(node) as Long
            var index = -1
            for (i in 0 until parent.childCount) {
                if (childId.invoke(parent, i) as Long == id) {
                    index = i
                    break
                }
            }
            if (index < 0) return EMPTY
            reversed.add(index)
            node = parent
        }
        return IntArray(reversed.size) { reversed[reversed.size - 1 - it] }
    }
}
