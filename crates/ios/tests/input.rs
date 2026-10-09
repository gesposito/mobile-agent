//! Task 3.8 on the device: input from the host through `dtuhidd`, checked
//! with the runner's tree. The SwiftUI fixture's List tab is in front: a tap
//! on `row-1` opens `detail-1`, a tap on `detail-back` returns, a resting
//! swipe moves the list by about its length, and the home button leaves for
//! the home screen.
//!
//! The runner is the installed `dev.mobileagent.runner` (spike S10's until
//! `agent-ios` replaces it), with the version `MOBILE_AGENT_IOS_RUNNER_VERSION`
//! names (`0.1.0` by default). The fixture must be installed.
//! `input_series` is the measurement, run on a quiet host:
//! `cargo test -p ios --test input -- --ignored --nocapture input_series`.

#![cfg(target_os = "macos")]

mod common;

use std::time::{Duration, Instant};

use ios::{
    AttachOptions, Button, Dispatch, Input, IosDriver, LaunchOptions, Lifecycle, Point, RunnerSpec,
    Session, Swipe,
};
use protocol::message::{Call, Reply, ScreenshotParams, SnapshotParams};
use protocol::snapshot::{Bounds, Node, Snapshot};

const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";
/// The iPhone SE (3rd generation)'s points to pixels.
const SCALE: f64 = 2.0;

fn spec() -> RunnerSpec {
    RunnerSpec::new(
        std::env::var("MOBILE_AGENT_IOS_RUNNER_VERSION").unwrap_or_else(|_| "0.1.0".to_owned()),
    )
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

fn rank(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

fn find<'a>(node: &'a Node, id: &str) -> Option<&'a Node> {
    if node.res_id.as_deref() == Some(id) {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, id))
}

/// The bounds of the node `id` in `snapshot`, in pixels.
fn bounds(snapshot: &Snapshot, id: &str) -> Option<Bounds> {
    snapshot
        .windows
        .iter()
        .filter_map(|window| window.root.as_ref())
        .find_map(|root| find(root, id))
        .map(|node| node.bounds)
}

fn center(bounds: Bounds) -> Point {
    Point::new(
        f64::from(bounds.left + bounds.right) / 2.0 / SCALE,
        f64::from(bounds.top + bounds.bottom) / 2.0 / SCALE,
    )
}

/// The tree, read again while the foreground changes during a read, as it
/// does when a tap or the home button switches apps (`window_changed`).
fn snapshot(driver: &IosDriver, udid: &str) -> Snapshot {
    loop {
        let read = driver.call(
            udid,
            Call::Snapshot(SnapshotParams::default()),
            Duration::from_secs(10),
        );
        match read {
            Ok(Reply::Snapshot(snapshot)) => return snapshot,
            Ok(other) => panic!("a snapshot was answered with {other:?}"),
            Err(error) if error.retryable() => continue,
            Err(error) => panic!("the runner reads the tree: {error}"),
        }
    }
}

/// The first snapshot within `wait` where `done` holds, and when.
fn wait_for(
    driver: &IosDriver,
    udid: &str,
    wait: Duration,
    done: impl Fn(&Snapshot) -> bool,
) -> Option<(Snapshot, Duration)> {
    let started = Instant::now();
    loop {
        let read = snapshot(driver, udid);
        if done(&read) {
            return Some((read, started.elapsed()));
        }
        if started.elapsed() >= wait {
            return None;
        }
    }
}

fn screenshot(driver: &IosDriver, udid: &str) -> Vec<u8> {
    match driver
        .call(
            udid,
            Call::Screenshot(ScreenshotParams::default()),
            Duration::from_secs(10),
        )
        .expect("the runner takes a screenshot")
    {
        Reply::Screenshot(shot) => shot.data,
        other => panic!("a screenshot was answered with {other:?}"),
    }
}

/// The fixture on its List tab, the runner attached, `row-1` on screen.
fn on_the_list(driver: &IosDriver, udid: &str) {
    driver
        .launch(
            udid,
            SWIFTUI,
            &LaunchOptions::new()
                .env("FIXTURE_INITIAL_TAB", "list")
                .terminating_running(),
        )
        .expect("the SwiftUI fixture launches; install it first");
    driver
        .attach(udid, &AttachOptions::new(spec()))
        .expect("the runner starts");
    wait_for(driver, udid, Duration::from_secs(10), |read| {
        bounds(read, "row-1").is_some()
    })
    .expect("the List tab shows row-1");
}

/// A tap on `id`, and how long until `expected` showed in the tree.
fn tap_on(
    driver: &IosDriver,
    udid: &str,
    id: &str,
    expected: &str,
) -> (Dispatch, Option<Duration>) {
    let read = snapshot(driver, udid);
    let target = bounds(&read, id).unwrap_or_else(|| panic!("{id} is on screen"));
    let dispatch = driver.tap(udid, center(target)).expect("the tap is sent");
    let reached = wait_for(driver, udid, Duration::from_secs(3), |read| {
        bounds(read, expected).is_some()
    })
    .map(|(_, after)| after);
    (dispatch, reached)
}

#[test]
fn a_tap_reaches_the_screen_and_the_home_button_the_home_screen() {
    let Some(udid) = common::gated("a_tap_reaches_the_screen") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    on_the_list(&driver, &udid);
    let (opened, reached) = tap_on(&driver, &udid, "row-1", "detail-1");
    assert!(reached.is_some(), "the tap on row-1 opened detail-1");
    assert!(
        opened.acknowledged < Duration::from_millis(150),
        "{opened:?}"
    );
    let (_, back) = tap_on(&driver, &udid, "detail-back", "row-1");
    assert!(
        back.is_some(),
        "the tap on detail-back returned to the list"
    );

    let home = driver.button(&udid, Button::Home).expect("home is pressed");
    let left = wait_for(&driver, &udid, Duration::from_secs(5), |read| {
        bounds(read, "tab-list").is_none()
    });
    assert!(left.is_some(), "the home press left the fixture");
    eprintln!(
        "tap acknowledged {:.1} ms, detail-1 in the tree {:.0} ms after; home acknowledged {:.1} ms, \
         the fixture gone from the tree {:.0} ms after",
        milliseconds(opened.acknowledged),
        reached.map_or(0.0, milliseconds),
        milliseconds(home.acknowledged),
        left.map_or(0.0, |(_, after)| milliseconds(after))
    );
    driver.release(&udid).expect("releases");
}

/// The measured series: 20 taps after 2 warm-up taps, a resting swipe, and
/// the volume buttons, with the guard's load before and after.
#[test]
#[ignore = "a measurement, run on a quiet host"]
fn input_series() {
    let Some(udid) = common::gated("input_series") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    on_the_list(&driver, &udid);
    let quiet = common::quiet_host("input_series", Duration::from_secs(90 * 60));
    let before = common::load();

    // Taps: row-1 opens detail-1, detail-back returns.
    let mut acknowledged = Vec::new();
    let mut reached = Vec::new();
    let mut missed = 0;
    for index in 0..22 {
        let (id, expected) = if index % 2 == 0 {
            ("row-1", "detail-1")
        } else {
            ("detail-back", "row-1")
        };
        let (dispatch, shown) = tap_on(&driver, &udid, id, expected);
        if index >= 2 {
            acknowledged.push(milliseconds(dispatch.acknowledged));
            match shown {
                Some(after) => reached.push(milliseconds(after)),
                None => missed += 1,
            }
        }
    }
    acknowledged.sort_by(f64::total_cmp);
    reached.sort_by(f64::total_cmp);

    // A resting swipe up the list: row-6 moves by about the stroke.
    let read = snapshot(&driver, &udid);
    let row = bounds(&read, "row-6").expect("row-6 is on screen");
    let stroke = Swipe::new(
        Point::new(187.0, 520.0),
        Point::new(187.0, 320.0),
        Duration::from_millis(300),
    )
    .resting();
    let swiped = driver.swipe(&udid, &stroke).expect("the swipe is sent");
    std::thread::sleep(Duration::from_millis(800));
    let after = snapshot(&driver, &udid);
    let moved = bounds(&after, "row-6").map(|moved| row.top - moved.top);
    let stroke_pixels = (stroke.from.y - stroke.to.y) * SCALE;

    // The volume buttons, each pressed once: whether the screen changed.
    let mut volume = Vec::new();
    for button in [Button::VolumeUp, Button::VolumeDown] {
        let still = screenshot(&driver, &udid);
        let pressed = driver.button(&udid, button).expect("the button is sent");
        let changed = (0..8).any(|_| {
            std::thread::sleep(Duration::from_millis(150));
            screenshot(&driver, &udid) != still
        });
        volume.push((button, milliseconds(pressed.acknowledged), changed));
        std::thread::sleep(Duration::from_secs(3));
    }
    let after_load = common::load();

    driver.release(&udid).expect("releases");
    eprintln!(
        "taps: n={} acknowledged p50 {:.1} / p95 {:.1} ms (min {:.1}, max {:.1}); in the tree \
         p50 {:.0} / p95 {:.0} ms after the acknowledgement; missed {missed}",
        acknowledged.len(),
        rank(&acknowledged, 50),
        rank(&acknowledged, 95),
        acknowledged[0],
        acknowledged[acknowledged.len() - 1],
        if reached.is_empty() {
            0.0
        } else {
            rank(&reached, 50)
        },
        if reached.is_empty() {
            0.0
        } else {
            rank(&reached, 95)
        },
    );
    eprintln!(
        "resting swipe: {:.0} px stroke over {:?}, acknowledged {:.1} ms, row-6 moved {moved:?} px",
        stroke_pixels,
        stroke.duration,
        milliseconds(swiped.acknowledged)
    );
    eprintln!("volume: {volume:?}");
    eprintln!(
        "guard: {} checks over {:.0} s, relaxed {}; load before {before:?}, after {after_load:?}",
        quiet.checks,
        quiet.waited.as_secs_f64(),
        quiet.relaxed
    );
    assert_eq!(missed, 0, "every tap reached the screen");
    assert!(rank(&acknowledged, 95) < 150.0, "p95 under 150 ms");
    let moved = f64::from(moved.expect("row-6 is still on screen"));
    assert!(
        (0.8..=1.1).contains(&(moved / stroke_pixels)),
        "the list moved {moved} px for a {stroke_pixels} px stroke"
    );
}
