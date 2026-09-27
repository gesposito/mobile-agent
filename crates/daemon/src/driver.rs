//! Drivers: how the daemon finds targets and attaches to them.
//!
//! A [`Driver`] is the factory the target registry takes backends from: it
//! lists the targets of one kind and attaches to one, handing back the
//! [`mirror::backend::Backend`] the target's mirror runs on and the app
//! control the mirror has no place for. The daemon calls a driver from the
//! registry and from the target's own worker thread, one attach per target
//! for the daemon's lifetime.

use std::sync::Arc;

use mirror::MirrorConfig;
use mirror::backend::Backend;

use crate::fake::FakeDriver;

/// A driver's failure: a stable code, a message and a retryable flag.
pub type DriverError = protocol::Error;

/// Lists targets of one kind and attaches to them.
pub trait Driver: Send + Sync {
    /// A short name, such as `android`, `simulator` or `fake`.
    fn name(&self) -> &str;

    /// The targets connected now.
    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError>;

    /// Attaches to `target`. The daemon calls it on the target's worker
    /// thread, once, when a command first needs the target.
    fn attach(&self, target: &TargetInfo) -> Result<Attachment, DriverError>;

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
}

impl TargetClass {
    pub fn id(self) -> &'static str {
        match self {
            TargetClass::Simulator => "simulator",
            TargetClass::AndroidEmulator => "android-emulator",
            TargetClass::AndroidDevice => "android-device",
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
    /// OS name and version, such as `Android 16` or `iOS 26.5`.
    pub os: Option<String>,
    /// The driver that found it.
    pub driver: String,
}

/// A target's screen size in device points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// What attaching to a target yields.
pub struct Attachment {
    /// The backend the target's mirror runs on.
    pub backend: Backend,
    pub apps: Box<dyn AppControl>,
    /// The screen size, for gestures that no element anchors.
    pub display: Size,
    pub config: MirrorConfig,
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
    /// Launch environment variables (Android intent string extras).
    pub env: Vec<(String, String)>,
}

/// One row of `doctor`'s compatibility table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatibilityRow {
    /// What was validated, such as `Xcode 27.0 / iOS 26.5`.
    pub subject: String,
    /// The call site or path, such as `dtuhidd XPC input`.
    pub feature: String,
    /// `supported`, `degraded` or `unsupported`.
    pub status: String,
    pub note: Option<String>,
}

/// The drivers a daemon registers.
///
/// This is the registration point for device drivers. The Android driver
/// (`crates/android`: adb discovery, agent deploy and session, task 5.3)
/// and the Simulator driver (`crates/simulator`, tasks 6.1 to 6.9) join the
/// list here when they land: each implements [`Driver`], hands the daemon a
/// mirror backend per attached target, and fills `doctor`'s compatibility
/// table through [`Driver::compatibility`]. Until then the list holds only
/// the fake driver, and only when `fake` names fake targets
/// (`MOBILE_AGENT_FAKE_TARGETS` or the daemon's `--fake` flag).
pub fn registered(fake: Option<&str>) -> Result<Vec<Arc<dyn Driver>>, String> {
    let mut drivers: Vec<Arc<dyn Driver>> = Vec::new();
    if let Some(spec) = fake.filter(|spec| !spec.trim().is_empty()) {
        drivers.push(Arc::new(FakeDriver::parse(spec)?));
    }
    Ok(drivers)
}
