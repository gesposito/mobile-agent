//! Microbenchmark of the emulator frame hash (design D14), run by
//! `mobile-agent-bench micro`: `FrameHasher::key`, which the session takes
//! of every frame the controller streams, of a frame at the session's
//! working width. The frame is synthetic and built once, before any timing,
//! and its key is verified outside the timed region.

use std::hint::black_box;

use android::emulator::{BYTES_PER_PIXEL, Frame, FrameHasher};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use prost::bytes::Bytes;

/// The frames the session subscribes to: 540 pixels wide, RGB888, 540x1200
/// from the emulator's 1080x2400 Pixel 8 display (spike S7).
const WIDTH: usize = 540;
const HEIGHT: usize = 1200;

/// The content's cells: 20 columns and 25 rows, as on the fixtures' Big tab.
const CELL_WIDTH: usize = 27;
const CELL_HEIGHT: usize = 45;

/// A synthetic frame, rows top-down. Below the status bar, a vertical
/// gradient behind a grid of cells with white borders and a dark mark each,
/// so that the hash meets structure rather than a flat colour; the status
/// bar rows the hasher leaves out hold dark stripes instead.
fn screen() -> Vec<u8> {
    let status_bar = FrameHasher::default().status_bar_rows(HEIGHT as u32) as usize;
    let mut pixels = Vec::with_capacity(WIDTH * HEIGHT * BYTES_PER_PIXEL);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let pixel = if y < status_bar {
                if (x / 6 + y / 12) % 3 == 0 {
                    [230, 230, 230]
                } else {
                    [20, 20, 24]
                }
            } else {
                content_pixel(x, y - status_bar)
            };
            pixels.extend_from_slice(&pixel);
        }
    }
    pixels
}

fn content_pixel(x: usize, y: usize) -> [u8; 3] {
    let (cx, cy) = (x % CELL_WIDTH, y % CELL_HEIGHT);
    if cx == 0 || cy == 0 || y / CELL_HEIGHT >= 25 {
        return [255, 255, 255];
    }
    if (9..18).contains(&cx) && (17..28).contains(&cy) {
        return [28, 28, 30];
    }
    let shade = (y * 64 / HEIGHT) as u8;
    [220 - shade, 235 - shade / 2, 255]
}

fn frame(pixels: Vec<u8>) -> Frame {
    Frame::new(WIDTH as u32, HEIGHT as u32, Bytes::from(pixels)).expect("a 540x1200 RGB888 frame")
}

/// Flips the bytes of pixel (`x`, `y`).
fn poke(pixels: &mut [u8], (x, y): (usize, usize)) {
    let at = (y * WIDTH + x) * BYTES_PER_PIXEL;
    for byte in &mut pixels[at..at + BYTES_PER_PIXEL] {
        *byte ^= 0xFF;
    }
}

/// Checks, before anything is timed, the key the benchmark takes: the
/// frame's dimensions, a hash that holds across calls, sees one pixel of
/// the content change and leaves the status bar out.
fn verify(hasher: &FrameHasher, pixels: &[u8]) {
    let key = hasher.key(&frame(pixels.to_vec()));
    assert_eq!((key.width, key.height), (540, 1200), "the key's dimensions");
    assert_eq!(
        hasher.key(&frame(pixels.to_vec())),
        key,
        "the key holds across calls"
    );

    let mut content = pixels.to_vec();
    poke(&mut content, (300, 700));
    assert_ne!(
        hasher.key(&frame(content)).hash,
        key.hash,
        "the key sees a content pixel"
    );

    let mut clock = pixels.to_vec();
    poke(&mut clock, (60, 40));
    assert_eq!(
        hasher.key(&frame(clock)).hash,
        key.hash,
        "the key leaves the status bar out"
    );
}

fn benches(c: &mut Criterion) {
    let hasher = FrameHasher::default();
    let pixels = screen();
    verify(&hasher, &pixels);
    let frame = frame(pixels);

    // Criterion's `iter` drops each key inside the timed region, as spike
    // S8 timed its decode; `verify` checked the key.
    let mut group = c.benchmark_group("android");
    group.bench_function(BenchmarkId::new("frame-hasher", "key-540x1200"), |b| {
        b.iter(|| hasher.key(black_box(&frame)))
    });
    group.finish();
}

criterion_group!(frames, benches);
criterion_main!(frames);
