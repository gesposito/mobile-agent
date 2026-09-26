//! Per-stream event sequence tracking (spec: Event channel with sequence
//! numbers).
//!
//! Every stream numbers its events from 1 on each connection. On a lossless
//! stream each event is one more than the last; anything else means the two
//! sides disagree about the stream's state, so it goes out of sync until a
//! resync names the last number the agent sent. A lossy stream (frame
//! availability) only increases; the numbers it skips are counted as drops.

use std::collections::HashMap;

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
pub struct StreamTracker {
    streams: HashMap<Stream, StreamState>,
}

#[derive(Clone, Copy, Debug)]
struct StreamState {
    /// The last number accepted, or `None` while waiting for a resync.
    last: Option<u64>,
    /// Numbers a lossy stream skipped.
    dropped: u64,
}

impl Default for StreamState {
    fn default() -> StreamState {
        StreamState {
            last: Some(0),
            dropped: 0,
        }
    }
}

impl StreamTracker {
    pub fn new() -> StreamTracker {
        StreamTracker::default()
    }

    /// Checks the next event of `stream`.
    pub fn check(&mut self, stream: Stream, seq: u64) -> SeqCheck {
        let state = self.streams.entry(stream).or_default();
        let Some(last) = state.last else {
            return SeqCheck::Discard;
        };
        if stream.is_lossy() {
            if seq <= last {
                return SeqCheck::Discard;
            }
            let dropped = seq - last - 1;
            state.dropped += dropped;
            state.last = Some(seq);
            return SeqCheck::Deliver { dropped };
        }
        let expected = last.wrapping_add(1);
        if seq == expected {
            state.last = Some(seq);
            SeqCheck::Deliver { dropped: 0 }
        } else {
            state.last = None;
            SeqCheck::Gap {
                expected,
                received: seq,
            }
        }
    }

    /// The agent reports `stream` in sync again: `seq` is the last number it
    /// sent on it.
    pub fn resynced(&mut self, stream: Stream, seq: u64) {
        self.streams.entry(stream).or_default().last = Some(seq);
    }

    /// Whether `stream` is in sync; a stream not seen yet is.
    pub fn is_in_sync(&self, stream: Stream) -> bool {
        self.streams.get(&stream).is_none_or(|s| s.last.is_some())
    }

    /// Numbers a lossy stream skipped so far.
    pub fn dropped(&self, stream: Stream) -> u64 {
        self.streams.get(&stream).map_or(0, |s| s.dropped)
    }
}
