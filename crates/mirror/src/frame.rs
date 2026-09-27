//! Frame access (spec: Frame access): the latest frame as a JPEG no larger
//! than a requested long edge, upright, with its capture time, its age and
//! its offset from the last tree read. The mirror decides size and
//! quality; the frame source owns the pixels and the codec.

use std::time::{Duration, Instant};

use protocol::message::Rotation;

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

impl TreeSkew {
    /// A frame captured at `frame` against a tree last read at `tree`.
    pub(crate) fn between(frame: Instant, tree: Option<Instant>) -> TreeSkew {
        match tree {
            None => TreeSkew::NoTree,
            Some(tree) if frame >= tree => TreeSkew::FrameNewer(frame - tree),
            Some(tree) => TreeSkew::FrameOlder(tree - frame),
        }
    }
}

/// The upright size of a `width` by `height` capture rotated by
/// `rotation`, scaled so its long edge is at most `max`, never enlarged.
pub(crate) fn fit(width: u32, height: u32, rotation: Rotation, max: u32) -> (u32, u32) {
    let (w, h) = if matches!(rotation, Rotation::R90 | Rotation::R270) {
        (height, width)
    } else {
        (width, height)
    };
    let long = w.max(h);
    if long <= max || long == 0 {
        return (w.max(1), h.max(1));
    }
    let scale = |edge: u32| {
        let scaled = (f64::from(edge) * f64::from(max) / f64::from(long)).round();
        (scaled as u32).clamp(1, max)
    };
    if w >= h {
        (max, scale(h))
    } else {
        (scale(w), max)
    }
}

impl FrameRequest {
    /// The long edge to fit, at least one pixel.
    pub(crate) fn max_dimension(&self) -> u32 {
        self.max_dimension.unwrap_or(DEFAULT_MAX_DIMENSION).max(1)
    }

    /// The JPEG quality, clamped to 1 to 100.
    pub(crate) fn quality(&self) -> u8 {
        self.quality.unwrap_or(DEFAULT_QUALITY).clamp(1, 100)
    }
}
