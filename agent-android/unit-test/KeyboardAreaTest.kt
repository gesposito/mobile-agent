package dev.mobileagent.agent

import dev.mobileagent.agent.codec.Bounds
import kotlin.system.exitProcess

/**
 * Checks of [KeyboardArea] on the input-method windows the API 36 emulator
 * showed (task 10.25), in screen pixels on its 1080 by 2400 display, run by
 * agent-android/codec-test.sh on the host JVM:
 *
 *     KeyboardAreaTestKt
 *
 * Each window is given as its touchable region's rectangles, as
 * `AccessibilityWindowInfo.getRegionInScreen` returned them, and the bounds
 * of its nodes visible to the user, as the agent's walk read them. Prints one
 * PASS or FAIL line per check and exits 1 if any failed.
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

private fun show(b: Bounds?): String = if (b == null) "null" else "(${b.left},${b.top})-(${b.right},${b.bottom})"

/** Null when `actual` is `expected`, else both. */
private fun expect(expected: Bounds?, actual: Bounds?): String? {
    val same = expected?.let { e ->
        actual != null && e.left == actual.left && e.top == actual.top && e.right == actual.right && e.bottom == actual.bottom
    } ?: (actual == null)
    return if (same) null else "expected ${show(expected)}, got ${show(actual)}"
}

private fun b(left: Int, top: Int, right: Int, bottom: Int) = Bounds(left, top, right, bottom)

/** The containers of Gboard's window that span its root, from the status bar down. */
private val spanning = List(5) { b(0, 132, 1080, 2400) }

/** Gboard's floating toolbar: the pill at the left and its five buttons and divider. */
private val toolbar = listOf(
    b(21, 898, 1080, 2400),
    b(21, 898, 169, 1588), b(21, 898, 169, 1588), b(21, 898, 169, 1588),
    b(32, 908, 158, 1034), b(37, 913, 153, 1029),
    b(32, 1039, 158, 1175), b(32, 1044, 158, 1170), b(37, 1049, 153, 1165),
    b(32, 1175, 158, 1583),
    b(32, 1180, 158, 1306), b(37, 1185, 153, 1301),
    b(32, 1316, 158, 1442), b(37, 1321, 153, 1437),
    b(32, 1452, 158, 1578), b(37, 1457, 153, 1573),
    b(21, 1240, 147, 1245),
)

/** The toolbar and the navigation bar the input method draws, a transparent strip with the gesture handle. */
private val toolbarRegion = listOf(b(21, 898, 169, 1588), b(0, 2337, 1080, 2400))

fun main() {
    check("a floating toolbar covers the toolbar, not the box around it and the navigation bar") {
        // Gboard's stylus toolbar over Bluesky's search field and the Expo fixture's username
        // field: the platform's bounds are (0,898)-(1080,2400), so a typeahead row at 487 to 543
        // points, (0,1279)-(1080,1426), counted as hidden.
        expect(b(21, 898, 169, 1588), KeyboardArea.of(toolbarRegion, spanning + toolbar))
    }
    check("the emoji panel beside the toolbar covers both, not the navigation bar") {
        // Gboard's emoji panel, opened from the toolbar. Its input area spans the window
        // without children and holds no piece; the region's bands around the toolbar each
        // hold some of the panel: its header, emoji, and the row of categories below them.
        val region = listOf(
            b(186, 730, 1080, 898),
            b(21, 898, 169, 1588),
            b(186, 898, 1080, 1588),
            b(186, 1588, 1080, 1755),
            b(0, 2337, 1080, 2400),
        )
        val panel = listOf(
            b(186, 730, 1080, 2400), b(186, 730, 1080, 1755), b(186, 730, 1080, 1755),
            b(186, 751, 1080, 856), b(221, 775, 279, 832),
            b(197, 913, 294, 1010), b(294, 913, 391, 1010),
            b(186, 1666, 1080, 1755), b(538, 1666, 727, 1755),
        )
        expect(b(21, 730, 1080, 1755), KeyboardArea.of(region, spanning + b(0, 132, 1080, 2400) + panel + toolbar))
    }
    check("a docked keyboard covers its keys and the navigation bar below them") {
        // The keyboard Gboard shows for the Expo fixture's password field: one piece, whose
        // navigation bar below the keys (2274 to 2400) holds no node of its own.
        val keys = listOf(
            b(0, 1517, 1080, 2274), b(0, 1517, 1080, 2274),
            b(0, 1517, 1080, 1633), b(5, 1517, 112, 1633), b(47, 1530, 69, 1620),
        )
        expect(b(0, 1517, 1080, 2400), KeyboardArea.of(listOf(b(0, 1517, 1080, 2400)), spanning + keys))
    }
    check("a region whose pieces hold none of the window's nodes covers nothing it can tell") {
        expect(null, KeyboardArea.of(toolbarRegion, spanning))
    }
    check("a window without a region, as before Android 11, covers nothing it can tell") {
        expect(null, KeyboardArea.of(emptyList(), spanning + toolbar))
    }
    check("a node without an area holds no piece") {
        expect(null, KeyboardArea.of(toolbarRegion, spanning + b(21, 898, 21, 1588) + b(0, 2337, 1080, 2337)))
    }
    println("$passed passed, $failed failed")
    exitProcess(if (failed == 0) 0 else 1)
}
