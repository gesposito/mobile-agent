//! Framing (spec: Framed self-describing messages): every message travels as
//! a 32-bit big-endian payload length followed by exactly one CBOR item of
//! that length.
//!
//! The limit applies to the payload length. Before the handshake each side
//! uses its own configured maximum; afterwards both use the negotiated one,
//! the smaller of the two sides' maxima.

use core::fmt;

use minicbor::Decoder;
use minicbor::data::Type;

use crate::message::{Message, kind};

/// Bytes in a frame header.
pub const HEADER_LEN: usize = 4;

/// The default maximum frame payload: 16 MiB, room for a full-resolution PNG
/// screenshot, as agent-device's bridge allows.
pub const DEFAULT_MAX_FRAME: u32 = 16 * 1024 * 1024;

/// The smallest maximum frame payload a side may negotiate: 4 KiB, so that
/// hellos, errors and pings always fit.
pub const MIN_MAX_FRAME: u32 = 4 * 1024;

/// How many payload bytes of an oversized frame are read to find its kind
/// and request id before the rest is discarded.
const PEEK_LEN: usize = 32;

/// Appends `msg` to `out` as one frame. Refuses a payload larger than `limit`,
/// leaving `out` as it was.
pub fn write_frame(msg: &Message, limit: u32, out: &mut Vec<u8>) -> Result<(), FrameTooLarge> {
    let start = out.len();
    out.extend_from_slice(&[0; HEADER_LEN]);
    msg.encode(out);
    let size = out.len() - start - HEADER_LEN;
    match u32::try_from(size) {
        Ok(len) if len <= limit => {
            out[start..start + HEADER_LEN].copy_from_slice(&len.to_be_bytes());
            Ok(())
        }
        _ => {
            out.truncate(start);
            Err(FrameTooLarge { size, limit })
        }
    }
}

/// A message whose payload is larger than the frame limit, refused by the sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTooLarge {
    /// Payload bytes.
    pub size: usize,
    pub limit: u32,
}

impl fmt::Display for FrameTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a frame of {} bytes exceeds the maximum frame size of {} bytes",
            self.size, self.limit
        )
    }
}

impl std::error::Error for FrameTooLarge {}

/// An incoming frame whose declared length exceeds the limit. Its payload is
/// discarded, never delivered; the kind and request id come from its first
/// bytes when they can be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Oversized {
    /// The declared payload length.
    pub size: u32,
    pub limit: u32,
    /// The message kind, if the payload starts like a message.
    pub kind: Option<u8>,
    /// The request id of a request or response.
    pub id: Option<u32>,
}

impl fmt::Display for Oversized {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a frame of {} bytes", self.size)?;
        match (self.kind, self.id) {
            (Some(k), Some(id)) => write!(f, " ({} for request {id})", kind::name(k))?,
            (Some(k), None) => write!(f, " ({})", kind::name(k))?,
            (None, _) => {}
        }
        write!(
            f,
            " exceeds the maximum frame size of {} bytes; its payload was discarded",
            self.limit
        )
    }
}

impl std::error::Error for Oversized {}

/// Splits a byte stream into frame payloads, buffering partial frames.
#[derive(Debug)]
pub struct FrameDecoder {
    limit: u32,
    buf: Vec<u8>,
    /// Where the unconsumed bytes of `buf` start.
    start: usize,
    /// Bytes of an oversized frame still to drop as they arrive.
    discard: u64,
}

impl FrameDecoder {
    pub fn new(limit: u32) -> FrameDecoder {
        FrameDecoder {
            limit,
            buf: Vec::new(),
            start: 0,
            discard: 0,
        }
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// Applies `limit` to frames whose header has not been read yet.
    pub fn set_limit(&mut self, limit: u32) {
        self.limit = limit;
    }

    /// Adds bytes read from the stream.
    pub fn push(&mut self, bytes: &[u8]) {
        let mut bytes = bytes;
        if self.discard > 0 {
            let n = usize::try_from(self.discard).map_or(bytes.len(), |d| d.min(bytes.len()));
            self.discard -= n as u64;
            bytes = &bytes[n..];
        }
        if bytes.is_empty() {
            return;
        }
        if self.start > 0 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete payload, an oversized frame's report, or `None` until
    /// more bytes arrive.
    pub fn next_frame(&mut self) -> Option<Result<&[u8], Oversized>> {
        if self.discard > 0 || self.buf.len() - self.start < HEADER_LEN {
            return None;
        }
        let body = self.start + HEADER_LEN;
        let header: [u8; HEADER_LEN] = self.buf[self.start..body]
            .try_into()
            .expect("the header is HEADER_LEN bytes");
        let declared = u32::from_be_bytes(header);
        let len = declared as usize;
        let have = self.buf.len() - body;
        if declared > self.limit {
            let wanted = len.min(PEEK_LEN);
            if have < wanted {
                return None;
            }
            let (kind, id) = peek(&self.buf[body..body + wanted]);
            let taken = have.min(len);
            self.start = body + taken;
            self.discard = u64::from(declared) - taken as u64;
            return Some(Err(Oversized {
                size: declared,
                limit: self.limit,
                kind,
                id,
            }));
        }
        if have < len {
            self.buf.reserve(len - have);
            return None;
        }
        self.start = body + len;
        Some(Ok(&self.buf[body..body + len]))
    }

    /// Whether the decoder is dropping the rest of an oversized frame.
    pub fn is_discarding(&self) -> bool {
        self.discard > 0
    }

    /// Bytes held for frames not yet complete.
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.start
    }
}

/// The kind and, for a request or response, the id at the start of a payload:
/// key 0 and key 1 of a map, which the encoders write first, or element 0 of
/// an array. Whatever cannot be read is `None`.
fn peek(prefix: &[u8]) -> (Option<u8>, Option<u32>) {
    let mut d = Decoder::new(prefix);
    match d.datatype() {
        Ok(Type::Map | Type::MapIndef) => {
            let Ok(len) = d.map() else {
                return (None, None);
            };
            let mut found_kind = None;
            let mut id = None;
            for _ in 0..len.unwrap_or(u64::MAX).min(4) {
                let Ok(key) = d.i64() else { break };
                let read = match key {
                    0 => d.u8().map(|k| found_kind = Some(k)),
                    1 => d.u32().map(|v| id = Some(v)),
                    _ => d.skip(),
                };
                if read.is_err() || (found_kind.is_some() && id.is_some()) {
                    break;
                }
            }
            let id = id.filter(|_| matches!(found_kind, Some(kind::REQUEST | kind::RESPONSE)));
            (found_kind, id)
        }
        Ok(Type::Array | Type::ArrayIndef) => match d.array() {
            Ok(_) => (d.u8().ok(), None),
            Err(_) => (None, None),
        },
        _ => (None, None),
    }
}
