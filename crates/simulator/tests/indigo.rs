//! Task 6.3: the legacy Indigo touch transport. This host's CoreSimulator
//! (1171.7) would choose dtuhidd, so the tests force Indigo, which spike S3
//! found delivering touches beside dtuhidd.

#![cfg(target_os = "macos")]

mod common;

use std::time::Duration;

use simulator::{
    DriverOptions, Input, Point, SimulatorDriver, Swipe, TransportChoice, TransportKind,
};

use common::{ExpoOnLogin, expo};

fn indigo() -> SimulatorDriver {
    SimulatorDriver::open_with(DriverOptions {
        transport: TransportChoice::Only(TransportKind::Indigo),
    })
    .expect("the driver opens")
}

#[test]
fn a_tap_lands_on_the_fixture_button() {
    let Some(udid) = common::gated("a_tap_lands_on_the_fixture_button") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = indigo();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
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
    let Some(udid) = common::gated("a_swipe_scrolls_the_list") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = indigo();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_expo(&driver, &udid, "list");
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
    );
    let after = common::raw_screenshot(&driver, &udid);
    let (shift, best, unshifted) = common::vertical_shift(&before, &after, expo::LIST_ROWS);
    let points = shift as f64 * 874.0 / after.height as f64;
    eprintln!(
        "Indigo swipe: the list moved {shift} px ({points:.0} points); difference {best:.2} there, {unshifted:.2} unshifted"
    );
    assert!(points > 100.0, "the list moved only {points:.0} points");
    assert!(
        best < unshifted * 0.3,
        "the list did not move as a whole: {best:.2} at the best shift, {unshifted:.2} unshifted"
    );
}
