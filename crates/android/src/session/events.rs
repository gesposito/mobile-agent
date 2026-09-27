//! The event stream: what the connection reports and, on an emulator with
//! its controller, frames, in arrival order on one channel.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use mirror::backend::{EventStream, StreamItem, Timed};
use protocol::session::HostEvent;

/// The stream item for a host session event, or `None` for events the
/// mirror has no use for: an oversized frame that answered no request, and
/// a failed resync, which the session retries on its next read (the gap
/// that caused it has already reached the mirror).
pub fn stream_item(event: HostEvent) -> Option<StreamItem> {
    let _ = event;
    todo!("task 5.3")
}

/// The mirror's end of the channel.
pub(crate) struct AgentEvents {
    pub(crate) items: Receiver<Timed<StreamItem>>,
}

impl EventStream for AgentEvents {
    fn next(&mut self, deadline: Instant) -> Option<Timed<StreamItem>> {
        let _ = (deadline, RecvTimeoutError::Timeout);
        todo!("task 5.3")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::code;
    use protocol::event::{
        Event, EventPayload, Lifecycle, LifecycleKind, Stream, TreeChange, TreeChangeKind,
    };
    use protocol::frame::Oversized;
    use protocol::session::DisconnectReason;
    use std::sync::mpsc;
    use std::time::Duration;

    fn tree_event(seq: u64) -> Event {
        Event {
            seq,
            time_us: 5,
            payload: EventPayload::Tree(TreeChange {
                change: TreeChangeKind::TEXT,
                window: 3,
                path: vec![0, 1],
            }),
        }
    }

    #[test]
    fn protocol_events_and_sequence_notices_pass_through() {
        assert_eq!(
            stream_item(HostEvent::Event(tree_event(4))),
            Some(StreamItem::Event(tree_event(4)))
        );
        assert_eq!(
            stream_item(HostEvent::OutOfSync {
                stream: Stream::TREE,
                expected: 4,
                received: 6
            }),
            Some(StreamItem::OutOfSync {
                stream: Stream::TREE,
                expected: 4,
                received: 6
            })
        );
        assert_eq!(
            stream_item(HostEvent::Resynced {
                stream: Stream::TREE,
                seq: 9
            }),
            Some(StreamItem::Resynced {
                stream: Stream::TREE,
                seq: 9
            })
        );
        let warning = Event {
            seq: 1,
            time_us: 0,
            payload: EventPayload::Lifecycle(Lifecycle {
                change: LifecycleKind::WARNING,
                message: "input injection is unavailable".into(),
            }),
        };
        assert_eq!(
            stream_item(HostEvent::Event(warning.clone())),
            Some(StreamItem::Event(warning))
        );
    }

    #[test]
    fn the_close_is_a_disconnection_with_a_reconnect_hint() {
        let Some(StreamItem::Disconnected(message)) =
            stream_item(HostEvent::Closed(DisconnectReason::Eof))
        else {
            panic!("the close is no disconnection");
        };
        assert!(message.contains("closed the connection"), "{message}");
        assert!(message.contains("reconnect"), "{message}");
        let Some(StreamItem::Disconnected(silent)) = stream_item(HostEvent::Closed(
            DisconnectReason::Silent(Duration::from_secs(3)),
        )) else {
            panic!("silence is no disconnection");
        };
        assert!(silent.contains("3.0 s"), "{silent}");
    }

    #[test]
    fn stray_oversized_frames_and_failed_resyncs_are_not_items() {
        let oversized = Oversized {
            size: 20 << 20,
            limit: 16 << 20,
            kind: None,
            id: None,
            method: None,
        };
        assert_eq!(stream_item(HostEvent::FrameTooLarge(oversized)), None);
        let failed = HostEvent::ResyncFailed {
            stream: Stream::TREE,
            error: protocol::Error::new(code::INTERNAL, "no"),
        };
        assert_eq!(stream_item(failed), None);
    }

    #[test]
    fn next_returns_what_is_queued_and_waits_no_longer_than_the_deadline() {
        let (tx, rx) = mpsc::channel();
        let mut events = AgentEvents { items: rx };
        let past = Instant::now() - Duration::from_millis(10);
        assert_eq!(events.next(past), None, "nothing queued");
        let item = Timed {
            at: Instant::now(),
            item: StreamItem::Event(tree_event(1)),
        };
        tx.send(item.clone()).unwrap();
        assert_eq!(events.next(past), Some(item), "queued items need no wait");
        let started = Instant::now();
        assert_eq!(events.next(started + Duration::from_millis(80)), None);
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(75) && waited < Duration::from_millis(500),
            "waited {waited:?}"
        );
        let later = Timed {
            at: Instant::now(),
            item: StreamItem::Reconnected,
        };
        let sender = tx.clone();
        let sent = later.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            sender.send(sent).unwrap();
        });
        let started = Instant::now();
        assert_eq!(events.next(started + Duration::from_secs(5)), Some(later));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_closed_channel_ends_the_wait() {
        let (tx, rx) = mpsc::channel::<Timed<StreamItem>>();
        drop(tx);
        let mut events = AgentEvents { items: rx };
        let started = Instant::now();
        assert_eq!(events.next(started + Duration::from_millis(200)), None);
    }
}
