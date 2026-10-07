package dev.mobileagent.agent

import kotlin.system.exitProcess

/**
 * Checks of [Levels], the platform reads that exist only from some Android
 * level on, on the levels of the API 27 Nexus 5X and the API 36 emulator
 * (task 10.3), run by agent-android/codec-test.sh on the host JVM:
 *
 *     LevelsTestKt
 *
 * A read the level lacks is given as one that throws what Android 8.1 threw
 * for it, and the accessibility client as classes holding its cache the way
 * each level does. Prints one PASS or FAIL line per check and exits 1 if any
 * failed.
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

private fun expect(expected: IntArray, actual: IntArray): String? =
    if (expected.contentEquals(actual)) null else "expected ${expected.toList()}, got ${actual.toList()}"

/** A read Android 8.1 does not have, failing as it fails there. */
private fun missingMethod(): Int = throw NoSuchMethodError("No virtual method getWindowChanges()I in class AccessibilityEvent")

private fun missingField(): Int = throw NoSuchFieldError("No static field ACTION_PAGE_UP in class AccessibilityAction")

/** An accessibility client with one node cache per connection, as on API 36. */
object PerConnectionClient {
    val caches = mapOf(7 to "the cache of connection 7")

    @JvmStatic
    fun getCache(connectionId: Int): Any? = caches[connectionId]
}

/** An accessibility client with one node cache for the process, as on Android 8.1. */
object ProcessClient {
    private val sAccessibilityCache: Any = "the process's cache"

    fun cache(): Any = sAccessibilityCache
}

/** An accessibility client with neither. */
object NoCacheClient

/** Ids of `ACTION_SCROLL_UP` and `ACTION_PAGE_UP` on API 36. */
private const val SCROLL_UP = 16908344
private const val PAGE_UP = 16908358

fun main() {
    check("before Android 9 a windows change reads no change bits and stands for a state change") {
        val bits = Levels.windowChangeBits(27) { missingMethod() }
        if (bits == 0) null else "expected 0, got $bits"
    }
    check("from Android 9 a windows change carries the platform's bits") {
        val bits = Levels.windowChangeBits(28) { 0x5 } or Levels.windowChangeBits(36) { 0x5 }
        if (bits == 0x5) null else "expected 0x5, got $bits"
    }
    check("before Android 10 a scroll action comes without a page action") {
        expect(intArrayOf(SCROLL_UP), Levels.scrollIds(27, SCROLL_UP) { missingField() }) ?:
            expect(intArrayOf(SCROLL_UP), Levels.scrollIds(28, SCROLL_UP) { missingField() })
    }
    check("from Android 10 a scroll action comes with the page action that moves the same way") {
        expect(intArrayOf(SCROLL_UP, PAGE_UP), Levels.scrollIds(29, SCROLL_UP) { PAGE_UP }) ?:
            expect(intArrayOf(SCROLL_UP, PAGE_UP), Levels.scrollIds(36, SCROLL_UP) { PAGE_UP })
    }
    check("a client with a cache per connection gives the connection's") {
        val cache = Levels.nodeCache(PerConnectionClient::class.java, 7)
        if (cache === PerConnectionClient.caches[7]) null else "got $cache"
    }
    check("a client with one cache for the process, as on Android 8.1, gives that cache") {
        val cache = Levels.nodeCache(ProcessClient::class.java, 7)
        if (cache === ProcessClient.cache()) null else "got $cache"
    }
    check("a client with neither throws, so the caller reads no cache") {
        try {
            "got ${Levels.nodeCache(NoCacheClient::class.java, 7)}"
        } catch (e: ReflectiveOperationException) {
            null
        }
    }
    println("$passed passed, $failed failed")
    if (failed > 0) exitProcess(1)
}
