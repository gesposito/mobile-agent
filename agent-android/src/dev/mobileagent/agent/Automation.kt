package dev.mobileagent.agent

import android.accessibilityservice.AccessibilityServiceInfo
import android.app.UiAutomation
import android.os.HandlerThread
import android.os.Looper
import android.os.SystemClock
import java.lang.reflect.InvocationTargetException

/** Another process holds the UI automation role; the message is the platform's. */
class OwnershipConflict(message: String) : Exception(message)

/**
 * The UI automation role, taken as `uiautomator` takes it (design D2): a
 * reflective `UiAutomationConnection()` handed to the hidden
 * `UiAutomation(Looper, IUiAutomationConnection)` constructor, then the hidden
 * `connectWithTimeout`. Accessibility events arrive on [callbacks]. Validated
 * on API 36 (spike S1); every member exists from Android 11 to 17.
 */
class Automation private constructor(
    @JvmField val uia: UiAutomation,
    private val callbacks: HandlerThread,
    /** Time the connection took, in ms. */
    @JvmField val connectMs: Long,
) {
    companion object {
        private const val CONNECT_TIMEOUT_MS = 5000L

        /**
         * Connects once. Throws [OwnershipConflict] when another process holds
         * the role (the platform refuses with `already registered!`).
         */
        fun connect(): Automation {
            @Suppress("DEPRECATION")
            if (Looper.getMainLooper() == null) Looper.prepareMainLooper()
            val callbacks = HandlerThread("a11y-callbacks").apply { start() }
            val t0 = SystemClock.uptimeMillis()
            try {
                val connection = Class.forName("android.app.UiAutomationConnection")
                    .getDeclaredConstructor().newInstance()
                val iface = Class.forName("android.app.IUiAutomationConnection")
                val uia = UiAutomation::class.java.getDeclaredConstructor(Looper::class.java, iface)
                    .newInstance(callbacks.looper, connection) as UiAutomation
                // Other accessibility services, such as a screen reader, keep running.
                val flags = UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES
                try {
                    UiAutomation::class.java.getDeclaredMethod(
                        "connectWithTimeout",
                        Int::class.javaPrimitiveType,
                        Long::class.javaPrimitiveType,
                    ).invoke(uia, flags, CONNECT_TIMEOUT_MS)
                } catch (e: NoSuchMethodException) {
                    UiAutomation::class.java.getDeclaredMethod("connect", Int::class.javaPrimitiveType).invoke(uia, flags)
                }
                return Automation(uia, callbacks, SystemClock.uptimeMillis() - t0)
            } catch (e: InvocationTargetException) {
                callbacks.quitSafely()
                val cause = e.targetException
                if (cause is IllegalStateException && cause.message?.contains("already registered") == true) {
                    throw OwnershipConflict(cause.message ?: "already registered")
                }
                throw cause
            } catch (t: Throwable) {
                callbacks.quitSafely()
                throw t
            }
        }
    }

    /**
     * Adds FLAG_RETRIEVE_INTERACTIVE_WINDOWS, which the connection does not
     * set at registration, so the window list covers every interactive
     * window, the keyboard and system overlays included.
     */
    fun enableInteractiveWindows() {
        val info: AccessibilityServiceInfo = uia.serviceInfo
        info.flags = info.flags or AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        uia.serviceInfo = info
    }

    /**
     * Waits until the window list has an active window with a root: for tens
     * of milliseconds after the flag is set the list is empty or partial
     * (spike S1: up to 55 ms). Returns the ms waited, or -1 on timeout.
     */
    fun awaitActiveWindow(timeoutMs: Long): Long {
        val t0 = SystemClock.uptimeMillis()
        while (true) {
            val active = uia.windows.firstOrNull { it.isActive }
            if (active != null && active.root != null) return SystemClock.uptimeMillis() - t0
            if (SystemClock.uptimeMillis() - t0 >= timeoutMs) return -1
            Thread.sleep(5)
        }
    }

    /** Releases the role; the platform also restores the rotation state it recorded at connect. */
    fun disconnect() {
        try {
            UiAutomation::class.java.getDeclaredMethod("disconnect").invoke(uia)
        } catch (e: ReflectiveOperationException) {
            // The process is about to exit, which releases the role too.
        }
        callbacks.quitSafely()
    }
}
