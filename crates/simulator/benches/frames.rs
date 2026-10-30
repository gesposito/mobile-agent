//! Microbenchmarks of the frame pipeline (design D14), run by
//! `mobile-agent-bench micro`: what the frame thread does with each read of
//! an iPhone 17 Simulator's display at the default frame options, a
//! 1206x2622 BGRA frame box-filtered down to 460x1000, its hashes and, on
//! macOS, its JPEG at quality 80, each alone and in sequence: the sequence
//! without the JPEG, as the daemon's display subscription runs it for each
//! frame whose pixels changed, from the full-size frame to the hash the
//! mirror compares (design D3 of `profiling`), and on macOS with it. The
//! frame is synthetic and built once, before any timing, and every result
//! is verified outside the timed region.

use std::hint::black_box;

#[cfg(target_os = "macos")]
use simulator::Encoder;
use simulator::{
    FrameOptions, FrameView, content_hash, fit_within, pixels_hash, scale_bgra, status_bar_rows,
    thumbnail_hash,
};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

/// The iPhone 17 Simulator's display: 1206x2622 BGRA pixels, each 4,824-byte
/// row padded to 4,864 bytes by its IOSurface (spike S2).
const WIDTH: usize = 1206;
const HEIGHT: usize = 2622;
const STRIDE: usize = 4864;

/// What `fit_within` gives for the display at the default `max_dimension`,
/// 1000, as S2 measured; the ids below name it.
const SCALED: (usize, usize) = (460, 1000);

/// The default JPEG quality; the ids below name it.
const QUALITY: u8 = 80;

/// The bytes S2's frame of the Expo fixture's Big tab took at 460x1000 and
/// quality 80 through turbojpeg.
#[cfg(target_os = "macos")]
const S2_BIG_TAB_JPEG: usize = 82_219;

/// The Expo fixture's Big tab, the frame S2 encoded, in display pixels
/// (fixtures/expo/src/screens/BigScreen.tsx and TabBar.tsx): 20 columns and
/// 25 rows of 19-point cells, centred under the "500 elements" header, and
/// the 56-point tab bar above the home indicator.
const CELL: usize = 57;
const GRID_LEFT: usize = 33;
const GRID_TOP: usize = 348;
const TAB_BAR_TOP: usize = 2352;
const TAB_BAR_HEIGHT: usize = 168;

/// The fixture's colours, in BGRA.
const TEXT: [u8; 4] = [30, 28, 28, 255];
const CELL_FILL: [u8; 4] = [255, 235, 220, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const SEPARATOR: [u8; 4] = [200, 198, 198, 255];
const INACTIVE: [u8; 4] = [147, 142, 142, 255];
const TINT: [u8; 4] = [255, 122, 0, 255];

/// Seven-segment glyphs: their size, the gap between them and their
/// strokes' thickness, in pixels.
#[derive(Clone, Copy)]
struct Font {
    width: usize,
    height: usize,
    gap: usize,
    stroke: usize,
}

/// 17-point semibold: the header and the clock.
const HEADLINE: Font = Font {
    width: 27,
    height: 36,
    gap: 6,
    stroke: 6,
};
/// 15-point medium, and bold for the selected tab: the tab bar.
const LABEL: Font = Font {
    width: 24,
    height: 33,
    gap: 5,
    stroke: 4,
};
const SELECTED: Font = Font { stroke: 6, ..LABEL };
/// 7-point regular: the cells' numbers.
const SMALL: Font = Font {
    width: 12,
    height: 15,
    gap: 2,
    stroke: 2,
};

impl Font {
    fn width_of(self, text: &[u8]) -> usize {
        text.len() * (self.width + self.gap) - self.gap
    }

    /// Whether pixel (`x`, `y`) is ink of `text` written from (`left`,
    /// `top`).
    fn ink(self, text: &[u8], (left, top): (usize, usize), (x, y): (usize, usize)) -> bool {
        if x < left || !(top..top + self.height).contains(&y) {
            return false;
        }
        let (glyph, along) = (
            (x - left) / (self.width + self.gap),
            (x - left) % (self.width + self.gap),
        );
        along < self.width
            && text
                .get(glyph)
                .is_some_and(|&glyph| self.lit(segments(glyph), (along, y - top)))
    }

    /// Whether pixel (`x`, `y`) of a glyph lies on one of the `lit`
    /// segments.
    fn lit(self, lit: u8, (x, y): (usize, usize)) -> bool {
        let Font {
            width,
            height,
            stroke,
            ..
        } = self;
        let (left, right) = (x < stroke, x >= width - stroke);
        let (upper, lower) = (y < height / 2, y >= height / 2);
        let centre = (width - stroke) / 2 <= x && x < (width + stroke) / 2;
        let middle = (height - stroke) / 2 <= y && y < (height + stroke) / 2;
        let dots = (height / 4..height / 4 + stroke).contains(&y)
            || (height * 3 / 4 - stroke..height * 3 / 4).contains(&y);
        let on = [
            y < stroke,
            right && upper,
            right && lower,
            y >= height - stroke,
            left && lower,
            left && upper,
            middle,
            centre && dots,
        ];
        (0..8).any(|segment| on[segment] && lit & (1 << segment) != 0)
    }
}

/// The segments a glyph lights: bits from the top segment clockwise, then
/// the middle one, then a colon's two dots.
fn segments(glyph: u8) -> u8 {
    match glyph {
        b'0' => 0x3F,
        b'1' => 0x06,
        b'2' => 0x5B,
        b'3' => 0x4F,
        b'4' => 0x66,
        b'5' | b'S' | b's' => 0x6D,
        b'6' => 0x7D,
        b'7' => 0x07,
        b'8' => 0x7F,
        b'9' => 0x6F,
        b':' => 0x80,
        b'A' => 0x77,
        b'B' => 0x7C,
        b'L' => 0x38,
        b'e' => 0x7B,
        b'g' => 0x6F,
        b'i' => 0x10,
        b'l' => 0x30,
        b'm' | b'n' => 0x54,
        b'o' => 0x5C,
        b't' => 0x78,
        b'y' => 0x6E,
        _ => 0,
    }
}

/// A synthetic frame of the Expo fixture's Big tab with the status bar
/// clock at 9:`minute`, in the display's layout. The ground darkens a
/// little towards the bottom, and the cells, their numbers, the header and
/// the tab bar give the hashes and the encoder edges and strokes rather
/// than a flat colour; the status bar rows hold the clock, the signal bars
/// and the battery instead.
fn screen(minute: usize) -> Vec<u8> {
    let clock = format!("9:{minute:02}");
    let status_bar = status_bar_rows(HEIGHT);
    // The padding after each row holds bytes no pixel has, as in the
    // pipeline's own tests.
    let mut data = vec![0xAB; STRIDE * HEIGHT];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let pixel = if y < status_bar {
                status_bar_pixel(clock.as_bytes(), (x, y))
            } else {
                content_pixel((x, y))
            };
            let at = y * STRIDE + x * 4;
            data[at..at + 4].copy_from_slice(&pixel);
        }
    }
    data
}

fn ground(y: usize) -> [u8; 4] {
    let shade = (y * 16 / HEIGHT) as u8;
    [255 - shade / 3, 255 - shade * 2 / 3, 255 - shade, 255]
}

fn status_bar_pixel(clock: &[u8], (x, y): (usize, usize)) -> [u8; 4] {
    if HEADLINE.ink(clock, (174, 78), (x, y)) {
        return TEXT;
    }
    // Four signal bars, rising to the right.
    if (870..950).contains(&x) && (84..114).contains(&y) {
        let bar = (x - 870) / 20;
        if (x - 870) % 20 < 14 && y >= 114 - (bar + 1) * 7 {
            return TEXT;
        }
    }
    // The battery's outline and its charge.
    if (1020..1100).contains(&x) && (80..116).contains(&y) {
        let outline = !(1024..1096).contains(&x) || !(84..112).contains(&y);
        let charge = (1028..1080).contains(&x) && (88..108).contains(&y);
        if outline || charge {
            return TEXT;
        }
    }
    ground(y)
}

fn content_pixel((x, y): (usize, usize)) -> [u8; 4] {
    if HEADLINE.ink(b"500 elements", (48, 237), (x, y)) {
        return TEXT;
    }
    if let Some(pixel) = cell_pixel((x, y)) {
        return pixel;
    }
    if (TAB_BAR_TOP..TAB_BAR_TOP + TAB_BAR_HEIGHT).contains(&y) {
        return tab_bar_pixel((x, y));
    }
    // The home indicator.
    if (402..804).contains(&x) && (2585..2600).contains(&y) {
        return TEXT;
    }
    ground(y)
}

/// The pixel of a cell of the grid, inside its white hairline border its
/// fill and its number, if (`x`, `y`) is on one.
fn cell_pixel((x, y): (usize, usize)) -> Option<[u8; 4]> {
    let (across, down) = (x.checked_sub(GRID_LEFT)?, y.checked_sub(GRID_TOP)?);
    let (column, row) = (across / CELL, down / CELL);
    if column >= 20 || row >= 25 {
        return None;
    }
    let (x, y) = (across % CELL, down % CELL);
    if x == 0 || y == 0 || x == CELL - 1 || y == CELL - 1 {
        return Some(WHITE);
    }
    let number = row * 20 + column + 1;
    let digits = [
        b'0' + (number / 100) as u8,
        b'0' + (number / 10 % 10) as u8,
        b'0' + (number % 10) as u8,
    ];
    let digits = &digits[2 - number.ilog10() as usize..];
    let origin = (
        (CELL - SMALL.width_of(digits)) / 2,
        (CELL - SMALL.height) / 2,
    );
    Some(if SMALL.ink(digits, origin, (x, y)) {
        TEXT
    } else {
        CELL_FILL
    })
}

/// The tab bar: a separator over five labels, the Big tab's selected.
fn tab_bar_pixel((x, y): (usize, usize)) -> [u8; 4] {
    const LABELS: [&[u8]; 5] = [b"Login", b"List", b"Big", b"Anim", b"System"];
    if y == TAB_BAR_TOP {
        return SEPARATOR;
    }
    let tab = x * 5 / WIDTH;
    let (font, colour) = if tab == 2 {
        (SELECTED, TINT)
    } else {
        (LABEL, INACTIVE)
    };
    let origin = (
        (2 * tab + 1) * WIDTH / 10 - font.width_of(LABELS[tab]) / 2,
        TAB_BAR_TOP + (TAB_BAR_HEIGHT - font.height) / 2,
    );
    if font.ink(LABELS[tab], origin, (x, y)) {
        colour
    } else {
        WHITE
    }
}

/// The top left pixel inside the border of the cell at `column` and `row`.
fn inside(column: usize, row: usize) -> (usize, usize) {
    (GRID_LEFT + column * CELL + 1, GRID_TOP + row * CELL + 1)
}

/// Paints the `width` x `height` block at (`left`, `top`) of a frame in the
/// display's layout in the tint colour, as a pressed cell is filled.
fn paint(frame: &mut [u8], (left, top): (usize, usize), (width, height): (usize, usize)) {
    for y in top..top + height {
        for x in left..left + width {
            let at = y * STRIDE + x * 4;
            frame[at..at + 4].copy_from_slice(&TINT);
        }
    }
}

fn view(frame: &[u8]) -> FrameView<'_> {
    FrameView::new(frame, WIDTH, HEIGHT, STRIDE).expect("a frame in the display's layout")
}

fn packed(scaled: &[u8]) -> FrameView<'_> {
    FrameView::new(scaled, SCALED.0, SCALED.1, SCALED.0 * 4).expect("a packed frame of its size")
}

/// The frame's three hashes as the frame thread takes them, of its
/// downscaled pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hashes {
    pixels: u64,
    thumbnail: u64,
    content: u64,
}

fn hashes(frame: &[u8]) -> Hashes {
    let mut scaled = Vec::new();
    scale_bgra(&view(frame), SCALED.0, SCALED.1, &mut scaled);
    let output = packed(&scaled);
    Hashes {
        pixels: pixels_hash(&scaled),
        thumbnail: thumbnail_hash(&output),
        content: content_hash(&output),
    }
}

/// The frame thread's encode step: the shipped encoder at the default
/// quality on macOS, and on other hosts, which have no encoder, nothing.
#[cfg(target_os = "macos")]
fn encode_step() -> impl FnMut(&[u8], usize, usize) -> Option<Vec<u8>> {
    let mut encoder = Encoder::new(QUALITY).expect("an encoder is compiled in");
    move |pixels, width, height| encoder.encode(pixels, width, height).ok()
}

#[cfg(not(target_os = "macos"))]
fn encode_step() -> impl FnMut(&[u8], usize, usize) -> Option<Vec<u8>> {
    no_jpeg
}

/// The encode step of a frame thread that only hashes, as the daemon's
/// display subscription runs it: no JPEG.
fn no_jpeg(_: &[u8], _: usize, _: usize) -> Option<Vec<u8>> {
    None
}

/// What the frame thread delivers of a read.
#[derive(Debug, PartialEq, Eq)]
struct Delivered {
    size: (usize, usize),
    hash: u64,
    jpeg: Option<Vec<u8>>,
    content_hash: u64,
}

/// A read of the frame thread (`read_frames` in `src/macos/frames.rs`)
/// whose pixels changed, in its order: the downscale to `max_dimension`
/// into `scaled`, the hash of every delivered pixel, the JPEG, then the
/// content hash of the delivered frame.
fn read(
    source: &FrameView<'_>,
    max_dimension: usize,
    scaled: &mut Vec<u8>,
    encode: &mut impl FnMut(&[u8], usize, usize) -> Option<Vec<u8>>,
) -> Delivered {
    let (width, height) = fit_within(source.width(), source.height(), max_dimension);
    scale_bgra(source, width, height, scaled);
    let hash = pixels_hash(scaled);
    let jpeg = encode(scaled, width, height);
    let view =
        FrameView::new(scaled, width, height, width * 4).expect("a packed frame of its own size");
    Delivered {
        size: (width, height),
        hash,
        jpeg,
        content_hash: content_hash(&view),
    }
}

/// Checks the shipped encoder's JPEG of the scaled frame: the encoder the
/// build prefers, the JPEG's first and last markers, and a size within a
/// tenth of S2's frame of the same screen, so that the encoder meets a real
/// screen's detail.
#[cfg(target_os = "macos")]
fn verify_jpeg(scaled: &[u8]) -> Vec<u8> {
    let mut encoder = Encoder::new(QUALITY).expect("an encoder is compiled in");
    let expected = if cfg!(feature = "turbojpeg") {
        "turbojpeg"
    } else {
        "jpeg-encoder"
    };
    assert_eq!(encoder.name(), expected, "the build's preferred encoder");
    let jpeg = encoder
        .encode(scaled, SCALED.0, SCALED.1)
        .expect("the frame encodes");
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "the JPEG starts with SOI");
    assert_eq!(
        &jpeg[jpeg.len() - 2..],
        &[0xFF, 0xD9],
        "the JPEG ends with EOI"
    );
    assert!(
        jpeg.len().abs_diff(S2_BIG_TAB_JPEG) < S2_BIG_TAB_JPEG / 10,
        "the JPEG has {} bytes, S2's frame of the Big tab {S2_BIG_TAB_JPEG}",
        jpeg.len()
    );
    jpeg
}

/// Checks, before anything is timed, what the benchmarks time: the
/// defaults the ids name, the scaled size, hashes that hold across calls,
/// see a change in the content and, for `content_hash`, none in the status
/// bar, on macOS a whole JPEG, and a read of the frame thread that delivers
/// all of them.
fn verify(frame: &[u8], scaled: &[u8]) {
    let options = FrameOptions::default();
    let max_dimension = options.max_dimension as usize;
    assert_eq!(
        fit_within(WIDTH, HEIGHT, max_dimension),
        SCALED,
        "the default frame size"
    );
    assert_eq!(options.quality, QUALITY, "the default quality");
    assert_eq!(
        scaled.len(),
        SCALED.0 * SCALED.1 * 4,
        "the scaled frame's bytes"
    );

    let base = hashes(frame);
    assert_eq!(hashes(frame), base, "the hashes hold across calls");

    // One pixel of the content changes the hash of every delivered pixel.
    // The thumbnail hashes see only what moves a cell of their 32-pixel
    // thumbnail by half a level, such as a cell of the grid shown pressed.
    let mut pixel = frame.to_vec();
    paint(&mut pixel, inside(10, 20), (1, 1));
    assert_ne!(
        hashes(&pixel).pixels,
        base.pixels,
        "pixels_hash sees a content pixel"
    );
    let mut pressed = frame.to_vec();
    paint(&mut pressed, inside(9, 12), (CELL - 2, CELL - 2));
    let changed = hashes(&pressed);
    assert_ne!(
        changed.pixels, base.pixels,
        "pixels_hash sees a pressed cell"
    );
    assert_ne!(
        changed.thumbnail, base.thumbnail,
        "thumbnail_hash sees a pressed cell"
    );
    assert_ne!(
        changed.content, base.content,
        "content_hash sees a pressed cell"
    );

    // The clock's tick changes every hash but the content hash.
    let ticked = hashes(&screen(42));
    assert_ne!(
        ticked.pixels, base.pixels,
        "pixels_hash sees the clock tick"
    );
    assert_ne!(
        ticked.thumbnail, base.thumbnail,
        "thumbnail_hash sees the clock tick"
    );
    assert_eq!(
        ticked.content, base.content,
        "content_hash leaves the status bar out"
    );

    #[cfg(target_os = "macos")]
    let jpeg = Some(verify_jpeg(scaled));
    #[cfg(not(target_os = "macos"))]
    let jpeg = None;

    let mut buffer = Vec::new();
    let delivered = read(&view(frame), max_dimension, &mut buffer, &mut encode_step());
    assert_eq!(buffer, scaled, "the read's frame");
    assert_eq!(
        delivered,
        Delivered {
            size: SCALED,
            hash: base.pixels,
            jpeg,
            content_hash: base.content,
        },
        "the read's size, hashes and JPEG"
    );

    // The read that only hashes delivers the same frame and hashes.
    let mut buffer = Vec::new();
    let hashed = read(&view(frame), max_dimension, &mut buffer, &mut no_jpeg);
    assert_eq!(buffer, scaled, "the hashing read's frame");
    assert_eq!(
        hashed,
        Delivered {
            size: SCALED,
            hash: base.pixels,
            jpeg: None,
            content_hash: base.content,
        },
        "the hashing read's size and hashes"
    );
}

fn benches(c: &mut Criterion) {
    let max_dimension = FrameOptions::default().max_dimension as usize;
    let frame = screen(41);
    let source = view(&frame);
    let mut scaled = Vec::new();
    scale_bgra(&source, SCALED.0, SCALED.1, &mut scaled);
    verify(&frame, &scaled);
    let output = packed(&scaled);
    #[cfg(target_os = "macos")]
    let mut encode = encode_step();
    let mut buffer = scaled.clone();

    // Criterion's `iter` drops each routine's output inside the timed
    // region, as spike S8 timed its decode; `verify` checked those outputs,
    // and the frame the timed downscales leave is checked after them.
    let mut group = c.benchmark_group("simulator");
    group.bench_function(
        BenchmarkId::new("scale_bgra", "1206x2622-to-460x1000"),
        |b| {
            b.iter(|| {
                scale_bgra(
                    black_box(&source),
                    SCALED.0,
                    SCALED.1,
                    black_box(&mut buffer),
                )
            })
        },
    );
    group.bench_function(BenchmarkId::new("pixels_hash", "460x1000"), |b| {
        b.iter(|| pixels_hash(black_box(&scaled)))
    });
    group.bench_function(BenchmarkId::new("thumbnail_hash", "460x1000"), |b| {
        b.iter(|| thumbnail_hash(black_box(&output)))
    });
    group.bench_function(BenchmarkId::new("content_hash", "460x1000"), |b| {
        b.iter(|| content_hash(black_box(&output)))
    });
    #[cfg(target_os = "macos")]
    group.bench_function(BenchmarkId::new("encode", "460x1000-q80"), |b| {
        b.iter(|| encode(black_box(&scaled), SCALED.0, SCALED.1))
    });
    // The frame's hash end to end, which the benchmarks above time apart:
    // the downscale, the hash of every delivered pixel and the content hash,
    // a thumbnail's below the status bar, which the mirror compares.
    group.bench_function(BenchmarkId::new("pipeline", "frame-to-hashes"), |b| {
        b.iter(|| read(black_box(&source), max_dimension, &mut buffer, &mut no_jpeg))
    });
    #[cfg(target_os = "macos")]
    group.bench_function(BenchmarkId::new("pipeline", "frame-to-jpeg"), |b| {
        b.iter(|| read(black_box(&source), max_dimension, &mut buffer, &mut encode))
    });
    group.finish();
    assert_eq!(buffer, scaled, "the timed downscales' frame");
}

criterion_group!(frames, benches);
criterion_main!(frames);
