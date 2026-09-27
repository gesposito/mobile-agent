//! The message envelope (design D4): hello, welcome, request, response,
//! event, ping and pong.
//!
//! Every message except an event is a map keyed by small integers whose key 0
//! is the message kind ([`kind`]); an event is a positional array whose first
//! element is the kind.

use core::fmt;

use minicbor::data::Type;
use minicbor::decode::{self, Decoder};
use minicbor::encode::{self, Encoder, Write};
use minicbor::{Decode, Encode};

use crate::capability::Capabilities;
use crate::cbor::{self, Elements};
use crate::error::Error;
use crate::event::{Event, Stream};
use crate::snapshot::{Bounds, Snapshot};

/// Message kinds: key 0 of a map-shaped message, element 0 of an event.
pub mod kind {
    pub const HELLO: u8 = 0;
    pub const WELCOME: u8 = 1;
    pub const REQUEST: u8 = 2;
    pub const RESPONSE: u8 = 3;
    pub const EVENT: u8 = 4;
    pub const PING: u8 = 5;
    pub const PONG: u8 = 6;

    /// The name of a kind, for messages about frames.
    pub fn name(kind: u8) -> &'static str {
        match kind {
            HELLO => "hello",
            WELCOME => "welcome",
            REQUEST => "request",
            RESPONSE => "response",
            EVENT => "event",
            PING => "ping",
            PONG => "pong",
            _ => "unknown kind",
        }
    }
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
    pub fn encode(&self, out: &mut Vec<u8>) {
        minicbor::encode(self, out).expect("encoding into a Vec cannot fail");
    }

    /// The message's CBOR encoding.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode(&mut out);
        out
    }

    /// Decodes one message that fills all of `bytes`.
    pub fn decode(bytes: &[u8]) -> Result<Message, DecodeError> {
        let mut d = Decoder::new(bytes);
        let msg = match peek_kind(&d)? {
            kind::HELLO => Message::Hello(d.decode()?),
            kind::WELCOME => Message::Welcome(d.decode()?),
            kind::REQUEST => Message::Request(d.decode()?),
            kind::RESPONSE => Message::Response(d.decode()?),
            kind::EVENT => Message::Event(d.decode()?),
            kind::PING => Message::Ping(d.decode()?),
            kind::PONG => Message::Pong(d.decode()?),
            other => return Err(DecodeError::UnknownKind(other)),
        };
        if d.position() != bytes.len() {
            return Err(DecodeError::TrailingBytes {
                consumed: d.position(),
                len: bytes.len(),
            });
        }
        Ok(msg)
    }
}

/// Reads the kind of the message at the decoder's position without moving it:
/// element 0 of an array, key 0 of a map.
fn peek_kind(d: &Decoder<'_>) -> Result<u8, decode::Error> {
    let mut p = d.clone();
    match p.datatype()? {
        Type::Array | Type::ArrayIndef => {
            let mut items = Elements::begin(&mut p)?;
            items.require(&mut p, 0, "kind", |d| d.u8())
        }
        Type::Map | Type::MapIndef => {
            // Key 0 comes first in the encoders' layout; look further for others.
            let len = p.map()?;
            let mut seen = 0;
            loop {
                let more = match len {
                    Some(n) => seen < n,
                    None => p.datatype()? != Type::Break,
                };
                if !more {
                    return Err(cbor::missing(0, "kind").at(d.position()));
                }
                if p.i64()? == 0 {
                    return p.u8();
                }
                p.skip()?;
                seen += 1;
            }
        }
        other => Err(decode::Error::type_mismatch(other)
            .with_message("a message is a map or an array")
            .at(d.position())),
    }
}

impl<C> Encode<C> for Message {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        match self {
            Message::Hello(m) => m.encode(e, ctx),
            Message::Welcome(m) => m.encode(e, ctx),
            Message::Request(m) => m.encode(e, ctx),
            Message::Response(m) => m.encode(e, ctx),
            Message::Event(m) => m.encode(e, ctx),
            Message::Ping(m) => m.encode(e, ctx),
            Message::Pong(m) => m.encode(e, ctx),
        }
    }
}

impl From<Hello> for Message {
    fn from(m: Hello) -> Message {
        Message::Hello(m)
    }
}

impl From<Welcome> for Message {
    fn from(m: Welcome) -> Message {
        Message::Welcome(m)
    }
}

impl From<Request> for Message {
    fn from(m: Request) -> Message {
        Message::Request(m)
    }
}

impl From<Response> for Message {
    fn from(m: Response) -> Message {
        Message::Response(m)
    }
}

impl From<Event> for Message {
    fn from(m: Event) -> Message {
        Message::Event(m)
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
///
/// On the wire: `{0: 0, 1: versions, 2: capabilities, 3: max_frame, 4: agent, ?5: token}`.
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
///
/// On the wire: `{0: 1, 1: versions, ?2: version, 3: max_frame, ?4: error}`.
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
///
/// On the wire: `{0: 2, 1: id, 2: method, ?3: deadline_ms, 4: params}`, where
/// params is a map, empty for methods without parameters.
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
///
/// On the wire: `{0: 3, 1: id, 2: method, 3: result}` or
/// `{0: 3, 1: id, 2: method, 4: error}`; an empty result is an empty map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The request's id.
    pub id: u32,
    /// The request's method, echoed so the result decodes on its own.
    pub method: Method,
    pub result: Result<Reply, Error>,
}

/// A liveness probe; the peer answers with a [`Pong`] carrying the same nonce.
///
/// On the wire: `{0: 5, 1: nonce}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ping {
    pub nonce: u64,
}

/// The answer to a [`Ping`].
///
/// On the wire: `{0: 6, 1: nonce}`.
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
    /// Read nodes' bounds again past the platform's node cache
    /// ([`BoundsParams`] to [`Probed`]); needs `bounds`.
    BOUNDS = 11, "bounds";
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
            Method::BOUNDS => Capabilities::BOUNDS,
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
    Bounds(BoundsParams),
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
            Call::Bounds(_) => Method::BOUNDS,
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
    /// For `bounds`.
    Bounds(Probed),
    /// For `frames_start`, `frames_stop` and `shutdown`: an empty map.
    Empty,
}

/// Parameters of `snapshot`. Absent fields take the agent's defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct SnapshotParams {
    /// Deepest node level to read below each window root.
    #[n(0)]
    pub max_depth: Option<u32>,
    /// Most nodes to read across all windows.
    #[n(1)]
    pub max_nodes: Option<u32>,
    /// Longest wait for the UI to go idle before reading; 0 skips the wait.
    #[n(2)]
    pub idle_ms: Option<u32>,
    /// Evict the platform's node cache of the windows that send no change
    /// events before reading: input the agent did not inject, such as the
    /// emulator controller's, left them stale. The tree stream and every
    /// other window's cache are left alone, unlike a resync. An agent that
    /// does not know the key skips it and reads with its cache.
    #[n(3)]
    pub evict_event_less: Option<bool>,
    /// Windows whose nodes the agent evicts from the platform's node cache
    /// before reading, by window id: bounds probes found their nodes moved
    /// without a change event, which leaves the cache holding the old
    /// bounds. An agent that does not know the key skips it and reads with
    /// its cache.
    #[n(4)]
    pub evict: Option<Vec<u32>>,
}

/// Parameters of `screenshot`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct ScreenshotParams {
    #[n(0)]
    pub format: Option<ImageFormat>,
    /// JPEG quality, 1 to 100.
    #[n(1)]
    pub quality: Option<u8>,
    /// Longest side of the returned image; larger screens are scaled down.
    #[n(2)]
    pub max_dimension: Option<u32>,
}

/// Parameters of `tap`: a long press when `duration_ms` is present.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct TapParams {
    #[n(0)]
    pub x: i32,
    #[n(1)]
    pub y: i32,
    #[n(2)]
    pub duration_ms: Option<u32>,
}

/// Parameters of `swipe`.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct SwipeParams {
    #[n(0)]
    pub x1: i32,
    #[n(1)]
    pub y1: i32,
    #[n(2)]
    pub x2: i32,
    #[n(3)]
    pub y2: i32,
    #[n(4)]
    pub duration_ms: u32,
}

/// Parameters of `key`.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct KeyParams {
    /// Platform key code, such as Android's `KEYCODE_ENTER` (66).
    #[n(0)]
    pub code: u32,
    /// Platform meta-state bits.
    #[n(1)]
    pub meta: Option<u32>,
}

/// Parameters of `text_set`.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct TextParams {
    #[n(0)]
    pub text: String,
    /// The node to set; absent for the focused field.
    #[n(1)]
    pub node: Option<u32>,
    /// How to enter the text; absent lets the agent choose.
    #[n(2)]
    pub mode: Option<TextMethod>,
}

/// Parameters of `frames_start`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct FramesParams {
    #[n(0)]
    pub max_fps: Option<u32>,
    #[n(1)]
    pub max_dimension: Option<u32>,
}

/// Parameters of `resync`.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct ResyncParams {
    #[n(0)]
    pub stream: Stream,
}

/// Parameters of `bounds`: nodes of the last snapshot, by id, whose bounds
/// the agent reads again past the platform's node cache.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct BoundsParams {
    #[cbor(n(0), decode_with = "crate::cbor::decode_vec")]
    pub nodes: Vec<u32>,
}

/// The result of `bounds`: each node still on screen with its bounds as the
/// app reports them now, in the request's order; a node that is gone, or
/// that the last snapshot did not hold, is left out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Probed {
    #[cbor(n(0), decode_with = "crate::cbor::decode_vec")]
    pub nodes: Vec<NodeBounds>,
}

/// One node of a [`Probed`], on the wire `[node, bounds]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode)]
#[cbor(array)]
pub struct NodeBounds {
    /// The node's id in the last snapshot.
    #[n(0)]
    pub node: u32,
    /// Its bounds in screen pixels.
    #[n(1)]
    pub bounds: Bounds,
}

/// The result of `screenshot`.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Screenshot {
    #[n(0)]
    pub format: ImageFormat,
    #[n(1)]
    pub width: u32,
    #[n(2)]
    pub height: u32,
    /// Display rotation when the image was taken.
    #[n(3)]
    pub rotation: Rotation,
    /// The encoded image.
    #[cbor(n(4), with = "minicbor::bytes")]
    pub data: Vec<u8>,
}

/// The result of an input method: whether the platform accepted the events.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct InputAck {
    #[n(0)]
    pub accepted: bool,
}

/// The result of `text_set`: which way the text went in.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct TextAck {
    #[n(0)]
    pub method: TextMethod,
}

/// The result of `resync`: the stream is in sync again from `seq + 1`.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct ResyncAck {
    #[n(0)]
    pub stream: Stream,
    /// The last sequence number the agent sent on the stream before answering.
    #[n(1)]
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

impl<C> Encode<C> for Hello {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        e.map(5 + u64::from(self.token.is_some()))?;
        e.u8(0)?.u8(kind::HELLO)?;
        e.u8(1)?.encode_with(&self.versions, ctx)?;
        e.u8(2)?.encode_with(self.capabilities, ctx)?;
        e.u8(3)?.u32(self.max_frame)?;
        e.u8(4)?.str(&self.agent)?;
        if let Some(token) = &self.token {
            e.u8(5)?.str(token)?;
        }
        Ok(())
    }
}

impl<'b, C> Decode<'b, C> for Hello {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Hello, decode::Error> {
        let at = d.position();
        let mut k = None;
        let mut versions = None;
        let mut capabilities = None;
        let mut max_frame = None;
        let mut agent = None;
        let mut token = None;
        cbor::map_entries(d, |d, key| {
            match key {
                0 => k = Some(d.u8()?),
                1 => versions = Some(cbor::decode_vec(d, &mut *ctx)?),
                2 => capabilities = Some(Capabilities::decode(d, &mut *ctx)?),
                3 => max_frame = Some(d.u32()?),
                4 => agent = Some(d.str()?.to_owned()),
                5 => token = Option::<String>::decode(d, &mut *ctx)?,
                _ => d.skip()?,
            }
            Ok(())
        })?;
        cbor::expect_kind(k, kind::HELLO, at)?;
        Ok(Hello {
            versions: versions.ok_or_else(|| cbor::missing(1, "versions").at(at))?,
            capabilities: capabilities.ok_or_else(|| cbor::missing(2, "capabilities").at(at))?,
            max_frame: max_frame.ok_or_else(|| cbor::missing(3, "max_frame").at(at))?,
            agent: agent.ok_or_else(|| cbor::missing(4, "agent").at(at))?,
            token,
        })
    }
}

impl<C> Encode<C> for Welcome {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        e.map(3 + u64::from(self.version.is_some()) + u64::from(self.error.is_some()))?;
        e.u8(0)?.u8(kind::WELCOME)?;
        e.u8(1)?.encode_with(&self.versions, ctx)?;
        if let Some(version) = self.version {
            e.u8(2)?.u16(version)?;
        }
        e.u8(3)?.u32(self.max_frame)?;
        if let Some(error) = &self.error {
            e.u8(4)?.encode_with(error, ctx)?;
        }
        Ok(())
    }
}

impl<'b, C> Decode<'b, C> for Welcome {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Welcome, decode::Error> {
        let at = d.position();
        let mut k = None;
        let mut versions = None;
        let mut version = None;
        let mut max_frame = None;
        let mut error = None;
        cbor::map_entries(d, |d, key| {
            match key {
                0 => k = Some(d.u8()?),
                1 => versions = Some(cbor::decode_vec(d, &mut *ctx)?),
                2 => version = Option::<u16>::decode(d, &mut *ctx)?,
                3 => max_frame = Some(d.u32()?),
                4 => error = Option::<Error>::decode(d, &mut *ctx)?,
                _ => d.skip()?,
            }
            Ok(())
        })?;
        cbor::expect_kind(k, kind::WELCOME, at)?;
        Ok(Welcome {
            versions: versions.ok_or_else(|| cbor::missing(1, "versions").at(at))?,
            version,
            max_frame: max_frame.ok_or_else(|| cbor::missing(3, "max_frame").at(at))?,
            error,
        })
    }
}

impl<C> Encode<C> for Request {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        e.map(4 + u64::from(self.deadline_ms.is_some()))?;
        e.u8(0)?.u8(kind::REQUEST)?;
        e.u8(1)?.u32(self.id)?;
        e.u8(2)?.u16(self.call.method().0)?;
        if let Some(ms) = self.deadline_ms {
            e.u8(3)?.u32(ms)?;
        }
        e.u8(4)?;
        match &self.call {
            Call::Snapshot(p) => p.encode(e, ctx)?,
            Call::Screenshot(p) => p.encode(e, ctx)?,
            Call::Tap(p) => p.encode(e, ctx)?,
            Call::Swipe(p) => p.encode(e, ctx)?,
            Call::Key(p) => p.encode(e, ctx)?,
            Call::TextSet(p) => p.encode(e, ctx)?,
            Call::FramesStart(p) => p.encode(e, ctx)?,
            Call::Resync(p) => p.encode(e, ctx)?,
            Call::Bounds(p) => p.encode(e, ctx)?,
            Call::FramesStop | Call::Shutdown | Call::Unknown(_) => {
                e.map(0)?;
            }
        }
        Ok(())
    }
}

impl<'b, C> Decode<'b, C> for Request {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Request, decode::Error> {
        let at = d.position();
        let mut k = None;
        let mut id = None;
        let mut method = None;
        let mut deadline_ms = None;
        let mut params_at = None;
        cbor::map_entries(d, |d, key| {
            match key {
                0 => k = Some(d.u8()?),
                1 => id = Some(d.u32()?),
                2 => method = Some(Method(d.u16()?)),
                3 => deadline_ms = Option::<u32>::decode(d, &mut *ctx)?,
                // Decoded below, once the method is known whatever the key order.
                4 => {
                    params_at = Some(d.position());
                    d.skip()?;
                }
                _ => d.skip()?,
            }
            Ok(())
        })?;
        cbor::expect_kind(k, kind::REQUEST, at)?;
        let id = id.ok_or_else(|| cbor::missing(1, "id").at(at))?;
        let method = method.ok_or_else(|| cbor::missing(2, "method").at(at))?;
        let params_at = params_at.ok_or_else(|| cbor::missing(4, "params").at(at))?;
        let mut params = d.clone();
        params.set_position(params_at);
        let call = Call::decode_params(method, &mut params, ctx)?;
        Ok(Request {
            id,
            deadline_ms,
            call,
        })
    }
}

impl Call {
    fn decode_params<'b, C>(
        method: Method,
        d: &mut Decoder<'b>,
        ctx: &mut C,
    ) -> Result<Call, decode::Error> {
        Ok(match method {
            Method::SNAPSHOT => Call::Snapshot(Decode::decode(d, ctx)?),
            Method::SCREENSHOT => Call::Screenshot(Decode::decode(d, ctx)?),
            Method::TAP => Call::Tap(Decode::decode(d, ctx)?),
            Method::SWIPE => Call::Swipe(Decode::decode(d, ctx)?),
            Method::KEY => Call::Key(Decode::decode(d, ctx)?),
            Method::TEXT_SET => Call::TextSet(Decode::decode(d, ctx)?),
            Method::FRAMES_START => Call::FramesStart(Decode::decode(d, ctx)?),
            Method::FRAMES_STOP => {
                cbor::skip_map(d)?;
                Call::FramesStop
            }
            Method::RESYNC => Call::Resync(Decode::decode(d, ctx)?),
            Method::SHUTDOWN => {
                cbor::skip_map(d)?;
                Call::Shutdown
            }
            Method::BOUNDS => Call::Bounds(Decode::decode(d, ctx)?),
            other => {
                d.skip()?;
                Call::Unknown(other)
            }
        })
    }
}

impl Reply {
    /// Whether this is the result type of `method`.
    pub fn answers(&self, method: Method) -> bool {
        matches!(
            (self, method),
            (Reply::Snapshot(_), Method::SNAPSHOT)
                | (Reply::Screenshot(_), Method::SCREENSHOT)
                | (Reply::Input(_), Method::TAP | Method::SWIPE | Method::KEY)
                | (Reply::Text(_), Method::TEXT_SET)
                | (Reply::Resync(_), Method::RESYNC)
                | (Reply::Bounds(_), Method::BOUNDS)
                | (
                    Reply::Empty,
                    Method::FRAMES_START | Method::FRAMES_STOP | Method::SHUTDOWN
                )
        )
    }

    fn decode_for<'b, C>(
        method: Method,
        d: &mut Decoder<'b>,
        ctx: &mut C,
    ) -> Result<Reply, decode::Error> {
        Ok(match method {
            Method::SNAPSHOT => Reply::Snapshot(Decode::decode(d, ctx)?),
            Method::SCREENSHOT => Reply::Screenshot(Decode::decode(d, ctx)?),
            Method::TAP | Method::SWIPE | Method::KEY => Reply::Input(Decode::decode(d, ctx)?),
            Method::TEXT_SET => Reply::Text(Decode::decode(d, ctx)?),
            Method::RESYNC => Reply::Resync(Decode::decode(d, ctx)?),
            Method::BOUNDS => Reply::Bounds(Decode::decode(d, ctx)?),
            Method::FRAMES_START | Method::FRAMES_STOP | Method::SHUTDOWN => {
                cbor::skip_map(d)?;
                Reply::Empty
            }
            other => {
                return Err(decode::Error::message(format!(
                    "a result for {other}, which this version does not know"
                ))
                .at(d.position()));
            }
        })
    }
}

impl<C> Encode<C> for Reply {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        match self {
            Reply::Snapshot(r) => r.encode(e, ctx),
            Reply::Screenshot(r) => r.encode(e, ctx),
            Reply::Input(r) => r.encode(e, ctx),
            Reply::Text(r) => r.encode(e, ctx),
            Reply::Resync(r) => r.encode(e, ctx),
            Reply::Bounds(r) => r.encode(e, ctx),
            Reply::Empty => {
                e.map(0)?;
                Ok(())
            }
        }
    }
}

impl<C> Encode<C> for Response {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        e.map(4)?;
        e.u8(0)?.u8(kind::RESPONSE)?;
        e.u8(1)?.u32(self.id)?;
        e.u8(2)?.u16(self.method.0)?;
        match &self.result {
            Ok(reply) => {
                debug_assert!(reply.answers(self.method), "{reply:?} for {}", self.method);
                e.u8(3)?.encode_with(reply, ctx)?;
            }
            Err(error) => {
                e.u8(4)?.encode_with(error, ctx)?;
            }
        }
        Ok(())
    }
}

impl<'b, C> Decode<'b, C> for Response {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Response, decode::Error> {
        let at = d.position();
        let mut k = None;
        let mut id = None;
        let mut method = None;
        let mut result_at = None;
        let mut error = None;
        cbor::map_entries(d, |d, key| {
            match key {
                0 => k = Some(d.u8()?),
                1 => id = Some(d.u32()?),
                2 => method = Some(Method(d.u16()?)),
                // Decoded below, once the method is known whatever the key order.
                3 => {
                    result_at = Some(d.position());
                    d.skip()?;
                }
                4 => error = Option::<Error>::decode(d, &mut *ctx)?,
                _ => d.skip()?,
            }
            Ok(())
        })?;
        cbor::expect_kind(k, kind::RESPONSE, at)?;
        let id = id.ok_or_else(|| cbor::missing(1, "id").at(at))?;
        let method = method.ok_or_else(|| cbor::missing(2, "method").at(at))?;
        let result = match (result_at, error) {
            (Some(result_at), None) => {
                let mut result = d.clone();
                result.set_position(result_at);
                Ok(Reply::decode_for(method, &mut result, ctx)?)
            }
            (None, Some(error)) => Err(error),
            (Some(_), Some(_)) => {
                return Err(
                    decode::Error::message("a response with both a result and an error").at(at),
                );
            }
            (None, None) => {
                return Err(decode::Error::message(
                    "a response with neither a result nor an error",
                )
                .at(at));
            }
        };
        Ok(Response { id, method, result })
    }
}

impl<C> Encode<C> for Ping {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        _: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        e.map(2)?.u8(0)?.u8(kind::PING)?.u8(1)?.u64(self.nonce)?;
        Ok(())
    }
}

impl<'b, C> Decode<'b, C> for Ping {
    fn decode(d: &mut Decoder<'b>, _: &mut C) -> Result<Ping, decode::Error> {
        decode_nonce(d, kind::PING).map(|nonce| Ping { nonce })
    }
}

impl<C> Encode<C> for Pong {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        _: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        e.map(2)?.u8(0)?.u8(kind::PONG)?.u8(1)?.u64(self.nonce)?;
        Ok(())
    }
}

impl<'b, C> Decode<'b, C> for Pong {
    fn decode(d: &mut Decoder<'b>, _: &mut C) -> Result<Pong, decode::Error> {
        decode_nonce(d, kind::PONG).map(|nonce| Pong { nonce })
    }
}

fn decode_nonce(d: &mut Decoder<'_>, expected: u8) -> Result<u64, decode::Error> {
    let at = d.position();
    let mut k = None;
    let mut nonce = None;
    cbor::map_entries(d, |d, key| {
        match key {
            0 => k = Some(d.u8()?),
            1 => nonce = Some(d.u64()?),
            _ => d.skip()?,
        }
        Ok(())
    })?;
    cbor::expect_kind(k, expected, at)?;
    nonce.ok_or_else(|| cbor::missing(1, "nonce").at(at))
}
