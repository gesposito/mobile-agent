//! Task 3.4: application-level liveness. After a second of silence a side
//! pings; after three seconds of silence it declares the other side dead, so
//! a killed agent is reported within the spec's five seconds. Time is passed
//! in explicitly, so these tests run the clock themselves.

mod common;

use std::time::{Duration, Instant};

use common::{Link, agent_config, every_capability, host_config};
use protocol::code;
use protocol::event::{EventPayload, WindowChange, WindowChangeKind};
use protocol::frame::HEADER_LEN;
use protocol::message::{Call, Message, SnapshotParams, kind};
use protocol::session::{
    AgentEvent, DisconnectReason, HostEvent, HostSession, PEER_TIMEOUT, PING_INTERVAL,
};

/// The kinds of the frames in `bytes`.
fn kinds(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + HEADER_LEN].try_into().unwrap()) as usize;
        let payload = &bytes[at + HEADER_LEN..at + HEADER_LEN + len];
        out.push(Message::decode(payload).unwrap().kind());
        at += HEADER_LEN + len;
    }
    out
}

#[test]
fn defaults_meet_the_five_second_bound() {
    assert_eq!(PING_INTERVAL, Duration::from_secs(1));
    assert_eq!(PEER_TIMEOUT, Duration::from_secs(3));
    assert!(PEER_TIMEOUT < Duration::from_secs(5));
}

#[test]
fn a_ping_goes_out_after_a_second_of_silence() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    assert_eq!(link.host.poll_timeout(), Some(link.at(1000)));
    link.host.handle_timeout(link.at(999));
    assert!(link.host.poll_transmit().is_none());
    link.host.handle_timeout(link.at(1000));
    let sent = link.host.poll_transmit().expect("a ping");
    assert_eq!(kinds(&sent), [kind::PING]);
    // No second ping until another second has passed without an answer.
    assert_eq!(link.host.poll_timeout(), Some(link.at(2000)));
    link.host.handle_timeout(link.at(1999));
    assert!(link.host.poll_transmit().is_none());
    link.host.handle_timeout(link.at(2000));
    assert_eq!(kinds(&link.host.poll_transmit().unwrap()), [kind::PING]);
}

/// Spec, Liveness detection: a killed agent is reported disconnected within
/// five seconds and a later command fails at once with a reconnect hint.
#[test]
fn a_killed_agent_is_reported_disconnected_within_five_seconds() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    let pending = link
        .host
        .send_request(
            Call::Snapshot(SnapshotParams::default()),
            None,
            link.at(100),
        )
        .unwrap();
    link.pump(link.at(100));
    let _ = link.agent_events();
    // The agent process dies: nothing more arrives, not even an end of stream.
    let killed = link.at(100);
    let closed_at = loop {
        let now = link.host.poll_timeout().expect("the host keeps a timer");
        assert!(
            now <= killed + Duration::from_secs(5),
            "still waiting at {now:?}"
        );
        link.host.handle_timeout(now);
        let _ = link.host.poll_transmit();
        let events = link.host_events();
        if let Some(HostEvent::Closed(reason)) = events.last() {
            assert_eq!(*reason, DisconnectReason::Silent(Duration::from_secs(3)));
            let failed = events.iter().find_map(|e| match e {
                HostEvent::Response {
                    id, result: Err(e), ..
                } if *id == pending => Some(e),
                _ => None,
            });
            let failed = failed.expect("the pending request fails");
            assert_eq!(failed.code, code::DISCONNECTED);
            break now;
        }
    };
    assert!(closed_at - killed <= Duration::from_secs(5));
    assert!(link.host.is_closed());
    let err = link
        .host
        .send_request(Call::Snapshot(SnapshotParams::default()), None, closed_at)
        .unwrap_err();
    assert_eq!(err.code, code::DISCONNECTED);
    assert!(err.message.contains("reconnect"), "{}", err.message);
    assert!(link.host.poll_transmit().is_none(), "nothing is sent");
}

#[test]
fn answered_pings_keep_the_connection_alive() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    for step in 1..=40 {
        let now = link.at(step * 250);
        link.host.handle_timeout(now);
        link.agent.handle_timeout(now);
        link.pump(now);
    }
    assert!(!link.host.is_closed());
    assert!(!link.agent.is_closed());
    assert!(link.host_events().is_empty());
    assert!(link.agent_events().is_empty());
}

#[test]
fn a_chatty_agent_needs_no_pings_from_the_host() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    let mut host_pings = 0;
    for step in 1..=20 {
        let now = link.at(step * 500);
        link.agent.emit(
            EventPayload::Window(WindowChange {
                change: WindowChangeKind::FOCUS,
                window: 1,
            }),
            step,
        );
        link.host.handle_timeout(now);
        link.agent.handle_timeout(now);
        if let Some(bytes) = link.host.poll_transmit() {
            host_pings += kinds(&bytes).iter().filter(|k| **k == kind::PING).count();
            link.agent.receive(&bytes, now);
        }
        link.pump(now);
    }
    assert_eq!(host_pings, 0);
    assert!(!link.host.is_closed());
    assert!(
        !link.agent.is_closed(),
        "the agent pinged the host and heard back"
    );
}

#[test]
fn the_agent_detects_a_silent_host_too() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    link.agent.handle_timeout(link.at(1000));
    assert_eq!(kinds(&link.agent.poll_transmit().unwrap()), [kind::PING]);
    link.agent.handle_timeout(link.at(2999));
    assert!(!link.agent.is_closed());
    link.agent.handle_timeout(link.at(3000));
    let events = link.agent_events();
    assert!(
        matches!(
            events.as_slice(),
            [AgentEvent::Closed(DisconnectReason::Silent(_))]
        ),
        "{events:?}"
    );
}

#[test]
fn a_hello_that_never_comes_times_out() {
    let t0 = Instant::now();
    let mut host = HostSession::new(host_config(&[1]), t0);
    assert_eq!(host.poll_timeout(), Some(t0 + Duration::from_secs(3)));
    host.handle_timeout(t0 + Duration::from_millis(2999));
    assert!(!host.is_closed());
    host.handle_timeout(t0 + Duration::from_secs(3));
    assert!(host.poll_transmit().is_none(), "no pings before the hello");
    assert!(matches!(
        host.poll_event(),
        Some(HostEvent::Closed(DisconnectReason::Silent(_)))
    ));
}

#[test]
fn bytes_of_a_discarded_frame_do_not_count_as_signs_of_life() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    // A corrupt length announces an enormous frame; what follows is dropped.
    let mut junk = u32::MAX.to_be_bytes().to_vec();
    junk.extend([0u8; 64]);
    link.host.receive(&junk, link.at(500));
    assert!(matches!(
        link.host_events().as_slice(),
        [HostEvent::FrameTooLarge(_)]
    ));
    for ms in [1000, 1500, 2000, 2500, 3000, 3499] {
        link.host.receive(&[0u8; 100], link.at(ms));
        link.host.handle_timeout(link.at(ms));
    }
    assert!(!link.host.is_closed());
    link.host.handle_timeout(link.at(3500));
    assert!(link.host.is_closed(), "3 s after the last frame header");
}
