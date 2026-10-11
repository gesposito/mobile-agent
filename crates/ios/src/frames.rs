//! Frames from the device's screenshots (design D23; spec
//! `ios-device-driver`, "Frames and screenshots from the host"). iOS 26.6.2
//! refuses the display stream and AVFoundation waits on the camera
//! permission (S11), so a subscription polls PNG screenshots on two
//! connections, DVT and CoreDevice, and turns them into frames here:
//!
//! - a PNG with the same bytes as the one before is skipped before it is
//!   decoded: on a still screen 246 of 247 were;
//! - a decoded picture is downscaled with the Simulator pipeline's
//!   `scale_bgra` to the long edge asked for, and is a frame when a pixel
//!   below the status bar moved more than two levels from the last frame,
//!   the tolerance that counted none of S11's 1,283 still frames and caught
//!   every tap and drag on its first changed frame;
//! - the status bar's band is the device's: 20 points on a home-button
//!   iPhone, as on the SE in S11, where `status_bar_rows`' 7.5% (an iPhone
//!   17's) covers most of a back button.

use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use simulator::{FrameView, fit_within, pixels_hash, scale_bgra};

use crate::error::Result;
use crate::model::{Capture, Frame, FrameOptions, FrameStats, Screen};

/// The levels a pixel's channel may move between two captures of a still
/// screen: S11 saw at most one.
pub const TOLERANCE: u8 = 2;
/// The status bar of a home-button iPhone, in points (S11: 20 on the SE).
pub const HOME_BUTTON_STATUS_BAR: f64 = 20.0;
/// The share of other screens' height left out, the Simulator's: an iPhone
/// 17's 59 of 874 points, which covers the taller bars of Face ID phones.
pub const STATUS_BAR_FRACTION: f64 = 0.075;
/// How many delivered frames are held for encoding.
pub const KEPT_FRAMES: usize = 4;
/// How many undelivered frames a subscription keeps before dropping the
/// oldest.
pub const QUEUE_CAPACITY: usize = 16;

/// RGB or RGBA, 8 or 16 bits a channel, to packed BGRA; the size.
pub fn decode_png(bytes: &[u8], out: &mut Vec<u8>) -> std::result::Result<(usize, usize), String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or("the PNG's size overflows")?;
    let mut buffer = vec![0; size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    let (width, height) = (info.width as usize, info.height as usize);
    out.clear();
    out.reserve(width * height * 4);
    match info.color_type {
        png::ColorType::Rgb => {
            for row in buffer.chunks_exact(info.line_size).take(height) {
                for pixel in row[..width * 3].as_chunks::<3>().0 {
                    out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                }
            }
        }
        png::ColorType::Rgba => {
            for row in buffer.chunks_exact(info.line_size).take(height) {
                for pixel in row[..width * 4].as_chunks::<4>().0 {
                    out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
            }
        }
        other => return Err(format!("a PNG of colour type {other:?}")),
    }
    Ok((width, height))
}

/// The status bar's height in points on `screen`: 20 on a home-button
/// iPhone, whose screen is under twice as tall as wide; 7.5% of the height
/// on others, whose bars of 44 to 62 points it covers.
pub fn status_bar_points(screen: &Screen) -> f64 {
    let (width, height) = screen.points();
    if height < width * 2.0 {
        HOME_BUTTON_STATUS_BAR
    } else {
        height * STATUS_BAR_FRACTION
    }
}

/// The rows of a frame `height` pixels tall, scaled from `screen`, that
/// `points` of status bar take, rounded up.
pub fn status_bar_rows(points: f64, screen: &Screen, height: usize) -> usize {
    let pixels = points * screen.scale;
    (pixels * height as f64 / f64::from(screen.height)).ceil() as usize
}

/// Whether some pixel of packed BGRA `a` below row `top` moved more than
/// `tolerance` levels in a channel from `b`, both `width` pixels wide.
pub fn changed(a: &[u8], b: &[u8], width: usize, top: usize, tolerance: u8) -> bool {
    let start = (top * width * 4).min(a.len()).min(b.len());
    let (a, _) = a[start..].as_chunks::<4>();
    let (b, _) = b[start..].as_chunks::<4>();
    a.iter().zip(b).any(|(x, y)| {
        x[0].abs_diff(y[0]) > tolerance
            || x[1].abs_diff(y[1]) > tolerance
            || x[2].abs_diff(y[2]) > tolerance
    })
}

/// A delivered frame's pixels, held for encoding.
#[derive(Clone, Debug)]
pub struct Held {
    pub frame: Frame,
    pub pixels: Vec<u8>,
}

/// How many PNGs, by digest, are remembered as showing the last frame.
const KNOWN_PNGS: usize = 8;

/// Captures in, frames out: the pipeline's state between two captures.
pub struct Pipeline {
    max_dimension: usize,
    status_bar: f64,
    screen: Screen,
    /// The digests of PNGs known to show the last frame within the
    /// tolerance: the one delivered and the ones decoded since and found
    /// unchanged. A PNG among them is skipped before decoding, whichever
    /// connection it came on.
    known: Vec<u64>,
    sequence: u64,
    held: VecDeque<Held>,
    stats: FrameStats,
    started: Instant,
    decoded: Vec<u8>,
}

impl Pipeline {
    pub fn new(options: &FrameOptions, screen: Screen) -> Pipeline {
        Pipeline {
            max_dimension: options.max_dimension as usize,
            status_bar: options
                .status_bar
                .unwrap_or_else(|| status_bar_points(&screen)),
            screen,
            known: Vec::new(),
            sequence: 0,
            held: VecDeque::new(),
            stats: FrameStats::default(),
            started: Instant::now(),
            decoded: Vec::new(),
        }
    }

    /// Takes one capture: the PNG `png` from `source`, requested at
    /// `requested` and arrived at `arrived`; returns the frame it is, if
    /// any.
    pub fn take(
        &mut self,
        png: &[u8],
        source: Capture,
        requested: Instant,
        arrived: Instant,
    ) -> Option<Frame> {
        self.stats.captures[source.index()] += 1;
        let digest = pixels_hash(png);
        if self.known.contains(&digest) {
            self.stats.identical += 1;
            return None;
        }
        let (width, height) = match decode_png(png, &mut self.decoded) {
            Ok(size) => size,
            Err(reason) => {
                self.failed(source, &format!("a capture did not decode: {reason}"));
                return None;
            }
        };
        self.stats.decoded += 1;
        let view = FrameView::new(&self.decoded, width, height, width * 4).ok()?;
        let (to_width, to_height) = fit_within(width, height, self.max_dimension);
        let mut pixels = Vec::with_capacity(to_width * to_height * 4);
        scale_bgra(&view, to_width, to_height, &mut pixels);
        let top = status_bar_rows(self.status_bar, &self.screen, to_height).min(to_height);
        self.stats.status_bar_rows = top as u32;
        if let Some(last) = self.held.back()
            && (last.frame.width, last.frame.height) == (to_width as u32, to_height as u32)
            && !changed(&pixels, &last.pixels, to_width, top, TOLERANCE)
        {
            self.stats.unchanged += 1;
            if self.known.len() < KNOWN_PNGS {
                self.known.push(digest);
            }
            return None;
        }
        self.sequence += 1;
        let frame = Frame {
            sequence: self.sequence,
            requested,
            arrived,
            width: to_width as u32,
            height: to_height as u32,
            hash: pixels_hash(&pixels[top * to_width * 4..]),
            source,
        };
        self.held.push_back(Held { frame, pixels });
        while self.held.len() > KEPT_FRAMES {
            self.held.pop_front();
        }
        self.known = vec![digest];
        self.stats.delivered += 1;
        Some(frame)
    }

    /// A capture that failed on `source`.
    pub fn failed(&mut self, source: Capture, reason: &str) {
        let _ = source;
        self.stats.failures += 1;
        self.stats.failure.get_or_insert_with(|| reason.to_owned());
    }

    /// A frame the consumer fell too far behind to read.
    pub fn dropped(&mut self) {
        self.stats.dropped += 1;
    }

    pub fn stats(&self) -> FrameStats {
        FrameStats {
            elapsed: self.started.elapsed(),
            ..self.stats.clone()
        }
    }

    /// The delivered frame `sequence`, when it is still held.
    pub fn held(&self, sequence: u64) -> Option<&Held> {
        self.held
            .iter()
            .find(|held| held.frame.sequence == sequence)
    }

    /// The last frame delivered.
    pub fn latest(&self) -> Option<Frame> {
        self.held.back().map(|held| held.frame)
    }
}

/// What feeds a [`FrameStream`]: the pollers and the pipeline on macOS.
pub(crate) trait FrameFeed: Send + Sync {
    /// The next frame, waiting up to `timeout`; `None` once it passed or
    /// the subscription ended.
    fn next(&self, timeout: Duration) -> Option<Frame>;
    /// The last frame delivered.
    fn latest(&self) -> Option<Frame>;
    /// Frame `sequence`, held, as a JPEG of exactly `width` by `height`.
    fn encode(&self, sequence: u64, width: u32, height: u32, quality: u8) -> Result<Vec<u8>>;
    fn stats(&self) -> FrameStats;
    /// Closes the connection `connection`'s poller captures on.
    fn close_connection(&self, connection: Capture);
}

/// A frame subscription: the frame on screen when it started, then one
/// each time a capture shows a change. Dropping it ends the polling.
pub struct FrameStream {
    feed: Box<dyn FrameFeed>,
}

impl FrameStream {
    pub(crate) fn new(feed: Box<dyn FrameFeed>) -> FrameStream {
        FrameStream { feed }
    }

    /// The next frame, waiting up to `timeout`; `None` when none came.
    pub fn next(&self, timeout: Duration) -> Option<Frame> {
        self.feed.next(timeout)
    }

    /// The next frame if one is waiting.
    pub fn try_next(&self) -> Option<Frame> {
        self.feed.next(Duration::ZERO)
    }

    /// The last frame delivered, read or not: what the mirror's
    /// `FrameSource::latest` answers.
    pub fn latest(&self) -> Option<Frame> {
        self.feed.latest()
    }

    /// Frame `sequence`, among the last few delivered, as a JPEG scaled to
    /// exactly `width` by `height` (no larger than the frame): what the
    /// mirror's `FrameSource::encode` asks.
    pub fn encode(&self, sequence: u64, width: u32, height: u32, quality: u8) -> Result<Vec<u8>> {
        self.feed.encode(sequence, width, height, quality)
    }

    pub fn stats(&self) -> FrameStats {
        self.feed.stats()
    }

    /// Closes the connection `connection`'s poller captures on, as a
    /// dropped connection closes: the device tests' way to see the poller
    /// open it again (task 9.8 of physical-ios).
    pub fn close_connection(&self, connection: Capture) {
        self.feed.close_connection(connection);
    }
}

/// Frames between the pipeline and the consumer. When the consumer falls
/// [`QUEUE_CAPACITY`] behind, the oldest frame is dropped.
pub(crate) struct FrameQueue {
    state: Mutex<(VecDeque<Frame>, bool)>,
    ready: Condvar,
}

impl FrameQueue {
    pub(crate) fn new() -> FrameQueue {
        FrameQueue {
            state: Mutex::new((VecDeque::new(), false)),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, (VecDeque<Frame>, bool)> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues `frame`; whether the oldest was dropped for it.
    pub(crate) fn push(&self, frame: Frame) -> bool {
        let mut state = self.lock();
        let dropped = state.0.len() >= QUEUE_CAPACITY;
        if dropped {
            state.0.pop_front();
        }
        state.0.push_back(frame);
        drop(state);
        self.ready.notify_one();
        dropped
    }

    /// The oldest frame, waiting up to `timeout`; after `close`, what is
    /// left, then nothing.
    pub(crate) fn pop(&self, timeout: Duration) -> Option<Frame> {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        loop {
            if let Some(frame) = state.0.pop_front() {
                return Some(frame);
            }
            if state.1 {
                return None;
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            state = self
                .ready
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    pub(crate) fn close(&self) {
        self.lock().1 = true;
        self.ready.notify_all();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The iPhone SE (3rd generation): 750x1334 pixels at 2x.
    pub(crate) const SE: Screen = Screen {
        width: 750,
        height: 1334,
        scale: 2.0,
    };

    /// An 8-bit RGB PNG of `width` by `height`, each pixel `fill(x, y)`.
    pub(crate) fn png_of(width: u32, height: u32, fill: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut data = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&fill(x, y));
            }
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("a header");
        writer.write_image_data(&data).expect("the pixels");
        drop(writer);
        out
    }

    /// A still screen, grey with a gradient, 750 by 1334 when `full`, one
    /// tenth otherwise.
    pub(crate) fn screen(level: impl Fn(u32, u32) -> u8, full: bool) -> Vec<u8> {
        let (width, height) = if full { (750, 1334) } else { (75, 133) };
        png_of(width, height, |x, y| {
            let base = 100u8.wrapping_add((x % 50) as u8);
            [base, level(x, y), 80]
        })
    }

    fn options(max_dimension: u32) -> FrameOptions {
        FrameOptions {
            max_fps: 0,
            max_dimension,
            status_bar: None,
        }
    }

    fn take(pipeline: &mut Pipeline, png: &[u8]) -> Option<Frame> {
        let now = Instant::now();
        pipeline.take(png, Capture::Dvt, now, now)
    }

    #[test]
    fn eight_and_sixteen_bit_pngs_become_bgra() {
        let png = png_of(
            2,
            1,
            |x, _| if x == 0 { [10, 20, 30] } else { [40, 50, 60] },
        );
        let mut out = Vec::new();
        assert_eq!(decode_png(&png, &mut out), Ok((2, 1)));
        assert_eq!(out, vec![30, 20, 10, 255, 60, 50, 40, 255]);
        let mut sixteen = Vec::new();
        let mut encoder = png::Encoder::new(&mut sixteen, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Sixteen);
        let mut writer = encoder.write_header().expect("a header");
        writer
            .write_image_data(&[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc])
            .expect("the pixel");
        drop(writer);
        assert_eq!(decode_png(&sixteen, &mut out), Ok((1, 1)));
        assert_eq!(out, vec![0x9a, 0x56, 0x12, 255], "the high bytes");
        assert!(decode_png(b"not a png", &mut out).is_err());
    }

    #[test]
    fn the_status_bar_comes_from_the_devices_screen() {
        assert_eq!(status_bar_points(&SE), HOME_BUTTON_STATUS_BAR);
        // An iPhone 15's 393x852 points.
        let face_id = Screen {
            width: 1179,
            height: 2556,
            scale: 3.0,
        };
        assert!((status_bar_points(&face_id) - 852.0 * 0.075).abs() < 1e-9);
        // 40 of the SE's 1334 rows, in a frame 1000 tall.
        assert_eq!(status_bar_rows(20.0, &SE, 1000), 30);
        assert_eq!(status_bar_rows(20.0, &SE, 1334), 40);
    }

    #[test]
    fn a_pixel_moved_more_than_two_levels_below_the_status_bar_is_a_change() {
        let width = 4;
        let base = vec![100u8; width * 10 * 4];
        let moved = |row: usize, by: u8| {
            let mut frame = base.clone();
            frame[(row * width + 1) * 4 + 1] += by;
            frame
        };
        assert!(!changed(&base, &base, width, 3, TOLERANCE));
        assert!(
            !changed(&moved(5, 2), &base, width, 3, TOLERANCE),
            "two levels"
        );
        assert!(changed(&moved(5, 3), &base, width, 3, TOLERANCE), "three");
        assert!(
            !changed(&moved(2, 90), &base, width, 3, TOLERANCE),
            "inside the status bar"
        );
    }

    #[test]
    fn identical_pngs_are_skipped_before_decoding() {
        let mut pipeline = Pipeline::new(&options(0), SE);
        let still = screen(|_, _| 120, false);
        let first = take(&mut pipeline, &still).expect("the first frame");
        assert_eq!(first.sequence, 1);
        assert_eq!(take(&mut pipeline, &still), None);
        assert_eq!(take(&mut pipeline, &still), None);
        let stats = pipeline.stats();
        assert_eq!(stats.decoded, 1, "the repeats were not decoded");
        assert_eq!(stats.identical, 2);
        assert_eq!(stats.delivered, 1);
    }

    #[test]
    fn a_frame_is_a_pixel_below_the_status_bar_more_than_two_levels_from_the_last_frame() {
        let mut pipeline = Pipeline::new(&options(0), SE);
        let (row, column) = (100, 30);
        let at = |by: u8| {
            screen(
                move |x, y| {
                    if (x, y) == (column, row) {
                        120 + by
                    } else {
                        120
                    }
                },
                false,
            )
        };
        let first = take(&mut pipeline, &at(0)).expect("the first frame");
        assert_eq!(
            take(&mut pipeline, &at(2)),
            None,
            "two levels is the screen's own noise"
        );
        let second = take(&mut pipeline, &at(3)).expect("three levels is a change");
        assert_eq!(second.sequence, 2);
        assert_ne!(second.hash, first.hash);
        let stats = pipeline.stats();
        assert_eq!((stats.decoded, stats.unchanged, stats.delivered), (3, 1, 2));
    }

    #[test]
    fn a_change_inside_the_devices_status_bar_is_no_frame() {
        let mut pipeline = Pipeline::new(&options(0), SE);
        take(&mut pipeline, &screen(|_, _| 120, false)).expect("the first frame");
        // Row 3 of the tenth-size screen: inside the SE's 20 points.
        let clock = screen(|x, y| if y == 3 && x < 20 { 250 } else { 120 }, false);
        assert_eq!(take(&mut pipeline, &clock), None);
        assert_eq!(
            pipeline.stats().status_bar_rows,
            4,
            "40 of 1334 rows in 133"
        );
    }

    #[test]
    fn no_still_frame_is_counted_through_one_level_noise_and_drift_is_measured_from_the_last_frame()
    {
        let mut pipeline = Pipeline::new(&options(0), SE);
        let level = |by: u8| {
            screen(
                move |x, y| if (x, y) == (10, 60) { 120 + by } else { 120 },
                false,
            )
        };
        take(&mut pipeline, &level(0)).expect("the first frame");
        for index in 0..40 {
            assert_eq!(
                take(&mut pipeline, &level(index % 2)),
                None,
                "capture {index}"
            );
        }
        assert_eq!(take(&mut pipeline, &level(2)), None);
        assert!(
            take(&mut pipeline, &level(3)).is_some(),
            "three levels from the last frame, though one from the capture before"
        );
        assert_eq!(pipeline.stats().delivered, 2);
    }

    #[test]
    fn frames_are_scaled_to_the_long_edge_asked_for_and_held_for_encoding() {
        let mut pipeline = Pipeline::new(&options(1000), SE);
        let frame = take(&mut pipeline, &screen(|_, _| 120, true)).expect("a frame");
        assert_eq!((frame.width, frame.height), (562, 1000));
        let held = pipeline.held(frame.sequence).expect("held");
        assert_eq!(held.pixels.len(), 562 * 1000 * 4);
        let mut native = Pipeline::new(&options(0), SE);
        let frame = take(&mut native, &screen(|_, _| 120, true)).expect("a frame");
        assert_eq!((frame.width, frame.height), (750, 1334));
        for level in 1..=6u8 {
            take(&mut native, &screen(|_, _| 120 + level * 10, true)).expect("a change");
        }
        assert!(native.held(1).is_none(), "only the last frames are held");
        assert!(native.held(7).is_some());
        assert_eq!(native.latest().map(|frame| frame.sequence), Some(7));
    }

    #[test]
    fn a_png_that_does_not_decode_is_a_failure_not_a_frame() {
        let mut pipeline = Pipeline::new(&options(0), SE);
        assert_eq!(take(&mut pipeline, b"\x89PNG broken"), None);
        let stats = pipeline.stats();
        assert_eq!(stats.failures, 1);
        assert!(stats.failure.is_some());
    }
}
