//! When things last happened on a target, as the mirror saw them, so that a
//! daemon can time a tap's effects, name the signal a settle ended on and
//! bring the replica up to the display without watching the backend itself.

use std::time::Instant;

use crate::backend::{FrameMeta, ReadReason};

/// The mirror's latest frame, changes, read and input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timestamps {
    /// The latest frame the mirror received: its capture time, number and
    /// hash.
    pub last_frame: Option<FrameMeta>,
    /// The capture time of the latest frame whose hash differed from the
    /// frame before it. The first frame is no change.
    pub last_frame_change: Option<Instant>,
    /// The latest tree change: the arrival of a tree or window event, a
    /// window change noted to the mirror, or the end of a read that found a
    /// delta. The first acquisition is no change.
    pub last_tree_change: Option<Instant>,
    /// The latest read the replica took in.
    pub last_read: Option<ReadSpan>,
    /// When the mirror last injected input.
    pub last_action: Option<Instant>,
}

/// One read of the tree: when it began and ended, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadSpan {
    pub started: Instant,
    pub finished: Instant,
    pub reason: ReadReason,
}
