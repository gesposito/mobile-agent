//! Per-stream event sequence tracking (spec: Event channel with sequence
//! numbers).
//!
//! Every stream numbers its events from 1 on each connection. On a lossless
//! stream each event is one more than the last; anything else means the two
//! sides disagree about the stream's state, so it goes out of sync until a
//! resync names the last number the agent sent. A lossy stream (frame
//! availability) only increases; the numbers it skips are counted as drops.

use crate::event::Stream;

/// What to do with an event, given its stream and sequence number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqCheck {
    /// Deliver it. `dropped` counts the numbers a lossy stream skipped before
    /// it; always 0 on a lossless stream.
    Deliver { dropped: u64 },
    /// A lossless stream skipped or repeated a number and is now out of sync:
    /// request a resync. The event itself is not delivered.
    Gap { expected: u64, received: u64 },
    /// Drop it: its stream awaits a resync, or a lossy stream went backwards.
    Discard,
}

/// Sequence state of every stream seen on one connection.
#[derive(Clone, Debug, Default)]
pub struct StreamTracker {}

impl StreamTracker {
    pub fn new() -> StreamTracker {
        StreamTracker::default()
    }

    /// Checks the next event of `stream`.
    pub fn check(&mut self, stream: Stream, seq: u64) -> SeqCheck {
        let _ = (stream, seq);
        todo!("check")
    }

    /// The agent reports `stream` in sync again: `seq` is the last number it
    /// sent on it.
    pub fn resynced(&mut self, stream: Stream, seq: u64) {
        let _ = (stream, seq);
        todo!("resynced")
    }

    /// Whether `stream` is in sync; a stream not seen yet is.
    pub fn is_in_sync(&self, stream: Stream) -> bool {
        let _ = stream;
        todo!("is_in_sync")
    }

    /// Numbers a lossy stream skipped so far.
    pub fn dropped(&self, stream: Stream) -> u64 {
        let _ = stream;
        todo!("dropped")
    }
}
