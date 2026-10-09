//! The driver traits. They carry no `idevice` types, so the daemon can hold
//! the driver behind them and adapt it to its own driver contract
//! (`crates/daemon/src/driver.rs`), as it does the Simulator driver.
//!
//! Every method names its device with `target`, the device identifier
//! `devices` lists: names are not unique, so no method takes one.

use crate::error::Result;
use crate::model::DeviceInfo;
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
}
