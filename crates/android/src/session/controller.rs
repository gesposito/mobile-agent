//! The emulator's controller in a session: its touches, and its frames on
//! the event stream with hashes that leave the status bar out (design D7).

use std::collections::VecDeque;
use std::future::Future;
use std::time::{Duration, Instant};

use mirror::backend::{BackendError, EncodeRequest, FrameMeta};
use protocol::message::Rotation;

use crate::emulator::{Controller, Endpoint, Frame, FrameHasher, Options, Unavailable};

/// How long after input the agent did not inject a read waits, as the
/// agent's idle wait counts a quiet window from its own input (design D8's
/// 200 ms).
pub const INPUT_QUIET: Duration = Duration::from_millis(200);

/// How many recent frames are kept for encoding.
pub(crate) const KEPT_FRAMES: usize = 4;

/// A connected controller and the runtime its calls run on.
pub struct ControllerLink {
    runtime: tokio::runtime::Runtime,
    controller: Controller,
}

impl std::fmt::Debug for ControllerLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerLink")
            .field("controller", &self.controller)
            .finish_non_exhaustive()
    }
}

impl ControllerLink {
    /// Connects to the controller at `endpoint` on a runtime of its own.
    pub fn connect(endpoint: &Endpoint, options: Options) -> Result<ControllerLink, Unavailable> {
        let _ = (endpoint, options);
        todo!("task 5.3")
    }

    pub fn controller(&self) -> &Controller {
        &self.controller
    }

    /// Whether the AVD declares a keyboard, so keys could go this way.
    pub fn keyboard(&self) -> bool {
        self.controller.keys_supported()
    }

    /// Runs `future` on the link's runtime from a thread outside it.
    pub(crate) fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}

/// The metadata of `frame`, the `seq`-th of the stream: its hash leaves the
/// status bar out; its capture time is its arrival minus the lag the
/// emulator's timestamp shows, at most a second.
pub fn frame_meta(
    frame: &Frame,
    seq: u64,
    hasher: &FrameHasher,
    arrived: Instant,
    now_epoch_us: u64,
) -> FrameMeta {
    let _ = (frame, seq, hasher, arrived, now_epoch_us);
    todo!("task 5.3")
}

/// RGB888 `pixels` of `width` by `height` turned clockwise by `rotation`;
/// returns the pixels and their new size.
pub fn rotate_rgb(
    pixels: &[u8],
    width: u32,
    height: u32,
    rotation: Rotation,
) -> (Vec<u8>, u32, u32) {
    let _ = (pixels, width, height, rotation);
    todo!("task 5.3")
}

/// RGB888 `pixels` of `width` by `height` scaled to `to_width` by
/// `to_height`: each output pixel averages the input area it covers, or
/// repeats the nearest one when enlarging.
pub fn resize_rgb(
    pixels: &[u8],
    width: u32,
    height: u32,
    to_width: u32,
    to_height: u32,
) -> Vec<u8> {
    let _ = (pixels, width, height, to_width, to_height);
    todo!("task 5.3")
}

/// `frame` as a JPEG: turned by `request.rotation`, then scaled to exactly
/// `request.width` by `request.height`, at `request.quality`.
pub fn encode_jpeg(frame: &Frame, request: &EncodeRequest) -> Result<Vec<u8>, BackendError> {
    let _ = (frame, request);
    todo!("task 5.3")
}

/// The most recent frames with their metadata, newest last.
#[derive(Default)]
pub(crate) struct FrameStore {
    frames: VecDeque<(FrameMeta, Frame)>,
}

impl FrameStore {
    pub(crate) fn push(&mut self, meta: FrameMeta, frame: Frame) {
        let _ = (meta, frame, KEPT_FRAMES);
        todo!("task 5.3")
    }

    pub(crate) fn latest(&self) -> Option<FrameMeta> {
        todo!("task 5.3")
    }

    pub(crate) fn get(&self, seq: u64) -> Option<&Frame> {
        let _ = seq;
        todo!("task 5.3")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::bytes::Bytes;

    fn frame(width: u32, height: u32, fill: impl Fn(u32, u32) -> [u8; 3]) -> Frame {
        let mut pixels = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&fill(x, y));
            }
        }
        Frame::new(width, height, Bytes::from(pixels)).unwrap()
    }

    fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
        let at = ((y * width + x) * 3) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2]]
    }

    #[test]
    fn frame_hashes_leave_the_status_bar_out() {
        let hasher = FrameHasher::default();
        let now = Instant::now();
        let base = frame(10, 100, |_, _| [9, 9, 9]);
        let clock = frame(10, 100, |x, y| {
            if y < 3 && x == 1 {
                [200, 0, 0]
            } else {
                [9, 9, 9]
            }
        });
        let content = frame(10, 100, |x, y| {
            if y == 50 && x == 1 {
                [200, 0, 0]
            } else {
                [9, 9, 9]
            }
        });
        let a = frame_meta(&base, 1, &hasher, now, 0);
        let b = frame_meta(&clock, 2, &hasher, now, 0);
        let c = frame_meta(&content, 3, &hasher, now, 0);
        assert_eq!(a.hash, b.hash, "a status bar change is no change");
        assert_ne!(a.hash, c.hash);
        assert_eq!((c.seq, c.width, c.height), (3, 10, 100));
        assert_eq!(c.rotation, Rotation::R0);
    }

    #[test]
    fn the_capture_time_is_the_arrival_minus_the_emulators_lag() {
        let hasher = FrameHasher::default();
        let arrived = Instant::now();
        let produced = frame(2, 2, |_, _| [0, 0, 0]).with_timing(7, 1_000_000);
        let meta = frame_meta(&produced, 1, &hasher, arrived, 1_015_000);
        assert_eq!(arrived - meta.at, Duration::from_millis(15));
        // A clock that runs ahead or a missing timestamp gives the arrival;
        // a lag over a second is capped.
        assert_eq!(
            frame_meta(&produced, 1, &hasher, arrived, 900_000).at,
            arrived
        );
        let untimed = frame(2, 2, |_, _| [0, 0, 0]);
        assert_eq!(
            frame_meta(&untimed, 1, &hasher, arrived, 5_000_000).at,
            arrived
        );
        let late = frame_meta(&produced, 1, &hasher, arrived, 9_000_000);
        assert_eq!(arrived - late.at, Duration::from_secs(1));
    }

    /// A 3 by 2 image: A B C over D E F.
    fn letters() -> Vec<u8> {
        [b'A', b'B', b'C', b'D', b'E', b'F']
            .iter()
            .flat_map(|&c| [c, c, c])
            .collect()
    }

    fn rows(pixels: &[u8], width: u32, height: u32) -> Vec<String> {
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| pixel(pixels, width, x, y)[0] as char)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn rotation_turns_the_pixels_clockwise() {
        let (r0, w, h) = rotate_rgb(&letters(), 3, 2, Rotation::R0);
        assert_eq!(rows(&r0, w, h), ["ABC", "DEF"]);
        let (r90, w, h) = rotate_rgb(&letters(), 3, 2, Rotation::R90);
        assert_eq!((w, h), (2, 3));
        assert_eq!(rows(&r90, w, h), ["DA", "EB", "FC"]);
        let (r180, w, h) = rotate_rgb(&letters(), 3, 2, Rotation::R180);
        assert_eq!(rows(&r180, w, h), ["FED", "CBA"]);
        let (r270, w, h) = rotate_rgb(&letters(), 3, 2, Rotation::R270);
        assert_eq!(rows(&r270, w, h), ["CF", "BE", "AD"]);
    }

    #[test]
    fn shrinking_averages_and_enlarging_repeats() {
        // Left half red, right half blue, 4 by 2.
        let halves = frame(4, 2, |x, _| if x < 2 { [200, 0, 0] } else { [0, 0, 100] });
        let small = resize_rgb(halves.pixels(), 4, 2, 2, 1);
        assert_eq!(small, [200, 0, 0, 0, 0, 100]);
        // A 2 by 1 checker averages to its mean.
        let checker = [0, 0, 0, 255, 255, 255];
        assert_eq!(resize_rgb(&checker, 2, 1, 1, 1), [128, 128, 128]);
        let one = [10, 20, 30];
        assert_eq!(resize_rgb(&one, 1, 1, 2, 2), [10, 20, 30].repeat(4));
    }

    /// The width and height in a JPEG's start-of-frame segment.
    fn jpeg_size(data: &[u8]) -> (u32, u32) {
        assert_eq!(&data[..2], [0xFF, 0xD8], "no start of image");
        assert_eq!(&data[data.len() - 2..], [0xFF, 0xD9], "no end of image");
        let mut at = 2;
        while at + 9 < data.len() {
            assert_eq!(data[at], 0xFF);
            let marker = data[at + 1];
            let len = u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
            if (0xC0..=0xC2).contains(&marker) {
                let h = u16::from_be_bytes([data[at + 5], data[at + 6]]);
                let w = u16::from_be_bytes([data[at + 7], data[at + 8]]);
                return (u32::from(w), u32::from(h));
            }
            at += 2 + len;
        }
        panic!("no start of frame");
    }

    #[test]
    fn frames_encode_as_jpeg_turned_and_scaled_to_the_request() {
        let wide = frame(160, 80, |x, y| [(x % 256) as u8, (y % 256) as u8, 7]);
        let request = EncodeRequest {
            seq: 1,
            width: 40,
            height: 80,
            quality: 80,
            rotation: Rotation::R90,
        };
        let data = encode_jpeg(&wide, &request).unwrap();
        assert_eq!(jpeg_size(&data), (40, 80));
    }

    #[test]
    fn the_store_keeps_the_latest_frames_by_sequence() {
        let hasher = FrameHasher::default();
        let now = Instant::now();
        let mut store = FrameStore::default();
        assert_eq!(store.latest(), None);
        for seq in 1..=6u64 {
            let f = frame(2, 2, |_, _| [seq as u8, 0, 0]);
            store.push(frame_meta(&f, seq, &hasher, now, 0), f);
        }
        assert_eq!(store.latest().map(|m| m.seq), Some(6));
        assert!(store.get(6).is_some());
        assert!(store.get(3).is_some());
        assert!(store.get(2).is_none(), "only {KEPT_FRAMES} frames are kept");
        assert_eq!(store.get(5).unwrap().pixels()[0], 5);
    }
}
