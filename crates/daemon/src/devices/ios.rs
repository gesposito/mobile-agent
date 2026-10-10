//! The physical iOS driver (`crates/ios`, design D18 to D25) behind the
//! daemon's driver contract and the mirror's backend traits, as
//! `crates/ios/README.md`'s Driver contract maps them. The driver registers
//! only when `MOBILE_AGENT_DRIVERS` names `ios` ([`super::OPT_IN`]).
//!
//! - Discovery lists every iPhone attached over USB, ready or in the first
//!   readiness state it has not passed (`device_locked`,
//!   `developer_mode_off`...), the state's message from `crates/ios` its
//!   advice. The driver opens on first use and keeps the host's signing
//!   identity once found, until `devices` asks it to forget it.
//! - Attaching connects the device (readiness, the developer disk image,
//!   the CoreDevice tunnel), starts the resident runner `agent-ios` builds,
//!   of this workspace's `agent-ios/VERSION`, installing the bundle
//!   `agent-ios/build.sh` wrote, or the one `MOBILE_AGENT_IOS_RUNNER_APP`
//!   names, when the phone's runner is missing or of another version, and
//!   subscribes to the screen.
//! - The tree source reads the foreground through the runner's snapshot,
//!   in pixels, put into points by the screen's scale from lockdown. Every
//!   window sends no change events (design D21), so the mirror re-reads after
//!   input, when frames change and on its poll. A read the runner refuses
//!   while the foreground changes (`window_changed`) is retried for a few
//!   seconds.
//! - The event stream carries frames: screenshots polled on DVT and
//!   CoreDevice, a frame each time a pixel below the status bar changes
//!   (design D23); encoded frames come from the ones the stream holds.
//! - Input goes through the disk image's `dtuhidd` from the host where the
//!   device's iOS takes its reports, else through the runner's synthesis
//!   (`MOBILE_AGENT_IOS_INPUT` picks one way alone); iOS key codes are
//!   keyboard usages or consumer-page buttons, as on the Simulator. Text goes
//!   through the runner into the field with keyboard focus, tapping the
//!   field a ref names first: typed, or set where typing fails, and set when
//!   the method is left to the driver and the field holds text, so the text
//!   replaces it.
//! - Apps launch through DVT's process control with their launch
//!   environment; a relaunch ends the running instance first.
//! - Screenshots come from DVT on demand; `status` names the input path,
//!   the frames and their capture rate, the runner and the tunnel.
//! - Dropping the attachment releases the runner, which ends its process,
//!   and closes what the driver opened for the device.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use ios::{
    AttachOptions as SessionOptions, CallSite, DeviceInfo, Devices as _, Dispatch, FrameOptions,
    FrameStream, Frames as _, ImageFormat, Input as _, InputChoice, InputPath, IosDriver,
    LaunchOptions, Lifecycle as _, RunnerSpec, RunnerState, ScreenshotOptions, Session as _,
    TargetStatus,
};
use mirror::backend::{
    Backend, BackendError, BackendInfo, EncodeRequest, EventStream, FrameMeta, FrameSource, Input,
    InputAck, InputSink, NodeHandle, ReadRequest, Release, StreamItem, SystemClock, Timed,
    TreeRead, TreeSource,
};
use mirror::{MirrorConfig, Point, SlowFrames};
use protocol::message::{Call, Reply, Rotation, SnapshotParams, TextMethod, TextParams};
use protocol::snapshot::{Node, NodeFlags, Snapshot};
use serde_json::{Map, Value, json};

use super::simulator::{has_focused_field, node_center};
use crate::driver::{
    AppControl, AttachOptions, Attachment, Capture, CaptureRequest, CompatibilityRow, Driver,
    DriverError, FixedScreen, Image, Launch, Platform, Report, Size, TargetClass, TargetInfo,
    TargetReport,
};
use crate::handlers::millis;
use crate::wire::code::{APP_NOT_FOUND, INVALID_ARGUMENT, UNKNOWN_KEY};

/// The driver's name, as `status`, `doctor` and `--drivers` give it.
pub(crate) const NAME: &str = "ios";

/// The variable that names the runner bundle an attach installs, a relative
/// path taken from the workspace's root, as the device tests of
/// `crates/ios` take it.
pub(crate) const RUNNER_APP_VAR: &str = "MOBILE_AGENT_IOS_RUNNER_APP";

/// The variable that picks the way touches, buttons and keys go: `auto`,
/// the default, by the device's iOS; `dtuhidd` or `runner` alone.
pub(crate) const INPUT_VAR: &str = "MOBILE_AGENT_IOS_INPUT";

/// Where `agent-ios/build.sh` writes the runner bundle, from the workspace's
/// root.
pub(crate) const RUNNER_BUNDLE: &str = "agent-ios/dist/MobileAgentRunner-Runner.app";

/// The screenshots a second a physical iPhone serves over the two
/// connections the frames poll: 19.0 and 19.2 on the iPhone SE (3rd
/// generation) with iOS 26.6.2 in task 3.5, 19.4 at most from four pollers
/// in spike S11 (design D23).
pub(crate) const CAPTURES_A_SECOND: u32 = 19;

/// The time between two of those captures: 52.6 ms.
pub(crate) const CAPTURE_INTERVAL: Duration =
    Duration::from_nanos(1_000_000_000 / CAPTURES_A_SECOND as u64);

/// The tree backend's name in snapshots' verdicts.
const BACKEND: &str = "ios-runner";

/// How long one call to the runner may take, a restart it waits for
/// included: a snapshot of a 500-cell screen takes about 550 ms.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long reads retry what the runner refuses while the foreground
/// changes under it (`window_changed`).
const READ_BUDGET: Duration = Duration::from_secs(5);

/// The pause between two tries of a read.
const READ_RETRY: Duration = Duration::from_millis(100);

/// How long text waits for the field it tapped to take the focus: a tree
/// read takes about half a second on the device.
const FOCUS_WAIT: Duration = Duration::from_millis(1500);

/// How long typing takes a character beside the call's own time: the runner
/// types at 60 characters a second (design D22).
const TYPING_PER_CHARACTER: Duration = Duration::from_millis(17);

const KEYBOARD_PAGE: u32 = 0x07;
const CONSUMER_PAGE: u32 = 0x0C;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The iPhones attached over USB.
pub(crate) struct IosDevices {
    /// The driver, opened on first use.
    driver: Mutex<Option<Arc<IosDriver>>>,
}

impl IosDevices {
    pub(crate) fn new() -> IosDevices {
        IosDevices {
            driver: Mutex::new(None),
        }
    }

    fn driver(&self) -> Result<Arc<IosDriver>, DriverError> {
        let mut slot = lock(&self.driver);
        if let Some(driver) = slot.as_ref() {
            return Ok(Arc::clone(driver));
        }
        let driver = Arc::new(IosDriver::open().map_err(|e| error(&e))?);
        *slot = Some(Arc::clone(&driver));
        Ok(driver)
    }
}

impl Driver for IosDevices {
    fn name(&self) -> &str {
        NAME
    }

    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError> {
        let devices = self.driver()?.devices().map_err(|e| error(&e))?;
        Ok(devices.into_iter().map(target_info).collect())
    }

    fn attach(
        &self,
        target: &TargetInfo,
        _options: &AttachOptions,
    ) -> Result<Attachment, DriverError> {
        attach(self.driver()?, &target.id)
    }

    fn compatibility(&self) -> Vec<CompatibilityRow> {
        doctor_rows(&ios::call_sites())
    }

    fn doctor(&self) -> Map<String, Value> {
        let bundle = runner_bundle(std::env::var_os(RUNNER_APP_VAR).as_deref());
        let mut fields = Map::new();
        fields.insert("iosRunner".into(), runner_doctor(&bundle));
        fields
    }

    /// Forgets the host's signing identity the driver keeps, when it is
    /// open, so that the listing `devices` makes checks it again.
    fn forget(&self) {
        if let Some(driver) = lock(&self.driver).as_ref() {
            driver.forget_identity();
        }
    }
}

/// What the attachment knows beside the driver's own status, for `status`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Facts {
    /// The device's iOS version, which picks the input path.
    pub ios_version: Option<String>,
    /// The way the session asked input to go.
    pub input: InputChoice,
    /// The last input dispatched, with the path it took.
    pub last_input: Option<Dispatch>,
    /// Frames the event stream delivered.
    pub frames: u64,
    /// Captures a second over both connections so far.
    pub capture_rate: Option<f64>,
    /// Why the screen could not be subscribed to.
    pub frame_error: Option<String>,
    /// The runner bundle an attach installs.
    pub bundle: Option<PathBuf>,
    /// The battery as the attach's probe read it.
    pub battery: Option<ios::Battery>,
}

/// What the parts of one target's backend share.
struct Shared {
    driver: Arc<IosDriver>,
    udid: String,
    /// Pixels per point, from lockdown.
    scale: f64,
    /// The device's iOS version as the attach found it.
    ios_version: Option<String>,
    /// The battery as the attach's probe read it.
    battery: Option<ios::Battery>,
    input: InputChoice,
    bundle: PathBuf,
    /// The frame subscription, `None` when it could not start.
    stream: Option<FrameStream>,
    frame_error: Option<String>,
    /// The latest read, where text finds the field a handle names.
    last: Mutex<Option<Snapshot>>,
    last_input: Mutex<Option<Dispatch>>,
    frames: AtomicU64,
}

impl Shared {
    fn remember(&self, snapshot: &Snapshot) {
        *lock(&self.last) = Some(snapshot.clone());
    }

    fn dispatched(&self, dispatch: Dispatch) {
        *lock(&self.last_input) = Some(dispatch);
    }

    fn facts(&self) -> Facts {
        let capture_rate = self.stream.as_ref().and_then(|stream| {
            let stats = stream.stats();
            let seconds = stats.elapsed.as_secs_f64();
            let captures: u64 = stats.captures.iter().sum();
            (seconds > 0.0).then(|| (captures as f64 / seconds * 10.0).round() / 10.0)
        });
        Facts {
            ios_version: self.ios_version.clone(),
            input: self.input,
            last_input: *lock(&self.last_input),
            frames: self.frames.load(Ordering::Relaxed),
            capture_rate,
            frame_error: self.frame_error.clone(),
            bundle: Some(self.bundle.clone()),
            battery: self.battery,
        }
    }

    /// The field `handle` names in the latest read: its centre in points,
    /// whether it has the focus and whether it holds text.
    fn field(&self, handle: NodeHandle) -> Option<Field> {
        let last = lock(&self.last);
        let snapshot = last.as_ref()?;
        let node = node_of(snapshot, handle)?;
        Some(Field {
            center: node_center(snapshot, self.scale, handle)?,
            focused: node.flags.contains(NodeFlags::FOCUSED),
            holds_text: node.text.as_deref().is_some_and(|text| !text.is_empty()),
        })
    }
}

/// A field text goes into, as the latest read has it.
struct Field {
    center: Point,
    focused: bool,
    holds_text: bool,
}

/// Releases the runner and closes what the driver opened for the device
/// when dropped.
struct Detach {
    driver: Arc<IosDriver>,
    udid: String,
}

impl Drop for Detach {
    fn drop(&mut self) {
        let _ = self.driver.release(&self.udid);
        let _ = self.driver.reset_target(&self.udid);
    }
}

fn attach(driver: Arc<IosDriver>, udid: &str) -> Result<Attachment, DriverError> {
    let input = input_choice(std::env::var(INPUT_VAR).ok().as_deref())?;
    let bundle = runner_bundle(std::env::var_os(RUNNER_APP_VAR).as_deref());
    // Created first, so a failure below releases what the attach started.
    let detach = Detach {
        driver: Arc::clone(&driver),
        udid: udid.to_owned(),
    };
    let connected = driver.connect(udid).map_err(|e| error(&e))?;
    let options = SessionOptions {
        input,
        ..SessionOptions::new(RunnerSpec::new(agent_ios::VERSION).with_bundle(&bundle))
    };
    driver
        .attach(udid, &options)
        .map_err(|e| runner_error(&e, &bundle))?;
    let screen = driver.screen(udid).map_err(|e| error(&e))?;
    let (stream, frame_error) = match driver.frames(udid, &FrameOptions::default()) {
        Ok(stream) => (Some(stream), None),
        Err(failure) => (None, Some(failure.to_string())),
    };
    let (width, height) = screen.points();
    let has_frames = stream.is_some();
    let shared = Arc::new(Shared {
        driver: Arc::clone(&driver),
        udid: udid.to_owned(),
        scale: screen.scale,
        ios_version: connected.device.ios_version.clone(),
        battery: connected.device.battery,
        input,
        bundle,
        stream,
        frame_error,
        last: Mutex::new(None),
        last_input: Mutex::new(None),
        frames: AtomicU64::new(0),
    });
    let part = || Part(Arc::clone(&shared));
    Ok(Attachment {
        backend: Backend {
            tree: Box::new(part()),
            events: Box::new(part()),
            frames: has_frames.then(|| Box::new(part()) as Box<dyn FrameSource>),
            input: Box::new(part()),
            clock: Box::new(SystemClock),
        },
        apps: Box::new(Apps {
            shared: Arc::clone(&shared),
            _detach: detach,
        }),
        display: Box::new(FixedScreen(Size { width, height })),
        // The screenshots come slowly (design D23): settle waits two
        // captures without a change and holds after input for its first
        // changed frame, as the re-read after input does.
        config: MirrorConfig {
            slow_frames: Some(SlowFrames::every(CAPTURE_INTERVAL)),
            ..MirrorConfig::default()
        },
        capture: Some(Box::new(part())),
        report: Some(Arc::new(part())),
        input_notes: None,
        input_session: None,
    })
}

/// One of the backend's parts; all share the target's state.
struct Part(Arc<Shared>);

/// Reads the foreground's tree through the runner, retrying for
/// [`READ_BUDGET`] what it refuses while the foreground changes.
fn read(shared: &Shared) -> Result<Snapshot, DriverError> {
    let deadline = Instant::now() + READ_BUDGET;
    loop {
        let call = Call::Snapshot(SnapshotParams::default());
        match shared.driver.call(&shared.udid, call, CALL_TIMEOUT) {
            Ok(Reply::Snapshot(snapshot)) => {
                shared.remember(&snapshot);
                return Ok(snapshot);
            }
            Ok(other) => return Err(unexpected("snapshot", &other)),
            Err(failure) if failure.retryable() && Instant::now() + READ_RETRY < deadline => {
                thread::sleep(READ_RETRY);
            }
            Err(failure) => return Err(error(&failure)),
        }
    }
}

/// The failure of a call the runner answered with another reply.
fn unexpected(method: &str, reply: &Reply) -> DriverError {
    let kind = match reply {
        Reply::Snapshot(_) => "a snapshot",
        Reply::Screenshot(_) => "a screenshot",
        Reply::Input(_) => "an input acknowledgement",
        Reply::Text(_) => "a text acknowledgement",
        Reply::Resync(_) => "a resync",
        Reply::Bounds(_) => "bounds",
        Reply::Empty => "nothing",
    };
    DriverError::new(
        "device_error",
        format!("the runner answered {method} with {kind}"),
    )
}

impl TreeSource for Part {
    /// Every read covers the whole foreground, the keyboard's window with
    /// the application's.
    fn read(&mut self, _request: &ReadRequest) -> Result<TreeRead, BackendError> {
        let snapshot = read(&self.0)?;
        Ok(TreeRead {
            snapshot,
            backend: BackendInfo::new(BACKEND, false),
            scale: self.0.scale,
        })
    }
}

/// A frame as the mirror takes it: the screenshot's arrival as its time.
fn meta(frame: ios::Frame) -> FrameMeta {
    FrameMeta {
        seq: frame.sequence,
        at: frame.arrived,
        hash: frame.hash,
        width: frame.width,
        height: frame.height,
        rotation: Rotation::R0,
    }
}

impl EventStream for Part {
    fn next(&mut self, deadline: Instant) -> Option<Timed<StreamItem>> {
        let wait = deadline.saturating_duration_since(Instant::now());
        let frame = self.0.stream.as_ref().and_then(|stream| stream.next(wait));
        let Some(frame) = frame else {
            // A stream that ended, or none at all, still waits out the
            // deadline, as the mirror expects.
            thread::sleep(deadline.saturating_duration_since(Instant::now()));
            return None;
        };
        self.0.frames.fetch_add(1, Ordering::Relaxed);
        Some(Timed {
            at: Instant::now(),
            item: StreamItem::Frame(meta(frame)),
        })
    }
}

impl FrameSource for Part {
    fn latest(&self) -> Option<FrameMeta> {
        self.0
            .stream
            .as_ref()
            .and_then(FrameStream::latest)
            .map(meta)
    }

    /// One of the frames the stream holds, scaled and encoded as JPEG; the
    /// frames are upright, so no rotation applies.
    fn encode(&mut self, request: &EncodeRequest) -> Result<Vec<u8>, BackendError> {
        let stream =
            self.0.stream.as_ref().ok_or_else(|| {
                BackendError::new(mirror::code::NO_FRAME, "no frame subscription")
            })?;
        stream
            .encode(request.seq, request.width, request.height, request.quality)
            .map_err(|e| error(&e))
    }
}

fn point(at: Point) -> ios::Point {
    ios::Point::new(at.x, at.y)
}

impl InputSink for Part {
    fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError> {
        let shared = &self.0;
        let (driver, udid) = (&shared.driver, shared.udid.as_str());
        let dispatch = match input {
            Input::Tap { at, hold: None } => driver.tap(udid, point(*at)),
            Input::Tap {
                at,
                hold: Some(hold),
            } => driver.press(udid, point(*at), *hold),
            Input::Swipe {
                from,
                to,
                duration,
                release,
            } => {
                let swipe = ios::Swipe::new(point(*from), point(*to), *duration);
                let swipe = match release {
                    Release::Moving => swipe,
                    Release::Resting => swipe.resting(),
                };
                driver.swipe(udid, &swipe)
            }
            Input::Key { code, meta } => match ios_key(*code, *meta)? {
                IosKey::Key(key) => driver.key(udid, key),
                IosKey::Button(button) => driver.button(udid, button),
            },
            Input::Text { text, node, method } => return self.text(text, *node, *method),
        }
        .map_err(|e| error(&e))?;
        shared.dispatched(dispatch);
        Ok(InputAck {
            accepted: true,
            method: None,
        })
    }
}

impl Part {
    /// Enters `text` through the runner into the field with keyboard focus,
    /// the field `node` names when given, which is tapped first unless the
    /// latest read shows it focused. Text whose method is left to the driver
    /// is set rather than typed when the field holds text, so it replaces
    /// what the field holds, as `fill` means; the runner reports the method
    /// it used.
    fn text(
        &mut self,
        text: &str,
        node: Option<NodeHandle>,
        method: TextMethod,
    ) -> Result<InputAck, BackendError> {
        let shared = &self.0;
        let mut mode = method;
        if let Some(handle) = node {
            let field = shared.field(handle).ok_or_else(|| {
                BackendError::new(
                    mirror::code::STALE_REF,
                    "the field is not in the latest read of the screen; take a snapshot",
                )
            })?;
            if !field.focused {
                let dispatch = shared
                    .driver
                    .tap(&shared.udid, point(field.center))
                    .map_err(|e| error(&e))?;
                shared.dispatched(dispatch);
                wait_for_focus(shared);
            }
            if method == TextMethod::AUTO && field.holds_text {
                mode = TextMethod::DIRECT;
            }
        }
        let characters = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
        let call = Call::TextSet(TextParams {
            text: text.to_owned(),
            node: None,
            mode: Some(mode),
        });
        let timeout = CALL_TIMEOUT + TYPING_PER_CHARACTER * characters;
        match shared
            .driver
            .call(&shared.udid, call, timeout)
            .map_err(|e| error(&e))?
        {
            Reply::Text(ack) => Ok(InputAck {
                accepted: true,
                method: Some(ack.method),
            }),
            other => Err(unexpected("text_set", &other)),
        }
    }
}

/// Reads the screen until an editable node has the focus, at most
/// [`FOCUS_WAIT`]: the tap that gave it has to reach the app first.
fn wait_for_focus(shared: &Shared) {
    let deadline = Instant::now() + FOCUS_WAIT;
    while Instant::now() < deadline {
        if let Ok(snapshot) = read(shared)
            && has_focused_field(&snapshot)
        {
            return;
        }
        thread::sleep(READ_RETRY);
    }
}

/// The node `handle` names in `snapshot`.
fn node_of(snapshot: &Snapshot, handle: NodeHandle) -> Option<&Node> {
    let window = snapshot
        .windows
        .iter()
        .find(|window| window.id == handle.window)?;
    let mut stack = vec![window.root.as_ref()?];
    while let Some(node) = stack.pop() {
        if node.id == handle.node {
            return Some(node);
        }
        stack.extend(node.children.iter());
    }
    None
}

struct Apps {
    shared: Arc<Shared>,
    _detach: Detach,
}

impl AppControl for Apps {
    /// Launches `app` with its launch environment through DVT's process
    /// control; a relaunch ends the running instance first, and a running
    /// app keeps its process otherwise.
    fn launch(&mut self, app: &str, launch: &Launch) -> Result<(), DriverError> {
        let options = LaunchOptions {
            arguments: Vec::new(),
            environment: launch.env.clone(),
            terminate_running: launch.relaunch,
        };
        self.shared
            .driver
            .launch(&self.shared.udid, app, &options)
            .map(|_| ())
            .map_err(|e| app_error(&e))
    }

    fn terminate(&mut self, app: &str) -> Result<(), DriverError> {
        self.shared
            .driver
            .terminate(&self.shared.udid, app)
            .map(|_| ())
            .map_err(|e| app_error(&e))
    }
}

impl Capture for Part {
    /// The screen now through DVT's screenshot, scaled to the long edge
    /// asked for and encoded as JPEG.
    fn screenshot(&mut self, request: &CaptureRequest) -> Result<Image, DriverError> {
        let options = ScreenshotOptions {
            max_dimension: request.max_dimension,
            quality: request.quality,
            format: ImageFormat::Jpeg,
        };
        let shot = self
            .0
            .driver
            .screenshot(&self.0.udid, &options)
            .map_err(|e| error(&e))?;
        Ok(Image {
            data: shot.data,
            width: shot.width,
            height: shot.height,
            format: "jpeg",
        })
    }
}

impl Report for Part {
    fn report(&self) -> TargetReport {
        match self.0.driver.status(&self.0.udid) {
            Ok(status) => status_report(&status, &self.0.facts()),
            Err(failure) => {
                let mut data = Map::new();
                data.insert("error".into(), failure.to_string().into());
                TargetReport {
                    lines: vec![format!("status: {failure}")],
                    data,
                }
            }
        }
    }
}

/// An iPhone as `devices` lists it: by its identifier, in the `ios-device`
/// class, named as its owner named it, else by its model; its state the
/// readiness code, and for a state that keeps it from a session the
/// state's message, which names the step to take.
pub(crate) fn target_info(device: DeviceInfo) -> TargetInfo {
    let advice = (!device.readiness.is_ready()).then(|| device.readiness.message());
    TargetInfo {
        name: device
            .name
            .clone()
            .or_else(|| device.model.clone())
            .unwrap_or_else(|| device.udid.clone()),
        platform: Platform::Ios,
        class: TargetClass::IosDevice,
        model: device.model,
        os: device.ios_version.map(|version| format!("iOS {version}")),
        state: device.readiness.code().to_owned(),
        advice,
        driver: NAME.to_owned(),
        id: device.udid,
    }
}

/// The workspace this daemon was built in.
fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/daemon sits two levels below the workspace's root")
}

/// The runner bundle an attach installs: the one `MOBILE_AGENT_IOS_RUNNER_APP`
/// (`variable`) names, a relative path taken from the workspace's root, else
/// the one `agent-ios/build.sh` writes in the workspace this daemon was
/// built in, as the Android driver finds its jar.
pub(crate) fn runner_bundle(variable: Option<&OsStr>) -> PathBuf {
    match variable.filter(|path| !path.is_empty()) {
        Some(path) => workspace().join(path),
        None => workspace().join(RUNNER_BUNDLE),
    }
}

/// What `doctor` says of the runner bundle: where it is and the version
/// the runner's hello must name, for doctor's check of the build.
pub(crate) fn runner_doctor(bundle: &Path) -> Value {
    json!({
        "bundle": bundle.to_string_lossy(),
        "version": agent_ios::VERSION,
        "variable": RUNNER_APP_VAR,
    })
}

/// The way input goes, from `MOBILE_AGENT_IOS_INPUT`'s value: `auto` (or
/// none), `dtuhidd` or `runner`, whatever the case.
pub(crate) fn input_choice(variable: Option<&str>) -> Result<InputChoice, DriverError> {
    match variable
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("" | "auto") => Ok(InputChoice::Auto),
        Some("dtuhidd") => Ok(InputChoice::Only(InputPath::Hid)),
        Some("runner") => Ok(InputChoice::Only(InputPath::Runner)),
        Some(other) => Err(DriverError::new(
            INVALID_ARGUMENT,
            format!(
                "{INPUT_VAR}={other:?}: use auto, for dtuhidd where the device's iOS takes its \
                 reports and the runner's synthesis elsewhere, dtuhidd, or runner"
            ),
        )),
    }
}

/// A driver error as the protocol carries it, with its code, a readiness
/// state's own among them, and retryable flag.
pub(crate) fn error(error: &ios::Error) -> DriverError {
    DriverError {
        code: error.code().to_owned(),
        message: error.to_string(),
        retryable: error.retryable(),
    }
}

/// A launch or stop's error: an app that is not installed is
/// `app_not_found`, as on the other drivers.
pub(crate) fn app_error(failure: &ios::Error) -> DriverError {
    match failure {
        ios::Error::AppNotInstalled { .. } => DriverError::new(APP_NOT_FOUND, failure.to_string()),
        other => error(other),
    }
}

/// The attach's error: one about the runner bundle names where the bundle
/// is built when it is not there.
fn runner_error(failure: &ios::Error, bundle: &Path) -> DriverError {
    let mut mapped = error(failure);
    let about_the_bundle = matches!(
        failure.code(),
        "invalid_bundle" | "runner_not_installed" | "runner_version_mismatch"
    );
    if about_the_bundle && !bundle.exists() {
        mapped.message.push_str(&format!(
            "; no runner bundle is built at {}: build it with just agent-ios-build \
             (agent-ios/build.sh, with {} set), or name one with {RUNNER_APP_VAR}",
            bundle.display(),
            ios::TEAM_VARIABLE
        ));
    }
    mapped
}

/// What an iOS key code names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IosKey {
    /// A key of the keyboard page, with its modifiers.
    Key(ios::Key),
    /// A hardware button of the consumer page.
    Button(ios::Button),
}

/// The key or button `code` names: `(page << 16) | usage`, keyboard keys
/// on page 7 (or no page) with `meta` holding the HID modifier byte, and
/// the buttons of the consumer page 0x0C a phone has, as `keys.rs` names
/// them for iOS.
pub(crate) fn ios_key(code: u32, meta: Option<u32>) -> Result<IosKey, DriverError> {
    let (page, usage) = (code >> 16, code & 0xFFFF);
    let unknown = || {
        DriverError::new(
            UNKNOWN_KEY,
            format!(
                "{code:#x} is no key or button an iPhone takes: keyboard usages (page 7) and the \
                 consumer page's Home, Power, VolumeUp and VolumeDown"
            ),
        )
    };
    match page {
        0 | KEYBOARD_PAGE => Ok(IosKey::Key(ios::Key {
            usage: u16::try_from(usage).map_err(|_| unknown())?,
            modifiers: modifiers(meta.unwrap_or(0)),
        })),
        CONSUMER_PAGE => Ok(IosKey::Button(match usage {
            0x40 => ios::Button::Home,
            0x30 => ios::Button::Lock,
            0xE9 => ios::Button::VolumeUp,
            0xEA => ios::Button::VolumeDown,
            _ => return Err(unknown()),
        })),
        _ => Err(unknown()),
    }
}

/// The modifiers the HID modifier byte holds, the right-hand keys counting
/// as the left ones.
fn modifiers(meta: u32) -> ios::Modifiers {
    let held = |left: u32| meta & (left | left << 4) != 0;
    ios::Modifiers {
        control: held(0x01),
        shift: held(0x02),
        option: held(0x04),
        command: held(0x08),
    }
}

/// The compatibility table as `doctor`'s rows, in the shape the Simulator
/// driver's take: the iOS range the driver supports, then every call site
/// with the iOS and Xcode it was validated on, `supported` where it was not
/// exercised.
pub(crate) fn doctor_rows(sites: &[CallSite]) -> Vec<CompatibilityRow> {
    let range = CompatibilityRow {
        subject: format!("iOS {}, Xcode {}", ios::VALIDATED_IOS, ios::VALIDATED_XCODE),
        feature: format!(
            "physical iPhones: {}, through idevice {}",
            ios::supported_range(),
            ios::IDEVICE
        ),
        status: "supported".to_owned(),
        note: Some(format!(
            "host input through dtuhidd from iOS {}; another iOS is refused with unsupported_ios",
            ios::FIRST_HID_IOS
        )),
    };
    let calls = sites.iter().map(|site| CompatibilityRow {
        subject: site.validated_on.to_owned(),
        feature: format!("{} ({})", site.call, site.service),
        status: if site.validated_on.contains("not exercised") {
            "supported"
        } else {
            "validated"
        }
        .to_owned(),
        note: None,
    });
    std::iter::once(range).chain(calls).collect()
}

/// The way input goes as `facts` say, and why: the path the last input
/// took, else the one the session's choice and the device's iOS pick.
fn input_path(facts: &Facts) -> (InputPath, String) {
    let (planned, reason) = match facts.input {
        InputChoice::Only(InputPath::Runner) => (
            InputPath::Runner,
            format!("{INPUT_VAR}=runner asks for the runner's synthesis"),
        ),
        InputChoice::Only(InputPath::Hid) => (
            InputPath::Hid,
            format!("{INPUT_VAR}=dtuhidd asks for dtuhidd alone"),
        ),
        InputChoice::Auto => match ios::hid_input(facts.ios_version.as_deref()) {
            Ok(()) => (
                InputPath::Hid,
                format!(
                    "iOS {} takes dtuhidd's reports from the host without a display stream",
                    facts.ios_version.as_deref().unwrap_or_default()
                ),
            ),
            Err(reason) => (InputPath::Runner, reason),
        },
    };
    (facts.last_input.map_or(planned, |last| last.path), reason)
}

fn path_name(path: InputPath) -> &'static str {
    match path {
        InputPath::Hid => "dtuhidd",
        InputPath::Runner => "runner",
    }
}

/// `count` and `noun`, plural unless the count is one.
fn counted(count: u64, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// What `status` shows about an attached iPhone: the input path and why,
/// with text through the runner; the frames, screenshots polled at the
/// device's rate; the runner, its agent, state, restarts, start and the
/// leftovers of a lost session its start ended; the tunnel and the disk
/// image; the runner bundle; the battery as the attach read it.
pub(crate) fn status_report(status: &TargetStatus, facts: &Facts) -> TargetReport {
    let mut data = Map::new();
    let mut lines = Vec::new();

    let (path, reason) = input_path(facts);
    let mut line = format!("input: {} ({reason})", path_name(path));
    if let Some(last) = facts.last_input {
        line.push_str(&format!(
            ", the last acknowledged in {:.1} ms",
            millis(last.acknowledged)
        ));
    }
    line.push_str("; text through the runner, typed or set");
    lines.push(line);
    data.insert("input".into(), path_name(path).into());
    data.insert("inputReason".into(), reason.into());
    data.insert(
        "lastInputMs".into(),
        facts
            .last_input
            .map(|last| millis(last.acknowledged))
            .into(),
    );

    match &facts.frame_error {
        None => {
            let rate = facts.capture_rate.map_or_else(
                || "no capture yet".to_owned(),
                |rate| format!("{rate:.1} captures a second"),
            );
            lines.push(format!(
                "frames: screenshots polled on DVT and CoreDevice, {rate}, {} received",
                facts.frames
            ));
            data.insert("frames".into(), "screenshots".into());
        }
        Some(failure) => {
            lines.push(format!(
                "frames: none; the screenshots could not be subscribed to: {failure}"
            ));
            data.insert("frames".into(), Value::Null);
        }
    }
    data.insert("captureRate".into(), facts.capture_rate.into());
    data.insert("framesReceived".into(), facts.frames.into());
    data.insert("frameError".into(), facts.frame_error.clone().into());

    match &status.runner {
        Some(runner) => {
            let agent = runner.agent.as_deref().unwrap_or("no hello yet");
            let mut line = format!(
                "runner: {agent}, {}, {}",
                runner.state.name(),
                counted(u64::from(runner.restarts), "restart")
            );
            match &runner.state {
                RunnerState::Starting { step } => line.push_str(&format!(", waiting for {step}")),
                RunnerState::Restarting { cause } => line.push_str(&format!(", after {cause}")),
                RunnerState::Failed { code, message } => {
                    line.push_str(&format!(", {code}: {message}"));
                }
                RunnerState::AwaitingApproval => line.push_str(&format!(
                    ", {}",
                    ios::Readiness::AutomationNotApproved.message()
                )),
                RunnerState::Ready | RunnerState::Stopped => {}
            }
            if let Some(started) = runner.started_in {
                line.push_str(&format!(
                    ", hello {:.0} ms after its start",
                    millis(started)
                ));
            }
            if !runner.leftovers.is_empty() {
                let pids: Vec<String> = runner.leftovers.iter().map(u32::to_string).collect();
                line.push_str(&format!(
                    ", ended {} left from a lost session (pid {})",
                    counted(runner.leftovers.len() as u64, "runner"),
                    pids.join(", ")
                ));
            }
            if runner.replaced {
                line.push_str(", installed from the runner bundle");
            }
            lines.push(line);
            data.insert(
                "runner".into(),
                json!({
                    "agent": runner.agent,
                    "state": runner.state.name(),
                    "restarts": runner.restarts,
                    "handshakeRetries": runner.handshake_retries,
                    "replaced": runner.replaced,
                    "leftovers": runner.leftovers,
                    "startedMs": runner.started_in.map(millis),
                    "lastDeath": runner.last_death,
                }),
            );
        }
        None => {
            lines.push("runner: not attached".to_owned());
            data.insert("runner".into(), Value::Null);
        }
    }

    match &status.tunnel {
        Some(tunnel) => {
            let missing = if tunnel.missing.is_empty() {
                "none missing".to_owned()
            } else {
                format!("missing {}", tunnel.missing.join(", "))
            };
            lines.push(format!(
                "tunnel: {} RSD services, {missing}, opened in {:.1} ms, {}",
                tunnel.services.len(),
                millis(tunnel.opened_in),
                counted(u64::from(tunnel.opens), "open")
            ));
            data.insert(
                "tunnel".into(),
                json!({
                    "opens": tunnel.opens,
                    "openedMs": millis(tunnel.opened_in),
                    "services": tunnel.services.len(),
                    "missing": tunnel.missing,
                }),
            );
        }
        None => {
            data.insert("tunnel".into(), Value::Null);
        }
    }
    match status.disk_image.as_ref().map(|report| &report.image) {
        Some(ios::DiskImage::Mounted {
            image_type,
            mount_path,
        }) => {
            let at = mount_path.as_deref().unwrap_or("its mount point");
            lines.push(format!("disk image: {image_type} at {at}"));
            data.insert(
                "diskImage".into(),
                json!({"type": image_type, "path": mount_path}),
            );
        }
        _ => {
            data.insert("diskImage".into(), Value::Null);
        }
    }
    if let Some(bundle) = &facts.bundle {
        lines.push(format!("runner bundle: {}", bundle.display()));
    }
    data.insert(
        "runnerBundle".into(),
        facts
            .bundle
            .as_ref()
            .map(|bundle| bundle.to_string_lossy().into_owned())
            .into(),
    );
    match facts.battery {
        Some(battery) => {
            let mut parts = Vec::new();
            if let Some(level) = battery.level {
                parts.push(format!("{level}%"));
            }
            if let Some(charging) = battery.charging {
                parts.push(if charging { "charging" } else { "not charging" }.to_owned());
            }
            if let Some(external) = battery.external_power {
                parts.push(
                    if external {
                        "on external power"
                    } else {
                        "on its battery"
                    }
                    .to_owned(),
                );
            }
            lines.push(format!("battery: {}, when attached", parts.join(", ")));
            data.insert(
                "battery".into(),
                json!({
                    "levelPct": battery.level,
                    "charging": battery.charging,
                    "externalPower": battery.external_power,
                }),
            );
        }
        None => {
            data.insert("battery".into(), Value::Null);
        }
    }
    TargetReport { lines, data }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use ios::{
        DiskImage, InputPath, Modifiers, MountReport, Probe, Readiness, RunnerState, RunnerStatus,
        Trust, TunnelStatus,
    };
    use serde_json::json;

    use crate::driver::{Platform, TargetClass};

    const PHONE: &str = "placeholder-udid-phone";

    fn device(readiness: Readiness) -> DeviceInfo {
        DeviceInfo {
            udid: PHONE.into(),
            name: Some("placeholder-iphone".into()),
            model: Some("iPhone14,6".into()),
            ios_version: Some("26.6.2".into()),
            build: Some("23G90".into()),
            battery: None,
            probe: Probe {
                trust: Trust::Trusted,
                developer_mode: Some(true),
                disk_image: Some(DiskImage::Mounted {
                    image_type: "Personalized".into(),
                    mount_path: Some("/System/Developer".into()),
                }),
                password_protected: Some(false),
            },
            readiness,
        }
    }

    /// Task 5.1, spec `ios-device-driver` "Ready device listed": an iPhone
    /// is listed by its identifier in the `ios-device` class, with its name,
    /// model and iOS version, ready.
    #[test]
    fn a_ready_iphone_is_listed_with_its_name_model_and_ios_version() {
        let target = target_info(device(Readiness::Ready));
        assert_eq!(target.id, PHONE);
        assert_eq!(target.platform, Platform::Ios);
        assert_eq!(target.class, TargetClass::IosDevice);
        assert_eq!(target.class.id(), "ios-device");
        assert_eq!(target.name, "placeholder-iphone");
        assert_eq!(target.model.as_deref(), Some("iPhone14,6"));
        assert_eq!(target.os.as_deref(), Some("iOS 26.6.2"));
        assert_eq!(target.state, "ready");
        assert_eq!(target.advice, None);
        assert_eq!(target.driver, NAME);
    }

    /// Task 5.1, spec `ios-device-driver` "Device not ready": an unready
    /// iPhone is listed in its readiness state, the state's message from
    /// `crates/ios` its advice; one lockdown described nothing of goes by
    /// its identifier.
    #[test]
    fn an_unready_iphone_is_listed_in_its_state_with_the_step_as_advice() {
        for readiness in [
            Readiness::NotTrusted {
                reason: "usbmuxd holds no pairing record for it".into(),
            },
            Readiness::DeveloperModeOff,
            Readiness::DiskImageUnmounted,
            Readiness::Locked,
            Readiness::NoSigningIdentity {
                reason: "MOBILE_AGENT_IOS_TEAM is not set".into(),
            },
        ] {
            let target = target_info(device(readiness.clone()));
            assert_eq!(target.state, readiness.code());
            assert_eq!(target.advice.as_deref(), Some(readiness.message().as_str()));
        }
        let bare = DeviceInfo {
            name: None,
            model: None,
            ios_version: None,
            ..device(Readiness::NotTrusted {
                reason: "lockdown did not answer".into(),
            })
        };
        let target = target_info(bare);
        assert_eq!(target.name, PHONE);
        assert_eq!(target.os, None);
    }

    fn status() -> TargetStatus {
        TargetStatus {
            tunnel: Some(TunnelStatus {
                opens: 1,
                opened_in: Duration::from_micros(77_000),
                services: (0..82)
                    .map(|n| format!("placeholder.service.{n}"))
                    .collect(),
                missing: Vec::new(),
            }),
            disk_image: Some(MountReport {
                image: DiskImage::Mounted {
                    image_type: "Personalized".into(),
                    mount_path: Some("/System/Developer".into()),
                },
                attempts: Vec::new(),
            }),
            services_open: vec!["com.apple.instruments.server.services.processcontrol"],
            runner: Some(RunnerStatus {
                state: RunnerState::Ready,
                agent: Some("mobile-agent-ios-runner 0.2.0".into()),
                capabilities: protocol::Capabilities::TREE
                    | protocol::Capabilities::INPUT
                    | protocol::Capabilities::SCREENSHOT
                    | protocol::Capabilities::TEXT_SET,
                restarts: 1,
                handshake_retries: 0,
                replaced: false,
                leftovers: vec![412],
                started_in: Some(Duration::from_millis(812)),
                last_death: Some("the runner's connection closed".into()),
            }),
        }
    }

    fn facts() -> Facts {
        Facts {
            ios_version: Some("26.6.2".into()),
            input: InputChoice::Auto,
            last_input: None,
            frames: 34,
            capture_rate: Some(19.1),
            frame_error: None,
            bundle: Some(PathBuf::from(
                "/placeholder/agent-ios/dist/MobileAgentRunner-Runner.app",
            )),
            battery: None,
        }
    }

    /// Task 6.2: the status names the battery the attach's probe read, which
    /// the harness records in a run's provenance; a probe that read none
    /// leaves it null.
    #[test]
    fn the_status_names_the_battery_the_attach_read() {
        let charging = Facts {
            battery: Some(ios::Battery {
                level: Some(76),
                charging: Some(true),
                external_power: Some(true),
            }),
            ..facts()
        };
        let report = status_report(&status(), &charging);
        let data = Value::Object(report.data.clone());
        assert_eq!(
            data["battery"],
            json!({"levelPct": 76, "charging": true, "externalPower": true}),
            "{data}"
        );
        assert!(
            report
                .lines
                .iter()
                .any(|line| line == "battery: 76%, charging, on external power, when attached"),
            "{:?}",
            report.lines
        );
        let report = status_report(&status(), &facts());
        assert_eq!(report.data["battery"], Value::Null);
        assert!(
            report
                .lines
                .iter()
                .all(|line| !line.starts_with("battery:")),
            "{:?}",
            report.lines
        );
    }

    /// Task 5.1, spec `ios-device-driver` "Runner stays up": the status
    /// names the input path, the frame path with its capture rate, and the
    /// runner's agent, restarts and the leftovers its start ended.
    #[test]
    fn the_status_names_the_input_path_the_frames_with_their_rate_and_the_runner() {
        let report = status_report(&status(), &facts());
        let data = Value::Object(report.data.clone());
        assert_eq!(data["input"], "dtuhidd", "{data}");
        assert_eq!(data["frames"], "screenshots", "{data}");
        assert_eq!(data["captureRate"], 19.1, "{data}");
        assert_eq!(data["framesReceived"], 34, "{data}");
        assert_eq!(
            data["runner"]["agent"], "mobile-agent-ios-runner 0.2.0",
            "{data}"
        );
        assert_eq!(data["runner"]["state"], "ready", "{data}");
        assert_eq!(data["runner"]["restarts"], 1, "{data}");
        assert_eq!(data["runner"]["leftovers"], json!([412]), "{data}");
        assert_eq!(data["runner"]["startedMs"], 812.0, "{data}");
        assert_eq!(data["tunnel"]["services"], 82, "{data}");
        let text = report.lines.join("\n");
        for part in [
            "input: dtuhidd",
            "frames: screenshots",
            "19.1 captures a second",
            "34 received",
            "runner: mobile-agent-ios-runner 0.2.0",
            "1 restart",
            "412",
        ] {
            assert!(text.contains(part), "{part:?} is missing from:\n{text}");
        }

        // The runner's synthesis, asked for, and the input it took.
        let runner = Facts {
            input: InputChoice::Only(InputPath::Runner),
            last_input: Some(Dispatch {
                path: InputPath::Runner,
                reports: 1,
                first: Duration::from_micros(290_000),
                acknowledged: Duration::from_micros(292_200),
            }),
            ..facts()
        };
        let report = status_report(&status(), &runner);
        let data = Value::Object(report.data.clone());
        assert_eq!(data["input"], "runner", "{data}");
        assert_eq!(data["lastInputMs"], 292.2, "{data}");
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.starts_with("input: runner")),
            "{:?}",
            report.lines
        );

        // No frames: why.
        let blind = Facts {
            frames: 0,
            capture_rate: None,
            frame_error: Some("DVT's takeScreenshot failed".into()),
            ..facts()
        };
        let report = status_report(&status(), &blind);
        let data = Value::Object(report.data.clone());
        assert_eq!(data["frames"], Value::Null, "{data}");
        assert_eq!(data["frameError"], "DVT's takeScreenshot failed", "{data}");
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.starts_with("frames: none") && line.contains("takeScreenshot")),
            "{:?}",
            report.lines
        );
    }

    /// Task 5.1, spec `ios-device-driver` "Table lists the device paths":
    /// the supported range, then each call site with the iOS and Xcode it
    /// was validated on, in the shape the Simulator driver's rows take.
    #[test]
    fn the_compatibility_rows_are_the_supported_range_then_every_call_site() {
        let sites = ios::call_sites();
        let rows = doctor_rows(&sites);
        assert_eq!(rows.len(), sites.len() + 1);
        let range = &rows[0];
        assert!(
            range.subject.contains("iOS 26.6.2") && range.subject.contains("Xcode 27.0"),
            "{range:?}"
        );
        assert!(
            range.feature.contains("17.4") && range.feature.contains("27"),
            "{range:?}"
        );
        assert_eq!(range.status, "supported");
        for (row, site) in rows[1..].iter().zip(&sites) {
            assert_eq!(row.subject, site.validated_on);
            assert!(
                row.feature.contains(site.call) && row.feature.contains(site.service),
                "{row:?}"
            );
            let status = if site.validated_on.contains("not exercised") {
                "supported"
            } else {
                "validated"
            };
            assert_eq!(row.status, status, "{row:?}");
        }
    }

    /// Task 5.1: the runner bundle is the one `agent-ios/build.sh` writes in
    /// this workspace, unless `MOBILE_AGENT_IOS_RUNNER_APP` names another, a
    /// relative path taken from the workspace's root, as the device tests of
    /// `crates/ios` take it.
    #[test]
    fn the_runner_bundle_is_the_built_one_or_the_one_the_variable_names() {
        let built = runner_bundle(None);
        assert!(built.is_absolute(), "{}", built.display());
        assert!(
            built.ends_with("agent-ios/dist/MobileAgentRunner-Runner.app"),
            "{}",
            built.display()
        );
        assert_eq!(runner_bundle(Some(OsStr::new(""))), built);
        let dist = built.parent().expect("the bundle sits in dist");
        assert_eq!(
            runner_bundle(Some(OsStr::new("agent-ios/dist/Other.app"))),
            dist.join("Other.app")
        );
        assert_eq!(
            runner_bundle(Some(OsStr::new("/placeholder/Runner.app"))),
            PathBuf::from("/placeholder/Runner.app")
        );
        let doctor = runner_doctor(&built);
        assert_eq!(doctor["bundle"], built.to_string_lossy().as_ref());
        assert_eq!(doctor["version"], agent_ios::VERSION);
    }

    /// Task 5.1: `MOBILE_AGENT_IOS_INPUT` picks the way input goes: `auto`,
    /// the default, by the device's iOS; `dtuhidd` or `runner` alone.
    #[test]
    fn the_input_variable_takes_auto_dtuhidd_or_runner() {
        for (value, choice) in [
            (None, InputChoice::Auto),
            (Some(""), InputChoice::Auto),
            (Some("auto"), InputChoice::Auto),
            (Some(" Runner "), InputChoice::Only(InputPath::Runner)),
            (Some("dtuhidd"), InputChoice::Only(InputPath::Hid)),
        ] {
            assert_eq!(input_choice(value), Ok(choice), "{value:?}");
        }
        let refused = input_choice(Some("bluetooth")).unwrap_err();
        assert_eq!(refused.code, "invalid_argument");
        for part in [INPUT_VAR, "bluetooth", "auto", "dtuhidd", "runner"] {
            assert!(refused.message.contains(part), "{refused:?}");
        }
    }

    /// Task 5.1: the driver's errors keep their codes, a readiness state's
    /// among them, and their retryable flag; an app that is not installed is
    /// `app_not_found`, as on the other drivers.
    #[test]
    fn driver_errors_keep_their_code_and_retryable_flag() {
        let locked = error(&ios::Error::NotReady {
            target: PHONE.into(),
            readiness: Readiness::Locked,
        });
        assert_eq!(locked.code, "device_locked");
        assert!(!locked.retryable);
        assert!(locked.message.contains("unlock it"), "{locked:?}");
        let late = error(&ios::Error::Timeout {
            what: "the runner's reply".into(),
            after: Duration::from_secs(10),
        });
        assert_eq!(late.code, "timeout");
        assert!(late.retryable);
        let missing = app_error(&ios::Error::AppNotInstalled {
            target: PHONE.into(),
            bundle_id: "dev.mobileagent.placeholder".into(),
        });
        assert_eq!(missing.code, "app_not_found");
        assert!(
            missing.message.contains("dev.mobileagent.placeholder"),
            "{missing:?}"
        );
        let refused = app_error(&ios::Error::LaunchFailed {
            bundle_id: "dev.mobileagent.placeholder".into(),
            reason: "denied".into(),
        });
        assert_eq!(refused.code, "launch_failed");
    }

    /// Physical-ios, task 9.3 (design D26): a runner session that ended for
    /// good, the runner dead again after its restart or its restart failed
    /// (`runner_died`), reaches the daemon as `disconnected`, its message
    /// kept, so that the worker detaches the target and the next command
    /// attaches again, as after a lost connection the driver reports so.
    #[test]
    fn a_runner_that_died_and_could_not_start_again_is_disconnected() {
        let died = error(&ios::Error::RunnerDied {
            target: PHONE.into(),
            cause: "its connection closed: eof; starting it again failed: no hello".into(),
        });
        assert_eq!(died.code, "disconnected", "{died:?}");
        assert!(!died.retryable);
        assert!(
            died.message.contains("died and could not be started again")
                && died.message.contains("no hello"),
            "{died:?}"
        );
    }

    /// Task 5.1: iOS key codes are keyboard usages with the HID modifier
    /// byte, or consumer-page buttons, as on the Simulator.
    #[test]
    fn keyboard_codes_become_keys_and_consumer_codes_buttons() {
        let key = |usage, modifiers| IosKey::Key(ios::Key { usage, modifiers });
        assert_eq!(
            ios_key(0x07 << 16 | 0x28, None).unwrap(),
            key(0x28, Modifiers::default())
        );
        assert_eq!(
            ios_key(0x04, Some(0x02 | 0x80)).unwrap(),
            key(
                0x04,
                Modifiers {
                    shift: true,
                    command: true,
                    ..Modifiers::default()
                }
            ),
            "a code without a page is a keyboard usage; right-hand keys count as left"
        );
        let button = |code: u32| ios_key(0x0C << 16 | code, None).unwrap();
        assert_eq!(button(0x40), IosKey::Button(ios::Button::Home));
        assert_eq!(button(0x30), IosKey::Button(ios::Button::Lock));
        assert_eq!(button(0xE9), IosKey::Button(ios::Button::VolumeUp));
        assert_eq!(button(0xEA), IosKey::Button(ios::Button::VolumeDown));
        for unknown in [0x0C << 16 | 0xCF, 0x09 << 16 | 0x01] {
            let refused = ios_key(unknown, None).unwrap_err();
            assert_eq!(refused.code, "unknown_key", "{refused:?}");
        }
    }
}
