//! Task 4.8: screenshots with the display's orientation (spec android-driver,
//! Screenshot capture), landscape on the Compose fixture, which rotates.

use std::time::{Duration, Instant};

use agent_android_host_tests::client::Client;
use agent_android_host_tests::fixture::RotationGuard;
use agent_android_host_tests::{Agent, Device, Launch, device, fixture, ms};
use protocol::message::{Call, ImageFormat, Reply, Rotation, Screenshot, ScreenshotParams};

fn screenshot(client: &mut Client, params: ScreenshotParams) -> (Screenshot, Duration) {
    let t0 = Instant::now();
    match client.call(Call::Screenshot(params)) {
        Ok(Reply::Screenshot(s)) => (s, t0.elapsed()),
        other => panic!("screenshot failed: {other:?}"),
    }
}

/// An image as luma, row by row.
struct Gray {
    width: usize,
    height: usize,
    luma: Vec<u8>,
}

fn decode_png(bytes: &[u8]) -> Gray {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().expect("a PNG header");
    let mut buf = vec![0; reader.output_buffer_size().expect("a buffer size")];
    let info = reader.next_frame(&mut buf).expect("a PNG frame");
    let channels = info.color_type.samples();
    let (width, height) = (info.width as usize, info.height as usize);
    let luma = buf[..info.buffer_size()]
        .chunks(channels)
        .map(|p| {
            if channels >= 3 {
                ((u32::from(p[0]) * 3 + u32::from(p[1]) * 6 + u32::from(p[2])) / 10) as u8
            } else {
                p[0]
            }
        })
        .collect();
    Gray {
        width,
        height,
        luma,
    }
}

/// Mean luma over a `cols` x `rows` grid of blocks.
fn blocks(img: &Gray, cols: usize, rows: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        for c in 0..cols {
            let (x0, x1) = (c * img.width / cols, (c + 1) * img.width / cols);
            let (y0, y1) = (r * img.height / rows, (r + 1) * img.height / rows);
            let mut sum = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += u64::from(img.luma[y * img.width + x]);
                }
            }
            out.push(sum as f64 / ((x1 - x0) * (y1 - y0)) as f64);
        }
    }
    out
}

fn mean_abs(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f64>() / a.len() as f64
}

/// The platform's own capture, `screencap -p`, which follows the display's rotation.
fn screencap(dev: &Device) -> Gray {
    let out = dev
        .adb()
        .args(["exec-out", "screencap", "-p"])
        .output()
        .expect("run screencap");
    assert!(out.status.success(), "screencap failed");
    decode_png(&out.stdout)
}

/// Width and height from a JPEG's start-of-frame marker.
fn jpeg_size(data: &[u8]) -> (u32, u32) {
    assert_eq!(&data[..2], [0xFF, 0xD8], "not a JPEG");
    let mut i = 2;
    while i + 9 < data.len() {
        assert_eq!(data[i], 0xFF, "a JPEG marker at {i}");
        let marker = data[i + 1];
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        if (0xC0..=0xC3).contains(&marker) {
            let h = u16::from_be_bytes([data[i + 5], data[i + 6]]);
            let w = u16::from_be_bytes([data[i + 7], data[i + 8]]);
            return (u32::from(w), u32::from(h));
        }
        i += 2 + len;
    }
    panic!("no start-of-frame marker");
}

#[test]
fn a_landscape_screenshot_has_landscape_dimensions_and_the_display_orientation() {
    let Some(dev) =
        device("a_landscape_screenshot_has_landscape_dimensions_and_the_display_orientation")
    else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    // Declared first, so it puts the rotation back after the agent is gone.
    let rotation = RotationGuard::new(&dev);
    rotation.freeze(1);
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let (shot, took) = screenshot(
        &mut client,
        ScreenshotParams {
            format: Some(ImageFormat::PNG),
            ..ScreenshotParams::default()
        },
    );
    let reference = screencap(&dev);
    assert_eq!(shot.format, ImageFormat::PNG);
    assert_eq!(shot.rotation, Rotation::R90);
    assert!(
        shot.width > shot.height,
        "{}x{} is not landscape",
        shot.width,
        shot.height
    );
    let image = decode_png(&shot.data);
    assert_eq!(
        (image.width as u32, image.height as u32),
        (shot.width, shot.height)
    );
    assert_eq!(
        (image.width, image.height),
        (reference.width, reference.height)
    );
    // Upright: the same picture as the platform's capture, block by block,
    // and unlike it when turned half way round.
    let ours = blocks(&image, 48, 24);
    let theirs = blocks(&reference, 48, 24);
    let mut turned = ours.clone();
    turned.reverse();
    let same = mean_abs(&ours, &theirs);
    let upside_down = mean_abs(&turned, &theirs);
    println!(
        "landscape {}x{} PNG in {:.1} ms; mean block difference {same:.2} upright, {upside_down:.2} turned",
        shot.width,
        shot.height,
        ms(took)
    );
    assert!(same < 6.0, "differs from screencap by {same:.2}");
    assert!(
        upside_down > same * 3.0 + 3.0,
        "{upside_down:.2} against {same:.2}"
    );
    drop(client);
    drop(agent);
    drop(rotation);
}

#[test]
fn a_portrait_screenshot_is_a_jpeg_of_the_display_and_scales_on_request() {
    let Some(dev) = device("a_portrait_screenshot_is_a_jpeg_of_the_display_and_scales_on_request")
    else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "big");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let size = dev.shell("wm size");
    let physical = size
        .lines()
        .find_map(|l| l.strip_prefix("Physical size: "))
        .expect("wm size");
    let (w, h) = physical
        .trim()
        .split_once('x')
        .map(|(w, h)| (w.parse::<u32>().unwrap(), h.parse::<u32>().unwrap()))
        .expect("WxH");
    let mut times = Vec::new();
    for _ in 0..5 {
        let (shot, took) = screenshot(&mut client, ScreenshotParams::default());
        assert_eq!(shot.format, ImageFormat::JPEG);
        assert_eq!(shot.rotation, Rotation::R0);
        assert_eq!((shot.width, shot.height), (w, h));
        assert_eq!(jpeg_size(&shot.data), (w, h));
        times.push(ms(took));
    }
    let (small, _) = screenshot(
        &mut client,
        ScreenshotParams {
            max_dimension: Some(1200),
            quality: Some(60),
            ..ScreenshotParams::default()
        },
    );
    assert_eq!(small.height, 1200);
    assert_eq!(small.width, w * 1200 / h);
    assert_eq!(jpeg_size(&small.data), (small.width, small.height));
    println!("portrait JPEG {w}x{h}: {times:.1?} ms");
}
