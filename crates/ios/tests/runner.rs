//! Task 3.4 on the device: the runner installed on the phone started through
//! the test manager, fifty mixed commands served by one runner process with
//! no restart, and the runner ended on release.
//!
//! The runner is `dev.mobileagent.runner`, spike S10's until `agent-ios`
//! replaces it, saying hello as `mobile-agent-ios-runner` with the version
//! `MOBILE_AGENT_IOS_RUNNER_VERSION` names (S10's `0.1.0` by default). The
//! SwiftUI fixture is brought to the front first when it is installed. The
//! taps land on the status bar, which scrolls a list to its top at most.

#![cfg(target_os = "macos")]

mod common;

use std::time::{Duration, Instant};

use ios::{
    AttachOptions, Devices, IosDriver, LaunchOptions, Lifecycle, RUNNER_BUNDLE_ID, RunnerSpec,
    RunnerState, Session,
};
use protocol::message::{Call, Reply, ScreenshotParams, SnapshotParams, TapParams};

const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";

/// The variable naming the runner's version, when it is not S10's.
const VERSION_VARIABLE: &str = "MOBILE_AGENT_IOS_RUNNER_VERSION";

fn spec() -> RunnerSpec {
    RunnerSpec::new(std::env::var(VERSION_VARIABLE).unwrap_or_else(|_| "0.1.0".to_owned()))
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// Nearest-rank percentile of sorted samples.
fn rank(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

#[test]
fn the_runner_serves_fifty_commands_with_one_process_and_no_restart() {
    let Some(udid) = common::gated("the_runner_serves_fifty_commands") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    if driver
        .launch(&udid, SWIFTUI, &LaunchOptions::new().terminating_running())
        .is_err()
    {
        eprintln!("the SwiftUI fixture is not installed; the runner reads what is in front");
    }
    let attached = driver
        .attach(&udid, &AttachOptions::new(spec()))
        .expect("the runner starts");
    assert_eq!(attached.runner.state, RunnerState::Ready);
    let before = driver
        .running(&udid, RUNNER_BUNDLE_ID)
        .expect("the process list");
    assert!(before.is_some(), "DVT lists the runner's process");

    let mut times: [Vec<f64>; 3] = Default::default();
    let started = Instant::now();
    for index in 0..50 {
        let (kind, call) = match index % 3 {
            0 => (0, Call::Snapshot(SnapshotParams::default())),
            1 => (1, Call::Screenshot(ScreenshotParams::default())),
            _ => (
                2,
                Call::Tap(TapParams {
                    x: 375,
                    y: 10,
                    duration_ms: None,
                }),
            ),
        };
        let sent = Instant::now();
        let reply = driver
            .call(&udid, call, Duration::from_secs(10))
            .unwrap_or_else(|error| panic!("command {index}: {error}"));
        times[kind].push(milliseconds(sent.elapsed()));
        match (kind, reply) {
            (0, Reply::Snapshot(_)) | (1, Reply::Screenshot(_)) | (2, Reply::Input(_)) => {}
            (_, other) => panic!("command {index} answered {other:?}"),
        }
    }
    let total = started.elapsed();

    let after = driver
        .running(&udid, RUNNER_BUNDLE_ID)
        .expect("the process list");
    assert_eq!(
        after, before,
        "the same runner process served every command"
    );
    let status = driver
        .status(&udid)
        .expect("status")
        .runner
        .expect("a runner");
    assert_eq!(status.state, RunnerState::Ready);
    assert_eq!(status.restarts, 0, "no restart");

    let released = driver.release(&udid).expect("releases");
    assert_eq!(
        driver
            .running(&udid, RUNNER_BUNDLE_ID)
            .expect("the process list"),
        None,
        "no runner process is left"
    );
    for samples in &mut times {
        samples.sort_by(f64::total_cmp);
    }
    eprintln!(
        "attach {:.0} ms (hello {:.0} ms after the start, {} leftovers ended); 50 commands in {:.1} s: \
         snapshot p50 {:.0} / p95 {:.0} ms (n={}), screenshot p50 {:.0} / p95 {:.0} ms (n={}), \
         tap p50 {:.0} / p95 {:.0} ms (n={}); release {:.0} ms, {} process signalled; agent {:?}",
        milliseconds(attached.elapsed),
        attached.runner.started_in.map_or(0.0, milliseconds),
        attached.runner.leftovers.len(),
        total.as_secs_f64(),
        rank(&times[0], 50),
        rank(&times[0], 95),
        times[0].len(),
        rank(&times[1], 50),
        rank(&times[1], 95),
        times[1].len(),
        rank(&times[2], 50),
        rank(&times[2], 95),
        times[2].len(),
        milliseconds(released.elapsed),
        released.ended.len(),
        attached.runner.agent
    );
}
