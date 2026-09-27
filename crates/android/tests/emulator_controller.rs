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

use android::adb::{AdbServer, Device, ShellPacket};
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

/// Whether `getevent -l` output shows a contact lifted: a tracking id of -1.
fn lifted(lines: &str) -> bool {
    lines
        .lines()
        .any(|l| l.contains("ABS_MT_TRACKING_ID") && l.trim_end().ends_with("ffffffff"))
}

/// The value of the last `axis` line in `getevent -l` output.
fn axis_value(lines: &str, axis: &str) -> Option<String> {
    lines
        .lines()
        .filter(|l| l.contains(axis))
        .filter_map(|l| l.split_whitespace().last())
        .last()
        .map(str::to_owned)
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

    // `exec` makes getevent the shell's own process, so closing the stream
    // sends it SIGHUP.
    let mut events = device
        .shell_stream(&format!("echo $$; exec getevent -l {input}"))
        .unwrap();
    events.set_read_timeout(Some(WAIT)).unwrap();
    let mut startup = String::new();
    while !startup.contains("name:") {
        match events.read_packet().unwrap() {
            Some(ShellPacket::Stdout(data)) => startup.push_str(&String::from_utf8_lossy(&data)),
            other => panic!("getevent did not start: {other:?}, output {startup:?}"),
        }
    }
    let pid = startup.lines().next().unwrap().trim().to_owned();

    // Reads until the contact is lifted, then closes the stream.
    let (sender, received) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut seen = String::new();
        while !lifted(&seen) {
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

    // On the status bar, where a tap changes nothing: (300, 60) at 1080x2400.
    let (x, y) = (display.width as i32 * 5 / 18, 60);
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

    let value = |axis| {
        let hex = axis_value(&seen, axis).unwrap_or_else(|| panic!("no {axis} in {seen:?}"));
        i64::from_str_radix(&hex, 16).unwrap()
    };
    // The touchscreen reports 0 to 32767 across the display.
    let tolerance = 2 * 32767 / i64::from(display.width) + 1;
    let expected_x = i64::from(x) * 32767 / i64::from(display.width);
    let expected_y = i64::from(y) * 32767 / i64::from(display.height);
    assert!(
        (value("ABS_MT_POSITION_X") - expected_x).abs() <= tolerance,
        "x {} against {expected_x}: {seen}",
        value("ABS_MT_POSITION_X")
    );
    assert!(
        (value("ABS_MT_POSITION_Y") - expected_y).abs() <= tolerance,
        "y {} against {expected_y}: {seen}",
        value("ABS_MT_POSITION_Y")
    );
    assert!(lifted(&seen), "not lifted: {seen}");
    eprintln!(
        "tap at ({x}, {y}) reached {input}; sendTouch down {:.2} ms, up {:.2} ms",
        down.as_secs_f64() * 1e3,
        up.as_secs_f64() * 1e3
    );
}

/// Sum of absolute byte differences between an RGB row and an RGBA row.
fn row_difference(rgb: &[u8], rgba: &[u8]) -> u64 {
    rgb.chunks_exact(3)
        .zip(rgba.chunks_exact(4))
        .map(|(a, b)| (0..3).map(|i| u64::from(a[i].abs_diff(b[i]))).sum::<u64>())
        .sum()
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_screenshots_are_full_resolution_and_top_down() {
    let Some(serial) = serial_or_skip("raw_screenshots_are_full_resolution_and_top_down") else {
        return;
    };
    let controller = connect(&serial).await;
    let display = controller.hardware().display;
    let started = Instant::now();
    let shot = controller.screenshot().await.unwrap();
    let took = started.elapsed();
    assert_eq!(
        (shot.width(), shot.height()),
        (display.width, display.height)
    );

    // screencap writes a small header, then RGBA rows top-down.
    let cap = adb(&serial).shell("screencap").unwrap();
    assert!(cap.success(), "{}", cap.stderr_text());
    let (w, h) = (display.width as usize, display.height as usize);
    assert!(
        cap.stdout.len() > w * h * 4,
        "screencap wrote {} bytes",
        cap.stdout.len()
    );
    let rgba = &cap.stdout[cap.stdout.len() - w * h * 4..];

    let skip = FrameHasher::default().status_bar_rows(display.height);
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
        "getScreenshot {}x{} RGB888 in {:.1} ms; mean byte difference from screencap: \
         {upright:.3} read top-down, {flipped:.3} read bottom-up",
        shot.width(),
        shot.height(),
        took.as_secs_f64() * 1e3
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
