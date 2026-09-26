//! The message envelope (design D4): hello, welcome, request, response,
//! event, ping and pong.
//!
//! Every message except an event is a map keyed by small integers whose key 0
//! is the message kind ([`kind`]); an event is a positional array whose first
//! element is the kind.

use core::fmt;

use crate::capability::Capabilities;
use crate::error::Error;
use crate::event::{Event, Stream};
use crate::snapshot::Snapshot;

/// Message kinds: key 0 of a map-shaped message, element 0 of an event.
pub mod kind {
    pub const HELLO: u8 = 0;
    pub const WELCOME: u8 = 1;
    pub const REQUEST: u8 = 2;
    pub const RESPONSE: u8 = 3;
    pub const EVENT: u8 = 4;
    pub const PING: u8 = 5;
    pub const PONG: u8 = 6;
}

/// One message, the payload of one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Hello(Hello),
    Welcome(Welcome),
    Request(Request),
    Response(Response),
    Event(Event),
    Ping(Ping),
    Pong(Pong),
}

impl Message {
    /// The kind this message carries on the wire.
    pub fn kind(&self) -> u8 {
        match self {
            Message::Hello(_) => kind::HELLO,
            Message::Welcome(_) => kind::WELCOME,
            Message::Request(_) => kind::REQUEST,
            Message::Response(_) => kind::RESPONSE,
            Message::Event(_) => kind::EVENT,
            Message::Ping(_) => kind::PING,
            Message::Pong(_) => kind::PONG,
        }
    }

    /// Appends the message's CBOR encoding to `out`.
    pub fn encode(&self, _out: &mut Vec<u8>) {
        todo!("encode {:?}", self.kind())
    }

    /// The message's CBOR encoding.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode(&mut out);
        out
    }

    /// Decodes one message that fills all of `bytes`.
    pub fn decode(_bytes: &[u8]) -> Result<Message, DecodeError> {
        todo!("decode")
    }
}

/// Why a payload could not be decoded.
#[derive(Debug)]
pub enum DecodeError {
    /// Malformed CBOR, or a value of the wrong type or shape.
    Cbor(minicbor::decode::Error),
    /// A message kind this version does not know. Sessions skip these.
    UnknownKind(u8),
    /// The message ended before the payload did.
    TrailingBytes { consumed: usize, len: usize },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Cbor(e) => write!(f, "invalid message: {e}"),
            DecodeError::UnknownKind(k) => write!(f, "unknown message kind {k}"),
            DecodeError::TrailingBytes { consumed, len } => write!(
                f,
                "{} bytes after the message, which ends at byte {consumed} of {len}",
                len - consumed
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<minicbor::decode::Error> for DecodeError {
    fn from(e: minicbor::decode::Error) -> DecodeError {
        DecodeError::Cbor(e)
    }
}

/// The agent's first message on every connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// Protocol versions the agent speaks.
    pub versions: Vec<u16>,
    pub capabilities: Capabilities,
    /// The largest frame payload the agent accepts, in bytes.
    pub max_frame: u32,
    /// Agent name and version, such as `mobile-agent-android 0.1.0`.
    pub agent: String,
    /// The per-launch token the host passed when it started the agent.
    pub token: Option<String>,
}

/// The host's answer to a hello: the negotiated version and frame size, or
/// the reason it refuses the connection before closing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// Protocol versions the host speaks.
    pub versions: Vec<u16>,
    /// The selected version: the highest both sides speak. Absent on refusal.
    pub version: Option<u16>,
    /// The negotiated maximum frame payload: the smaller of both sides'.
    pub max_frame: u32,
    /// Why the host refuses the connection.
    pub error: Option<Error>,
}

/// A request from the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Assigned by the host; the response echoes it.
    pub id: u32,
    /// Relative deadline in milliseconds.
    pub deadline_ms: Option<u32>,
    /// The method and its parameters.
    pub call: Call,
}

/// The answer to one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The request's id.
    pub id: u32,
    /// The request's method, echoed so the result decodes on its own.
    pub method: Method,
    pub result: Result<Reply, Error>,
}

/// A liveness probe; the peer answers with a [`Pong`] carrying the same nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ping {
    pub nonce: u64,
}

/// The answer to a [`Ping`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pong {
    pub nonce: u64,
}

wire_enum! {
    /// A request method, `u16` on the wire.
    pub struct Method(u16);
    /// Read the UI tree ([`SnapshotParams`] to [`Snapshot`]); needs `tree`.
    SNAPSHOT = 1, "snapshot";
    /// Capture the screen ([`ScreenshotParams`] to [`Screenshot`]); needs `screenshot`.
    SCREENSHOT = 2, "screenshot";
    /// Tap or long-press ([`TapParams`] to [`InputAck`]); needs `input`.
    TAP = 3, "tap";
    /// Swipe ([`SwipeParams`] to [`InputAck`]); needs `input`.
    SWIPE = 4, "swipe";
    /// Press a key ([`KeyParams`] to [`InputAck`]); needs `input`.
    KEY = 5, "key";
    /// Set text in a field ([`TextParams`] to [`TextAck`]); needs `text_set`.
    TEXT_SET = 6, "text_set";
    /// Start frame streaming ([`FramesParams`] to an empty result); needs `frames`.
    FRAMES_START = 7, "frames_start";
    /// Stop frame streaming (no parameters, empty result); needs `frames`.
    FRAMES_STOP = 8, "frames_stop";
    /// Resynchronize a stream ([`ResyncParams`] to [`ResyncAck`]); needs `events`.
    RESYNC = 9, "resync";
    /// Exit cleanly (no parameters, empty result); needs nothing.
    SHUTDOWN = 10, "shutdown";
}

impl Method {
    /// The capability an agent must advertise before the host sends this
    /// method; empty for `shutdown` and for methods this version does not know.
    pub fn required_capability(self) -> Capabilities {
        match self {
            Method::SNAPSHOT => Capabilities::TREE,
            Method::SCREENSHOT => Capabilities::SCREENSHOT,
            Method::TAP | Method::SWIPE | Method::KEY => Capabilities::INPUT,
            Method::TEXT_SET => Capabilities::TEXT_SET,
            Method::FRAMES_START | Method::FRAMES_STOP => Capabilities::FRAMES,
            Method::RESYNC => Capabilities::EVENTS,
            _ => Capabilities::NONE,
        }
    }
}

/// A method with its parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Snapshot(SnapshotParams),
    Screenshot(ScreenshotParams),
    Tap(TapParams),
    Swipe(SwipeParams),
    Key(KeyParams),
    TextSet(TextParams),
    FramesStart(FramesParams),
    FramesStop,
    Resync(ResyncParams),
    Shutdown,
    /// A method this version does not know; its parameters were skipped and
    /// are sent as an empty map.
    Unknown(Method),
}

impl Call {
    pub fn method(&self) -> Method {
        match self {
            Call::Snapshot(_) => Method::SNAPSHOT,
            Call::Screenshot(_) => Method::SCREENSHOT,
            Call::Tap(_) => Method::TAP,
            Call::Swipe(_) => Method::SWIPE,
            Call::Key(_) => Method::KEY,
            Call::TextSet(_) => Method::TEXT_SET,
            Call::FramesStart(_) => Method::FRAMES_START,
            Call::FramesStop => Method::FRAMES_STOP,
            Call::Resync(_) => Method::RESYNC,
            Call::Shutdown => Method::SHUTDOWN,
            Call::Unknown(method) => *method,
        }
    }
}

/// The result of a successful request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// For `snapshot`.
    Snapshot(Snapshot),
    /// For `screenshot`.
    Screenshot(Screenshot),
    /// For `tap`, `swipe` and `key`.
    Input(InputAck),
    /// For `text_set`.
    Text(TextAck),
    /// For `resync`.
    Resync(ResyncAck),
    /// For `frames_start`, `frames_stop` and `shutdown`: an empty map.
    Empty,
}

/// Parameters of `snapshot`. Absent fields take the agent's defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnapshotParams {
    /// Deepest node level to read below each window root.
    pub max_depth: Option<u32>,
    /// Most nodes to read across all windows.
    pub max_nodes: Option<u32>,
    /// Longest wait for the UI to go idle before reading; 0 skips the wait.
    pub idle_ms: Option<u32>,
}

/// Parameters of `screenshot`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenshotParams {
    pub format: Option<ImageFormat>,
    /// JPEG quality, 1 to 100.
    pub quality: Option<u8>,
    /// Longest side of the returned image; larger screens are scaled down.
    pub max_dimension: Option<u32>,
}

/// Parameters of `tap`: a long press when `duration_ms` is present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapParams {
    pub x: i32,
    pub y: i32,
    pub duration_ms: Option<u32>,
}

/// Parameters of `swipe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwipeParams {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    pub duration_ms: u32,
}

/// Parameters of `key`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyParams {
    /// Platform key code, such as Android's `KEYCODE_ENTER` (66).
    pub code: u32,
    /// Platform meta-state bits.
    pub meta: Option<u32>,
}

/// Parameters of `text_set`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextParams {
    pub text: String,
    /// The node to set; absent for the focused field.
    pub node: Option<u32>,
    /// How to enter the text; absent lets the agent choose.
    pub mode: Option<TextMethod>,
}

/// Parameters of `frames_start`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FramesParams {
    pub max_fps: Option<u32>,
    pub max_dimension: Option<u32>,
}

/// Parameters of `resync`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResyncParams {
    pub stream: Stream,
}

/// The result of `screenshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screenshot {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    /// Display rotation when the image was taken.
    pub rotation: Rotation,
    /// The encoded image.
    pub data: Vec<u8>,
}

/// The result of an input method: whether the platform accepted the events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputAck {
    pub accepted: bool,
}

/// The result of `text_set`: which way the text went in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextAck {
    pub method: TextMethod,
}

/// The result of `resync`: the stream is in sync again from `seq + 1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResyncAck {
    pub stream: Stream,
    /// The last sequence number the agent sent on the stream before answering.
    pub seq: u64,
}

wire_enum! {
    /// Image encodings.
    pub struct ImageFormat(u8);
    PNG = 0, "png";
    JPEG = 1, "jpeg";
}

wire_enum! {
    /// Display rotation, with Android's `Surface.ROTATION_*` values.
    pub struct Rotation(u8);
    R0 = 0, "0";
    R90 = 1, "90";
    R180 = 2, "180";
    R270 = 3, "270";
}

wire_enum! {
    /// How text is entered.
    pub struct TextMethod(u8);
    /// Let the agent choose (requests only).
    AUTO = 0, "auto";
    /// Set the field's text directly.
    DIRECT = 1, "direct";
    /// Type it as key events.
    KEYS = 2, "keys";
}
