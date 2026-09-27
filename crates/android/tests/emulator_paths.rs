//! Integration tests for path selection (task 5.5): the emulator fast path
//! with its controller, and the fallback to the agent without it.
//!
//! They run only when `MOBILE_AGENT_ANDROID_SERIAL` names an emulator started
//! with `-grpc <port> -grpc-use-token` and with the Compose fixture
//! installed, one at a time, and need the agent jar. An emulator without its
//! controller is stood for by an endpoint on a closed port. Each test starts
//! the fixture, attaches, taps the Login tab through the mirror and stops its
//! agent; a disturbed test runs again once the device is quiet.

mod common;

use std::time::{Duration, Instant};

use android::adb::{AdbServer, Device};
use android::agent::{Attached, DeployOptions, Deployer, Records};
use android::emulator::{self, Options};
use android::session::{ControllerChoice, Path, Session, SessionOptions, select_paths};
use common::{
    Disturbed, Fixture, Leftovers, agent_jar, compose_in_front, device_lock, disturbed_by,
    launch_compose, retry_when_disturbed, serial_or_skip,
};
use mirror::{Action, FrameRequest, Mirror, MirrorConfig, MirrorError, SnapshotOptions, Target};

fn device(serial: &str) -> Device {
    AdbServer::from_env().device(serial)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn attach(device: &Device, leftovers: &mut Leftovers) -> Result<Attached, Disturbed> {
    let attached = Deployer::new(device.clone(), agent_jar(), Records::in_memory())
        .attach(&DeployOptions {
            idle_exit: Duration::from_secs(10),
            ..DeployOptions::default()
        })
        .map_err(|e| disturbed_by(device, e))?;
    leftovers.track(attached.agent.pid, &attached.agent.socket);
    Ok(attached)
}

/// Shows the Big tab, taps the Login tab through the mirror and returns how
/// long the tree took to show the Login screen.
fn tap_login(device: &Device, mirror: &mut Mirror) -> Result<Duration, Disturbed> {
    let big = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    compose_in_front(device, "the first snapshot")?;
    if !big.text.contains("cell-500") {
        return Err(Disturbed("the first snapshot shows another tab".into()));
    }
    let tab = big
        .elements
        .iter()
        .find(|e| e.identifier.as_deref() == Some("tab-login"))
        .and_then(|e| e.reference)
        .expect("a ref for tab-login");
    let tapped = Instant::now();
    let outcome = mirror
        .act(&Action::Tap {
            target: Target::Ref(tab),
            hold: None,
        })
        .unwrap();
    assert!(outcome.accepted);
    let login = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    let changed = tapped.elapsed();
    compose_in_front(device, "the tap")?;
    assert!(login.text.contains("login-username"), "{}", login.text);
    assert!(!login.text.contains("cell-1"), "{}", login.text);
    Ok(changed)
}

/// Spec scenario "Control endpoint enabled" (android-driver, Emulator fast
/// path): taps and frames go through the controller and the status names
/// the fast path, with keys and text on the agent and the keyboard
/// reported.
#[test]
fn with_the_controller_taps_and_frames_use_it_and_the_status_names_the_fast_path() {
    let test = "with_the_controller_taps_and_frames_use_it_and_the_status_names_the_fast_path";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || enabled(&serial, &device));
}

fn enabled(serial: &str, device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    launch_compose(device, "big")?;
    let _fixture = Fixture(device.clone());
    let selection = select_paths(serial, true, &ControllerChoice::Auto, Options::default());
    assert!(
        selection.link.is_some(),
        "no controller: {:?}",
        selection.paths.warning
    );
    let paths = selection.paths.clone();
    assert_eq!(paths.input, Path::Controller);
    assert_eq!(paths.frames, Some(Path::Controller));
    assert_eq!((paths.keys, paths.screenshots), (Path::Agent, Path::Agent));
    assert_eq!(paths.warning, None);
    let keyboard = paths.keyboard.expect("the controller reports the keyboard");

    let attached = attach(device, &mut leftovers)?;
    let session = Session::start_selected(attached, SessionOptions::default(), selection)
        .unwrap_or_else(|e| panic!("{e}"));
    let text = session.status().to_string();
    assert!(
        text.contains("input: emulator controller at http://"),
        "{text}"
    );
    assert!(text.contains("frames: emulator controller"), "{text}");
    assert!(text.contains("keys and text: agent"), "{text}");
    let declares = if keyboard {
        "declares a keyboard"
    } else {
        "declares no keyboard"
    };
    assert!(text.contains(declares), "{text}");

    let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());
    let changed = tap_login(device, &mut mirror)?;
    let status = session.status();
    assert_eq!(status.inputs, (1, 0), "the tap went through the agent");
    assert_eq!(status.controller_error, None);
    let frame = mirror.frame(&FrameRequest::default()).unwrap();
    assert_eq!(&frame.data[..2], [0xFF, 0xD8], "a JPEG");
    assert!(frame.width.max(frame.height) <= 1000);
    assert!(status.frames_received > 0);
    eprintln!("{status}");
    eprintln!(
        "fast path: the Login screen was in the tree {:.1} ms after the controller's tap; \
         latest frame {}x{}, {} bytes, {:.1} ms old",
        ms(changed),
        frame.width,
        frame.height,
        frame.data.len(),
        ms(frame.age)
    );
    session.stop(Duration::from_secs(3)).unwrap();
    Ok(())
}

/// Spec scenario "Control endpoint disabled": with the controller's port
/// closed, as on an emulator started without the control flag, taps and
/// screenshots go through the agent and the status carries the warning with
/// the launch flag and the console command.
#[test]
fn without_the_controller_taps_and_screenshots_use_the_agent_and_the_status_warns() {
    let test = "without_the_controller_taps_and_screenshots_use_the_agent_and_the_status_warns";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || disabled(&serial, &device));
}

fn disabled(serial: &str, device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    launch_compose(device, "big")?;
    let _fixture = Fixture(device.clone());
    let mut endpoint = emulator::resolve(serial).unwrap_or_else(|e| panic!("{e}"));
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    endpoint.uri = format!("http://127.0.0.1:{closed}");
    let console = endpoint.console_port.expect("the console port");
    let selection = select_paths(
        serial,
        true,
        &ControllerChoice::Endpoint(endpoint),
        Options::default(),
    );
    assert!(selection.link.is_none());
    let paths = selection.paths.clone();
    assert_eq!((paths.input, paths.frames), (Path::Agent, None));
    let grpc = console + 3000;
    let warning = paths.warning.clone().expect("a warning");
    assert!(
        warning.contains(&format!("-grpc {grpc} -grpc-use-token")),
        "{warning}"
    );
    assert!(warning.contains(&format!("`grpc {grpc}`")), "{warning}");

    let attached = attach(device, &mut leftovers)?;
    let session = Session::start_selected(attached, SessionOptions::default(), selection)
        .unwrap_or_else(|e| panic!("{e}"));
    let text = session.status().to_string();
    assert!(text.contains("input: agent"), "{text}");
    assert!(text.contains("frames: none"), "{text}");
    assert!(text.contains(&format!("warning: {warning}")), "{text}");

    let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());
    let changed = tap_login(device, &mut mirror)?;
    assert_eq!(
        session.status().inputs,
        (0, 1),
        "the tap did not go through the agent"
    );
    let shot = session
        .screenshot(protocol::message::ScreenshotParams::default())
        .unwrap();
    assert_eq!(shot.format, protocol::message::ImageFormat::JPEG);
    assert_eq!(&shot.data[..2], [0xFF, 0xD8]);
    assert!(
        shot.width > 0 && shot.height > shot.width,
        "{}x{}",
        shot.width,
        shot.height
    );
    assert!(matches!(
        mirror.frame(&FrameRequest::default()),
        Err(MirrorError::NoFrame)
    ));
    eprintln!("{}", session.status());
    eprintln!(
        "agent path: the Login screen was in the tree {:.1} ms after the agent's tap; \
         screenshot {}x{} through the agent",
        ms(changed),
        shot.width,
        shot.height
    );
    session.stop(Duration::from_secs(3)).unwrap();
    Ok(())
}
