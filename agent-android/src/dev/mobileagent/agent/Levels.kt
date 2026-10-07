package dev.mobileagent.agent

/**
 * The platform reads that exist only from some Android level on (task 10.3).
 * Each function takes the level and the read, so codec-test.sh checks the
 * gates on the host JVM without the platform. A level that has the read
 * takes it as it is; only an older one takes another path.
 */
object Levels {
    /** `AccessibilityEvent.getWindowChanges`, from Android 9. */
    const val WINDOW_CHANGES = 28

    /** `AccessibilityAction.ACTION_PAGE_UP` and the other page actions, from Android 10. */
    const val PAGE_ACTIONS = 29

    /**
     * What a windows change changed, `read` giving the platform's bits from
     * Android 9. Before it the event says only that the window list changed,
     * naming no window: 0, which stands for a state change.
     */
    inline fun windowChangeBits(sdk: Int, read: () -> Int): Int = if (sdk >= WINDOW_CHANGES) read() else 0

    /**
     * The ids of a scroll action, `scroll`, and of the page action `page`
     * gives, which moves the same way. Before Android 10 the platform has no
     * page actions and no node offers one, so only the scroll action's.
     */
    inline fun scrollIds(sdk: Int, scroll: Int, page: () -> Int): IntArray =
        if (sdk >= PAGE_ACTIONS) intArrayOf(scroll, page()) else intArrayOf(scroll)

    /**
     * The node cache of the UI automation connection `connectionId` in the
     * accessibility client class `client`: its hidden `getCache(connectionId)`
     * where the client keeps a cache per connection, as on API 36. Android 8.1
     * has no such method and keeps one cache for the process,
     * `sAccessibilityCache`, which is then the connection's. Throws a
     * [ReflectiveOperationException] when the client has neither.
     */
    fun nodeCache(client: Class<*>, connectionId: Int): Any? = try {
        client.getDeclaredMethod("getCache", Int::class.javaPrimitiveType).invoke(null, connectionId)
    } catch (e: NoSuchMethodException) {
        client.getDeclaredField("sAccessibilityCache").apply { isAccessible = true }.get(null)
    }
}
