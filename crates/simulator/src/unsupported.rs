//! The driver on hosts other than macOS. The type exists so that callers such
//! as the daemon compile on every platform; opening it always fails, and it
//! has no values, so none of its methods can run.

use std::path::Path;
use std::time::Duration;

use crate::driver::{Frames, Input, Lifecycle};
use crate::error::{Error, Result};
use crate::frames::FrameStream;
use crate::model::{
    BootReport, Button, DeviceInfo, DeviceTypeInfo, Dispatch, DriverOptions, FrameOptions, Gesture,
    Key, LaunchOptions, Launched, Point, PreferencesOutcome, RuntimeInfo, Screenshot,
    ScreenshotOptions, Swipe, TargetStatus, TextEntry, TextMethod,
};

/// Drives iOS Simulators; available on macOS only.
pub enum SimulatorDriver {}

impl SimulatorDriver {
    /// Fails with [`Error::PlatformUnsupported`].
    pub fn open() -> Result<SimulatorDriver> {
        Err(Error::PlatformUnsupported)
    }

    /// Fails with [`Error::PlatformUnsupported`].
    pub fn open_with(_options: DriverOptions) -> Result<SimulatorDriver> {
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

    pub fn reset_target(&self, _target: &str) -> Result<()> {
        match *self {}
    }

    pub fn compatibility(&self) -> Result<crate::compat::Compatibility> {
        match *self {}
    }

    #[cfg(feature = "fault-injection")]
    pub fn inject_fault(&self, _target: &str) -> Result<()> {
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

    fn status(&self, _target: &str) -> Result<TargetStatus> {
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

impl Input for SimulatorDriver {
    fn tap(&self, _target: &str, _point: Point) -> Result<Dispatch> {
        match *self {}
    }

    fn press(&self, _target: &str, _point: Point, _hold: Duration) -> Result<Dispatch> {
        match *self {}
    }

    fn swipe(&self, _target: &str, _swipe: &Swipe) -> Result<Dispatch> {
        match *self {}
    }

    fn gesture(&self, _target: &str, _gesture: &Gesture) -> Result<Dispatch> {
        match *self {}
    }

    fn button(&self, _target: &str, _button: Button) -> Result<Dispatch> {
        match *self {}
    }

    fn key(&self, _target: &str, _key: Key) -> Result<Dispatch> {
        match *self {}
    }

    fn type_text(&self, _target: &str, _text: &str, _method: TextMethod) -> Result<TextEntry> {
        match *self {}
    }
}
