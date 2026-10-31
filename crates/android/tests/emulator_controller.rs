//! Integration tests for the emulator controller client (task 5.4).
//!
//! They run only when `MOBILE_AGENT_ANDROID_SERIAL` names an emulator started
//! with `-grpc <port> -grpc-use-token`, and print a skip line otherwise. Frames
//! are only observed. The one input is a tap on the status bar, where a tap
//! changes nothing; `getevent` watches the touchscreen for it and ends when its
//! shell stream closes. Nothing is left on the device.

mod common;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use android::adb::{AdbServer, Device, ShellPacket, Stdio};
use android::emulator::{
    ChangeDetector, Controller, Discovery, FrameHasher, Observation, Options, resolve, scaled_size,
};
use common::{percentile, serial_or_skip};

/// The mirror's working width in S7's measurements.
const WORKING_WIDTH: u32 = 540;
const WAIT: Duration = Duration::from_secs(5);

async fn connect(serial: &str) -> Controller {
    let endpoint = resolve(serial).unwrap_or_else(|e| panic!("{e}"));
    Controller::connect(&endpoint, Options::default())
        .await
        .unwrap_or_else(|e| panic!("{e}"))
}

fn adb(serial: &str) -> Device {
    AdbServer::from_env().device(serial)
}

/// The discovery file behind the serial's endpoint.
fn discovery(serial: &str) -> Discovery {
    let endpoint = resolve(serial).unwrap_or_else(|e| panic!("{e}"));
    let file = endpoint.discovery_file.expect("a discovery file");
    Discovery::parse(&std::fs::read_to_string(file).unwrap(), None).unwrap()
}

#[tokio::test]
async fn discovery_finds_the_endpoint_and_its_token() {
    let Some(serial) = serial_or_skip("discovery_finds_the_endpoint_and_its_token") else {
        return;
    };
    let endpoint = resolve(&serial).unwrap_or_else(|e| panic!("{e}"));
    let found = discovery(&serial);
    assert_eq!(found.serial().as_deref(), Some(serial.as_str()));
    assert_eq!(Some(endpoint.uri.clone()), found.grpc_endpoint());
    assert_eq!(endpoint.token, found.grpc_token);
    assert!(
        endpoint.token.is_some(),
        "start the emulator with -grpc-use-token"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_display_size_and_keyboard_match_the_avd() {
    let Some(serial) = serial_or_skip("the_display_size_and_keyboard_match_the_avd") else {
        return;
    };
    let controller = connect(&serial).await;
    let hardware = controller.hardware();

    let wm = adb(&serial).shell("wm size").unwrap().stdout_text();
    let physical = wm
        .lines()
        .find_map(|l| l.strip_prefix("Physical size: "))
        .unwrap_or_else(|| panic!("no physical size in {wm:?}"));
    let (w, h) = physical.trim().split_once('x').unwrap();
    assert_eq!(
        (hardware.display.width, hardware.display.height),
        (w.parse().unwrap(), h.parse().unwrap())
    );

    let avd_dir = discovery(&serial).avd_dir().expect("avd.dir");
    let config = std::fs::read_to_string(avd_dir.join("config.ini")).unwrap();
    let keyboard = config
        .lines()
        .find_map(|l| l.strip_prefix("hw.keyboard="))
        .map(|v| v.trim().eq_ignore_ascii_case("yes"));
    if let Some(keyboard) = keyboard {
        assert_eq!(controller.keys_supported(), keyboard);
    }
    eprintln!(
        "{serial}: display {}x{} from hardwareConfig, hw.keyboard={}: keys {} the controller",
        hardware.display.width,
        hardware.display.height,
        if hardware.keyboard { "yes" } else { "no" },
        if controller.keys_supported() {
            "may use"
        } else {
            "must not use"
        },
    );
}

#[tokio::test]
async fn frames_arrive_scaled_to_the_working_width() {
    let Some(serial) = serial_or_skip("frames_arrive_scaled_to_the_working_width") else {
        return;
    };
    let controller = connect(&serial).await;
    let (width, height) = scaled_size(controller.hardware().display, WORKING_WIDTH);
    assert!(
        width > 0 && height > 0,
        "the display is narrower than {WORKING_WIDTH}"
    );

    let subscribed = Instant::now();
    let mut stream = controller.frames(WORKING_WIDTH).await.unwrap();
    assert_eq!(stream.requested_size(), (width, height));
    let first = stream
        .next(WAIT)
        .await
        .unwrap()
        .expect("a first frame within 5 s");
    let first_after = subscribed.elapsed();
    assert_eq!((first.width(), first.height()), (width, height));
    assert_eq!(first.pixels().len(), (width * height) as usize * 3);

    // Frames come only when the guest posts one; count what a few seconds bring.
    let mut detector = ChangeDetector::new(FrameHasher::default());
    detector.observe(&first);
    let window = Duration::from_secs(3);
    let started = Instant::now();
    let (mut frames, mut changed, mut lags) = (0u32, 0u32, Vec::new());
    while let Some(left) = window.checked_sub(started.elapsed()) {
        let Some(frame) = stream.next(left).await.unwrap() else {
            break;
        };
        frames += 1;
        if detector.observe(&frame) == Observation::Changed {
            changed += 1;
        }
        let now_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        lags.push(now_us.saturating_sub(frame.timestamp_us()) as f64 / 1e3);
    }
    let seconds = started.elapsed().as_secs_f64();
    let lag = if lags.is_empty() {
        "no frames to time".to_owned()
    } else {
        format!(
            "timestamp to arrival p50 {:.1} ms, p95 {:.1} ms",
            percentile(&mut lags, 50.0),
            percentile(&mut lags, 95.0)
        )
    };
    eprintln!(
        "stream at {width}x{height}: first frame {:.1} ms after subscribing; then {frames} \
         frames in {seconds:.1} s ({:.1} fps), {changed} with changed content outside the \
         status bar; {lag}",
        first_after.as_secs_f64() * 1e3,
        f64::from(frames) / seconds
    );
}

/// The first multi-touch device, which carries the main display's touches.
fn touchscreen(device: &Device) -> String {
    let listing = device.shell("getevent -pl").unwrap().stdout_text();
    let mut current = None;
    for line in listing.lines() {
        if let Some(rest) = line.strip_prefix("add device ") {
            current = rest
                .split_once(": ")
                .map(|(_, path)| path.trim().to_owned());
        } else if line.contains("ABS_MT_POSITION_X") {
            return current.expect("a device before its axes");
        }
    }
    panic!("no multi-touch device in {listing}");
}

/// The event lines of `input` in `getevent -l` output, which prefixes each
/// event with its device when it watches several.
fn events_of<'a>(output: &'a str, input: &str) -> Vec<&'a str> {
    let prefix = format!("{input}:");
    output.lines().filter(|l| l.starts_with(&prefix)).collect()
}

/// How many contacts the events show lifted: tracking ids of -1.
fn lifts(events: &[&str]) -> usize {
    events
        .iter()
        .filter(|l| l.contains("ABS_MT_TRACKING_ID") && l.trim_end().ends_with("ffffffff"))
        .count()
}

/// The value of the last `axis` event.
fn axis_value(events: &[&str], axis: &str) -> Option<i64> {
    let hex = events
        .iter()
        .rev()
        .find(|l| l.contains(axis))?
        .split_whitespace()
        .next_back()?;
    i64::from_str_radix(hex, 16).ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_touch_lands_on_the_touchscreen() {
    let Some(serial) = serial_or_skip("a_touch_lands_on_the_touchscreen") else {
        return;
    };
    let controller = connect(&serial).await;
    let display = controller.hardware().display;
    let device = adb(&serial);
    let input = touchscreen(&device);

    // getevent block-buffers on a pipe, so it runs on a pseudo-terminal, and
    // `exec` makes it the session's own process, which the terminal's hangup
    // ends when the stream closes. Watching every device, it announces each
    // one it opens; once the touchscreen is announced, its events are queued.
    let mut events = device
        .shell_stream_with("echo $$; exec getevent -l", Stdio::Pty)
        .unwrap();
    events.set_read_timeout(Some(WAIT)).unwrap();
    let announced = format!(": {input}\r\n  name:");
    let mut startup = String::new();
    while !startup.contains(&announced) {
        match events.read_packet().unwrap() {
            Some(ShellPacket::Stdout(data)) => startup.push_str(&String::from_utf8_lossy(&data)),
            other => panic!("getevent did not start: {other:?}, output {startup:?}"),
        }
    }
    let pid = startup.lines().next().unwrap().trim().to_owned();

    // Reads until both contacts are lifted, then closes the stream.
    let (sender, received) = mpsc::channel();
    let watched = input.clone();
    let reader = thread::spawn(move || {
        let mut seen = String::new();
        while lifts(&events_of(&seen, &watched)) < 2 {
            match events.read_packet() {
                Ok(Some(ShellPacket::Stdout(data))) => {
                    seen.push_str(&String::from_utf8_lossy(&data))
                }
                _ => break,
            }
        }
        drop(events);
        let _ = sender.send(seen);
    });

    // Two taps on the status bar, where a tap changes nothing: (250, 50), then
    // (300, 70) at 1080x2400. evdev drops a position equal to the slot's last
    // one, so only a tap away from the previous contact reports its position.
    let width = display.width as i32;
    controller.tap(width * 25 / 108, 50).await.unwrap();
    let (x, y) = (width * 5 / 18, 70);
    let down = controller
        .touch(&[android::emulator::TouchPoint::down(x, y)])
        .await
        .unwrap();
    let up = controller
        .touch(&[android::emulator::TouchPoint::up(x, y)])
        .await
        .unwrap();
    let seen = received.recv_timeout(WAIT).unwrap_or_default();
    reader.join().unwrap();

    // getevent must be gone with its stream.
    let deadline = Instant::now() + WAIT;
    while device.shell(&format!("kill -0 {pid}")).unwrap().success() {
        if Instant::now() > deadline {
            device.shell(&format!("kill {pid}")).unwrap();
            panic!("getevent {pid} outlived its stream");
        }
        thread::sleep(Duration::from_millis(20));
    }

    let touches = events_of(&seen, &input);
    let value = |axis| {
        axis_value(&touches, axis).unwrap_or_else(|| panic!("no {axis} from {input}: {seen:?}"))
    };
    // The touchscreen reports 0 to 32767 across the display.
    let tolerance = 2 * 32767 / i64::from(display.width) + 1;
    let expected_x = i64::from(x) * 32767 / i64::from(display.width);
    let expected_y = i64::from(y) * 32767 / i64::from(display.height);
    let (seen_x, seen_y) = (value("ABS_MT_POSITION_X"), value("ABS_MT_POSITION_Y"));
    assert!(
        (seen_x - expected_x).abs() <= tolerance,
        "x {seen_x} against {expected_x}: {touches:?}"
    );
    assert!(
        (seen_y - expected_y).abs() <= tolerance,
        "y {seen_y} against {expected_y}: {touches:?}"
    );
    assert_eq!(lifts(&touches), 2, "{touches:?}");
    eprintln!(
        "tap at ({x}, {y}) reached {input} as ({seen_x}, {seen_y}) of 32767, expected \
         ({expected_x}, {expected_y}); sendTouch down {:.2} ms, up {:.2} ms",
        down.as_secs_f64() * 1e3,
        up.as_secs_f64() * 1e3
    );
}

/// Sum of absolute byte differences between an RGB row and an RGBA row.
fn row_difference(rgb: &[u8], rgba: &[u8]) -> u64 {
    let (rgb, _) = rgb.as_chunks::<3>();
    let (rgba, _) = rgba.as_chunks::<4>();
    rgb.iter()
        .zip(rgba)
        .map(|(a, b)| (0..3).map(|i| u64::from(a[i].abs_diff(b[i]))).sum::<u64>())
        .sum()
}

/// screencap's RGBA rows, top-down, after the small header it writes first.
fn screencap(serial: &str, width: usize, height: usize) -> Vec<u8> {
    let cap = adb(serial).shell("screencap").unwrap();
    assert!(cap.success(), "{}", cap.stderr_text());
    let size = width * height * 4;
    assert!(
        cap.stdout.len() > size,
        "screencap wrote {} bytes",
        cap.stdout.len()
    );
    cap.stdout[cap.stdout.len() - size..].to_vec()
}

/// Mean byte difference of every fourth row of two RGBA screens of `width`,
/// from row `skip` down.
fn rgba_difference(a: &[u8], b: &[u8], width: usize, skip: usize) -> f64 {
    let stride = width * 4;
    let (mut sum, mut bytes) = (0u64, 0u64);
    for y in (skip..a.len() / stride).step_by(4) {
        let row = y * stride..(y + 1) * stride;
        sum += a[row.clone()]
            .iter()
            .zip(&b[row])
            .map(|(x, y)| u64::from(x.abs_diff(*y)))
            .sum::<u64>();
        bytes += stride as u64;
    }
    sum as f64 / bytes as f64
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_screenshots_are_full_resolution_and_top_down() {
    let Some(serial) = serial_or_skip("raw_screenshots_are_full_resolution_and_top_down") else {
        return;
    };
    let controller = connect(&serial).await;
    let display = controller.hardware().display;
    let mut took = Vec::new();
    for _ in 0..10 {
        let started = Instant::now();
        let shot = controller.screenshot().await.unwrap();
        took.push(started.elapsed().as_secs_f64() * 1e3);
        assert_eq!(
            (shot.width(), shot.height()),
            (display.width, display.height)
        );
    }

    // The screenshot and screencap must show one screen. The app the previous
    // test left can still be going away when this one starts, the longer
    // under software rendering, and a screen that changed between the two
    // reads matches neither way (170.6 top-down against 160.1 bottom-up on
    // CI's swiftshader emulator, where the same screen held still read 0.0
    // against 80.1). So screencap reads the screen before and after the
    // screenshot, and the comparison takes a screenshot between two that
    // agree.
    let (w, h) = (display.width as usize, display.height as usize);
    let skip = FrameHasher::default().status_bar_rows(display.height);
    let mut still = None;
    let mut moving = Vec::new();
    for _ in 0..20 {
        let before = screencap(&serial, w, h);
        let shot = controller.screenshot().await.unwrap();
        let after = screencap(&serial, w, h);
        let moved = rgba_difference(&before, &after, w, skip as usize);
        if moved < 1.0 {
            still = Some((shot, after));
            break;
        }
        moving.push(moved);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let (shot, rgba) = still.unwrap_or_else(|| {
        panic!(
            "the screen never held still across a screenshot; screencaps differed by {moving:.1?}"
        )
    });

    let (mut upright, mut flipped, mut bytes) = (0u64, 0u64, 0u64);
    for y in (skip..display.height).step_by(4) {
        let raw = shot.row(y);
        let same_row = &rgba[y as usize * w * 4..(y as usize + 1) * w * 4];
        let mirror = (h - 1 - y as usize) * w * 4;
        upright += row_difference(raw, same_row);
        flipped += row_difference(raw, &rgba[mirror..mirror + w * 4]);
        bytes += raw.len() as u64;
    }
    let (upright, flipped) = (upright as f64 / bytes as f64, flipped as f64 / bytes as f64);
    eprintln!(
        "getScreenshot {}x{} RGB888 over {} calls: p50 {:.1} ms, p95 {:.1} ms; mean byte \
         difference from screencap: {upright:.3} read top-down, {flipped:.3} read bottom-up, \
         after {} reads of a screen that moved",
        shot.width(),
        shot.height(),
        took.len(),
        percentile(&mut took, 50.0),
        percentile(&mut took, 95.0),
        moving.len()
    );
    assert!(
        upright < 1.0 || upright * 2.0 < flipped,
        "rows do not match top-down: {upright:.3} against {flipped:.3} flipped"
    );
}

#[tokio::test]
async fn a_closed_port_falls_back_with_the_launch_flag_and_console_command() {
    let Some(serial) =
        serial_or_skip("a_closed_port_falls_back_with_the_launch_flag_and_console_command")
    else {
        return;
    };
    let mut endpoint = resolve(&serial).unwrap_or_else(|e| panic!("{e}"));
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    endpoint.uri = format!("http://127.0.0.1:{closed}");
    let err = Controller::connect(&endpoint, Options::default())
        .await
        .unwrap_err();
    let port = endpoint.console_port.expect("the console port") + 3000;
    let warning = err.warning();
    assert!(
        warning.contains(&format!("-grpc {port} -grpc-use-token")),
        "{warning}"
    );
    assert!(warning.contains(&format!("`grpc {port}`")), "{warning}");
    assert!(warning.contains("without a restart"), "{warning}");
    eprintln!("{warning}");
}

#[tokio::test]
async fn a_call_without_the_token_is_refused() {
    let Some(serial) = serial_or_skip("a_call_without_the_token_is_refused") else {
        return;
    };
    let mut endpoint = resolve(&serial).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        endpoint.token.take().is_some(),
        "start the emulator with -grpc-use-token"
    );
    let err = Controller::connect(&endpoint, Options::default())
        .await
        .unwrap_err();
    assert!(err.reason.contains("Unauthenticated"), "{}", err.reason);
}
