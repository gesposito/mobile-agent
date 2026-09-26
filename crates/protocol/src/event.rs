//! Events: unsolicited messages on per-stream sequences (spec: Event channel
//! with sequence numbers).
//!
//! On the wire an event is a positional array `[4, stream, seq, time_us,
//! ...]` whose remaining elements depend on the stream.

use crate::message::Rotation;

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
