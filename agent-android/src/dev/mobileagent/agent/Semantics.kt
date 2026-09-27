package dev.mobileagent.agent

import android.graphics.Rect
import android.os.Build
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction
import android.view.accessibility.AccessibilityWindowInfo
import dev.mobileagent.agent.codec.Scroll
import dev.mobileagent.agent.codec.ScrollAxis
import dev.mobileagent.agent.codec.ScrollFlag
import dev.mobileagent.agent.codec.ScrollPosition
import dev.mobileagent.agent.codec.ScrollRange
import dev.mobileagent.agent.codec.ScrollUnit
import dev.mobileagent.agent.codec.WindowType
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * What a snapshot says beyond the platform's own node and window states
 * (task 3.6): whether a window's content sends change events, which system
 * process owns it, whether it is a modal, and how each scrolling node
 * scrolls. Every input is a field of a node or window the walk already
 * holds, so none of this costs a round trip.
 */
object Semantics {
    /** Class names of Compose's host views start with this. */
    private const val COMPOSE_PREFIX = "androidx.compose."

    /**
     * The extra data key every Compose semantics node advertises. A Compose
     * dialog's window names no Compose class (its host view reports itself
     * as `android.view.View`), so this is what finds Compose there.
     */
    private const val COMPOSE_EXTRA = "androidx.compose.ui.semantics.id"

    /**
     * Packages whose application windows are the platform's own screens and
     * prompts, owned by the system rather than by an app under automation.
     */
    private val PLATFORM_PACKAGES = setOf(
        "android",
        "com.android.systemui",
        "com.android.permissioncontroller",
        "com.google.android.permissioncontroller",
        "com.android.packageinstaller",
        "com.google.android.packageinstaller",
        "com.android.intentresolver",
    )

    /**
     * The package of the default home app, the launcher, from the hidden
     * `IPackageManager.getHomeActivities` (7 ms on the API 36 emulator);
     * null when none is set or it cannot be read.
     */
    private val homePackage: String? by lazy {
        try {
            val pm = Class.forName("android.app.ActivityThread").getMethod("getPackageManager").invoke(null)
            val home = pm!!.javaClass.getMethod("getHomeActivities", List::class.java)
                .invoke(pm, ArrayList<android.content.pm.ResolveInfo>()) as android.content.ComponentName?
            home?.packageName
        } catch (e: Exception) {
            null
        }
    }

    private val SCROLL_BACK = intArrayOf(
        AccessibilityAction.ACTION_SCROLL_UP.id,
        AccessibilityAction.ACTION_PAGE_UP.id,
        AccessibilityAction.ACTION_SCROLL_LEFT.id,
        AccessibilityAction.ACTION_PAGE_LEFT.id,
    )
    private val SCROLL_FORWARD = intArrayOf(
        AccessibilityAction.ACTION_SCROLL_DOWN.id,
        AccessibilityAction.ACTION_PAGE_DOWN.id,
        AccessibilityAction.ACTION_SCROLL_RIGHT.id,
        AccessibilityAction.ACTION_PAGE_RIGHT.id,
    )
    private val VERTICAL = intArrayOf(
        AccessibilityAction.ACTION_SCROLL_UP.id,
        AccessibilityAction.ACTION_PAGE_UP.id,
        AccessibilityAction.ACTION_SCROLL_DOWN.id,
        AccessibilityAction.ACTION_PAGE_DOWN.id,
    )
    private val HORIZONTAL = intArrayOf(
        AccessibilityAction.ACTION_SCROLL_LEFT.id,
        AccessibilityAction.ACTION_PAGE_LEFT.id,
        AccessibilityAction.ACTION_SCROLL_RIGHT.id,
        AccessibilityAction.ACTION_PAGE_RIGHT.id,
    )

    /** Whether `n` belongs to Compose content, which sends no change events to UI automation (spike S1). */
    fun hostsCompose(n: AccessibilityNodeInfo): Boolean =
        n.className?.startsWith(COMPOSE_PREFIX) == true || COMPOSE_EXTRA in n.availableExtraData

    /**
     * The system process that owns a window of protocol type `type` whose
     * content is package `pkg`: every window that is not an application's
     * (status and navigation bars, the keyboard, overlays), and application
     * windows of the platform's packages or of the launcher. Null for the
     * app under automation and when the package is unknown.
     */
    fun systemOwner(type: Int, pkg: String?): String? {
        if (pkg == null) return null
        val application = type == WindowType.APPLICATION || type == WindowType.UNKNOWN
        return if (!application || pkg in PLATFORM_PACKAGES || pkg == homePackage) pkg else null
    }

    /**
     * Whether `w` is a modal. The platform leaves out of the interactive
     * windows every window a touch-modal window blocks, so an application
     * or system window that floats inset from the display's edges on both
     * axes, with no application window listed below it, blocks what it
     * floats over: a dialog, a permission prompt. A letterboxed activity
     * spans the display along one axis and is not taken for one; neither is
     * a picture-in-picture window. `all` is every interactive window and
     * `display` the display's width and height.
     */
    fun modal(w: AccessibilityWindowInfo, all: List<AccessibilityWindowInfo>, display: IntArray?, rect: Rect): Boolean {
        if (display == null || w.isInPictureInPictureMode) return false
        val type = w.type
        if (type != AccessibilityWindowInfo.TYPE_APPLICATION && type != AccessibilityWindowInfo.TYPE_SYSTEM) return false
        w.getBoundsInScreen(rect)
        val spansWidth = rect.left <= 0 && rect.right >= display[0]
        val spansHeight = rect.top <= 0 && rect.bottom >= display[1]
        if (spansWidth || spansHeight) return false
        return all.none { it.layer < w.layer && it.type == AccessibilityWindowInfo.TYPE_APPLICATION }
    }

    /**
     * The default display's width and height as it is turned now, from the
     * hidden `DisplayManagerGlobal.getDisplayInfo`; null when they cannot be
     * read.
     */
    fun displaySize(): IntArray? = try {
        val cls = Class.forName("android.hardware.display.DisplayManagerGlobal")
        val global = cls.getDeclaredMethod("getInstance").invoke(null)
        val info = cls.getDeclaredMethod("getDisplayInfo", Int::class.javaPrimitiveType).invoke(global, 0)!!
        intArrayOf(info.javaClass.getField("logicalWidth").getInt(info), info.javaClass.getField("logicalHeight").getInt(info))
    } catch (e: Exception) {
        null
    }

    /**
     * Whether window `w`, whose root is `root`, is drawn away from where its layout puts it, as
     * the platform draws a window while it animates the window in or out: its frame partly off
     * the display (`display`, the width and height), or its frame or its root on screen at another
     * size than the root's layout. Its bounds and its nodes' are then those of a frame of that
     * animation. The frame, not the root's bounds, is what is checked against the display, since a
     * window panned for the keyboard moves its root and not its frame; and the frame's size is
     * checked as well as the root's, since a root fetched again can come back unscaled while the
     * frame and the other nodes still read scaled. The layout's size is the root's bounds in the
     * window, which the platform reports from Android 14.
     */
    fun transformed(w: AccessibilityWindowInfo, root: AccessibilityNodeInfo?, display: IntArray?): Boolean {
        val frame = Rect()
        w.getBoundsInScreen(frame)
        if (frame.isEmpty) return false
        if (display != null &&
            (frame.left < 0 || frame.top < 0 || frame.right > display[0] || frame.bottom > display[1])
        ) {
            return true
        }
        if (root == null || Build.VERSION.SDK_INT < 34) return false
        val layout = Rect()
        root.getBoundsInWindow(layout)
        if (layout.isEmpty) return false
        val screen = Rect()
        root.getBoundsInScreen(screen)
        return sizeDiffers(frame, layout) || sizeDiffers(screen, layout)
    }

    private fun sizeDiffers(a: Rect, b: Rect) = abs(a.width() - b.width()) > 1 || abs(a.height() - b.height()) > 1

    /** Whether `n` scrolls: the platform marks it scrollable or it offers a scroll action. */
    fun scrolls(n: AccessibilityNodeInfo): Boolean {
        if (n.isScrollable) return true
        for (a in n.actionList) {
            val id = a.id
            if (id == AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD || id == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD ||
                id in SCROLL_BACK || id in SCROLL_FORWARD
            ) {
                return true
            }
        }
        return false
    }

    /**
     * The scroll semantics of `n`, whose fetched children are `children`,
     * all of them unless the walk skipped one (`complete` false):
     *
     * - The axis from the directional scroll actions, else from a collection
     *   one item wide or high.
     * - Back and forward from the logical scroll actions, else from the
     *   directional ones (a right-to-left layout scrolls forward to the left).
     * - For a collection, its items along the axis, and the items in view
     *   when they can be told: from every child in view naming its item,
     *   from the child indices when every item is a child (Settings), or at
     *   either end of the range, where the children in view are the first
     *   or the last items (a Compose lazy list names no item, and mid-list
     *   its position stays unknown). Each way needs every child, so a
     *   collection that lost one reports no items in view.
     * - Otherwise a range info, as a percentage of its range.
     */
    fun scroll(n: AccessibilityNodeInfo, children: List<AccessibilityNodeInfo>, complete: Boolean): Scroll {
        var backward = false
        var forward = false
        var upOrLeft = false
        var downOrRight = false
        var vertical = false
        var horizontal = false
        for (a in n.actionList) {
            val id = a.id
            when {
                id == AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD -> backward = true
                id == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD -> forward = true
                id in SCROLL_BACK -> upOrLeft = true
                id in SCROLL_FORWARD -> downOrRight = true
            }
            if (id in VERTICAL) vertical = true
            if (id in HORIZONTAL) horizontal = true
        }
        val logical = backward || forward
        val back = if (logical) backward else upOrLeft
        val ahead = if (logical) forward else downOrRight
        val collection = n.collectionInfo
        val axis = when {
            vertical != horizontal -> if (vertical) ScrollAxis.VERTICAL else ScrollAxis.HORIZONTAL
            vertical -> ScrollAxis.UNKNOWN
            collection != null && collection.columnCount == 1 && collection.rowCount != 1 -> ScrollAxis.VERTICAL
            collection != null && collection.rowCount == 1 && collection.columnCount != 1 -> ScrollAxis.HORIZONTAL
            else -> ScrollAxis.UNKNOWN
        }
        val flags = (if (back) ScrollFlag.BACK else 0) or (if (ahead) ScrollFlag.FORWARD else 0)
        if (collection != null) {
            val byRows = when (axis) {
                ScrollAxis.VERTICAL -> true
                ScrollAxis.HORIZONTAL -> false
                else -> collection.columnCount == 1
            }
            val count = if (byRows) collection.rowCount else collection.columnCount
            if (count > 0) {
                val inView = if (complete) itemsInView(count, byRows, back, ahead, children) else null
                return Scroll(axis, flags, ScrollRange(ScrollUnit.ITEMS, 0, count - 1), inView)
            }
        }
        val info = n.rangeInfo
        if (info != null && info.type != AccessibilityNodeInfo.RangeInfo.RANGE_TYPE_INDETERMINATE && info.max > info.min) {
            val percent = ((info.current - info.min) * 100f / (info.max - info.min)).roundToInt().coerceIn(0, 100)
            return Scroll(axis, flags, ScrollRange(ScrollUnit.PERCENT, 0, 100), ScrollPosition(percent, percent))
        }
        return Scroll(axis, flags)
    }

    private fun itemsInView(
        count: Int,
        byRows: Boolean,
        back: Boolean,
        ahead: Boolean,
        children: List<AccessibilityNodeInfo>,
    ): ScrollPosition? {
        val inView = children.filter { it.isVisibleToUser }
        if (inView.isEmpty()) return null
        if (inView.all { it.collectionItemInfo != null }) {
            var first = Int.MAX_VALUE
            var last = Int.MIN_VALUE
            for (c in inView) {
                val item = c.collectionItemInfo
                val index = if (byRows) item.rowIndex else item.columnIndex
                val span = if (byRows) item.rowSpan else item.columnSpan
                first = minOf(first, index)
                last = maxOf(last, index + maxOf(span, 1) - 1)
            }
            return ScrollPosition(first.coerceIn(0, count - 1), last.coerceIn(0, count - 1))
        }
        if (children.size == count) {
            return ScrollPosition(children.indexOfFirst { it.isVisibleToUser }, children.indexOfLast { it.isVisibleToUser })
        }
        return when {
            !back -> ScrollPosition(0, minOf(inView.size, count) - 1)
            !ahead -> ScrollPosition(maxOf(0, count - inView.size), count - 1)
            else -> null
        }
    }
}
