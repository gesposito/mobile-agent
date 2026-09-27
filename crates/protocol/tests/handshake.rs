//! Task 3.3: the handshake (the agent speaks first; the highest common
//! version wins; no common version fails listing both sets), capability
//! refusal, request multiplexing and deadlines.

mod common;

use std::time::Duration;

use common::{Link, agent_config, every_capability, host_config, one};
use protocol::frame::{DEFAULT_MAX_FRAME, HEADER_LEN, MIN_MAX_FRAME, write_frame};
use protocol::message::{
    BoundsParams, Call, FramesParams, ImageFormat, InputAck, Message, Method, Ping, Reply,
    Response, Rotation, Screenshot, ScreenshotParams, SnapshotParams, TapParams, TextParams, kind,
};
use protocol::session::{
    AgentEvent, AgentSession, DisconnectReason, HandshakeError, HostConfig, HostEvent, HostSession,
    Responded, select_version,
};
use protocol::snapshot::{Snapshot, Truncation};
use protocol::{Capabilities, code};

fn empty_snapshot() -> Snapshot {
    Snapshot {
        strings: vec![],
        windows: vec![],
        node_count: 0,
        truncated: Truncation::EMPTY,
        seq: 0,
    }
}

fn screenshot(bytes: usize) -> Reply {
    Reply::Screenshot(Screenshot {
        format: ImageFormat::PNG,
        width: 10,
        height: 10,
        rotation: Rotation::R0,
        data: vec![7; bytes],
    })
}

/// The one request the agent has received since the last call.
fn only_request(link: &mut Link) -> protocol::message::Request {
    match link.agent_events().as_slice() {
        [AgentEvent::Request(r)] => r.clone(),
        other => panic!("expected one request, got {other:?}"),
    }
}

#[test]
fn the_agent_speaks_first() {
    let mut link = Link::new(host_config(&[1]), agent_config(&[1], every_capability()));
    assert!(
        link.host.poll_transmit().is_none(),
        "the host waits for the hello"
    );
    let hello = link
        .agent
        .poll_transmit()
        .expect("the agent sends its hello at once");
    assert_eq!(
        Message::decode(&hello[HEADER_LEN..]).unwrap().kind(),
        kind::HELLO
    );
    link.host.receive(&hello, link.t0);
    let welcome = link
        .host
        .poll_transmit()
        .expect("the host answers the hello");
    assert_eq!(
        Message::decode(&welcome[HEADER_LEN..]).unwrap().kind(),
        kind::WELCOME
    );
}

#[test]
fn the_highest_common_version_is_selected() {
    assert_eq!(select_version(&[1, 2, 3], &[3, 2, 1]), Some(3));
    assert_eq!(select_version(&[1, 2], &[2, 3]), Some(2));
    assert_eq!(select_version(&[3], &[1]), None);
    assert_eq!(select_version(&[], &[1]), None);
}

/// Spec, Version negotiation on connect: common version selected.
#[test]
fn both_sides_proceed_on_the_common_version() {
    let mut link = Link::new(
        host_config(&[1, 2]),
        agent_config(&[2, 3], every_capability()),
    );
    link.pump(link.t0);
    let HostEvent::Ready(host) = one(link.host_events()) else {
        panic!("host not ready")
    };
    let AgentEvent::Ready(agent) = one(link.agent_events()) else {
        panic!("agent not ready")
    };
    assert_eq!(host.version, 2);
    assert_eq!(agent.version, 2);
    assert_eq!(host.capabilities, every_capability());
    assert_eq!(host.agent, "placeholder-agent 0.0.0");
    assert_eq!(link.host.negotiated().map(|n| n.version), Some(2));
}

/// Spec, Version negotiation on connect: no common version.
#[test]
fn no_common_version_closes_the_connection_listing_both_sets() {
    let mut link = Link::new(host_config(&[3]), agent_config(&[1], every_capability()));
    link.pump(link.t0);
    let host_events = link.host_events();
    let [HostEvent::Closed(DisconnectReason::Handshake(err))] = host_events.as_slice() else {
        panic!("expected a handshake failure, got {host_events:?}")
    };
    assert_eq!(
        *err,
        HandshakeError::NoCommonVersion {
            host: vec![3],
            agent: vec![1],
        }
    );
    let text = err.to_string();
    assert!(text.contains("[3]") && text.contains("[1]"), "{text}");
    assert!(text.contains("redeploy"), "{text}");
    assert!(link.host.is_closed());
    // The agent learns why before the host closes the connection.
    let agent_events = link.agent_events();
    let [AgentEvent::Closed(DisconnectReason::Handshake(HandshakeError::Refused(refusal)))] =
        agent_events.as_slice()
    else {
        panic!("expected the agent to be refused, got {agent_events:?}")
    };
    assert_eq!(refusal.code, code::VERSION_MISMATCH);
    assert!(!refusal.retryable);
    assert!(link.agent.is_closed());
    assert!(matches!(
        link.agent.close_reason(),
        Some(DisconnectReason::Handshake(HandshakeError::Refused(_)))
    ));
    assert!(matches!(
        link.host.close_reason(),
        Some(DisconnectReason::Handshake(
            HandshakeError::NoCommonVersion { .. }
        ))
    ));
    let err = link
        .host
        .send_request(Call::Shutdown, None, link.t0)
        .unwrap_err();
    assert_eq!(err.code, code::DISCONNECTED);
}

#[test]
fn the_frame_limit_is_the_smaller_maximum() {
    let host = HostConfig {
        max_frame: 1 << 20,
        ..host_config(&[1])
    };
    let mut agent = agent_config(&[1], every_capability());
    agent.max_frame = 64 * 1024;
    let mut link = Link::new(host, agent);
    link.pump(link.t0);
    let HostEvent::Ready(host) = one(link.host_events()) else {
        panic!("host not ready")
    };
    let AgentEvent::Ready(agent) = one(link.agent_events()) else {
        panic!("agent not ready")
    };
    assert_eq!(host.max_frame, 64 * 1024);
    assert_eq!(agent.max_frame, 64 * 1024);
}

#[test]
fn a_frame_limit_below_the_minimum_is_refused() {
    let mut agent = agent_config(&[1], every_capability());
    agent.max_frame = MIN_MAX_FRAME - 1;
    let mut link = Link::new(host_config(&[1]), agent);
    link.pump(link.t0);
    let events = link.host_events();
    assert!(
        matches!(
            events.as_slice(),
            [HostEvent::Closed(DisconnectReason::Handshake(
                HandshakeError::FrameLimitTooSmall { limit }
            ))] if *limit == MIN_MAX_FRAME - 1
        ),
        "{events:?}"
    );
}

#[test]
fn a_hello_with_the_wrong_token_is_refused() {
    let host = HostConfig {
        token: Some("placeholder-token-a".to_owned()),
        ..host_config(&[1])
    };
    let mut agent = agent_config(&[1], every_capability());
    agent.token = Some("placeholder-token-b".to_owned());
    let mut link = Link::new(host, agent);
    link.pump(link.t0);
    let events = link.host_events();
    assert!(
        matches!(
            events.as_slice(),
            [HostEvent::Closed(DisconnectReason::Handshake(
                HandshakeError::TokenMismatch
            ))]
        ),
        "{events:?}"
    );
    let agent_events = link.agent_events();
    assert!(
        matches!(
            agent_events.as_slice(),
            [AgentEvent::Closed(DisconnectReason::Handshake(HandshakeError::Refused(e)))]
                if e.code == code::TOKEN_MISMATCH
        ),
        "{agent_events:?}"
    );
}

#[test]
fn a_first_message_other_than_a_hello_is_a_protocol_error() {
    let t0 = std::time::Instant::now();
    let mut host = HostSession::new(host_config(&[1]), t0);
    let mut wire = Vec::new();
    write_frame(
        &Message::Ping(Ping { nonce: 1 }),
        DEFAULT_MAX_FRAME,
        &mut wire,
    )
    .unwrap();
    host.receive(&wire, t0);
    assert!(host.is_closed());
    assert!(matches!(
        host.poll_event(),
        Some(HostEvent::Closed(DisconnectReason::Protocol(_)))
    ));
}

#[test]
fn requests_wait_for_the_handshake() {
    let t0 = std::time::Instant::now();
    let mut host = HostSession::new(host_config(&[1]), t0);
    let err = host
        .send_request(Call::Snapshot(SnapshotParams::default()), None, t0)
        .unwrap_err();
    assert_eq!(err.code, code::NOT_READY);
    assert!(err.retryable);
    assert!(host.poll_transmit().is_none());
}

/// Spec, Capability advertisement: a command that needs an unadvertised
/// capability is refused, naming it, and not forwarded.
#[test]
fn an_unadvertised_capability_is_refused_naming_it() {
    let mut link = Link::ready(
        host_config(&[1]),
        agent_config(&[1], Capabilities::SCREENSHOT),
    );
    let err = link
        .host
        .send_request(Call::FramesStart(FramesParams::default()), None, link.t0)
        .unwrap_err();
    assert_eq!(err.code, code::CAPABILITY_MISSING);
    assert!(!err.retryable);
    assert!(err.message.contains("frame streaming"), "{}", err.message);
    assert!(err.message.contains("frames"), "{}", err.message);
    assert!(
        link.host.poll_transmit().is_none(),
        "the request is not forwarded"
    );
    // What the agent advertised still goes through.
    link.host
        .send_request(Call::Screenshot(ScreenshotParams::default()), None, link.t0)
        .unwrap();
    assert!(link.host.poll_transmit().is_some());
    // Task 10.11: a bounds probe needs its own capability, which an agent
    // from before it does not advertise.
    let err = link
        .host
        .send_request(Call::Bounds(BoundsParams { nodes: vec![3] }), None, link.t0)
        .unwrap_err();
    assert_eq!(err.code, code::CAPABILITY_MISSING);
    assert!(err.message.contains("bounds probes"), "{}", err.message);
    assert!(link.host.poll_transmit().is_none());
}

/// Spec, Request multiplexing: overlapping requests are matched by id
/// whatever order their responses arrive in.
#[test]
fn overlapping_requests_are_matched_by_id() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    let snapshot_id = link
        .host
        .send_request(Call::Snapshot(SnapshotParams::default()), None, link.t0)
        .unwrap();
    let screenshot_id = link
        .host
        .send_request(Call::Screenshot(ScreenshotParams::default()), None, link.t0)
        .unwrap();
    assert_ne!(snapshot_id, screenshot_id);
    link.pump(link.t0);
    let requests = link.agent_events();
    assert_eq!(requests.len(), 2, "{requests:?}");
    // The screenshot finishes first.
    assert_eq!(
        link.agent.respond(screenshot_id, Ok(screenshot(100))),
        Responded::Sent
    );
    assert_eq!(
        link.agent
            .respond(snapshot_id, Ok(Reply::Snapshot(empty_snapshot()))),
        Responded::Sent
    );
    link.pump(link.t0);
    let events = link.host_events();
    assert_eq!(
        events,
        [
            HostEvent::Response {
                id: screenshot_id,
                method: Method::SCREENSHOT,
                result: Ok(screenshot(100)),
            },
            HostEvent::Response {
                id: snapshot_id,
                method: Method::SNAPSHOT,
                result: Ok(Reply::Snapshot(empty_snapshot())),
            },
        ]
    );
}

#[test]
fn request_ids_are_unique_and_never_zero() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    let mut ids = std::collections::HashSet::new();
    for _ in 0..1000 {
        let id = link
            .host
            .send_request(Call::Shutdown, None, link.t0)
            .unwrap();
        assert_ne!(id, 0);
        assert!(ids.insert(id), "id {id} reused while in flight");
    }
}

/// Spec, Structured errors: an unknown method is answered with a
/// non-retryable error.
#[test]
fn the_agent_answers_an_unknown_method_with_a_permanent_error() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    let id = link
        .host
        .send_request(Call::Unknown(Method(999)), None, link.t0)
        .unwrap();
    link.pump(link.t0);
    assert!(
        link.agent_events().is_empty(),
        "answered by the session itself"
    );
    let events = link.host_events();
    let [
        HostEvent::Response {
            id: got,
            result: Err(err),
            ..
        },
    ] = events.as_slice()
    else {
        panic!("expected an error response, got {events:?}")
    };
    assert_eq!(*got, id);
    assert_eq!(err.code, code::UNKNOWN_METHOD);
    assert!(!err.retryable);
}

/// Spec, Deadlines and cancellation: the agent answers a request whose
/// deadline passed with a deadline error and serves the next request.
#[test]
fn a_passed_deadline_is_answered_and_the_connection_stays_usable() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    let slow = link
        .host
        .send_request(
            Call::Snapshot(SnapshotParams::default()),
            Some(Duration::from_millis(200)),
            link.t0,
        )
        .unwrap();
    link.pump(link.t0);
    let request = only_request(&mut link);
    assert_eq!(request.deadline_ms, Some(200));
    assert_eq!(link.agent.deadline(slow), Some(link.at(200)));
    assert_eq!(link.agent.poll_timeout(), Some(link.at(200)));

    // The tree read is still running when the deadline passes.
    link.agent.handle_timeout(link.at(199));
    assert!(
        link.agent.poll_transmit().is_none(),
        "not before the deadline"
    );
    link.agent.handle_timeout(link.at(200));
    link.pump(link.at(201));
    let events = link.host_events();
    let [
        HostEvent::Response {
            id,
            method,
            result: Err(err),
        },
    ] = events.as_slice()
    else {
        panic!("expected a deadline error, got {events:?}")
    };
    assert_eq!((*id, *method), (slow, Method::SNAPSHOT));
    assert_eq!(err.code, code::DEADLINE_EXCEEDED);
    assert!(err.message.contains("200 ms"), "{}", err.message);

    // The walk finishes late: its answer is dropped, not sent.
    assert_eq!(
        link.agent
            .respond(slow, Ok(Reply::Snapshot(empty_snapshot()))),
        Responded::NotInFlight
    );
    assert!(link.agent.poll_transmit().is_none());

    // The next request is served normally.
    let tap = link
        .host
        .send_request(
            Call::Tap(TapParams {
                x: 1,
                y: 2,
                duration_ms: None,
            }),
            Some(Duration::from_millis(200)),
            link.at(300),
        )
        .unwrap();
    link.pump(link.at(300));
    let request = only_request(&mut link);
    assert_eq!(request.id, tap);
    link.agent
        .respond(tap, Ok(Reply::Input(InputAck { accepted: true })));
    link.pump(link.at(310));
    assert_eq!(
        link.host_events(),
        [HostEvent::Response {
            id: tap,
            method: Method::TAP,
            result: Ok(Reply::Input(InputAck { accepted: true })),
        }]
    );
}

#[test]
fn the_host_reports_a_deadline_itself_when_the_agent_stays_silent() {
    let host = HostConfig {
        deadline_grace: Duration::from_millis(500),
        ..host_config(&[1])
    };
    let mut link = Link::ready(host, agent_config(&[1], every_capability()));
    let id = link
        .host
        .send_request(
            Call::Snapshot(SnapshotParams::default()),
            Some(Duration::from_millis(200)),
            link.t0,
        )
        .unwrap();
    link.pump(link.t0);
    let _ = link.agent_events();
    assert_eq!(link.host.poll_timeout(), Some(link.at(700)));
    link.host.handle_timeout(link.at(699));
    assert!(link.host_events().is_empty());
    link.host.handle_timeout(link.at(700));
    let events = link.host_events();
    assert!(
        matches!(
            events.as_slice(),
            [HostEvent::Response { id: got, result: Err(e), .. }]
                if *got == id && e.code == code::DEADLINE_EXCEEDED
        ),
        "{events:?}"
    );
    // A late answer is dropped; the next request is served normally.
    link.agent
        .respond(id, Ok(Reply::Snapshot(empty_snapshot())));
    link.pump(link.at(800));
    assert!(link.host_events().is_empty());
    let next = link
        .host
        .send_request(
            Call::Snapshot(SnapshotParams::default()),
            None,
            link.at(800),
        )
        .unwrap();
    link.pump(link.at(800));
    let _ = only_request(&mut link);
    link.agent
        .respond(next, Ok(Reply::Snapshot(empty_snapshot())));
    link.pump(link.at(810));
    assert!(matches!(
        link.host_events().as_slice(),
        [HostEvent::Response { id, result: Ok(_), .. }] if *id == next
    ));
}

/// Spec, Framed self-describing messages: a response larger than the
/// negotiated maximum reaches the host as an error naming the limit and the
/// request, with no partial payload.
#[test]
fn an_oversized_response_reaches_the_host_as_an_error_naming_the_limit_and_request() {
    let mut agent = agent_config(&[1], every_capability());
    agent.max_frame = 64 * 1024;
    let mut link = Link::ready(host_config(&[1]), agent);
    let id = link
        .host
        .send_request(Call::Screenshot(ScreenshotParams::default()), None, link.t0)
        .unwrap();
    link.pump(link.t0);
    let _ = only_request(&mut link);
    assert_eq!(
        link.agent.respond(id, Ok(screenshot(100_000))),
        Responded::TooLarge
    );
    link.pump(link.t0);
    let events = link.host_events();
    let [
        HostEvent::Response {
            id: got,
            method,
            result: Err(err),
        },
    ] = events.as_slice()
    else {
        panic!("expected an error response, got {events:?}")
    };
    assert_eq!((*got, *method), (id, Method::SCREENSHOT));
    assert_eq!(err.code, code::FRAME_TOO_LARGE);
    assert!(err.message.contains("65536"), "{}", err.message);
    assert!(
        err.message.contains(&format!("request {id}")),
        "{}",
        err.message
    );
    assert!(err.message.contains("screenshot"), "{}", err.message);
}

#[test]
fn an_oversized_response_frame_is_refused_by_the_host_too() {
    let mut agent = agent_config(&[1], every_capability());
    agent.max_frame = 64 * 1024;
    let mut link = Link::ready(host_config(&[1]), agent);
    let id = link
        .host
        .send_request(Call::Screenshot(ScreenshotParams::default()), None, link.t0)
        .unwrap();
    let after = link
        .host
        .send_request(Call::Shutdown, None, link.t0)
        .unwrap();
    link.pump(link.t0);
    let _ = link.agent_events();
    // An agent that ignores the negotiated limit writes the frame anyway.
    let mut wire = Vec::new();
    let big = Message::Response(Response {
        id,
        method: Method::SCREENSHOT,
        result: Ok(screenshot(100_000)),
    });
    write_frame(&big, u32::MAX, &mut wire).unwrap();
    link.host.receive(&wire, link.t0);
    link.agent.respond(after, Ok(Reply::Empty));
    link.pump(link.t0);
    let events = link.host_events();
    assert_eq!(events.len(), 2, "{events:?}");
    let HostEvent::Response {
        id: got,
        result: Err(err),
        ..
    } = &events[0]
    else {
        panic!("expected an error for the oversized frame, got {events:?}")
    };
    assert_eq!(*got, id);
    assert_eq!(err.code, code::FRAME_TOO_LARGE);
    assert!(err.message.contains("65536"), "{}", err.message);
    assert_eq!(
        events[1],
        HostEvent::Response {
            id: after,
            method: Method::SHUTDOWN,
            result: Ok(Reply::Empty),
        }
    );
}

#[test]
fn an_oversized_request_is_refused_before_it_is_sent() {
    let mut agent = agent_config(&[1], every_capability());
    agent.max_frame = 64 * 1024;
    let mut link = Link::ready(host_config(&[1]), agent);
    let err = link
        .host
        .send_request(
            Call::TextSet(TextParams {
                text: "x".repeat(100_000),
                node: None,
                mode: None,
            }),
            None,
            link.t0,
        )
        .unwrap_err();
    assert_eq!(err.code, code::FRAME_TOO_LARGE);
    assert!(err.message.contains("65536"), "{}", err.message);
    assert!(link.host.poll_transmit().is_none());
}

#[test]
fn the_end_of_the_stream_fails_pending_requests_with_a_reconnect_hint() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    let id = link
        .host
        .send_request(Call::Snapshot(SnapshotParams::default()), None, link.t0)
        .unwrap();
    link.host.receive_eof(link.at(10));
    let events = link.host_events();
    assert!(
        matches!(
            events.as_slice(),
            [
                HostEvent::Response { id: got, result: Err(e), .. },
                HostEvent::Closed(DisconnectReason::Eof),
            ] if *got == id && e.code == code::DISCONNECTED
        ),
        "{events:?}"
    );
    let err = link
        .host
        .send_request(Call::Snapshot(SnapshotParams::default()), None, link.at(11))
        .unwrap_err();
    assert_eq!(err.code, code::DISCONNECTED);
    assert!(err.message.contains("reconnect"), "{}", err.message);
    assert!(link.host.poll_timeout().is_none());
}

#[test]
fn agent_session_rejects_requests_without_the_advertised_capability() {
    // A host that ignores the capability set still gets a refusal.
    let t0 = std::time::Instant::now();
    let mut agent = AgentSession::new(agent_config(&[1], Capabilities::TREE), t0);
    let mut host = HostSession::new(host_config(&[1]), t0);
    while let Some(b) = agent.poll_transmit() {
        host.receive(&b, t0);
    }
    while let Some(b) = host.poll_transmit() {
        agent.receive(&b, t0);
    }
    let _ = std::iter::from_fn(|| agent.poll_event()).count();
    let mut wire = Vec::new();
    write_frame(
        &Message::Request(protocol::message::Request {
            id: 5,
            deadline_ms: None,
            call: Call::Tap(TapParams {
                x: 0,
                y: 0,
                duration_ms: None,
            }),
        }),
        DEFAULT_MAX_FRAME,
        &mut wire,
    )
    .unwrap();
    agent.receive(&wire, t0);
    assert!(agent.poll_event().is_none());
    let out = agent.poll_transmit().expect("an error response");
    let Message::Response(resp) = Message::decode(&out[HEADER_LEN..]).unwrap() else {
        panic!("not a response")
    };
    assert_eq!(resp.id, 5);
    assert_eq!(resp.result.unwrap_err().code, code::CAPABILITY_MISSING);
}
