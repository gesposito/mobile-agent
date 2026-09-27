//! The mirror: one target's replica and the questions it answers.

use std::time::{Duration, Instant};

use protocol::message::TextMethod;

use crate::backend::Backend;
use crate::error::MirrorError;
use crate::geometry::Point;
use crate::snapshot::{Snapshot, SnapshotOptions, TreeDelta};

/// How often a window that sends no change events is re-read when nothing
/// else re-reads it.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The shortest time from the end of a read of a window that sends no change
/// events to a re-read that frame changes trigger (design D5: re-read after
/// frames change and once more when they stop, bounded by a minimum
/// interval).
pub const DEFAULT_MIN_REREAD_INTERVAL: Duration = Duration::from_millis(200);

/// Tuning for one mirror.
#[derive(Clone, Debug, PartialEq)]
pub struct MirrorConfig {
    pub poll_interval: Duration,
    pub min_reread_interval: Duration,
}

impl Default for MirrorConfig {
    fn default() -> MirrorConfig {
        MirrorConfig {
            poll_interval: DEFAULT_POLL_INTERVAL,
            min_reread_interval: DEFAULT_MIN_REREAD_INTERVAL,
        }
    }
}

/// Where an action lands.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A position in device points.
    Point(Point),
}

/// An action the mirror injects through the backend's input sink.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A tap, or a long press when `hold` is present.
    Tap {
        target: Target,
        hold: Option<Duration>,
    },
    Swipe {
        from: Point,
        to: Point,
        duration: Duration,
    },
    Key {
        code: u32,
        meta: Option<u32>,
    },
    /// Text for the focused field.
    Text {
        text: String,
        method: TextMethod,
    },
}

/// What happened to an action.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionOutcome {
    /// Whether the platform accepted the input.
    pub accepted: bool,
    /// Where a tap landed, in device points.
    pub at: Option<Point>,
    /// For text, the way it went in.
    pub method: Option<TextMethod>,
}

/// One target's live replica.
pub struct Mirror {
    backend: Backend,
    config: MirrorConfig,
}

impl Mirror {
    /// A mirror over `backend`. Nothing is read until the replica is first
    /// needed.
    pub fn new(backend: Backend, config: MirrorConfig) -> Mirror {
        Mirror { backend, config }
    }

    pub fn config(&self) -> &MirrorConfig {
        &self.config
    }

    /// Applies what the target reported so far and performs the reads that
    /// are due, without waiting.
    pub fn pump(&mut self) -> Result<(), MirrorError> {
        let _ = &self.backend;
        todo!("task 7.1")
    }

    /// Keeps the replica current until `deadline`: applies items as they
    /// arrive and performs reads as they fall due.
    pub fn run_until(&mut self, _deadline: Instant) -> Result<(), MirrorError> {
        todo!("task 7.1")
    }

    /// When a read of a window that sends no change events falls due next.
    pub fn next_wakeup(&self) -> Option<Instant> {
        todo!("task 7.1")
    }

    /// A snapshot of the replica, acquiring it first when none exists.
    pub fn snapshot(&mut self, _options: &SnapshotOptions) -> Result<Snapshot, MirrorError> {
        todo!("task 7.1")
    }

    /// Injects `action` and marks the windows that send no change events
    /// for a re-read.
    pub fn act(&mut self, _action: &Action) -> Result<ActionOutcome, MirrorError> {
        todo!("task 7.1")
    }

    /// The replica's revision: it grows whenever a read changes the replica.
    pub fn revision(&self) -> u64 {
        todo!("task 7.1")
    }

    /// The most recent deltas, oldest first.
    pub fn deltas(&self) -> &[TreeDelta] {
        todo!("task 7.1")
    }

    /// The replica as text, one line per window and node, for debugging and
    /// for comparing replicas.
    pub fn dump(&self) -> String {
        todo!("task 7.1")
    }
}
