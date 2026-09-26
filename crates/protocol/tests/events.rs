//! Task 3.4: per-stream event sequence tracking. A gap on a lossless stream
//! marks it out of sync and requests a resync; a lossy stream counts drops.

mod common;

use common::{Link, agent_config, every_capability, host_config, one};
use protocol::event::{
    EventPayload, FrameAvailable, Lifecycle, LifecycleKind, Stream, TreeChange, TreeChangeKind,
    WindowChange, WindowChangeKind,
};
use protocol::message::{Call, Method, Reply, ResyncAck, ResyncParams, Rotation};
use protocol::sequence::{SeqCheck, StreamTracker};
use protocol::session::{AgentEvent, HostEvent};

fn tree(window: u32) -> EventPayload {
    EventPayload::Tree(TreeChange {
        change: TreeChangeKind::CONTENT,
        window,
        path: vec![0, 1],
    })
}

fn window(window: u32) -> EventPayload {
    EventPayload::Window(WindowChange {
        change: WindowChangeKind::STATE,
        window,
    })
}

fn frame() -> EventPayload {
    EventPayload::Frame(FrameAvailable {
        width: 1080,
        height: 2400,
        rotation: Rotation::R0,
    })
}

#[test]
fn sequences_start_at_one() {
    let mut tracker = StreamTracker::new();
    assert_eq!(
        tracker.check(Stream::TREE, 1),
        SeqCheck::Deliver { dropped: 0 }
    );
    let mut tracker = StreamTracker::new();
    assert_eq!(
        tracker.check(Stream::TREE, 2),
        SeqCheck::Gap {
            expected: 1,
            received: 2,
        }
    );
}

/// Spec, Event channel with sequence numbers: event 41 then event 43 marks the
/// stream out of sync.
#[test]
fn a_gap_marks_the_stream_out_of_sync() {
    let mut tracker = StreamTracker::new();
    for seq in 1..=41 {
        assert_eq!(
            tracker.check(Stream::TREE, seq),
            SeqCheck::Deliver { dropped: 0 }
        );
    }
    assert!(tracker.is_in_sync(Stream::TREE));
    assert_eq!(
        tracker.check(Stream::TREE, 43),
        SeqCheck::Gap {
            expected: 42,
            received: 43,
        }
    );
    assert!(!tracker.is_in_sync(Stream::TREE));
    // Later events on the stream wait for the resync.
    assert_eq!(tracker.check(Stream::TREE, 44), SeqCheck::Discard);
    assert_eq!(tracker.check(Stream::TREE, 45), SeqCheck::Discard);
    tracker.resynced(Stream::TREE, 45);
    assert!(tracker.is_in_sync(Stream::TREE));
    assert_eq!(
        tracker.check(Stream::TREE, 46),
        SeqCheck::Deliver { dropped: 0 }
    );
}

#[test]
fn a_repeated_or_earlier_number_also_marks_the_stream_out_of_sync() {
    let mut tracker = StreamTracker::new();
    for seq in 1..=5 {
        tracker.check(Stream::WINDOW, seq);
    }
    assert_eq!(
        tracker.check(Stream::WINDOW, 5),
        SeqCheck::Gap {
            expected: 6,
            received: 5,
        }
    );
}

#[test]
fn streams_are_tracked_independently() {
    let mut tracker = StreamTracker::new();
    assert_eq!(
        tracker.check(Stream::TREE, 1),
        SeqCheck::Deliver { dropped: 0 }
    );
    assert_eq!(
        tracker.check(Stream::WINDOW, 1),
        SeqCheck::Deliver { dropped: 0 }
    );
    assert_eq!(
        tracker.check(Stream::TREE, 3),
        SeqCheck::Gap {
            expected: 2,
            received: 3,
        }
    );
    assert!(tracker.is_in_sync(Stream::WINDOW));
    assert_eq!(
        tracker.check(Stream::WINDOW, 2),
        SeqCheck::Deliver { dropped: 0 }
    );
    // A stream this version does not know is tracked as lossless.
    assert_eq!(
        tracker.check(Stream(200), 1),
        SeqCheck::Deliver { dropped: 0 }
    );
    assert!(matches!(
        tracker.check(Stream(200), 3),
        SeqCheck::Gap { .. }
    ));
}

#[test]
fn a_lossy_stream_counts_drops_without_a_resync() {
    let mut tracker = StreamTracker::new();
    assert_eq!(
        tracker.check(Stream::FRAME, 1),
        SeqCheck::Deliver { dropped: 0 }
    );
    assert_eq!(
        tracker.check(Stream::FRAME, 5),
        SeqCheck::Deliver { dropped: 3 }
    );
    assert_eq!(tracker.check(Stream::FRAME, 5), SeqCheck::Discard);
    assert_eq!(tracker.check(Stream::FRAME, 4), SeqCheck::Discard);
    assert_eq!(
        tracker.check(Stream::FRAME, 6),
        SeqCheck::Deliver { dropped: 0 }
    );
    assert!(tracker.is_in_sync(Stream::FRAME));
    assert_eq!(tracker.dropped(Stream::FRAME), 3);
}

#[test]
fn the_agent_numbers_each_stream_from_one() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    assert_eq!(link.agent.emit(tree(1), 10), Some(1));
    assert_eq!(link.agent.emit(tree(1), 11), Some(2));
    assert_eq!(link.agent.emit(window(1), 12), Some(1));
    assert_eq!(link.agent.last_seq(Stream::TREE), 2);
    assert_eq!(link.agent.last_seq(Stream::LIFECYCLE), 0);
    link.pump(link.t0);
    let seqs: Vec<(Stream, u64)> = link
        .host_events()
        .into_iter()
        .map(|e| match e {
            HostEvent::Event(e) => (e.stream(), e.seq),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        seqs,
        [(Stream::TREE, 1), (Stream::TREE, 2), (Stream::WINDOW, 1)]
    );
}

/// Spec, Event channel with sequence numbers: after event 41 the daemon
/// receives event 43, marks the stream out of sync and requests a full
/// resynchronization from the agent.
#[test]
fn a_sequence_gap_requests_a_resync_from_the_agent() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    for _ in 1..=41 {
        link.agent.emit(tree(7), 100);
    }
    // The agent's event queue overflows and drops event 42.
    assert_eq!(link.agent.skip_event(Stream::TREE), Some(42));
    assert_eq!(link.agent.emit(tree(7), 101), Some(43));
    assert_eq!(link.agent.emit(tree(7), 102), Some(44));
    link.pump(link.t0);

    let events = link.host_events();
    assert_eq!(events.len(), 42, "{events:?}");
    assert!(
        events[..41]
            .iter()
            .all(|e| matches!(e, HostEvent::Event(_)))
    );
    assert_eq!(
        events[41],
        HostEvent::OutOfSync {
            stream: Stream::TREE,
            expected: 42,
            received: 43,
        }
    );
    assert!(!link.host.streams().is_in_sync(Stream::TREE));

    // The host asked the agent for a resync of that stream.
    let requests = link.agent_events();
    let [AgentEvent::Request(request)] = requests.as_slice() else {
        panic!("expected a resync request, got {requests:?}")
    };
    assert_eq!(
        request.call,
        Call::Resync(ResyncParams {
            stream: Stream::TREE,
        })
    );
    let base = link.agent.last_seq(Stream::TREE);
    assert_eq!(base, 44);
    link.agent.respond(
        request.id,
        Ok(Reply::Resync(ResyncAck {
            stream: Stream::TREE,
            seq: base,
        })),
    );
    link.agent.emit(tree(7), 103);
    link.pump(link.t0);
    let events = link.host_events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(
        events[0],
        HostEvent::Resynced {
            stream: Stream::TREE,
            seq: 44,
        }
    );
    assert!(matches!(&events[1], HostEvent::Event(e) if e.seq == 45));
    assert!(link.host.streams().is_in_sync(Stream::TREE));
}

#[test]
fn a_resync_can_be_requested_on_demand() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    link.agent.emit(window(3), 1);
    link.pump(link.t0);
    let _ = link.host_events();
    let id = link.host.resync(Stream::WINDOW, link.t0).unwrap();
    link.pump(link.t0);
    let AgentEvent::Request(request) = one(link.agent_events()) else {
        panic!("not a request")
    };
    assert_eq!(request.id, id);
    link.agent.respond(
        id,
        Ok(Reply::Resync(ResyncAck {
            stream: Stream::WINDOW,
            seq: 1,
        })),
    );
    link.pump(link.t0);
    assert_eq!(
        link.host_events(),
        [HostEvent::Resynced {
            stream: Stream::WINDOW,
            seq: 1,
        }]
    );
}

#[test]
fn a_failed_resync_is_reported() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    link.agent.emit(tree(1), 1);
    link.agent.skip_event(Stream::TREE);
    link.agent.emit(tree(1), 2);
    link.pump(link.t0);
    let _ = link.host_events();
    let AgentEvent::Request(request) = one(link.agent_events()) else {
        panic!("not a request")
    };
    link.agent.respond(
        request.id,
        Err(protocol::Error::new(
            protocol::code::INTERNAL,
            "cache reset failed",
        )),
    );
    link.pump(link.t0);
    let events = link.host_events();
    assert!(
        matches!(
            events.as_slice(),
            [HostEvent::ResyncFailed { stream, error }]
                if *stream == Stream::TREE && error.code == protocol::code::INTERNAL
        ),
        "{events:?}"
    );
    assert!(!link.host.streams().is_in_sync(Stream::TREE));
}

#[test]
fn frame_events_skip_numbers_without_a_resync() {
    let mut link = Link::ready(host_config(&[1]), agent_config(&[1], every_capability()));
    link.agent.emit(frame(), 1);
    link.agent.skip_event(Stream::FRAME);
    link.agent.skip_event(Stream::FRAME);
    link.agent.emit(frame(), 2);
    link.agent.emit(
        EventPayload::Lifecycle(Lifecycle {
            change: LifecycleKind::READY,
            message: String::new(),
        }),
        3,
    );
    link.pump(link.t0);
    let events = link.host_events();
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(events.iter().all(|e| matches!(e, HostEvent::Event(_))));
    assert!(link.agent_events().is_empty(), "no resync requested");
    assert_eq!(link.host.streams().dropped(Stream::FRAME), 2);
    assert_eq!(
        Method::RESYNC.required_capability(),
        protocol::Capabilities::EVENTS
    );
}
