//! The driver on hosts other than macOS. The type exists so that callers such
//! as the daemon compile on every platform; opening it always fails, and it
//! has no values, so none of its methods can run.

use std::path::Path;
use std::time::Duration;

use protocol::message::{Call, Reply};

use crate::driver::{Devices, Frames, Input, Lifecycle, Session};
use crate::error::{Error, Result};
use crate::frames::FrameStream;
use crate::model::{
    AttachOptions, Attached, Button, Connected, DeviceInfo, Dispatch, FrameOptions,
    FramePathStatus, Installed, Key, LaunchOptions, Launched, Point, Released, Screenshot,
    ScreenshotOptions, Swipe, TargetStatus, Terminated,
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

impl Session for IosDriver {
    fn attach(&self, _target: &str, _options: &AttachOptions) -> Result<Attached> {
        match *self {}
    }

    fn call(&self, _target: &str, _call: Call, _timeout: Duration) -> Result<Reply> {
        match *self {}
    }

    fn release(&self, _target: &str) -> Result<Released> {
        match *self {}
    }
}

impl Frames for IosDriver {
    fn frames(&self, _target: &str, _options: &FrameOptions) -> Result<FrameStream> {
        match *self {}
    }

    fn screenshot(&self, _target: &str, _options: &ScreenshotOptions) -> Result<Screenshot> {
        match *self {}
    }

    fn frame_paths(&self, _target: &str) -> Result<Vec<FramePathStatus>> {
        match *self {}
    }
}

impl Input for IosDriver {
    fn tap(&self, _target: &str, _point: Point) -> Result<Dispatch> {
        match *self {}
    }

    fn press(&self, _target: &str, _point: Point, _hold: Duration) -> Result<Dispatch> {
        match *self {}
    }

    fn swipe(&self, _target: &str, _swipe: &Swipe) -> Result<Dispatch> {
        match *self {}
    }

    fn button(&self, _target: &str, _button: Button) -> Result<Dispatch> {
        match *self {}
    }

    fn key(&self, _target: &str, _key: Key) -> Result<Dispatch> {
        match *self {}
    }

    fn keys(&self, _target: &str, _keys: &[Key]) -> Result<Dispatch> {
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
