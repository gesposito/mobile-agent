//! Task 6.3: the legacy Indigo touch transport. This host's CoreSimulator
//! (1171.7) would choose dtuhidd, so the tests force Indigo. They run on a
//! device booted for them: on the long-running shared Simulator, whose
//! dtuhidd had served many sessions, Indigo touches did not land even with
//! spike S3's 40 ms hold, while on a fresh boot they did, as in S3.

#![cfg(target_os = "macos")]

mod common;

use std::time::Duration;

use simulator::{
    DriverOptions, Input, Point, SimulatorDriver, Swipe, TransportChoice, TransportKind,
};

use common::expo;

fn indigo() -> SimulatorDriver {
    SimulatorDriver::open_with(DriverOptions {
        transport: TransportChoice::Only(TransportKind::Indigo),
        ..DriverOptions::default()
    })
    .expect("the driver opens")
}

#[test]
fn a_tap_lands_on_the_fixture_button() {
    let Some(_) = common::gated("a_tap_lands_on_the_fixture_button") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = indigo();
    let udid = common::fresh_device(&driver);
    // On a host loaded past 100 a late redraw of the fresh device's first
    // screen counted as the blank tap's doing.
    common::wait_for_quiet_host(8.0, Duration::from_secs(90));
    common::show_expo(&driver, &udid, "login");
    let (stream, settled) = common::settle(&driver, &udid);
    let before = common::raw_screenshot(&driver, &udid);

    // A tap on blank space changes nothing.
    let blank = driver.tap(&udid, expo::BLANK).expect("a tap");
    assert_eq!(blank.transport, TransportKind::Indigo);
    assert!(
        common::wait_for_change(&stream, &settled, Duration::from_secs(1)).is_none(),
        "a tap on blank space changed the screen"
    );

    // A tap on Sign in turns "Signed out" into "Invalid credentials".
    let dispatch = driver.tap(&udid, expo::SIGN_IN).expect("a tap");
    assert_eq!(dispatch.transport, TransportKind::Indigo);
    assert!(dispatch.completed >= dispatch.dispatched);
    assert!(
        common::wait_for_change(&stream, &settled, Duration::from_secs(2)).is_some(),
        "the tap on Sign in changed nothing"
    );
    common::drain_until_quiet(
        &stream,
        None,
        Duration::from_millis(500),
        Duration::from_secs(10),
    );
    let after = common::raw_screenshot(&driver, &udid);
    let status = common::difference(&before, &after, expo::STATUS_ROWS);
    let fields = common::difference(&before, &after, expo::FIELD_ROWS);
    eprintln!(
        "Indigo tap: dispatched in {:?}, completed in {:?}; status line changed by {status:.2}, fields by {fields:.2}",
        dispatch.dispatched, dispatch.completed
    );
    assert!(status > 1.0, "the status line changed ({status:.2})");
    assert!(fields < 0.5, "the fields did not change ({fields:.2})");
}

#[test]
fn a_swipe_scrolls_the_list() {
    let Some(_) = common::gated("a_swipe_scrolls_the_list") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = indigo();
    let udid = common::fresh_device(&driver);
    common::show_expo(&driver, &udid, "list");
    // The drag's 23 events keep 17 ms apart through SimulatorKit's builder;
    // on a host loaded past 100 their timing broke up and the list either
    // barely moved or flung, so it waits for a quieter host, as the frame
    // CPU test does.
    let load = common::wait_for_quiet_host(8.0, Duration::from_secs(90));
    let (stream, _) = common::settle(&driver, &udid);
    let before = common::raw_screenshot(&driver, &udid);

    // A slow drag of 250 points, so the list moves about that far and the
    // two screens still overlap.
    let swipe = Swipe::new(
        Point::new(201.0, 650.0),
        Point::new(201.0, 400.0),
        Duration::from_millis(1000),
    );
    let dispatch = driver.swipe(&udid, &swipe).expect("a swipe");
    assert_eq!(dispatch.transport, TransportKind::Indigo);
    assert_eq!(dispatch.events, 23);
    common::drain_until_quiet(
        &stream,
        None,
        Duration::from_millis(700),
        Duration::from_secs(10),
    )
    .expect("the list moved");
    let after = common::raw_screenshot(&driver, &udid);
    let (shift, best, unshifted) = common::vertical_shift(&before, &after, expo::LIST_ROWS);
    let points = shift as f64 * 874.0 / after.height as f64;
    eprintln!(
        "Indigo swipe at load {load:.1}: the list moved {shift} px ({points:.0} points); difference {best:.2} there, {unshifted:.2} unshifted"
    );
    assert!(points > 100.0, "the list moved only {points:.0} points");
    assert!(
        best < unshifted * 0.3,
        "the list did not move as a whole: {best:.2} at the best shift, {unshifted:.2} unshifted"
    );
}
