//! Physical iOS device driver (design D18 to D25, spec `ios-device-driver`):
//! discovery and readiness over usbmuxd and lockdown, the CoreDevice tunnel
//! and the developer services over it, and app lifecycle through DVT and the
//! installation proxy, driven from the host through the `idevice` crate in
//! this process, with no root, no helper process and no network interface
//! of its own.
//!
//! [`IosDriver`] implements the driver traits, [`Devices`] and
//! [`Lifecycle`]; on hosts other than macOS it exists but cannot be opened. `README.md` covers the
//! readiness states, the compatibility table and the map from spec
//! scenarios to tests.

// The platform-neutral parts are used only by the macOS driver.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod bundle;
mod compat;
mod discovery;
mod driver;
mod error;
mod identity;
mod model;
mod readiness;
mod runner;
mod services;
mod version;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

pub use bundle::{Bundle, inspect as inspect_bundle};
pub use compat::{
    CallSite, FIRST_SUPPORTED_IOS, FIRST_UNSUPPORTED_IOS, IDEVICE, VALIDATED_IOS, VALIDATED_XCODE,
    call_sites, render as render_compatibility, supported_ios, supported_range,
};
pub use driver::{Devices, Lifecycle, Session};
pub use error::{Error, Result};
pub use identity::{IdentityCheck, host_identity};
pub use model::{
    ATTACH_TIMEOUT, AttachOptions, Attached, Connected, Connection, DeviceInfo, DiskImage,
    Installed, LaunchOptions, Launched, MountAttempt, MountPath, MountReport, MuxEntry, Probe,
    RUNNER_BUNDLE_ID, RUNNER_NAME, RUNNER_PORT, Released, RunnerSpec, RunnerState, RunnerStatus,
    TEAM_VARIABLE, TargetStatus, Terminated, Trust, TunnelStatus,
};
pub use readiness::{Need, Readiness};
pub use services::{NEEDED, Needed};

#[cfg(target_os = "macos")]
pub use macos::IosDriver;
#[cfg(not(target_os = "macos"))]
pub use unsupported::IosDriver;
