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

/// Starts `package` afresh on `tab` and waits for its first frame.
pub fn launch(device: &Device, package: &str, tab: &str) {
    require(device, package);
    device.shell(&format!(
        "am start -W -S -n {package}/.MainActivity --es initial_tab {tab}"
    ));
    // The first frame is drawn before the tree has settled.
    std::thread::sleep(Duration::from_millis(800));
}

pub fn force_stop(device: &Device, package: &str) {
    device.shell(&format!("am force-stop {package}"));
}

/// Hides the soft keyboard if it is showing, with Back.
pub fn hide_keyboard(device: &Device) {
    let shown = device
        .shell("dumpsys input_method | grep -E 'mInputShown|isInputViewShown' || true")
        .contains("=true");
    if shown {
        device.shell("input keyevent 4");
        std::thread::sleep(Duration::from_millis(400));
    }
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
        let restore = |key: &str, value: &str| {
            if value != "null" && !value.is_empty() {
                let _ = self
                    .device
                    .shell_output(&format!("settings put system {key} {value}"));
            }
        };
        restore("user_rotation", &self.user);
        restore("accelerometer_rotation", &self.accelerometer);
        std::thread::sleep(Duration::from_millis(1000));
    }
}
