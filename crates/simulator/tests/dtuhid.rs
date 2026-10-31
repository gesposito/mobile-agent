//! Task 6.4: the dtuhidd XPC transport (simulator-driver spec, "Input
//! transport selection" and "Input actions"; design D6, spike S3).

#![cfg(target_os = "macos")]

mod common;

use std::time::{Duration, Instant};

use simulator::{
    Button, DriverOptions, Input, Key, Lifecycle, SimulatorDriver, TransportChoice, TransportKind,
};

use common::{ExpoOnLogin, expo};

fn dtuhid() -> SimulatorDriver {
    SimulatorDriver::open_with(DriverOptions {
        transport: TransportChoice::Only(TransportKind::Dtuhid),
        ..DriverOptions::default()
    })
    .expect("the driver opens")
}

/// Nearest-rank percentile of ascending samples.
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let rank = (p * sorted.len() as f64 / 100.0).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[test]
fn a_tap_lands_on_the_fixture_button() {
    let Some(udid) = common::gated("a_tap_lands_on_the_fixture_button") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = dtuhid();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_expo(&driver, &udid, "login");
    let (stream, settled) = common::settle(&driver, &udid);
    let before = common::raw_screenshot(&driver, &udid);

    let blank = driver.tap(&udid, expo::BLANK).expect("a tap");
    assert_eq!(blank.transport, TransportKind::Dtuhid);
    assert!(
        common::wait_for_change(&stream, &settled, Duration::from_secs(1)).is_none(),
        "a tap on blank space changed the screen"
    );

    let dispatch = driver.tap(&udid, expo::SIGN_IN).expect("a tap");
    assert_eq!(dispatch.transport, TransportKind::Dtuhid);
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
        "dtuhidd tap: dispatched in {:?}, completed in {:?}; status line changed by {status:.2}, fields by {fields:.2}",
        dispatch.dispatched, dispatch.completed
    );
    assert!(status > 1.0, "the status line changed ({status:.2})");
    assert!(fields < 0.5, "the fields did not change ({fields:.2})");
}

/// A tap requested as the session opens waits for dtuhidd's activation
/// reply, goes out the moment it arrives, and reports the wait.
fn tap_before_activation(driver: &SimulatorDriver, udid: &str) -> simulator::Dispatch {
    common::show_expo(driver, udid, "login");
    let (stream, settled) = common::settle(driver, udid);
    let dispatch = driver
        .tap(udid, expo::TAB_LIST)
        .expect("the tap is delivered after activation");
    assert_eq!(dispatch.transport, TransportKind::Dtuhid);
    let wait = dispatch
        .activation_wait
        .expect("the tap was requested before activation, so it waited");
    assert!(
        dispatch.dispatched >= wait,
        "dispatched {:?} before activation completed ({wait:?})",
        dispatch.dispatched
    );
    // The queue must go out at once.
    common::under(
        "the held tap's flush after the activation reply",
        dispatch.dispatched - wait,
        Duration::from_millis(5),
    );
    assert!(
        common::wait_for_change(&stream, &settled, Duration::from_secs(3)).is_some(),
        "the held tap on the List tab changed nothing"
    );
    let status = driver.status(udid).expect("status");
    let input = status.input.expect("an input session");
    assert_eq!(input.transport, TransportKind::Dtuhid);
    assert_eq!(input.connections, 1);
    assert!(input.activation.is_some());
    dispatch
}

#[test]
fn input_before_activation() {
    let Some(udid) = common::gated("input_before_activation") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = dtuhid();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    let dispatch = tap_before_activation(&driver, &udid);
    eprintln!(
        "dtuhidd, warm daemon: activation waited {:?}, the held tap went out {:?} after the request",
        dispatch.activation_wait.unwrap_or_default(),
        dispatch.dispatched
    );
}

#[test]
fn the_first_session_of_a_boot_waits_for_dtuhidd_to_start() {
    let Some(_) = common::gated("the_first_session_of_a_boot_waits_for_dtuhidd_to_start") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = dtuhid();
    let udid = common::fresh_device(&driver);
    if common::dtuhidd_started(&udid) {
        eprintln!(
            "the_first_session_of_a_boot_waits_for_dtuhidd_to_start: skipped; dtuhidd already \
             ran on {udid}'s current boot, so it is not fresh"
        );
        return;
    }
    let dispatch = tap_before_activation(&driver, &udid);
    let wait = dispatch.activation_wait.unwrap_or_default();
    eprintln!(
        "dtuhidd, first session of a boot: activation waited {wait:?}, the held tap went out {:?} after the request",
        dispatch.dispatched
    );
    // S3: 267 to 333 ms while launchd starts the daemon, against about 9 ms
    // once it runs.
    assert!(
        wait > Duration::from_millis(50),
        "a cold daemon answered in {wait:?}"
    );
}

#[test]
fn warm_tap_latency() {
    let Some(udid) = common::gated("warm_tap_latency") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = dtuhid();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_expo(&driver, &udid, "login");
    // Warm the session: it has dispatched input before the measured taps.
    driver.tap(&udid, expo::BLANK).expect("a first tap");
    let mut dispatched = Vec::new();
    let mut completed = Vec::new();
    for _ in 0..100 {
        let dispatch = driver.tap(&udid, expo::BLANK).expect("a warm tap");
        assert_eq!(dispatch.activation_wait, None, "the session was active");
        dispatched.push(dispatch.dispatched);
        completed.push(dispatch.completed);
        std::thread::sleep(Duration::from_millis(20));
    }
    dispatched.sort();
    completed.sort();
    eprintln!(
        "dtuhidd warm tap over 100: touch-down handed to XPC p50 {:?} p95 {:?} max {:?}; \
         whole tap sent (XPC barrier) p50 {:?} p95 {:?} max {:?}",
        percentile(&dispatched, 50.0),
        percentile(&dispatched, 95.0),
        dispatched[99],
        percentile(&completed, 50.0),
        percentile(&completed, 95.0),
        completed[99],
    );
    common::under(
        "the warm touch-down's hand-off at p95",
        percentile(&dispatched, 95.0),
        Duration::from_millis(20),
    );
    common::under(
        "the warm tap's whole sending at p95",
        percentile(&completed, 95.0),
        Duration::from_millis(20),
    );
}

#[test]
fn one_connection_carries_touch_keys_and_buttons() {
    let Some(udid) = common::gated("one_connection_carries_touch_keys_and_buttons") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = dtuhid();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_expo(&driver, &udid, "login");

    // A touch: focus the username field, then let the keyboard slide in.
    driver.tap(&udid, expo::USERNAME).expect("a tap");
    std::thread::sleep(Duration::from_millis(1500));
    let focused = common::raw_screenshot(&driver, &udid);

    // Keys: three characters appear in the field.
    for character in ['a', 'b', 'c'] {
        let key = Key::character(character).expect("a usage");
        let dispatch = driver.key(&udid, key).expect("a key");
        assert_eq!(dispatch.transport, TransportKind::Dtuhid);
        std::thread::sleep(Duration::from_millis(60));
    }
    std::thread::sleep(Duration::from_millis(500));
    let typed = common::raw_screenshot(&driver, &udid);
    let field = common::difference(&focused, &typed, (95.0, 130.0));
    assert!(field > 0.5, "the typed text changed the field ({field:.2})");

    // A button: Home sends the fixture to the background. The focused
    // field's cursor keeps blinking, so the screens are compared whole.
    let home = driver.button(&udid, Button::Home).expect("the Home button");
    assert_eq!(home.transport, TransportKind::Dtuhid);
    std::thread::sleep(Duration::from_millis(1500));
    let home_screen = common::raw_screenshot(&driver, &udid);
    let changed = common::difference(&typed, &home_screen, (80.0, 800.0));
    assert!(changed > 10.0, "Home changed the screen by {changed:.2}");

    let input = driver.status(&udid).expect("status").input.expect("input");
    assert_eq!(input.transport, TransportKind::Dtuhid);
    assert_eq!(input.connections, 1, "one connection carried all of it");
}

/// Task 6.13: the input session opens and dtuhidd activates without any
/// input, so that the daemon's `open` can activate it while the app
/// launches and the first click waits for nothing.
#[test]
fn the_input_session_opens_and_activates_without_input() {
    let Some(udid) = common::gated("the_input_session_opens_and_activates_without_input") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = dtuhid();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_expo(&driver, &udid, "login");
    let (stream, settled) = common::settle(&driver, &udid);
    assert_eq!(
        driver.status(&udid).expect("status").input,
        None,
        "a new driver has no session yet"
    );

    let start = Instant::now();
    let status = driver
        .open_input(&udid, Duration::from_secs(5))
        .expect("the input session opens");
    let took = start.elapsed();
    eprintln!("the input session opened and activated in {took:?}: {status:?}");
    assert_eq!(status.transport, TransportKind::Dtuhid);
    let activation = status.activation.expect("dtuhidd answered within the wait");
    assert!(activation <= took);
    assert_eq!(status.connections, 1);
    assert_eq!(
        driver.status(&udid).expect("status").input,
        Some(status.clone())
    );
    assert!(
        common::wait_for_change(&stream, &settled, Duration::from_secs(1)).is_none(),
        "opening the session sent no input"
    );

    // Opening it again keeps the session and does not wait.
    let start = Instant::now();
    let again = driver
        .open_input(&udid, Duration::from_secs(5))
        .expect("the open session is kept");
    common::under(
        "opening the open session again",
        start.elapsed(),
        Duration::from_millis(50),
    );
    assert_eq!(again, status);

    // The first input waits for nothing.
    let tap = driver.tap(&udid, expo::BLANK).expect("a tap");
    eprintln!("the first tap after it dispatched in {:?}", tap.dispatched);
    assert_eq!(tap.activation_wait, None, "the session was already active");
    common::under(
        "the first tap's dispatch",
        tap.dispatched,
        Duration::from_millis(20),
    );
}

/// A session opened without a wait returns at once and activates on its
/// own, as the daemon's `open` uses it.
#[test]
fn an_input_session_opened_without_waiting_activates_on_its_own() {
    let Some(udid) = common::gated("an_input_session_opened_without_waiting_activates_on_its_own")
    else {
        return;
    };
    let _serial = common::exclusive();
    let driver = dtuhid();
    let start = Instant::now();
    let status = driver
        .open_input(&udid, Duration::ZERO)
        .expect("the input session opens");
    let returned = start.elapsed();
    eprintln!("open_input without a wait returned in {returned:?}: {status:?}");
    assert_eq!(status.transport, TransportKind::Dtuhid);
    common::under(
        "open_input without a wait",
        returned,
        Duration::from_millis(500),
    );
    let active = common::wait_for(Duration::from_secs(5), || {
        driver
            .status(&udid)
            .is_ok_and(|status| status.input.is_some_and(|input| input.activation.is_some()))
    });
    assert!(active, "dtuhidd answered the probe on its own");
}
