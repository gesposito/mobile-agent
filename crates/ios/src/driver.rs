//! The driver traits. They carry no `idevice` types, so the daemon can hold
//! the driver behind them and adapt it to its own driver contract
//! (`crates/daemon/src/driver.rs`), as it does the Simulator driver.
//!
//! Every method names its device with `target`, the device identifier
//! `devices` lists: names are not unique, so no method takes one.

use crate::error::Result;
use crate::model::{Connected, DeviceInfo, TargetStatus};
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
