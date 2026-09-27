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
        if width == 0 || height == 0 {
            return Err(FrameError::Empty);
        }
        let row_bytes = width * 4;
        if stride < row_bytes {
            return Err(FrameError::StrideTooSmall { stride, row_bytes });
        }
        let needed = stride * (height - 1) + row_bytes;
        if data.len() < needed {
            return Err(FrameError::BufferTooShort {
                len: data.len(),
                needed,
            });
        }
        Ok(FrameView {
            data,
            width,
            height,
            stride,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    #[cfg(test)]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// The frame from row `top` down.
    ///
    /// # Panics
    /// When `top` leaves no rows.
    pub fn rows_from(&self, top: usize) -> FrameView<'a> {
        assert!(
            top < self.height,
            "rows_from({top}) of a {}-row frame",
            self.height
        );
        FrameView {
            data: &self.data[top * self.stride..],
            width: self.width,
            height: self.height - top,
            stride: self.stride,
        }
    }

    /// Row `y` without its padding.
    fn row(&self, y: usize) -> &'a [u8] {
        let start = y * self.stride;
        &self.data[start..start + self.width * 4]
    }
}

/// The size that fits `width` x `height` inside `max_dim` on the longer side,
/// keeping the aspect ratio. Never upscales; `max_dim == 0` means native.
pub fn fit_within(width: usize, height: usize, max_dim: usize) -> (usize, usize) {
    let longer = width.max(height);
    if max_dim == 0 || longer <= max_dim {
        return (width, height);
    }
    let side = |length: usize| ((length * max_dim + longer / 2) / longer).max(1);
    (side(width), side(height))
}

/// Box-filters `src` down to `dst_width` x `dst_height` into `dst` as packed
/// BGRA: each output pixel is the rounded mean of the source pixels it
/// covers. At the source size this copies the rows without their padding.
///
/// # Panics
/// When the destination is empty or larger than the source in either axis.
pub fn scale_bgra(src: &FrameView<'_>, dst_width: usize, dst_height: usize, dst: &mut Vec<u8>) {
    assert!(
        (1..=src.width).contains(&dst_width) && (1..=src.height).contains(&dst_height),
        "scale_bgra only downscales: {}x{} to {dst_width}x{dst_height}",
        src.width,
        src.height
    );
    dst.clear();
    dst.reserve(dst_width * dst_height * 4);
    if dst_width == src.width && dst_height == src.height {
        for y in 0..src.height {
            dst.extend_from_slice(src.row(y));
        }
        return;
    }
    // Output pixel (ox, oy) covers source columns x_bounds[ox]..x_bounds[ox + 1]
    // and rows y_bounds[oy]..y_bounds[oy + 1], so every source pixel lands in
    // exactly one output pixel. Each output row first sums its source rows
    // per column, a widening add the compiler vectorizes; each output pixel
    // then sums its columns and divides by its pixel count.
    let x_bounds: Vec<usize> = (0..=dst_width).map(|x| x * src.width / dst_width).collect();
    let y_bounds: Vec<usize> = (0..=dst_height)
        .map(|y| y * src.height / dst_height)
        .collect();
    let spans: Vec<usize> = y_bounds.windows(2).map(|rows| rows[1] - rows[0]).collect();
    // A span takes one of two lengths, the floor and the ceiling of the
    // ratio, so each length's divisors are prepared once.
    let mut divisors: Vec<(usize, Vec<Divisor>)> = Vec::new();
    for &rows in &spans {
        if !divisors.iter().any(|(length, _)| *length == rows) {
            let row = x_bounds
                .windows(2)
                .map(|columns| Divisor::new((columns[1] - columns[0]) * rows))
                .collect();
            divisors.push((rows, row));
        }
    }
    // Column sums fit in 16 bits while a span has at most 257 rows, which
    // halves the vertical pass; the sums per pixel are 32 bits either way.
    if spans
        .iter()
        .all(|&rows| rows <= usize::from(u16::MAX) / 255)
    {
        box_filter::<u16>(src, &x_bounds, &y_bounds, &divisors, dst);
    } else {
        box_filter::<u32>(src, &x_bounds, &y_bounds, &divisors, dst);
    }
}

/// A column sum of the vertical pass.
trait ColumnSum: Copy + Default {
    fn add(self, byte: u8) -> Self;
    fn widen(self) -> u32;
}

impl ColumnSum for u16 {
    fn add(self, byte: u8) -> u16 {
        self.wrapping_add(u16::from(byte))
    }

    fn widen(self) -> u32 {
        u32::from(self)
    }
}

impl ColumnSum for u32 {
    fn add(self, byte: u8) -> u32 {
        self.wrapping_add(u32::from(byte))
    }

    fn widen(self) -> u32 {
        self
    }
}

fn box_filter<S: ColumnSum>(
    src: &FrameView<'_>,
    x_bounds: &[usize],
    y_bounds: &[usize],
    divisors: &[(usize, Vec<Divisor>)],
    dst: &mut Vec<u8>,
) {
    let widest = x_bounds
        .windows(2)
        .map(|columns| columns[1] - columns[0])
        .max()
        .unwrap_or(1);
    match widest {
        1 => rows::<S, 1>(src, x_bounds, y_bounds, divisors, dst),
        2 => rows::<S, 2>(src, x_bounds, y_bounds, divisors, dst),
        3 => rows::<S, 3>(src, x_bounds, y_bounds, divisors, dst),
        4 => rows::<S, 4>(src, x_bounds, y_bounds, divisors, dst),
        _ => rows::<S, 0>(src, x_bounds, y_bounds, divisors, dst),
    }
}

/// The two passes for spans of at most `W` columns: every output pixel sums
/// exactly `W` columns, those past its own span weighted by zero, so the loop
/// has no branch that the alternating span lengths would mispredict. `W = 0`
/// sums each span's own length instead, for any width.
fn rows<S: ColumnSum, const W: usize>(
    src: &FrameView<'_>,
    x_bounds: &[usize],
    y_bounds: &[usize],
    divisors: &[(usize, Vec<Divisor>)],
    dst: &mut Vec<u8>,
) {
    // Zeros past the last column, so the last pixel's weighted reads stay in
    // bounds.
    let mut column_sums = vec![S::default(); (src.width + W) * 4];
    for rows in y_bounds.windows(2) {
        let (top, bottom) = (rows[0], rows[1]);
        for (sum, &byte) in column_sums.iter_mut().zip(src.row(top)) {
            *sum = S::default().add(byte);
        }
        for y in top + 1..bottom {
            for (sum, &byte) in column_sums.iter_mut().zip(src.row(y)) {
                *sum = sum.add(byte);
            }
        }
        let row_divisors = &divisors
            .iter()
            .find(|(length, _)| *length == bottom - top)
            .expect("a divisor row for every span length")
            .1;
        for (columns, divisor) in x_bounds.windows(2).zip(row_divisors) {
            let (left, span) = (columns[0], columns[1] - columns[0]);
            let mut total = [0u32; 4];
            if W == 0 {
                for pixel in column_sums[left * 4..(left + span) * 4].chunks_exact(4) {
                    for (total, &channel) in total.iter_mut().zip(pixel) {
                        *total = total.wrapping_add(channel.widen());
                    }
                }
            } else {
                for (index, pixel) in column_sums[left * 4..(left + W) * 4]
                    .chunks_exact(4)
                    .enumerate()
                {
                    let weight = u32::from(index < span);
                    for (total, &channel) in total.iter_mut().zip(pixel) {
                        *total = total.wrapping_add(channel.widen().wrapping_mul(weight));
                    }
                }
            }
            dst.extend_from_slice(&divisor.rounded(total));
        }
    }
}

/// Division by a pixel count, rounded to nearest with halves up, done as a
/// multiplication. With `m = floor(2^k / count) + 1`, `(n * m) >> k` equals
/// `n / count` for every `n` below `256 * count`, as a mean of bytes is,
/// once `2^k >= 256 * count^2`: the error `n / 2^k` stays below the `1 /
/// count` that separates two quotients. Counts up to 181 take `k = 23` and
/// 32-bit products, which vectorize; counts below 2^20 take `k = 48` and
/// 64-bit products; larger counts divide.
#[derive(Clone, Copy, Debug)]
enum Divisor {
    Narrow { half: u32, multiplier: u32 },
    Wide { half: u32, multiplier: u64 },
    Exact { count: u32 },
}

impl Divisor {
    fn new(count: usize) -> Divisor {
        let count = count as u32;
        let half = count / 2;
        if count <= 181 {
            Divisor::Narrow {
                half,
                multiplier: (1 << 23) / count + 1,
            }
        } else if count < 1 << 20 {
            Divisor::Wide {
                half,
                multiplier: (1 << 48) / u64::from(count) + 1,
            }
        } else {
            Divisor::Exact { count }
        }
    }

    fn rounded(self, totals: [u32; 4]) -> [u8; 4] {
        match self {
            // The products fit, as the type's comment shows, so the wrapping
            // operations only spare debug builds their overflow checks.
            Divisor::Narrow { half, multiplier } => {
                totals.map(|total| (total.wrapping_add(half).wrapping_mul(multiplier) >> 23) as u8)
            }
            Divisor::Wide { half, multiplier } => totals.map(|total| {
                (u64::from(total.wrapping_add(half)).wrapping_mul(multiplier) >> 48) as u8
            }),
            Divisor::Exact { count } => totals.map(|total| ((total + count / 2) / count) as u8),
        }
    }
}

/// 64-bit FNV-1a over the blue, green and red bytes of a box-filtered
/// thumbnail whose longer side is `HASH_DIM`. A change must move some
/// thumbnail cell's mean by half a level to alter it, so a blinking cursor or
/// a single glyph does not, which is what settle wants (D8).
pub fn thumbnail_hash(frame: &FrameView<'_>) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let (width, height) = fit_within(frame.width, frame.height, HASH_DIM);
    let mut thumbnail = Vec::with_capacity(width * height * 4);
    scale_bgra(frame, width, height, &mut thumbnail);
    thumbnail
        .chunks_exact(4)
        .flat_map(|pixel| &pixel[..3])
        .fold(FNV_OFFSET, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
        })
}

/// A hash of every byte of packed pixels, so any change at the delivered
/// resolution alters it: the test for whether the display updated.
pub fn pixels_hash(pixels: &[u8]) -> u64 {
    // Each word goes through an xor, a multiply and a rotation, which spreads
    // a change of any bit over the whole state. Four independent lanes keep
    // the multiplier busy instead of waiting on one chain: about 0.1 ms for a
    // 460x1000 frame.
    const MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;
    let mix = |state: u64, word: u64| (state ^ word).wrapping_mul(MULTIPLIER).rotate_left(29);
    let word = |bytes: &[u8]| u64::from_le_bytes(bytes.try_into().expect("eight bytes"));
    let seed = (pixels.len() as u64).wrapping_mul(MULTIPLIER);
    let mut lanes = [seed, seed ^ 1, seed ^ 2, seed ^ 3];
    let mut blocks = pixels.chunks_exact(32);
    for block in &mut blocks {
        for (lane, bytes) in lanes.iter_mut().zip(block.chunks_exact(8)) {
            *lane = mix(*lane, word(bytes));
        }
    }
    let mut hash = lanes.iter().fold(seed, |hash, &lane| mix(hash, lane));
    for &byte in blocks.remainder() {
        hash = mix(hash, u64::from(byte));
    }
    hash ^ (hash >> 32)
}

/// The rows the status bar takes at the top of a portrait frame.
pub fn status_bar_rows(height: usize) -> usize {
    (height as f64 * STATUS_BAR_FRACTION).ceil() as usize
}

/// The thumbnail hash of the frame below the status bar, whose clock changes
/// on its own and must not keep a screen from settling (S3).
pub fn content_hash(frame: &FrameView<'_>) -> u64 {
    let top = status_bar_rows(frame.height);
    if top >= frame.height {
        return thumbnail_hash(frame);
    }
    thumbnail_hash(&frame.rows_from(top))
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
        Throttle {
            interval: (max_fps > 0).then(|| Duration::from_secs_f64(1.0 / f64::from(max_fps))),
        }
    }

    #[cfg(test)]
    pub fn interval(&self) -> Option<Duration> {
        self.interval
    }

    /// Until when a reader that read at `last` sleeps; presents before then
    /// are stale. `None` when unthrottled.
    pub fn resume_at(&self, last: Instant) -> Option<Instant> {
        self.interval.map(|interval| last + interval.mul_f64(0.75))
    }

    /// How long a resumed reader waits for a fresh present before it reads a
    /// stale one.
    pub fn fresh_wait(&self) -> Duration {
        // A quarter of the interval brings the next scheduled read; the margin
        // covers a present that lands just after it.
        self.interval.map_or(Duration::ZERO, |interval| {
            interval / 4 + Duration::from_millis(2)
        })
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
    fn the_reciprocal_matches_rounded_division_for_every_mean_of_bytes() {
        for count in (1..=1100).chain([4095, 65_536, (1 << 20) - 1, 1 << 20, 3_162_132]) {
            let divisor = Divisor::new(count);
            let count = count as u32;
            let check = |total: u32| {
                let expected = ((total + count / 2) / count) as u8;
                assert_eq!(
                    divisor.rounded([total; 4]),
                    [expected; 4],
                    "{total} / {count}"
                );
            };
            if count <= 1100 {
                (0..=255 * count).for_each(check);
            } else {
                for total in [0, 1, count / 2, count - 1, count, 128 * count, 255 * count] {
                    check(total);
                }
            }
        }
    }

    #[test]
    fn a_tall_span_sums_in_32_bits() {
        // 600 rows into 1: a column sums 600 bytes, beyond 16 bits.
        let (data, stride) = frame(2, 600, 0, |_, _| WHITE);
        assert_eq!(
            scaled(&data, 2, 600, stride, 1, 1),
            vec![255, 255, 255, 255]
        );
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
