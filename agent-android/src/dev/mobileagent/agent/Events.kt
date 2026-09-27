package dev.mobileagent.agent

import android.os.SystemClock
import android.view.accessibility.AccessibilityEvent

/**
 * Accessibility events from the UI automation connection. They arrive on
 * the connection's callback thread after the platform's node cache has
 * processed them.
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
    }

    /** [SystemClock.uptimeMillis] of the last event, 0 before the first. */
    @Volatile var lastEvent = 0L
        private set

    fun start() {
        automation.uia.setOnAccessibilityEventListener { e -> onEvent(e) }
    }

    private fun onEvent(@Suppress("UNUSED_PARAMETER") e: AccessibilityEvent) {
        lastEvent = SystemClock.uptimeMillis()
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
}
