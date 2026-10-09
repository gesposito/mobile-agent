//! The device drivers behind the daemon's driver contract: the Simulator
//! driver of `crates/simulator`, the physical iPhone driver of `crates/ios`
//! and the Android driver of `crates/android`, each put behind the mirror's
//! backend traits (`mirror::backend`) with the app control, screenshots and
//! status the contract adds.
//!
//! - [`simulator`]: booted Simulators; the tree through the driver's
//!   spawned reader, frames from the display surface as the change signal,
//!   input through the host's transport, launches with a launch
//!   environment, and doctor's Xcode compatibility table.
//! - [`ios`]: iPhones attached over USB in their readiness states, opt-in;
//!   the tree through the resident runner, frames from screenshots polled
//!   at the device's rate, input through `dtuhidd` from the host, text
//!   through the runner, launches with a launch environment, and the
//!   device's rows of doctor's compatibility table.
//! - [`android`]: adb targets in every state; the resident agent deployed,
//!   reused or replaced and its session, the emulator's controller when it
//!   answers and the daemon does not leave it unused, launches by package
//!   with string extras, and the agent stopped when the target is detached.

pub(crate) mod android;
pub(crate) mod ios;
pub(crate) mod simulator;

use std::path::Path;
use std::sync::Arc;

use crate::driver::{Driver, EmulatorController};

/// The device drivers this host runs by default, in registration order: the
/// Simulator driver needs macOS.
pub(crate) const DEFAULT: &[&str] = if cfg!(target_os = "macos") {
    &[simulator::NAME, android::NAME]
} else {
    &[android::NAME]
};

/// The device drivers this host runs only when `MOBILE_AGENT_DRIVERS` names
/// them: the physical iPhone driver, on macOS. Its listing asks the
/// keychain for the runner's signing identity, about 45 ms on every listing
/// with no phone attached, and lockdown for each phone, which a phone that
/// does not answer holds for 10 s, while the target registry waits on it.
pub(crate) const OPT_IN: &[&str] = if cfg!(target_os = "macos") {
    &[ios::NAME]
} else {
    &[]
};

/// The device driver `name` (as [`DEFAULT`] and [`OPT_IN`] list them),
/// keeping what outlives the daemon under `state_dir`; `controller` says
/// whether the Android driver uses emulators' controllers.
pub(crate) fn driver(
    name: &str,
    state_dir: &Path,
    controller: EmulatorController,
) -> Result<Arc<dyn Driver>, String> {
    match name {
        simulator::NAME => Ok(Arc::new(simulator::SimulatorDevices::new())),
        ios::NAME => Ok(Arc::new(ios::IosDevices::new())),
        android::NAME => Ok(Arc::new(android::AndroidDevices::new(
            state_dir, controller,
        ))),
        other => Err(format!("{other:?} is not a device driver")),
    }
}
