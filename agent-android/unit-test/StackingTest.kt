package dev.mobileagent.agent

import dev.mobileagent.agent.codec.Bounds
import kotlin.system.exitProcess

/**
 * Checks of [Stacking] on the siblings the API 36 emulator showed under
 * Bluesky's search screen and elsewhere (task 10.28), in screen pixels on its
 * 1080 by 2400 display, run by agent-android/codec-test.sh on the host JVM:
 *
 *     StackingTestKt
 *
 * Each parent's children are given in the order the platform lists them,
 * with the order it draws them in (`AccessibilityNodeInfo.getDrawingOrder`),
 * whether each is visible to the user, and its bounds. Prints one PASS or
 * FAIL line per check and exits 1 if any failed.
 */
private var passed = 0
private var failed = 0

private fun check(name: String, body: () -> String?) {
    val problem = try {
        body()
    } catch (t: Throwable) {
        t.toString()
    }
    if (problem == null) {
        passed++
        println("PASS $name")
    } else {
        failed++
        println("FAIL $name: $problem")
    }
}

private fun b(left: Int, top: Int, right: Int, bottom: Int) = Bounds(left, top, right, bottom)

/** One child: where it is drawn among its siblings, whether it is visible, its bounds. */
private class Child(val order: Int, val visible: Boolean, val bounds: Bounds)

private fun child(order: Int, bounds: Bounds, visible: Boolean = true) = Child(order, visible, bounds)

/** Null when [Stacking.onTop] puts on top exactly the children `expected` holds true for. */
private fun expect(children: List<Child>, vararg expected: Boolean): String? {
    val actual = Stacking.onTop(
        IntArray(children.size) { children[it].order },
        BooleanArray(children.size) { children[it].visible },
        Array(children.size) { children[it].bounds },
    )
    return if (actual.contentEquals(expected)) null else "expected ${expected.toList()}, got ${actual.toList()}"
}

fun main() {
    check("an overlay drawn over the page it spans is on top, though the platform lists it first") {
        // Bluesky's recent searches and typeahead: a view group lists its children sorted by where
        // they lie, so the overlay comes before the Explore page it covers; only the order the
        // platform draws them in says which shows. The screen's own view spans the screen and is
        // drawn first; the header is drawn last and only touches the overlay at its bottom edge.
        expect(
            listOf(
                child(1, b(0, 0, 1080, 2400)),
                child(4, b(0, 132, 1080, 406)),
                child(3, b(0, 406, 1080, 2400)),
                child(2, b(0, 406, 1080, 2400)),
            ),
            false, true, true, false,
        )
    }
    check("with the overlay gone, the page is on top of the screen's own view") {
        // The Explore page alone, drawn after the screen's own view, which spans the screen.
        expect(
            listOf(child(1, b(0, 0, 1080, 2400)), child(3, b(0, 132, 1080, 406)), child(2, b(0, 406, 1080, 2400))),
            false, true, true,
        )
    }
    check("siblings that only touch at an edge do not overlap") {
        expect(listOf(child(4, b(0, 132, 1080, 406)), child(3, b(0, 406, 1080, 2400))), false, false)
    }
    check("a sibling drawn later over part of a node keeps it from being on top of the rest") {
        expect(
            listOf(child(1, b(0, 0, 100, 100)), child(2, b(0, 0, 100, 100)), child(3, b(50, 50, 150, 150))),
            false, false, true,
        )
    }
    check("a sibling the user cannot see, or without an area, overlaps nothing") {
        // The drawer's backdrop, drawn last and transparent while the drawer is closed, over the
        // screen and the tab bar drawn over the screen's bottom; a row scrolled out of its list,
        // whose bounds Android clips to nothing.
        expect(
            listOf(
                child(3, b(0, 0, 1080, 2400), visible = false),
                child(1, b(0, 0, 1080, 2400)),
                child(2, b(0, 2213, 1080, 2400)),
                child(4, b(0, 5161, 1080, 2400)),
            ),
            false, false, true, false,
        )
    }
    check("siblings drawn at the same place in the order are neither on top") {
        // Views not yet laid out all report 0.
        expect(listOf(child(0, b(0, 0, 100, 100)), child(0, b(50, 50, 150, 150))), false, false)
    }
    check("Compose's orders, which start at 0, count as any other") {
        // A navigation bar item's indicator and the label drawn over it.
        expect(listOf(child(0, b(37, 2137, 163, 2263)), child(1, b(37, 2190, 163, 2316))), false, true)
    }
    check("a lone child or none is on top of nothing") {
        expect(listOf(child(1, b(0, 0, 1080, 2400))), false) ?: expect(emptyList())
    }
    println("$passed passed, $failed failed")
    exitProcess(if (failed == 0) 0 else 1)
}
