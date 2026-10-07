//! The Simulator driver (`crates/simulator`, design D6 and D7) behind the
//! daemon's driver contract and the mirror's backend traits.
//!
//! - Discovery lists the booted Simulators of the default device set; the
//!   driver opens CoreSimulator on first use, not when the daemon starts.
//! - The tree source reads the foreground through the driver: its reader
//!   spawned into the Simulator, or the host translator, whose reads are
//!   marked fallback. Every Simulator window sends no change events, so the
//!   mirror re-reads after input, when frames change and on its poll. A
//!   read an app refuses while it launches (no process in front yet, a
//!   snapshot the runtime refused) is retried for a few seconds.
//! - The event stream carries the display's frames with the hash that
//!   leaves the status bar out, the mirror's change signal. The
//!   subscription only hashes; screenshots and encoded frames are taken
//!   from the display when asked for, at the size asked for.
//! - Input goes through the host's transport (dtuhidd, else Indigo) in
//!   points; iOS key codes are keyboard usages or consumer-page buttons.
//!   Text for a field taps it, waits for its focus and, unless keys were
//!   asked for, selects what it holds so the text replaces it, deleting it
//!   for an empty text. The input
//!   session can start ahead of the first input, which then does not wait
//!   for dtuhidd to activate.
//! - Apps launch with their launch environment, and a running app comes to
//!   the front unless a relaunch is asked for. The daemon notes a launch or
//!   a stop to the mirror as a change of the window in front, as it does
//!   for every driver.
//! - Dropping the attachment resets the target in the driver, which stops
//!   its reader and closes its input session.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use mirror::backend::{
    Backend, BackendError, BackendInfo, EncodeRequest, EventStream, FrameMeta, FrameSource, Input,
    InputAck, InputSink, NodeHandle, ReadRequest, Release, StreamItem, SystemClock, Timed,
    TreeRead, TreeSource,
};
use mirror::{MirrorConfig, Point};
use protocol::message::{Rotation, TextMethod as WireMethod};
use protocol::snapshot::{Node, NodeFlags, Snapshot, WindowType};
use serde_json::{Map, Value, json};
use simulator::{
    Button, Compatibility, DeviceInfo, DeviceState, Dispatch, ForegroundTree, FrameOptions,
    FrameStream, Frames as _, ImageFormat, Input as _, Key, LaunchOptions, Lifecycle as _,
    Modifiers, ReaderStatus, ScreenshotOptions, SimulatorDriver, Status, Swipe, TargetStatus,
    TextMethod, TransportKind, Tree as _, TreeOptions,
};

use crate::driver::{
    AppControl, AttachOptions, Attachment, Capture, CaptureRequest, CompatibilityRow, Driver,
    DriverError, FixedScreen, Image, InputNotes, InputSession, Launch, Platform, Report, Size,
    TargetClass, TargetInfo, TargetReport,
};
use crate::handlers::millis;
use crate::wire::code::{APP_NOT_FOUND, UNKNOWN_KEY};

/// The driver's name, as `status`, `doctor` and `--drivers` give it.
pub(crate) const NAME: &str = "simulator";

/// How long reads retry the failures a launching app causes (no process
/// in front yet, a snapshot the runtime refused): an app answers 1.5 to
/// 2 s after its launch.
const READ_BUDGET: Duration = Duration::from_secs(5);

/// The pause between two tries of a read.
const READ_RETRY: Duration = Duration::from_millis(100);

/// How long text waits for the field it tapped to take the focus.
const FOCUS_WAIT: Duration = Duration::from_millis(1500);

/// How long the finger of a swipe released resting rests at the end of its
/// path before it creeps on and lifts (task 10.16; the creep is the
/// Simulator driver's, see `simulator::gesture::swipe`). On the iOS 26.5
/// Simulator, 600-point swipes over 300 ms released this way moved the
/// fixtures' lists 543 to 573 points, where swipes that lifted moving moved
/// the Expo fixture's 1,522 to 1,608.
const REST: Duration = Duration::from_millis(60);

/// The Simulator driver's swipe from `from` to `to` over `duration`,
/// released as `release` says.
fn swipe(from: Point, to: Point, duration: Duration, release: Release) -> Swipe {
    Swipe {
        rest: match release {
            Release::Moving => None,
            Release::Resting => Some(REST),
        },
        ..Swipe::new(point(from), point(to), duration)
    }
}

/// The usage of the `a` key, which selects everything with Command held.
const USAGE_A: u32 = 0x04;

/// The usage of the delete key, Backspace, which deletes the selection.
const USAGE_DELETE: u32 = 0x2A;

const KEYBOARD_PAGE: u32 = 0x07;
const CONSUMER_PAGE: u32 = 0x0C;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The Simulators of the default device set.
pub(crate) struct SimulatorDevices {
    /// The driver, opened on first use: loading CoreSimulator costs about
    /// 330 ms, which a daemon that never lists a Simulator does not pay.
    driver: Mutex<Option<Arc<SimulatorDriver>>>,
}

impl SimulatorDevices {
    pub(crate) fn new() -> SimulatorDevices {
        SimulatorDevices {
            driver: Mutex::new(None),
        }
    }

    fn driver(&self) -> Result<Arc<SimulatorDriver>, DriverError> {
        let mut slot = lock(&self.driver);
        if let Some(driver) = slot.as_ref() {
            return Ok(Arc::clone(driver));
        }
        let driver = Arc::new(SimulatorDriver::open().map_err(|e| error(&e))?);
        *slot = Some(Arc::clone(&driver));
        Ok(driver)
    }
}

impl Driver for SimulatorDevices {
    fn name(&self) -> &str {
        NAME
    }

    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError> {
        let devices = self.driver()?.devices().map_err(|e| error(&e))?;
        Ok(devices
            .into_iter()
            .filter(|device| is_target(device.state))
            .map(target_info)
            .collect())
    }

    fn attach(
        &self,
        target: &TargetInfo,
        _options: &AttachOptions,
    ) -> Result<Attachment, DriverError> {
        attach(self.driver()?, &target.id)
    }

    fn compatibility(&self) -> Vec<CompatibilityRow> {
        match self
            .driver()
            .and_then(|driver| driver.compatibility().map_err(|e| error(&e)))
        {
            Ok(table) => doctor_rows(&table),
            Err(failure) => vec![CompatibilityRow {
                subject: "Simulator driver".to_owned(),
                feature: "CoreSimulator of the selected Xcode".to_owned(),
                status: "unsupported".to_owned(),
                note: Some(failure.message),
            }],
        }
    }
}

/// What the parts of one target's backend share.
#[derive(Default)]
struct Shared {
    /// The latest read and its scale, where text finds the field a handle
    /// names.
    last: Mutex<Option<(Snapshot, f64)>>,
    /// The latest frame the event stream delivered.
    latest: Mutex<Option<FrameMeta>>,
    frames: AtomicU64,
    /// Why the frame subscription could not start.
    frame_error: Mutex<Option<String>>,
    /// How long the last input waited for its transport to activate.
    activation: Mutex<Option<Duration>>,
    /// Whether the input session was started ahead of any input, so the
    /// next input is its first.
    started_ahead: AtomicBool,
}

impl Shared {
    fn remember(&self, tree: &ForegroundTree) {
        *lock(&self.last) = Some((tree.snapshot.clone(), tree.scale));
    }

    fn center(&self, handle: NodeHandle) -> Option<Point> {
        let last = lock(&self.last);
        let (snapshot, scale) = last.as_ref()?;
        node_center(snapshot, *scale, handle)
    }
}

/// Resets the target in the driver when dropped, which stops its reader and
/// closes its input session.
struct Detach {
    driver: Arc<SimulatorDriver>,
    udid: String,
}

impl Drop for Detach {
    fn drop(&mut self) {
        let _ = self.driver.reset_target(&self.udid);
    }
}

fn attach(driver: Arc<SimulatorDriver>, udid: &str) -> Result<Attachment, DriverError> {
    let udid = udid.to_owned();
    // Created first, so a failure below stops what the attach started.
    let detach = Detach {
        driver: Arc::clone(&driver),
        udid: udid.clone(),
    };
    let shared = Arc::new(Shared::default());
    let options = FrameOptions {
        encode: false,
        ..FrameOptions::default()
    };
    let stream = match driver.frames(&udid, &options) {
        Ok(stream) => Some(stream),
        Err(failure) => {
            *lock(&shared.frame_error) = Some(failure.to_string());
            None
        }
    };
    // The first read spawns the reader and gives the screen's size.
    let first = read(&driver, &udid)?;
    let display = display_of(&first).ok_or_else(|| {
        DriverError::new(
            "no_display",
            format!("the first read of {udid} named no window to take the screen size from"),
        )
    })?;
    shared.remember(&first);
    let frames = stream.is_some().then(|| {
        Box::new(DisplayFrames {
            driver: Arc::clone(&driver),
            udid: udid.clone(),
            shared: Arc::clone(&shared),
        }) as Box<dyn FrameSource>
    });
    Ok(Attachment {
        backend: Backend {
            tree: Box::new(ReaderTree {
                driver: Arc::clone(&driver),
                udid: udid.clone(),
                shared: Arc::clone(&shared),
            }),
            events: Box::new(DisplayEvents {
                stream,
                shared: Arc::clone(&shared),
            }),
            frames,
            input: Box::new(HostInput {
                driver: Arc::clone(&driver),
                udid: udid.clone(),
                shared: Arc::clone(&shared),
            }),
            clock: Box::new(SystemClock),
        },
        apps: Box::new(Apps {
            driver: Arc::clone(&driver),
            udid: udid.clone(),
            _detach: detach,
        }),
        display: Box::new(FixedScreen(display)),
        config: MirrorConfig::default(),
        capture: Some(Box::new(Screenshots {
            driver: Arc::clone(&driver),
            udid: udid.clone(),
        })),
        report: Some(Arc::new(StatusReport {
            driver: Arc::clone(&driver),
            udid: udid.clone(),
            shared: Arc::clone(&shared),
            plan: OnceLock::new(),
        })),
        input_notes: Some(Arc::new(Activations(Arc::clone(&shared)))),
        input_session: Some(Box::new(Session {
            driver,
            udid,
            shared,
        })),
    })
}

/// Reads the foreground's tree, retrying for [`READ_BUDGET`] the failures
/// a launching app causes.
fn read(driver: &SimulatorDriver, udid: &str) -> Result<ForegroundTree, DriverError> {
    let deadline = Instant::now() + READ_BUDGET;
    loop {
        match driver.snapshot(udid, &TreeOptions::default()) {
            Ok(tree) => return Ok(tree),
            Err(failure) if failure.retryable() && Instant::now() + READ_RETRY < deadline => {
                thread::sleep(READ_RETRY);
            }
            Err(failure) => return Err(error(&failure)),
        }
    }
}

struct ReaderTree {
    driver: Arc<SimulatorDriver>,
    udid: String,
    shared: Arc<Shared>,
}

impl TreeSource for ReaderTree {
    /// Every read covers the whole foreground: the owner's window and the
    /// keyboard's.
    fn read(&mut self, _request: &ReadRequest) -> Result<TreeRead, BackendError> {
        let tree = read(&self.driver, &self.udid)?;
        self.shared.remember(&tree);
        Ok(TreeRead {
            backend: BackendInfo::new(tree.backend.name(), tree.backend.fallback()),
            scale: tree.scale,
            snapshot: tree.snapshot,
        })
    }
}

struct DisplayEvents {
    /// `None` when the display could not be subscribed to.
    stream: Option<FrameStream>,
    shared: Arc<Shared>,
}

impl EventStream for DisplayEvents {
    fn next(&mut self, deadline: Instant) -> Option<Timed<StreamItem>> {
        let wait = deadline.saturating_duration_since(Instant::now());
        let frame = self.stream.as_ref().and_then(|stream| stream.next(wait));
        let Some(frame) = frame else {
            // A stream that ended, or none at all, still waits out the
            // deadline, as the mirror expects.
            thread::sleep(deadline.saturating_duration_since(Instant::now()));
            return None;
        };
        let meta = FrameMeta {
            seq: frame.sequence,
            at: frame.captured,
            hash: frame.content_hash,
            width: frame.width,
            height: frame.height,
            rotation: Rotation::R0,
        };
        *lock(&self.shared.latest) = Some(meta);
        self.shared.frames.fetch_add(1, Ordering::Relaxed);
        Some(Timed {
            at: Instant::now(),
            item: StreamItem::Frame(meta),
        })
    }
}

struct DisplayFrames {
    driver: Arc<SimulatorDriver>,
    udid: String,
    shared: Arc<Shared>,
}

impl FrameSource for DisplayFrames {
    fn latest(&self) -> Option<FrameMeta> {
        *lock(&self.shared.latest)
    }

    /// The subscription only hashes, so the display is read again at the
    /// size asked for: the frame on screen now, which a still display
    /// leaves equal to the latest delivered.
    fn encode(&mut self, request: &EncodeRequest) -> Result<Vec<u8>, BackendError> {
        let options = ScreenshotOptions {
            max_dimension: request.width.max(request.height),
            quality: request.quality,
            format: ImageFormat::Jpeg,
        };
        self.driver
            .screenshot(&self.udid, &options)
            .map(|shot| shot.data)
            .map_err(|e| error(&e))
    }
}

struct HostInput {
    driver: Arc<SimulatorDriver>,
    udid: String,
    shared: Arc<Shared>,
}

fn point(at: Point) -> simulator::Point {
    simulator::Point::new(at.x, at.y)
}

impl HostInput {
    /// The input went in: notes how long it waited for its transport to
    /// activate, no time at all for the first input of a session started
    /// ahead of it, whose effects can come as late.
    fn accepted(&self, dispatch: simulator::Result<Dispatch>) -> Result<InputAck, BackendError> {
        let dispatch = dispatch.map_err(|e| error(&e))?;
        let first = self.shared.started_ahead.swap(false, Ordering::SeqCst);
        *lock(&self.shared.activation) =
            dispatch.activation_wait.or(first.then_some(Duration::ZERO));
        Ok(InputAck {
            accepted: true,
            method: None,
        })
    }
}

impl InputSink for HostInput {
    fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError> {
        *lock(&self.shared.activation) = None;
        let (driver, udid) = (&self.driver, self.udid.as_str());
        match input {
            Input::Tap { at, hold: None } => self.accepted(driver.tap(udid, point(*at))),
            Input::Tap {
                at,
                hold: Some(hold),
            } => self.accepted(driver.press(udid, point(*at), *hold)),
            Input::Swipe {
                from,
                to,
                duration,
                release,
            } => self.accepted(driver.swipe(udid, &swipe(*from, *to, *duration, *release))),
            Input::Key { code, meta } => match ios_key(*code, *meta)? {
                IosKey::Key(key) => self.accepted(driver.key(udid, key)),
                IosKey::Button(button) => self.accepted(driver.button(udid, button)),
            },
            Input::Text { text, node, method } => self.text(text, *node, *method),
        }
    }
}

impl HostInput {
    /// Enters `text` into the field `node` names, tapping it first, or into
    /// the focused field. Unless keys were asked for, what the field holds
    /// is selected first so the text replaces it, and deleted for an empty
    /// text.
    fn text(
        &mut self,
        text: &str,
        node: Option<NodeHandle>,
        method: WireMethod,
    ) -> Result<InputAck, BackendError> {
        let (driver, udid) = (&self.driver, self.udid.as_str());
        if let Some(handle) = node {
            let center = self.shared.center(handle).ok_or_else(|| {
                BackendError::new(
                    mirror::code::STALE_REF,
                    "the field is not in the latest read of the screen; take a snapshot",
                )
            })?;
            driver.tap(udid, point(center)).map_err(|e| error(&e))?;
            self.wait_for_focus();
            for key in readying_keys(text, method) {
                driver.key(udid, key).map_err(|e| error(&e))?;
            }
        }
        // An empty text leaves nothing to type or paste once the field is
        // readied.
        let entered = if text.is_empty() {
            TextMethod::Keyboard
        } else {
            driver
                .type_text(udid, text, text_method(method))
                .map_err(|e| error(&e))?
                .method
        };
        Ok(InputAck {
            accepted: true,
            method: Some(wire_method(entered)),
        })
    }

    /// Reads the screen until an editable node has the focus, at most
    /// [`FOCUS_WAIT`]: the tap that gave it has to reach the app first.
    fn wait_for_focus(&self) {
        let deadline = Instant::now() + FOCUS_WAIT;
        while Instant::now() < deadline {
            if let Ok(tree) = self.driver.snapshot(&self.udid, &TreeOptions::default()) {
                self.shared.remember(&tree);
                if has_focused_field(&tree.snapshot) {
                    return;
                }
            }
            thread::sleep(READ_RETRY);
        }
    }
}

/// The keys that ready a field text tapped for `text`, entered as `method`
/// asks: unless keys were asked for, Command-A selects what the field holds,
/// so the text replaces it, and for an empty text, which types or pastes
/// nothing, the delete key then deletes what it selected.
fn readying_keys(text: &str, method: WireMethod) -> Vec<Key> {
    if method == WireMethod::KEYS {
        return Vec::new();
    }
    let select_all = Key {
        usage: USAGE_A,
        modifiers: Modifiers {
            command: true,
            ..Modifiers::default()
        },
    };
    if text.is_empty() {
        vec![select_all, Key::usage(USAGE_DELETE)]
    } else {
        vec![select_all]
    }
}

/// Whether an application window of `snapshot` has a focused editable node.
fn has_focused_field(snapshot: &Snapshot) -> bool {
    let focused = NodeFlags::FOCUSED | NodeFlags::EDITABLE;
    snapshot
        .windows
        .iter()
        .filter(|window| window.kind != WindowType::INPUT_METHOD)
        .filter_map(|window| window.root.as_ref())
        .any(|root| find(root, |node| node.flags.contains(focused)).is_some())
}

/// The first node under `root`, in pre-order, that `wanted` accepts.
fn find(root: &Node, wanted: impl Fn(&Node) -> bool) -> Option<&Node> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if wanted(node) {
            return Some(node);
        }
        stack.extend(node.children.iter().rev());
    }
    None
}

/// The Simulator's input session, which the driver opens on the first
/// input or ahead of it (`open_input`) and dtuhidd activates a while later
/// (9 ms once dtuhidd runs, 270 to 330 ms for the first session of a boot,
/// spike S3).
struct Session {
    driver: Arc<SimulatorDriver>,
    udid: String,
    shared: Arc<Shared>,
}

impl InputSession for Session {
    /// Opens the session without any input, which sends dtuhidd its
    /// activation probe, and returns without waiting for the reply.
    fn start(&mut self) -> Result<(), DriverError> {
        let status = self.driver.status(&self.udid).map_err(|e| error(&e))?;
        if status.input.is_some() {
            return Ok(());
        }
        self.driver
            .open_input(&self.udid, Duration::ZERO)
            .map_err(|e| error(&e))?;
        self.shared.started_ahead.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Active once dtuhidd answered its probe; Indigo needs no activation.
    fn active(&mut self) -> bool {
        self.driver.status(&self.udid).is_ok_and(|status| {
            status.input.is_some_and(|input| {
                input.activation.is_some() || input.transport != TransportKind::Dtuhid
            })
        })
    }
}

/// What the Simulator's input transport said about the last input.
struct Activations(Arc<Shared>);

impl InputNotes for Activations {
    fn activation_wait(&self) -> Option<Duration> {
        *lock(&self.0.activation)
    }
}

struct Apps {
    driver: Arc<SimulatorDriver>,
    udid: String,
    _detach: Detach,
}

impl AppControl for Apps {
    fn launch(&mut self, app: &str, launch: &Launch) -> Result<(), DriverError> {
        let options = LaunchOptions {
            arguments: Vec::new(),
            environment: launch.env.clone(),
            terminate_running: launch.relaunch,
        };
        match self.driver.launch(&self.udid, app, &options) {
            Ok(_) => Ok(()),
            Err(failure) if installed(&self.udid, app) == Some(false) => Err(DriverError::new(
                APP_NOT_FOUND,
                format!(
                    "{app} is not installed on Simulator {}; install it with xcrun simctl install ({failure})",
                    self.udid
                ),
            )),
            Err(failure) => Err(error(&failure)),
        }
    }

    fn terminate(&mut self, app: &str) -> Result<(), DriverError> {
        self.driver
            .terminate(&self.udid, app)
            .map_err(|e| error(&e))
    }
}

/// Whether `app` is installed on the Simulator, as `simctl` finds its
/// container; `None` when `simctl` cannot say.
fn installed(udid: &str, app: &str) -> Option<bool> {
    Command::new("xcrun")
        .args(["simctl", "get_app_container", udid, app])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|status| status.success())
}

struct Screenshots {
    driver: Arc<SimulatorDriver>,
    udid: String,
}

impl Capture for Screenshots {
    fn screenshot(&mut self, request: &CaptureRequest) -> Result<Image, DriverError> {
        let options = ScreenshotOptions {
            max_dimension: request.max_dimension,
            quality: request.quality,
            format: ImageFormat::Jpeg,
        };
        let shot = self
            .driver
            .screenshot(&self.udid, &options)
            .map_err(|e| error(&e))?;
        Ok(Image {
            data: shot.data,
            width: shot.width,
            height: shot.height,
            format: "jpeg",
        })
    }
}

struct StatusReport {
    driver: Arc<SimulatorDriver>,
    udid: String,
    shared: Arc<Shared>,
    /// The host's input transports in the order the driver tries them,
    /// read from its compatibility table once asked for.
    plan: OnceLock<Option<String>>,
}

impl StatusReport {
    fn plan(&self) -> Option<&str> {
        self.plan
            .get_or_init(|| {
                let table = self.driver.compatibility().ok()?;
                table
                    .rows
                    .into_iter()
                    .find(|row| row.component == "input transport")
                    .map(|row| row.detected)
            })
            .as_deref()
    }
}

impl Report for StatusReport {
    fn report(&self) -> TargetReport {
        match self.driver.status(&self.udid) {
            Ok(status) => {
                let plan = if status.input.is_none() {
                    self.plan()
                } else {
                    None
                };
                let frame_error = lock(&self.shared.frame_error).clone();
                status_report(
                    &status,
                    plan,
                    self.shared.frames.load(Ordering::Relaxed),
                    frame_error.as_deref(),
                )
            }
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

/// What an iOS key code of `keys.rs` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IosKey {
    /// A key of the keyboard page, with its modifiers.
    Key(Key),
    /// A button of the consumer page.
    Button(Button),
}

/// Whether a Simulator in `state` is a target: booted ones only, since
/// only they run apps and take input.
pub(crate) fn is_target(state: DeviceState) -> bool {
    state == DeviceState::Booted
}

/// A Simulator as `devices` lists it.
pub(crate) fn target_info(device: DeviceInfo) -> TargetInfo {
    TargetInfo {
        id: device.udid,
        platform: Platform::Ios,
        class: TargetClass::Simulator,
        name: device.name,
        model: device.device_type.map(|device_type| device_type.name),
        os: device.runtime.map(|runtime| runtime.name),
        state: device.state.to_string().to_ascii_lowercase(),
        advice: None,
        driver: NAME.to_owned(),
    }
}

/// The key or button `code` names: `(page << 16) | usage`, keyboard keys
/// on page 7 (or no page) with `meta` holding the HID modifier byte, and
/// buttons on the consumer page 0x0C.
pub(crate) fn ios_key(code: u32, meta: Option<u32>) -> Result<IosKey, DriverError> {
    let (page, usage) = (code >> 16, code & 0xFFFF);
    let unknown = || {
        DriverError::new(
            UNKNOWN_KEY,
            format!(
                "{code:#x} is no key or button a Simulator takes: keyboard usages (page 7) and \
                 the consumer page's Home, Power, VolumeUp, VolumeDown and Siri"
            ),
        )
    };
    match page {
        0 | KEYBOARD_PAGE => Ok(IosKey::Key(Key {
            usage,
            modifiers: modifiers(meta.unwrap_or(0)),
        })),
        CONSUMER_PAGE => Ok(IosKey::Button(match usage {
            0x40 => Button::Home,
            0x30 => Button::Lock,
            0xE9 => Button::VolumeUp,
            0xEA => Button::VolumeDown,
            0xCF => Button::Siri,
            _ => return Err(unknown()),
        })),
        _ => Err(unknown()),
    }
}

/// The modifiers the HID modifier byte holds, the right-hand keys counting
/// as the left ones.
fn modifiers(meta: u32) -> Modifiers {
    let held = |left: u32| meta & (left | left << 4) != 0;
    Modifiers {
        control: held(0x01),
        shift: held(0x02),
        option: held(0x04),
        command: held(0x08),
    }
}

/// A Simulator driver error as the protocol carries it, with its code and
/// retryable flag.
pub(crate) fn error(error: &simulator::Error) -> DriverError {
    DriverError {
        code: error.code().to_owned(),
        message: error.to_string(),
        retryable: error.retryable(),
    }
}

/// The Simulator driver's compatibility table as `doctor`'s rows: the
/// host's components, then every private call and where it was validated.
pub(crate) fn doctor_rows(table: &Compatibility) -> Vec<CompatibilityRow> {
    let status = |status: Status| match status {
        Status::Validated => "validated",
        Status::Supported => "supported",
        Status::Limited => "limited",
        Status::Unsupported => "unsupported",
    };
    let components = table.rows.iter().map(|row| CompatibilityRow {
        subject: format!("{} {}", row.component, row.detected),
        feature: row.expected.clone(),
        status: status(row.status).to_owned(),
        note: (!row.note.is_empty()).then(|| row.note.clone()),
    });
    let calls = table.call_sites.iter().map(|site| CompatibilityRow {
        subject: site.validated_on.to_owned(),
        feature: format!("{} ({})", site.call, site.image),
        status: "validated".to_owned(),
        note: None,
    });
    components.chain(calls).collect()
}

fn reader_json(reader: &ReaderStatus) -> Value {
    json!({
        "name": reader.name,
        "pid": reader.pid,
        "path": reader.path.to_string_lossy(),
        "readyMs": reader.ready_in.map(millis),
        "state": reader.state.to_string(),
        "spawns": reader.spawns,
    })
}

/// What `status` shows about an attached Simulator: its input transport,
/// or the host's plan before the first input, how its tree is read and by
/// which reader, its frames, and what took it out of service.
pub(crate) fn status_report(
    status: &TargetStatus,
    plan: Option<&str>,
    frames: u64,
    frame_error: Option<&str>,
) -> TargetReport {
    let mut data = Map::new();
    let mut lines = Vec::new();
    match &status.input {
        Some(input) => {
            let mut line = format!("input: {} ({})", input.transport, input.reason);
            if let Some(activation) = input.activation {
                line.push_str(&format!(", activated in {:.1} ms", millis(activation)));
            }
            let plural = if input.connections == 1 { "" } else { "s" };
            line.push_str(&format!(", {} connection{plural}", input.connections));
            lines.push(line);
            data.insert("input".into(), input.transport.to_string().into());
            data.insert("inputReason".into(), input.reason.clone().into());
            data.insert("activationMs".into(), input.activation.map(millis).into());
            data.insert("connections".into(), input.connections.into());
        }
        None => {
            lines.push(match plan {
                Some(plan) => format!("input: {plan}, opened with the first input"),
                None => "input: opened with the first input".to_owned(),
            });
            data.insert("input".into(), Value::Null);
        }
    }
    data.insert("inputPlan".into(), plan.into());
    match &status.tree {
        Some(tree) => {
            let fallback = if tree.backend.fallback() {
                " (fallback)"
            } else {
                ""
            };
            lines.push(format!("tree: {}{fallback}: {}", tree.backend, tree.reason));
            if let Some(reader) = &tree.reader {
                let ready = reader.ready_in.map_or_else(
                    || "not ready".to_owned(),
                    |ready| format!("ready {:.1} ms after its spawn", millis(ready)),
                );
                let spawns = if reader.spawns == 1 {
                    "spawn"
                } else {
                    "spawns"
                };
                lines.push(format!(
                    "reader: {}, pid {}, {}, {ready}, {} {spawns}, from {}",
                    reader.name,
                    reader.pid,
                    reader.state,
                    reader.spawns,
                    reader.path.display()
                ));
            }
            data.insert("tree".into(), tree.backend.name().into());
            data.insert("treeReason".into(), tree.reason.clone().into());
            data.insert(
                "reader".into(),
                tree.reader.as_ref().map_or(Value::Null, reader_json),
            );
        }
        None => {
            lines.push("tree: not read yet".to_owned());
            data.insert("tree".into(), Value::Null);
            data.insert("reader".into(), Value::Null);
        }
    }
    let mut line = format!("frames: {frames} received from the display");
    if let Some(failure) = frame_error {
        line.push_str(&format!(
            "; the display could not be subscribed to: {failure}"
        ));
    }
    lines.push(line);
    data.insert("framesReceived".into(), frames.into());
    data.insert("frameError".into(), frame_error.into());
    match &status.unavailable {
        Some(diagnostics) => {
            lines.push(format!(
                "unavailable: {} {} (with {})",
                diagnostics.call, diagnostics.failure, diagnostics.host
            ));
            data.insert(
                "unavailable".into(),
                json!({
                    "call": diagnostics.call,
                    "failure": diagnostics.failure,
                    "host": diagnostics.host,
                }),
            );
        }
        None => {
            data.insert("unavailable".into(), Value::Null);
        }
    }
    TargetReport { lines, data }
}

/// The screen in points: the largest window of a read over its scale.
pub(crate) fn display_of(tree: &ForegroundTree) -> Option<Size> {
    let area = |b: &protocol::snapshot::Bounds| {
        i64::from(b.right - b.left).max(0) * i64::from(b.bottom - b.top).max(0)
    };
    let window = tree
        .snapshot
        .windows
        .iter()
        .max_by_key(|window| area(&window.bounds))?;
    let bounds = window.bounds;
    let width = f64::from(bounds.right - bounds.left);
    let height = f64::from(bounds.bottom - bounds.top);
    (width > 0.0 && height > 0.0 && tree.scale > 0.0).then(|| Size {
        width: width / tree.scale,
        height: height / tree.scale,
    })
}

/// The middle of the node `handle` names in `snapshot`, in points.
pub(crate) fn node_center(snapshot: &Snapshot, scale: f64, handle: NodeHandle) -> Option<Point> {
    let window = snapshot
        .windows
        .iter()
        .find(|window| window.id == handle.window)?;
    let node = find(window.root.as_ref()?, |node| node.id == handle.node)?;
    let bounds = node.bounds;
    Some(Point::new(
        f64::from(bounds.left + bounds.right) / 2.0 / scale,
        f64::from(bounds.top + bounds.bottom) / 2.0 / scale,
    ))
}

/// How text goes into a field for a request's method: key events when it
/// asks for keys, a paste, which sets the text in one operation, when it
/// asks for direct setting, the driver's choice otherwise.
pub(crate) fn text_method(method: WireMethod) -> TextMethod {
    if method == WireMethod::KEYS {
        TextMethod::Keyboard
    } else if method == WireMethod::DIRECT {
        TextMethod::Paste
    } else {
        TextMethod::Auto
    }
}

/// The method a reply names for how the driver entered text: a paste sets
/// the text in one operation, as direct setting does.
pub(crate) fn wire_method(method: TextMethod) -> WireMethod {
    match method {
        TextMethod::Keyboard => WireMethod::KEYS,
        TextMethod::Paste => WireMethod::DIRECT,
        TextMethod::Auto => WireMethod::AUTO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    use protocol::snapshot::{
        Bounds, Node, NodeFlags, Truncation, Window, WindowFlags, WindowType,
    };
    use simulator::{
        CallSite, DeviceTypeInfo, InputStatus, Modifiers, ReaderState, ReaderStatus, Row,
        RuntimeInfo, Status, TransportKind, TreeBackend, TreeFailure, TreeStatus,
    };

    fn device(state: DeviceState) -> DeviceInfo {
        DeviceInfo {
            udid: "7B292E1E-7A29-41A5-9C69-2ABA1CEB3683".into(),
            name: "mobile-agent-spike".into(),
            state,
            runtime: Some(RuntimeInfo {
                name: "iOS 26.5".into(),
                version: "26.5".into(),
                build: "23F77".into(),
                identifier: "com.apple.CoreSimulator.SimRuntime.iOS-26-5".into(),
            }),
            device_type: Some(DeviceTypeInfo {
                name: "iPhone 17".into(),
                identifier: "com.apple.CoreSimulator.SimDeviceType.iPhone-17".into(),
            }),
        }
    }

    /// Task 10.16: a swipe released resting rests 60 ms at its end, which
    /// the Simulator driver follows with its creep; one released moving
    /// lifts at once.
    #[test]
    fn a_swipe_released_resting_rests_60_ms_before_the_finger_lifts() {
        let (from, to) = (Point::new(201.0, 700.0), Point::new(201.0, 150.0));
        let duration = Duration::from_millis(300);
        let moving = swipe(from, to, duration, Release::Moving);
        assert_eq!(moving.rest, None);
        assert_eq!(moving.duration, duration);
        let resting = swipe(from, to, duration, Release::Resting);
        assert_eq!(resting.rest, Some(Duration::from_millis(60)));
        assert_eq!((resting.from.y, resting.to.y), (700.0, 150.0));
    }

    #[test]
    fn booted_simulators_are_listed_with_their_runtime_model_and_state() {
        assert!(is_target(DeviceState::Booted));
        for state in [
            DeviceState::Shutdown,
            DeviceState::Booting,
            DeviceState::ShuttingDown,
            DeviceState::Creating,
        ] {
            assert!(!is_target(state), "{state}");
        }
        let target = target_info(device(DeviceState::Booted));
        assert_eq!(target.id, "7B292E1E-7A29-41A5-9C69-2ABA1CEB3683");
        assert_eq!(target.platform, crate::driver::Platform::Ios);
        assert_eq!(target.class, crate::driver::TargetClass::Simulator);
        assert_eq!(target.name, "mobile-agent-spike");
        assert_eq!(target.model.as_deref(), Some("iPhone 17"));
        assert_eq!(target.os.as_deref(), Some("iOS 26.5"));
        assert_eq!(target.state, "booted");
        assert_eq!(target.advice, None);
        assert_eq!(target.driver, NAME);
    }

    #[test]
    fn keyboard_codes_become_keys_and_consumer_codes_buttons() {
        let key = |usage, modifiers| IosKey::Key(Key { usage, modifiers });
        assert_eq!(
            ios_key(0x07 << 16 | 0x28, None).unwrap(),
            key(0x28, Modifiers::default())
        );
        assert_eq!(
            ios_key(0x04, Some(0x08 | 0x02)).unwrap(),
            key(
                0x04,
                Modifiers {
                    shift: true,
                    command: true,
                    ..Modifiers::default()
                }
            ),
            "a code without a page is a keyboard usage"
        );
        assert_eq!(
            ios_key(0x07 << 16 | 0x04, Some(0x01 | 0x40)).unwrap(),
            key(
                0x04,
                Modifiers {
                    control: true,
                    option: true,
                    ..Modifiers::default()
                }
            ),
            "right-hand modifiers count as the left ones"
        );
        let button = |code: u32| ios_key(0x0C << 16 | code, None).unwrap();
        assert_eq!(button(0x40), IosKey::Button(Button::Home));
        assert_eq!(button(0x30), IosKey::Button(Button::Lock));
        assert_eq!(button(0xE9), IosKey::Button(Button::VolumeUp));
        assert_eq!(button(0xEA), IosKey::Button(Button::VolumeDown));
        for unknown in [0x0C << 16 | 0x99, 0x09 << 16 | 0x01] {
            let refused = ios_key(unknown, None).unwrap_err();
            assert_eq!(refused.code, "unknown_key", "{refused:?}");
        }
    }

    #[test]
    fn driver_errors_keep_their_code_and_retryable_flag() {
        let early = error(&simulator::Error::Tree {
            kind: TreeFailure::NoForeground,
            message: "starting".into(),
        });
        assert_eq!(early.code, "no_foreground");
        assert!(early.retryable);
        assert!(early.message.contains("starting"), "{early:?}");
        let gone = error(&simulator::Error::NotFound { target: "X".into() });
        assert_eq!(gone.code, "not_found");
        assert!(!gone.retryable);
    }

    #[test]
    fn the_compatibility_table_becomes_doctor_rows_with_every_private_call() {
        let table = Compatibility {
            rows: vec![Row {
                component: "Xcode".into(),
                detected: "27.0 (27A266a)".into(),
                expected: "Xcode 26.0 up to but not including 28".into(),
                status: Status::Validated,
                note: "validated on Xcode 27.0".into(),
            }],
            call_sites: vec![CallSite {
                call: "-[SimDevice lookup:error:]",
                image: "CoreSimulator",
                validated_on: "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: spike S3",
            }],
        };
        let rows = doctor_rows(&table);
        assert_eq!(
            rows,
            [
                CompatibilityRow {
                    subject: "Xcode 27.0 (27A266a)".into(),
                    feature: "Xcode 26.0 up to but not including 28".into(),
                    status: "validated".into(),
                    note: Some("validated on Xcode 27.0".into()),
                },
                CompatibilityRow {
                    subject: "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: spike S3".into(),
                    feature: "-[SimDevice lookup:error:] (CoreSimulator)".into(),
                    status: "validated".into(),
                    note: None,
                },
            ]
        );
    }

    fn status(input: Option<InputStatus>) -> TargetStatus {
        TargetStatus {
            device: device(DeviceState::Booted),
            input,
            tree: Some(TreeStatus {
                backend: TreeBackend::Reader,
                reason: "simulator-reader runs in the Simulator".into(),
                reader: Some(ReaderStatus {
                    name: "simulator-reader 0.1.0".into(),
                    path: PathBuf::from("/cache/simulator-reader-00ff"),
                    pid: 4242,
                    spawned_at: SystemTime::UNIX_EPOCH,
                    ready_in: Some(Duration::from_micros(292_400)),
                    state: ReaderState::Ready,
                    spawns: 1,
                }),
            }),
            unavailable: None,
        }
    }

    /// Spec scenarios "Two runtimes on one host" and "Spawned helper is
    /// reported" (simulator-driver): the status names the transport and
    /// the reader with its readiness time.
    #[test]
    fn the_status_names_the_transport_and_the_reader_with_its_readiness() {
        let report = status_report(
            &status(Some(InputStatus {
                transport: TransportKind::Dtuhid,
                activation: Some(Duration::from_micros(9_100)),
                connections: 1,
                reason: "CoreSimulator 1171.7 is 1155.4 or later".into(),
            })),
            Some("dtuhidd, then Indigo"),
            34,
            None,
        );
        let data = serde_json::Value::Object(report.data.clone());
        assert_eq!(data["input"], "dtuhidd", "{data}");
        assert_eq!(data["activationMs"], 9.1, "{data}");
        assert_eq!(data["connections"], 1, "{data}");
        assert_eq!(data["tree"], "simulator-reader", "{data}");
        assert_eq!(data["reader"]["pid"], 4242, "{data}");
        assert_eq!(data["reader"]["readyMs"], 292.4, "{data}");
        assert_eq!(data["reader"]["state"], "ready", "{data}");
        assert_eq!(data["framesReceived"], 34, "{data}");
        let text = report.lines.join("\n");
        assert!(
            text.contains("input: dtuhidd") && text.contains("1155.4"),
            "{text}"
        );
        assert!(
            text.contains("simulator-reader 0.1.0") && text.contains("292.4 ms"),
            "{text}"
        );
        assert!(text.contains("frames: 34"), "{text}");

        let before = status_report(&status(None), Some("dtuhidd, then Indigo"), 0, Some("gone"));
        let data = serde_json::Value::Object(before.data.clone());
        assert_eq!(data["input"], serde_json::Value::Null, "{data}");
        assert_eq!(data["inputPlan"], "dtuhidd, then Indigo", "{data}");
        let text = before.lines.join("\n");
        assert!(
            text.contains("input: dtuhidd, then Indigo") && text.contains("gone"),
            "{text}"
        );
    }

    fn node(id: u32, bounds: Bounds, children: Vec<Node>) -> Node {
        Node {
            id,
            class: 0,
            res_pkg: None,
            res_id: None,
            text: None,
            desc: None,
            bounds,
            flags: NodeFlags::ENABLED | NodeFlags::VISIBLE,
            children,
            scroll: None,
        }
    }

    fn window(id: u32, kind: WindowType, bounds: Bounds, root: Node) -> Window {
        Window {
            id,
            kind,
            layer: 0,
            package: None,
            title: Some("Fixture".into()),
            bounds,
            flags: WindowFlags::default(),
            root: Some(root),
            emits_events: Some(false),
            owner: None,
            modal: false,
        }
    }

    fn read() -> Snapshot {
        let screen = Bounds::new(0, 0, 1206, 2622);
        Snapshot {
            strings: vec!["XCUIElementTypeButton".into()],
            windows: vec![
                window(
                    7,
                    WindowType::INPUT_METHOD,
                    Bounds::new(0, 1700, 1206, 2622),
                    node(1, Bounds::new(0, 1700, 1206, 2622), Vec::new()),
                ),
                window(
                    4242,
                    WindowType::APPLICATION,
                    screen,
                    node(
                        2,
                        screen,
                        vec![node(3, Bounds::new(48, 534, 274, 637), Vec::new())],
                    ),
                ),
            ],
            node_count: 3,
            truncated: Truncation::default(),
            seq: 0,
        }
    }

    #[test]
    fn the_display_is_the_largest_window_in_points() {
        let tree = ForegroundTree::new(
            read(),
            3.0,
            TreeBackend::Reader,
            None,
            Duration::from_millis(14),
        )
        .expect("a foreground");
        assert_eq!(
            display_of(&tree),
            Some(Size {
                width: 402.0,
                height: 874.0
            })
        );
    }

    #[test]
    fn a_node_handle_is_found_in_the_read_that_numbered_it() {
        let snapshot = read();
        let center = |window, node| node_center(&snapshot, 3.0, NodeHandle { window, node });
        assert_eq!(center(4242, 3), Some(Point::new(161.0 / 3.0, 585.5 / 3.0)));
        assert_eq!(center(7, 3), None, "the node is in another window");
        assert_eq!(center(4242, 9), None);
    }

    /// Task 10.23: text replaces what a field holds, so an empty text, for
    /// which nothing is typed or pasted, deletes what Command-A selected;
    /// keys add to the field and ready nothing.
    #[test]
    fn an_empty_text_deletes_what_the_field_held() {
        let select_all = Key {
            usage: USAGE_A,
            modifiers: Modifiers {
                command: true,
                ..Modifiers::default()
            },
        };
        let delete = Key::usage(USAGE_DELETE);
        for method in [WireMethod::AUTO, WireMethod::DIRECT] {
            assert_eq!(readying_keys("agent", method), [select_all], "{method:?}");
            assert_eq!(
                readying_keys("", method),
                [select_all, delete],
                "{method:?}"
            );
        }
        assert_eq!(readying_keys("agent", WireMethod::KEYS), []);
        assert_eq!(readying_keys("", WireMethod::KEYS), []);
    }

    /// Task 10.23: direct text goes in in one piece, so a field that took
    /// keys out of order takes it whole; a Simulator pastes it.
    #[test]
    fn keys_are_typed_direct_text_is_pasted_and_a_paste_reads_as_direct_setting() {
        assert_eq!(text_method(WireMethod::KEYS), TextMethod::Keyboard);
        assert_eq!(text_method(WireMethod::AUTO), TextMethod::Auto);
        assert_eq!(text_method(WireMethod::DIRECT), TextMethod::Paste);
        assert_eq!(wire_method(TextMethod::Keyboard), WireMethod::KEYS);
        assert_eq!(wire_method(TextMethod::Paste), WireMethod::DIRECT);
    }
}
