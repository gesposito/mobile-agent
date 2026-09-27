//! What the daemon reads of a target's change signals beside the mirror:
//! when frames changed, by their hashes, and when tree and window events
//! arrived. The mirror keeps these instants to itself (a settle reports how
//! many changes it saw, not when), so the daemon reads them off the
//! backend's event stream on its way to the mirror. With the mirror's tree
//! deltas they give a tap's first effects and the signal a settle wait
//! ended on.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mirror::backend::{EventStream, StreamItem, Timed};
use protocol::event::EventPayload;

/// Frame changes remembered: about 17 s of a 60 fps animation.
const KEEP: usize = 1024;

/// A tree change and a frame change this close count as one moment: about
/// a frame at 60 fps.
pub(crate) const TOGETHER: Duration = Duration::from_millis(20);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the observer has seen.
#[derive(Default)]
struct Log {
    /// The last frame's hash.
    previous: Option<u64>,
    /// Capture times of frames whose hash differed from the frame before,
    /// oldest first.
    frame_changes: VecDeque<Instant>,
    /// When the last tree or window event arrived.
    tree_event: Option<Instant>,
}

/// Records the change signals a target reports; clones share the record.
#[derive(Clone, Default)]
pub(crate) struct Observer {
    log: Arc<Mutex<Log>>,
}

impl Observer {
    /// `inner`, recording what passes through it.
    pub fn tee(&self, inner: Box<dyn EventStream>) -> Box<dyn EventStream> {
        Box::new(Tee {
            inner,
            observer: self.clone(),
        })
    }

    /// Records one item as the mirror receives it. A frame counts as a
    /// change when its hash differs from the previous frame's, as the
    /// mirror counts it.
    pub fn record(&self, timed: &Timed<StreamItem>) {
        let mut log = lock(&self.log);
        match &timed.item {
            StreamItem::Frame(meta) => {
                if log.previous.is_some_and(|previous| previous != meta.hash) {
                    if log.frame_changes.len() == KEEP {
                        log.frame_changes.pop_front();
                    }
                    log.frame_changes.push_back(meta.at);
                }
                log.previous = Some(meta.hash);
            }
            StreamItem::Event(event) => {
                if matches!(
                    event.payload,
                    EventPayload::Tree(_) | EventPayload::Window(_)
                ) {
                    log.tree_event = log.tree_event.max(Some(timed.at));
                }
            }
            _ => {}
        }
    }

    /// The capture time of the first frame change at or after `since`.
    pub fn first_frame_change_since(&self, since: Instant) -> Option<Instant> {
        lock(&self.log)
            .frame_changes
            .iter()
            .find(|&&at| at >= since)
            .copied()
    }

    /// The capture time of the last frame change.
    pub fn last_frame_change(&self) -> Option<Instant> {
        lock(&self.log).frame_changes.back().copied()
    }

    /// When the last tree or window event arrived.
    pub fn last_tree_event(&self) -> Option<Instant> {
        lock(&self.log).tree_event
    }
}

/// An event stream that shows every item to an observer on its way.
struct Tee {
    inner: Box<dyn EventStream>,
    observer: Observer,
}

impl EventStream for Tee {
    fn next(&mut self, deadline: Instant) -> Option<Timed<StreamItem>> {
        let item = self.inner.next(deadline)?;
        self.observer.record(&item);
        Some(item)
    }
}

/// The signal a settle wait ended on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndedBy {
    /// The tree changed last.
    Tree,
    /// Frames changed last.
    Frame,
    /// The tree and the frames changed last, within [`TOGETHER`].
    Both,
    /// Nothing changed since the last action, or the screen was already
    /// quiet when the wait began.
    Nothing,
    /// The screen did not settle before the wait's maximum.
    Timeout,
}

impl EndedBy {
    /// `tree`, `frame`, `both`, `none` or `timeout`.
    pub fn name(self) -> &'static str {
        match self {
            EndedBy::Tree => "tree",
            EndedBy::Frame => "frame",
            EndedBy::Both => "both",
            EndedBy::Nothing => "none",
            EndedBy::Timeout => "timeout",
        }
    }
}

/// The last changes a settle wait counts its quiet window from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LastChanges {
    /// The last tree change: an event's arrival or a delta's read.
    pub tree: Option<Instant>,
    /// The last frame change; absent when the wait ignores frames.
    pub frame: Option<Instant>,
    /// The last input the daemon injected.
    pub action: Option<Instant>,
}

/// Which signal a settle wait that began at `began` ended on. A settled
/// wait ends one quiet window after the latest of the last tree change, the
/// last frame change and the last action; the signal that changed last is
/// the one it waited out, and both when their last changes came within
/// [`TOGETHER`]. When the latest was the action, or the quiet window had
/// already passed when the wait began, nothing ended it.
pub(crate) fn ended_by(
    settled: bool,
    last: LastChanges,
    began: Instant,
    quiet: Duration,
) -> EndedBy {
    if !settled {
        return EndedBy::Timeout;
    }
    let change = last.tree.max(last.frame);
    let Some(quiet_from) = change.max(last.action) else {
        return EndedBy::Nothing;
    };
    if quiet_from + quiet <= began {
        return EndedBy::Nothing;
    }
    let Some(change) = change else {
        return EndedBy::Nothing;
    };
    if last.action.is_some_and(|action| change + TOGETHER < action) {
        return EndedBy::Nothing;
    }
    let recent = |at: Option<Instant>| at.is_some_and(|at| at + TOGETHER >= change);
    match (recent(last.tree), recent(last.frame)) {
        (true, true) => EndedBy::Both,
        (true, false) => EndedBy::Tree,
        (false, true) => EndedBy::Frame,
        (false, false) => EndedBy::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use mirror::backend::FrameMeta;
    use protocol::event::{Event, Lifecycle, LifecycleKind, WindowChange, WindowChangeKind};
    use protocol::message::Rotation;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn frame(at: Instant, hash: u64) -> Timed<StreamItem> {
        Timed {
            at,
            item: StreamItem::Frame(FrameMeta {
                seq: 1,
                at,
                hash,
                width: 1,
                height: 1,
                rotation: Rotation::R0,
            }),
        }
    }

    fn event(at: Instant, payload: EventPayload) -> Timed<StreamItem> {
        Timed {
            at,
            item: StreamItem::Event(Event {
                seq: 1,
                time_us: 0,
                payload,
            }),
        }
    }

    #[test]
    fn only_frames_whose_hash_changed_count_as_frame_changes() {
        let t0 = Instant::now();
        let observer = Observer::default();
        for (offset, hash) in [(0, 1), (10, 1), (20, 2), (30, 2), (40, 3)] {
            observer.record(&frame(t0 + ms(offset), hash));
        }
        assert_eq!(observer.first_frame_change_since(t0), Some(t0 + ms(20)));
        assert_eq!(
            observer.first_frame_change_since(t0 + ms(21)),
            Some(t0 + ms(40))
        );
        assert_eq!(observer.first_frame_change_since(t0 + ms(41)), None);
        assert_eq!(observer.last_frame_change(), Some(t0 + ms(40)));
    }

    #[test]
    fn tree_and_window_events_are_tree_changes_and_others_are_not() {
        let t0 = Instant::now();
        let observer = Observer::default();
        let window = EventPayload::Window(WindowChange {
            change: WindowChangeKind::STATE,
            window: 1,
        });
        let lifecycle = EventPayload::Lifecycle(Lifecycle {
            change: LifecycleKind::WARNING,
            message: String::new(),
        });
        observer.record(&event(t0 + ms(5), window));
        observer.record(&event(t0 + ms(9), lifecycle));
        assert_eq!(observer.last_tree_event(), Some(t0 + ms(5)));
        assert_eq!(observer.last_frame_change(), None);
    }

    #[test]
    fn the_tee_records_what_passes_through() {
        struct Two(Vec<Timed<StreamItem>>);
        impl EventStream for Two {
            fn next(&mut self, _deadline: Instant) -> Option<Timed<StreamItem>> {
                self.0.pop()
            }
        }
        let t0 = Instant::now();
        let observer = Observer::default();
        let mut tee = observer.tee(Box::new(Two(vec![frame(t0 + ms(8), 2), frame(t0, 1)])));
        while tee.next(t0).is_some() {}
        assert_eq!(observer.last_frame_change(), Some(t0 + ms(8)));
    }

    fn at(t0: Instant, offset: Option<u64>) -> Option<Instant> {
        offset.map(|n| t0 + ms(n))
    }

    /// `ended_by` with changes and the wait's start as offsets from a base.
    fn ended(
        settled: bool,
        tree: Option<u64>,
        frame: Option<u64>,
        action: Option<u64>,
        began: u64,
    ) -> EndedBy {
        let t0 = Instant::now();
        let last = LastChanges {
            tree: at(t0, tree),
            frame: at(t0, frame),
            action: at(t0, action),
        };
        ended_by(settled, last, t0 + ms(began), ms(200))
    }

    #[test]
    fn a_wait_that_did_not_settle_timed_out() {
        assert_eq!(
            ended(false, Some(10), Some(900), Some(0), 5),
            EndedBy::Timeout
        );
    }

    #[test]
    fn the_signal_that_changed_last_ended_the_wait() {
        assert_eq!(
            ended(true, Some(30), Some(130), Some(0), 20),
            EndedBy::Frame
        );
        assert_eq!(ended(true, Some(130), Some(30), Some(0), 20), EndedBy::Tree);
        assert_eq!(ended(true, Some(130), None, Some(0), 20), EndedBy::Tree);
        assert_eq!(ended(true, None, Some(40), Some(0), 20), EndedBy::Frame);
    }

    #[test]
    fn changes_within_20_ms_of_each_other_ended_it_together() {
        assert_eq!(ended(true, Some(30), Some(30), Some(0), 20), EndedBy::Both);
        assert_eq!(ended(true, Some(30), Some(48), Some(0), 20), EndedBy::Both);
        assert_eq!(ended(true, Some(30), Some(52), Some(0), 20), EndedBy::Frame);
    }

    #[test]
    fn a_change_at_the_action_counts_but_older_ones_do_not() {
        assert_eq!(ended(true, Some(0), Some(0), Some(1), 20), EndedBy::Both);
        assert_eq!(ended(true, Some(0), None, Some(100), 120), EndedBy::Nothing);
        assert_eq!(ended(true, None, None, Some(100), 120), EndedBy::Nothing);
        assert_eq!(ended(true, None, None, None, 0), EndedBy::Nothing);
    }

    #[test]
    fn a_screen_quiet_before_the_wait_began_ended_on_nothing() {
        assert_eq!(
            ended(true, Some(30), Some(30), Some(0), 230),
            EndedBy::Nothing
        );
        assert_eq!(ended(true, Some(30), Some(30), Some(0), 229), EndedBy::Both);
    }
}
