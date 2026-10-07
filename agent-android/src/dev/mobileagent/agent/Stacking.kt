package dev.mobileagent.agent

import dev.mobileagent.agent.codec.Bounds

/**
 * Which of a parent's children are drawn over the siblings they overlap (task
 * 10.28), which their order in the tree does not say: a view group gives its
 * children to accessibility sorted by where they lie, top to bottom and then
 * left to right (`ViewGroup.addChildrenForAccessibility`), so on the API 36
 * emulator Bluesky's recent searches and typeahead, drawn over the Explore
 * page they span, come before it. The order a parent draws its children in
 * is a node field of its own (`AccessibilityNodeInfo.getDrawingOrder`, from
 * Android 7): the position among the siblings from 1 for views, counted
 * among those the tree holds, 0 for a view not yet laid out, and from 0 for
 * Compose, which keeps its children in that order.
 *
 * A child is on top when the user can see it, it has an area, and it overlaps
 * a sibling the user can see, with an area, and is drawn after every such
 * sibling. Siblings that only touch at an edge do not overlap, and two drawn
 * at the same place in the order cover neither. A child drawn over some
 * siblings and under another it overlaps is not on top, so its flag never
 * covers a sibling drawn after it; the mirror judges what an on-top node
 * covers by the bounds of what its siblings hold (`crates/mirror`).
 *
 * Calls no platform API, so codec-test.sh checks it on the host JVM.
 */
object Stacking {
    /**
     * Whether each child is on top. `order` is where the platform draws each
     * child among its siblings, `visible` whether it is visible to the user
     * and `bounds` its bounds on screen, in pixels. The children are swept by
     * their top edge, so only pairs that overlap from top to bottom are
     * compared: one row with each of its neighbours in a list.
     */
    fun onTop(order: IntArray, visible: BooleanArray, bounds: Array<Bounds>): BooleanArray {
        val n = order.size
        val overlaps = BooleanArray(n)
        val under = BooleanArray(n)
        val shown = (0 until n).filter { visible[it] && area(bounds[it]) }.sortedBy { bounds[it].top }
        for (a in shown.indices) {
            val i = shown[a]
            val bi = bounds[i]
            for (k in a + 1 until shown.size) {
                val j = shown[k]
                val bj = bounds[j]
                // Every later child starts at or below this one's bottom edge.
                if (bj.top >= bi.bottom) break
                if (bj.left >= bi.right || bi.left >= bj.right) continue
                overlaps[i] = true
                overlaps[j] = true
                if (order[j] >= order[i]) under[i] = true
                if (order[i] >= order[j]) under[j] = true
            }
        }
        return BooleanArray(n) { overlaps[it] && !under[it] }
    }

    private fun area(b: Bounds) = b.right > b.left && b.bottom > b.top
}
