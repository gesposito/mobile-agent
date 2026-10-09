//! Drivers: how the daemon finds targets and attaches to them.
//!
//! A [`Driver`] is the factory the target registry takes backends from: it
//! lists the targets of one kind and attaches to one, handing back the
//! [`mirror::backend::Backend`] the target's mirror runs on and what the
//! mirror has no place for: app control, on-demand screenshots and the
//! driver's own status. The daemon calls a driver from the registry and
//! from the target's own worker thread, one attach per target until the
//! target is detached; dropping the [`Attachment`] detaches it.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use mirror::MirrorConfig;
use mirror::backend::Backend;
use serde_json::{Map, Value};

use crate::devices;
use crate::fake::{FakeDriver, Typing};

/// A driver's failure: a stable code, a message and a retryable flag.
pub type DriverError = protocol::Error;

/// Lists targets of one kind and attaches to them.
pub trait Driver: Send + Sync {
    /// A short name, such as `android`, `simulator` or `fake`.
    fn name(&self) -> &str;

    /// The targets connected now.
    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError>;

    /// Attaches to `target`. The daemon calls it on the target's worker
    /// thread, once, when a command first needs the target, with that
    /// command's `options`.
    fn attach(
        &self,
        target: &TargetInfo,
        options: &AttachOptions,
    ) -> Result<Attachment, DriverError>;

    /// Rows this driver contributes to `doctor`'s compatibility table.
    fn compatibility(&self) -> Vec<CompatibilityRow> {
        Vec::new()
    }
}

/// A target's operating system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Platform {
    Android,
    Ios,
}

impl Platform {
    /// `android` or `ios`, as the `--platform` flag takes it.
    pub fn id(self) -> &'static str {
        match self {
            Platform::Android => "android",
            Platform::Ios => "ios",
        }
    }

    pub fn parse(text: &str) -> Option<Platform> {
        match text.to_ascii_lowercase().as_str() {
            "android" => Some(Platform::Android),
            "ios" => Some(Platform::Ios),
            _ => None,
        }
    }
}

/// A kind of target, as the benchmark harness classes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TargetClass {
    Simulator,
    AndroidEmulator,
    AndroidDevice,
    /// A physical iPhone attached over USB (design D24).
    IosDevice,
}

impl TargetClass {
    pub fn id(self) -> &'static str {
        match self {
            TargetClass::Simulator => "simulator",
            TargetClass::AndroidEmulator => "android-emulator",
            TargetClass::AndroidDevice => "android-device",
            TargetClass::IosDevice => "ios-device",
        }
    }
}

/// One target a driver found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetInfo {
    /// Simulator UDID or adb serial; `--device` takes it.
    pub id: String,
    pub platform: Platform,
    pub class: TargetClass,
    /// A human name, such as the device model or the Simulator's name.
    pub name: String,
    pub model: Option<String>,
    /// OS name and version, such as `Android API 36` or `iOS 26.5`.
    pub os: Option<String>,
    /// The state its platform reports: `booted` for a Simulator; the adb
    /// server's `device`, `offline`, `unauthorized` and so on for Android.
    pub state: String,
    /// What makes the target usable when its state keeps it from being
    /// automated, such as accepting the USB debugging prompt.
    pub advice: Option<String>,
    /// The driver that found it.
    pub driver: String,
}

/// How to attach.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttachOptions {
    /// Take over what keeps the target from automation, such as another
    /// holder of Android's UI automation: the command's `--force`.
    pub force: bool,
}

/// A target's screen size in device points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// A target's screen, whose size a driver reads as it is now: a display
/// that rotates swaps its sides.
pub trait Screen: Send {
    /// The screen's size in device points now.
    fn size(&mut self) -> Result<Size, DriverError>;
}

/// A screen whose size does not change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedScreen(pub Size);

impl Screen for FixedScreen {
    fn size(&mut self) -> Result<Size, DriverError> {
        Ok(self.0)
    }
}

/// What attaching to a target yields. Dropping it detaches the target: a
/// driver stops what it started for the target, such as the Android agent
/// or the Simulator's reader, when its parts are dropped.
pub struct Attachment {
    /// The backend the target's mirror runs on.
    pub backend: Backend,
    pub apps: Box<dyn AppControl>,
    /// The screen, for gestures that no element anchors.
    pub display: Box<dyn Screen>,
    pub config: MirrorConfig,
    /// Screenshots the driver takes on demand. Without one, `screenshot`
    /// encodes the mirror's latest frame.
    pub capture: Option<Box<dyn Capture>>,
    /// What `status` reports about the target from the driver: its input
    /// path or transport, helpers and failures.
    pub report: Option<Arc<dyn Report>>,
    /// What the driver knows about the input it dispatched last.
    pub input_notes: Option<Arc<dyn InputNotes>>,
    /// The target's input session, for a driver whose first input waits for
    /// the session to activate.
    pub input_session: Option<Box<dyn InputSession>>,
}

/// A target's input session, which activates before input goes in.
pub trait InputSession: Send {
    /// Starts the session, unless it has started, without sending any
    /// input, so that it activates meanwhile.
    fn start(&mut self) -> Result<(), DriverError>;

    /// Whether the session is active, so input goes in without waiting.
    fn active(&mut self) -> bool;
}

/// What a driver knows about the input it dispatched last.
pub trait InputNotes: Send + Sync {
    /// How long the last input waited for its input transport to activate:
    /// `Some` for the first input of a new input session, whose effects can
    /// come later than a warm session's.
    fn activation_wait(&self) -> Option<Duration>;
}

/// Launches and stops apps: what the mirror's backend has no place for.
pub trait AppControl: Send {
    /// Launches `app`, or brings it to the front if it runs; restarts it
    /// first when `launch.relaunch` is set.
    fn launch(&mut self, app: &str, launch: &Launch) -> Result<(), DriverError>;

    /// Stops `app`.
    fn terminate(&mut self, app: &str) -> Result<(), DriverError>;
}

/// How to launch an app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// Stop the app first if it runs.
    pub relaunch: bool,
    /// The launch environment on iOS; string extras of the launch intent
    /// on Android, under the same names.
    pub env: Vec<(String, String)>,
}

/// Takes screenshots on demand.
pub trait Capture: Send {
    /// The screen as it is now.
    fn screenshot(&mut self, request: &CaptureRequest) -> Result<Image, DriverError>;
}

/// How to take a screenshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureRequest {
    /// The longest side of the image in pixels; larger screens are scaled
    /// down.
    pub max_dimension: u32,
    /// JPEG quality, 1 to 100.
    pub quality: u8,
}

/// An encoded screenshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// `jpeg` or `png`.
    pub format: &'static str,
}

/// Reports what the driver knows about an attached target. The daemon asks
/// from the thread serving `status`, while the target's worker may be busy.
pub trait Report: Send + Sync {
    fn report(&self) -> TargetReport;
}

/// What `status` shows about a target from its driver.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetReport {
    /// Lines for people, such as `input: dtuhidd, ...`.
    pub lines: Vec<String>,
    /// The same facts as JSON fields.
    pub data: Map<String, Value>,
}

/// One row of `doctor`'s compatibility table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatibilityRow {
    /// What was validated or found, such as `Xcode 27.0 (27A266a)`.
    pub subject: String,
    /// The call site or path, such as `-[SimDevice lookup:error:]`, or
    /// what the driver expects.
    pub feature: String,
    /// The Simulator driver's words: `validated`, `supported`, `limited`
    /// or `unsupported`.
    pub status: String,
    pub note: Option<String>,
}

/// Whether taps, swipes and frames on an Android emulator go through the
/// emulator's gRPC controller: `--emulator-controller` or
/// `MOBILE_AGENT_EMULATOR_CONTROLLER`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EmulatorController {
    /// Through the controller whenever the emulator serves it and it
    /// answers, else through the agent.
    #[default]
    Auto,
    /// Through the agent on every emulator, as on a device, with no frame
    /// stream. Emulator 37.1.11 serves its token-protected controller even
    /// when started without `-grpc`, and only `-ports` leaves it off, so
    /// this is how the agent path is taken on an emulator that serves it.
    Off,
}

impl EmulatorController {
    /// Parses `auto` or `off`, whatever the case.
    pub fn parse(text: &str) -> Result<EmulatorController, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(EmulatorController::Auto),
            "off" => Ok(EmulatorController::Off),
            _ => Err(format!(
                "emulator controller {text:?}: use auto, to take the emulator's gRPC controller when it answers, or off, to leave it unused"
            )),
        }
    }

    /// `auto` or `off`, as the flag takes it and `status` reports it.
    pub fn name(self) -> &'static str {
        match self {
            EmulatorController::Auto => "auto",
            EmulatorController::Off => "off",
        }
    }
}

/// Which drivers a daemon registers, and where they keep what outlives it.
#[derive(Clone, Copy, Debug)]
pub struct Registration<'a> {
    /// Fake targets, as `MOBILE_AGENT_FAKE_TARGETS` lists them.
    pub fake: Option<&'a str>,
    /// How long fake targets take to show a change.
    pub fake_latency: Duration,
    /// How long after the attach fake targets' frame streams start.
    pub fake_frame_delay: Duration,
    /// How long fake targets' fixture apps show their launch splash.
    pub fake_splash: Duration,
    /// How long after they start fake targets' input sessions activate.
    pub fake_activation: Duration,
    /// How long detaching a fake target takes.
    pub fake_stop: Duration,
    /// Whether fake targets' lists glide on after a stroke whose finger
    /// lifts moving.
    pub fake_fling: bool,
    /// How fake targets' fields take text whose method the target may
    /// choose.
    pub fake_typing: Typing,
    /// Device drivers by name, as `MOBILE_AGENT_DRIVERS` lists them; see
    /// [`device_drivers`].
    pub drivers: Option<&'a str>,
    /// The daemon's own directory, where drivers keep state for the next
    /// daemon, such as the records of the Android agents it started.
    pub state_dir: &'a Path,
    /// Whether Android emulators' controllers serve input and frames, for
    /// the Android driver and fake emulators alike.
    pub emulator_controller: EmulatorController,
}

/// The drivers a daemon registers.
///
/// This is the registration point for device drivers: the Simulator driver
/// (`crates/simulator`) on macOS and the Android driver (`crates/android`)
/// everywhere, both put behind the mirror's backend traits in
/// [`crate::devices`], the Simulator driver also filling `doctor`'s Xcode
/// compatibility table through [`Driver::compatibility`]. Fake targets
/// stand in for them: when `fake` names any, only the fake driver serves,
/// showing every change `fake_latency` after its cause, its frames from
/// `fake_frame_delay` after the attach, its fixture apps' launch splash for
/// `fake_splash`, its input `fake_activation` after its session starts,
/// taking `fake_stop` to detach and, with `fake_fling`, its list gliding on
/// after a stroke that lifts moving, its fields taking text as `fake_typing`
/// says, unless `drivers` names device drivers too.
/// With `emulator_controller` off, the Android driver leaves every
/// emulator's controller unused and fake emulators come without a frame
/// source.
pub fn registered(registration: &Registration<'_>) -> Result<Vec<Arc<dyn Driver>>, String> {
    let fake = registration.fake.filter(|spec| !spec.trim().is_empty());
    let mut drivers: Vec<Arc<dyn Driver>> = Vec::new();
    for name in device_drivers(registration.drivers, fake.is_some())? {
        drivers.push(devices::driver(
            name,
            registration.state_dir,
            registration.emulator_controller,
        )?);
    }
    if let Some(spec) = fake {
        drivers.push(Arc::new(
            FakeDriver::parse(spec)?
                .with_latency(registration.fake_latency)
                .with_frame_delay(registration.fake_frame_delay)
                .with_splash(registration.fake_splash)
                .with_activation(registration.fake_activation)
                .with_stop(registration.fake_stop)
                .with_fling(registration.fake_fling)
                .with_typing(registration.fake_typing)
                .with_emulator_controller(registration.emulator_controller),
        ));
    }
    Ok(drivers)
}

/// The device drivers a daemon registers: those `drivers` names
/// (comma-separated `simulator`, or `ios`, and `android`; `none` for
/// none), else every device driver this host runs unless fake targets are
/// named.
pub fn device_drivers(drivers: Option<&str>, fake: bool) -> Result<Vec<&'static str>, String> {
    let Some(list) = drivers.filter(|list| !list.trim().is_empty()) else {
        return Ok(if fake {
            Vec::new()
        } else {
            devices::DEFAULT.to_vec()
        });
    };
    let entries: Vec<String> = list
        .split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect();
    if entries.iter().any(|entry| entry == "none") {
        return if entries.len() == 1 {
            Ok(Vec::new())
        } else {
            Err(format!(
                "drivers {list:?}: none names no driver, so it cannot stand beside others"
            ))
        };
    }
    let mut names: Vec<&'static str> = Vec::new();
    for entry in &entries {
        let name = match entry.as_str() {
            "simulator" | "ios" => devices::simulator::NAME,
            "android" => devices::android::NAME,
            other => {
                return Err(format!(
                    "drivers {list:?}: {other:?} is not a device driver; name simulator (or ios), android, or none"
                ));
            }
        };
        if !devices::DEFAULT.contains(&name) {
            return Err(format!(
                "the {name} driver needs macOS, which this host is not"
            ));
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The device drivers this host runs, in registration order.
    fn host() -> Vec<&'static str> {
        if cfg!(target_os = "macos") {
            vec!["simulator", "android"]
        } else {
            vec!["android"]
        }
    }

    #[test]
    fn every_device_driver_the_host_runs_registers_unless_fake_targets_are_named() {
        assert_eq!(device_drivers(None, false).unwrap(), host());
        assert_eq!(device_drivers(Some("  "), false).unwrap(), host());
        assert!(device_drivers(None, true).unwrap().is_empty());
    }

    #[test]
    fn named_drivers_register_alone_or_beside_fake_targets() {
        assert_eq!(device_drivers(Some("android"), true).unwrap(), ["android"]);
        assert_eq!(
            device_drivers(Some(" Android , android"), false).unwrap(),
            ["android"]
        );
        assert!(device_drivers(Some("none"), false).unwrap().is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!(
                device_drivers(Some("android,simulator"), false).unwrap(),
                ["android", "simulator"]
            );
        } else {
            let refused = device_drivers(Some("simulator"), false).unwrap_err();
            assert!(refused.contains("macOS"), "{refused}");
        }
        let unknown = device_drivers(Some("windows"), false).unwrap_err();
        assert!(
            unknown.contains("windows")
                && unknown.contains("simulator")
                && unknown.contains("ios")
                && unknown.contains("none"),
            "{unknown}"
        );
        assert!(device_drivers(Some("none,android"), false).is_err());
    }

    /// Task 5.1, design D25: `ios` names the physical iPhone driver, which
    /// registers only when the run names it, alone or beside the others,
    /// never by default: its listing asks the keychain for the signing
    /// identity and lockdown for each phone, and waits on a phone that does
    /// not answer. Off macOS it is refused.
    #[test]
    fn the_ios_driver_registers_only_when_named() {
        assert!(!device_drivers(None, false).unwrap().contains(&"ios"));
        if cfg!(target_os = "macos") {
            assert_eq!(device_drivers(Some("ios"), false).unwrap(), ["ios"]);
            assert_eq!(
                device_drivers(Some(" IOS , simulator, ios"), false).unwrap(),
                ["ios", "simulator"]
            );
            assert_eq!(device_drivers(Some("ios"), true).unwrap(), ["ios"]);
        } else {
            let refused = device_drivers(Some("ios"), false).unwrap_err();
            assert!(refused.contains("macOS"), "{refused}");
        }
    }

    #[test]
    fn registration_puts_device_drivers_before_the_fake_one() {
        let dir = std::env::temp_dir().join(format!("daemon-registration-{}", std::process::id()));
        let names = |fake: Option<&str>, drivers: Option<&str>| -> Vec<String> {
            registered(&Registration {
                fake,
                fake_latency: Duration::ZERO,
                fake_frame_delay: Duration::ZERO,
                fake_splash: Duration::ZERO,
                fake_activation: Duration::ZERO,
                fake_stop: Duration::ZERO,
                fake_fling: false,
                fake_typing: Typing::Whole,
                drivers,
                state_dir: &dir,
                emulator_controller: EmulatorController::Auto,
            })
            .unwrap()
            .iter()
            .map(|driver| driver.name().to_owned())
            .collect()
        };
        assert_eq!(names(Some("ios"), None), ["fake"]);
        assert_eq!(names(Some("ios"), Some("android")), ["android", "fake"]);
        assert_eq!(names(None, Some("none")), Vec::<String>::new());
        assert_eq!(names(Some(" "), Some("android")), ["android"]);
        if cfg!(target_os = "macos") {
            assert_eq!(names(Some("ios-device"), Some("ios")), ["ios", "fake"]);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Task 10.12: the emulator controller setting takes `auto` or `off`
    /// and names what it takes in its refusal.
    #[test]
    fn the_emulator_controller_setting_is_auto_or_off() {
        assert_eq!(EmulatorController::default(), EmulatorController::Auto);
        assert_eq!(
            EmulatorController::parse("off"),
            Ok(EmulatorController::Off)
        );
        assert_eq!(
            EmulatorController::parse(" OFF "),
            Ok(EmulatorController::Off)
        );
        assert_eq!(
            EmulatorController::parse("auto"),
            Ok(EmulatorController::Auto)
        );
        let refused = EmulatorController::parse("on").unwrap_err();
        assert!(
            refused.contains("\"on\"") && refused.contains("auto") && refused.contains("off"),
            "{refused}"
        );
        assert_eq!(EmulatorController::Off.name(), "off");
        assert_eq!(EmulatorController::Auto.name(), "auto");
    }
}
