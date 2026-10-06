//! Task 10.19: the drivers a run names in `MOBILE_AGENT_DRIVERS` reach the
//! daemons the device tests start, so a run against the Simulator alone
//! starts them without the Android driver, whose listing asks the adb server
//! for its devices. A daemon with fake targets serves them alone all the
//! same, since device drivers named beside fake targets register too.
//!
//! The test sets the variable in its own environment, as a shell sets it for
//! `cargo test`, so it is the only test of this binary. The Simulator driver
//! runs on macOS alone, and so does the test.

#![cfg(target_os = "macos")]

mod support;

use serde_json::json;
use support::Daemon;

/// The drivers `daemon` registered, as `doctor` lists them.
#[track_caller]
fn drivers(daemon: &Daemon) -> Vec<String> {
    let doctor = daemon.request(json!({"action": "doctor"}));
    assert_eq!(doctor["success"], true, "{doctor}");
    serde_json::from_value(doctor["data"]["drivers"].clone())
        .unwrap_or_else(|e| panic!("doctor lists no drivers ({e}): {doctor}"))
}

#[test]
fn the_drivers_a_run_names_reach_device_daemons_and_not_fake_target_ones() {
    // SAFETY: this binary runs this test alone, and the test sets the
    // variable before it starts a thread or a process.
    unsafe { std::env::set_var("MOBILE_AGENT_DRIVERS", "simulator") };

    let devices = Daemon::builder().start();
    assert_eq!(
        drivers(&devices),
        ["simulator"],
        "a daemon serving the host's devices registers the drivers the run names"
    );

    let fakes = Daemon::start("ios");
    assert_eq!(
        drivers(&fakes),
        ["fake"],
        "a daemon with fake targets registers no device driver"
    );
}
