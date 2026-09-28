//! Measurements of the emulator controller's frames around a tap, not checks:
//! ignored by default and run with
//!
//! ```sh
//! MOBILE_AGENT_ANDROID_SERIAL=emulator-5554 cargo test --release -p android \
//!   --test frame_timing -- --ignored --nocapture --test-threads 1
//! ```
//!
//! on an emulator that serves its controller, with the Compose fixture
//! installed, and the Expo fixture for its taps (task 10.13,
//! `docs/research/emulator-frames.md`). The device must be otherwise quiet.
//!
//! - `measure_the_frame_stream` subscribes for 10 s windows while the Compose
//!   fixture's Anim tab spins, at several sizes, formats and transports. It
//!   prints the frames delivered, the frame events the emulator counted in
//!   `seq`, the events it coalesced while it was busy, and the inter-arrival,
//!   timestamp-to-arrival and hash times.
//! - `measure_tap_to_frame` finds `row-1` and `detail-back` on each fixture's
//!   List tab through the agent, stops the agent, then taps `row-1` with the
//!   controller's `sendTouch`, down then up, as the session does. It times the
//!   first frame whose hash, taken as the session takes it, differs from the
//!   settled one: to its `timestampUs`, the end of the span the daemon's
//!   `frameChangeMs` covers, and to its arrival. The frames between the tap and
//!   the change, and the `seq` step to it, show whether the stream was busy
//!   when the changed frame was posted. `detail-back` brings the list back.
//!
//! `MOBILE_AGENT_FRAME_TIMING_FIXTURES` (default `compose,expo`),
//! `MOBILE_AGENT_FRAME_TIMING_WIDTHS` (`540`),
//! `MOBILE_AGENT_FRAME_TIMING_TAPS` (50, after 3 warm-up taps) and
//! `MOBILE_AGENT_FRAME_TIMING_INPUTS` (`touch`; `touch,stream` alternates the
//! session's two `sendTouch` calls with one `streamInputEvent` call carrying
//! both, tap by tap) choose what `measure_tap_to_frame` runs.
//! `MOBILE_AGENT_FRAME_TIMING_OUT` names a directory for a CSV of each series,
//! in host epoch microseconds, to line up with a trace of the guest.

mod common;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write as _;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use android::adb::{AdbServer, Device};
use android::agent::{DeployOptions, Deployer, Records};
use android::emulator::proto::emulator_controller_client::EmulatorControllerClient;
use android::emulator::proto::image_format::ImgFormat;
use android::emulator::proto::image_transport::TransportChannel;
use android::emulator::proto::{
    Image, ImageFormat, ImageTransport, InputEvent, Touch, TouchEvent, input_event,
};
use android::emulator::{
    Controller, Endpoint, Frame, FrameHasher, MAX_MESSAGE, Options, TouchPoint, resolve,
    scaled_size,
};
use android::session::{Session, SessionOptions};
use common::{Leftovers, agent_jar, device_lock, percentile, serial_or_skip, wait_until_quiet};
use mirror::{Action, Element, Mirror, MirrorConfig, Point, SnapshotOptions, Target};
use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use xxhash_rust::xxh3::xxh3_64;

const COMPOSE: &str = "dev.mobileagent.fixture.compose";
const EXPO: &str = "dev.mobileagent.fixture.expo";

/// No hash change for this long counts as settled.
const QUIET: Duration = Duration::from_millis(400);

/// The longest wait for a screen to change or settle.
const WAIT: Duration = Duration::from_secs(5);

/// Each stream window.
const WINDOW: Duration = Duration::from_secs(10);

/// Unmeasured taps at the start of each tap series.
const WARMUP: usize = 3;

fn epoch_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// Milliseconds from `from` to `to`, both in epoch microseconds.
fn span_ms(from: u64, to: u64) -> f64 {
    (to as f64 - from as f64) / 1e3
}

/// The 1-minute load average.
fn load() -> String {
    std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .nth(1)
                .unwrap_or("?")
                .to_owned()
        })
        .unwrap_or_else(|_| "?".to_owned())
}

/// n, minimum, p50, p95, maximum and mean of `samples`.
fn summary(label: &str, samples: &[f64]) -> String {
    if samples.is_empty() {
        return format!("{label}: no samples");
    }
    let mut sorted = samples.to_vec();
    let p50 = percentile(&mut sorted, 50.0);
    let p95 = percentile(&mut sorted, 95.0);
    format!(
        "{label}: n={} min {:.2} p50 {p50:.2} p95 {p95:.2} max {:.2} mean {:.2}",
        sorted.len(),
        sorted[0],
        sorted[sorted.len() - 1],
        sorted.iter().sum::<f64>() / sorted.len() as f64
    )
}

fn setting(var: &str, default: &str) -> String {
    std::env::var(var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// Whether `package` is installed on `device`.
fn installed(device: &Device, package: &str) -> bool {
    device
        .shell(&format!("pm path {package}"))
        .is_ok_and(|out| out.stdout_text().starts_with("package:"))
}

/// Starts `package` afresh on `tab` and waits until it is in front and has
/// drawn its first screen.
fn launch(device: &Device, package: &str, tab: &str) {
    let out = device
        .shell(&format!(
            "am start -W -S -n {package}/.MainActivity --es initial_tab {tab}"
        ))
        .unwrap();
    assert!(
        out.stdout_text().contains("Status: ok"),
        "{package} did not start: {}{}",
        out.stdout_text(),
        out.stderr_text()
    );
    let started = Instant::now();
    while !common::foreground(device).contains(package) {
        assert!(
            started.elapsed() < WAIT,
            "{package} is not in front: {}",
            common::foreground(device)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(1500));
}

/// Stops the app when dropped.
struct Launched(Device, &'static str);

impl Drop for Launched {
    fn drop(&mut self) {
        let _ = self.0.shell(&format!("am force-stop {}", self.1));
    }
}

/// Runs `body` on a mirror over an agent on the agent's own paths, then
/// stops the agent, so that only the controller reads the screen afterwards.
fn with_agent<R>(
    device: &Device,
    leftovers: &mut Leftovers,
    body: impl FnOnce(&mut Mirror, f64) -> R,
) -> R {
    let deployer = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
    let attached = deployer
        .attach(&DeployOptions {
            idle_exit: Duration::from_secs(10),
            ..DeployOptions::default()
        })
        .unwrap_or_else(|e| panic!("{e}"));
    leftovers.track(attached.agent.pid, &attached.agent.socket);
    let session = Session::start(attached, SessionOptions::default(), None).unwrap();
    let scale = session.scale();
    let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());
    let result = body(&mut mirror, scale);
    drop(mirror);
    session.stop(Duration::from_secs(3)).unwrap();
    result
}

/// The element whose identifier is `id`, once a snapshot lists it.
fn element_of(mirror: &mut Mirror, id: &str) -> Element {
    let started = Instant::now();
    loop {
        let snapshot = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
        if let Some(element) = snapshot
            .elements
            .iter()
            .find(|e| e.identifier.as_deref() == Some(id))
        {
            return element.clone();
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no {id} in {}",
            snapshot.text
        );
        mirror
            .run_until(Instant::now() + Duration::from_millis(50))
            .unwrap();
    }
}

/// The centre, in device points, of the element whose identifier is `id`.
fn centre_of(mirror: &mut Mirror, id: &str) -> Point {
    element_of(mirror, id).bounds.center()
}

fn tap_through(mirror: &mut Mirror, at: Point) {
    let outcome = mirror
        .act(&Action::Tap {
            target: Target::Point(at),
            hold: None,
        })
        .unwrap();
    assert!(outcome.accepted, "the agent refused the tap at {at:?}");
}

fn pixels(at: Point, scale: f64) -> (i32, i32) {
    ((at.x * scale).round() as i32, (at.y * scale).round() as i32)
}

/// Adds the bearer token to every call when there is one.
#[derive(Clone)]
struct Bearer(Option<MetadataValue<Ascii>>);

impl Interceptor for Bearer {
    fn call(
        &mut self,
        mut request: tonic::Request<()>,
    ) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(value) = &self.0 {
            request
                .metadata_mut()
                .insert("authorization", value.clone());
        }
        Ok(request)
    }
}

type Client = EmulatorControllerClient<InterceptedService<Channel, Bearer>>;

/// A client of its own for the frame stream, which can ask for the formats
/// and transports the session's client does not.
async fn stream_client(endpoint: &Endpoint) -> Client {
    let channel = tonic::transport::Endpoint::from_shared(endpoint.uri.clone())
        .unwrap()
        .tcp_nodelay(true)
        .connect()
        .await
        .unwrap();
    let bearer = endpoint
        .token
        .as_ref()
        .map(|token| format!("Bearer {token}").parse().unwrap());
    EmulatorControllerClient::with_interceptor(channel, Bearer(bearer))
        .max_decoding_message_size(MAX_MESSAGE)
}

/// What a stream subscription asks for.
#[derive(Clone, Copy, Debug)]
struct Layout {
    /// 0 for the display size.
    width: u32,
    rgba: bool,
    mmap: bool,
}

impl Layout {
    fn rgb(width: u32) -> Layout {
        Layout {
            width,
            rgba: false,
            mmap: false,
        }
    }

    fn bytes_per_pixel(self) -> usize {
        if self.rgba { 4 } else { 3 }
    }

    fn name(self) -> String {
        format!(
            "{} {} over {}",
            if self.rgba { "RGBA8888" } else { "RGB888" },
            if self.width == 0 {
                "at the display size".to_owned()
            } else {
                format!("{} px wide", self.width)
            },
            if self.mmap { "MMAP" } else { "gRPC" }
        )
    }
}

/// One frame as it arrived.
#[derive(Clone, Debug)]
struct Arrival {
    at: Instant,
    arrival_us: u64,
    seq: u32,
    timestamp_us: u64,
    key: u64,
    hash: Duration,
}

/// The session's key of an RGB888 frame, or the same hash over the rows
/// below the status bar for another layout.
fn key_of(pixels: &[u8], width: u32, height: u32, bytes_per_pixel: usize) -> u64 {
    let rows = FrameHasher::default().status_bar_rows(height) as usize;
    let stride = width as usize * bytes_per_pixel;
    xxh3_64(&pixels[(rows * stride).min(pixels.len())..])
}

async fn read_frames(
    mut client: Client,
    request: ImageFormat,
    layout: Layout,
    mapped: Option<PathBuf>,
    arrivals: Sender<Arrival>,
) {
    let mut stream = client
        .stream_screenshot(request)
        .await
        .unwrap()
        .into_inner();
    let file = mapped.map(|path| File::open(path).unwrap());
    let hasher = FrameHasher::default();
    let mut buffer = Vec::new();
    while let Ok(Some(image)) = stream.message().await {
        let at = Instant::now();
        let arrival_us = epoch_us();
        let Image {
            format,
            image,
            seq,
            timestamp_us,
            ..
        } = image;
        let (width, height) = format.as_ref().map_or((0, 0), |f| (f.width, f.height));
        let len = width as usize * height as usize * layout.bytes_per_pixel();
        let hashing = Instant::now();
        let key = match &file {
            Some(file) if len > 0 => {
                buffer.resize(len, 0);
                file.read_exact_at(&mut buffer, 0).unwrap();
                key_of(&buffer, width, height, layout.bytes_per_pixel())
            }
            // As the session hashes: the message's own bytes, no copy.
            None if !layout.rgba => {
                Frame::new(width, height, image).map_or(0, |frame| hasher.key(&frame).hash)
            }
            None => key_of(&image, width, height, 4),
            Some(_) => 0,
        };
        let hash = hashing.elapsed();
        let arrival = Arrival {
            at,
            arrival_us,
            seq,
            timestamp_us,
            key,
            hash,
        };
        if arrivals.send(arrival).is_err() {
            return;
        }
    }
}

/// A subscription to the frames and the screen they show.
struct Feed {
    arrivals: Receiver<Arrival>,
    reader: tokio::task::JoinHandle<()>,
    mapped: Option<PathBuf>,
    size: (u32, u32),
    latest: Arrival,
    last_change: Instant,
}

impl Feed {
    fn open(runtime: &tokio::runtime::Runtime, endpoint: &Endpoint, layout: Layout) -> Feed {
        let controller = runtime
            .block_on(Controller::connect(endpoint, Options::default()))
            .unwrap_or_else(|e| panic!("{e}"));
        let display = controller.hardware().display;
        let requested = scaled_size(display, layout.width);
        let size = if requested == (0, 0) {
            (display.width, display.height)
        } else {
            requested
        };
        let format = if layout.rgba {
            ImgFormat::Rgba8888
        } else {
            ImgFormat::Rgb888
        };
        let mut request = ImageFormat {
            format: format as i32,
            width: requested.0,
            height: requested.1,
            ..ImageFormat::default()
        };
        let mapped = layout.mmap.then(|| {
            let path = std::env::temp_dir().join(common::unique_name("frames"));
            let file = File::create(&path).unwrap();
            file.set_len((size.0 as usize * size.1 as usize * layout.bytes_per_pixel()) as u64)
                .unwrap();
            request.transport = Some(ImageTransport {
                channel: TransportChannel::Mmap as i32,
                handle: format!("file://{}", path.display()),
            });
            path
        });
        let client = runtime.block_on(stream_client(endpoint));
        let (sender, arrivals) = mpsc::channel();
        let reader = runtime.spawn(read_frames(client, request, layout, mapped.clone(), sender));
        let latest = arrivals
            .recv_timeout(WAIT)
            .expect("the stream's first frame");
        Feed {
            arrivals,
            reader,
            mapped,
            size,
            last_change: latest.at,
            latest,
        }
    }

    fn take(&mut self, arrival: Arrival) {
        if arrival.key != self.latest.key {
            self.last_change = arrival.at;
        }
        self.latest = arrival;
    }

    fn next(&mut self, wait: Duration) -> Option<Arrival> {
        match self.arrivals.recv_timeout(wait) {
            Ok(arrival) => {
                self.take(arrival.clone());
                Some(arrival)
            }
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => panic!("the frame stream ended"),
        }
    }

    /// Waits until the screen shows a key `until` accepts and has not
    /// changed for [`QUIET`]; whether it did within [`WAIT`].
    fn settle(&mut self, until: impl Fn(u64) -> bool) -> bool {
        let started = Instant::now();
        loop {
            let now = Instant::now();
            let accepted = until(self.latest.key);
            if accepted && now >= self.last_change + QUIET {
                return true;
            }
            if now >= started + WAIT {
                return false;
            }
            let wait = if accepted {
                (self.last_change + QUIET).saturating_duration_since(now)
            } else {
                Duration::from_millis(50)
            };
            self.next(wait.max(Duration::from_millis(1)));
        }
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.reader.abort();
        if let Some(path) = &self.mapped {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// How a tap reaches the emulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Input {
    /// `sendTouch` down, then `sendTouch` up once the down is acknowledged,
    /// as the session taps.
    Touch,
    /// Down and up in one `streamInputEvent` call, without a round trip
    /// between them.
    Stream,
}

impl Input {
    fn parse(name: &str) -> Input {
        match name.trim() {
            "touch" => Input::Touch,
            "stream" => Input::Stream,
            other => panic!("no input {other}; touch or stream"),
        }
    }
}

fn touch_event(point: TouchPoint) -> InputEvent {
    InputEvent {
        r#type: Some(input_event::Type::TouchEvent(TouchEvent {
            touches: vec![Touch {
                x: point.x,
                y: point.y,
                identifier: point.id,
                pressure: point.pressure,
                ..Touch::default()
            }],
            display: 0,
        })),
    }
}

/// The connections a tap series taps through.
struct Taps {
    runtime: tokio::runtime::Runtime,
    controller: Controller,
    events: Client,
}

impl Taps {
    /// Presses and lifts contact 0 at `(x, y)` through `input`; returns the
    /// epoch microseconds once the down and once the up were acknowledged.
    fn tap(&mut self, input: Input, (x, y): (i32, i32)) -> (u64, u64) {
        match input {
            Input::Touch => {
                self.runtime
                    .block_on(self.controller.touch(&[TouchPoint::down(x, y)]))
                    .unwrap();
                let down = epoch_us();
                self.runtime
                    .block_on(self.controller.touch(&[TouchPoint::up(x, y)]))
                    .unwrap();
                (down, epoch_us())
            }
            Input::Stream => {
                let events = tonic::codegen::tokio_stream::iter(vec![
                    touch_event(TouchPoint::down(x, y)),
                    touch_event(TouchPoint::up(x, y)),
                ]);
                self.runtime
                    .block_on(self.events.stream_input_event(events))
                    .unwrap();
                let done = epoch_us();
                (done, done)
            }
        }
    }
}

/// One measured tap.
struct Tapped {
    input: Input,
    t0_us: u64,
    down_us: u64,
    up_us: u64,
    /// Since the last frame before the tap arrived.
    idle: Duration,
    /// Frames between the tap and the change, all unchanged.
    between: u32,
    before: Arrival,
    changed: Arrival,
}

/// A tap series: `taps` taps on `at` after [`WARMUP`], each through the next
/// of `inputs` in turn, with `back` bringing the list back after each and
/// frames `width` wide.
struct Series<'a> {
    fixture: &'a str,
    width: u32,
    at: (i32, i32),
    back: (i32, i32),
    taps: usize,
    inputs: &'a [Input],
    /// Where to write each tap's times, when given.
    out: Option<&'a Path>,
}

fn print_taps(label: &str, taps: &[&Tapped]) {
    let series = |f: &dyn Fn(&Tapped) -> f64| taps.iter().map(|t| f(t)).collect::<Vec<f64>>();
    eprintln!("  {label}:");
    let lines = [
        summary(
            "tap to the changed frame's timestampUs (frameChangeMs), ms",
            &series(&|t| span_ms(t.t0_us, t.changed.timestamp_us)),
        ),
        summary(
            "tap to its arrival, ms",
            &series(&|t| span_ms(t.t0_us, t.changed.arrival_us)),
        ),
        summary(
            "its timestampUs to arrival, ms",
            &series(&|t| span_ms(t.changed.timestamp_us, t.changed.arrival_us)),
        ),
        summary(
            "down acknowledged, ms",
            &series(&|t| span_ms(t.t0_us, t.down_us)),
        ),
        summary(
            "up acknowledged, ms",
            &series(&|t| span_ms(t.t0_us, t.up_us)),
        ),
        summary("quiet before the tap, ms", &series(&|t| ms(t.idle))),
        summary(
            "hash of the changed frame, us",
            &series(&|t| t.changed.hash.as_secs_f64() * 1e6),
        ),
    ];
    for line in lines {
        eprintln!("    {line}");
    }
    let mut between: BTreeMap<u32, u32> = BTreeMap::new();
    let mut steps: BTreeMap<u32, u32> = BTreeMap::new();
    for t in taps {
        *between.entry(t.between).or_default() += 1;
        *steps
            .entry(t.changed.seq.saturating_sub(t.before.seq))
            .or_default() += 1;
    }
    eprintln!("    unchanged frames between the tap and the change: {between:?}");
    eprintln!("    seq step from the frame before to the changed one: {steps:?}");
}

/// Runs `series` on the emulator `serial`, prints the timings per input and
/// writes them when `series.out` names a directory.
fn tap_series(serial: &str, series: &Series) {
    let fixture = series.fixture;
    let runtime = runtime();
    let endpoint = resolve(serial).unwrap_or_else(|e| panic!("{e}"));
    let controller = runtime
        .block_on(Controller::connect(&endpoint, Options::default()))
        .unwrap_or_else(|e| panic!("{e}"));
    let events = runtime.block_on(stream_client(&endpoint));
    let mut feed = Feed::open(&runtime, &endpoint, Layout::rgb(series.width));
    let mut taps = Taps {
        runtime,
        controller,
        events,
    };
    let load_before = load();
    let mut measured: Vec<Tapped> = Vec::new();
    let mut misses = 0;
    for index in 0..WARMUP + series.taps {
        let input = series.inputs[index % series.inputs.len()];
        assert!(feed.settle(|_| true), "{fixture}: the list did not settle");
        while let Ok(arrival) = feed.arrivals.try_recv() {
            feed.take(arrival);
        }
        let before_tap = feed.latest.clone();
        let t0 = Instant::now();
        let t0_us = epoch_us();
        let (down_us, up_us) = taps.tap(input, series.at);
        let mut before = before_tap.clone();
        let mut between = 0;
        let mut changed = None;
        while changed.is_none() && t0.elapsed() < WAIT {
            if let Some(arrival) = feed.next(WAIT.saturating_sub(t0.elapsed())) {
                if arrival.key == before_tap.key {
                    between += 1;
                    before = arrival;
                } else {
                    changed = Some(arrival);
                }
            }
        }
        let Some(changed) = changed else {
            misses += 1;
            assert!(misses < 3, "{fixture}: three taps changed nothing");
            continue;
        };
        let opened = feed.settle(|key| key != before_tap.key);
        taps.tap(Input::Touch, series.back);
        feed.last_change = Instant::now();
        let restored = feed.settle(|key| key == before_tap.key);
        assert!(
            opened && restored,
            "{fixture}: tap {index} left the detail open or the list changed"
        );
        if index >= WARMUP {
            measured.push(Tapped {
                input,
                t0_us,
                down_us,
                up_us,
                idle: t0.saturating_duration_since(before_tap.at),
                between,
                before,
                changed,
            });
        }
    }
    let (w, h) = feed.size;
    eprintln!(
        "{fixture}, frames {w}x{h}: {} taps after {WARMUP} warm-up, load {load_before} -> {}",
        measured.len(),
        load()
    );
    let mut inputs = series.inputs.to_vec();
    inputs.dedup();
    for input in inputs {
        let taps: Vec<&Tapped> = measured.iter().filter(|t| t.input == input).collect();
        print_taps(&format!("{input:?}"), &taps);
    }
    if let Some(dir) = series.out {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("taps-{fixture}-{w}.csv"));
        let mut csv = File::create(&path).unwrap();
        writeln!(
            csv,
            "input,t0_us,down_us,up_us,idle_ms,between,before_seq,before_ts_us,\
             before_arrival_us,change_seq,change_ts_us,change_arrival_us"
        )
        .unwrap();
        for t in &measured {
            writeln!(
                csv,
                "{:?},{},{},{},{:.3},{},{},{},{},{},{},{}",
                t.input,
                t.t0_us,
                t.down_us,
                t.up_us,
                ms(t.idle),
                t.between,
                t.before.seq,
                t.before.timestamp_us,
                t.before.arrival_us,
                t.changed.seq,
                t.changed.timestamp_us,
                t.changed.arrival_us
            )
            .unwrap();
        }
        eprintln!("  wrote {}", path.display());
    }
}

/// One stream window: what arrived after the subscription's first frame.
fn stream_window(runtime: &tokio::runtime::Runtime, endpoint: &Endpoint, layout: Layout) {
    let load_before = load();
    let mut feed = Feed::open(runtime, endpoint, layout);
    let started = Instant::now();
    let mut arrivals = Vec::new();
    while let Some(left) = WINDOW.checked_sub(started.elapsed()) {
        if let Some(arrival) = feed.next(left) {
            arrivals.push(arrival);
        }
    }
    let (w, h) = feed.size;
    drop(feed);
    if arrivals.len() < 2 {
        eprintln!(
            "{}: {} frames in {WINDOW:?}; is the spinner running?",
            layout.name(),
            arrivals.len()
        );
        return;
    }
    let first = &arrivals[0];
    let last = &arrivals[arrivals.len() - 1];
    let seconds = (last.at - first.at).as_secs_f64();
    let events = last.seq.saturating_sub(first.seq);
    let pairs: Vec<(&Arrival, &Arrival)> = arrivals.iter().zip(arrivals.iter().skip(1)).collect();
    let coalesced: u32 = pairs
        .iter()
        .map(|(a, b)| b.seq.saturating_sub(a.seq).saturating_sub(1))
        .sum();
    let changed = pairs.iter().filter(|(a, b)| a.key != b.key).count();
    let gaps: Vec<f64> = pairs.iter().map(|(a, b)| ms(b.at - a.at)).collect();
    let lags: Vec<f64> = arrivals
        .iter()
        .map(|a| span_ms(a.timestamp_us, a.arrival_us))
        .collect();
    let hashes: Vec<f64> = arrivals
        .iter()
        .map(|a| a.hash.as_secs_f64() * 1e6)
        .collect();
    eprintln!(
        "{} ({w}x{h}): {:.1} frames/s delivered, {:.1} frame events/s, {coalesced} coalesced, \
         {changed} of {} changed; load {load_before} -> {}",
        layout.name(),
        (arrivals.len() - 1) as f64 / seconds,
        f64::from(events) / seconds,
        arrivals.len() - 1,
        load()
    );
    eprintln!("  {}", summary("inter-arrival, ms", &gaps));
    eprintln!("  {}", summary("timestampUs to arrival, ms", &lags));
    eprintln!("  {}", summary("hash, us", &hashes));
}

#[test]
#[ignore = "a measurement; run with --ignored on a quiet emulator"]
fn measure_the_frame_stream() {
    let Some(serial) = serial_or_skip("measure_the_frame_stream") else {
        return;
    };
    let _device = device_lock();
    let device = AdbServer::from_env().device(&serial);
    wait_until_quiet(&device);
    let mut leftovers = Leftovers::new(&device);
    launch(&device, COMPOSE, "anim");
    let _launched = Launched(device.clone(), COMPOSE);
    // The agent turns the spinner on unless it already turns, and goes, so
    // only the stream reads the screen while it turns.
    with_agent(&device, &mut leftovers, |mirror, _| {
        let toggle = element_of(mirror, "anim-toggle");
        if toggle.checked != Some(true) {
            tap_through(mirror, toggle.bounds.center());
        }
    });
    let runtime = runtime();
    let endpoint = resolve(&serial).unwrap_or_else(|e| panic!("{e}"));
    let layouts = [
        Layout::rgb(270),
        Layout::rgb(540),
        Layout::rgb(0),
        Layout {
            rgba: true,
            ..Layout::rgb(540)
        },
        Layout {
            mmap: true,
            ..Layout::rgb(540)
        },
    ];
    // Two rounds, the second in reverse, so drift reaches every layout.
    for layout in layouts.iter().chain(layouts.iter().rev()) {
        stream_window(&runtime, &endpoint, *layout);
    }
}

#[test]
#[ignore = "a measurement; run with --ignored on a quiet emulator"]
fn measure_tap_to_frame() {
    let Some(serial) = serial_or_skip("measure_tap_to_frame") else {
        return;
    };
    let _device = device_lock();
    let device = AdbServer::from_env().device(&serial);
    let fixtures = setting("MOBILE_AGENT_FRAME_TIMING_FIXTURES", "compose,expo");
    let widths: Vec<u32> = setting("MOBILE_AGENT_FRAME_TIMING_WIDTHS", "540")
        .split(',')
        .map(|w| w.trim().parse().expect("widths in whole pixels"))
        .collect();
    let taps: usize = setting("MOBILE_AGENT_FRAME_TIMING_TAPS", "50")
        .parse()
        .expect("a number of taps");
    let inputs: Vec<Input> = setting("MOBILE_AGENT_FRAME_TIMING_INPUTS", "touch")
        .split(',')
        .map(Input::parse)
        .collect();
    let out = std::env::var_os("MOBILE_AGENT_FRAME_TIMING_OUT").map(PathBuf::from);
    for fixture in fixtures.split(',').map(str::trim) {
        let package = match fixture {
            "compose" => COMPOSE,
            "expo" => EXPO,
            other => panic!("no fixture {other}; compose or expo"),
        };
        if !installed(&device, package) {
            eprintln!("skipped {fixture}: {package} is not installed");
            continue;
        }
        wait_until_quiet(&device);
        let mut leftovers = Leftovers::new(&device);
        launch(&device, package, "list");
        let _launched = Launched(device.clone(), package);
        let (at, back) = with_agent(&device, &mut leftovers, |mirror, scale| {
            let row = centre_of(mirror, "row-1");
            tap_through(mirror, row);
            let back = centre_of(mirror, "detail-back");
            tap_through(mirror, back);
            centre_of(mirror, "row-1");
            (pixels(row, scale), pixels(back, scale))
        });
        wait_until_quiet(&device);
        eprintln!("{fixture}: row-1 at {at:?}, detail-back at {back:?} (pixels)");
        for &width in &widths {
            let series = Series {
                fixture,
                width,
                at,
                back,
                taps,
                inputs: &inputs,
                out: out.as_deref(),
            };
            tap_series(&serial, &series);
        }
    }
}
