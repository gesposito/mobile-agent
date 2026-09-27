//! Task 6.5: the input transport comes from the host's CoreSimulator, the
//! same for every Simulator it boots (simulator-driver spec, "Input
//! transport selection"), and sessions on two Simulators stay apart
//! ("Parallel Simulators"). This host has one runtime, iOS 26.5, so the two
//! Simulators share it; the unit tests in `select.rs` cover the rule's
//! independence from the runtime and the unsupported-combination diagnostic.

#![cfg(target_os = "macos")]

mod common;

use std::process::Command;
use std::thread;
use std::time::Duration;

use simulator::{Input, Lifecycle, SimulatorDriver, TransportKind};

use common::{ExpoOnLogin, expo};

/// The CoreSimulator the host has installed, read from its bundle.
fn host_core_simulator() -> String {
    let output = Command::new("plutil")
        .args([
            "-extract",
            "CFBundleVersion",
            "raw",
            "-o",
            "-",
            "/Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/Resources/Info.plist",
        ])
        .output()
        .expect("plutil runs");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn two_simulators_get_the_hosts_transport() {
    let Some(shared) = common::gated("two_simulators_get_the_hosts_transport") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let fresh = common::fresh_device(&driver);
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: shared.clone(),
    };
    let core_simulator = host_core_simulator();
    for udid in [&shared, &fresh] {
        common::show_expo(&driver, udid, "login");
        let dispatch = driver.tap(udid, expo::BLANK).expect("a tap");
        assert_eq!(dispatch.transport, TransportKind::Dtuhid);
        let status = driver.status(udid).expect("status");
        let runtime = status.device.runtime.as_ref().expect("a runtime");
        let input = status.input.expect("an input session");
        eprintln!(
            "{} ({} {}): {} because {}",
            status.device.name, runtime.name, runtime.build, input.transport, input.reason
        );
        assert_eq!(input.transport, TransportKind::Dtuhid);
        assert!(
            input
                .reason
                .contains(&format!("CoreSimulator {core_simulator}")),
            "the status names the host rule: {}",
            input.reason
        );
    }
}

#[test]
fn two_sessions() {
    let Some(shared) = common::gated("two_sessions") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let fresh = common::fresh_device(&driver);
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: shared.clone(),
    };
    common::show_expo(&driver, &shared, "login");
    common::show_expo(&driver, &fresh, "login");
    let (shared_frames, shared_settled) = common::settle(&driver, &shared);
    let (fresh_frames, fresh_settled) = common::settle(&driver, &fresh);

    // A tap on one Simulator changes that Simulator only.
    driver.tap(&shared, expo::TAB_LIST).expect("a tap");
    // The changes are waited for longer than the absence of one: at a load
    // above 200 a tab switch took more than 2 s to draw.
    let changed = common::wait_for_change(&shared_frames, &shared_settled, Duration::from_secs(5))
        .expect("the tapped Simulator changed");
    assert!(
        common::wait_for_change(&fresh_frames, &fresh_settled, Duration::from_secs(1)).is_none(),
        "the other Simulator did not"
    );
    let shared_settled = common::drain_until_quiet(
        &shared_frames,
        Some(changed),
        Duration::from_millis(700),
        Duration::from_secs(10),
    )
    .expect("the List tab settled");

    // Both driven at once, each from its own thread, and each lands.
    thread::scope(|scope| {
        let (driver, shared, fresh) = (&driver, &shared, &fresh);
        let one = scope.spawn(move || driver.tap(shared, expo::TAB_LOGIN));
        let two = scope.spawn(move || driver.tap(fresh, expo::TAB_LIST));
        one.join()
            .expect("a thread")
            .expect("a tap on the shared Simulator");
        two.join()
            .expect("a thread")
            .expect("a tap on the fresh Simulator");
    });
    assert!(
        common::wait_for_change(&shared_frames, &shared_settled, Duration::from_secs(5)).is_some(),
        "the shared Simulator changed"
    );
    assert!(
        common::wait_for_change(&fresh_frames, &fresh_settled, Duration::from_secs(5)).is_some(),
        "the fresh Simulator changed"
    );

    // Each has its own input session.
    for udid in [&shared, &fresh] {
        let input = driver.status(udid).expect("status").input.expect("input");
        assert_eq!(input.connections, 1, "{udid} has its own connection");
    }
}
