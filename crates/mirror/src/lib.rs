//! UI mirror: a live, locally queryable replica of a device's windows,
//! accessibility tree and latest frame (spec `ui-mirror`, design D5, D8).
//!
//! A [`Mirror`] keeps one target's replica current from what the target's
//! driver reports through a [`backend::Backend`]: full tree reads, change
//! events with sequence numbers, frames with low-resolution hashes, and the
//! input the mirror injects itself. Once warm, snapshots are served from the
//! replica without a device round trip. Windows whose toolkit sends no change
//! events are re-read in full after every injected action, when frames change
//! and on a poll.
//!
//! The crate holds no device code. Drivers implement the traits in
//! [`backend`]; [`testing`] provides a scripted fake target.

pub mod backend;
mod error;
mod geometry;
mod mirror;
mod refs;
mod replica;
mod roles;
mod snapshot;
pub mod testing;
mod verdict;
mod view;

pub use error::{MirrorError, StaleCause, code};
pub use geometry::{Point, Rect};
pub use mirror::{
    Action, ActionOutcome, DEFAULT_MIN_REREAD_INTERVAL, DEFAULT_POLL_INTERVAL, Mirror,
    MirrorConfig, Target,
};
pub use refs::Ref;
pub use snapshot::{
    Element, ElementId, RefreshMode, Role, Snapshot, SnapshotOptions, TreeDelta, WindowSource,
    WindowSummary,
};
pub use verdict::{Degradation, Verdict};
