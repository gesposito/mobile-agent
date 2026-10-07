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

        /** How long the window list must stay the same before the first read. */
        private const val STABLE_MS = 50L

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
     * The interactive windows. The platform call can return null for a
     * moment while its window list is replaced; that is retried briefly and
     * then read as no windows.
     */
    fun windows(): List<android.view.accessibility.AccessibilityWindowInfo> {
        for (attempt in 0 until 5) {
            @Suppress("UNNECESSARY_SAFE_CALL", "USELESS_ELVIS")
            val list: List<android.view.accessibility.AccessibilityWindowInfo>? = uia.windows
            if (list != null) return list
            Thread.sleep(10)
        }
        return emptyList()
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
     * Waits until the window list has an active window with a root and has
     * not changed for [STABLE_MS]: for tens of milliseconds after the flag is
     * set the list is empty or partial (spike S1: complete after 9 to 55 ms,
     * one window of two in half the attaches). Returns the ms waited, or -1
     * when no active window came within `timeoutMs`.
     */
    fun awaitActiveWindow(timeoutMs: Long): Long {
        val t0 = SystemClock.uptimeMillis()
        var activeAt = -1L
        var ids = emptyList<Int>()
        var stableSince = t0
        while (true) {
            val now = SystemClock.uptimeMillis()
            val windows = windows()
            val current = windows.map { it.id }.sorted()
            if (current != ids) {
                ids = current
                stableSince = now
            }
            if (activeAt < 0) {
                val active = windows.firstOrNull { it.isActive }
                if (active != null && active.root != null) activeAt = now
            }
            if (activeAt >= 0 && now - stableSince >= STABLE_MS) return now - t0
            if (now - t0 >= timeoutMs) return if (activeAt >= 0) now - t0 else -1
            Thread.sleep(5)
        }
    }

    private val connectionId: Int by lazy {
        UiAutomation::class.java.getDeclaredField("mConnectionId").apply { isAccessible = true }.getInt(uia)
    }

    /** The connection's node cache, a hidden object ([Levels.nodeCache]); null where it cannot be reached. */
    private fun cache(): Any? = try {
        Levels.nodeCache(Class.forName("android.view.accessibility.AccessibilityInteractionClient"), connectionId)
    } catch (e: ReflectiveOperationException) {
        null
    }

    /** Empties the connection's node cache, so the next read fetches every node. */
    fun clearCache() {
        if (android.os.Build.VERSION.SDK_INT >= 34) {
            uia.clearCache()
        } else {
            cache()?.let { it.javaClass.getDeclaredMethod("clear").invoke(it) }
        }
    }

    /**
     * Evicts the cached nodes of one window: its root, fetched afresh (roots
     * bypass the cache), and everything below it, through the cache's hidden
     * `clearSubTree`. Empties the whole cache where that is unavailable.
     */
    fun evictWindow(windowId: Int) {
        val root = windows().firstOrNull { it.id == windowId }?.root ?: return
        try {
            val cache = cache() ?: return clearCache()
            cache.javaClass.getDeclaredMethod("clearSubTree", android.view.accessibility.AccessibilityNodeInfo::class.java)
                .invoke(cache, root)
        } catch (e: ReflectiveOperationException) {
            clearCache()
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
