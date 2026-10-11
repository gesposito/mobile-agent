//! Frames and screenshots from the host (design D23): DVT's
//! `takeScreenshot` and CoreDevice's `capturescreenshot` polled side by
//! side, each on a connection of its own, their PNGs turned into frames by
//! the pipeline (`crate::frames`) on a thread of its own; and the on-demand
//! screenshot through DVT, at the native size or scaled and encoded.
//!
//! A poller whose connection stops answering is reopened and, if that does
//! not help, left out (design D26 of physical-ios): two captures in a row
//! that fail, or one whose connection closed, make it open its connection
//! and channel again, once; failing the same way on the new connection, it
//! leaves the frames to the other poller, and the stats say why. The
//! on-demand screenshot takes CoreDevice's capture when DVT's fails, and
//! keeps to the connection that served. A healthy connection pays nothing
//! for either.
//!
//! The display stream and AVFoundation are later additions: the display
//! service of iOS 26.6.2 lists no media feature and refuses the stream, and
//! AVFoundation needs the macOS camera permission (S11). `frame_paths`
//! says which path serves and why the others do not.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use idevice::ReadWrite;
use idevice::core_device::{
    DisplayServiceClient, ImageFormat as CaptureFormat, ScreenCaptureServiceClient,
};
use plist::Value;
use tokio::sync::{Notify, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::driver::{Slot, Target, dvt_channel};
use super::dvt::{self, CallError, DvtChannel};
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

/// How many captures in a row may fail on a poller's connection before the
/// poller opens it again, once, or, on the connection it opened again, is
/// left out (design D26 of physical-ios): the second, so that one capture
/// slower than the wait costs no connection, where DVT's `takeScreenshot`
/// answered none of 106 on a degraded phone in task 9.6. A connection that
/// closed is opened again at once.
pub(crate) const MISSES: u32 = 2;

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

/// The capture connection beside `connection`.
fn other(connection: Capture) -> Capture {
    match connection {
        Capture::Dvt => Capture::CoreDevice,
        Capture::CoreDevice => Capture::Dvt,
    }
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
    /// The poller opened its connection again.
    Reopened {
        source: Capture,
    },
    /// The poller left the frames to the other one; why.
    LeftOut {
        source: Capture,
        reason: String,
    },
}

/// Why a capture brought no picture.
struct Missed {
    reason: String,
    /// Whether its connection takes no more captures: it closed.
    closed: bool,
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

    /// One capture, waiting `wait` for it. A reply that does not come in
    /// time leaves the connection as it is: DVT's channel waits for it and
    /// sends the next capture once it came, and CoreDevice's next request
    /// takes it for its own, an earlier picture, so two captures in a row
    /// that time out tell a connection that stopped answering.
    async fn capture(&mut self, wait: Duration) -> std::result::Result<Vec<u8>, Missed> {
        match self {
            Poller::Dvt(channel) => match channel.call_patiently(take_screenshot(), wait).await {
                Ok(reply) => screenshot_png(&reply).map_err(|reason| Missed {
                    reason: format!("DVT's takeScreenshot: {reason}"),
                    closed: false,
                }),
                Err(CallError::TimedOut(after)) => Err(Missed {
                    reason: format!(
                        "DVT's reply to takeScreenshot did not happen within {after:?}"
                    ),
                    closed: false,
                }),
                Err(CallError::NotSent(cause) | CallError::Closed(cause)) => Err(Missed {
                    reason: format!("DVT's takeScreenshot: {cause}"),
                    closed: true,
                }),
            },
            Poller::CoreDevice(client) => {
                match timeout(wait, client.take_screenshot(None, CaptureFormat::Png)).await {
                    Ok(Ok(png)) => Ok(png),
                    Ok(Err(error)) => Err(Missed {
                        reason: format!("CoreDevice's capturescreenshot: {}", describe(&error)),
                        closed: true,
                    }),
                    Err(_) => Err(Missed {
                        reason: format!(
                            "CoreDevice's capturescreenshot did not answer within {wait:?}"
                        ),
                        closed: false,
                    }),
                }
            }
        }
    }
}

/// Opens a poller's connection and channel again: the poller's connection
/// in, the new poller out once it opened, or why it could not.
pub(crate) type Reopen =
    Arc<dyn Fn(Capture) -> oneshot::Receiver<std::result::Result<Poller, String>> + Send + Sync>;

/// The reopener of the pollers of `slot`'s target, through its tunnel. Each
/// reopen runs on a thread of its own, driven by `runtime`, as the runner's
/// supervisor does, since rustc cannot show the generic connection's future
/// `Send` for every lifetime it borrows; it holds the target's lock only to
/// connect.
pub(crate) fn reopener<H: Host>(
    slot: Weak<Slot<H>>,
    runtime: tokio::runtime::Handle,
    limits: Limits,
) -> Reopen {
    Arc::new(move |connection| {
        let (opened, receiver) = oneshot::channel();
        let (slot, runtime) = (slot.clone(), runtime.clone());
        // A thread that could not start drops `opened`, which fails the
        // reopen.
        let _ = std::thread::Builder::new()
            .name("ios-capture-reopen".into())
            .spawn(move || {
                let poller = match slot.upgrade() {
                    Some(slot) => runtime
                        .block_on(reopen(&slot, connection, limits))
                        .map_err(|error| error.to_string()),
                    None => Err("the driver no longer holds the target".to_owned()),
                };
                let _ = opened.send(poller);
            });
        receiver
    })
}

/// A new capture client on `connection` through the tunnel of `slot`'s
/// target.
async fn reopen<H: Host>(slot: &Slot<H>, connection: Capture, limits: Limits) -> Result<Poller> {
    match connection {
        Capture::Dvt => {
            let client = slot.state.lock().await.tunnel.connect().await?;
            Ok(Poller::Dvt(dvt_channel(client, SCREENSHOT, limits).await?))
        }
        Capture::CoreDevice => {
            let client = slot
                .state
                .lock()
                .await
                .tunnel
                .connect::<ScreenCapture>()
                .await?;
            Ok(Poller::CoreDevice(Box::new(client)))
        }
    }
}

/// When a poller requests its captures: `None` back to back, otherwise one
/// every `every`, the connections staggered so that together they take at
/// most `max_fps` a second.
pub(crate) fn pace(max_fps: u32, pollers: u32) -> Option<Duration> {
    (max_fps > 0).then(|| Duration::from_secs(u64::from(pollers)) / max_fps)
}

/// How a poller requests its captures and when it gives up on one.
#[derive(Clone, Copy)]
struct Schedule {
    every: Option<Duration>,
    offset: Duration,
    wait: Duration,
}

/// Polls `poller` until the subscription ends, opening its connection again
/// through `reopen` once [`MISSES`] captures in a row failed or one found
/// it closed, and leaving the frames to the other poller when the same
/// befalls the connection it opened again before any capture served.
/// `close` closes the connection, as the stream's test hook asks.
async fn poll(
    mut poller: Poller,
    schedule: Schedule,
    taken: Sender<Taken>,
    mut stop: watch::Receiver<bool>,
    reopen: Reopen,
    close: Arc<Notify>,
) {
    let source = poller.source();
    let mut next = tokio::time::Instant::now() + schedule.offset;
    // Captures in a row that failed on this connection.
    let mut misses = 0;
    // Whether the connection was opened again since a capture last served.
    let mut reopened = false;
    loop {
        if let Some(every) = schedule.every {
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
            captured = poller.capture(schedule.wait) => captured,
            () = close.notified() => Err(Missed {
                reason: "the stream's test hook closed the connection".to_owned(),
                closed: true,
            }),
            _ = stop.changed() => return,
        };
        let missed = match captured {
            Ok(png) => {
                (misses, reopened) = (0, false);
                let arrived = Instant::now();
                let message = Taken::Png {
                    png,
                    source,
                    requested,
                    arrived,
                };
                if taken.send(message).is_err() {
                    return;
                }
                continue;
            }
            Err(missed) => missed,
        };
        misses += 1;
        let reason = missed.reason.clone();
        if taken.send(Taken::Failed { source, reason }).is_err() {
            return;
        }
        if !missed.closed && misses < MISSES {
            continue;
        }
        if reopened {
            let reason = if missed.closed {
                format!(
                    "its connection closed again after it was opened again: {}",
                    missed.reason
                )
            } else {
                format!(
                    "{misses} captures in a row failed again on the connection it opened \
                     again, the last: {}",
                    missed.reason
                )
            };
            let _ = taken.send(Taken::LeftOut { source, reason });
            return;
        }
        let opened = tokio::select! {
            opened = reopen(source) => opened,
            _ = stop.changed() => return,
        };
        match opened {
            Ok(Ok(new)) => {
                poller = new;
                (misses, reopened) = (0, true);
                if taken.send(Taken::Reopened { source }).is_err() {
                    return;
                }
            }
            failed => {
                let why = failed.unwrap_or_else(|_| Err("the reopen did not start".to_owned()));
                let reason = format!(
                    "its connection could not be opened again ({}), after {}",
                    why.err().unwrap_or_default(),
                    missed.reason
                );
                let _ = taken.send(Taken::LeftOut { source, reason });
                return;
            }
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
            Taken::Reopened { source } => shared.pipeline().reopened(source),
            Taken::LeftOut { source, reason } => shared.pipeline().left_out(source, &reason),
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
    /// What closes each poller's connection, DVT's first.
    closes: [Arc<Notify>; 2],
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

    fn close_connection(&self, connection: Capture) {
        self.closes[connection.index()].notify_one();
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
        reopen: Reopen,
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
        let closes = [Arc::new(Notify::new()), Arc::new(Notify::new())];
        let every = pace(options.max_fps, pollers.len() as u32);
        let count = pollers.len() as u32;
        let handles = pollers
            .into_iter()
            .enumerate()
            .map(|(index, poller)| {
                let schedule = Schedule {
                    every,
                    offset: every.map_or(Duration::ZERO, |every| every * index as u32 / count),
                    wait: limits.reply,
                };
                let close = Arc::clone(&closes[poller.source().index()]);
                runtime.spawn(poll(
                    poller,
                    schedule,
                    taken.clone(),
                    stopping.clone(),
                    Arc::clone(&reopen),
                    close,
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
            closes,
        })))
    }

    /// The screen now: the device's PNG at the native size, or scaled to
    /// `options.max_dimension` and encoded as `options.format`, through
    /// DVT's screenshot channel, or CoreDevice's capture once DVT's failed
    /// where CoreDevice's served.
    pub(crate) async fn screenshot(
        &mut self,
        options: &ScreenshotOptions,
        limits: Limits,
    ) -> Result<Screenshot> {
        let started = Instant::now();
        let (png, connection) = self.screenshot_png(limits).await?;
        let unreadable = |reason: String| Error::Device {
            call: match connection {
                Capture::Dvt => "DVT's takeScreenshot",
                Capture::CoreDevice => "CoreDevice's capturescreenshot",
            }
            .into(),
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

    /// The screen as the device's PNG, and the connection it came on: the
    /// one the last screenshot came on, DVT's at first, else the other,
    /// which the next screenshot then takes first. When both fail, the
    /// first one's failure is the screenshot's.
    async fn screenshot_png(&mut self, limits: Limits) -> Result<(Vec<u8>, Capture)> {
        let first = self.screenshots;
        let failure = match self.take_png(first, limits).await {
            Ok(png) => return Ok((png, first)),
            Err(failure) => failure,
        };
        let second = other(first);
        match self.take_png(second, limits).await {
            Ok(png) => {
                self.screenshots = second;
                Ok((png, second))
            }
            Err(_) => Err(failure),
        }
    }

    /// The screen as the device's PNG through `connection`: DVT's kept
    /// screenshot channel, or CoreDevice's screen capture, kept once used.
    async fn take_png(&mut self, connection: Capture, limits: Limits) -> Result<Vec<u8>> {
        if connection == Capture::Dvt {
            let reply = self
                .call(SCREENSHOT, take_screenshot(), true, limits)
                .await?;
            return screenshot_png(&reply).map_err(|reason| Error::Device {
                call: "DVT's takeScreenshot".into(),
                reason,
            });
        }
        if self.screen_capture.is_none() {
            let client = self.tunnel.connect::<ScreenCapture>().await?;
            self.screen_capture = Some(Box::new(client));
        }
        let client = self
            .screen_capture
            .as_mut()
            .expect("CoreDevice's screen capture was just connected");
        let taken = timeout(
            limits.reply,
            client.take_screenshot(None, CaptureFormat::Png),
        )
        .await;
        let failure = match taken {
            Ok(Ok(png)) => return Ok(png),
            Ok(Err(error)) => Error::Device {
                call: "CoreDevice's capturescreenshot".into(),
                reason: describe(&error),
            },
            Err(_) => Error::Timeout {
                what: "CoreDevice's reply to capturescreenshot".into(),
                after: limits.reply,
            },
        };
        // A connection that failed a capture is not kept for the next.
        self.screen_capture = None;
        Err(failure)
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
    use crate::macos::driver::tests::{FakeHost, PHONE, SHORT, core};

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

    /// The stream's stats once `done` holds, within `wait`.
    fn stats_once(
        stream: &crate::frames::FrameStream,
        wait: Duration,
        done: impl Fn(&FrameStats) -> bool,
    ) -> FrameStats {
        let deadline = Instant::now() + wait;
        loop {
            let stats = stream.stats();
            if done(&stats) || Instant::now() >= deadline {
                return stats;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Physical-ios, task 9.8 (design D26): a poller whose connection stops
    /// answering, the other connection's captures going on, reopens its
    /// connection and channel once two captures in a row timed out, and
    /// polls on through the new one.
    #[test]
    fn a_poller_whose_captures_time_out_twice_in_a_row_reopens_its_connection_and_polls_on() {
        let core = core(showing(screen(|_, _| 120, false)));
        let stream = core.frames(PHONE, &polling()).expect("subscribed");
        stream
            .next(Duration::from_secs(1))
            .expect("the first frame");
        core.host.device.stall_capture_connections(Capture::Dvt);
        let core_device = core.host.device.captures()[1];
        let stats = stats_once(&stream, Duration::from_secs(4), |stats| {
            stats.reopens[0] == 1
        });
        assert_eq!(stats.reopens, [1, 0], "{stats:?}");
        assert!(stats.failures >= 2, "two timed out first: {stats:?}");
        let dvt = core.host.device.captures()[0];
        std::thread::sleep(Duration::from_millis(200));
        let served = core.host.device.captures();
        assert!(served[0] > dvt, "DVT's new connection captures: {served:?}");
        assert!(served[1] > core_device, "CoreDevice's went on: {served:?}");
        assert_eq!(stream.stats().left_out, [None, None]);
    }

    /// Physical-ios, task 9.8 (design D26): a poller whose captures still
    /// time out after its connection was reopened, as DVT's `takeScreenshot`
    /// answered none on any connection in task 9.6
    /// (`probes/dvt-screenshot-fixed.txt`), is left out of the frames, the
    /// stats saying why, and the other connection's poller goes on: a
    /// change on screen is still a frame.
    #[test]
    fn a_poller_still_failing_after_its_reopen_is_left_out_and_the_other_polls_on() {
        let core = core(showing(screen(|_, _| 120, false)));
        let stream = core.frames(PHONE, &polling()).expect("subscribed");
        stream
            .next(Duration::from_secs(1))
            .expect("the first frame");
        core.host.device.stall_captures(Capture::Dvt);
        let stats = stats_once(&stream, Duration::from_secs(6), |stats| {
            stats.left_out[0].is_some()
        });
        let why = stats.left_out[0].clone().expect("DVT's poller left out");
        assert!(why.contains("takeScreenshot"), "{why}");
        assert_eq!(stats.reopens, [1, 0], "{stats:?}");
        assert_eq!(stats.left_out[1], None, "{stats:?}");
        let core_device = core.host.device.captures()[1];
        core.host.device.show(screen(
            |x, y| if (x, y) == (40, 90) { 200 } else { 120 },
            false,
        ));
        let changed = stream
            .next(Duration::from_secs(1))
            .expect("the change is a frame");
        assert_eq!(changed.source, Capture::CoreDevice);
        assert!(core.host.device.captures()[1] > core_device);
    }

    /// Physical-ios, task 9.8 (design D26): a poller whose connection
    /// closes, as a dropped connection does, opens it again at once and
    /// captures at its next turn, so the frames lose one capture interval:
    /// the gap between its captures across the drop is two intervals.
    #[test]
    fn a_poller_whose_connection_closes_opens_it_again_within_a_capture_interval() {
        let core = core(showing(screen(|_, _| 120, false)));
        let options = FrameOptions {
            max_fps: 10,
            ..polling()
        };
        let every = pace(options.max_fps, 2).expect("paced");
        let stream = core.frames(PHONE, &options).expect("subscribed");
        stream
            .next(Duration::from_secs(1))
            .expect("the first frame");
        std::thread::sleep(every);
        core.host.device.drop_capture_connection(Capture::Dvt);
        let stats = stats_once(&stream, Duration::from_secs(2), |stats| {
            stats.reopens[0] == 1
        });
        assert_eq!(stats.reopens, [1, 0], "{stats:?}");
        let reopened = core.host.device.served(Capture::Dvt).len();
        let deadline = Instant::now() + every * 3;
        while core.host.device.served(Capture::Dvt).len() == reopened && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let served = core.host.device.served(Capture::Dvt);
        assert!(served.len() > reopened, "DVT captures again");
        let gap = served
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .max()
            .expect("captures");
        assert!(
            gap <= every * 2 + Duration::from_millis(150),
            "{gap:?} between two captures, against {every:?} a turn"
        );
    }

    /// Physical-ios, task 9.8: the stream's test hook closes a poller's
    /// connection, which the poller opens again, as it does a dropped one,
    /// so the device test can see the reopen on the phone.
    #[test]
    fn the_test_hook_closes_a_pollers_connection_and_the_poller_opens_it_again() {
        let core = core(showing(screen(|_, _| 120, false)));
        let stream = core.frames(PHONE, &polling()).expect("subscribed");
        stream
            .next(Duration::from_secs(1))
            .expect("the first frame");
        stream.close_connection(Capture::Dvt);
        let stats = stats_once(&stream, Duration::from_secs(2), |stats| {
            stats.reopens[0] == 1
        });
        assert_eq!(stats.reopens, [1, 0], "{stats:?}");
        let dvt = core.host.device.captures()[0];
        std::thread::sleep(Duration::from_millis(100));
        assert!(core.host.device.captures()[0] > dvt, "DVT captures again");
    }

    /// Physical-ios, task 9.8: a tunnel the driver closed itself, here
    /// containing a phone whose runner fell silent with a call in flight,
    /// is not opened again by the pollers whose connections it closed: they
    /// are left out, and the next command opens the tunnel.
    #[test]
    fn the_pollers_open_no_tunnel_the_driver_closed() {
        let host = showing(screen(|_, _| 120, false));
        host.device.install_runner("0.1.0");
        let core = core(host);
        let options = crate::model::AttachOptions {
            runner: crate::model::RunnerSpec::new("0.1.0"),
            target_app: None,
            timeout: Duration::from_secs(5),
            input: crate::model::InputChoice::Auto,
        };
        core.attach(PHONE, &options).expect("attaches");
        let stream = core.frames(PHONE, &polling()).expect("subscribed");
        stream
            .next(Duration::from_secs(1))
            .expect("the first frame");
        let opens = core.host.device.opens();
        core.host.device.silence_runner();
        let lost = core
            .call(
                PHONE,
                protocol::message::Call::Snapshot(protocol::message::SnapshotParams::default()),
                Duration::from_secs(10),
            )
            .expect_err("the runner fell silent");
        assert_eq!(lost.code(), "disconnected", "{lost}");
        let stats = stats_once(&stream, Duration::from_secs(2), |stats| {
            stats.left_out.iter().all(Option::is_some)
        });
        assert!(
            stats.left_out.iter().all(Option::is_some),
            "both left out: {stats:?}"
        );
        assert_eq!(stats.reopens, [0, 0], "{stats:?}");
        assert_eq!(core.host.device.opens(), opens, "no tunnel opened again");
    }

    /// Physical-ios, task 9.8 (design D26): the on-demand screenshot takes
    /// CoreDevice's capture when DVT's screenshot does not answer, as it
    /// answered none in task 9.6, and keeps to it, so the next one waits for
    /// no timeout; the connection it keeps for that is listed open.
    #[test]
    fn the_screenshot_takes_the_other_connection_when_its_own_fails_and_keeps_to_it() {
        let png = screen(|_, _| 120, false);
        let core = core(showing(png.clone()));
        core.host.device.stall_captures(Capture::Dvt);
        let native = ScreenshotOptions {
            max_dimension: 0,
            quality: 90,
            format: ImageFormat::Png,
        };
        let started = Instant::now();
        let shot = core
            .screenshot(PHONE, &native)
            .expect("a screenshot through CoreDevice");
        assert_eq!(shot.data, png, "the device's own bytes");
        assert!(
            started.elapsed() >= SHORT.reply,
            "DVT was asked first: {:?}",
            started.elapsed()
        );
        assert_eq!(core.host.device.captures(), [0, 1]);
        let started = Instant::now();
        core.screenshot(PHONE, &native)
            .expect("a screenshot through CoreDevice again");
        assert!(
            started.elapsed() < SHORT.reply,
            "straight to CoreDevice: {:?}",
            started.elapsed()
        );
        assert_eq!(core.host.device.captures(), [0, 2]);
        assert!(
            core.status(PHONE)
                .expect("status")
                .services_open
                .contains(&crate::services::SCREEN_CAPTURE),
            "the screenshot's CoreDevice connection is kept"
        );
    }

    #[test]
    fn the_pace_spreads_the_rate_over_the_connections() {
        assert_eq!(pace(0, 2), None, "back to back");
        assert_eq!(pace(10, 2), Some(Duration::from_millis(200)));
        assert_eq!(pace(30, 2), Some(Duration::from_secs(2) / 30));
    }
}
