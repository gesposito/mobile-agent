//! The values the driver's traits take and return. Nothing here depends on
//! Apple frameworks, so these types exist on every platform.

use std::fmt;
use std::time::Duration;

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
