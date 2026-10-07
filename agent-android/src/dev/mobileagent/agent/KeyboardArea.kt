package dev.mobileagent.agent

import dev.mobileagent.agent.codec.Bounds

/**
 * The part of the screen an input-method window covers (task 10.25).
 *
 * The platform gives a window the bounds of its touchable region
 * (`AccessibilityWindowInfo.getBoundsInScreen` is the bounds of
 * `getRegionInScreen`), and an input method can take touches over pieces of
 * the screen far apart: on the API 36 emulator Gboard's floating toolbar is a
 * pill at the left of the screen, 898 to 1588 pixels down, and its region adds
 * the navigation bar the input method draws across the bottom edge, a
 * transparent strip with the gesture handle, so the box around both spans the
 * screen from the toolbar down, over content the input method leaves in view.
 * What the window covers is the pieces that hold what it shows: a piece holds
 * a node visible to the user that lies within it. A docked keyboard's region
 * is one piece, its keys and the navigation bar below them, which it keeps
 * whole although no node of its own lies in that navigation bar.
 *
 * Calls no platform API, so codec-test.sh checks it on the host JVM.
 */
object KeyboardArea {
    /**
     * The box around the pieces of an input-method window's touchable region,
     * `pieces`, that hold one of `shown`, the bounds of its nodes visible to
     * the user, all in screen pixels. Null when no piece holds one, or without
     * a region (before Android 11), and the window keeps the platform's bounds.
     */
    fun of(pieces: List<Bounds>, shown: List<Bounds>): Bounds? {
        var area: Bounds? = null
        for (piece in pieces) {
            if (shown.none { holds(piece, it) }) continue
            area = area?.let { union(it, piece) } ?: piece
        }
        return area
    }

    /** Whether `node` has an area and lies within `piece`. */
    private fun holds(piece: Bounds, node: Bounds): Boolean =
        node.right > node.left && node.bottom > node.top &&
            node.left >= piece.left && node.top >= piece.top &&
            node.right <= piece.right && node.bottom <= piece.bottom

    private fun union(a: Bounds, b: Bounds) =
        Bounds(minOf(a.left, b.left), minOf(a.top, b.top), maxOf(a.right, b.right), maxOf(a.bottom, b.bottom))
}
