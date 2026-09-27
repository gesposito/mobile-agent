//! Task 10.12: the daemon's switch that leaves Android emulators' gRPC
//! controllers unused (`--emulator-controller off`,
//! `MOBILE_AGENT_EMULATOR_CONTROLLER=off`). Emulator 37.1.11 serves its
//! token-protected controller even when started without `-grpc`, so the
//! switch is how an emulator that serves it takes the agent's path. Over the
//! socket with fake targets: `status` and `doctor` report the setting, and
//! an emulator's input and frames take the agent's path, without a frame
//! source, as a device's do.

mod support;

use std::time::Duration;

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

/// Attaches `device` with a snapshot.
#[track_caller]
fn attach(daemon: &Daemon, device: &str) {
    let reply = daemon.request(json!({"action": "snapshot", "device": device}));
    assert_eq!(reply["success"], true, "{reply}");
}

#[test]
fn the_switch_puts_an_emulators_input_and_frames_on_the_agent_path() {
    let daemon = Daemon::builder()
        .fake("android,ios")
        .env("MOBILE_AGENT_EMULATOR_CONTROLLER", "off")
        .start();
    attach(&daemon, "emulator-5554");
    attach(&daemon, "FAKE-IOS-1");
    let status = daemon.request(json!({"action": "status"}));
    assert_eq!(
        status["data"]["daemon"]["emulatorController"], "off",
        "{status}"
    );
    let emulator = target(&status, "emulator-5554");
    assert_eq!(emulator["frameSource"], false, "{emulator}");
    let driver = &emulator["driver"];
    assert_eq!(driver["input"], "agent", "{emulator}");
    assert_eq!(driver["frames"], Value::Null, "{emulator}");
    assert_eq!(driver["emulatorController"], "off", "{emulator}");
    assert!(
        driver["lines"]
            .as_array()
            .is_some_and(|lines| lines.iter().any(|l| l
                .as_str()
                .is_some_and(|l| l.starts_with("emulator controller: off")))),
        "status says why: {emulator}"
    );
    // A Simulator keeps its frames: the switch concerns emulators alone.
    assert_eq!(
        target(&status, "FAKE-IOS-1")["frameSource"],
        true,
        "{status}"
    );
    let doctor = daemon.request(json!({"action": "doctor"}));
    assert_eq!(doctor["data"]["emulatorController"], "off", "{doctor}");
}

#[test]
fn without_the_switch_an_emulator_takes_the_controllers_path() {
    let daemon = Daemon::builder().fake("android").start();
    attach(&daemon, "emulator-5554");
    let status = daemon.request(json!({"action": "status"}));
    assert_eq!(
        status["data"]["daemon"]["emulatorController"], "auto",
        "{status}"
    );
    let emulator = target(&status, "emulator-5554");
    assert_eq!(emulator["frameSource"], true, "{emulator}");
    assert_ne!(emulator["driver"]["input"], "agent", "{emulator}");
    let doctor = daemon.request(json!({"action": "doctor"}));
    assert_eq!(doctor["data"]["emulatorController"], "auto", "{doctor}");
}

#[test]
fn the_flag_beats_the_variable_and_a_value_other_than_auto_or_off_stops_the_start() {
    let daemon = Daemon::builder()
        .fake("android")
        .env("MOBILE_AGENT_EMULATOR_CONTROLLER", "off")
        .arg("--emulator-controller")
        .arg("auto")
        .start();
    let status = daemon.request(json!({"action": "status"}));
    assert_eq!(
        status["data"]["daemon"]["emulatorController"], "auto",
        "{status}"
    );

    let mut refused = Daemon::builder()
        .fake("android")
        .arg("--emulator-controller=never")
        .spawn();
    let exit = refused
        .wait_exit(Duration::from_secs(10))
        .expect("the daemon exits");
    assert_eq!(exit.code(), Some(2), "a usage error");
    let stderr = refused.stderr();
    assert!(
        stderr.contains("never") && stderr.contains("auto") && stderr.contains("off"),
        "{stderr}"
    );
}
