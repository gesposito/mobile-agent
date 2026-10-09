//! Frames and screenshots from the host (design D23): DVT's
//! `takeScreenshot` and CoreDevice's `capturescreenshot` polled side by
//! side, each on a connection of its own, their PNGs turned into frames by
//! the pipeline (`crate::frames`) on a thread of its own; and the on-demand
//! screenshot through DVT, at the native size or scaled and encoded.
//!
//! The display stream and AVFoundation are later additions: the display
//! service of iOS 26.6.2 lists no media feature and refuses the stream, and
//! AVFoundation needs the macOS camera permission (S11). `frame_paths`
//! says which path serves and why the others do not.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use idevice::ReadWrite;
use idevice::core_device::{
    DisplayServiceClient, ImageFormat as CaptureFormat, ScreenCaptureServiceClient,
};
use plist::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::driver::Target;
use super::dvt::{self, DvtChannel};
use super::host::{Host, describe};
use super::tunnel::Limits;
use crate::error::{Error, Result};
use crate::frames::{FrameFeed, FrameQueue, Pipeline};
use crate::model::{
    Capture, Frame, FrameOptions, FramePath, FramePathStatus, FrameStats, ImageFormat, Screenshot,
    ScreenshotOptions,
};

/// DVT's screenshot channel.
pub(crate) const SCREENSHOT: &str = "com.apple.instruments.server.services.screenshot";
/// Its one call, answered with the screen as a PNG.
pub(crate) const TAKE_SCREENSHOT: &str = "takeScreenshot";

pub(crate) type ScreenCapture = ScreenCaptureServiceClient<Box<dyn ReadWrite>>;
pub(crate) type DisplayService = DisplayServiceClient<Box<dyn ReadWrite>>;

/// The call DVT's screenshot channel takes.
pub(crate) fn take_screenshot() -> dvt::Call {
    dvt::Call {
        selector: TAKE_SCREENSHOT,
        args: Vec::new(),
    }
}

/// The PNG a screenshot's reply carries.
pub(crate) fn screenshot_png(
    reply: &idevice::dvt::message::Message,
) -> std::result::Result<Vec<u8>, String> {
    match &reply.data {
        Some(Value::Data(png)) => Ok(png.clone()),
        other => Err(format!("the reply carried {other:?}, not a picture")),
    }
}

/// A PNG's size from its header.
pub(crate) fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    (png.len() >= 24 && &png[1..4] == b"PNG").then(|| {
        (
            u32::from_be_bytes([png[16], png[17], png[18], png[19]]),
            u32::from_be_bytes([png[20], png[21], png[22], png[23]]),
        )
    })
}

/// What a poller hands the pipeline.
enum Taken {
    Png {
        png: Vec<u8>,
        source: Capture,
        requested: Instant,
        arrived: Instant,
    },
    Failed {
        source: Capture,
        reason: String,
    },
}

/// One connection's capture client.
pub(crate) enum Poller {
    Dvt(DvtChannel),
    CoreDevice(Box<ScreenCapture>),
}

impl Poller {
    fn source(&self) -> Capture {
        match self {
            Poller::Dvt(_) => Capture::Dvt,
            Poller::CoreDevice(_) => Capture::CoreDevice,
        }
    }

    async fn capture(&mut self, wait: Duration) -> std::result::Result<Vec<u8>, String> {
        match self {
            Poller::Dvt(channel) => {
                let reply = channel
                    .call(take_screenshot(), wait)
                    .await
                    .map_err(|error| format!("DVT's takeScreenshot: {error:?}"))?;
                screenshot_png(&reply)
            }
            Poller::CoreDevice(client) => {
                timeout(wait, client.take_screenshot(None, CaptureFormat::Png))
                    .await
                    .map_err(|_| {
                        format!("CoreDevice's capturescreenshot did not answer within {wait:?}")
                    })?
                    .map_err(|error| {
                        format!("CoreDevice's capturescreenshot: {}", describe(&error))
                    })
            }
        }
    }
}

/// When a poller requests its captures: `None` back to back, otherwise one
/// every `every`, the connections staggered so that together they take at
/// most `max_fps` a second.
pub(crate) fn pace(max_fps: u32, pollers: u32) -> Option<Duration> {
    (max_fps > 0).then(|| Duration::from_secs(u64::from(pollers)) / max_fps)
}

/// Polls `poller` until the subscription ends or a capture fails.
async fn poll(
    mut poller: Poller,
    every: Option<Duration>,
    offset: Duration,
    wait: Duration,
    taken: Sender<Taken>,
    mut stop: watch::Receiver<bool>,
) {
    let source = poller.source();
    let mut next = tokio::time::Instant::now() + offset;
    loop {
        if let Some(every) = every {
            tokio::select! {
                () = tokio::time::sleep_until(next) => {}
                _ = stop.changed() => return,
            }
            next = (next + every).max(tokio::time::Instant::now());
        }
        if *stop.borrow() {
            return;
        }
        let requested = Instant::now();
        let captured = tokio::select! {
            captured = poller.capture(wait) => captured,
            _ = stop.changed() => return,
        };
        let message = match captured {
            Ok(png) => Taken::Png {
                png,
                source,
                requested,
                arrived: Instant::now(),
            },
            Err(reason) => Taken::Failed { source, reason },
        };
        let failed = matches!(message, Taken::Failed { .. });
        if taken.send(message).is_err() || failed {
            return;
        }
    }
}

/// What the stream's parts share.
struct Shared {
    pipeline: Mutex<Pipeline>,
    queue: FrameQueue,
}

impl Shared {
    fn pipeline(&self) -> MutexGuard<'_, Pipeline> {
        self.pipeline.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The pipeline's thread: every capture in, frames to the queue, until
/// both pollers ended.
fn run_pipeline(taken: Receiver<Taken>, shared: Arc<Shared>) {
    while let Ok(message) = taken.recv() {
        match message {
            Taken::Png {
                png,
                source,
                requested,
                arrived,
            } => {
                let frame = shared.pipeline().take(&png, source, requested, arrived);
                if let Some(frame) = frame
                    && shared.queue.push(frame)
                {
                    shared.pipeline().dropped();
                }
            }
            Taken::Failed { source, reason } => shared.pipeline().failed(source, &reason),
        }
    }
    shared.queue.close();
}

/// A subscription's feed: the pollers on the driver's runtime and the
/// pipeline on its thread.
struct Polled {
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
    pollers: Vec<JoinHandle<()>>,
}

impl Drop for Polled {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        for poller in &self.pollers {
            poller.abort();
        }
    }
}

impl FrameFeed for Polled {
    fn next(&self, timeout: Duration) -> Option<Frame> {
        self.shared.queue.pop(timeout)
    }

    fn latest(&self) -> Option<Frame> {
        self.shared.pipeline().latest()
    }

    fn encode(&self, sequence: u64, width: u32, height: u32, quality: u8) -> Result<Vec<u8>> {
        let pipeline = self.shared.pipeline();
        let held = pipeline
            .held(sequence)
            .ok_or_else(|| Error::InvalidArgument {
                reason: format!(
                    "frame {sequence} is no longer held; ask for the latest frame again"
                ),
            })?;
        encode_jpeg(
            &held.pixels,
            held.frame.width as usize,
            held.frame.height as usize,
            (width as usize, height as usize),
            quality,
        )
    }

    fn stats(&self) -> FrameStats {
        self.shared.pipeline().stats()
    }
}

/// `pixels`, packed BGRA of `width` by `height`, scaled to `to` (no larger)
/// and encoded as a JPEG at `quality`.
pub(crate) fn encode_jpeg(
    pixels: &[u8],
    width: usize,
    height: usize,
    to: (usize, usize),
    quality: u8,
) -> Result<Vec<u8>> {
    let scaled = scaled(pixels, width, height, to)?;
    let mut encoder = simulator::Encoder::new(quality).map_err(|error| Error::Io {
        what: "starting the JPEG encoder".into(),
        reason: error.to_string(),
    })?;
    encoder
        .encode(&scaled, to.0, to.1)
        .map_err(|error| Error::Io {
            what: "encoding a JPEG".into(),
            reason: error.to_string(),
        })
}

/// `pixels`, packed BGRA of `width` by `height`, scaled down to `to`.
fn scaled(pixels: &[u8], width: usize, height: usize, to: (usize, usize)) -> Result<Vec<u8>> {
    if to == (width, height) {
        return Ok(pixels.to_vec());
    }
    if to.0 == 0 || to.1 == 0 || to.0 > width || to.1 > height {
        return Err(Error::InvalidArgument {
            reason: format!(
                "a {}x{} image cannot be made from a {width}x{height} frame, which is only \
                 scaled down",
                to.0, to.1
            ),
        });
    }
    let view = simulator::FrameView::new(pixels, width, height, width * 4).map_err(|error| {
        Error::InvalidArgument {
            reason: error.to_string(),
        }
    })?;
    let mut out = Vec::with_capacity(to.0 * to.1 * 4);
    simulator::scale_bgra(&view, to.0, to.1, &mut out);
    Ok(out)
}

/// Packed BGRA of `width` by `height` as an 8-bit RGBA PNG.
fn encode_png(pixels: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let rgba: Vec<u8> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[blue, green, red, alpha]| [red, green, blue, alpha])
        .collect();
    let mut out = Vec::new();
    let failed = |error: png::EncodingError| Error::Io {
        what: "encoding a PNG".into(),
        reason: error.to_string(),
    };
    let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(failed)?;
    writer.write_image_data(&rgba).map_err(failed)?;
    drop(writer);
    Ok(out)
}

impl<H: Host> Target<H> {
    /// Opens the two capture connections and starts the subscription.
    pub(crate) async fn frames(
        &mut self,
        options: &FrameOptions,
        limits: Limits,
        runtime: &tokio::runtime::Handle,
    ) -> Result<crate::frames::FrameStream> {
        let screen = self.screen().await?;
        let dvt = self.open_channel(SCREENSHOT, limits).await?;
        let core_device = self.tunnel.connect::<ScreenCapture>().await?;
        let pollers = [Poller::Dvt(dvt), Poller::CoreDevice(Box::new(core_device))];
        let shared = Arc::new(Shared {
            pipeline: Mutex::new(Pipeline::new(options, screen)),
            queue: FrameQueue::new(),
        });
        let (taken, receiver) = mpsc::channel();
        let (stop, stopping) = watch::channel(false);
        let every = pace(options.max_fps, pollers.len() as u32);
        let count = pollers.len() as u32;
        let handles = pollers
            .into_iter()
            .enumerate()
            .map(|(index, poller)| {
                let offset = every.map_or(Duration::ZERO, |every| every * index as u32 / count);
                runtime.spawn(poll(
                    poller,
                    every,
                    offset,
                    limits.reply,
                    taken.clone(),
                    stopping.clone(),
                ))
            })
            .collect();
        drop(taken);
        let pipeline = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("ios-frames".into())
            .spawn(move || run_pipeline(receiver, pipeline))
            .map_err(|error| Error::Io {
                what: "starting the frame pipeline".into(),
                reason: error.to_string(),
            })?;
        Ok(crate::frames::FrameStream::new(Box::new(Polled {
            shared,
            stop,
            pollers: handles,
        })))
    }

    /// The screen now, through DVT's screenshot channel: the device's PNG
    /// at the native size, or scaled to `options.max_dimension` and
    /// encoded as `options.format`.
    pub(crate) async fn screenshot(
        &mut self,
        options: &ScreenshotOptions,
        limits: Limits,
    ) -> Result<Screenshot> {
        let started = Instant::now();
        let reply = self
            .call(SCREENSHOT, take_screenshot(), true, limits)
            .await?;
        let png = screenshot_png(&reply).map_err(|reason| Error::Device {
            call: "DVT's takeScreenshot".into(),
            reason,
        })?;
        let unreadable = |reason: String| Error::Device {
            call: "DVT's takeScreenshot".into(),
            reason,
        };
        if options.format == ImageFormat::Png && options.max_dimension == 0 {
            let (width, height) =
                png_size(&png).ok_or_else(|| unreadable("the picture is not a PNG".into()))?;
            return Ok(Screenshot {
                width,
                height,
                format: ImageFormat::Png,
                data: png,
                elapsed: started.elapsed(),
            });
        }
        let mut pixels = Vec::new();
        let (width, height) = crate::frames::decode_png(&png, &mut pixels).map_err(unreadable)?;
        let to = simulator::fit_within(width, height, options.max_dimension as usize);
        let data = match options.format {
            ImageFormat::Jpeg => encode_jpeg(&pixels, width, height, to, options.quality)?,
            ImageFormat::Png => encode_png(&scaled(&pixels, width, height, to)?, to.0, to.1)?,
        };
        Ok(Screenshot {
            width: to.0 as u32,
            height: to.1 as u32,
            format: options.format,
            data,
            elapsed: started.elapsed(),
        })
    }

    /// Which frame path serves the device, and why the others do not: the
    /// display service is asked what media it supports.
    pub(crate) async fn frame_paths(&mut self, limits: Limits) -> Result<Vec<FramePathStatus>> {
        self.tunnel.open().await?;
        let missing: Vec<&str> = self
            .tunnel
            .status()
            .map(|status| {
                [crate::services::DVT, crate::services::SCREEN_CAPTURE]
                    .into_iter()
                    .filter(|needed| !status.services.iter().any(|listed| listed == needed))
                    .collect()
            })
            .unwrap_or_default();
        let screenshots = FramePathStatus {
            path: FramePath::Screenshots,
            active: missing.is_empty(),
            reason: (!missing.is_empty())
                .then(|| format!("RSD does not list {}", missing.join(" and "))),
        };
        let display = self.display_stream(limits).await;
        Ok(vec![screenshots, display, avfoundation()])
    }

    /// The display stream's state: off, and why.
    async fn display_stream(&mut self, limits: Limits) -> FramePathStatus {
        let off = |reason: String| FramePathStatus {
            path: FramePath::DisplayStream,
            active: false,
            reason: Some(reason),
        };
        let mut client = match self.tunnel.connect::<DisplayService>().await {
            Ok(client) => client,
            Err(error) => return off(format!("the display service could not be reached: {error}")),
        };
        let info = match timeout(limits.reply, client.get_media_support_info()).await {
            Ok(Ok(info)) => info,
            Ok(Err(error)) => {
                return off(format!(
                    "the display service's getmediasupportinfo failed: {}",
                    describe(&error)
                ));
            }
            Err(_) => return off("the display service did not answer".into()),
        };
        let features = info
            .as_dictionary()
            .and_then(|info| info.get("supportedFeatures"))
            .and_then(Value::as_unsigned_integer)
            .unwrap_or(0);
        if features == 0 {
            off(
                "the display service lists no media features (supportedFeatures 0): iOS \
                 26.6.2 refuses the stream until iOS 27 (S11), so frames come from screenshots"
                    .into(),
            )
        } else {
            off(format!(
                "the display service lists media features ({features:#x}), but the display \
                 stream is a later addition, not in this driver yet"
            ))
        }
    }
}

/// AVFoundation's capture: off, a later addition.
fn avfoundation() -> FramePathStatus {
    FramePathStatus {
        path: FramePath::AvFoundation,
        active: false,
        reason: Some(
            "a later addition: macOS shows the phone to AVFoundation only once CoreMediaIO's \
             screen-capture allowance is set, and its frames need the macOS camera permission \
             for the process that starts the daemon (S11)"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::frames::tests::screen;
    use crate::macos::driver::tests::{FakeHost, PHONE, core};

    /// A JPEG's width and height, from its start-of-frame segment.
    fn jpeg_size(data: &[u8]) -> (u32, u32) {
        assert_eq!(&data[..2], [0xFF, 0xD8], "no start of image");
        let mut at = 2;
        while at + 9 < data.len() {
            let marker = data[at + 1];
            let length = u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
            if (0xC0..=0xC2).contains(&marker) {
                let height = u16::from_be_bytes([data[at + 5], data[at + 6]]);
                let width = u16::from_be_bytes([data[at + 7], data[at + 8]]);
                return (u32::from(width), u32::from(height));
            }
            at += 2 + length;
        }
        panic!("no start of frame");
    }

    fn polling() -> FrameOptions {
        FrameOptions {
            max_fps: 0,
            max_dimension: 0,
            status_bar: None,
        }
    }

    /// A host whose screen shows `png`, each capture taking 5 ms.
    fn showing(png: Vec<u8>) -> FakeHost {
        let host = FakeHost::ready();
        host.device.show(png);
        host.device.capture_delay(Duration::from_millis(5));
        host
    }

    #[test]
    fn frames_are_polled_on_dvt_and_coredevice_side_by_side() {
        let core = core(showing(screen(|_, _| 120, false)));
        let stream = core.frames(PHONE, &polling()).expect("subscribed");
        let first = stream
            .next(Duration::from_secs(1))
            .expect("the first frame");
        assert_eq!(first.sequence, 1);
        std::thread::sleep(Duration::from_millis(200));
        let stats = stream.stats();
        assert!(
            stats.captures[0] > 2 && stats.captures[1] > 2,
            "both connections poll: {stats:?}"
        );
        let served = core.host.device.captures();
        assert!(served[0] > 2 && served[1] > 2, "{served:?}");
    }

    #[test]
    fn a_still_screen_counts_no_frame_after_the_first() {
        let core = core(showing(screen(|_, _| 120, false)));
        let stream = core.frames(PHONE, &polling()).expect("subscribed");
        stream
            .next(Duration::from_secs(1))
            .expect("the first frame");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(stream.try_next(), None, "nothing while the screen is still");
        let stats = stream.stats();
        assert_eq!(stats.delivered, 1, "{stats:?}");
        assert_eq!(
            stats.decoded, 1,
            "every repeat skipped before decoding: {stats:?}"
        );
        assert!(stats.identical >= stats.captures.iter().sum::<u64>() - 1);
    }

    #[test]
    fn a_change_on_screen_is_the_next_frame() {
        let core = core(showing(screen(|_, _| 120, false)));
        let stream = core.frames(PHONE, &polling()).expect("subscribed");
        let first = stream
            .next(Duration::from_secs(1))
            .expect("the first frame");
        core.host.device.show(screen(
            |x, y| if (x, y) == (40, 90) { 200 } else { 120 },
            false,
        ));
        let changed = stream
            .next(Duration::from_secs(1))
            .expect("the change is a frame");
        assert_eq!(changed.sequence, 2);
        assert_ne!(changed.hash, first.hash);
        assert_eq!(stream.latest(), Some(changed));
    }

    #[test]
    fn frames_are_throttled_to_the_rate_asked_for() {
        let core = core(showing(screen(|_, _| 120, false)));
        let options = FrameOptions {
            max_fps: 10,
            ..polling()
        };
        let stream = core.frames(PHONE, &options).expect("subscribed");
        let started = Instant::now();
        stream
            .next(Duration::from_secs(1))
            .expect("the first frame");
        std::thread::sleep(Duration::from_millis(1000).saturating_sub(started.elapsed()));
        let captures: u64 = stream.stats().captures.iter().sum();
        assert!(
            (6..=12).contains(&captures),
            "10 a second over both connections: {captures}"
        );
    }

    #[test]
    fn a_held_frame_encodes_as_a_jpeg_of_the_size_asked_for() {
        let core = core(showing(screen(|_, _| 120, false)));
        let stream = core.frames(PHONE, &polling()).expect("subscribed");
        let frame = stream.next(Duration::from_secs(1)).expect("a frame");
        let jpeg = stream.encode(frame.sequence, 30, 53, 80).expect("encoded");
        assert_eq!(jpeg_size(&jpeg), (30, 53));
        assert!(stream.encode(frame.sequence + 9, 30, 53, 80).is_err());
    }

    #[test]
    fn the_screenshot_is_the_devices_png_at_full_size_or_scaled_to_a_long_edge() {
        let png = screen(|_, _| 120, false);
        let core = core(showing(png.clone()));
        let native = ScreenshotOptions {
            max_dimension: 0,
            quality: 90,
            format: ImageFormat::Png,
        };
        let shot = core.screenshot(PHONE, &native).expect("a screenshot");
        assert_eq!(shot.data, png, "the device's own bytes");
        assert_eq!((shot.width, shot.height), (75, 133));
        let jpeg = core
            .screenshot(
                PHONE,
                &ScreenshotOptions {
                    max_dimension: 50,
                    quality: 80,
                    format: ImageFormat::Jpeg,
                },
            )
            .expect("a JPEG");
        assert_eq!(jpeg.format, ImageFormat::Jpeg);
        assert_eq!((jpeg.width, jpeg.height), (28, 50));
        assert_eq!(jpeg_size(&jpeg.data), (28, 50));
        let small = core
            .screenshot(
                PHONE,
                &ScreenshotOptions {
                    max_dimension: 50,
                    quality: 80,
                    format: ImageFormat::Png,
                },
            )
            .expect("a PNG");
        assert_eq!(png_size(&small.data), Some((28, 50)));
        assert_eq!(core.host.device.captures()[0], 3, "through DVT");
    }

    #[test]
    fn the_display_stream_and_avfoundation_say_why_they_are_off() {
        let core = core(FakeHost::ready());
        let paths = core.frame_paths(PHONE).expect("the paths");
        let path = |which| {
            paths
                .iter()
                .find(|status| status.path == which)
                .cloned()
                .unwrap_or_else(|| panic!("{which:?} listed"))
        };
        assert!(path(FramePath::Screenshots).active);
        let stream = path(FramePath::DisplayStream);
        assert!(!stream.active);
        let reason = stream.reason.expect("a reason");
        assert!(reason.contains("media features"), "{reason}");
        let camera = path(FramePath::AvFoundation);
        assert!(!camera.active);
        assert!(camera.reason.expect("a reason").contains("camera"));
        core.host.device.media_features(9);
        let paths = core.frame_paths(PHONE).expect("the paths");
        let stream = paths
            .iter()
            .find(|status| status.path == FramePath::DisplayStream)
            .expect("listed");
        assert!(!stream.active, "a later addition either way");
        assert!(
            stream
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("later")),
            "{stream:?}"
        );
    }

    #[test]
    fn the_pace_spreads_the_rate_over_the_connections() {
        assert_eq!(pace(0, 2), None, "back to back");
        assert_eq!(pace(10, 2), Some(Duration::from_millis(200)));
        assert_eq!(pace(30, 2), Some(Duration::from_secs(2) / 30));
    }
}
