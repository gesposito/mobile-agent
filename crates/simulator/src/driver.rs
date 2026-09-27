//! The driver traits: lifecycle, input, frames and tree. They carry no Apple
//! types, so the mirror and the daemon can hold any backend behind them; they
//! live here until `crates/mirror` defines its own.
//!
//! Every method names its device with `target`: a UDID, or a name that exactly
//! one device has.

use std::path::Path;

use crate::error::Result;
use crate::model::{BootReport, DeviceInfo, LaunchOptions, Launched};

/// Lists Simulators and drives their lifecycle (simulator-driver spec,
/// "Simulator discovery and lifecycle").
pub trait Lifecycle {
    /// Every device of the default set, with its identifier, name, runtime
    /// and state.
    fn devices(&self) -> Result<Vec<DeviceInfo>>;

    /// Boots the device and returns once it is usable. The device outlives
    /// this process, as with `simctl boot`.
    fn boot(&self, target: &str) -> Result<BootReport>;

    /// Shuts the device down; a device already shut down is left alone.
    fn shutdown(&self, target: &str) -> Result<()>;

    /// Installs the application bundle at `app`.
    fn install(&self, target: &str, app: &Path) -> Result<()>;

    /// Launches an installed application with the given arguments and
    /// environment and returns its process identifier.
    fn launch(&self, target: &str, bundle_id: &str, options: &LaunchOptions) -> Result<Launched>;

    /// Terminates the application; one that is not running is left alone.
    fn terminate(&self, target: &str, bundle_id: &str) -> Result<()>;
}
