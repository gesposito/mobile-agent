//! The driver-facing contract: what a target's driver implements so that a
//! [`Mirror`](crate::Mirror) can keep a replica of the target's screen.
//!
//! A driver hands the mirror a [`Backend`]:
//!
//! - a [`TreeSource`], which reads the target's windows in full into the
//!   protocol's snapshot payload, window metadata and scroll semantics
//!   included, above all whether each window's toolkit sends change events,
//!   and, where it can, reads a few nodes' bounds again past its caches;
//! - an [`EventStream`] of everything the target reports on its own: protocol
//!   events with their per-stream sequence numbers, frames with their
//!   low-resolution hashes, and changes to the stream itself (a gap the
//!   driver saw, a resync, a switch of tree backend, a reconnection);
//! - optionally a [`FrameSource`], which encodes the latest frame;
//! - an [`InputSink`], through which the mirror injects input, so it knows
//!   when to re-read windows that send no change events;
//! - a [`Clock`]. The mirror never reads the system clock itself, so tests
//!   drive time through [`crate::testing`].
//!
//! Every call is synchronous: a driver answers from its own threads or
//! blocks until the device answers. The mirror calls one method at a time.
//!
//! Units: tree payloads carry bounds in physical pixels, as the protocol
//! defines them, and [`TreeRead::scale`] says how many pixels make a device
//! point. Everything the mirror hands back to the driver, input coordinates
//! included, is in device points.
//!
//! What the mirror reads from a payload's nodes:
//!
//! - `class`: Android's widget class, as the agent reports it (Compose
//!   reports the class of the role a node was given). The Simulator driver
//!   passes the `XCUIElementType` name (`XCUIElementTypeButton`), or the
//!   UIKit class for elements typed `Other`, so that an alert controller
//!   (`_UIAlertControllerPhoneTVMacView`) is recognized and cuts what it
//!   covers.
//! - `desc` is the label and `text` the value: Android's content description
//!   and text (the agent can pass a field's hint as its description), iOS's
//!   label and value.
//! - `res_id` is the test identifier: the resource id after its package
//!   prefix, Compose's test tag, iOS's accessibility identifier.
//! - The `visible` flag, as the platform reports it; where it reports none,
//!   as on iOS, whether the frame overlaps the screen. Nodes outside their
//!   scroll container's viewport feed scroll hints, and where the platform
//!   leaves them out, as Compose does, the container's `scroll` semantics.
//! - The `active` window flag marks the window that owns the foreground; a
//!   window's `owner` names the system process that owns it, such as
//!   SpringBoard for a permission alert.
//!
//! What the mirror reads from a payload's windows (task 3.6):
//!
//! - `emits_events`: `Some(false)` puts the window on the re-read path
//!   (after each injected action, when frames change and on a poll) and
//!   `Some(true)` keeps it off, whatever its classes; without it, a window
//!   holding a node of any class under `androidx.compose.` is taken to send
//!   no events (spike S1): `androidx.compose.ui.platform.AndroidComposeView`
//!   in spike S1, `androidx.compose.ui.platform.ComposeView` on the API 36
//!   emulator. Simulator drivers set `Some(false)` for every window.
//! - `owner`: [`Owner::System`] with the name it indexes; absent,
//!   [`Owner::Target`].
//! - `modal`: the window cuts the application windows below it.

use std::time::{Duration, Instant};

use protocol::event::{Event, Stream};
use protocol::message::{Rotation, TextMethod};
use protocol::snapshot::Bounds;
use protocol::snapshot::Snapshot as TreePayload;

pub use crate::geometry::Point;

/// A driver's failure, as the protocol defines errors: a stable code, a
/// message, and whether the same call can succeed when repeated.
pub type BackendError = protocol::Error;

/// Where the mirror takes the time from.
pub trait Clock: Send {
    fn now(&self) -> Instant;
}

/// The system's monotonic clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Everything a mirror needs from one target.
pub struct Backend {
    pub tree: Box<dyn TreeSource>,
    pub events: Box<dyn EventStream>,
    /// Absent on targets without a frame path, such as Android devices,
    /// which take screenshots on demand instead.
    pub frames: Option<Box<dyn FrameSource>>,
    pub input: Box<dyn InputSink>,
    pub clock: Box<dyn Clock>,
}

/// Reads a target's windows in full.
pub trait TreeSource: Send {
    /// Reads the windows `request` names, or all of them.
    ///
    /// The source may return more windows than requested, but every window
    /// it returns must be current as of the read: a window whose toolkit
    /// sends no change events (Jetpack Compose on Android, every Simulator
    /// window) is read with the platform's node cache cleared, and so is
    /// every window a read for [`ReadReason::Moved`] names. A requested
    /// window missing from the result is taken as gone.
    fn read(&mut self, request: &ReadRequest) -> Result<TreeRead, BackendError>;

    /// Whether [`TreeSource::probe`] reads nodes' bounds again from the
    /// target; `false` unless the source says otherwise.
    fn can_probe(&self) -> bool {
        false
    }

    /// Reads the bounds of `nodes`, nodes of the latest read, again from the
    /// target, past whatever cache still holds them where that read found
    /// them. Views an app moves without a change event keep their old
    /// bounds in a platform's node cache; a settle wait without frames
    /// probes a few anchor nodes to see them move. Answers each node still on screen with its bounds in
    /// physical pixels; a node missing from the answer is gone. Called only
    /// when [`TreeSource::can_probe`] says so.
    fn probe(&mut self, nodes: &[NodeHandle]) -> Result<Vec<ProbedNode>, BackendError> {
        let _ = nodes;
        Err(BackendError::new(
            protocol::code::CAPABILITY_MISSING,
            "this tree source cannot read nodes' bounds again",
        ))
    }
}

/// A node's bounds as a probe read them again, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbedNode {
    pub node: NodeHandle,
    pub bounds: Bounds,
}

/// What the mirror asks a [`TreeSource`] to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadRequest {
    /// Window ids to read; `None` reads every window.
    pub windows: Option<Vec<u32>>,
    /// Why the mirror reads, so a driver can decide what caches to drop.
    pub reason: ReadReason,
}

/// Why the mirror reads the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReadReason {
    /// A window that sends no change events is due for its periodic poll.
    Poll,
    /// Frames changed since a window that sends no change events was read.
    Frames,
    /// Bounds probes found the window's nodes moved without a change event,
    /// which leaves the platform's node cache holding their old bounds: the
    /// source reads the window with that cache cleared.
    Moved,
    /// Change events named the window, or the set of windows changed.
    Events,
    /// The mirror injected input and re-reads the windows that send no
    /// change events.
    Action,
    /// No replica exists yet: the first acquisition after attach.
    Attach,
    /// The replica is rebuilt after a sequence gap, a reconnection or a
    /// switch of tree backend; refs issued before it are stale.
    Resync,
}

impl ReadReason {
    /// The snake_case name, as reports print it.
    pub fn name(self) -> &'static str {
        match self {
            ReadReason::Poll => "poll",
            ReadReason::Frames => "frames",
            ReadReason::Moved => "moved",
            ReadReason::Events => "events",
            ReadReason::Action => "action",
            ReadReason::Attach => "attach",
            ReadReason::Resync => "resync",
        }
    }
}

/// One read of a target's windows.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRead {
    /// The windows and their nodes, bounds in physical pixels, with each
    /// window's metadata and each scrolling node's scroll semantics. `seq`
    /// is the last tree-stream event the target had sent when it read the
    /// tree: the mirror skips events up to it, since the read already
    /// reflects them.
    pub snapshot: TreePayload,
    /// The backend that produced the read. A read from another backend than
    /// the previous one makes the mirror rebuild its replica.
    pub backend: BackendInfo,
    /// Physical pixels per device point.
    pub scale: f64,
}

/// Who owns a window, from the snapshot's window metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Owner {
    /// The application under automation.
    #[default]
    Target,
    /// System UI, named by its package or bundle id, such as
    /// `com.apple.springboard` for a permission alert.
    System(String),
}

/// The backend that produced a read.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BackendInfo {
    /// A stable name such as `android-agent`, `simulator-reader` or
    /// `simulator-translator`.
    pub name: String,
    /// Whether this is a fallback with known gaps; its snapshots are marked
    /// degraded.
    pub fallback: bool,
}

impl BackendInfo {
    pub fn new(name: impl Into<String>, fallback: bool) -> BackendInfo {
        BackendInfo {
            name: name.into(),
            fallback,
        }
    }
}

/// Everything a target reports on its own, in arrival order.
pub trait EventStream: Send {
    /// The next item, waiting until `deadline` at most, or `None` once the
    /// deadline passes with nothing to report. With a deadline that has
    /// already passed it returns what is queued without waiting.
    fn next(&mut self, deadline: Instant) -> Option<Timed<StreamItem>>;
}

/// An item and the host time it arrived.
#[derive(Clone, Debug, PartialEq)]
pub struct Timed<T> {
    pub at: Instant,
    pub item: T,
}

/// Something a target reported.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamItem {
    /// A protocol event with its per-stream sequence number. Tree and window
    /// events mark the replica for a re-read; lifecycle warnings reach the
    /// snapshot verdict. A number that skips ahead on a lossless stream
    /// makes the mirror resynchronize.
    Event(Event),
    /// A new frame with its hash.
    Frame(FrameMeta),
    /// The driver found a lossless stream out of sync (the host session's
    /// `OutOfSync`); the mirror resynchronizes.
    OutOfSync {
        stream: Stream,
        expected: u64,
        received: u64,
    },
    /// The stream is in sync again from `seq` (the host session's
    /// `Resynced`).
    Resynced { stream: Stream, seq: u64 },
    /// The driver switched tree backends, for example from the Simulator's
    /// in-guest reader to the host translator; the mirror resynchronizes.
    BackendChanged(BackendInfo),
    /// A new connection to the target: sequence numbers start again at 1
    /// and the mirror resynchronizes.
    Reconnected,
    /// The target is gone; the message says why and how to reconnect.
    Disconnected(String),
}

/// A frame's identity, timing and hash, without its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameMeta {
    /// Increasing per frame source; it may skip numbers.
    pub seq: u64,
    /// Host time of the capture.
    pub at: Instant,
    /// Hash of a low-resolution thumbnail with the status bar left out
    /// (spike S7), so that equal hashes mean no visible change. Frame
    /// callbacks fire without pixel changes (spike S2); only a changed hash
    /// counts as a change.
    pub hash: u64,
    /// Size of the captured pixels.
    pub width: u32,
    pub height: u32,
    /// Clockwise rotation that makes the captured pixels upright.
    pub rotation: Rotation,
}

/// Encodes frames. The mirror decides the size and quality; the source
/// owns the pixels and the codec.
pub trait FrameSource: Send {
    /// The most recent frame, if any.
    fn latest(&self) -> Option<FrameMeta>;

    /// Encodes frame `request.seq` as JPEG, rotated by `request.rotation`
    /// and then scaled to exactly `request.width` by `request.height`.
    fn encode(&mut self, request: &EncodeRequest) -> Result<Vec<u8>, BackendError>;
}

/// How the mirror wants a frame encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeRequest {
    pub seq: u64,
    /// Output size after rotation, in pixels.
    pub width: u32,
    pub height: u32,
    /// JPEG quality, 1 to 100.
    pub quality: u8,
    /// Clockwise rotation to apply before scaling.
    pub rotation: Rotation,
}

/// Injects input.
pub trait InputSink: Send {
    fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError>;
}

/// How the finger of a swipe leaves the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Release {
    /// It lifts at the end of its path, still moving, as after a flick: the
    /// platform flings the content on at the finger's speed.
    #[default]
    Moving,
    /// It comes to rest at the end of its path before it lifts, so the
    /// content stops where the finger left it (task 10.16). How a driver
    /// makes its platform see the finger stop is its own: Android's velocity
    /// tracker takes a pointer that has not moved for 40 ms as stopped,
    /// while UIKit takes a drag's velocity from its last moves.
    Resting,
}

/// One input action, coordinates in device points.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A tap, or a long press when `hold` is present.
    Tap { at: Point, hold: Option<Duration> },
    /// A stroke from `from` to `to` over `duration`, its finger leaving the
    /// screen as `release` says.
    Swipe {
        from: Point,
        to: Point,
        duration: Duration,
        release: Release,
    },
    /// A platform key code, such as Android's `KEYCODE_ENTER` (66).
    Key { code: u32, meta: Option<u32> },
    /// Text for `node`, or for the focused field when absent.
    Text {
        text: String,
        node: Option<NodeHandle>,
        method: TextMethod,
    },
}

/// A node as the latest read of its window numbered it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeHandle {
    pub window: u32,
    pub node: u32,
}

/// What the platform said about an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputAck {
    /// Whether the platform accepted the events.
    pub accepted: bool,
    /// For text, the way it went in.
    pub method: Option<TextMethod>,
}
