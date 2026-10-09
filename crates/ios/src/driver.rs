//! The driver traits. They carry no `idevice` types, so the daemon can hold
//! the driver behind them and adapt it to its own driver contract
//! (`crates/daemon/src/driver.rs`), as it does the Simulator driver.
//!
//! Every method names its device with `target`, the device identifier
//! `devices` lists: names are not unique, so no method takes one.

use std::path::Path;
use std::time::Duration;

use protocol::message::{Call, Reply};

use crate::error::Result;
use crate::model::{
    AttachOptions, Attached, Button, Connected, DeviceInfo, Dispatch, Installed, Key,
    LaunchOptions, Launched, Point, Released, Swipe, TargetStatus, Terminated,
};
use crate::readiness::Need;

/// Lists the devices attached over USB and what stands between each and a
/// session (spec `ios-device-driver`, "Device discovery and readiness").
pub trait Devices {
    /// Every iOS device attached over USB, with its identifier, name, model,
    /// iOS version and readiness. A device that is not ready is listed too,
    /// in the first state it has not passed.
    fn devices(&self) -> Result<Vec<DeviceInfo>>;

    /// Reads `target` afresh and returns it when nothing that `need` covers
    /// stands in the way; otherwise fails with the code of the first state
    /// that does ([`crate::Error::NotReady`]).
    fn require(&self, target: &str, need: Need) -> Result<DeviceInfo>;

    /// Gets `target` ready for its developer services (spec
    /// `ios-device-driver`, "Transport without root or helpers"): fails on
    /// what stands in the way, mounts the developer disk image when the
    /// device lacks it, and opens the CoreDevice tunnel in this process
    /// unless it is open. The tunnel is kept for the target's lifetime.
    fn connect(&self, target: &str) -> Result<Connected>;

    /// What the driver holds for `target`: its tunnel and the disk image as
    /// the last `connect` found it; nothing before a `connect`.
    fn status(&self, target: &str) -> Result<TargetStatus>;
}

/// Installs, launches and terminates apps and tells whether they run (spec
/// `ios-device-driver`, "App lifecycle"; design D19), connecting the device
/// first when it is not connected.
pub trait Lifecycle {
    /// Installs the signed bundle at `app`, a `.app` built for a device,
    /// through the installation proxy, replacing an installed one.
    fn install(&self, target: &str, app: &Path) -> Result<Installed>;

    /// Launches the installed app `bundle_id` with `options`' arguments and
    /// environment, through DVT's process control, and returns its pid. The
    /// lock is read first, and a locked device fails with `device_locked`
    /// without the launch being tried.
    fn launch(&self, target: &str, bundle_id: &str, options: &LaunchOptions) -> Result<Launched>;

    /// Ends the app's processes with SIGKILL through DVT and returns once
    /// they left the process list; an app that is not running is left alone.
    fn terminate(&self, target: &str, bundle_id: &str) -> Result<Terminated>;

    /// The pid of the app's process, from DVT's process list, when it runs.
    fn running(&self, target: &str, bundle_id: &str) -> Result<Option<u32>>;
}

/// The runner's session (spec `ios-device-driver`, "One resident runner per
/// session"; design D20): one runner per target, started once through the
/// test manager and kept for every command until the target is released.
pub trait Session {
    /// Starts the runner on `target` and returns once it said hello with
    /// the version `options.runner` names. Runner processes left from a lost
    /// session are ended first; a runner that is missing, or says hello with
    /// another version, is replaced by the bundle `options.runner` names. A
    /// session that serves is kept and reported as it is.
    fn attach(&self, target: &str, options: &AttachOptions) -> Result<Attached>;

    /// Sends `call` to the runner over the device protocol and returns its
    /// reply, waiting at most `timeout` in all. A runner that died is
    /// started again once, and a call made meanwhile waits for it within
    /// its timeout.
    fn call(&self, target: &str, call: Call, timeout: Duration) -> Result<Reply>;

    /// Ends the session: closes the runner's connection, ends the test
    /// manager's session and signals any runner process still running.
    fn release(&self, target: &str) -> Result<Released>;
}

/// Touches, hardware buttons and keys (spec `ios-device-driver`, "Input
/// actions without idle waits"; design D22). They go through the developer
/// disk image's `dtuhidd` from the host where the device's iOS takes its
/// reports without a display stream, from iOS 18 up to 27, and through the
/// runner's synthesis otherwise when a runner is attached. Each is
/// acknowledged once its last report is written, with no idle wait. Points
/// are in points on the portrait screen.
pub trait Input {
    /// A tap: a contact held 50 ms, then the release.
    fn tap(&self, target: &str, point: Point) -> Result<Dispatch>;

    /// A contact held for `hold`: a long press when it is long enough.
    fn press(&self, target: &str, point: Point, hold: Duration) -> Result<Dispatch>;

    /// A one-finger stroke, which rests before it lifts when it says so.
    fn swipe(&self, target: &str, swipe: &Swipe) -> Result<Dispatch>;

    /// A short press of a hardware button, as an Indigo button event.
    fn button(&self, target: &str, button: Button) -> Result<Dispatch>;

    /// A key with its modifiers held around it, as Indigo keyboard events.
    fn key(&self, target: &str, key: Key) -> Result<Dispatch>;

    /// Keys typed one after another on a virtual keyboard the driver
    /// registers for them and removes afterwards.
    fn keys(&self, target: &str, keys: &[Key]) -> Result<Dispatch>;
}
