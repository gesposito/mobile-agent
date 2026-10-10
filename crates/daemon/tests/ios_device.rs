//! The physical iOS driver over the socket (task 5.1): a fake iPhone, the
//! `ios-device` stand-in, listed in its class and state and reporting its
//! input path, frames and runner in `status`; and a daemon that registers
//! the `ios` driver, which tells `doctor` the runner bundle an attach
//! installs and fills the compatibility table with the device's rows.

mod support;

use serde_json::{Value, json};
use support::Daemon;

/// The entry `status` gives the target `id`.
#[track_caller]
fn target(status: &Value, id: &str) -> Value {
    status["data"]["targets"]
        .as_array()
        .and_then(|targets| targets.iter().find(|t| t["id"] == id))
        .cloned()
        .unwrap_or_else(|| panic!("status lists no target {id}: {status}"))
}

/// Spec `ios-device-driver`, "Ready device listed", on the fake: `devices`
/// lists the iPhone in the `ios-device` class, ready, with its iOS.
#[test]
fn devices_lists_a_fake_iphone_in_the_ios_device_class() {
    let daemon = Daemon::start("ios-device,ios");
    let reply = daemon.request(json!({"action": "devices"}));
    assert_eq!(reply["success"], true, "{reply}");
    let devices = reply["data"]["devices"].as_array().expect("a device list");
    let phone = devices
        .iter()
        .find(|d| d["id"] == "FAKE-IOS-DEVICE-1")
        .unwrap_or_else(|| panic!("no fake iPhone in {reply}"));
    assert_eq!(phone["platform"], "ios", "{phone}");
    assert_eq!(phone["class"], "ios-device", "{phone}");
    assert_eq!(phone["state"], "ready", "{phone}");
    assert_eq!(phone["os"], "iOS 26.6.2", "{phone}");
    assert!(phone.get("advice").is_none(), "{phone}");
    let simulator = devices
        .iter()
        .find(|d| d["id"] == "FAKE-IOS-1")
        .unwrap_or_else(|| panic!("no fake Simulator in {reply}"));
    assert_eq!(simulator["class"], "simulator", "{simulator}");
}

/// Spec `ios-device-driver`, "Runner stays up", on the fake: once attached,
/// `status` names the iPhone's input path, its frames and its runner, with
/// the runner's version and restarts.
#[test]
fn status_names_a_fake_iphones_input_frames_and_runner() {
    let daemon = Daemon::start("ios-device");
    let snapshot = daemon.request(json!({"action": "snapshot", "interactive": true}));
    assert_eq!(snapshot["success"], true, "{snapshot}");
    let status = daemon.request(json!({"action": "status"}));
    let phone = target(&status, "FAKE-IOS-DEVICE-1");
    assert_eq!(phone["class"], "ios-device", "{phone}");
    assert_eq!(phone["attached"], true, "{phone}");
    assert_eq!(phone["attaches"], 1, "{phone}");
    assert_eq!(phone["frameSource"], true, "{phone}");
    let driver = &phone["driver"];
    assert_eq!(driver["input"], "dtuhidd", "{phone}");
    assert_eq!(driver["frames"], "screenshots", "{phone}");
    let agent = format!(
        "mobile-agent-ios-runner {}",
        include_str!("../../../agent-ios/VERSION").trim()
    );
    assert_eq!(driver["runner"]["agent"], agent.as_str(), "{phone}");
    assert_eq!(driver["runner"]["restarts"], 0, "{phone}");
    let lines: Vec<&str> = driver["lines"]
        .as_array()
        .map(|lines| lines.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for start in ["input: dtuhidd", "frames: screenshots", "runner: "] {
        assert!(
            lines.iter().any(|line| line.starts_with(start)),
            "no line starts {start:?}: {lines:?}"
        );
    }
}

/// Spec `ios-device-driver`, "Table lists the device paths": a daemon that
/// registers the `ios` driver, named alone in `MOBILE_AGENT_DRIVERS`, lists
/// it in `doctor`, names the runner bundle its attach installs with the
/// version it expects, and fills the compatibility table with the supported
/// range and the device's call sites. Nothing asks a phone.
#[cfg(target_os = "macos")]
#[test]
fn a_daemon_with_the_ios_driver_names_its_runner_bundle_and_the_device_rows() {
    let daemon = Daemon::builder().env("MOBILE_AGENT_DRIVERS", "ios").start();
    let doctor = daemon.request(json!({"action": "doctor"}));
    assert_eq!(doctor["success"], true, "{doctor}");
    let data = &doctor["data"];
    assert_eq!(data["drivers"], json!(["ios"]), "{doctor}");
    let runner = &data["iosRunner"];
    assert!(
        runner["bundle"]
            .as_str()
            .is_some_and(|bundle| bundle.ends_with("agent-ios/dist/MobileAgentRunner-Runner.app")),
        "{doctor}"
    );
    assert_eq!(
        runner["version"],
        include_str!("../../../agent-ios/VERSION").trim(),
        "{doctor}"
    );
    let rows = data["compatibility"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let feature = |part: &str| {
        rows.iter().any(|row| {
            row["feature"]
                .as_str()
                .is_some_and(|feature| feature.contains(part))
        })
    };
    assert!(feature("17.4"), "the supported range: {rows:?}");
    for call in ["ListDevices", "mainTouchscreen", "takeScreenshot", "27183"] {
        assert!(feature(call), "no row for {call}: {rows:?}");
    }
    assert!(
        rows.iter().all(|row| row["subject"]
            .as_str()
            .is_some_and(|subject| subject.contains("iOS 26.6.2"))),
        "every row names the iOS it was validated on: {rows:?}"
    );
}
