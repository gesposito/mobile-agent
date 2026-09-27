//! Helpers shared by the integration tests.
//!
//! Every integration test drives a real Simulator, so each one first checks
//! `MOBILE_AGENT_SIM_UDID`. When it is unset, as in CI, the test prints why it
//! skipped and passes. Set it to the UDID of a booted iPhone 17 Simulator with
//! both fixtures installed and on the Expo fixture's Login tab:
//!
//! ```sh
//! MOBILE_AGENT_SIM_UDID=<udid> cargo test -p simulator
//! ```
//!
//! Tests that create their own devices install the fixtures from
//! `fixtures/*/build`, which `just fixtures-build` produces, or from the paths
//! in `MOBILE_AGENT_SWIFTUI_APP` and `MOBILE_AGENT_EXPO_APP`. Those devices are
//! deleted when the test ends, whether it passed or not.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use simulator::{Lifecycle, SimulatorDriver};

/// The variable that enables the integration tests.
pub const UDID_VARIABLE: &str = "MOBILE_AGENT_SIM_UDID";

pub const SWIFTUI_BUNDLE: &str = "dev.mobileagent.fixture.swiftui";
pub const EXPO_BUNDLE: &str = "dev.mobileagent.fixture.expo";

/// The prefix of every device a test creates, so leftovers are easy to find.
pub const DEVICE_PREFIX: &str = "mobile-agent-simtest";

/// The booted Simulator to drive, or `None` after printing why the test is
/// skipped.
pub fn gated(test: &str) -> Option<String> {
    match std::env::var(UDID_VARIABLE) {
        Ok(udid) if !udid.trim().is_empty() => Some(udid.trim().to_string()),
        _ => {
            eprintln!(
                "{test}: skipped; set {UDID_VARIABLE} to the UDID of a booted Simulator \
                 with the fixtures installed"
            );
            None
        }
    }
}

/// Serializes the tests of one binary that change what a Simulator shows.
/// Cargo runs test binaries one after another, so this covers the rest.
pub fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn driver() -> SimulatorDriver {
    SimulatorDriver::open().expect("the driver opens")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/simulator sits two levels below the workspace root")
        .to_path_buf()
}

fn fixture_app(variable: &str, relative: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join(relative));
    assert!(
        path.is_dir(),
        "{} does not exist; run `just fixtures-build` or set {variable}",
        path.display()
    );
    path
}

/// The SwiftUI fixture's application bundle.
pub fn swiftui_app() -> PathBuf {
    fixture_app(
        "MOBILE_AGENT_SWIFTUI_APP",
        "fixtures/swiftui/build/Build/Products/Debug-iphonesimulator/FixtureSwiftUI.app",
    )
}

/// The Expo fixture's application bundle.
pub fn expo_app() -> PathBuf {
    fixture_app(
        "MOBILE_AGENT_EXPO_APP",
        "fixtures/expo/build/Build/Products/Release-iphonesimulator/FixtureExpo.app",
    )
}

/// A name no other test run uses.
pub fn unique(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{label}-{}-{nanos}", std::process::id())
}

/// The device's data directory in the default device set.
pub fn data_path(udid: &str) -> PathBuf {
    let home = std::env::var_os("HOME").expect("HOME is set");
    PathBuf::from(home)
        .join("Library/Developer/CoreSimulator/Devices")
        .join(udid)
        .join("data")
}

/// The device's accessibility preferences file.
pub fn accessibility_preferences(udid: &str) -> PathBuf {
    data_path(udid).join("Library/Preferences/com.apple.Accessibility.plist")
}

/// The value at `key` of a property list, as `plutil -extract ... raw` prints it.
pub fn plist_value(file: &Path, key: &str) -> Option<String> {
    let output = Command::new("plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(file)
        .output()
        .expect("plutil runs");
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// A device made for one test, shut down and deleted when dropped.
pub struct TemporaryDevice<'a> {
    driver: &'a SimulatorDriver,
    pub udid: String,
    pub name: String,
    deleted: bool,
}

impl<'a> TemporaryDevice<'a> {
    /// An iPhone 17 on the newest iOS runtime.
    pub fn create(driver: &'a SimulatorDriver, label: &str) -> TemporaryDevice<'a> {
        let name = unique(&format!("{DEVICE_PREFIX}-{label}"));
        let device = driver
            .create_device(&name, "iPhone 17", None)
            .expect("the device is created");
        assert_eq!(device.name, name);
        TemporaryDevice {
            driver,
            udid: device.udid,
            name,
            deleted: false,
        }
    }

    /// Shuts the device down and deletes it, failing the test on an error.
    pub fn delete(mut self) {
        self.driver
            .shutdown(&self.udid)
            .expect("the device shuts down");
        self.driver
            .delete_device(&self.udid)
            .expect("the device is deleted");
        self.deleted = true;
    }
}

impl Drop for TemporaryDevice<'_> {
    fn drop(&mut self) {
        if self.deleted {
            return;
        }
        if let Err(error) = self.driver.shutdown(&self.udid) {
            eprintln!("shutting down {} failed: {error}", self.udid);
        }
        if let Err(error) = self.driver.delete_device(&self.udid) {
            eprintln!(
                "deleting {} failed: {error}; delete it with `xcrun simctl delete {}`",
                self.udid, self.udid
            );
        }
    }
}

/// A device line of `xcrun simctl list devices`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimctlDevice {
    pub udid: String,
    pub name: String,
    pub state: String,
}

/// The devices `xcrun simctl list devices` reports. Its lines read
/// `    <name> (<UDID>) (<state>)`, and names may contain parentheses, so the
/// UDID is found by its shape.
pub fn simctl_devices() -> Vec<SimctlDevice> {
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices"])
        .output()
        .expect("xcrun simctl runs");
    assert!(output.status.success(), "xcrun simctl list devices failed");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_simctl_line)
        .collect()
}

pub fn parse_simctl_line(line: &str) -> Option<SimctlDevice> {
    let bytes = line.as_bytes();
    let start = (0..line.len()).find(|&i| {
        bytes[i] == b'('
            && line.get(i + 1..i + 37).is_some_and(is_uuid)
            && bytes.get(i + 37) == Some(&b')')
    })?;
    let name = line[..start].trim().to_string();
    let udid = line[start + 1..start + 37].to_string();
    let state = line[start + 38..]
        .trim_start()
        .strip_prefix('(')?
        .split(')')
        .next()?
        .to_string();
    Some(SimctlDevice { udid, name, state })
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// The command line and environment of a host process, from `ps -wwE`.
/// Simulator applications are ordinary processes of this user.
pub fn process_environment(pid: i32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-wwE", "-o", "command=", "-p"])
        .arg(pid.to_string())
        .output()
        .expect("ps runs");
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn process_alive(pid: i32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Polls `condition` every 50 ms until it holds or `timeout` passes.
pub fn wait_for(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}
