//! Task 6.8: a failure inside the private framework layer takes its target
//! out of service with diagnostics while the other targets keep working
//! (simulator-driver spec, "Failure containment"). The failure is injected:
//! the target's next private call raises a real Objective-C exception inside
//! the driver's guard. Task 6.6 adds the scenario's own case, a tree read.

#![cfg(target_os = "macos")]

mod common;

use std::time::{Duration, Instant};

use simulator::{
    Error, Frames, Input, Lifecycle, ScreenshotOptions, SimulatorDriver, Tree, TreeOptions,
};

use common::{ExpoOnLogin, expo};

#[test]
fn framework_call_throws() {
    let Some(shared) = common::gated("framework_call_throws") else {
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
    driver
        .tap(&shared, expo::BLANK)
        .expect("the shared Simulator works");

    // The shared Simulator's next private call throws.
    driver.inject_fault(&shared).expect("armed");
    let error = driver
        .screenshot(&shared, &ScreenshotOptions::default())
        .expect_err("the request fails");
    eprintln!("{error}");
    assert_eq!(error.code(), "target_unavailable");
    let Error::Unavailable { diagnostics, .. } = &error else {
        panic!("expected an unavailable target, got {error:?}");
    };
    assert!(
        diagnostics.failure.contains("injected"),
        "the diagnostics carry the exception: {diagnostics:?}"
    );
    assert!(!diagnostics.call.is_empty(), "{diagnostics:?}");
    assert!(
        diagnostics.host.contains("CoreSimulator"),
        "{diagnostics:?}"
    );

    // It stays out of service, failing at once, and says so in its status.
    let start = Instant::now();
    assert!(matches!(
        driver.tap(&shared, expo::BLANK),
        Err(Error::Unavailable { .. })
    ));
    assert!(start.elapsed() < Duration::from_millis(100));
    let status = driver.status(&shared).expect("status still answers");
    assert_eq!(status.unavailable.as_ref(), Some(diagnostics));

    // The other Simulator keeps working: input, a frame and a screenshot.
    driver
        .tap(&fresh, expo::TAB_LIST)
        .expect("a tap on the other Simulator");
    let stream = driver
        .frames(&fresh, &simulator::FrameOptions::default())
        .expect("frames from the other Simulator");
    assert!(stream.next(Duration::from_secs(2)).is_some());
    driver
        .screenshot(&fresh, &ScreenshotOptions::default())
        .expect("a screenshot of the other Simulator");
    assert_eq!(driver.status(&fresh).expect("status").unavailable, None);

    // A reset brings the target back.
    driver.reset_target(&shared).expect("reset");
    assert_eq!(driver.status(&shared).expect("status").unavailable, None);
    driver
        .screenshot(&shared, &ScreenshotOptions::default())
        .expect("the shared Simulator works again");
    driver.tap(&shared, expo::BLANK).expect("and takes input");
}

#[test]
fn a_tree_read_that_throws_takes_only_its_target_out_of_service() {
    let Some(shared) =
        common::gated("a_tree_read_that_throws_takes_only_its_target_out_of_service")
    else {
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
    let read = |udid: &str| driver.snapshot(udid, &TreeOptions::default());
    common::snapshot_until(&driver, &shared, Duration::from_secs(10), |_| true);
    common::snapshot_until(&driver, &fresh, Duration::from_secs(20), |_| true);

    driver.inject_fault(&shared).expect("armed");
    let error = read(&shared).expect_err("the tree read fails");
    eprintln!("{error}");
    let Error::Unavailable { diagnostics, .. } = &error else {
        panic!("expected an unavailable target, got {error:?}");
    };
    assert!(diagnostics.failure.contains("injected"), "{diagnostics:?}");
    assert!(matches!(read(&shared), Err(Error::Unavailable { .. })));
    assert_eq!(
        driver.status(&shared).expect("status").unavailable.as_ref(),
        Some(diagnostics)
    );

    // The other Simulator's tree is still read.
    let other = read(&fresh).expect("the other Simulator's tree");
    assert!(other.snapshot.node_count > 1);

    driver.reset_target(&shared).expect("reset");
    common::snapshot_until(&driver, &shared, Duration::from_secs(10), |_| true);
}
