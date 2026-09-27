//! The signal a settle wait ended on. The mirror times a target's changes
//! (`Mirror::timestamps`, task 7.10): the last tree change, the last frame
//! change and the last injected input. A settled wait ended one quiet
//! window after the latest of them, and the signal that changed last is
//! the one it waited out.

use std::time::{Duration, Instant};

use mirror::Timestamps;

/// A tree change and a frame change this close count as one moment: about
/// a frame at 60 fps.
pub(crate) const TOGETHER: Duration = Duration::from_millis(20);

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
    /// The last tree change: an event's arrival, a window change noted to
    /// the mirror, or the end of a read that found a delta.
    pub tree: Option<Instant>,
    /// The last frame change; absent when the wait ignores frames.
    pub frame: Option<Instant>,
    /// The last input the mirror injected.
    pub action: Option<Instant>,
}

impl LastChanges {
    /// The changes the mirror timed, with frames only when the wait
    /// watched them.
    pub fn of(stamps: &Timestamps, frames: bool) -> LastChanges {
        LastChanges {
            tree: stamps.last_tree_change,
            frame: stamps.last_frame_change.filter(|_| frames),
            action: stamps.last_action,
        }
    }
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

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
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
