//! Framing (spec: Framed self-describing messages): every message travels as
//! a 32-bit big-endian payload length followed by exactly one CBOR item of
//! that length.
//!
//! The limit applies to the payload length. Before the handshake each side
//! uses its own configured maximum; afterwards both use the negotiated one,
//! the smaller of the two sides' maxima.

use core::fmt;

use crate::message::Message;

/// Bytes in a frame header.
pub const HEADER_LEN: usize = 4;

/// The default maximum frame payload: 16 MiB, room for a full-resolution PNG
/// screenshot, as agent-device's bridge allows.
pub const DEFAULT_MAX_FRAME: u32 = 16 * 1024 * 1024;

/// The smallest maximum frame payload a side may negotiate: 4 KiB, so that
/// hellos, errors and pings always fit.
pub const MIN_MAX_FRAME: u32 = 4 * 1024;

/// Appends `msg` to `out` as one frame. Refuses a payload larger than `limit`,
/// leaving `out` as it was.
pub fn write_frame(_msg: &Message, _limit: u32, _out: &mut Vec<u8>) -> Result<(), FrameTooLarge> {
    todo!("write_frame")
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
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("Oversized display")
    }
}

impl std::error::Error for Oversized {}

/// Splits a byte stream into frame payloads, buffering partial frames.
#[derive(Debug)]
pub struct FrameDecoder {
    limit: u32,
}

impl FrameDecoder {
    pub fn new(limit: u32) -> FrameDecoder {
        FrameDecoder { limit }
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// Applies `limit` to frames whose header has not been read yet.
    pub fn set_limit(&mut self, limit: u32) {
        self.limit = limit;
    }

    /// Adds bytes read from the stream.
    pub fn push(&mut self, _bytes: &[u8]) {
        todo!("push")
    }

    /// The next complete payload, an oversized frame's report, or `None` until
    /// more bytes arrive.
    pub fn next_frame(&mut self) -> Option<Result<&[u8], Oversized>> {
        todo!("next_frame")
    }

    /// Whether the decoder is dropping the rest of an oversized frame.
    pub fn is_discarding(&self) -> bool {
        todo!("is_discarding")
    }

    /// Bytes held for frames not yet complete.
    pub fn buffered(&self) -> usize {
        todo!("buffered")
    }
}
