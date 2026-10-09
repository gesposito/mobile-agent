//! The driver on hosts other than macOS. The type exists so that callers such
//! as the daemon compile on every platform; opening it always fails, and it
//! has no values, so none of its methods can run.

use std::path::Path;

use crate::driver::{Devices, Lifecycle};
use crate::error::{Error, Result};
use crate::model::{
    Connected, DeviceInfo, Installed, LaunchOptions, Launched, TargetStatus, Terminated,
};
use crate::readiness::Need;

/// Drives physical iOS devices; available on macOS only.
pub enum IosDriver {}

impl IosDriver {
    /// Fails with [`Error::PlatformUnsupported`].
    pub fn open() -> Result<IosDriver> {
        Err(Error::PlatformUnsupported)
    }

    pub fn reset_target(&self, _target: &str) -> Result<()> {
        match *self {}
    }
}

impl Devices for IosDriver {
    fn devices(&self) -> Result<Vec<DeviceInfo>> {
        match *self {}
    }

    fn require(&self, _target: &str, _need: Need) -> Result<DeviceInfo> {
        match *self {}
    }

    fn connect(&self, _target: &str) -> Result<Connected> {
        match *self {}
    }

    fn status(&self, _target: &str) -> Result<TargetStatus> {
        match *self {}
    }
}

impl Lifecycle for IosDriver {
    fn install(&self, _target: &str, _app: &Path) -> Result<Installed> {
        match *self {}
    }

    fn launch(
        &self,
        _target: &str,
        _bundle_id: &str,
        _options: &LaunchOptions,
    ) -> Result<Launched> {
        match *self {}
    }

    fn terminate(&self, _target: &str, _bundle_id: &str) -> Result<Terminated> {
        match *self {}
    }

    fn running(&self, _target: &str, _bundle_id: &str) -> Result<Option<u32>> {
        match *self {}
    }
}
