//! Task 3.3 on the device: the SwiftUI fixture installed from its signed
//! bundle, launched with an environment, found running, and terminated, and
//! the install timed.
//!
//! The bundle is the device build of `fixtures/swiftui`, signed for the team
//! `MOBILE_AGENT_IOS_TEAM` names, which `MOBILE_AGENT_IOS_SWIFTUI_APP` points
//! to (`spikes/s10-ios-runner/fixtures.sh swiftui` builds it until task 6.1
//! moves the device build into the fixture's own script). Without it the
//! tests skip. The fixture stays installed.

#![cfg(target_os = "macos")]

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use ios::{Devices, IosDriver, LaunchOptions, Lifecycle};

const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";

/// The variable naming the fixture's device build.
const APP_VARIABLE: &str = "MOBILE_AGENT_IOS_SWIFTUI_APP";

/// The device and the fixture's bundle, or `None` after printing why the
/// test is skipped.
fn gated(test: &str) -> Option<(String, PathBuf)> {
    let udid = common::gated(test)?;
    match std::env::var_os(APP_VARIABLE).map(PathBuf::from) {
        Some(app) if app.is_dir() => Some((udid, app)),
        _ => {
            eprintln!(
                "{test}: skipped; set {APP_VARIABLE} to the SwiftUI fixture's device build \
                 (spikes/s10-ios-runner/fixtures.sh swiftui prints its path)"
            );
            None
        }
    }
}

/// The host's load averages, as `sysctl` gives them.
fn load() -> String {
    let output = Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .expect("sysctl runs");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_matches(|c| c == '{' || c == '}')
        .trim()
        .to_owned()
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

#[test]
fn the_fixture_is_installed_launched_with_an_environment_found_running_and_terminated() {
    let Some((udid, app)) = gated("the_fixture_is_installed_launched") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    let installed = driver.install(&udid, &app).expect("the fixture installs");
    assert_eq!(installed.bundle_id, SWIFTUI);

    let options = LaunchOptions::new()
        .env("FIXTURE_INITIAL_TAB", "Big")
        .terminating_running();
    let launched = driver
        .launch(&udid, SWIFTUI, &options)
        .expect("the fixture launches");
    assert!(launched.pid > 0);
    let listing = Instant::now();
    assert_eq!(
        driver.running(&udid, SWIFTUI).expect("the process list"),
        Some(launched.pid)
    );
    let first_listing = listing.elapsed();
    let listing = Instant::now();
    driver.running(&udid, SWIFTUI).expect("the process list");
    let second_listing = listing.elapsed();

    let relaunched = driver
        .launch(&udid, SWIFTUI, &options)
        .expect("the fixture launches again");
    assert_ne!(
        relaunched.pid, launched.pid,
        "KillExisting starts a new process"
    );

    let terminated = driver
        .terminate(&udid, SWIFTUI)
        .expect("the fixture terminates");
    assert_eq!(terminated.pids, vec![relaunched.pid]);
    assert_eq!(
        driver.running(&udid, SWIFTUI).expect("the process list"),
        None
    );
    let again = driver
        .terminate(&udid, SWIFTUI)
        .expect("terminating it again does nothing");
    assert!(again.pids.is_empty());

    let missing = driver
        .launch(
            &udid,
            "dev.mobileagent.placeholder.missing",
            &LaunchOptions::new(),
        )
        .expect_err("not installed");
    assert_eq!(missing.code(), "app_not_installed", "{missing}");

    eprintln!(
        "install {:.0} ms, launch {:.1} ms and again {:.1} ms (the lock read included), \
         the process list {:.1} ms and again {:.1} ms, terminate {:.1} ms",
        milliseconds(installed.elapsed),
        milliseconds(launched.elapsed),
        milliseconds(relaunched.elapsed),
        milliseconds(first_listing),
        milliseconds(second_listing),
        milliseconds(terminated.elapsed)
    );
    driver.reset_target(&udid).expect("resets");
}

/// One series of installs of the fixture, timed, with the host's load before
/// and after: `cargo test -p ios --test lifecycle -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, run on a quiet host"]
fn install_series() {
    let Some((udid, app)) = gated("install_series") else {
        return;
    };
    let _device = common::exclusive();
    const RUNS: usize = 10;
    let driver = IosDriver::open().expect("the driver opens");
    driver.connect(&udid).expect("connects");
    let before = load();
    let started = Instant::now();
    let mut samples: Vec<f64> = (0..RUNS)
        .map(|_| {
            milliseconds(
                driver
                    .install(&udid, &app)
                    .expect("the fixture installs")
                    .elapsed,
            )
        })
        .collect();
    let after = load();
    samples.sort_by(f64::total_cmp);
    let rank = |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1];
    eprintln!(
        "install of the SwiftUI fixture, {RUNS} runs over {:.1} s: p50 {:.0} ms, p95 {:.0} ms, \
         min {:.0} ms, max {:.0} ms; load before {before}, after {after}; samples {samples:?}",
        started.elapsed().as_secs_f64(),
        rank(50),
        rank(95),
        samples[0],
        samples[RUNS - 1]
    );
}
