//! Raw frames from the controller and change detection over them.
//!
//! The controller streams a frame whenever the guest posts one, changed or not,
//! and the status bar clock changes on its own (spike S7). A frame is therefore
//! reduced to a key, its dimensions plus an xxh3 hash of its pixels without the
//! status bar rows, and a change is a key that differs from the previous one.

use std::fmt;

use prost::bytes::Bytes;

/// Bytes per pixel of the RGB888 frames the client requests.
pub const BYTES_PER_PIXEL: usize = 3;

/// A raw RGB888 frame.
///
/// Row 0 is the top of the screen. The proto documents raw rows as bottom-up,
/// but the emulator sends them top-down: S7 matched a raw frame read top-down
/// against the PNG of the same screen, and tap locations only lined up that way.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    width: u32,
    height: u32,
    pixels: Bytes,
    seq: u32,
    timestamp_us: u64,
}

/// Why bytes do not make a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Width or height is zero: the display is off or has not posted yet.
    Empty,
    /// The buffer is not `width * height * 3` bytes long.
    SizeMismatch { expected: usize, actual: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Empty => write!(f, "the frame has no pixels"),
            FrameError::SizeMismatch { expected, actual } => write!(
                f,
                "an RGB888 frame of these dimensions has {expected} bytes, got {actual}"
            ),
        }
    }
}

impl std::error::Error for FrameError {}

impl Frame {
    /// A frame of `pixels` in RGB888, rows top-down.
    pub fn new(width: u32, height: u32, pixels: Bytes) -> Result<Self, FrameError> {
        let _ = (width, height, pixels);
        todo!()
    }

    /// Sets the stream sequence number and the emulator's production time.
    pub fn with_timing(self, seq: u32, timestamp_us: u64) -> Self {
        Self {
            seq,
            timestamp_us,
            ..self
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// RGB888 pixels, `width * 3` bytes per row, top row first.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Row `y`, counted from the top of the screen.
    ///
    /// # Panics
    ///
    /// When `y` is not below the height.
    pub fn row(&self, y: u32) -> &[u8] {
        let _ = y;
        todo!()
    }

    /// The emulator's sequence number in its stream; gaps are frames it skipped.
    pub fn seq(&self) -> u32 {
        self.seq
    }

    /// When the emulator estimates it produced the frame, in microseconds since
    /// the Unix epoch; before the frame was copied, scaled and sent.
    pub fn timestamp_us(&self) -> u64 {
        self.timestamp_us
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.pixels.len())
            .field("seq", &self.seq)
            .field("timestamp_us", &self.timestamp_us)
            .finish()
    }
}

/// Identity of a frame's content outside the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameKey {
    pub width: u32,
    pub height: u32,
    pub hash: u64,
}

/// Hashes frame content without the status bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameHasher {
    status_bar_fraction: f32,
}

impl FrameHasher {
    /// The share of the screen height taken for the status bar when nothing
    /// better is known: 6 %, 144 of 2400 rows, which S7 found covers the Pixel 8
    /// profile's status bar.
    pub const DEFAULT_STATUS_BAR_FRACTION: f32 = 0.06;

    /// A hasher that leaves out the top `status_bar_fraction` of every frame,
    /// clamped to 0 through 1.
    pub fn new(status_bar_fraction: f32) -> Self {
        let _ = status_bar_fraction;
        todo!()
    }

    /// Rows at the top of a frame `height` rows high that the hash leaves out.
    pub fn status_bar_rows(&self, height: u32) -> u32 {
        let _ = height;
        todo!()
    }

    /// The key of `frame`.
    pub fn key(&self, frame: &Frame) -> FrameKey {
        let _ = frame;
        todo!()
    }
}

impl Default for FrameHasher {
    fn default() -> Self {
        Self::new(Self::DEFAULT_STATUS_BAR_FRACTION)
    }
}

/// What a frame means relative to the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    /// The first frame since the detector started or was reset.
    First,
    /// Same content as the previous frame outside the status bar.
    Unchanged,
    /// Different content, or different dimensions, from the previous frame.
    Changed,
}

/// Detects content changes in a sequence of frames, such as a frame stream.
/// Feeds the settle signal: frames keep arriving while nothing changes.
#[derive(Debug, Clone)]
pub struct ChangeDetector {
    hasher: FrameHasher,
    last: Option<FrameKey>,
}

impl ChangeDetector {
    pub fn new(hasher: FrameHasher) -> Self {
        Self { hasher, last: None }
    }

    /// Classifies `frame` against the previous frame and remembers it.
    pub fn observe(&mut self, frame: &Frame) -> Observation {
        let _ = frame;
        todo!()
    }

    /// Forgets the previous frame, so the next one is [`Observation::First`].
    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 4;
    const H: u32 = 50;

    fn pixels(fill: u8) -> Vec<u8> {
        vec![fill; W as usize * H as usize * BYTES_PER_PIXEL]
    }

    fn frame(data: Vec<u8>) -> Frame {
        Frame::new(W, H, Bytes::from(data)).unwrap()
    }

    /// Changes one pixel on screen row `row`, which is buffer row `row`.
    fn poke(data: &mut [u8], row: u32, col: u32) {
        data[(row * W + col) as usize * BYTES_PER_PIXEL] ^= 0xff;
    }

    #[test]
    fn a_frame_must_hold_width_times_height_rgb_pixels() {
        assert_eq!(
            Frame::new(0, 0, Bytes::new()).unwrap_err(),
            FrameError::Empty
        );
        assert_eq!(
            Frame::new(W, 0, Bytes::new()).unwrap_err(),
            FrameError::Empty
        );
        let short = pixels(1)[1..].to_vec();
        assert_eq!(
            Frame::new(W, H, Bytes::from(short)).unwrap_err(),
            FrameError::SizeMismatch {
                expected: 600,
                actual: 599
            }
        );
        // An RGBA buffer is not an RGB888 frame of the same size.
        let rgba = vec![0u8; W as usize * H as usize * 4];
        assert!(matches!(
            Frame::new(W, H, Bytes::from(rgba)),
            Err(FrameError::SizeMismatch { .. })
        ));
        let f = frame(pixels(1)).with_timing(7, 1_000);
        assert_eq!((f.width(), f.height(), f.pixels().len()), (W, H, 600));
        assert_eq!((f.seq(), f.timestamp_us()), (7, 1_000));
    }

    #[test]
    fn rows_are_top_down() {
        let mut data = pixels(0);
        data[..W as usize * BYTES_PER_PIXEL].fill(9);
        let f = frame(data);
        assert!(
            f.row(0).iter().all(|&b| b == 9),
            "row 0 is the first buffer row"
        );
        assert!(f.row(H - 1).iter().all(|&b| b == 0));
        assert_eq!(f.row(1).len(), W as usize * BYTES_PER_PIXEL);
    }

    #[test]
    fn the_status_bar_share_becomes_whole_rows() {
        let hasher = FrameHasher::default();
        assert_eq!(hasher.status_bar_rows(2400), 144);
        assert_eq!(hasher.status_bar_rows(1200), 72);
        assert_eq!(FrameHasher::new(0.0).status_bar_rows(1200), 0);
        assert_eq!(FrameHasher::new(2.0).status_bar_rows(1200), 1200);
        assert_eq!(FrameHasher::new(-1.0).status_bar_rows(1200), 0);
    }

    #[test]
    fn the_first_frame_then_identical_content_is_unchanged() {
        let mut d = ChangeDetector::new(FrameHasher::new(0.0));
        assert_eq!(d.observe(&frame(pixels(10))), Observation::First);
        assert_eq!(d.observe(&frame(pixels(10))), Observation::Unchanged);
    }

    #[test]
    fn one_changed_pixel_is_a_change_and_becomes_the_new_reference() {
        let a = pixels(10);
        let mut b = a.clone();
        poke(&mut b, 30, 3);
        let mut d = ChangeDetector::new(FrameHasher::new(0.0));
        d.observe(&frame(a.clone()));
        assert_eq!(d.observe(&frame(b.clone())), Observation::Changed);
        assert_eq!(d.observe(&frame(b)), Observation::Unchanged);
        assert_eq!(d.observe(&frame(a)), Observation::Changed);
    }

    #[test]
    fn a_change_in_the_status_bar_is_unchanged() {
        // 0.1 of 50 rows: screen rows 0 to 4 are the status bar.
        let base = pixels(10);
        let mut clock = base.clone();
        poke(&mut clock, 4, 0);
        let mut content = base.clone();
        poke(&mut content, 5, 0);
        let mut d = ChangeDetector::new(FrameHasher::new(0.1));
        d.observe(&frame(base));
        assert_eq!(d.observe(&frame(clock)), Observation::Unchanged);
        assert_eq!(d.observe(&frame(content)), Observation::Changed);
    }

    #[test]
    fn the_status_bar_is_at_the_start_of_the_buffer() {
        // A reader that took raw rows as bottom-up would leave out the last
        // rows instead and miss this change at the bottom of the screen.
        let base = pixels(10);
        let mut bottom = base.clone();
        poke(&mut bottom, H - 1, 2);
        let mut d = ChangeDetector::new(FrameHasher::new(0.1));
        d.observe(&frame(base));
        assert_eq!(d.observe(&frame(bottom)), Observation::Changed);
    }

    #[test]
    fn a_dimension_change_is_a_change() {
        let mut d = ChangeDetector::new(FrameHasher::default());
        d.observe(&frame(pixels(10)));
        let rotated = Frame::new(H, W, Bytes::from(pixels(10))).unwrap();
        assert_eq!(d.observe(&rotated), Observation::Changed);
    }

    #[test]
    fn reset_starts_over() {
        let mut d = ChangeDetector::new(FrameHasher::default());
        d.observe(&frame(pixels(10)));
        d.reset();
        assert_eq!(d.observe(&frame(pixels(10))), Observation::First);
    }

    #[test]
    fn keys_carry_the_dimensions() {
        let key = FrameHasher::default().key(&frame(pixels(3)));
        assert_eq!((key.width, key.height), (W, H));
        assert_eq!(key, FrameHasher::default().key(&frame(pixels(3))));
        assert_ne!(key.hash, FrameHasher::default().key(&frame(pixels(4))).hash);
    }
}
