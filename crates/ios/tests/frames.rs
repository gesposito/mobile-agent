//! Task 3.5 on the device: screenshots at the native size and at a long
//! edge, the frame paths and why two of them are off, a frame for a tap, and
//! the measured minute of frames on a still screen.
//!
//! The SwiftUI fixture's List tab is in front (it must be installed); the
//! tap lands on `row-1`, at (187.5, 46) points on the iPhone SE (3rd
//! generation), and the fixture is launched again afterwards.
//! `frames_series` is the measurement, run on a quiet host:
//! `cargo test -p ios --test frames -- --ignored --nocapture frames_series`;
//! `a_dvt_connection_closed_mid_session_is_opened_again` is task 9.8's
//! device check, the same way.

#![cfg(target_os = "macos")]

mod common;

use std::process::Command;
use std::time::{Duration, Instant};

use ios::{
    Capture, FrameOptions, FramePath, Frames, ImageFormat, Input, IosDriver, LaunchOptions,
    Lifecycle, Point, ScreenshotOptions,
};

const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";
/// `row-1` of the List tab on the SE.
const ROW_1: Point = Point { x: 187.5, y: 46.0 };

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
    std::thread::sleep(Duration::from_secs(2));
}

/// This process's CPU time, every thread's, user and system.
fn cpu_time() -> Duration {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage fills the struct it is given for RUSAGE_SELF.
    let usage = unsafe {
        libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr());
        usage.assume_init()
    };
    let time = |value: libc::timeval| {
        Duration::from_secs(value.tv_sec as u64) + Duration::from_micros(value.tv_usec as u64)
    };
    time(usage.ru_utime) + time(usage.ru_stime)
}

/// usbmuxd's CPU time so far, as `ps` reads it, when it can.
fn usbmuxd_time() -> Option<Duration> {
    let pid = Command::new("pgrep")
        .args(["-x", "usbmuxd"])
        .output()
        .ok()?;
    let pid = String::from_utf8_lossy(&pid.stdout).trim().to_owned();
    let output = Command::new("ps")
        .args(["-o", "time=", "-p", &pid])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    // [[hh:]mm:]ss.cc
    let mut seconds = 0.0;
    for part in text.split(':') {
        seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(Duration::from_secs_f64(seconds))
}

#[test]
fn screenshots_frame_paths_and_a_frame_for_a_tap_come_from_the_phone() {
    let Some(udid) = common::gated("screenshots_frame_paths") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    on_the_list(&driver, &udid);

    let native = driver
        .screenshot(
            &udid,
            &ScreenshotOptions {
                max_dimension: 0,
                quality: 90,
                format: ImageFormat::Png,
            },
        )
        .expect("a screenshot");
    assert_eq!((native.width, native.height), (750, 1334));
    let scaled = driver
        .screenshot(
            &udid,
            &ScreenshotOptions {
                max_dimension: 1000,
                quality: 80,
                format: ImageFormat::Jpeg,
            },
        )
        .expect("a scaled screenshot");
    assert_eq!((scaled.width, scaled.height), (562, 1000));
    assert_eq!(&scaled.data[..2], [0xFF, 0xD8], "a JPEG");

    let paths = driver.frame_paths(&udid).expect("the frame paths");
    for status in &paths {
        eprintln!(
            "{:?}: active {}, {:?}",
            status.path, status.active, status.reason
        );
    }
    let path = |which| {
        paths
            .iter()
            .find(|status| status.path == which)
            .expect("listed")
    };
    assert!(path(FramePath::Screenshots).active);
    assert!(!path(FramePath::DisplayStream).active);
    assert!(!path(FramePath::AvFoundation).active);

    let stream = driver
        .frames(&udid, &FrameOptions::default())
        .expect("subscribed");
    let first = stream
        .next(Duration::from_secs(3))
        .expect("the first frame");
    assert_eq!((first.width, first.height), (562, 1000));
    std::thread::sleep(Duration::from_millis(500));
    while stream.try_next().is_some() {}
    let tapped = driver.tap(&udid, ROW_1).expect("the tap");
    let acknowledged = Instant::now();
    let changed = stream
        .next(Duration::from_secs(2))
        .expect("the tap's change is a frame");
    eprintln!(
        "screenshots: native PNG {} bytes in {:.0} ms, JPEG at 1000 px {} bytes in {:.0} ms; \
         first frame after {:.0} ms; the tap acknowledged in {:.1} ms, its first changed frame \
         {:.0} ms after; {:?}",
        native.data.len(),
        native.elapsed.as_secs_f64() * 1e3,
        scaled.data.len(),
        scaled.elapsed.as_secs_f64() * 1e3,
        first.arrived.duration_since(first.requested).as_secs_f64() * 1e3,
        tapped.acknowledged.as_secs_f64() * 1e3,
        changed
            .arrived
            .saturating_duration_since(acknowledged)
            .as_secs_f64()
            * 1e3,
        stream.stats()
    );
    drop(stream);
    on_the_list(&driver, &udid);
}

/// A minute of frames on the fixture's still List tab: the capture rate
/// over both connections, this process's CPU and usbmuxd's, and the frames
/// counted after the first, which a still screen must leave at none.
#[test]
#[ignore = "a measurement, run on a quiet host"]
fn frames_series() {
    let Some(udid) = common::gated("frames_series") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    on_the_list(&driver, &udid);
    let quiet = common::quiet_host("frames_series", Duration::from_secs(90 * 60));
    let before = common::load();
    let stream = driver
        .frames(&udid, &FrameOptions::default())
        .expect("subscribed");
    stream
        .next(Duration::from_secs(3))
        .expect("the first frame");
    let (cpu, usbmuxd, started) = (cpu_time(), usbmuxd_time(), Instant::now());
    let window = Duration::from_secs(60);
    let at_start = stream.stats();
    let mut counted = 0;
    while started.elapsed() < window {
        if stream
            .next(window.saturating_sub(started.elapsed()))
            .is_some()
        {
            counted += 1;
        }
    }
    let elapsed = started.elapsed();
    let cpu = cpu_time() - cpu;
    let usbmuxd = usbmuxd_time()
        .zip(usbmuxd)
        .map(|(after, before)| after - before);
    let stats = stream.stats();
    let after = common::load();
    let captures = [
        stats.captures[0] - at_start.captures[0],
        stats.captures[1] - at_start.captures[1],
    ];
    let rate = (captures[0] + captures[1]) as f64 / elapsed.as_secs_f64();
    let share = cpu.as_secs_f64() / elapsed.as_secs_f64() * 100.0;
    let usbmuxd_share = usbmuxd.map(|time| time.as_secs_f64() / elapsed.as_secs_f64() * 100.0);
    eprintln!(
        "frames over {:.1} s on a still screen: {rate:.1} captures a second (DVT {}, CoreDevice \
         {}), identical {}, decoded {}, unchanged {}, frames counted after the first {counted}; \
         CPU {share:.1}% of a core in this process, usbmuxd {usbmuxd_share:?}%; failures {} \
         {:?}; guard {} checks over {:.0} s, relaxed {}; load before {before:?}, after {after:?}",
        elapsed.as_secs_f64(),
        captures[0],
        captures[1],
        stats.identical - at_start.identical,
        stats.decoded - at_start.decoded,
        stats.unchanged - at_start.unchanged,
        stats.failures,
        stats.failure,
        quiet.checks,
        quiet.waited.as_secs_f64(),
        quiet.relaxed
    );
    assert_eq!(counted, 0, "no frame of a still screen counted");
    assert!(rate >= 15.0, "at least 15 captures a second: {rate:.1}");
    assert!(share < 25.0, "under a quarter of a core: {share:.1}%");
}

/// Physical-ios, task 9.8 (design D26), the device check: DVT's poller's
/// connection, closed mid-session through the stream's test hook as a
/// dropped connection closes, is opened again, and the poller captures again
/// within a capture interval or so of its own, CoreDevice's poller going on
/// meanwhile. Prints DVT's capture interval over five seconds before the
/// close, the time to the reopen and to the first capture after it, and the
/// stats, for the record to set beside the interval.
#[test]
#[ignore = "a device check of about ten seconds"]
fn a_dvt_connection_closed_mid_session_is_opened_again() {
    let Some(udid) = common::gated("a_dvt_connection_closed_mid_session_is_opened_again") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    on_the_list(&driver, &udid);
    let stream = driver
        .frames(&udid, &FrameOptions::default())
        .expect("subscribed");
    stream
        .next(Duration::from_secs(3))
        .expect("the first frame");
    let window = Duration::from_secs(5);
    let before = stream.stats();
    std::thread::sleep(window);
    let at_close = stream.stats();
    let captured = at_close.captures[0] - before.captures[0];
    assert!(captured > 0, "DVT's poller captures: {at_close:?}");
    let interval = window / u32::try_from(captured).unwrap_or(u32::MAX);
    let closed = Instant::now();
    stream.close_connection(Capture::Dvt);
    let until = |done: &dyn Fn(&ios::FrameStats) -> bool| {
        while closed.elapsed() < Duration::from_secs(10) {
            if done(&stream.stats()) {
                return Some(closed.elapsed());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    };
    let reopened = until(&|stats| stats.reopens[0] >= 1);
    let counted = stream.stats().captures[0];
    let captures_again = until(&|stats| stats.captures[0] > counted);
    let stats = stream.stats();
    eprintln!(
        "DVT's poller: a capture every {:.1} ms over {window:?} before the close ({captured} \
         captures); after the close, reopened at {reopened:?} and capturing again at \
         {captures_again:?}; CoreDevice's {} captures during; {stats:?}",
        interval.as_secs_f64() * 1e3,
        stats.captures[1] - at_close.captures[1],
    );
    drop(stream);
    assert_eq!(stats.reopens, [1, 0], "{stats:?}");
    assert_eq!(stats.left_out, [None, None], "{stats:?}");
    let again = captures_again.expect("DVT's poller captures again");
    assert!(
        again < Duration::from_secs(1),
        "{again:?} to capture again, against a capture every {interval:?}"
    );
    on_the_list(&driver, &udid);
}
