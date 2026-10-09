//! Tasks 3.4 and 3.6 on the device, with a person at the phone: a runner
//! session live, the USB cable pulled, the next command failing as
//! `disconnected` with its cause, and after the replug a new attach that
//! starts over, ending the runner process the pull left behind (S10 saw it
//! outlive the pull).
//!
//! `cargo test -p ios --test containment -- --ignored --nocapture`: the test
//! says on standard error when to pull the cable and when to plug it back,
//! and waits up to ten minutes for each.

#![cfg(target_os = "macos")]

mod common;

use std::time::{Duration, Instant};

use ios::{
    AttachOptions, Devices, Input, IosDriver, Lifecycle, Point, RUNNER_BUNDLE_ID, RunnerSpec,
    RunnerState, Session,
};
use protocol::message::{Call, SnapshotParams};

const PERSON: Duration = Duration::from_secs(600);

fn spec() -> RunnerSpec {
    RunnerSpec::new(
        std::env::var("MOBILE_AGENT_IOS_RUNNER_VERSION").unwrap_or_else(|_| "0.1.0".to_owned()),
    )
}

fn snapshot() -> Call {
    Call::Snapshot(SnapshotParams::default())
}

#[test]
#[ignore = "needs a person to pull and replug the cable"]
fn a_pulled_cable_is_reported_and_a_replug_attaches_again() {
    let Some(udid) = common::gated("a_pulled_cable_is_reported") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    let first = driver
        .attach(&udid, &AttachOptions::new(spec()))
        .expect("the runner starts");
    assert_eq!(first.runner.state, RunnerState::Ready);
    driver
        .call(&udid, snapshot(), Duration::from_secs(10))
        .expect("the runner serves");
    let pid = driver
        .running(&udid, RUNNER_BUNDLE_ID)
        .expect("the process list")
        .expect("the runner runs");

    eprintln!("containment: PULL THE CABLE NOW (waiting up to ten minutes)");
    let asked = Instant::now();
    let error = loop {
        assert!(asked.elapsed() < PERSON, "the cable was not pulled");
        match driver.call(&udid, snapshot(), Duration::from_secs(5)) {
            Ok(_) => std::thread::sleep(Duration::from_millis(300)),
            Err(error) => break error,
        }
    };
    let noticed = asked.elapsed();
    let tapped = driver.tap(&udid, Point::new(187.0, 300.0));
    eprintln!(
        "containment: the command after the pull failed with {} after {:.1} s: {error}; a tap then \
         gave {:?}",
        error.code(),
        noticed.as_secs_f64(),
        tapped.as_ref().map_err(|error| error.code())
    );
    assert_eq!(error.code(), "disconnected", "{error}");

    eprintln!("containment: PLUG THE CABLE BACK IN NOW (waiting up to ten minutes)");
    let asked = Instant::now();
    loop {
        assert!(asked.elapsed() < PERSON, "the phone did not come back");
        let listed = driver
            .devices()
            .map(|devices| devices.iter().any(|device| device.udid == udid))
            .unwrap_or(false);
        if listed {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let listed = asked.elapsed();
    // The phone answers lockdown a moment after usbmuxd lists it.
    let attaching = Instant::now();
    let again = loop {
        match driver.attach(&udid, &AttachOptions::new(spec())) {
            Ok(attached) => break attached,
            Err(error) if attaching.elapsed() < Duration::from_secs(60) => {
                eprintln!("containment: attach not yet: {}", error.code());
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(error) => panic!("the new attach failed: {error}"),
        }
    };
    let tap = driver
        .tap(&udid, Point::new(187.0, 5.0))
        .expect("input again");
    driver
        .call(&udid, snapshot(), Duration::from_secs(10))
        .expect("the new runner serves");
    eprintln!(
        "containment: listed again {:.1} s after the request to replug; the new attach took \
         {:.0} ms (hello {:.0} ms after its start), ended leftovers {:?} (the old runner was {}), \
         restarts {}; a tap then acknowledged in {:.1} ms",
        listed.as_secs_f64(),
        again.elapsed.as_secs_f64() * 1e3,
        again
            .runner
            .started_in
            .map_or(0.0, |started| started.as_secs_f64() * 1e3),
        again.runner.leftovers,
        pid,
        again.runner.restarts,
        tap.acknowledged.as_secs_f64() * 1e3
    );
    assert_eq!(again.runner.state, RunnerState::Ready);
    assert_eq!(again.runner.restarts, 0, "a new session");
    driver.release(&udid).expect("releases");
}
