//! The values the driver's traits take and return. Nothing here depends on
//! Apple frameworks, so these types exist on every platform.

use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::tree::{TreeBackend, TreeChoice};

/// A device's lifecycle state, as `-[SimDevice state]` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceState {
    Creating,
    Shutdown,
    Booting,
    Booted,
    ShuttingDown,
    /// A value this driver does not know.
    Other(u64),
}

impl DeviceState {
    /// Maps CoreSimulator's `SimDeviceState` value.
    pub fn from_raw(raw: u64) -> DeviceState {
        match raw {
            0 => DeviceState::Creating,
            1 => DeviceState::Shutdown,
            2 => DeviceState::Booting,
            3 => DeviceState::Booted,
            4 => DeviceState::ShuttingDown,
            other => DeviceState::Other(other),
        }
    }
}

impl fmt::Display for DeviceState {
    /// The names `xcrun simctl list` prints.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceState::Creating => f.write_str("Creating"),
            DeviceState::Shutdown => f.write_str("Shutdown"),
            DeviceState::Booting => f.write_str("Booting"),
            DeviceState::Booted => f.write_str("Booted"),
            DeviceState::ShuttingDown => f.write_str("Shutting Down"),
            DeviceState::Other(raw) => write!(f, "in state {raw}"),
        }
    }
}

/// An installed Simulator runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeInfo {
    /// For example "iOS 26.5".
    pub name: String,
    /// For example "26.5".
    pub version: String,
    /// For example "23F77".
    pub build: String,
    /// For example "com.apple.CoreSimulator.SimRuntime.iOS-26-5".
    pub identifier: String,
}

/// A device type, such as an iPhone model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceTypeInfo {
    /// For example "iPhone 17".
    pub name: String,
    /// For example "com.apple.CoreSimulator.SimDeviceType.iPhone-17".
    pub identifier: String,
}

/// One Simulator of the default device set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub udid: String,
    pub name: String,
    pub state: DeviceState,
    /// `None` when the device's runtime is not installed on this host.
    pub runtime: Option<RuntimeInfo>,
    pub device_type: Option<DeviceTypeInfo>,
}

/// Arguments and environment for an application launch. The environment
/// reaches the process as given; `simctl`'s `SIMCTL_CHILD_` prefix is only
/// `simctl`'s own convention.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
    /// Terminate a running instance first, so the launch starts a new process
    /// that sees these arguments and this environment.
    pub terminate_running: bool,
}

impl LaunchOptions {
    pub fn new() -> LaunchOptions {
        LaunchOptions::default()
    }

    /// Adds one argument.
    pub fn argument(mut self, argument: impl Into<String>) -> LaunchOptions {
        self.arguments.push(argument.into());
        self
    }

    /// Adds one environment variable.
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> LaunchOptions {
        self.environment.push((name.into(), value.into()));
        self
    }

    /// Terminates a running instance before launching.
    pub fn terminating_running(mut self) -> LaunchOptions {
        self.terminate_running = true;
        self
    }
}

/// A launched application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Launched {
    pub pid: i32,
    /// From the launch call until CoreSimulator returned the process identifier.
    pub elapsed: Duration,
}

/// What `boot` did to a device's accessibility preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferencesOutcome {
    /// Some of the keys were missing or false, and all of them were written.
    Written,
    /// Every key was already set.
    AlreadySet,
    /// The device was not shut down, so its preferences belong to the guest
    /// and were left alone.
    Skipped,
}

/// What `boot` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootReport {
    /// The device was already booted and ready, so nothing was done.
    pub already_booted: bool,
    /// From the call until CoreSimulator's boot status turned terminal, which
    /// is when the device is usable, some time after its state reads Booted.
    pub elapsed: Duration,
    /// The accessibility preferences, written before the boot so that tree
    /// reads work from the first one (spike S4).
    pub accessibility: PreferencesOutcome,
}

/// How a frame subscription reads the display (design D7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameOptions {
    /// Reads at most this many frames a second, skipping the presents in
    /// between; 0 reads every present, up to the display's 60 Hz.
    pub max_fps: u32,
    /// The longer side of a delivered frame in pixels; 0 keeps the native
    /// size. 1000 gives 460x1000 on an iPhone 17.
    pub max_dimension: u32,
    /// JPEG quality, 1 to 100.
    pub quality: u8,
    /// Encodes delivered frames as JPEG. Without it a frame carries only its
    /// hashes, which is all settle needs.
    pub encode: bool,
}

impl Default for FrameOptions {
    /// 30 fps at 1000 pixels and quality 80: 12 percent of one core in S2.
    fn default() -> FrameOptions {
        FrameOptions {
            max_fps: 30,
            max_dimension: 1000,
            quality: 80,
            encode: true,
        }
    }
}

/// One frame of a subscription.
#[derive(Clone, Debug)]
pub struct Frame {
    /// 1 for the frame read at subscription, then one more per frame.
    pub sequence: u64,
    /// When the surface was read.
    pub captured_at: SystemTime,
    /// The same moment on the monotonic clock, for measuring latency.
    pub captured: Instant,
    pub width: u32,
    pub height: u32,
    /// The JPEG, when the subscription encodes.
    pub jpeg: Option<Vec<u8>>,
    /// Hash of every delivered pixel: two frames differ exactly when it does.
    pub hash: u64,
    /// Hash of a thumbnail of the frame below the status bar, for settle: the
    /// status bar clock does not change it (D8).
    pub content_hash: u64,
    /// The render server wrote the surface during the read, so the frame may
    /// be torn (S2). The present that did it causes a later read.
    pub torn: bool,
}

/// Counters of a frame subscription.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    /// Present callbacks from the display, whether or not pixels changed.
    pub presents: u64,
    /// Surface reads, which the throttle keeps below the presents.
    pub reads: u64,
    /// Frames delivered, the one read at subscription included.
    pub delivered: u64,
    /// Delivered frames that may be torn.
    pub torn: u64,
    /// Frames dropped because the receiver fell 16 frames behind; the oldest
    /// go first.
    pub dropped: u64,
    /// CPU time of the pipeline: the reading thread and the callbacks.
    pub cpu: Duration,
    /// Since the subscription started.
    pub elapsed: Duration,
    /// The JPEG encoder in use: "turbojpeg", "jpeg-encoder", or "none".
    pub encoder: &'static str,
}

/// The encoding of a screenshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    /// Baseline JPEG.
    Jpeg,
    /// Packed BGRA, four bytes a pixel and no row padding.
    Bgra,
}

/// How to take a screenshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenshotOptions {
    /// The longer side in pixels; 0, the default, keeps the native size.
    pub max_dimension: u32,
    /// JPEG quality, 1 to 100.
    pub quality: u8,
    pub format: ImageFormat,
}

impl Default for ScreenshotOptions {
    fn default() -> ScreenshotOptions {
        ScreenshotOptions {
            max_dimension: 0,
            quality: 90,
            format: ImageFormat::Jpeg,
        }
    }
}

/// The display, read on demand.
#[derive(Clone, Debug)]
pub struct Screenshot {
    pub captured_at: SystemTime,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub data: Vec<u8>,
}

/// A position on the upright screen in iOS points, from the top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }
}

/// The screen edge a touch starts from. iOS reads its system gestures, such
/// as the home indicator swipe, from this flag rather than the coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Edge {
    #[default]
    None,
    Top,
    Left,
    Bottom,
    Right,
}

/// A one-finger drag from `from` to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swipe {
    pub from: Point,
    pub to: Point,
    /// From touch-down to the end of the path; with the distance it sets
    /// the velocity. Without a rest the finger lifts there, moving, and the
    /// content glides on.
    pub duration: Duration,
    /// Positions sent between the two ends.
    pub steps: u32,
    pub edge: Edge,
    /// How long the finger rests at `to` before it creeps on and lifts, so
    /// the content stops where the finger left it (task 10.16); see
    /// [`crate::gesture::swipe`].
    pub rest: Option<Duration>,
}

impl Swipe {
    /// 20 steps, no edge and no rest, as in spike S3.
    pub fn new(from: Point, to: Point, duration: Duration) -> Swipe {
        Swipe {
            from,
            to,
            duration,
            steps: 20,
            edge: Edge::None,
            rest: None,
        }
    }
}

/// A gesture of one or two fingers, each following a path of points that is
/// walked at an even pace over `duration`.
#[derive(Clone, Debug, PartialEq)]
pub struct Gesture {
    pub fingers: Vec<Vec<Point>>,
    pub duration: Duration,
    pub steps: u32,
}

/// A hardware button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Home,
    Lock,
    VolumeUp,
    VolumeDown,
    Siri,
}

/// Modifier keys held around a key press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub option: bool,
    pub command: bool,
}

/// A key of the USB HID keyboard page (0x07).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub usage: u32,
    pub modifiers: Modifiers,
}

impl Key {
    /// A usage with no modifiers.
    pub fn usage(usage: u32) -> Key {
        Key {
            usage,
            modifiers: Modifiers::default(),
        }
    }

    /// The key that types `character` on a US layout, with Shift when it
    /// needs it; `None` for a character no key types.
    pub fn character(character: char) -> Option<Key> {
        let (usage, shift) = crate::keymap::usage_for(character)?;
        Some(Key {
            usage,
            modifiers: Modifiers {
                shift,
                ..Modifiers::default()
            },
        })
    }

    /// A key that types nothing: return, escape, delete, tab, space, up,
    /// down, left, right or home.
    pub fn named(name: &str) -> Option<Key> {
        crate::keymap::usage_for_name(name).map(Key::usage)
    }
}

/// The input transports (design D6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// The dtuhidd XPC service, from CoreSimulator 1155.4.
    Dtuhid,
    /// SimulatorKit's legacy Indigo HID client, before 1155.4.
    Indigo,
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TransportKind::Dtuhid => "dtuhidd",
            TransportKind::Indigo => "Indigo",
        })
    }
}

/// How one input request went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dispatch {
    pub transport: TransportKind,
    /// How long the input waited for the transport's activation reply, when
    /// it was requested before activation completed.
    pub activation_wait: Option<Duration>,
    /// From the request until the first event was handed to the transport.
    pub dispatched: Duration,
    /// From the request until the transport had sent every event.
    pub completed: Duration,
    /// The events sent.
    pub events: usize,
}

/// How the driver picks a device's input transport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransportChoice {
    /// The transport of the host's CoreSimulator (design D6).
    #[default]
    Auto,
    /// Only this transport, for tests and diagnosis.
    Only(TransportKind),
}

/// How a driver is opened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DriverOptions {
    pub transport: TransportChoice,
    pub tree: TreeChoice,
    /// A reader binary built for the Simulator to spawn instead of the one
    /// built into the driver, for development and diagnosis.
    pub reader: Option<PathBuf>,
}

/// The input session of a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputStatus {
    pub transport: TransportKind,
    /// How long the transport took to answer its activation probe; `None`
    /// until it has, and for a transport without one.
    pub activation: Option<Duration>,
    /// Connections opened to the device: one, unless the peer went away and
    /// the session reconnected.
    pub connections: u32,
    /// Why this transport: the host version rule that chose it, or the
    /// driver option that forced it.
    pub reason: String,
}

/// Why a device was taken out of service: the private call that failed,
/// how, when, and on which host versions.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostics {
    pub call: String,
    pub failure: String,
    pub at: SystemTime,
    /// The host's CoreSimulator and Xcode.
    pub host: String,
}

/// How a device's tree is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeStatus {
    /// The backend reads use.
    pub backend: TreeBackend,
    /// Why this backend: the reader runs, or why it could not.
    pub reason: String,
    /// The reader spawned into the device, once one was.
    pub reader: Option<ReaderStatus>,
}

/// The helper process the driver spawned into a Simulator to read its tree
/// (simulator-driver spec, "No test runner and no per-app build"). Nothing
/// is installed: it runs from its path on the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderStatus {
    /// Its name and version, as its hello announced them, such as
    /// `simulator-reader 0.1.0`.
    pub name: String,
    /// The binary on the host it was spawned from.
    pub path: PathBuf,
    pub pid: i32,
    pub spawned_at: SystemTime,
    /// From the spawn call until it could serve: its hello, sent once its
    /// runtime was loaded. The spec's gate is 500 ms.
    pub ready_in: Option<Duration>,
    pub state: ReaderState,
    /// Readers spawned into this device by this driver so far.
    pub spawns: u32,
}

/// Where a reader is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReaderState {
    /// Spawned, not connected yet.
    Starting,
    /// Serving reads.
    Ready,
    /// The process ended, with its exit status when known.
    Exited(Option<i32>),
    /// It could not start, or its connection failed.
    Failed(String),
}

impl fmt::Display for ReaderState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReaderState::Starting => f.write_str("starting"),
            ReaderState::Ready => f.write_str("ready"),
            ReaderState::Exited(Some(status)) => write!(f, "exited with status {status}"),
            ReaderState::Exited(None) => f.write_str("exited"),
            ReaderState::Failed(reason) => write!(f, "failed: {reason}"),
        }
    }
}

/// What the driver knows about one device.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetStatus {
    pub device: DeviceInfo,
    /// The input session, once input was sent to the device.
    pub input: Option<InputStatus>,
    /// How its tree is read, once it was.
    pub tree: Option<TreeStatus>,
    /// Set once a failure in the private framework layer took the device
    /// out of service.
    pub unavailable: Option<Diagnostics>,
}

/// How text goes into the focused field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextMethod {
    /// Keyboard events for a short string a US keyboard can type, the
    /// pasteboard otherwise.
    #[default]
    Auto,
    /// One key press per character.
    Keyboard,
    /// The device pasteboard, then Command-V.
    Paste,
}

/// The longest string `TextMethod::Auto` types key by key; longer ones are
/// pasted, since typing takes a few tens of milliseconds a character.
const KEYBOARD_LIMIT: usize = 32;

impl TextMethod {
    /// How `type_text` enters `text` when asked to as `self`: `Auto` types
    /// a string of at most 32 characters that a US keyboard can type and
    /// pastes any other; `Keyboard` and `Paste` stay as they are.
    pub fn resolve(self, text: &str) -> TextMethod {
        match self {
            TextMethod::Auto
                if text.chars().count() <= KEYBOARD_LIMIT
                    && text.chars().all(|c| Key::character(c).is_some()) =>
            {
                TextMethod::Keyboard
            }
            TextMethod::Auto => TextMethod::Paste,
            chosen => chosen,
        }
    }
}

/// What `type_text` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextEntry {
    /// `Keyboard` or `Paste`, never `Auto`.
    pub method: TextMethod,
    pub characters: usize,
    pub elapsed: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Auto` types a short string a US keyboard can type and pastes a
    /// long one or one with a character no key types; the other methods
    /// stay as asked.
    #[test]
    fn auto_types_a_short_typeable_string_and_pastes_any_other() {
        assert_eq!(TextMethod::Auto.resolve("Smoke Org"), TextMethod::Keyboard);
        assert_eq!(TextMethod::Auto.resolve(""), TextMethod::Keyboard);
        let longest = "a".repeat(KEYBOARD_LIMIT);
        assert_eq!(TextMethod::Auto.resolve(&longest), TextMethod::Keyboard);
        let longer = "a".repeat(KEYBOARD_LIMIT + 1);
        assert_eq!(TextMethod::Auto.resolve(&longer), TextMethod::Paste);
        assert_eq!(TextMethod::Auto.resolve("café"), TextMethod::Paste);
        assert_eq!(TextMethod::Keyboard.resolve(&longer), TextMethod::Keyboard);
        assert_eq!(TextMethod::Paste.resolve("Smoke Org"), TextMethod::Paste);
    }
}
