//! Task 3.1: every message kind round-trips through CBOR in the layout of
//! design D4, and the layout rules the Kotlin codec must follow hold.

use minicbor::Decoder;
use minicbor::data::Type;
use protocol::event::{
    Event, EventPayload, FrameAvailable, Lifecycle, LifecycleKind, Stream, TreeChange,
    TreeChangeKind, WindowChange, WindowChangeKind,
};
use protocol::message::{
    BoundsParams, Call, DecodeError, FramesParams, Hello, ImageFormat, InputAck, KeyParams,
    Message, Method, NodeBounds, Ping, Pong, Probed, Reply, Request, Response, ResyncAck,
    ResyncParams, Rotation, Screenshot, ScreenshotParams, SnapshotParams, SwipeParams, TapParams,
    TextAck, TextMethod, TextParams, Welcome,
};
use protocol::snapshot::{
    Bounds, MAX_NODE_DEPTH, Node, NodeFlags, Scroll, ScrollAxis, ScrollFlags, ScrollPosition,
    ScrollRange, ScrollUnit, Snapshot, Truncation, Window, WindowFlags, WindowType,
};
use protocol::{Capabilities, Error, PROTOCOL_VERSION, SUPPORTED_VERSIONS, code};

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).expect("hex byte"))
        .collect()
}

fn show(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Encodes `msg`, decodes the bytes and checks that nothing was lost.
fn round_trip(msg: Message) -> Vec<u8> {
    let bytes = msg.to_vec();
    let back =
        Message::decode(&bytes).unwrap_or_else(|e| panic!("decoding {} failed: {e}", show(&bytes)));
    assert_eq!(back, msg, "round trip of {}", show(&bytes));
    bytes
}

fn leaf(id: u32, class: u32, text: &str) -> Node {
    Node {
        id,
        class,
        res_pkg: None,
        res_id: None,
        text: Some(text.to_owned()),
        desc: None,
        bounds: Bounds::new(0, 0, 10, 10),
        flags: NodeFlags::ENABLED,
        children: Vec::new(),
        scroll: None,
    }
}

fn sample_snapshot() -> Snapshot {
    Snapshot {
        strings: vec![
            "android.widget.FrameLayout".to_owned(),
            "android.widget.TextView".to_owned(),
            "dev.placeholder.fixture:id/".to_owned(),
            "dev.placeholder.fixture".to_owned(),
        ],
        windows: vec![
            Window {
                id: 12,
                kind: WindowType::APPLICATION,
                layer: 0,
                package: Some(3),
                title: Some("Fixture".to_owned()),
                bounds: Bounds::new(0, 0, 1080, 2400),
                flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
                root: Some(Node {
                    id: 0,
                    class: 0,
                    res_pkg: Some(2),
                    res_id: Some("root".to_owned()),
                    text: None,
                    desc: Some("Main content".to_owned()),
                    bounds: Bounds::new(i32::MIN, -257, -25, i32::MAX),
                    flags: NodeFlags::ENABLED | NodeFlags::VISIBLE,
                    children: vec![
                        leaf(
                            1,
                            1,
                            "Flights \u{2708} caf\u{e9} \u{65e5}\u{672c} \u{1f642}",
                        ),
                        leaf(2, 1, ""),
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
        node_count: 3,
        truncated: Truncation::EMPTY,
        seq: 41,
    }
}

#[test]
fn protocol_version_is_one() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert_eq!(SUPPORTED_VERSIONS, &[1]);
}

#[test]
fn hello_carries_the_version_list_and_capability_set() {
    round_trip(Message::Hello(Hello {
        versions: vec![1, 2, 65535],
        capabilities: Capabilities::TREE | Capabilities::SCREENSHOT,
        max_frame: 16 * 1024 * 1024,
        agent: "mobile-agent-android 0.1.0".to_owned(),
        token: Some("placeholder-token".to_owned()),
    }));
    round_trip(Message::Hello(Hello {
        versions: vec![1],
        capabilities: Capabilities::NONE,
        max_frame: u32::MAX,
        agent: String::new(),
        token: None,
    }));
}

#[test]
fn welcome_round_trips_with_and_without_a_selected_version() {
    round_trip(Message::Welcome(Welcome {
        versions: vec![1, 2],
        version: Some(2),
        max_frame: 4096,
        error: None,
    }));
    round_trip(Message::Welcome(Welcome {
        versions: vec![3],
        version: None,
        max_frame: 4096,
        error: Some(Error::new(
            code::VERSION_MISMATCH,
            "no common protocol version",
        )),
    }));
}

#[test]
fn capability_set_covers_every_required_capability() {
    let all = Capabilities::TREE
        | Capabilities::INPUT
        | Capabilities::SCREENSHOT
        | Capabilities::FRAMES
        | Capabilities::EVENTS
        | Capabilities::TEXT_SET
        | Capabilities::BOUNDS;
    assert_eq!(
        all.names().collect::<Vec<_>>(),
        [
            "tree",
            "input",
            "screenshot",
            "frames",
            "events",
            "text_set",
            "bounds"
        ]
    );
    assert_eq!(Capabilities::FRAMES.label(), "frame streaming");
    // Task 10.11: bounds probes are bit 6, and `bounds` needs them.
    assert_eq!(Capabilities::BOUNDS.0, 1 << 6);
    assert_eq!(Capabilities::BOUNDS.label(), "bounds probes");
    assert_eq!(Method::BOUNDS.0, 11);
    assert_eq!(Method::BOUNDS.name(), Some("bounds"));
    assert_eq!(Method::BOUNDS.required_capability(), Capabilities::BOUNDS);
    assert!(all.contains(Capabilities::FRAMES));
    assert!(!Capabilities::SCREENSHOT.contains(Capabilities::FRAMES));
    // Bits a newer agent defines survive a round trip.
    let with_unknown = all | Capabilities(1 << 31);
    let bytes = round_trip(Message::Hello(Hello {
        versions: vec![1],
        capabilities: with_unknown,
        max_frame: 4096,
        agent: "a".to_owned(),
        token: None,
    }));
    let Message::Hello(hello) = Message::decode(&bytes).unwrap() else {
        panic!("not a hello")
    };
    assert_eq!(hello.capabilities, with_unknown);
}

#[test]
fn request_round_trips_for_every_method() {
    let calls = vec![
        Call::Snapshot(SnapshotParams::default()),
        Call::Snapshot(SnapshotParams {
            max_depth: Some(64),
            max_nodes: Some(5000),
            idle_ms: Some(0),
            ..SnapshotParams::default()
        }),
        Call::Snapshot(SnapshotParams {
            evict_event_less: Some(true),
            ..SnapshotParams::default()
        }),
        Call::Snapshot(SnapshotParams {
            evict: Some(vec![12, 65536, u32::MAX]),
            ..SnapshotParams::default()
        }),
        Call::Snapshot(SnapshotParams {
            evict: Some(Vec::new()),
            ..SnapshotParams::default()
        }),
        Call::Bounds(BoundsParams {
            nodes: vec![0, 23, 24, 255, 65536, u32::MAX],
        }),
        Call::Bounds(BoundsParams::default()),
        Call::Screenshot(ScreenshotParams {
            format: Some(ImageFormat::JPEG),
            quality: Some(80),
            max_dimension: Some(1280),
        }),
        Call::Tap(TapParams {
            x: 540,
            y: -1,
            duration_ms: Some(600),
        }),
        Call::Swipe(SwipeParams {
            x1: 100,
            y1: 2000,
            x2: 100,
            y2: 300,
            duration_ms: 300,
            rest_ms: None,
        }),
        Call::Swipe(SwipeParams {
            x1: 100,
            y1: 2000,
            x2: 100,
            y2: 300,
            duration_ms: 300,
            rest_ms: Some(60),
        }),
        Call::Key(KeyParams {
            code: 66,
            meta: Some(0x1000),
        }),
        Call::TextSet(TextParams {
            text: "h\u{e9}llo \u{1f642}".to_owned(),
            node: Some(42),
            mode: Some(TextMethod::DIRECT),
        }),
        Call::FramesStart(FramesParams {
            max_fps: Some(30),
            max_dimension: None,
        }),
        Call::FramesStop,
        Call::Resync(ResyncParams {
            stream: Stream::TREE,
        }),
        Call::Shutdown,
        Call::Unknown(Method(999)),
    ];
    for (i, call) in calls.into_iter().enumerate() {
        let id = [0, 1, 23, 24, 255, 256, 65535, 65536, u32::MAX][i % 9];
        round_trip(Message::Request(Request {
            id,
            deadline_ms: None,
            call: call.clone(),
        }));
        round_trip(Message::Request(Request {
            id,
            deadline_ms: Some(200),
            call,
        }));
    }
}

#[test]
fn response_with_a_result_round_trips_for_every_reply() {
    let replies = [
        (Method::SNAPSHOT, Reply::Snapshot(sample_snapshot())),
        (
            Method::SCREENSHOT,
            Reply::Screenshot(Screenshot {
                format: ImageFormat::PNG,
                width: 1080,
                height: 2400,
                rotation: Rotation::R90,
                data: (0..300u32).map(|i| i as u8).collect(),
            }),
        ),
        (Method::TAP, Reply::Input(InputAck { accepted: true })),
        (Method::KEY, Reply::Input(InputAck { accepted: false })),
        (
            Method::TEXT_SET,
            Reply::Text(TextAck {
                method: TextMethod::KEYS,
            }),
        ),
        (
            Method::RESYNC,
            Reply::Resync(ResyncAck {
                stream: Stream::WINDOW,
                seq: 1 << 32,
            }),
        ),
        (
            Method::BOUNDS,
            Reply::Bounds(Probed {
                nodes: vec![
                    NodeBounds {
                        node: 7,
                        bounds: Bounds::new(-411, 0, 1080, 2400),
                    },
                    NodeBounds {
                        node: u32::MAX,
                        bounds: Bounds::new(i32::MIN, -1, i32::MAX, 24),
                    },
                ],
            }),
        ),
        (Method::BOUNDS, Reply::Bounds(Probed::default())),
        (Method::FRAMES_START, Reply::Empty),
        (Method::FRAMES_STOP, Reply::Empty),
        (Method::SHUTDOWN, Reply::Empty),
    ];
    for (id, (method, reply)) in replies.into_iter().enumerate() {
        round_trip(Message::Response(Response {
            id: id as u32,
            method,
            result: Ok(reply),
        }));
    }
}

#[test]
fn response_with_an_error_keeps_code_message_and_retryable_flag() {
    let err = Error {
        code: "window_changed".to_owned(),
        message: "the foreground window changed during acquisition".to_owned(),
        retryable: true,
    };
    let bytes = round_trip(Message::Response(Response {
        id: 7,
        method: Method::SNAPSHOT,
        result: Err(err.clone()),
    }));
    let Message::Response(back) = Message::decode(&bytes).unwrap() else {
        panic!("not a response")
    };
    assert_eq!(back.result, Err(err));
    round_trip(Message::Response(Response {
        id: 8,
        method: Method(999),
        result: Err(Error::unknown_method(Method(999))),
    }));
}

/// Spec, Structured errors: a tree read that fails because the foreground
/// window changed is retryable; an unknown method is not.
#[test]
fn transient_errors_are_retryable_and_permanent_ones_are_not() {
    let changed = Error::new(code::WINDOW_CHANGED, "the foreground window changed");
    assert!(changed.retryable);
    let unknown = Error::unknown_method(Method(999));
    assert_eq!(unknown.code, "unknown_method");
    assert!(!unknown.retryable);
    assert!(unknown.message.contains("999"), "{}", unknown.message);
    for code in code::ALL {
        assert!(
            code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{code} is not snake_case"
        );
        assert!(
            protocol::error::retryable(code).is_some(),
            "{code} has no entry"
        );
    }
    assert_eq!(protocol::error::retryable("no_such_code"), None);
    assert!(!Error::new("no_such_code", "x").retryable);
}

#[test]
fn event_round_trips_on_every_stream() {
    let payloads = [
        EventPayload::Tree(TreeChange {
            change: TreeChangeKind::CONTENT,
            window: 12,
            path: (0..25).collect(),
        }),
        EventPayload::Window(WindowChange {
            change: WindowChangeKind::ADDED,
            window: 13,
        }),
        EventPayload::Frame(FrameAvailable {
            width: 1080,
            height: 2400,
            rotation: Rotation::R270,
        }),
        EventPayload::Lifecycle(Lifecycle {
            change: LifecycleKind::OWNERSHIP_LOST,
            message: "UI automation taken by another process".to_owned(),
        }),
        EventPayload::Unknown(Stream(200)),
    ];
    for (i, payload) in payloads.into_iter().enumerate() {
        round_trip(Message::Event(Event {
            seq: [1, u64::MAX, 41, 1 << 32, 0][i],
            time_us: 5_000_000_000,
            payload,
        }));
    }
}

#[test]
fn ping_and_pong_round_trip() {
    round_trip(Message::Ping(Ping { nonce: 0 }));
    round_trip(Message::Pong(Pong { nonce: u64::MAX }));
}

#[test]
fn snapshot_round_trips_with_its_string_table_enums_and_flags() {
    let snapshot = sample_snapshot();
    let bytes = round_trip(Message::Response(Response {
        id: 1,
        method: Method::SNAPSHOT,
        result: Ok(Reply::Snapshot(snapshot.clone())),
    }));
    // The class names appear once, in the string table, however many nodes use them.
    let needle = b"android.widget.TextView";
    let hits = bytes.windows(needle.len()).filter(|w| w == needle).count();
    assert_eq!(hits, 1);
    let truncated = Snapshot {
        truncated: Truncation::DEPTH | Truncation::NODES,
        ..snapshot
    };
    round_trip(Message::Response(Response {
        id: 2,
        method: Method::SNAPSHOT,
        result: Ok(Reply::Snapshot(truncated)),
    }));
}

#[test]
fn ping_is_an_integer_keyed_map() {
    assert_eq!(
        Message::Ping(Ping { nonce: 0 }).to_vec(),
        hex("a2 00 05 01 00")
    );
    assert_eq!(
        Message::Pong(Pong { nonce: u64::MAX }).to_vec(),
        hex("a2 00 06 01 1b ff ff ff ff ff ff ff ff")
    );
}

#[test]
fn maps_write_keys_in_ascending_order_and_omit_absent_fields() {
    let without_deadline = Message::Request(Request {
        id: 1,
        deadline_ms: None,
        call: Call::Snapshot(SnapshotParams::default()),
    });
    assert_eq!(without_deadline.to_vec(), hex("a4 00 02 01 01 02 01 04 a0"));
    let with_deadline = Message::Request(Request {
        id: 1,
        deadline_ms: Some(200),
        call: Call::Snapshot(SnapshotParams {
            max_nodes: Some(500),
            ..SnapshotParams::default()
        }),
    });
    assert_eq!(
        with_deadline.to_vec(),
        hex("a5 00 02 01 01 02 01 03 18 c8 04 a1 01 19 01 f4")
    );
    // Task 3.7: the eviction of event-less windows is key 3, a boolean, and
    // omitted when absent.
    let evicting = Message::Request(Request {
        id: 1,
        deadline_ms: None,
        call: Call::Snapshot(SnapshotParams {
            idle_ms: Some(0),
            evict_event_less: Some(true),
            ..SnapshotParams::default()
        }),
    });
    assert_eq!(
        evicting.to_vec(),
        hex("a4 00 02 01 01 02 01 04 a2 02 00 03 f5")
    );
    // Task 10.11: the windows to evict are key 4, an array of window ids,
    // omitted when absent.
    let moved = Message::Request(Request {
        id: 1,
        deadline_ms: None,
        call: Call::Snapshot(SnapshotParams {
            idle_ms: Some(0),
            evict: Some(vec![12]),
            ..SnapshotParams::default()
        }),
    });
    assert_eq!(
        moved.to_vec(),
        hex("a4 00 02 01 01 02 01 04 a2 02 00 04 81 0c")
    );
    // A bounds probe is method 11 with the node ids at key 0, and its
    // result lists each node as [node, bounds].
    let probe = Message::Request(Request {
        id: 1,
        deadline_ms: None,
        call: Call::Bounds(BoundsParams { nodes: vec![3, 7] }),
    });
    assert_eq!(
        probe.to_vec(),
        hex("a4 00 02 01 01 02 0b 04 a1 00 82 03 07")
    );
    let probed = Message::Response(Response {
        id: 1,
        method: Method::BOUNDS,
        result: Ok(Reply::Bounds(Probed {
            nodes: vec![NodeBounds {
                node: 7,
                bounds: Bounds::new(0, -1, 24, 2400),
            }],
        })),
    });
    assert_eq!(
        probed.to_vec(),
        hex("a4 00 03 01 01 02 0b 03 a1 00 81 82 07 84 00 20 18 18 19 09 60")
    );
    let error = Message::Response(Response {
        id: 24,
        method: Method::TAP,
        result: Err(Error {
            code: "busy".to_owned(),
            message: String::new(),
            retryable: true,
        }),
    });
    assert_eq!(
        error.to_vec(),
        hex("a4 00 03 01 18 18 02 03 04 a3 00 64 62 75 73 79 01 60 02 f5")
    );
}

#[test]
fn events_are_positional_arrays() {
    let event = Message::Event(Event {
        seq: 1,
        time_us: 2,
        payload: EventPayload::Tree(TreeChange {
            change: TreeChangeKind::CONTENT,
            window: 3,
            path: vec![0, 1],
        }),
    });
    assert_eq!(event.to_vec(), hex("87 04 00 01 02 00 03 82 00 01"));
}

#[test]
fn node_children_are_omitted_when_empty() {
    let snapshot = Snapshot {
        strings: vec!["V".to_owned()],
        windows: vec![Window {
            id: 1,
            kind: WindowType::APPLICATION,
            layer: 0,
            package: None,
            title: None,
            bounds: Bounds::new(0, 0, 1, 1),
            flags: WindowFlags::EMPTY,
            root: Some(Node {
                id: 0,
                class: 0,
                res_pkg: None,
                res_id: None,
                text: None,
                desc: None,
                bounds: Bounds::new(0, 0, 1, 1),
                flags: NodeFlags::EMPTY,
                children: Vec::new(),
                scroll: None,
            }),
            emits_events: None,
            owner: None,
            modal: false,
        }],
        node_count: 1,
        truncated: Truncation::EMPTY,
        seq: 0,
    };
    let bytes = Message::Response(Response {
        id: 0,
        method: Method::SNAPSHOT,
        result: Ok(Reply::Snapshot(snapshot)),
    })
    .to_vec();
    // response {0: 3, 1: 0, 2: 1, 3: snapshot}
    // snapshot {0: ["V"], 1: [window], 2: 1, 3: 0, 4: 0}
    // window {0: 1, 1: 1, 2: 0, 5: [0, 0, 1, 1], 6: 0, 7: node}
    // node {0: 0, 1: 0, 6: [0, 0, 1, 1], 7: 0}, with no key 8 for the children
    let expected = hex(concat!(
        "a4 00 03 01 00 02 01 03 ",
        "a5 00 81 61 56 01 81 ",
        "a6 00 01 01 01 02 00 05 84 00 00 01 01 06 00 07 ",
        "a4 00 00 01 00 06 84 00 00 01 01 07 00 ",
        "02 01 03 00 04 00"
    ));
    assert_eq!(show(&bytes), show(&expected));
}

fn snapshot_response(snapshot: Snapshot) -> Message {
    Message::Response(Response {
        id: 0,
        method: Method::SNAPSHOT,
        result: Ok(Reply::Snapshot(snapshot)),
    })
}

/// A list of 200 items with the first 16 in view.
fn list_scroll() -> Scroll {
    Scroll {
        axis: ScrollAxis::VERTICAL,
        flags: ScrollFlags::FORWARD,
        range: Some(ScrollRange {
            unit: ScrollUnit::ITEMS,
            min: 0,
            max: 199,
        }),
        position: Some(ScrollPosition { first: 0, last: 15 }),
    }
}

/// One system window, modal, owned by string 1 and sending no change
/// events, whose root is a list of 200 items.
fn tagged_snapshot() -> Snapshot {
    Snapshot {
        strings: vec!["V".to_owned(), "sys".to_owned()],
        windows: vec![Window {
            id: 1,
            kind: WindowType::SYSTEM,
            layer: 0,
            package: None,
            title: None,
            bounds: Bounds::new(0, 0, 1, 1),
            flags: WindowFlags::EMPTY,
            root: Some(Node {
                id: 0,
                class: 0,
                res_pkg: None,
                res_id: None,
                text: None,
                desc: None,
                bounds: Bounds::new(0, 0, 1, 1),
                flags: NodeFlags::EMPTY,
                children: Vec::new(),
                scroll: Some(list_scroll()),
            }),
            emits_events: Some(false),
            owner: Some(1),
            modal: true,
        }],
        node_count: 1,
        truncated: Truncation::EMPTY,
        seq: 0,
    }
}

/// The frame of [`tagged_snapshot`], with its root's scroll semantics
/// replaced by `scroll` (hex).
fn tagged_frame(scroll: &str) -> Vec<u8> {
    hex(&[
        // response {0: 3, 1: 0, 2: 1, 3: snapshot}
        "a4 00 03 01 00 02 01 03",
        // snapshot {0: ["V", "sys"], 1: [window], ...}
        "a5 00 82 61 56 63 73 79 73 01 81",
        // window {0: 1, 1: 3, 2: 0, 5: bounds, 6: 0, 7: node, 8: false, 9: 1, 10: true}
        "a9 00 01 01 03 02 00 05 84 00 00 01 01 06 00 07",
        // node {0: 0, 1: 0, 6: bounds, 7: 0, 9: scroll}
        "a5 00 00 01 00 06 84 00 00 01 01 07 00 09",
        scroll,
        "08 f4 09 01 0a f5",
        // ... 2: 1, 3: 0, 4: 0}
        "02 01 03 00 04 00",
    ]
    .join(" "))
}

/// Task 3.6: window metadata (change events as a tri-state, a system owner,
/// the modal flag) and scroll semantics (axis, directions, range and
/// position) survive a round trip in every state.
#[test]
fn window_metadata_and_scroll_semantics_round_trip() {
    let mut snapshot = sample_snapshot();
    let app = &mut snapshot.windows[0];
    app.emits_events = Some(false);
    let root = app.root.as_mut().unwrap();
    root.scroll = Some(list_scroll());
    root.children[0].scroll = Some(Scroll {
        axis: ScrollAxis::HORIZONTAL,
        flags: ScrollFlags::BACK | ScrollFlags::FORWARD,
        range: Some(ScrollRange {
            unit: ScrollUnit::PERCENT,
            min: 0,
            max: 100,
        }),
        position: None,
    });
    root.children[1].scroll = Some(Scroll {
        axis: ScrollAxis::UNKNOWN,
        flags: ScrollFlags::EMPTY,
        range: None,
        position: None,
    });
    let keyboard = &mut snapshot.windows[1];
    keyboard.emits_events = Some(true);
    keyboard.owner = Some(3);
    keyboard.modal = true;
    round_trip(snapshot_response(snapshot));

    // Values this version does not define are kept as they are.
    let mut newer = tagged_snapshot();
    newer.windows[0].root.as_mut().unwrap().scroll = Some(Scroll {
        axis: ScrollAxis(9),
        flags: ScrollFlags(1 << 31),
        range: Some(ScrollRange {
            unit: ScrollUnit(7),
            min: i32::MIN,
            max: i32::MAX,
        }),
        position: Some(ScrollPosition {
            first: -1,
            last: 65536,
        }),
    });
    round_trip(snapshot_response(newer));
}

/// Window metadata takes keys 8 to 10 after the root, scroll semantics key
/// 9 after the children; ranges and positions are positional arrays.
#[test]
fn window_metadata_and_scroll_semantics_follow_the_layout() {
    let bytes = round_trip(snapshot_response(tagged_snapshot()));
    // scroll {0: 1, 1: 2, 2: [0, 0, 199], 3: [0, 15]}
    let expected = tagged_frame("a4 00 01 01 02 02 83 00 00 18 c7 03 82 00 0f");
    assert_eq!(show(&bytes), show(&expected));

    // Absent semantics take no bytes: no range or position, no metadata.
    let mut bare = tagged_snapshot();
    let window = &mut bare.windows[0];
    window.emits_events = None;
    window.owner = None;
    window.modal = false;
    window.root.as_mut().unwrap().scroll = Some(Scroll {
        axis: ScrollAxis::VERTICAL,
        flags: ScrollFlags::BACK,
        range: None,
        position: None,
    });
    let bytes = round_trip(snapshot_response(bare));
    let expected = hex(concat!(
        "a4 00 03 01 00 02 01 03 ",
        "a5 00 82 61 56 63 73 79 73 01 81 ",
        "a6 00 01 01 03 02 00 05 84 00 00 01 01 06 00 07 ",
        "a5 00 00 01 00 06 84 00 00 01 01 07 00 09 a2 00 01 01 01 ",
        "02 01 03 00 04 00"
    ));
    assert_eq!(show(&bytes), show(&expected));
}

/// Frames from an agent that predates window metadata and scroll semantics
/// decode with both absent and encode back to the same bytes.
#[test]
fn a_snapshot_without_the_metadata_keys_decodes_with_them_absent() {
    // The layout of `node_children_are_omitted_when_empty`.
    let old = hex(concat!(
        "a4 00 03 01 00 02 01 03 ",
        "a5 00 81 61 56 01 81 ",
        "a6 00 01 01 01 02 00 05 84 00 00 01 01 06 00 07 ",
        "a4 00 00 01 00 06 84 00 00 01 01 07 00 ",
        "02 01 03 00 04 00"
    ));
    let Message::Response(Response {
        result: Ok(Reply::Snapshot(snapshot)),
        ..
    }) = Message::decode(&old).unwrap()
    else {
        panic!("not a snapshot")
    };
    let window = &snapshot.windows[0];
    assert_eq!(window.emits_events, None);
    assert_eq!(window.owner, None);
    assert!(!window.modal);
    assert_eq!(window.root.as_ref().unwrap().scroll, None);
    assert_eq!(show(&snapshot_response(snapshot).to_vec()), show(&old));
}

/// Keys and array elements a later version adds inside scroll semantics
/// are skipped.
#[test]
fn receivers_skip_what_they_do_not_know_in_scroll_semantics() {
    // scroll {0: 1, 1: 2, 2: [0, 0, 199, "x"], 3: [0, 15, 7], 5: "y"}
    let frame = tagged_frame("a5 00 01 01 02 02 84 00 00 18 c7 61 78 03 83 00 0f 07 05 61 79");
    assert_eq!(
        Message::decode(&frame).unwrap(),
        snapshot_response(tagged_snapshot())
    );
}

#[test]
fn a_scroll_range_or_position_missing_an_element_is_rejected() {
    // A range without its maximum.
    let short_range = tagged_frame("a4 00 01 01 02 02 82 00 00 03 82 00 0f");
    assert!(Message::decode(&short_range).is_err());
    // A position without its last item.
    let short_position = tagged_frame("a4 00 01 01 02 02 83 00 00 18 c7 03 81 00");
    assert!(Message::decode(&short_position).is_err());
    // Scroll semantics without their directions.
    let no_flags = tagged_frame("a1 00 01");
    assert!(Message::decode(&no_flags).is_err());
}

/// Walks one CBOR item and fails on any indefinite-length array, map or string.
fn assert_definite(d: &mut Decoder<'_>, bytes: &[u8]) {
    match d.datatype().expect("valid CBOR") {
        Type::Array => {
            let n = d.array().unwrap().unwrap();
            for _ in 0..n {
                assert_definite(d, bytes);
            }
        }
        Type::Map => {
            let n = d.map().unwrap().unwrap();
            for _ in 0..2 * n {
                assert_definite(d, bytes);
            }
        }
        Type::ArrayIndef | Type::MapIndef | Type::BytesIndef | Type::StringIndef | Type::Break => {
            panic!(
                "indefinite-length item at byte {} of {}",
                d.position(),
                show(bytes)
            )
        }
        _ => d.skip().unwrap(),
    }
}

#[test]
fn every_length_is_definite() {
    let messages = [
        Message::Response(Response {
            id: 1,
            method: Method::SNAPSHOT,
            result: Ok(Reply::Snapshot(sample_snapshot())),
        }),
        Message::Event(Event {
            seq: 1,
            time_us: 1,
            payload: EventPayload::Tree(TreeChange {
                change: TreeChangeKind::TEXT,
                window: 1,
                path: vec![1, 2, 3],
            }),
        }),
        Message::Hello(Hello {
            versions: vec![1],
            capabilities: Capabilities::TREE,
            max_frame: 4096,
            agent: "a".to_owned(),
            token: None,
        }),
    ];
    for msg in messages {
        let bytes = msg.to_vec();
        let mut d = Decoder::new(&bytes);
        assert_definite(&mut d, &bytes);
        assert_eq!(d.position(), bytes.len());
    }
}

#[test]
fn decoders_skip_unknown_map_keys() {
    // {0: 5, 1: 7, 9: "x"}: a ping with a key this version does not know.
    let msg = Message::decode(&hex("a3 00 05 01 07 09 61 78")).unwrap();
    assert_eq!(msg, Message::Ping(Ping { nonce: 7 }));
}

#[test]
fn decoders_skip_trailing_array_elements() {
    // A window event with one more element than this version defines.
    let msg = Message::decode(&hex("87 04 01 05 06 01 0d 61 78")).unwrap();
    assert_eq!(
        msg,
        Message::Event(Event {
            seq: 5,
            time_us: 6,
            payload: EventPayload::Window(WindowChange {
                change: WindowChangeKind::ADDED,
                window: 13,
            }),
        })
    );
    // Task 10.11: a probed node with an element after its bounds.
    let msg = Message::decode(&hex(
        "a4 00 03 01 01 02 0b 03 a1 00 81 83 07 84 00 20 18 18 19 09 60 61 78",
    ))
    .unwrap();
    assert_eq!(
        msg,
        Message::Response(Response {
            id: 1,
            method: Method::BOUNDS,
            result: Ok(Reply::Bounds(Probed {
                nodes: vec![NodeBounds {
                    node: 7,
                    bounds: Bounds::new(0, -1, 24, 2400),
                }],
            })),
        })
    );
}

#[test]
fn unknown_kinds_are_reported_rather_than_misread() {
    match Message::decode(&hex("a1 00 09")) {
        Err(DecodeError::UnknownKind(9)) => {}
        other => panic!("expected UnknownKind(9), got {other:?}"),
    }
}

#[test]
fn unknown_method_request_still_decodes_so_it_can_be_answered() {
    // {0: 2, 1: 5, 2: 999, 4: {0: 1}}
    let msg = Message::decode(&hex("a4 00 02 01 05 02 19 03 e7 04 a1 00 01")).unwrap();
    assert_eq!(
        msg,
        Message::Request(Request {
            id: 5,
            deadline_ms: None,
            call: Call::Unknown(Method(999)),
        })
    );
}

#[test]
fn trailing_bytes_after_the_message_are_rejected() {
    match Message::decode(&hex("a2 00 05 01 00 00")) {
        Err(DecodeError::TrailingBytes {
            consumed: 5,
            len: 6,
        }) => {}
        other => panic!("expected TrailingBytes, got {other:?}"),
    }
}

#[test]
fn missing_required_fields_are_rejected() {
    // A ping without its nonce.
    assert!(Message::decode(&hex("a1 00 05")).is_err());
    // A response carrying neither a result nor an error.
    assert!(Message::decode(&hex("a3 00 03 01 01 02 03")).is_err());
}

#[test]
fn arrays_are_preallocated_from_their_length() {
    let bytes = Message::Event(Event {
        seq: 1,
        time_us: 1,
        payload: EventPayload::Tree(TreeChange {
            change: TreeChangeKind::CONTENT,
            window: 1,
            path: vec![1, 2, 3, 4, 5],
        }),
    })
    .to_vec();
    let Message::Event(Event {
        payload: EventPayload::Tree(change),
        ..
    }) = Message::decode(&bytes).unwrap()
    else {
        panic!("not a tree event")
    };
    // Growing from empty would leave room for 8.
    assert_eq!(change.path.capacity(), 5);
}

#[test]
fn a_bogus_array_length_fails_without_allocating_it() {
    // A tree event whose path claims 2^32 - 1 elements and holds none.
    let bytes = hex("87 04 00 01 01 00 01 9a ff ff ff ff");
    assert!(Message::decode(&bytes).is_err());
}

#[test]
fn node_nesting_is_limited() {
    fn chain(depth: usize) -> Node {
        let mut node = leaf(0, 0, "x");
        for _ in 0..depth {
            let mut parent = leaf(0, 0, "x");
            parent.children.push(node);
            node = parent;
        }
        node
    }
    let response = |depth| {
        Message::Response(Response {
            id: 1,
            method: Method::SNAPSHOT,
            result: Ok(Reply::Snapshot(Snapshot {
                strings: vec![],
                windows: vec![Window {
                    id: 1,
                    kind: WindowType::APPLICATION,
                    layer: 0,
                    package: None,
                    title: None,
                    bounds: Bounds::new(0, 0, 1, 1),
                    flags: WindowFlags::EMPTY,
                    root: Some(chain(depth)),
                    emits_events: None,
                    owner: None,
                    modal: false,
                }],
                node_count: depth as u32 + 1,
                truncated: Truncation::EMPTY,
                seq: 0,
            })),
        })
    };
    round_trip(response(MAX_NODE_DEPTH));
    let err = Message::decode(&response(MAX_NODE_DEPTH + 1).to_vec()).unwrap_err();
    assert!(err.to_string().contains("deeper than"), "{err}");
}
