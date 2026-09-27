//! iOS Simulator driver: lifecycle, input, frames and the accessibility tree,
//! all driven from the host (design D6, D7 and D12).
//!
//! The private Apple frameworks are loaded at run time and every call into
//! them runs inside an exception guard. [`SimulatorDriver`] implements the
//! driver traits, starting with [`Lifecycle`]; on hosts other than macOS it
//! exists but cannot be opened. `README.md` covers the build, the
//! compatibility table and the map from spec scenarios to tests.

// The platform-neutral helpers are used only by the macOS driver.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod driver;
mod error;
mod lookup;
mod model;
mod version;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

pub use driver::Lifecycle;
pub use error::{Error, Failure, Result};
pub use model::{
    BootReport, DeviceInfo, DeviceState, DeviceTypeInfo, LaunchOptions, Launched,
    PreferencesOutcome, RuntimeInfo,
};

#[cfg(target_os = "macos")]
pub use macos::SimulatorDriver;
#[cfg(not(target_os = "macos"))]
pub use unsupported::SimulatorDriver;
