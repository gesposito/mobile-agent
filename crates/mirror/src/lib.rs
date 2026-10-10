//! UI mirror: a live, locally queryable replica of a device's windows,
//! accessibility tree and latest frame (spec `ui-mirror`, design D5, D8).
//!
//! A [`Mirror`] keeps one target's replica current from what the target's
//! driver reports through a [`backend::Backend`]: full tree reads, change
//! events with sequence numbers, frames with low-resolution hashes, and the
//! input the mirror injects itself. Once warm, snapshots are served from the
//! replica without a device round trip. Windows whose toolkit sends no change
//! events are re-read in full after every injected action, when frames change
//! and on a poll, and, after text goes into one of their fields in one piece,
//! as soon as frames change until a read shows the field's text changed.
//!
//! The crate holds no device code. Drivers implement the traits in
//! [`backend`]; [`testing`] provides a scripted fake target.

pub mod backend;
mod delta;
mod diff;
mod error;
mod frame;
mod geometry;
mod json;
mod mirror;
mod refs;
mod render;
mod replica;
mod roles;
mod settle;
mod snapshot;
pub mod testing;
mod timestamps;
mod verdict;
mod view;
mod visibility;
mod wait;

pub use delta::{DeltaSnapshot, RefChange, RefEntry, RefField, TreeSplice};
pub use diff::{Change, Diff, Field};
pub use error::{MirrorError, ResyncCause, StaleCause, code};
pub use frame::{DEFAULT_MAX_DIMENSION, DEFAULT_QUALITY, FrameImage, FrameRequest, TreeSkew};
pub use geometry::{Point, Rect};
pub use mirror::{
    Action, ActionOutcome, DEFAULT_FIRST_FRAME_WAIT, DEFAULT_MIN_REREAD_INTERVAL,
    DEFAULT_OVERSCROLL, DEFAULT_POLL_INTERVAL, DEFAULT_PROBE_ANCHORS, DEFAULT_PROBE_INTERVAL,
    DEFAULT_STILL_POLL_INTERVAL, Mirror, MirrorConfig, SlowFrames, Target,
};
pub use refs::Ref;
pub use settle::{
    DEFAULT_QUIET_WINDOW, DEFAULT_SETTLE_MAX, Settle, SettleOptions, Signal, Signals,
};
pub use snapshot::{
    Direction, Element, ElementId, HintSource, RefreshMode, Role, ScrollHint, Snapshot,
    SnapshotOptions, TreeDelta, WindowSource, WindowSummary,
};
pub use timestamps::{ReadSpan, Timestamps};
pub use verdict::{Degradation, Verdict};
pub use visibility::{Uncovered, Visibility};
pub use wait::{Condition, Selector, SnapshotSummary, WaitOutcome};

// A daemon moves each target's mirror to the thread that serves it.
const _: fn() = || {
    fn send<T: Send>() {}
    send::<Mirror>();
};
