//! Task 6.2: frames from the display surface (simulator-driver spec, "Frames
//! and screenshots"; design D7).

#![cfg(target_os = "macos")]

mod common;

use std::time::{Duration, Instant, SystemTime};

use simulator::{FrameOptions, Frames, LaunchOptions, Lifecycle, ScreenshotOptions};

use common::ExpoOnLogin;

#[test]
fn frame_stream_during_animation() {
    let Some(udid) = common::gated("frame_stream_during_animation") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = common::driver();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_expo(&driver, &udid, "login");

    let stream = driver
        .frames(&udid, &FrameOptions::default())
        .expect("the display is subscribed");
    let first = stream
        .next(Duration::from_secs(2))
        .expect("the frame on screen arrives at once");
    assert_eq!(first.sequence, 1);
    let settled = common::drain_until_quiet(
        &stream,
        Some(first),
        Duration::from_millis(800),
        Duration::from_secs(10),
    )
    .expect("the first frame at least");

    // The transition: the fixture relaunches onto its Anim tab, whose
    // spinner then turns without end.
    driver
        .launch(
            &udid,
            common::EXPO_BUNDLE,
            &LaunchOptions::new()
                .env("FIXTURE_INITIAL_TAB", "anim")
                .terminating_running(),
        )
        .expect("the fixture relaunches");
    let during = common::collect(&stream, Duration::from_millis(1500));
    assert!(
        during.len() >= 20,
        "only {} frames arrived while the display changed",
        during.len()
    );
    let mut previous = settled;
    for frame in &during {
        assert_eq!(frame.sequence, previous.sequence + 1, "no gaps");
        assert!(frame.captured >= previous.captured, "in capture order");
        assert_ne!(frame.hash, previous.hash, "each frame changed pixels");
        let jpeg = frame.jpeg.as_deref().expect("frames are encoded");
        assert_eq!(
            common::jpeg_dimensions(jpeg),
            Some((frame.width, frame.height))
        );
        previous = frame.clone();
    }
    let stats = stream.stats();
    assert!(stats.presents >= stats.reads, "{stats:?}");
    assert_eq!(stats.delivered, previous.sequence, "{stats:?}");
}

#[test]
fn frames_stop_when_the_display_is_static() {
    let Some(udid) = common::gated("frames_stop_when_the_display_is_static") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = common::driver();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_expo(&driver, &udid, "anim");
    let stream = driver
        .frames(&udid, &FrameOptions::default())
        .expect("the display is subscribed");
    common::show_expo(&driver, &udid, "login");
    let settled = common::drain_until_quiet(
        &stream,
        None,
        Duration::from_millis(800),
        Duration::from_secs(10),
    )
    .expect("frames arrived while the fixture relaunched");

    let late = common::collect(&stream, Duration::from_secs(3));
    // A static screen still changes once a minute, when the status bar clock
    // ticks; nothing below the status bar may change.
    assert!(
        late.len() <= 1,
        "{} frames arrived from a static display",
        late.len()
    );
    for frame in &late {
        assert_eq!(
            frame.content_hash, settled.content_hash,
            "only the status bar changed"
        );
    }
}

/// One measured window of a subscription.
struct Window {
    fps: f64,
    pipeline: f64,
    process: f64,
    line: String,
}

fn measure(stream: &simulator::FrameStream, length: Duration) -> Window {
    let before = stream.stats();
    let cpu_before = common::process_cpu();
    let start = Instant::now();
    let frames = common::collect(stream, length);
    let elapsed = start.elapsed().as_secs_f64();
    let process = (common::process_cpu() - cpu_before).as_secs_f64() / elapsed;
    let after = stream.stats();
    let pipeline = (after.cpu - before.cpu).as_secs_f64() / elapsed;
    let fps = frames.len() as f64 / elapsed;
    let sizes: Vec<usize> = frames
        .iter()
        .filter_map(|f| f.jpeg.as_ref())
        .map(Vec::len)
        .collect();
    let line = format!(
        "{fps:.1} fps at {}x{} with {}, pipeline {:.1}% and process {:.1}% of one core; \
         {} presents, {} reads, {} torn, {} dropped; mean JPEG {} bytes",
        frames.first().map_or(0, |f| f.width),
        frames.first().map_or(0, |f| f.height),
        after.encoder,
        pipeline * 100.0,
        process * 100.0,
        after.presents - before.presents,
        after.reads - before.reads,
        after.torn - before.torn,
        after.dropped - before.dropped,
        sizes.iter().sum::<usize>() / sizes.len().max(1),
    );
    Window {
        fps,
        pipeline,
        process,
        line,
    }
}

/// The Anim tab's spinner changes on every present, so every read the
/// throttle lets through is a new frame. CPU time is per core-second, and
/// other processes on a busy host inflate it (the endpoint security software scanning a fresh build
/// took one run from 12 to 24 percent), so up to three windows are measured
/// and the gate holds when one of them meets it; every window is printed.
#[test]
fn frames_sustain_30_fps_under_a_fifth_of_a_core() {
    let Some(udid) = common::gated("frames_sustain_30_fps_under_a_fifth_of_a_core") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = common::driver();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_expo(&driver, &udid, "anim");
    let options = FrameOptions {
        max_fps: 30,
        max_dimension: 1000,
        quality: 80,
        encode: true,
    };
    let stream = driver.frames(&udid, &options).expect("subscribed");
    common::collect(&stream, Duration::from_millis(500));

    let mut windows = Vec::new();
    for _ in 0..3 {
        let window = measure(&stream, Duration::from_secs(4));
        eprintln!("{}", window.line);
        let met = window.fps >= 28.0 && window.pipeline < 0.20 && window.process < 0.20;
        windows.push(window);
        if met {
            return;
        }
    }
    let lines: Vec<&str> = windows.iter().map(|w| w.line.as_str()).collect();
    panic!(
        "no window held 28 fps under 20 percent of one core:\n{}",
        lines.join("\n")
    );
}

#[test]
fn a_screenshot_is_timestamped_at_the_display_size() {
    let Some(udid) = common::gated("a_screenshot_is_timestamped_at_the_display_size") else {
        return;
    };
    let driver = common::driver();
    let iphone_17 = driver
        .devices()
        .expect("devices")
        .into_iter()
        .find(|d| d.udid.eq_ignore_ascii_case(&udid))
        .and_then(|d| d.device_type)
        .is_some_and(|t| t.name == "iPhone 17");

    let before = SystemTime::now();
    let native = driver
        .screenshot(&udid, &ScreenshotOptions::default())
        .expect("a screenshot");
    assert!(native.captured_at >= before && native.captured_at <= SystemTime::now());
    assert_eq!(
        common::jpeg_dimensions(&native.data),
        Some((native.width, native.height))
    );
    assert!(native.height > native.width, "portrait");
    if iphone_17 {
        assert_eq!((native.width, native.height), (1206, 2622));
    }

    let small = driver
        .screenshot(
            &udid,
            &ScreenshotOptions {
                max_dimension: 1000,
                quality: 80,
                ..ScreenshotOptions::default()
            },
        )
        .expect("a scaled screenshot");
    assert_eq!(small.height, 1000);
    assert_eq!(
        common::jpeg_dimensions(&small.data),
        Some((small.width, small.height))
    );
    if iphone_17 {
        assert_eq!(small.width, 460);
    }
}
