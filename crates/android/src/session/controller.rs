//! The emulator's controller in a session: its touches, and its frames on
//! the event stream with hashes that leave the status bar out (design D7).

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use jpeg_encoder::{ColorType, Encoder};
use mirror::backend::{BackendError, EncodeRequest, FrameMeta, FrameSource, StreamItem, Timed};
use protocol::code;
use protocol::message::Rotation;

use crate::emulator::{
    self, BYTES_PER_PIXEL, Controller, Endpoint, Frame, FrameHasher, Options, Unavailable,
};

/// How long after input the agent did not inject a read waits, as the
/// agent's idle wait counts a quiet window from its own input (design D8's
/// 200 ms).
pub const INPUT_QUIET: Duration = Duration::from_millis(200);

/// How many recent frames are kept for encoding.
pub(crate) const KEPT_FRAMES: usize = 4;

/// The longest lag between the emulator's timestamp and a frame's arrival
/// that is believed; anything longer is a clock difference.
const MAX_LAG: Duration = Duration::from_secs(1);

/// How long the frame pump waits for a frame before checking whether it
/// should stop.
const FRAME_WAIT: Duration = Duration::from_millis(250);

/// How long the frame pump waits before subscribing again after the stream
/// failed.
const RESUBSCRIBE_AFTER: Duration = Duration::from_millis(500);

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
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("android-controller")
            .enable_all()
            .build()
            .map_err(|e| {
                Unavailable::new(
                    &endpoint.serial,
                    endpoint.console_port,
                    format!("cannot start the controller's runtime: {e}"),
                )
            })?;
        let controller = runtime.block_on(Controller::connect(endpoint, options))?;
        Ok(ControllerLink {
            runtime,
            controller,
        })
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

    /// Streams frames of `width` into the pump's store and onto its channel
    /// until it is stopped or nothing reads the channel any more.
    pub(crate) fn start_frames(&self, width: u32, pump: FramePump) {
        let controller = self.controller.clone();
        self.runtime.spawn(pump.run(controller, width));
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
    let lag = match frame.timestamp_us() {
        0 => Duration::ZERO,
        produced => Duration::from_micros(now_epoch_us.saturating_sub(produced)).min(MAX_LAG),
    };
    FrameMeta {
        seq,
        at: arrived.checked_sub(lag).unwrap_or(arrived),
        hash: hasher.key(frame).hash,
        width: frame.width(),
        height: frame.height(),
        rotation: Rotation::R0,
    }
}

/// Microseconds since the Unix epoch, the emulator's timestamp clock.
fn epoch_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
}

/// RGB888 `pixels` of `width` by `height` turned clockwise by `rotation`;
/// returns the pixels and their new size.
pub fn rotate_rgb(
    pixels: &[u8],
    width: u32,
    height: u32,
    rotation: Rotation,
) -> (Vec<u8>, u32, u32) {
    if !matches!(rotation, Rotation::R90 | Rotation::R180 | Rotation::R270) {
        return (pixels.to_vec(), width, height);
    }
    let (w, h) = (width as usize, height as usize);
    let (out_w, out_h) = if rotation == Rotation::R180 {
        (w, h)
    } else {
        (h, w)
    };
    let mut out = vec![0u8; out_w * out_h * BYTES_PER_PIXEL];
    for y in 0..out_h {
        for x in 0..out_w {
            let (sx, sy) = match rotation {
                Rotation::R90 => (y, h - 1 - x),
                Rotation::R180 => (w - 1 - x, h - 1 - y),
                _ => (w - 1 - y, x),
            };
            let from = (sy * w + sx) * BYTES_PER_PIXEL;
            let to = (y * out_w + x) * BYTES_PER_PIXEL;
            out[to..to + BYTES_PER_PIXEL].copy_from_slice(&pixels[from..from + BYTES_PER_PIXEL]);
        }
    }
    (out, out_w as u32, out_h as u32)
}

/// The input pixels that output pixel `o` of `to` covers along an axis of
/// `from`: at least one.
fn span(o: u32, from: u32, to: u32) -> (usize, usize) {
    let (o, from, to) = (u64::from(o), u64::from(from), u64::from(to));
    let start = o * from / to;
    let end = ((o + 1) * from).div_ceil(to).max(start + 1).min(from);
    (start as usize, end as usize)
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
    let (w, to_w) = (width as usize, to_width as usize);
    let mut out = vec![0u8; to_w * to_height as usize * BYTES_PER_PIXEL];
    let columns: Vec<(usize, usize)> = (0..to_width).map(|x| span(x, width, to_width)).collect();
    for oy in 0..to_height {
        let (y0, y1) = span(oy, height, to_height);
        for (ox, &(x0, x1)) in columns.iter().enumerate() {
            let mut sum = [0u64; BYTES_PER_PIXEL];
            for y in y0..y1 {
                let row = &pixels[(y * w + x0) * BYTES_PER_PIXEL..(y * w + x1) * BYTES_PER_PIXEL];
                for px in row.chunks_exact(BYTES_PER_PIXEL) {
                    for (total, &channel) in sum.iter_mut().zip(px) {
                        *total += u64::from(channel);
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u64;
            let at = (oy as usize * to_w + ox) * BYTES_PER_PIXEL;
            for (c, total) in sum.iter().enumerate() {
                out[at + c] = ((total + count / 2) / count) as u8;
            }
        }
    }
    out
}

/// `frame` as a JPEG: turned by `request.rotation`, then scaled to exactly
/// `request.width` by `request.height`, at `request.quality`.
pub fn encode_jpeg(frame: &Frame, request: &EncodeRequest) -> Result<Vec<u8>, BackendError> {
    let edge = |pixels: u32| u16::try_from(pixels).ok().filter(|&e| e > 0);
    let (Some(width), Some(height)) = (edge(request.width), edge(request.height)) else {
        return Err(BackendError::new(
            code::INVALID_PARAMS,
            format!(
                "a JPEG of {}x{} pixels cannot be encoded",
                request.width, request.height
            ),
        ));
    };
    let (turned, w, h) = rotate_rgb(
        frame.pixels(),
        frame.width(),
        frame.height(),
        request.rotation,
    );
    let pixels = if (w, h) == (request.width, request.height) {
        turned
    } else {
        resize_rgb(&turned, w, h, request.width, request.height)
    };
    let mut out = Vec::new();
    Encoder::new(&mut out, request.quality.clamp(1, 100))
        .encode(&pixels, width, height, ColorType::Rgb)
        .map_err(|e| BackendError::new(code::INTERNAL, format!("JPEG encoding failed: {e}")))?;
    Ok(out)
}

/// The most recent frames with their metadata, newest last.
#[derive(Default)]
pub(crate) struct FrameStore {
    frames: VecDeque<(FrameMeta, Frame)>,
}

impl FrameStore {
    pub(crate) fn push(&mut self, meta: FrameMeta, frame: Frame) {
        self.frames.push_back((meta, frame));
        while self.frames.len() > KEPT_FRAMES {
            self.frames.pop_front();
        }
    }

    pub(crate) fn latest(&self) -> Option<FrameMeta> {
        self.frames.back().map(|(meta, _)| *meta)
    }

    pub(crate) fn get(&self, seq: u64) -> Option<&Frame> {
        self.frames
            .iter()
            .find(|(meta, _)| meta.seq == seq)
            .map(|(_, frame)| frame)
    }
}

/// What the frame pump shares with the session.
#[derive(Clone)]
pub(crate) struct FramePump {
    pub(crate) store: Arc<Mutex<FrameStore>>,
    pub(crate) items: Sender<Timed<StreamItem>>,
    pub(crate) received: Arc<AtomicU64>,
    pub(crate) stop: Arc<AtomicBool>,
    /// The last failure of the frame stream, for the session's status.
    pub(crate) error: Arc<Mutex<Option<String>>>,
}

impl FramePump {
    async fn run(self, controller: Controller, width: u32) {
        let hasher = FrameHasher::default();
        let mut seq = 0u64;
        while !self.stop.load(Ordering::SeqCst) {
            let mut stream = match controller.frames(width).await {
                Ok(stream) => stream,
                Err(error) => {
                    self.fail(&error);
                    tokio::time::sleep(RESUBSCRIBE_AFTER).await;
                    continue;
                }
            };
            while !self.stop.load(Ordering::SeqCst) {
                match stream.next(FRAME_WAIT).await {
                    Ok(Some(frame)) => {
                        seq += 1;
                        let arrived = Instant::now();
                        let meta = frame_meta(&frame, seq, &hasher, arrived, epoch_us());
                        self.store
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(meta, frame);
                        self.received.fetch_add(1, Ordering::SeqCst);
                        let item = Timed {
                            at: arrived,
                            item: StreamItem::Frame(meta),
                        };
                        if self.items.send(item).is_err() {
                            return;
                        }
                    }
                    Ok(None) => {}
                    // A frame whose bytes do not fit its size; the stream goes on.
                    Err(emulator::Error::Frame { .. }) => {}
                    Err(error) => {
                        self.fail(&error);
                        break;
                    }
                }
            }
            tokio::time::sleep(RESUBSCRIBE_AFTER).await;
        }
    }

    fn fail(&self, error: &emulator::Error) {
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(error.to_string());
    }
}

/// The session's frame source: the latest controller frames.
pub(crate) struct ControllerFrames {
    pub(crate) store: Arc<Mutex<FrameStore>>,
}

impl FrameSource for ControllerFrames {
    fn latest(&self) -> Option<FrameMeta> {
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .latest()
    }

    fn encode(&mut self, request: &EncodeRequest) -> Result<Vec<u8>, BackendError> {
        let frame = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(request.seq)
            .cloned()
            .ok_or_else(|| BackendError {
                code: "no_frame".into(),
                message: format!(
                    "frame {} is no longer held; ask for the latest frame again",
                    request.seq
                ),
                retryable: true,
            })?;
        encode_jpeg(&frame, request)
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
