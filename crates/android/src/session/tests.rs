//! Session tests over a scripted agent: the backend traits and a mirror on
//! top of them, without a device.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mirror::backend::{
    BackendInfo, Input, NodeHandle, ProbedNode, ReadReason, ReadRequest, StreamItem,
};
use mirror::{Action, Mirror, MirrorConfig, Point, SettleOptions, SnapshotOptions, Target};
use protocol::code;
use protocol::event::{EventPayload, Lifecycle, LifecycleKind, Stream, TreeChange, TreeChangeKind};
use protocol::message::{
    BoundsParams, Call, ImageFormat, InputAck as AgentAck, NodeBounds, Probed, Reply, ResyncAck,
    Rotation, Screenshot, ScreenshotParams, TextAck, TextMethod,
};
use protocol::session::HostConfig;
use protocol::snapshot::{
    Bounds, Node, NodeFlags, Snapshot, Truncation, Window, WindowFlags, WindowType,
};

use super::tree::tests::compose_screen;
use super::*;
use crate::agent::{AgentInfo, Connection, Outcome, Stopped};
use crate::fake_agent::{FakeAgent, Handler, config};

const WAIT: Duration = Duration::from_secs(5);
const SCALE: f64 = 2.625;

/// A session over a scripted agent answering with `handler`.
fn session(handler: Handler) -> (FakeAgent, Session) {
    session_with(config(Some("tok")), handler)
}

/// A session over a scripted agent with `agent_config`.
fn session_with(
    agent_config: protocol::session::AgentConfig,
    handler: Handler,
) -> (FakeAgent, Session) {
    let agent = FakeAgent::start(agent_config, handler);
    let connection = Connection::open(
        agent.connect(),
        HostConfig {
            token: Some("tok".into()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let negotiated = connection.handshake(WAIT).unwrap();
    let info = AgentInfo::new(
        "emulator-5554",
        "mobile-agent-0.1.0-p1-00ff",
        812,
        &negotiated,
        "tok",
    );
    let session = Session::over(
        connection,
        info,
        Outcome::Reused,
        None,
        SCALE,
        SessionOptions::default(),
        Selection {
            link: None,
            paths: Paths::agent(false, None),
        },
    );
    (agent, session)
}

fn read(reason: ReadReason) -> ReadRequest {
    ReadRequest {
        windows: None,
        reason,
    }
}

/// A node with an identifier, a text and flags.
fn tagged(id: u32, class: u32, tag: &str, text: &str, bounds: Bounds, clickable: bool) -> Node {
    let mut flags = NodeFlags::ENABLED | NodeFlags::VISIBLE;
    if clickable {
        flags = flags | NodeFlags::CLICKABLE | NodeFlags::FOCUSABLE;
    }
    Node {
        id,
        class,
        res_pkg: None,
        res_id: Some(tag.into()),
        text: Some(text.into()),
        desc: None,
        bounds,
        flags,
        children: Vec::new(),
        scroll: None,
    }
}

/// The Compose fixture's app window with `children` under its ComposeView.
fn compose_app(children: Vec<Node>) -> Snapshot {
    let strings = super::tree::tests::strings();
    let full = Bounds::new(0, 0, 1080, 2400);
    let root = Node {
        id: 1,
        class: 0,
        res_pkg: None,
        res_id: None,
        text: None,
        desc: None,
        bounds: full,
        flags: NodeFlags::ENABLED | NodeFlags::VISIBLE,
        scroll: None,
        children: vec![Node {
            id: 2,
            class: 1,
            res_pkg: None,
            res_id: None,
            text: None,
            desc: None,
            bounds: full,
            flags: NodeFlags::ENABLED | NodeFlags::VISIBLE,
            children,
            scroll: None,
        }],
    };
    let node_count = 2 + root.children[0].children.len() as u32;
    Snapshot {
        strings,
        windows: vec![Window {
            id: 1,
            kind: WindowType::APPLICATION,
            layer: 1,
            package: Some(3),
            title: Some("FixtureCompose".into()),
            bounds: full,
            flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
            root: Some(root),
            // What the agent reports for a Compose window of the app.
            emits_events: Some(false),
            owner: None,
            modal: false,
        }],
        node_count,
        truncated: Truncation::EMPTY,
        seq: 0,
    }
}

/// The Big tab: a login tab and cells. After the tap on the tab, the Login
/// screen: the tab and a username field.
fn big_then_login(tapped: &AtomicBool) -> Snapshot {
    let tab = tagged(
        3,
        4,
        "tab-login",
        "Login",
        Bounds::new(0, 2200, 216, 2400),
        true,
    );
    if tapped.load(Ordering::SeqCst) {
        let field = tagged(
            4,
            4,
            "login-username",
            "",
            Bounds::new(40, 300, 1040, 420),
            true,
        );
        compose_app(vec![tab, field])
    } else {
        let mut children = vec![tab];
        for i in 1..=5 {
            let x = 40 + 200 * (i as i32 - 1);
            children.push(tagged(
                10 + i,
                4,
                &format!("cell-{i}"),
                &format!("Cell {i}"),
                Bounds::new(x, 400, x + 180, 520),
                true,
            ));
        }
        compose_app(children)
    }
}

#[test]
fn a_read_passes_the_agents_snapshot_through_with_the_scale_and_the_backend() {
    let (agent, session) = session(Box::new(|request| match &request.call {
        Call::Snapshot(_) => Some(Ok(Reply::Snapshot(compose_screen(3)))),
        _ => None,
    }));
    let mut backend = session.backend();
    let tree = backend.tree.read(&read(ReadReason::Attach)).unwrap();
    // The window metadata and scroll semantics travel in the payload.
    assert_eq!(tree.snapshot, compose_screen(3));
    assert_eq!(tree.scale, SCALE);
    assert_eq!(tree.backend, BackendInfo::new(BACKEND, false));
    let stats = session.last_read().unwrap();
    assert_eq!(
        (
            stats.node_count,
            stats.serialized,
            stats.windows,
            stats.reason,
            stats.cleared_cache
        ),
        (7, 7, 3, ReadReason::Attach, false)
    );
    let log = agent.log();
    let Call::Snapshot(params) = &log.requests[0].call else {
        panic!("{:?}", log.requests[0]);
    };
    assert_eq!(
        params.idle_ms, None,
        "an attach read takes the agent's idle bound"
    );
    assert_eq!(log.requests[0].deadline_ms, Some(10_000));
}

#[test]
fn a_failed_read_keeps_the_agents_code() {
    let (_agent, session) = session(Box::new(|request| match &request.call {
        Call::Snapshot(_) => Some(Err(protocol::Error::new(code::WINDOW_CHANGED, "moved"))),
        _ => None,
    }));
    let mut backend = session.backend();
    let error = backend.tree.read(&read(ReadReason::Events)).unwrap_err();
    assert_eq!(error.code, code::WINDOW_CHANGED);
    assert!(error.retryable);
}

#[test]
fn events_reach_the_backends_stream_in_order() {
    let (agent, session) = session(Box::new(|_| None));
    let mut backend = session.backend();
    agent.emit(EventPayload::Lifecycle(Lifecycle {
        change: LifecycleKind::WARNING,
        message: "input injection is unavailable".into(),
    }));
    agent.emit(EventPayload::Tree(TreeChange {
        change: TreeChangeKind::CONTENT,
        window: 1,
        path: vec![0],
    }));
    let first = backend.events.next(Instant::now() + WAIT).unwrap();
    let second = backend.events.next(Instant::now() + WAIT).unwrap();
    match (first.item, second.item) {
        (StreamItem::Event(a), StreamItem::Event(b)) => {
            assert!(matches!(a.payload, EventPayload::Lifecycle(_)), "{a:?}");
            assert!(matches!(b.payload, EventPayload::Tree(_)), "{b:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(first.at <= second.at);
}

#[test]
fn a_sequence_gap_reaches_the_mirror_and_the_stream_is_resynchronized() {
    let (agent, session) = session(Box::new(|request| match &request.call {
        Call::Resync(params) => Some(Ok(Reply::Resync(ResyncAck {
            stream: params.stream,
            seq: 3,
        }))),
        _ => None,
    }));
    let mut backend = session.backend();
    let change = || {
        EventPayload::Tree(TreeChange {
            change: TreeChangeKind::TEXT,
            window: 1,
            path: vec![0, 1],
        })
    };
    agent.emit(change());
    agent.skip(Stream::TREE);
    agent.emit(change());
    let mut items = Vec::new();
    while items.len() < 3 {
        let item = backend
            .events
            .next(Instant::now() + WAIT)
            .expect("the gap and its resync reach the stream");
        items.push(item.item);
    }
    assert!(matches!(items[0], StreamItem::Event(_)), "{items:?}");
    assert_eq!(
        items[1],
        StreamItem::OutOfSync {
            stream: Stream::TREE,
            expected: 2,
            received: 3
        }
    );
    assert_eq!(
        items[2],
        StreamItem::Resynced {
            stream: Stream::TREE,
            seq: 3
        }
    );
}

/// The protocol side of "killing the agent is reported disconnected": the
/// end of its connection reaches the mirror at once.
#[test]
fn the_agent_going_away_is_reported_disconnected_with_a_reconnect_hint() {
    let (agent, session) = session(Box::new(|_| None));
    let mut backend = session.backend();
    let gone = Instant::now();
    agent.close();
    let item = backend.events.next(Instant::now() + WAIT).unwrap();
    let StreamItem::Disconnected(message) = item.item else {
        panic!("{item:?}");
    };
    assert!(gone.elapsed() < Duration::from_secs(1));
    assert!(message.contains("reconnect"), "{message}");
    assert!(session.close_reason().is_some());
    let error = backend.tree.read(&read(ReadReason::Poll)).unwrap_err();
    assert_eq!(error.code, code::DISCONNECTED);
}

#[test]
fn input_goes_to_the_agent_in_pixels_and_returns_its_verdict() {
    let (agent, session) = session(Box::new(|request| match &request.call {
        Call::Tap(_) => Some(Ok(Reply::Input(AgentAck { accepted: true }))),
        Call::TextSet(_) => Some(Ok(Reply::Text(TextAck {
            method: TextMethod::DIRECT,
        }))),
        _ => None,
    }));
    let mut backend = session.backend();
    let ack = backend
        .input
        .perform(&Input::Tap {
            at: Point::new(100.0, 200.0),
            hold: None,
        })
        .unwrap();
    assert!(ack.accepted);
    let text = backend
        .input
        .perform(&Input::Text {
            text: "alice".into(),
            node: Some(NodeHandle { window: 1, node: 4 }),
            method: TextMethod::AUTO,
        })
        .unwrap();
    assert_eq!(text.method, Some(TextMethod::DIRECT));
    let log = agent.log();
    assert_eq!(
        log.requests[0].call,
        agent_call(
            &Input::Tap {
                at: Point::new(100.0, 200.0),
                hold: None
            },
            SCALE
        )
    );
    assert_eq!(log.requests[0].deadline_ms, Some(5_000));
    assert!(matches!(&log.requests[1].call, Call::TextSet(t) if t.node == Some(4)));
}

/// The unit-level shape of the full-stack test: the mirror serves the Big
/// tab, a tap through the sink goes to the agent, and the Compose window,
/// which sends no events, is read again with the tap's effect.
#[test]
fn a_mirror_on_the_session_reads_a_compose_window_again_after_a_tap() {
    let tapped = Arc::new(AtomicBool::new(false));
    let screen = Arc::clone(&tapped);
    let (_agent, session) = session(Box::new(move |request| match &request.call {
        Call::Snapshot(_) => Some(Ok(Reply::Snapshot(big_then_login(&screen)))),
        Call::Tap(_) => {
            screen.store(true, Ordering::SeqCst);
            Some(Ok(Reply::Input(AgentAck { accepted: true })))
        }
        _ => None,
    }));
    let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    assert!(before.text.contains("cell-5"), "{}", before.text);
    let tab = before
        .elements
        .iter()
        .find(|e| e.identifier.as_deref() == Some("tab-login"))
        .and_then(|e| e.reference)
        .expect("a ref for the login tab");
    let outcome = mirror
        .act(&Action::Tap {
            target: Target::Ref(tab),
            hold: None,
        })
        .unwrap();
    assert!(outcome.accepted);
    assert!(tapped.load(Ordering::SeqCst));
    let after = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    assert!(after.text.contains("login-username"), "{}", after.text);
    assert!(!after.text.contains("cell-1"), "{}", after.text);
    assert_eq!(session.last_read().unwrap().reason, ReadReason::Action);
}

/// Task 3.7: input the agent did not inject, the controller's, leaves only
/// the cache of the windows that send no change events stale, so the next
/// read asks the agent to evict those alone, with no resync of the whole
/// tree, once the input is a quiet window old.
#[test]
fn after_input_the_agent_did_not_inject_a_read_evicts_only_the_event_less_windows() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let (_agent, session) = session(Box::new(move |request| {
        seen.lock()
            .unwrap()
            .push((Instant::now(), request.call.clone()));
        match &request.call {
            Call::Snapshot(_) => Some(Ok(Reply::Snapshot(compose_screen(2)))),
            Call::Resync(p) => Some(Ok(Reply::Resync(ResyncAck {
                stream: p.stream,
                seq: 0,
            }))),
            _ => None,
        }
    }));
    let mut backend = session.backend();
    backend.tree.read(&read(ReadReason::Attach)).unwrap();
    let touched = Instant::now();
    session.note_foreign_input(touched);
    backend.tree.read(&read(ReadReason::Action)).unwrap();
    let calls = requests.lock().unwrap().clone();
    assert_eq!(calls.len(), 2, "no resync, only the two reads: {calls:?}");
    let Call::Snapshot(first) = &calls[0].1 else {
        panic!("{calls:?}")
    };
    assert_eq!(first.evict_event_less, None, "nothing to evict yet");
    let Call::Snapshot(params) = &calls[1].1 else {
        panic!("{calls:?}")
    };
    assert_eq!(params.evict_event_less, Some(true), "{params:?}");
    assert!(
        calls[1].0 >= touched + INPUT_QUIET,
        "the read did not wait for the input to be a quiet window old"
    );
    assert!(session.last_read().unwrap().cleared_cache);
    backend.tree.read(&read(ReadReason::Action)).unwrap();
    let calls = requests.lock().unwrap().clone();
    let Call::Snapshot(next) = &calls[2].1 else {
        panic!("{calls:?}")
    };
    assert_eq!(
        next.evict_event_less, None,
        "the agent's own input needs no eviction from the host"
    );
}

#[test]
fn without_compose_foreign_input_only_delays_the_read() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let views = {
        let mut s = compose_screen(0);
        // Every window sends change events, by the agent's account.
        s.windows[2].emits_events = Some(true);
        s
    };
    let (_agent, session) = session(Box::new(move |request| {
        seen.lock().unwrap().push(request.call.clone());
        match &request.call {
            Call::Snapshot(_) => Some(Ok(Reply::Snapshot(views.clone()))),
            _ => None,
        }
    }));
    let mut backend = session.backend();
    backend.tree.read(&read(ReadReason::Attach)).unwrap();
    session.note_foreign_input(Instant::now());
    backend.tree.read(&read(ReadReason::Action)).unwrap();
    let calls = requests.lock().unwrap().clone();
    assert!(
        calls
            .iter()
            .all(|c| matches!(c, Call::Snapshot(p) if p.evict_event_less.is_none())),
        "{calls:?}"
    );
    assert!(!session.last_read().unwrap().cleared_cache);
}

#[test]
fn screenshots_go_through_the_agent() {
    let shot = Screenshot {
        format: ImageFormat::JPEG,
        width: 540,
        height: 1200,
        rotation: Rotation::R0,
        data: vec![0xFF, 0xD8, 0xFF, 0xD9],
    };
    let answer = shot.clone();
    let (_agent, session) = session(Box::new(move |request| match &request.call {
        Call::Screenshot(_) => Some(Ok(Reply::Screenshot(answer.clone()))),
        _ => None,
    }));
    assert_eq!(
        session.screenshot(ScreenshotParams::default()).unwrap(),
        shot
    );
}

#[test]
fn stop_asks_the_agent_to_shut_down() {
    let (agent, session) = session(Box::new(|request| match &request.call {
        Call::Shutdown => Some(Ok(Reply::Empty)),
        _ => None,
    }));
    assert_eq!(
        session.stop(Duration::from_secs(1)).unwrap(),
        Stopped::Exited
    );
    assert!(
        agent
            .log()
            .requests
            .iter()
            .any(|r| r.call == Call::Shutdown)
    );
}

#[test]
#[should_panic(expected = "taken once")]
fn the_backend_is_taken_once() {
    let (_agent, session) = session(Box::new(|_| None));
    let _first = session.backend();
    let _second = session.backend();
}

/// Task 3.7: the eviction rides on the snapshot request, so an agent that
/// lost the events capability, which a resync needs, still gets its
/// event-less windows evicted after foreign input.
#[test]
fn without_the_events_capability_a_read_after_foreign_input_still_evicts() {
    let mut agent_config = config(Some("tok"));
    agent_config.capabilities = protocol::Capabilities::TREE | protocol::Capabilities::INPUT;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let (_agent, session) = session_with(
        agent_config,
        Box::new(move |request| {
            seen.lock().unwrap().push(request.call.clone());
            match &request.call {
                Call::Snapshot(_) => Some(Ok(Reply::Snapshot(compose_screen(2)))),
                _ => None,
            }
        }),
    );
    let mut backend = session.backend();
    backend.tree.read(&read(ReadReason::Attach)).unwrap();
    session.note_foreign_input(Instant::now());
    backend.tree.read(&read(ReadReason::Action)).unwrap();
    let calls = requests.lock().unwrap().clone();
    assert!(
        matches!(calls.as_slice(), [Call::Snapshot(_), Call::Snapshot(p)] if p.evict_event_less == Some(true)),
        "{calls:?}"
    );
    assert!(session.last_read().unwrap().cleared_cache);
}

/// An agent config as a 0.3.0 agent's hello fills it: bounds probes too.
fn probing() -> protocol::session::AgentConfig {
    let mut agent_config = config(Some("tok"));
    agent_config.capabilities |= protocol::Capabilities::BOUNDS;
    agent_config
}

/// Task 10.11: a probe asks the agent's `bounds` for the nodes' ids, within
/// its own short deadline, and maps each node the agent found back to its
/// window; a node the agent left out is gone.
#[test]
fn a_probe_asks_the_agent_for_the_nodes_and_maps_them_back_to_their_windows() {
    let (agent, session) = session_with(
        probing(),
        Box::new(|request| match &request.call {
            Call::Bounds(_) => Some(Ok(Reply::Bounds(Probed {
                nodes: vec![
                    NodeBounds {
                        node: 4,
                        bounds: Bounds::new(40, 300, 1040, 420),
                    },
                    NodeBounds {
                        node: 7,
                        bounds: Bounds::new(0, 0, 1080, 132),
                    },
                ],
            }))),
            _ => None,
        }),
    );
    let mut backend = session.backend();
    assert!(backend.tree.can_probe());
    let field = NodeHandle { window: 1, node: 4 };
    let clock = NodeHandle { window: 2, node: 7 };
    let gone = NodeHandle {
        window: 1,
        node: 99,
    };
    let found = backend.tree.probe(&[field, clock, gone]).unwrap();
    assert_eq!(
        found,
        [
            ProbedNode {
                node: field,
                bounds: Bounds::new(40, 300, 1040, 420),
            },
            ProbedNode {
                node: clock,
                bounds: Bounds::new(0, 0, 1080, 132),
            },
        ]
    );
    let log = agent.log();
    assert_eq!(
        log.requests[0].call,
        Call::Bounds(BoundsParams {
            nodes: vec![4, 7, 99]
        })
    );
    assert_eq!(log.requests[0].deadline_ms, Some(500));
}

/// An agent from before bounds probes does not advertise them, so the
/// mirror's settle never asks it.
#[test]
fn an_agent_without_bounds_probes_is_never_probed() {
    let (_agent, session) = session(Box::new(|_| None));
    let backend = session.backend();
    assert!(!backend.tree.can_probe());
}

/// Task 10.11: a read for nodes that bounds probes found moved asks the
/// agent to evict those windows' cache first, without an idle wait; other
/// reads evict nothing.
#[test]
fn a_read_for_moved_nodes_asks_the_agent_to_evict_their_windows() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let (_agent, session) = session_with(
        probing(),
        Box::new(move |request| {
            seen.lock().unwrap().push(request.call.clone());
            match &request.call {
                Call::Snapshot(_) => Some(Ok(Reply::Snapshot(compose_screen(1)))),
                _ => None,
            }
        }),
    );
    let mut backend = session.backend();
    backend.tree.read(&read(ReadReason::Attach)).unwrap();
    backend
        .tree
        .read(&ReadRequest {
            windows: Some(vec![1, 3]),
            reason: ReadReason::Moved,
        })
        .unwrap();
    assert!(session.last_read().unwrap().cleared_cache);
    backend.tree.read(&read(ReadReason::Events)).unwrap();
    let calls = requests.lock().unwrap().clone();
    let evicts: Vec<(Option<Vec<u32>>, Option<u32>)> = calls
        .iter()
        .map(|call| match call {
            Call::Snapshot(p) => (p.evict.clone(), p.idle_ms),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        evicts,
        [(None, None), (Some(vec![1, 3]), Some(0)), (None, Some(0))]
    );
}

/// Task 10.11, the unit-level shape of the emulator test: on a target
/// without frames, the agent's cache holds a screen where it was before it
/// slid in, and the agent's probes find it at rest. The mirror's settle
/// probes the agent, reads the window again with its cache evicted, and
/// leaves the replica holding the screen at rest.
#[test]
fn a_settle_without_frames_probes_the_agent_and_reads_the_moved_window_evicted() {
    let evicted = Arc::new(AtomicBool::new(false));
    let screen = Arc::clone(&evicted);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let at = |x: i32| {
        compose_app(vec![tagged(
            3,
            4,
            "tab-big",
            "Big",
            Bounds::new(x, 2200, x + 216, 2400),
            true,
        )])
    };
    let (_agent, session) = session_with(
        probing(),
        Box::new(move |request| {
            seen.lock().unwrap().push(request.call.clone());
            match &request.call {
                Call::Snapshot(p) => {
                    if p.evict.is_some() {
                        screen.store(true, Ordering::SeqCst);
                    }
                    // The cache keeps the tab where it was midway through
                    // the move until the window is evicted.
                    let x = if screen.load(Ordering::SeqCst) {
                        432
                    } else {
                        700
                    };
                    Some(Ok(Reply::Snapshot(at(x))))
                }
                Call::Bounds(p) => Some(Ok(Reply::Bounds(Probed {
                    nodes: p
                        .nodes
                        .iter()
                        .filter(|&&node| node == 3)
                        .map(|&node| NodeBounds {
                            node,
                            bounds: Bounds::new(432, 2200, 648, 2400),
                        })
                        .collect(),
                }))),
                _ => None,
            }
        }),
    );
    let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    mirror.note_window_change();
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    assert!(evicted.load(Ordering::SeqCst), "the window was evicted");
    let calls = requests.lock().unwrap().clone();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::Snapshot(p) if p.evict == Some(vec![1]))),
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::Bounds(p) if p.nodes == [3])),
        "{calls:?}"
    );
    let snapshot = mirror.snapshot(&SnapshotOptions::full()).unwrap();
    let tab = snapshot.by_identifier("tab-big").expect("the tab");
    assert!(
        (tab.bounds.left - 432.0 / SCALE).abs() < 1e-9,
        "the replica holds the tab at rest: {:?}",
        tab.bounds
    );
}
