//! Frame access (spec: Frame access): the latest frame as a JPEG no larger
//! than a requested long edge, upright, with its capture time, its age and
//! its offset from the last tree read. The mirror decides size and
//! quality; the frame source owns the pixels and the codec.

use std::time::{Duration, Instant};

/// The long edge a frame is scaled to when the request names none, the
/// dimension spike S2 measured the encoders at.
pub const DEFAULT_MAX_DIMENSION: u32 = 1000;

/// The JPEG quality when the request names none, as spikes S1 and S2 used.
pub const DEFAULT_QUALITY: u8 = 80;

/// How to return a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameRequest {
    /// The longest edge of the image, in pixels; smaller frames are not
    /// enlarged.
    pub max_dimension: Option<u32>,
    /// JPEG quality, clamped to 1 to 100.
    pub quality: Option<u8>,
}

/// The latest frame, encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameImage {
    pub seq: u64,
    /// JPEG bytes.
    pub data: Vec<u8>,
    /// Size of the upright image, in pixels.
    pub width: u32,
    pub height: u32,
    pub quality: u8,
    /// Host time of the capture.
    pub captured_at: Instant,
    /// How long ago the frame was captured.
    pub age: Duration,
    /// When the frame was captured relative to the last tree read.
    pub tree_skew: TreeSkew,
}

/// A frame's capture time against the last tree read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeSkew {
    /// Captured this long after the tree was last read.
    FrameNewer(Duration),
    /// Captured this long before the tree was last read.
    FrameOlder(Duration),
    /// The tree has not been read yet.
    NoTree,
}
