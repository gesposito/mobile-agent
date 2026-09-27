//! The driver traits: lifecycle, input, frames and tree. They carry no Apple
//! types, so the mirror and the daemon can hold any backend behind them; they
//! live here until `crates/mirror` defines its own.
//!
//! Every method names its device with `target`: a UDID, or a name that exactly
//! one device has.

use std::path::Path;
use std::time::Duration;

use crate::error::Result;
use crate::frames::FrameStream;
use crate::model::{
    BootReport, Button, DeviceInfo, Dispatch, FrameOptions, Gesture, Key, LaunchOptions, Launched,
    Point, Screenshot, ScreenshotOptions, Swipe,
};

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

/// Frames from the display surface and screenshots on demand (simulator-driver
/// spec, "Frames and screenshots"; design D7).
pub trait Frames {
    /// Subscribes to the display: the frame on screen now, then one each time
    /// the display's pixels change, up to `options.max_fps` a second. Nothing
    /// arrives while the display is static.
    fn frames(&self, target: &str, options: &FrameOptions) -> Result<FrameStream>;

    /// A JPEG of the display as it is now.
    fn screenshot(&self, target: &str, options: &ScreenshotOptions) -> Result<Screenshot>;
}

/// Touches, hardware buttons and keys (simulator-driver spec, "Input
/// actions"). Points are in iOS points on the upright screen.
pub trait Input {
    /// A tap: touch down and up at once, which S3 found enough.
    fn tap(&self, target: &str, point: Point) -> Result<Dispatch>;

    /// A touch held for `hold`, a long press when the hold is long enough.
    fn press(&self, target: &str, point: Point, hold: Duration) -> Result<Dispatch>;

    /// A one-finger drag whose duration sets its velocity.
    fn swipe(&self, target: &str, swipe: &Swipe) -> Result<Dispatch>;

    /// A gesture of one or two fingers, such as a pinch.
    fn gesture(&self, target: &str, gesture: &Gesture) -> Result<Dispatch>;

    /// A short press of a hardware button.
    fn button(&self, target: &str, button: Button) -> Result<Dispatch>;

    /// A key press, with its modifiers held around it.
    fn key(&self, target: &str, key: Key) -> Result<Dispatch>;
}
