//! Task 6.2: frames from the display surface (simulator-driver spec, "Frames
//! and screenshots"; design D7).

#![cfg(target_os = "macos")]

mod common;

use std::time::{Duration, Instant, SystemTime};

use simulator::{FrameOptions, Frames, Lifecycle, ScreenshotOptions};

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
    );

    // The transition: the fixture relaunches onto its Anim tab, whose
    // spinner then turns without end.
    common::show_expo(&driver, &udid, "anim");
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
    );

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

    let window = Duration::from_secs(5);
    let before = stream.stats();
    let cpu_before = common::process_cpu();
    let start = Instant::now();
    let frames = common::collect(&stream, window);
    let elapsed = start.elapsed().as_secs_f64();
    let process_cpu = (common::process_cpu() - cpu_before).as_secs_f64() / elapsed;
    let after = stream.stats();
    let pipeline_cpu = (after.cpu - before.cpu).as_secs_f64() / elapsed;
    let fps = frames.len() as f64 / elapsed;
    let sizes: Vec<usize> = frames
        .iter()
        .filter_map(|f| f.jpeg.as_ref())
        .map(Vec::len)
        .collect();
    eprintln!(
        "{fps:.1} fps delivered at {}x{} with {}, pipeline {:.1}% and process {:.1}% of one \
         core; {} presents, {} reads, {} torn, {} dropped; mean JPEG {} bytes",
        frames.first().map_or(0, |f| f.width),
        frames.first().map_or(0, |f| f.height),
        after.encoder,
        pipeline_cpu * 100.0,
        process_cpu * 100.0,
        after.presents - before.presents,
        after.reads - before.reads,
        after.torn - before.torn,
        after.dropped - before.dropped,
        sizes.iter().sum::<usize>() / sizes.len().max(1),
    );
    assert!(fps >= 28.0, "{fps:.1} fps");
    assert!(
        pipeline_cpu < 0.20,
        "the pipeline used {pipeline_cpu:.3} of a core"
    );
    assert!(
        process_cpu < 0.20,
        "the process used {process_cpu:.3} of a core"
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
        common::jpeg_dimensions(&native.jpeg),
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
            },
        )
        .expect("a scaled screenshot");
    assert_eq!(small.height, 1000);
    assert_eq!(
        common::jpeg_dimensions(&small.jpeg),
        Some((small.width, small.height))
    );
    if iphone_17 {
        assert_eq!(small.width, 460);
    }
}
