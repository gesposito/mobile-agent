//! The driver on hosts other than macOS. The type exists so that callers such
//! as the daemon compile on every platform; opening it always fails, and it
//! has no values, so none of its methods can run.

use std::path::Path;

use crate::driver::{Frames, Lifecycle};
use crate::error::{Error, Result};
use crate::frames::FrameStream;
use crate::model::{
    BootReport, DeviceInfo, DeviceTypeInfo, FrameOptions, LaunchOptions, Launched,
    PreferencesOutcome, RuntimeInfo, Screenshot, ScreenshotOptions,
};

/// Drives iOS Simulators; available on macOS only.
pub enum SimulatorDriver {}

impl SimulatorDriver {
    /// Fails with [`Error::PlatformUnsupported`].
    pub fn open() -> Result<SimulatorDriver> {
        Err(Error::PlatformUnsupported)
    }

    pub fn runtimes(&self) -> Result<Vec<RuntimeInfo>> {
        match *self {}
    }

    pub fn device_types(&self) -> Result<Vec<DeviceTypeInfo>> {
        match *self {}
    }

    pub fn create_device(
        &self,
        _name: &str,
        _device_type: &str,
        _runtime: Option<&str>,
    ) -> Result<DeviceInfo> {
        match *self {}
    }

    pub fn delete_device(&self, _target: &str) -> Result<()> {
        match *self {}
    }

    pub fn prepare_accessibility(&self, _target: &str) -> Result<PreferencesOutcome> {
        match *self {}
    }
}

impl Lifecycle for SimulatorDriver {
    fn devices(&self) -> Result<Vec<DeviceInfo>> {
        match *self {}
    }

    fn boot(&self, _target: &str) -> Result<BootReport> {
        match *self {}
    }

    fn shutdown(&self, _target: &str) -> Result<()> {
        match *self {}
    }

    fn install(&self, _target: &str, _app: &Path) -> Result<()> {
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

    fn terminate(&self, _target: &str, _bundle_id: &str) -> Result<()> {
        match *self {}
    }
}

impl Frames for SimulatorDriver {
    fn frames(&self, _target: &str, _options: &FrameOptions) -> Result<FrameStream> {
        match *self {}
    }

    fn screenshot(&self, _target: &str, _options: &ScreenshotOptions) -> Result<Screenshot> {
        match *self {}
    }
}
