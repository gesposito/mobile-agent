//! Task 3.8 on the device: input from the host through `dtuhidd`, checked
//! with the runner's tree. The SwiftUI fixture's List tab is in front: a tap
//! on `row-1` opens `detail-1`, a tap on `detail-back` returns, a resting
//! swipe moves the list by about its length, and the home button leaves for
//! the home screen.
//!
//! The runner is `agent-ios`'s, installed from the bundle
//! `MOBILE_AGENT_IOS_RUNNER_APP` names when the phone has another version.
//! The fixture must be installed.
//! `input_series` is the measurement, run on a quiet host; it checks the taps
//! with frames and the swipe with screenshots, so it needs no runner:
//! `cargo test -p ios --test input -- --ignored --nocapture input_series`.

#![cfg(target_os = "macos")]

mod common;

use std::time::{Duration, Instant};

use ios::{
    AttachOptions, Button, Dispatch, FrameOptions, Frames, ImageFormat, Input, IosDriver,
    LaunchOptions, Lifecycle, Point, ScreenshotOptions, Session, Swipe, decode_png,
};
use protocol::message::{Call, Reply, SnapshotParams};
use protocol::snapshot::{Bounds, Node, Snapshot};

const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";
/// The iPhone SE (3rd generation)'s points to pixels.
const SCALE: f64 = 2.0;

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
        .attach(udid, &AttachOptions::new(common::runner()))
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

/// The tab bar's Login and List buttons on the SE, in points.
const LOGIN_TAB: Point = Point { x: 62.0, y: 606.0 };
const LIST_TAB: Point = Point { x: 124.5, y: 606.0 };

/// The screen now, as packed BGRA of 750 by 1334.
fn pixels(driver: &IosDriver, udid: &str) -> Vec<u8> {
    let shot = driver
        .screenshot(
            udid,
            &ScreenshotOptions {
                max_dimension: 0,
                quality: 90,
                format: ImageFormat::Png,
            },
        )
        .expect("a screenshot");
    let mut out = Vec::new();
    assert_eq!(decode_png(&shot.data, &mut out), Ok((750, 1334)));
    out
}

/// How far the list moved up between `before` and `after`, in pixels: the
/// shift of the rows' text column (x 20 to 300) that matches best, among
/// 0 to 850 rows, over the list's area above the tab bar; a list that flung
/// further matches none well, which the caller reads as a fling.
fn shift(before: &[u8], after: &[u8]) -> usize {
    let (width, top, bottom) = (750, 100, 1100);
    let green = |frame: &[u8], x: usize, y: usize| i32::from(frame[(y * width + x) * 4 + 1]);
    (0..=850)
        .filter(|shift| bottom - shift > top + 140)
        .min_by_key(|&shift| {
            let mut total = 0i64;
            let mut count = 0i64;
            for y in (top..bottom - shift).step_by(2) {
                for x in (20..300).step_by(4) {
                    total += i64::from((green(after, x, y) - green(before, x, y + shift)).abs());
                    count += 1;
                }
            }
            total * 1000 / count.max(1)
        })
        .expect("a shift")
}

/// Waits until `stream` delivers no frame for 400 ms, at most 3 s: the
/// screen is still.
fn settle(stream: &ios::FrameStream) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(3) {
        if stream.next(Duration::from_millis(400)).is_none() {
            return;
        }
    }
}

/// The first frame captured after `requested`, within `wait`.
fn changed_after(
    stream: &ios::FrameStream,
    requested: Instant,
    wait: Duration,
) -> Option<ios::Frame> {
    let deadline = Instant::now() + wait;
    loop {
        let frame = stream.next(deadline.saturating_duration_since(Instant::now()))?;
        if frame.requested >= requested {
            return Some(frame);
        }
    }
}

/// The measured series: 20 taps after 2 warm-up taps, alternating the Login
/// and List tabs, each checked by a changed frame; a resting swipe up the
/// list, its travel measured from two screenshots; and the volume buttons.
/// The guard's load is recorded before and after.
#[test]
#[ignore = "a measurement, run on a quiet host"]
fn input_series() {
    let Some(udid) = common::gated("input_series") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    driver
        .launch(
            &udid,
            SWIFTUI,
            &LaunchOptions::new()
                .env("FIXTURE_INITIAL_TAB", "list")
                .terminating_running(),
        )
        .expect("the SwiftUI fixture launches; install it first");
    std::thread::sleep(Duration::from_secs(2));
    let quiet = common::quiet_host("input_series", Duration::from_secs(90 * 60));
    let before = common::load();
    let stream = driver
        .frames(&udid, &FrameOptions::default())
        .expect("subscribed");
    stream
        .next(Duration::from_secs(3))
        .expect("the first frame");

    // Taps: Login, then List, each changing the screen.
    let mut acknowledged = Vec::new();
    let mut changed = Vec::new();
    let mut missed = 0;
    for index in 0..22 {
        settle(&stream);
        let point = if index % 2 == 0 { LOGIN_TAB } else { LIST_TAB };
        let requested = Instant::now();
        let dispatch = driver.tap(&udid, point).expect("the tap is sent");
        let frame = changed_after(&stream, requested, Duration::from_secs(2));
        if index >= 2 {
            acknowledged.push(milliseconds(dispatch.acknowledged));
            match frame {
                Some(frame) => {
                    changed.push(milliseconds(
                        frame.arrived.saturating_duration_since(requested),
                    ));
                }
                None => missed += 1,
            }
        }
    }
    acknowledged.sort_by(f64::total_cmp);
    changed.sort_by(f64::total_cmp);
    drop(stream);

    // A resting swipe up the list, which the last tap left in front.
    std::thread::sleep(Duration::from_secs(1));
    let still = pixels(&driver, &udid);
    let stroke = Swipe::new(
        Point::new(187.0, 520.0),
        Point::new(187.0, 320.0),
        Duration::from_millis(300),
    )
    .resting();
    let swiped = driver.swipe(&udid, &stroke).expect("the swipe is sent");
    std::thread::sleep(Duration::from_millis(1500));
    let moved = shift(&still, &pixels(&driver, &udid));
    let stroke_pixels = ((stroke.from.y - stroke.to.y) * SCALE) as usize;

    // The volume buttons, each pressed once: whether the screen changed.
    let mut volume = Vec::new();
    for button in [Button::VolumeUp, Button::VolumeDown] {
        std::thread::sleep(Duration::from_secs(3));
        let stream = driver
            .frames(&udid, &FrameOptions::default())
            .expect("subscribed");
        stream
            .next(Duration::from_secs(3))
            .expect("the first frame");
        std::thread::sleep(Duration::from_millis(500));
        while stream.try_next().is_some() {}
        let pressed = driver.button(&udid, button).expect("the button is sent");
        let shown = stream.next(Duration::from_secs(2)).is_some();
        volume.push((button, milliseconds(pressed.acknowledged), shown));
    }
    let after_load = common::load();
    eprintln!(
        "taps: n={} acknowledged p50 {:.1} / p95 {:.1} ms (min {:.1}, max {:.1}); first changed \
         frame captured after the request p50 {:.0} / p95 {:.0} ms after it; missed {missed}",
        acknowledged.len(),
        rank(&acknowledged, 50),
        rank(&acknowledged, 95),
        acknowledged[0],
        acknowledged[acknowledged.len() - 1],
        if changed.is_empty() {
            0.0
        } else {
            rank(&changed, 50)
        },
        if changed.is_empty() {
            0.0
        } else {
            rank(&changed, 95)
        },
    );
    eprintln!(
        "resting swipe: a {stroke_pixels} px stroke over {:?}, acknowledged {:.1} ms; the list \
         moved {moved} px",
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
    let ratio = moved as f64 / stroke_pixels as f64;
    assert!(
        (0.8..=1.1).contains(&ratio),
        "the list moved {moved} px for a {stroke_pixels} px stroke"
    );
}

/// The lock button as an Indigo button event, with a person at the phone:
/// whether the phone locks, read from lockdown's `PasswordProtected`, then a
/// wait of up to ten minutes for the person to unlock it.
#[test]
#[ignore = "locks the phone; needs a person to unlock it"]
fn the_lock_button() {
    let Some(udid) = common::gated("the_lock_button") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    let unlocked = |driver: &IosDriver| {
        use ios::Devices;
        driver.require(&udid, ios::Need::Unlocked).is_ok()
    };
    assert!(unlocked(&driver), "the phone is unlocked before the press");
    let pressed = driver
        .button(&udid, Button::Lock)
        .expect("the press is sent");
    let started = Instant::now();
    let mut locked_after = None;
    while started.elapsed() < Duration::from_secs(5) {
        if !unlocked(&driver) {
            locked_after = Some(started.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    eprintln!(
        "lock button: acknowledged in {:.1} ms; the phone locked: {}",
        milliseconds(pressed.acknowledged),
        locked_after.map_or("no, within 5 s".to_owned(), |after| format!(
            "yes, PasswordProtected true {:.0} ms after the press",
            milliseconds(after)
        ))
    );
    if locked_after.is_some() {
        eprintln!("lock button: UNLOCK THE PHONE NOW (waiting up to ten minutes)");
        let asked = Instant::now();
        while !unlocked(&driver) {
            assert!(asked.elapsed() < Duration::from_secs(600), "not unlocked");
            std::thread::sleep(Duration::from_millis(500));
        }
        eprintln!(
            "lock button: unlocked {:.0} s after the request",
            asked.elapsed().as_secs_f64()
        );
    }
}
