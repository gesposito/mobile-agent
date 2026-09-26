//! Events: unsolicited messages on per-stream sequences (spec: Event channel
//! with sequence numbers).
//!
//! On the wire an event is a positional array `[4, stream, seq, time_us,
//! ...]` whose remaining elements depend on the stream.

use minicbor::decode::{self, Decoder};
use minicbor::encode::{self, Encoder, Write};
use minicbor::{Decode, Encode};

use crate::cbor::{self, Elements};
use crate::message::{Rotation, kind};

/// One event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Per stream and per connection: 1 for the first event, then one more
    /// per event on a lossless stream; increasing, possibly with gaps, on a
    /// lossy one.
    pub seq: u64,
    /// Monotonic device time in microseconds.
    pub time_us: u64,
    pub payload: EventPayload,
}

impl Event {
    pub fn stream(&self) -> Stream {
        self.payload.stream()
    }
}

/// What an event says; the variant determines the stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventPayload {
    Tree(TreeChange),
    Window(WindowChange),
    Frame(FrameAvailable),
    Lifecycle(Lifecycle),
    /// An event on a stream this version does not know; its stream-specific
    /// elements were skipped.
    Unknown(Stream),
}

impl EventPayload {
    pub fn stream(&self) -> Stream {
        match self {
            EventPayload::Tree(_) => Stream::TREE,
            EventPayload::Window(_) => Stream::WINDOW,
            EventPayload::Frame(_) => Stream::FRAME,
            EventPayload::Lifecycle(_) => Stream::LIFECYCLE,
            EventPayload::Unknown(stream) => *stream,
        }
    }
}

wire_enum! {
    /// An event stream.
    pub struct Stream(u8);
    /// Tree changes: `[change, window, path]`. Lossless.
    TREE = 0, "tree";
    /// Window changes: `[change, window]`. Lossless.
    WINDOW = 1, "window";
    /// Frame availability: `[width, height, rotation]`. Lossy.
    FRAME = 2, "frame";
    /// Agent lifecycle: `[change, message]`. Lossless.
    LIFECYCLE = 3, "lifecycle";
}

impl Stream {
    /// Whether the stream may skip sequence numbers without losing state, so
    /// a gap counts drops instead of requiring a resync. Only frame
    /// availability is lossy; unknown streams are treated as lossless.
    pub fn is_lossy(self) -> bool {
        self == Stream::FRAME
    }
}

/// A change inside one window's tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeChange {
    pub change: TreeChangeKind,
    pub window: u32,
    /// Child indices from the window root to the node the change is about.
    pub path: Vec<u32>,
}

wire_enum! {
    /// What changed in a tree.
    pub struct TreeChangeKind(u8);
    /// The subtree's structure or content.
    CONTENT = 0, "content";
    TEXT = 1, "text";
    SCROLL = 2, "scroll";
    FOCUS = 3, "focus";
    SELECTION = 4, "selection";
}

/// A change to the set or state of windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowChange {
    pub change: WindowChangeKind,
    pub window: u32,
}

wire_enum! {
    /// What changed about a window.
    pub struct WindowChangeKind(u8);
    /// Its state, such as a new activity or dialog.
    STATE = 0, "state";
    ADDED = 1, "added";
    REMOVED = 2, "removed";
    /// Which window is active or focused.
    FOCUS = 3, "focus";
    BOUNDS = 4, "bounds";
}

/// A new frame is available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameAvailable {
    pub width: u32,
    pub height: u32,
    pub rotation: Rotation,
}

/// An agent lifecycle change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lifecycle {
    pub change: LifecycleKind,
    pub message: String,
}

wire_enum! {
    /// What happened to the agent.
    pub struct LifecycleKind(u8);
    /// Attached and serving requests.
    READY = 0, "ready";
    /// Another process took the UI automation connection.
    OWNERSHIP_LOST = 1, "ownership_lost";
    /// About to exit.
    SHUTTING_DOWN = 2, "shutting_down";
    /// Still running with reduced ability; the message says what.
    WARNING = 3, "warning";
}

impl<C> Encode<C> for Event {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        let extra: u64 = match &self.payload {
            EventPayload::Tree(_) | EventPayload::Frame(_) => 3,
            EventPayload::Window(_) | EventPayload::Lifecycle(_) => 2,
            EventPayload::Unknown(_) => 0,
        };
        e.array(4 + extra)?
            .u8(kind::EVENT)?
            .u8(self.stream().0)?
            .u64(self.seq)?
            .u64(self.time_us)?;
        match &self.payload {
            EventPayload::Tree(t) => {
                e.u8(t.change.0)?.u32(t.window)?.encode_with(&t.path, ctx)?;
            }
            EventPayload::Window(w) => {
                e.u8(w.change.0)?.u32(w.window)?;
            }
            EventPayload::Frame(f) => {
                e.u32(f.width)?.u32(f.height)?.u8(f.rotation.0)?;
            }
            EventPayload::Lifecycle(l) => {
                e.u8(l.change.0)?.str(&l.message)?;
            }
            EventPayload::Unknown(_) => {}
        }
        Ok(())
    }
}

impl<'b, C> Decode<'b, C> for Event {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Event, decode::Error> {
        let at = d.position();
        let mut items = Elements::begin(d)?;
        let k = items.require(d, 0, "kind", |d| d.u8())?;
        cbor::expect_kind(Some(k), kind::EVENT, at)?;
        let stream = Stream(items.require(d, 1, "stream", |d| d.u8())?);
        let seq = items.require(d, 2, "seq", |d| d.u64())?;
        let time_us = items.require(d, 3, "time_us", |d| d.u64())?;
        let payload = match stream {
            Stream::TREE => EventPayload::Tree(TreeChange {
                change: TreeChangeKind(items.require(d, 4, "change", |d| d.u8())?),
                window: items.require(d, 5, "window", |d| d.u32())?,
                path: items.require(d, 6, "path", |d| cbor::decode_vec(d, &mut *ctx))?,
            }),
            Stream::WINDOW => EventPayload::Window(WindowChange {
                change: WindowChangeKind(items.require(d, 4, "change", |d| d.u8())?),
                window: items.require(d, 5, "window", |d| d.u32())?,
            }),
            Stream::FRAME => EventPayload::Frame(FrameAvailable {
                width: items.require(d, 4, "width", |d| d.u32())?,
                height: items.require(d, 5, "height", |d| d.u32())?,
                rotation: Rotation(items.require(d, 6, "rotation", |d| d.u8())?),
            }),
            Stream::LIFECYCLE => EventPayload::Lifecycle(Lifecycle {
                change: LifecycleKind(items.require(d, 4, "change", |d| d.u8())?),
                message: items.require(d, 5, "message", |d| d.str().map(str::to_owned))?,
            }),
            other => EventPayload::Unknown(other),
        };
        items.skip_rest(d)?;
        Ok(Event {
            seq,
            time_us,
            payload,
        })
    }
}
