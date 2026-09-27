package dev.mobileagent.agent.codec

/**
 * The golden sample set of crates/protocol/tests/common/samples.rs, built
 * independently in Kotlin, name for name. Unsigned 32-bit values are carried
 * in Int and 64-bit ones in Long as their bit patterns: -1 is u32::MAX or
 * u64::MAX.
 */
object Samples {
    private fun ok(id: Int, method: Int, reply: Reply) = Response(id, method, reply, null)

    private fun err(id: Int, method: Int, code: String, message: String, retryable: Boolean) =
        Response(id, method, null, WireError(code, message, retryable))

    private val snapshot = Snapshot(
        strings = arrayOf(
            "android.widget.FrameLayout",
            "android.widget.TextView",
            "android.widget.Button",
            "dev.placeholder.fixture:id/",
            "dev.placeholder.fixture",
        ),
        windows = arrayOf(
            Window(
                id = 12,
                type = WindowType.APPLICATION,
                layer = 0,
                packageIndex = 4,
                title = "Fixture",
                bounds = Bounds(0, 0, 1080, 2400),
                flags = WindowFlag.ACTIVE or WindowFlag.FOCUSED,
                root = Node(
                    id = 0,
                    cls = 0,
                    resPkg = null,
                    resId = null,
                    text = null,
                    desc = null,
                    bounds = Bounds(0, 0, 1080, 2400),
                    flags = NodeFlag.ENABLED or NodeFlag.VISIBLE,
                    children = arrayOf(
                        Node(
                            id = 1,
                            cls = 1,
                            resPkg = 3,
                            resId = "title",
                            text = "Flights \u2708 caf\u00e9 \u65e5\u672c \uD83D\uDE42",
                            desc = null,
                            bounds = Bounds(48, -7, 860, 53),
                            flags = NodeFlag.ENABLED or NodeFlag.VISIBLE,
                            children = Node.NO_CHILDREN,
                        ),
                        Node(
                            id = 2,
                            cls = 2,
                            resPkg = 3,
                            resId = "submit",
                            text = "Submit",
                            desc = "Submit the form",
                            bounds = Bounds(Int.MIN_VALUE, -257, -25, Int.MAX_VALUE),
                            flags = NodeFlag.CLICKABLE or NodeFlag.FOCUSABLE or NodeFlag.ENABLED or NodeFlag.VISIBLE,
                            children = arrayOf(
                                Node(
                                    id = 3,
                                    cls = 1,
                                    resPkg = null,
                                    resId = null,
                                    text = "",
                                    desc = null,
                                    bounds = Bounds(-24, 23, 24, 255),
                                    flags = 1 shl 31,
                                    children = Node.NO_CHILDREN,
                                ),
                            ),
                        ),
                    ),
                ),
            ),
            Window(
                id = 13,
                type = WindowType.INPUT_METHOD,
                layer = -2,
                packageIndex = null,
                title = null,
                bounds = Bounds(0, 1600, 1080, 2400),
                flags = 0,
                root = null,
            ),
        ),
        nodeCount = 4,
        truncated = 0,
        seq = 41,
    )

    private const val VISIBLE = NodeFlag.ENABLED or NodeFlag.VISIBLE

    /** A node without text, children or scroll semantics. */
    private fun plain(id: Int, cls: Int, bounds: Bounds, flags: Int) =
        Node(id, cls, null, null, null, null, bounds, flags, Node.NO_CHILDREN)

    /** Window metadata in every state: change events yes, no and unknown; a system owner or none; modal or not. */
    private val windowMetaSnapshot = Snapshot(
        strings = arrayOf(
            "android.widget.FrameLayout",
            "com.android.systemui",
            "com.google.android.permissioncontroller",
            "dev.placeholder.fixture",
        ),
        windows = arrayOf(
            Window(
                id = 30,
                type = WindowType.SYSTEM,
                layer = 1,
                packageIndex = 1,
                title = null,
                bounds = Bounds(0, 0, 1080, 132),
                flags = 0,
                root = plain(0, 0, Bounds(0, 0, 1080, 132), VISIBLE),
                emitsEvents = true,
                ownerIndex = 1,
                modal = false,
            ),
            Window(
                id = 31,
                type = WindowType.APPLICATION,
                layer = 0,
                packageIndex = 2,
                title = "Allow Fixture to access this device\u2019s location?",
                bounds = Bounds(28, 502, 1052, 1966),
                flags = WindowFlag.ACTIVE or WindowFlag.FOCUSED,
                root = plain(1, 0, Bounds(28, 502, 1052, 1966), VISIBLE),
                emitsEvents = null,
                ownerIndex = 2,
                modal = true,
            ),
            Window(
                id = 32,
                type = WindowType.APPLICATION,
                layer = -1,
                packageIndex = 3,
                title = "Fixture",
                bounds = Bounds(0, 0, 1080, 2400),
                flags = 0,
                root = plain(2, 0, Bounds(0, 0, 1080, 2400), VISIBLE),
                emitsEvents = false,
                ownerIndex = null,
                modal = false,
            ),
        ),
        nodeCount = 3,
        truncated = 0,
        seq = 7,
    )

    private fun scroller(id: Int, cls: Int, scroll: Scroll, children: Array<Node> = Node.NO_CHILDREN, resId: String? = null) =
        Node(
            id, cls, null, resId, null, null, Bounds(0, 132, 1080, 2127),
            VISIBLE or NodeFlag.SCROLLABLE, children, scroll,
        )

    /**
     * Scroll semantics in every shape: an item range with the items in view, a range without a
     * position, scroll actions alone, a percent range, and values this version does not define
     * with integers at head boundaries.
     */
    private val scrollSnapshot = Snapshot(
        strings = arrayOf(
            "android.view.View",
            "android.widget.ScrollView",
            "android.widget.HorizontalScrollView",
            "android.widget.Button",
        ),
        windows = arrayOf(
            Window(
                id = 40,
                type = WindowType.APPLICATION,
                layer = 0,
                packageIndex = null,
                title = null,
                bounds = Bounds(0, 0, 1080, 2400),
                flags = WindowFlag.ACTIVE,
                root = Node(
                    0, 0, null, null, null, null, Bounds(0, 0, 1080, 2400), VISIBLE,
                    arrayOf(
                        scroller(
                            1,
                            0,
                            Scroll(
                                ScrollAxis.VERTICAL,
                                ScrollFlag.FORWARD,
                                ScrollRange(ScrollUnit.ITEMS, 0, 199),
                                ScrollPosition(0, 15),
                            ),
                            arrayOf(
                                Node(2, 3, null, null, "Row 1", null, Bounds(0, 132, 1080, 258), VISIBLE, Node.NO_CHILDREN),
                            ),
                            resId = "row-list",
                        ),
                        scroller(
                            3,
                            2,
                            Scroll(
                                ScrollAxis.HORIZONTAL,
                                ScrollFlag.BACK or ScrollFlag.FORWARD,
                                ScrollRange(ScrollUnit.ITEMS, 0, 65535),
                            ),
                        ),
                        scroller(4, 1, Scroll(ScrollAxis.VERTICAL, ScrollFlag.BACK)),
                        scroller(
                            5,
                            0,
                            Scroll(ScrollAxis.UNKNOWN, 0, ScrollRange(ScrollUnit.PERCENT, 0, 100), ScrollPosition(37, 37)),
                        ),
                        scroller(
                            6,
                            0,
                            Scroll(200, 1 shl 31, ScrollRange(24, -24, Int.MAX_VALUE), ScrollPosition(-25, 256)),
                        ),
                    ),
                ),
                emitsEvents = false,
                ownerIndex = null,
                modal = false,
            ),
        ),
        nodeCount = 7,
        truncated = 0,
        seq = 8,
    )

    /** Every sample, by golden file name. */
    val ALL: List<Pair<String, Message>> = listOf(
        "hello" to Hello(
            versions = intArrayOf(1, 2),
            capabilities = Capability.TREE or Capability.INPUT or Capability.SCREENSHOT or
                Capability.EVENTS or Capability.TEXT_SET,
            maxFrame = 16 * 1024 * 1024,
            agent = "mobile-agent-android 0.1.0",
            token = "placeholder-token",
        ),
        "hello_minimal" to Hello(intArrayOf(1), 0, 4096, "", null),
        "welcome" to Welcome(intArrayOf(1), 1, 16 * 1024 * 1024, null),
        "welcome_refused" to Welcome(
            versions = intArrayOf(3),
            version = null,
            maxFrame = 65536,
            error = WireError(
                ErrorCode.VERSION_MISMATCH,
                "no common protocol version: the daemon speaks [3] and the agent speaks [1]; redeploy the agent that matches this daemon",
                false,
            ),
        ),
        "request_snapshot" to Request(1, 200, SnapshotCall(maxDepth = 64, maxNodes = 5000, idleMs = 0)),
        "request_snapshot_defaults" to Request(-1, null, SnapshotCall()),
        "request_screenshot" to Request(
            2,
            65536,
            ScreenshotCall(format = ImageFormat.JPEG, quality = 80, maxDimension = 1280),
        ),
        "request_tap" to Request(3, 1000, TapCall(540, 1200)),
        "request_long_press" to Request(23, null, TapCall(0, -1, durationMs = 600)),
        "request_swipe" to Request(24, null, SwipeCall(-24, -25, Int.MIN_VALUE, Int.MAX_VALUE, 300)),
        "request_key" to Request(255, null, KeyCall(66, meta = 0x1000)),
        "request_text_set" to Request(
            256,
            null,
            TextSetCall(
                "h\u00e9llo w\u00f6rld \u2013 \u2713 \u65e5\u672c \uD83D\uDE42\n\t\"quoted\"",
                node = 42,
                mode = TextMethod.DIRECT,
            ),
        ),
        "request_frames_start" to Request(65535, null, FramesStartCall(maxFps = 30)),
        "request_frames_stop" to Request(65536, null, FramesStopCall),
        "request_resync" to Request(7, null, ResyncCall(Stream.TREE)),
        "request_shutdown" to Request(8, 0, ShutdownCall),
        "request_unknown_method" to Request(9, null, UnknownCall(999)),
        "response_snapshot" to ok(1, Method.SNAPSHOT, snapshot),
        "response_snapshot_truncated" to ok(
            10,
            Method.SNAPSHOT,
            Snapshot(
                strings = arrayOf(),
                windows = arrayOf(),
                nodeCount = 0,
                truncated = Truncation.DEPTH or Truncation.NODES or Truncation.WINDOWS,
                seq = -1L,
            ),
        ),
        "response_snapshot_window_meta" to ok(11, Method.SNAPSHOT, windowMetaSnapshot),
        "response_snapshot_scroll" to ok(12, Method.SNAPSHOT, scrollSnapshot),
        "response_screenshot" to ok(
            2,
            Method.SCREENSHOT,
            Screenshot(ImageFormat.PNG, 1080, 2400, Rotation.R90, ByteArray(300) { (it % 256).toByte() }),
        ),
        "response_tap" to ok(3, Method.TAP, InputAck(true)),
        "response_key_rejected" to ok(255, Method.KEY, InputAck(false)),
        "response_text_set" to ok(256, Method.TEXT_SET, TextAck(TextMethod.KEYS)),
        "response_resync" to ok(7, Method.RESYNC, ResyncAck(Stream.TREE, 1L shl 32)),
        "response_frames_start" to ok(65535, Method.FRAMES_START, EmptyReply),
        "response_error_window_changed" to err(
            1,
            Method.SNAPSHOT,
            ErrorCode.WINDOW_CHANGED,
            "the foreground window changed during acquisition",
            true,
        ),
        "response_error_unknown_method" to err(
            9,
            999,
            ErrorCode.UNKNOWN_METHOD,
            "999 is not a method this agent implements",
            false,
        ),
        "response_error_frame_too_large" to err(
            2,
            Method.SCREENSHOT,
            ErrorCode.FRAME_TOO_LARGE,
            "the screenshot message for request 2 is 20000000 bytes, over the negotiated maximum frame size of 16777216 bytes",
            false,
        ),
        "event_tree" to Event(
            1,
            5_000_000_000L,
            TreeChange(
                TreeChangeKind.CONTENT,
                12,
                intArrayOf(0, 3, 1, 23, 24, 255, 256, 65535, 65536, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, -1),
            ),
        ),
        "event_window" to Event(2, 0, WindowChange(WindowChangeKind.ADDED, 13)),
        "event_frame" to Event(-1L, 123_456, FrameAvailable(1080, 2400, Rotation.R270)),
        "event_lifecycle" to Event(
            3,
            1,
            Lifecycle(
                LifecycleKind.OWNERSHIP_LOST,
                "The UI automation connection was taken by another process. ".repeat(5),
            ),
        ),
        "event_unknown_stream" to Event(4, 2, UnknownPayload(200)),
        "ping" to Ping(0),
        "pong" to Pong(-1L),
    )
}
