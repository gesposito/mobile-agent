//! The reference fixtures (`fixtures/CONTRACT.md`) and device settings the
//! tests change and put back.

use std::time::Duration;

use crate::adb::Device;

pub const COMPOSE: &str = "dev.mobileagent.fixture.compose";
pub const EXPO: &str = "dev.mobileagent.fixture.expo";

/// Whether `package` is installed.
pub fn installed(device: &Device, package: &str) -> bool {
    device
        .shell(&format!("pm path {package} || true"))
        .contains("package:")
}

/// Panics with a hint unless `package` is installed.
pub fn require(device: &Device, package: &str) {
    assert!(
        installed(device, package),
        "{package} is not installed on {}; build and install it from fixtures/",
        device.serial
    );
}

/// Starts `package` afresh on `tab`, in touch mode, and waits for its
/// first frame.
pub fn launch(device: &Device, package: &str, tab: &str) {
    require(device, package);
    touch_mode(device);
    device.shell(&format!(
        "am start -W -S -n {package}/.MainActivity --es initial_tab {tab}"
    ));
    // The first frame is drawn before the tree has settled.
    std::thread::sleep(Duration::from_millis(800));
}

/// Puts the display back in touch mode when it is not. Key events, which
/// several tests inject, take it out of touch mode, and an app launched
/// then gives its first field focus, so the keyboard is up before a test
/// taps anything. A tap on the status bar's first pixel, which does nothing
/// there, puts it back.
pub fn touch_mode(device: &Device) {
    let mode = device.shell_output("dumpsys window | grep -m1 mInTouchMode");
    if String::from_utf8_lossy(&mode.stdout).contains("mInTouchMode=false") {
        device.shell("input tap 1 1");
    }
}

pub fn force_stop(device: &Device, package: &str) {
    device.shell(&format!("am force-stop {package}"));
}

/// The display rotation settings, put back as they were when dropped.
pub struct RotationGuard<'a> {
    device: &'a Device,
    accelerometer: String,
    user: String,
}

impl<'a> RotationGuard<'a> {
    /// Records the current settings.
    pub fn new(device: &'a Device) -> RotationGuard<'a> {
        let get = |key: &str| {
            device
                .shell(&format!("settings get system {key}"))
                .trim()
                .to_owned()
        };
        RotationGuard {
            device,
            accelerometer: get("accelerometer_rotation"),
            user: get("user_rotation"),
        }
    }

    /// Freezes the display at `quarter_turns` (0 to 3) and waits for it to turn.
    pub fn freeze(&self, quarter_turns: u8) {
        self.device.shell(&format!(
            "settings put system accelerometer_rotation 0; settings put system user_rotation {quarter_turns}"
        ));
        std::thread::sleep(Duration::from_millis(1500));
    }
}

impl Drop for RotationGuard<'_> {
    fn drop(&mut self) {
        // Releasing UI automation also restores the rotation state the
        // connection recorded, which can land on another value than the one
        // recorded here, so each setting is written until it reads back.
        for attempt in 0..5 {
            let mut done = true;
            for (key, value) in [
                ("user_rotation", &self.user),
                ("accelerometer_rotation", &self.accelerometer),
            ] {
                if value == "null" || value.is_empty() {
                    continue;
                }
                let current = self.device.shell(&format!("settings get system {key}"));
                if current.trim() != value {
                    let _ = self
                        .device
                        .shell_output(&format!("settings put system {key} {value}"));
                    done = false;
                }
            }
            if done && attempt > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}
