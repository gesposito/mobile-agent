//! The golden sample set: one message per kind, parameter and result type,
//! with integers at every CBOR head-length boundary, u32 and u64 values past
//! 2^31 and 2^63, multi-byte UTF-8, a byte string and an array longer than 23
//! elements, window metadata in every state, scroll semantics in every
//! shape, a node drawn over its siblings, and a bounds probe with its result.
//! `agent-android/codec-test/Samples.kt` builds the same values in Kotlin;
//! the two lists must stay in step, name for name.

use protocol::Error;
use protocol::event::{
    Event, EventPayload, FrameAvailable, Lifecycle, LifecycleKind, Stream, TreeChange,
    TreeChangeKind, WindowChange, WindowChangeKind,
};
use protocol::message::{
    BoundsParams, Call, FramesParams, Hello, ImageFormat, InputAck, KeyParams, Message, Method,
    NodeBounds, Ping, Pong, Probed, Reply, Request, Response, ResyncAck, ResyncParams, Rotation,
    Screenshot, ScreenshotParams, SnapshotParams, SwipeParams, TapParams, TextAck, TextMethod,
    TextParams, Welcome,
};
use protocol::snapshot::{
    Bounds, Node, NodeFlags, Scroll, ScrollAxis, ScrollFlags, ScrollPosition, ScrollRange,
    ScrollUnit, Snapshot, Truncation, Window, WindowFlags, WindowType,
};
use protocol::{Capabilities, code};

fn request(id: u32, deadline_ms: Option<u32>, call: Call) -> Message {
    Message::Request(Request {
        id,
        deadline_ms,
        call,
    })
}

fn ok(id: u32, method: Method, reply: Reply) -> Message {
    Message::Response(Response {
        id,
        method,
        result: Ok(reply),
    })
}

fn err(id: u32, method: Method, code: &str, message: &str, retryable: bool) -> Message {
    Message::Response(Response {
        id,
        method,
        result: Err(Error {
            code: code.to_owned(),
            message: message.to_owned(),
            retryable,
        }),
    })
}

fn event(seq: u64, time_us: u64, payload: EventPayload) -> Message {
    Message::Event(Event {
        seq,
        time_us,
        payload,
    })
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        strings: vec![
            "android.widget.FrameLayout".to_owned(),
            "android.widget.TextView".to_owned(),
            "android.widget.Button".to_owned(),
            "dev.placeholder.fixture:id/".to_owned(),
            "dev.placeholder.fixture".to_owned(),
        ],
        windows: vec![
            Window {
                id: 12,
                kind: WindowType::APPLICATION,
                layer: 0,
                package: Some(4),
                title: Some("Fixture".to_owned()),
                bounds: Bounds::new(0, 0, 1080, 2400),
                flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
                root: Some(Node {
                    id: 0,
                    class: 0,
                    res_pkg: None,
                    res_id: None,
                    text: None,
                    desc: None,
                    bounds: Bounds::new(0, 0, 1080, 2400),
                    flags: NodeFlags::ENABLED | NodeFlags::VISIBLE,
                    children: vec![
                        Node {
                            id: 1,
                            class: 1,
                            res_pkg: Some(3),
                            res_id: Some("title".to_owned()),
                            text: Some(
                                "Flights \u{2708} caf\u{e9} \u{65e5}\u{672c} \u{1f642}".to_owned(),
                            ),
                            desc: None,
                            bounds: Bounds::new(48, -7, 860, 53),
                            flags: NodeFlags::ENABLED | NodeFlags::VISIBLE,
                            children: vec![],
                            scroll: None,
                        },
                        Node {
                            id: 2,
                            class: 2,
                            res_pkg: Some(3),
                            res_id: Some("submit".to_owned()),
                            text: Some("Submit".to_owned()),
                            desc: Some("Submit the form".to_owned()),
                            bounds: Bounds::new(i32::MIN, -257, -25, i32::MAX),
                            flags: NodeFlags::CLICKABLE
                                | NodeFlags::FOCUSABLE
                                | NodeFlags::ENABLED
                                | NodeFlags::VISIBLE,
                            children: vec![Node {
                                id: 3,
                                class: 1,
                                res_pkg: None,
                                res_id: None,
                                text: Some(String::new()),
                                desc: None,
                                bounds: Bounds::new(-24, 23, 24, 255),
                                flags: NodeFlags(1 << 31),
                                children: vec![],
                                scroll: None,
                            }],
                            scroll: None,
                        },
                    ],
                    scroll: None,
                }),
                emits_events: None,
                owner: None,
                modal: false,
            },
            Window {
                id: 13,
                kind: WindowType::INPUT_METHOD,
                layer: -2,
                package: None,
                title: None,
                bounds: Bounds::new(0, 1600, 1080, 2400),
                flags: WindowFlags::EMPTY,
                root: None,
                emits_events: None,
                owner: None,
                modal: false,
            },
        ],
        node_count: 4,
        truncated: Truncation::EMPTY,
        seq: 41,
    }
}

/// A node without text, children or scroll semantics.
fn plain(id: u32, class: u32, bounds: Bounds, flags: NodeFlags) -> Node {
    Node {
        id,
        class,
        res_pkg: None,
        res_id: None,
        text: None,
        desc: None,
        bounds,
        flags,
        children: vec![],
        scroll: None,
    }
}

/// Window metadata in every state: whether the window sends change events
/// (yes, no, unknown), a system owner or none, modal or not.
pub fn window_meta_snapshot() -> Snapshot {
    let visible = NodeFlags::ENABLED | NodeFlags::VISIBLE;
    Snapshot {
        strings: vec![
            "android.widget.FrameLayout".to_owned(),
            "com.android.systemui".to_owned(),
            "com.google.android.permissioncontroller".to_owned(),
            "dev.placeholder.fixture".to_owned(),
        ],
        windows: vec![
            Window {
                id: 30,
                kind: WindowType::SYSTEM,
                layer: 1,
                package: Some(1),
                title: None,
                bounds: Bounds::new(0, 0, 1080, 132),
                flags: WindowFlags::EMPTY,
                root: Some(plain(0, 0, Bounds::new(0, 0, 1080, 132), visible)),
                emits_events: Some(true),
                owner: Some(1),
                modal: false,
            },
            Window {
                id: 31,
                kind: WindowType::APPLICATION,
                layer: 0,
                package: Some(2),
                title: Some("Allow Fixture to access this device\u{2019}s location?".to_owned()),
                bounds: Bounds::new(28, 502, 1052, 1966),
                flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
                root: Some(plain(1, 0, Bounds::new(28, 502, 1052, 1966), visible)),
                emits_events: None,
                owner: Some(2),
                modal: true,
            },
            Window {
                id: 32,
                kind: WindowType::APPLICATION,
                layer: -1,
                package: Some(3),
                title: Some("Fixture".to_owned()),
                bounds: Bounds::new(0, 0, 1080, 2400),
                flags: WindowFlags::EMPTY,
                root: Some(plain(2, 0, Bounds::new(0, 0, 1080, 2400), visible)),
                emits_events: Some(false),
                owner: None,
                modal: false,
            },
        ],
        node_count: 3,
        truncated: Truncation::EMPTY,
        seq: 7,
    }
}

/// A window the platform draws scaled while it animates the window in: its
/// bounds and its nodes' are those of a frame of that animation.
pub fn transformed_window_snapshot() -> Snapshot {
    Snapshot {
        strings: vec![
            "android.widget.FrameLayout".to_owned(),
            "dev.placeholder.fixture".to_owned(),
        ],
        windows: vec![Window {
            id: 40,
            kind: WindowType::APPLICATION,
            layer: 0,
            package: Some(1),
            title: Some("Fixture".to_owned()),
            bounds: Bounds::new(28, 62, 1052, 2338),
            flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED | WindowFlags::TRANSFORMED,
            root: Some(plain(
                0,
                0,
                Bounds::new(28, 62, 1052, 2338),
                NodeFlags::ENABLED | NodeFlags::VISIBLE,
            )),
            emits_events: Some(true),
            owner: None,
            modal: false,
        }],
        node_count: 1,
        truncated: Truncation::EMPTY,
        seq: 9,
    }
}

/// A node drawn over the sibling its bounds overlap, which comes after it in
/// the tree: an overlay over the page below a header, as a view group gives
/// its children to accessibility sorted by where they lie.
pub fn on_top_snapshot() -> Snapshot {
    let visible = NodeFlags::ENABLED | NodeFlags::VISIBLE;
    let mut root = plain(0, 0, Bounds::new(0, 0, 1080, 2400), visible);
    root.children = vec![
        plain(1, 0, Bounds::new(0, 132, 1080, 406), visible),
        plain(
            2,
            1,
            Bounds::new(0, 406, 1080, 2400),
            visible | NodeFlags::ON_TOP,
        ),
        plain(3, 0, Bounds::new(0, 406, 1080, 2400), visible),
    ];
    Snapshot {
        strings: vec![
            "android.view.ViewGroup".to_owned(),
            "android.widget.AbsListView".to_owned(),
        ],
        windows: vec![Window {
            id: 41,
            kind: WindowType::APPLICATION,
            layer: 0,
            package: None,
            title: None,
            bounds: Bounds::new(0, 0, 1080, 2400),
            flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
            root: Some(root),
            emits_events: Some(true),
            owner: None,
            modal: false,
        }],
        node_count: 4,
        truncated: Truncation::EMPTY,
        seq: 10,
    }
}

/// Scroll semantics in every shape: an item range with the items in view,
/// a range without a position, scroll actions alone, a percent range, and
/// values this version does not define with integers at head boundaries.
pub fn scroll_snapshot() -> Snapshot {
    let visible = NodeFlags::ENABLED | NodeFlags::VISIBLE;
    let scrolls = visible | NodeFlags::SCROLLABLE;
    let scroller = |id: u32, class: u32, scroll: Scroll| Node {
        scroll: Some(scroll),
        ..plain(id, class, Bounds::new(0, 132, 1080, 2127), scrolls)
    };
    let mut list = scroller(
        1,
        0,
        Scroll {
            axis: ScrollAxis::VERTICAL,
            flags: ScrollFlags::FORWARD,
            range: Some(ScrollRange {
                unit: ScrollUnit::ITEMS,
                min: 0,
                max: 199,
            }),
            position: Some(ScrollPosition { first: 0, last: 15 }),
        },
    );
    list.res_id = Some("row-list".to_owned());
    list.children = vec![Node {
        text: Some("Row 1".to_owned()),
        ..plain(2, 3, Bounds::new(0, 132, 1080, 258), visible)
    }];
    Snapshot {
        strings: vec![
            "android.view.View".to_owned(),
            "android.widget.ScrollView".to_owned(),
            "android.widget.HorizontalScrollView".to_owned(),
            "android.widget.Button".to_owned(),
        ],
        windows: vec![Window {
            id: 40,
            kind: WindowType::APPLICATION,
            layer: 0,
            package: None,
            title: None,
            bounds: Bounds::new(0, 0, 1080, 2400),
            flags: WindowFlags::ACTIVE,
            root: Some(Node {
                children: vec![
                    list,
                    scroller(
                        3,
                        2,
                        Scroll {
                            axis: ScrollAxis::HORIZONTAL,
                            flags: ScrollFlags::BACK | ScrollFlags::FORWARD,
                            range: Some(ScrollRange {
                                unit: ScrollUnit::ITEMS,
                                min: 0,
                                max: 65535,
                            }),
                            position: None,
                        },
                    ),
                    scroller(
                        4,
                        1,
                        Scroll {
                            axis: ScrollAxis::VERTICAL,
                            flags: ScrollFlags::BACK,
                            range: None,
                            position: None,
                        },
                    ),
                    scroller(
                        5,
                        0,
                        Scroll {
                            axis: ScrollAxis::UNKNOWN,
                            flags: ScrollFlags::EMPTY,
                            range: Some(ScrollRange {
                                unit: ScrollUnit::PERCENT,
                                min: 0,
                                max: 100,
                            }),
                            position: Some(ScrollPosition {
                                first: 37,
                                last: 37,
                            }),
                        },
                    ),
                    scroller(
                        6,
                        0,
                        Scroll {
                            axis: ScrollAxis(200),
                            flags: ScrollFlags(1 << 31),
                            range: Some(ScrollRange {
                                unit: ScrollUnit(24),
                                min: -24,
                                max: i32::MAX,
                            }),
                            position: Some(ScrollPosition {
                                first: -25,
                                last: 256,
                            }),
                        },
                    ),
                ],
                ..plain(0, 0, Bounds::new(0, 0, 1080, 2400), visible)
            }),
            emits_events: Some(false),
            owner: None,
            modal: false,
        }],
        node_count: 7,
        truncated: Truncation::EMPTY,
        seq: 8,
    }
}

/// Every sample, by golden file name.
pub fn samples() -> Vec<(&'static str, Message)> {
    let long_message = "The UI automation connection was taken by another process. ".repeat(5);
    vec![
        (
            "hello",
            Message::Hello(Hello {
                versions: vec![1, 2],
                capabilities: Capabilities::TREE
                    | Capabilities::INPUT
                    | Capabilities::SCREENSHOT
                    | Capabilities::EVENTS
                    | Capabilities::TEXT_SET,
                max_frame: 16 * 1024 * 1024,
                agent: "mobile-agent-android 0.1.0".to_owned(),
                token: Some("placeholder-token".to_owned()),
            }),
        ),
        (
            "hello_minimal",
            Message::Hello(Hello {
                versions: vec![1],
                capabilities: Capabilities::NONE,
                max_frame: 4096,
                agent: String::new(),
                token: None,
            }),
        ),
        (
            "welcome",
            Message::Welcome(Welcome {
                versions: vec![1],
                version: Some(1),
                max_frame: 16 * 1024 * 1024,
                error: None,
            }),
        ),
        (
            "welcome_refused",
            Message::Welcome(Welcome {
                versions: vec![3],
                version: None,
                max_frame: 65536,
                error: Some(Error {
                    code: code::VERSION_MISMATCH.to_owned(),
                    message: "no common protocol version: the daemon speaks [3] and the agent speaks [1]; redeploy the agent that matches this daemon".to_owned(),
                    retryable: false,
                }),
            }),
        ),
        (
            "request_snapshot",
            request(
                1,
                Some(200),
                Call::Snapshot(SnapshotParams {
                    max_depth: Some(64),
                    max_nodes: Some(5000),
                    idle_ms: Some(0),
                    ..SnapshotParams::default()
                }),
            ),
        ),
        (
            "request_snapshot_defaults",
            request(u32::MAX, None, Call::Snapshot(SnapshotParams::default())),
        ),
        (
            "request_snapshot_evict",
            request(
                3,
                Some(10_000),
                Call::Snapshot(SnapshotParams {
                    idle_ms: Some(0),
                    evict_event_less: Some(true),
                    ..SnapshotParams::default()
                }),
            ),
        ),
        (
            "request_snapshot_evict_windows",
            request(
                4,
                Some(10_000),
                Call::Snapshot(SnapshotParams {
                    idle_ms: Some(0),
                    evict: Some(vec![12, 65536, u32::MAX]),
                    ..SnapshotParams::default()
                }),
            ),
        ),
        (
            "request_bounds",
            request(
                24,
                Some(500),
                Call::Bounds(BoundsParams {
                    nodes: vec![0, 23, 24, 255, 256, 65535, 65536, u32::MAX],
                }),
            ),
        ),
        (
            "request_screenshot",
            request(
                2,
                Some(65536),
                Call::Screenshot(ScreenshotParams {
                    format: Some(ImageFormat::JPEG),
                    quality: Some(80),
                    max_dimension: Some(1280),
                }),
            ),
        ),
        (
            "request_tap",
            request(
                3,
                Some(1000),
                Call::Tap(TapParams {
                    x: 540,
                    y: 1200,
                    duration_ms: None,
                }),
            ),
        ),
        (
            "request_long_press",
            request(
                23,
                None,
                Call::Tap(TapParams {
                    x: 0,
                    y: -1,
                    duration_ms: Some(600),
                }),
            ),
        ),
        (
            "request_swipe",
            request(
                24,
                None,
                Call::Swipe(SwipeParams {
                    x1: -24,
                    y1: -25,
                    x2: i32::MIN,
                    y2: i32::MAX,
                    duration_ms: 300,
                    rest_ms: None,
                }),
            ),
        ),
        (
            "request_swipe_resting",
            request(
                25,
                Some(1360),
                Call::Swipe(SwipeParams {
                    x1: 540,
                    y1: 2000,
                    x2: 540,
                    y2: 425,
                    duration_ms: 300,
                    rest_ms: Some(60),
                }),
            ),
        ),
        (
            "request_key",
            request(
                255,
                None,
                Call::Key(KeyParams {
                    code: 66,
                    meta: Some(0x1000),
                }),
            ),
        ),
        (
            "request_text_set",
            request(
                256,
                None,
                Call::TextSet(TextParams {
                    text: "h\u{e9}llo w\u{f6}rld \u{2013} \u{2713} \u{65e5}\u{672c} \u{1f642}\n\t\"quoted\"".to_owned(),
                    node: Some(42),
                    mode: Some(TextMethod::DIRECT),
                }),
            ),
        ),
        (
            "request_frames_start",
            request(
                65535,
                None,
                Call::FramesStart(FramesParams {
                    max_fps: Some(30),
                    max_dimension: None,
                }),
            ),
        ),
        ("request_frames_stop", request(65536, None, Call::FramesStop)),
        (
            "request_resync",
            request(
                7,
                None,
                Call::Resync(ResyncParams {
                    stream: Stream::TREE,
                }),
            ),
        ),
        ("request_shutdown", request(8, Some(0), Call::Shutdown)),
        (
            "request_unknown_method",
            request(9, None, Call::Unknown(Method(999))),
        ),
        ("response_snapshot", ok(1, Method::SNAPSHOT, Reply::Snapshot(snapshot()))),
        (
            "response_snapshot_truncated",
            ok(
                10,
                Method::SNAPSHOT,
                Reply::Snapshot(Snapshot {
                    strings: vec![],
                    windows: vec![],
                    node_count: 0,
                    truncated: Truncation::DEPTH | Truncation::NODES | Truncation::WINDOWS,
                    seq: u64::MAX,
                }),
            ),
        ),
        (
            "response_snapshot_window_meta",
            ok(11, Method::SNAPSHOT, Reply::Snapshot(window_meta_snapshot())),
        ),
        (
            "response_snapshot_transformed_window",
            ok(
                13,
                Method::SNAPSHOT,
                Reply::Snapshot(transformed_window_snapshot()),
            ),
        ),
        (
            "response_snapshot_scroll",
            ok(12, Method::SNAPSHOT, Reply::Snapshot(scroll_snapshot())),
        ),
        (
            "response_snapshot_on_top",
            ok(14, Method::SNAPSHOT, Reply::Snapshot(on_top_snapshot())),
        ),
        (
            "response_screenshot",
            ok(
                2,
                Method::SCREENSHOT,
                Reply::Screenshot(Screenshot {
                    format: ImageFormat::PNG,
                    width: 1080,
                    height: 2400,
                    rotation: Rotation::R90,
                    data: (0..300u32).map(|i| (i % 256) as u8).collect(),
                }),
            ),
        ),
        (
            "response_tap",
            ok(3, Method::TAP, Reply::Input(InputAck { accepted: true })),
        ),
        (
            "response_key_rejected",
            ok(255, Method::KEY, Reply::Input(InputAck { accepted: false })),
        ),
        (
            "response_text_set",
            ok(
                256,
                Method::TEXT_SET,
                Reply::Text(TextAck {
                    method: TextMethod::KEYS,
                }),
            ),
        ),
        (
            "response_resync",
            ok(
                7,
                Method::RESYNC,
                Reply::Resync(ResyncAck {
                    stream: Stream::TREE,
                    seq: 1 << 32,
                }),
            ),
        ),
        (
            "response_bounds",
            ok(
                24,
                Method::BOUNDS,
                Reply::Bounds(Probed {
                    nodes: vec![
                        NodeBounds {
                            node: 0,
                            bounds: Bounds::new(0, 0, 1080, 2400),
                        },
                        NodeBounds {
                            node: 24,
                            bounds: Bounds::new(-24, -25, 23, 24),
                        },
                        NodeBounds {
                            node: 65536,
                            bounds: Bounds::new(i32::MIN, -257, 256, i32::MAX),
                        },
                    ],
                }),
            ),
        ),
        (
            "response_frames_start",
            ok(65535, Method::FRAMES_START, Reply::Empty),
        ),
        (
            "response_error_window_changed",
            err(
                1,
                Method::SNAPSHOT,
                code::WINDOW_CHANGED,
                "the foreground window changed during acquisition",
                true,
            ),
        ),
        (
            "response_error_unknown_method",
            err(
                9,
                Method(999),
                code::UNKNOWN_METHOD,
                "999 is not a method this agent implements",
                false,
            ),
        ),
        (
            "response_error_frame_too_large",
            err(
                2,
                Method::SCREENSHOT,
                code::FRAME_TOO_LARGE,
                "the screenshot message for request 2 is 20000000 bytes, over the negotiated maximum frame size of 16777216 bytes",
                false,
            ),
        ),
        (
            "event_tree",
            event(
                1,
                5_000_000_000,
                EventPayload::Tree(TreeChange {
                    change: TreeChangeKind::CONTENT,
                    window: 12,
                    path: vec![
                        0, 3, 1, 23, 24, 255, 256, 65535, 65536, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13,
                        14, 15, 16, 17, 18, u32::MAX,
                    ],
                }),
            ),
        ),
        (
            "event_window",
            event(
                2,
                0,
                EventPayload::Window(WindowChange {
                    change: WindowChangeKind::ADDED,
                    window: 13,
                }),
            ),
        ),
        (
            "event_frame",
            event(
                u64::MAX,
                123_456,
                EventPayload::Frame(FrameAvailable {
                    width: 1080,
                    height: 2400,
                    rotation: Rotation::R270,
                }),
            ),
        ),
        (
            "event_lifecycle",
            event(
                3,
                1,
                EventPayload::Lifecycle(Lifecycle {
                    change: LifecycleKind::OWNERSHIP_LOST,
                    message: long_message,
                }),
            ),
        ),
        (
            "event_unknown_stream",
            event(4, 2, EventPayload::Unknown(Stream(200))),
        ),
        ("ping", Message::Ping(Ping { nonce: 0 })),
        ("pong", Message::Pong(Pong { nonce: u64::MAX })),
    ]
}
