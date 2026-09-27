//! The device drivers behind the daemon's driver contract: the Simulator
//! driver of `crates/simulator` and the Android driver of `crates/android`,
//! each put behind the mirror's backend traits (`mirror::backend`) with the
//! app control, screenshots and status the contract adds.
//!
//! - [`simulator`]: booted Simulators; the tree through the driver's
//!   spawned reader, frames from the display surface as the change signal,
//!   input through the host's transport, launches with a launch
//!   environment, and doctor's Xcode compatibility table.
//! - [`android`]: adb targets in every state; the resident agent deployed,
//!   reused or replaced and its session, the emulator's controller when it
//!   answers, launches by package with string extras, and the agent stopped
//!   when the target is detached.

pub(crate) mod android;
pub(crate) mod simulator;

use std::path::Path;
use std::sync::Arc;

use crate::driver::Driver;

/// The device drivers this host runs, in registration order: the Simulator
/// driver needs macOS.
pub(crate) const DEFAULT: &[&str] = if cfg!(target_os = "macos") {
    &[simulator::NAME, android::NAME]
} else {
    &[android::NAME]
};

/// The device driver `name` (as [`DEFAULT`] lists them), keeping what
/// outlives the daemon under `state_dir`.
pub(crate) fn driver(name: &str, state_dir: &Path) -> Result<Arc<dyn Driver>, String> {
    let _ = (name, state_dir);
    todo!("task 8.8")
}
