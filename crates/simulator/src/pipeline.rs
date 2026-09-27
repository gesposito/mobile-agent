//! The frame pipeline's pixel work, which needs no Simulator: views of BGRA
//! frames, sizing, the box downscale, the change hashes and the read
//! throttle. Ported from spikes S2 and S3.

use std::fmt;
use std::time::{Duration, Instant};

/// Longer side, in pixels, of the thumbnail the settle hash is taken from.
pub const HASH_DIM: usize = 32;

/// The share of a portrait frame's height the status bar takes, rounded up:
/// 59 of 874 points on an iPhone 17 (spike S3).
pub const STATUS_BAR_FRACTION: f64 = 0.075;

/// A borrowed BGRA8 frame whose rows are `stride` bytes apart.
#[derive(Clone, Copy, Debug)]
pub struct FrameView<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
}

/// Why a buffer cannot be viewed as a BGRA frame.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    Empty,
    StrideTooSmall { stride: usize, row_bytes: usize },
    BufferTooShort { len: usize, needed: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Empty => write!(f, "the frame has no pixels"),
            FrameError::StrideTooSmall { stride, row_bytes } => {
                write!(f, "stride {stride} is below the row size {row_bytes}")
            }
            FrameError::BufferTooShort { len, needed } => {
                write!(
                    f,
                    "the buffer holds {len} bytes and the frame needs {needed}"
                )
            }
        }
    }
}

impl<'a> FrameView<'a> {
    /// Views `data` as `width` x `height` BGRA pixels. The last row may omit
    /// its padding, so `data` needs `stride * (height - 1) + width * 4` bytes.
    pub fn new(
        data: &'a [u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<FrameView<'a>, FrameError> {
        let _ = (data, width, height, stride);
        unimplemented!("task 6.2: frame pipeline")
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn stride(&self) -> usize {
        self.stride
    }

    /// The frame from row `top` down.
    ///
    /// # Panics
    /// When `top` leaves no rows.
    pub fn rows_from(&self, top: usize) -> FrameView<'a> {
        let _ = top;
        unimplemented!("task 6.2: frame pipeline")
    }
}

/// The size that fits `width` x `height` inside `max_dim` on the longer side,
/// keeping the aspect ratio. Never upscales; `max_dim == 0` means native.
pub fn fit_within(width: usize, height: usize, max_dim: usize) -> (usize, usize) {
    let _ = (width, height, max_dim);
    unimplemented!("task 6.2: frame pipeline")
}

/// Box-filters `src` down to `dst_width` x `dst_height` into `dst` as packed
/// BGRA: each output pixel is the rounded mean of the source pixels it
/// covers. At the source size this copies the rows without their padding.
///
/// # Panics
/// When the destination is empty or larger than the source in either axis.
pub fn scale_bgra(src: &FrameView<'_>, dst_width: usize, dst_height: usize, dst: &mut Vec<u8>) {
    let _ = (src, dst_width, dst_height, dst);
    unimplemented!("task 6.2: frame pipeline")
}

/// 64-bit FNV-1a over the blue, green and red bytes of a box-filtered
/// thumbnail whose longer side is `HASH_DIM`. A change must move some
/// thumbnail cell's mean by half a level to alter it, so a blinking cursor or
/// a single glyph does not, which is what settle wants (D8).
pub fn thumbnail_hash(frame: &FrameView<'_>) -> u64 {
    let _ = frame;
    unimplemented!("task 6.2: frame pipeline")
}

/// A hash of every byte of packed pixels, so any change at the delivered
/// resolution alters it: the test for whether the display updated.
pub fn pixels_hash(pixels: &[u8]) -> u64 {
    let _ = pixels;
    unimplemented!("task 6.2: frame pipeline")
}

/// The rows the status bar takes at the top of a portrait frame.
pub fn status_bar_rows(height: usize) -> usize {
    let _ = height;
    unimplemented!("task 6.2: frame pipeline")
}

/// The thumbnail hash of the frame below the status bar, whose clock changes
/// on its own and must not keep a screen from settling (S3).
pub fn content_hash(frame: &FrameView<'_>) -> u64 {
    let _ = frame;
    unimplemented!("task 6.2: frame pipeline")
}

/// When a throttled reader reads. Late reads tear, because the render server
/// writes the surface without waiting for a reader's lock (S2), so a reader
/// skips presents instead of delaying them: after a read it sleeps through
/// three quarters of the interval and reads on the first present after that.
/// A present that arrived during the sleep is read only if no fresh one
/// follows, since then the display has stopped changing and nothing can tear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Throttle {
    interval: Option<Duration>,
}

impl Throttle {
    /// At most `max_fps` reads a second; 0 reads every present.
    pub fn new(max_fps: u32) -> Throttle {
        let _ = max_fps;
        unimplemented!("task 6.2: frame pipeline")
    }

    pub fn interval(&self) -> Option<Duration> {
        self.interval
    }

    /// Until when a reader that read at `last` sleeps; presents before then
    /// are stale. `None` when unthrottled.
    pub fn resume_at(&self, last: Instant) -> Option<Instant> {
        let _ = last;
        unimplemented!("task 6.2: frame pipeline")
    }

    /// How long a resumed reader waits for a fresh present before it reads a
    /// stale one.
    pub fn fresh_wait(&self) -> Duration {
        unimplemented!("task 6.2: frame pipeline")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `width` x `height` BGRA buffer with `pad` bytes of 0xAB after each
    /// row, filled by `pixel(x, y)`. Returns the buffer and its stride.
    fn frame(
        width: usize,
        height: usize,
        pad: usize,
        pixel: impl Fn(usize, usize) -> [u8; 4],
    ) -> (Vec<u8>, usize) {
        let stride = width * 4 + pad;
        let mut data = vec![0xAB; stride * height];
        for y in 0..height {
            for x in 0..width {
                let at = y * stride + x * 4;
                data[at..at + 4].copy_from_slice(&pixel(x, y));
            }
        }
        (data, stride)
    }

    fn view(data: &[u8], w: usize, h: usize, stride: usize) -> FrameView<'_> {
        FrameView::new(data, w, h, stride).expect("a valid frame")
    }

    fn scaled(data: &[u8], w: usize, h: usize, stride: usize, dw: usize, dh: usize) -> Vec<u8> {
        let mut out = Vec::new();
        scale_bgra(&view(data, w, h, stride), dw, dh, &mut out);
        out
    }

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn a_view_accepts_an_unpadded_last_row() {
        let data = [0u8; 8 + 8];
        let frame = view(&data, 2, 2, 8);
        assert_eq!((frame.width(), frame.height(), frame.stride()), (2, 2, 8));
        let short_last_row = [0u8; 12 + 8];
        assert!(FrameView::new(&short_last_row, 2, 2, 12).is_ok());
    }

    #[test]
    fn a_view_refuses_bad_geometry() {
        assert_eq!(
            FrameView::new(&[0; 15], 2, 2, 8).unwrap_err(),
            FrameError::BufferTooShort {
                len: 15,
                needed: 16
            }
        );
        assert_eq!(
            FrameView::new(&[0; 64], 4, 2, 8).unwrap_err(),
            FrameError::StrideTooSmall {
                stride: 8,
                row_bytes: 16
            }
        );
        assert_eq!(FrameView::new(&[], 0, 2, 8).unwrap_err(), FrameError::Empty);
    }

    #[test]
    fn fit_within_caps_the_longer_side_and_never_upscales() {
        assert_eq!(fit_within(1206, 2622, 1000), (460, 1000));
        assert_eq!(fit_within(2622, 1206, 1000), (1000, 460));
        assert_eq!(fit_within(460, 1000, HASH_DIM), (15, 32));
        assert_eq!(fit_within(300, 600, 1000), (300, 600));
        assert_eq!(fit_within(1206, 2622, 0), (1206, 2622));
        assert_eq!(fit_within(1, 5000, 100), (1, 100));
    }

    #[test]
    fn scaling_averages_the_covered_source_block() {
        let quadrants = |x: usize, y: usize| match (x < 2, y < 2) {
            (true, true) => [10, 20, 30, 255],
            (false, true) => [40, 50, 60, 255],
            (true, false) => [70, 80, 90, 255],
            (false, false) => [100, 110, 120, 255],
        };
        let (data, stride) = frame(4, 4, 0, quadrants);
        assert_eq!(
            scaled(&data, 4, 4, stride, 2, 2),
            vec![
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255
            ]
        );
    }

    #[test]
    fn scaling_rounds_the_mean_half_up() {
        let checker = |x: usize, y: usize| {
            if (x + y).is_multiple_of(2) {
                BLACK
            } else {
                WHITE
            }
        };
        let (data, stride) = frame(2, 2, 0, checker);
        assert_eq!(scaled(&data, 2, 2, stride, 1, 1), vec![128, 128, 128, 255]);
    }

    #[test]
    fn scaling_ignores_stride_padding_and_non_integer_ratios() {
        let (data, stride) = frame(6, 4, 12, |_, _| [1, 2, 3, 255]);
        let out = scaled(&data, 6, 4, stride, 3, 2);
        assert_eq!(out.len(), 3 * 2 * 4);
        assert!(out.chunks_exact(4).all(|p| p == [1, 2, 3, 255]));

        let (data, stride) = frame(1206, 2622, 40, |_, _| [9, 8, 7, 255]);
        let out = scaled(&data, 1206, 2622, stride, 460, 1000);
        assert_eq!(out.len(), 460 * 1000 * 4);
        assert!(out.chunks_exact(4).all(|p| p == [9, 8, 7, 255]));
    }

    #[test]
    fn scaling_to_the_source_size_packs_the_rows() {
        let (data, stride) = frame(3, 2, 8, |x, y| [x as u8, y as u8, 7, 255]);
        let expected: Vec<u8> = (0..2)
            .flat_map(|y| (0..3).flat_map(move |x| [x as u8, y as u8, 7, 255]))
            .collect();
        assert_eq!(scaled(&data, 3, 2, stride, 3, 2), expected);
    }

    #[test]
    fn rows_from_drops_the_rows_above_and_keeps_the_stride() {
        let (data, stride) = frame(3, 4, 4, |_, y| [y as u8, 0, 0, 255]);
        let lower = view(&data, 3, 4, stride).rows_from(1);
        assert_eq!(
            (lower.width(), lower.height(), lower.stride()),
            (3, 3, stride)
        );
        let mut copy = Vec::new();
        scale_bgra(&lower, 3, 3, &mut copy);
        let first_column: Vec<u8> = copy.chunks_exact(12).map(|row| row[0]).collect();
        assert_eq!(first_column, vec![1, 2, 3]);
    }

    #[test]
    fn the_thumbnail_hash_ignores_padding_alpha_and_a_single_pixel() {
        let gradient = |x: usize, y: usize| [(x % 256) as u8, (y % 256) as u8, 40, 255];
        let (a, a_stride) = frame(460, 1000, 0, gradient);
        let (b, b_stride) = frame(460, 1000, 64, gradient);
        assert_eq!(
            thumbnail_hash(&view(&a, 460, 1000, a_stride)),
            thumbnail_hash(&view(&b, 460, 1000, b_stride))
        );

        let (opaque, stride) = frame(64, 64, 0, |x, _| [x as u8, 1, 2, 255]);
        let (clear, _) = frame(64, 64, 0, |x, _| [x as u8, 1, 2, 0]);
        assert_eq!(
            thumbnail_hash(&view(&opaque, 64, 64, stride)),
            thumbnail_hash(&view(&clear, 64, 64, stride))
        );

        let (base, stride) = frame(460, 1000, 0, |_, _| BLACK);
        let (dot, _) = frame(460, 1000, 0, |x, y| {
            if (x, y) == (230, 500) { WHITE } else { BLACK }
        });
        assert_eq!(
            thumbnail_hash(&view(&base, 460, 1000, stride)),
            thumbnail_hash(&view(&dot, 460, 1000, stride))
        );
    }

    #[test]
    fn the_thumbnail_hash_changes_when_a_visible_region_changes() {
        let (base, stride) = frame(460, 1000, 0, |_, _| BLACK);
        let (square, _) = frame(460, 1000, 0, |x, y| {
            if (200..260).contains(&x) && (500..560).contains(&y) {
                WHITE
            } else {
                BLACK
            }
        });
        assert_ne!(
            thumbnail_hash(&view(&base, 460, 1000, stride)),
            thumbnail_hash(&view(&square, 460, 1000, stride))
        );
    }

    #[test]
    fn the_pixels_hash_sees_one_changed_byte_anywhere() {
        let (base, _) = frame(460, 1000, 0, |_, _| BLACK);
        let reference = pixels_hash(&base);
        assert_eq!(pixels_hash(&base.clone()), reference, "stable");
        for at in [0, 3, 460 * 4 * 500 + 17, base.len() - 1] {
            let mut changed = base.clone();
            changed[at] ^= 1;
            assert_ne!(pixels_hash(&changed), reference, "byte {at}");
        }
        assert_ne!(
            pixels_hash(&base[..base.len() - 4]),
            reference,
            "length counts"
        );
    }

    #[test]
    fn the_content_hash_leaves_the_status_bar_out() {
        assert_eq!(status_bar_rows(1000), 75);
        assert_eq!(status_bar_rows(2622), 197);
        let screen = |clock: u8| {
            frame(460, 1000, 0, move |x, y| {
                if y < 40 && x < 50 {
                    [clock, clock, clock, 255]
                } else {
                    [(x % 200) as u8, (y % 200) as u8, 90, 255]
                }
            })
        };
        let (nine, stride) = screen(0);
        let (ten, _) = screen(255);
        let (nine, ten) = (
            view(&nine, 460, 1000, stride),
            view(&ten, 460, 1000, stride),
        );
        assert_ne!(
            thumbnail_hash(&nine),
            thumbnail_hash(&ten),
            "the clock is seen"
        );
        assert_eq!(content_hash(&nine), content_hash(&ten), "and left out");
    }

    #[test]
    fn the_throttle_skips_presents_for_three_quarters_of_its_interval() {
        let unthrottled = Throttle::new(0);
        assert_eq!(unthrottled.interval(), None);
        assert_eq!(unthrottled.resume_at(Instant::now()), None);

        let throttle = Throttle::new(30);
        let interval = throttle.interval().expect("an interval");
        assert!(
            interval.abs_diff(Duration::from_secs_f64(1.0 / 30.0)) < Duration::from_micros(1),
            "{interval:?}"
        );
        let last = Instant::now();
        let resume = throttle.resume_at(last).expect("a resume time");
        assert!(
            (resume - last).abs_diff(interval.mul_f64(0.75)) < Duration::from_micros(1),
            "{:?}",
            resume - last
        );
        assert!(throttle.fresh_wait() > interval / 4);
        assert!(throttle.fresh_wait() < interval / 2);
    }
}
