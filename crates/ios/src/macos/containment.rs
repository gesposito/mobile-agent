//! Failure containment's tests (spec `ios-device-driver`, "Failure
//! containment"): a phone unplugged, locked or rebooted mid-session leaves
//! its target disconnected with the cause, another phone on the same host
//! goes on, and a new attach starts over; a command made while the runner
//! starts again waits for it within its timeout; and no command on the hot
//! path reads DVT's process list, which costs 474 to 728 ms on the phone.

use std::time::{Duration, Instant};

use protocol::message::{Call, Reply, SnapshotParams};

use crate::discovery::tests::recorded_images;
use crate::macos::driver::tests::{FakeHost, Images, Other, PHONE, core};
use crate::macos::driver::{Core, Gesture};
use crate::macos::dvt::RUNNING_PROCESSES;
use crate::macos::fake::FakeMounter;
use crate::macos::fake_runner::Start;
use crate::model::{
    AttachOptions, Disconnect, FrameOptions, Point, RunnerSpec, RunnerState, RunnerStatus,
    ScreenshotOptions, Swipe,
};
use crate::touch::TAP_HOLD;

const VERSION: &str = "0.1.0";
const OTHER: &str = "placeholder-udid-other-phone";
const WAIT: Duration = Duration::from_secs(2);

fn options() -> AttachOptions {
    AttachOptions {
        runner: RunnerSpec::new(VERSION),
        target_app: None,
        timeout: Duration::from_secs(5),
    }
}

fn tap(core: &Core<FakeHost>, target: &str) -> crate::error::Result<crate::model::Dispatch> {
    core.input(target, Gesture::Press(Point::new(50.0, 50.0), TAP_HOLD))
}

fn snapshot() -> Call {
    Call::Snapshot(SnapshotParams::default())
}

/// The phone and another, both with the runner installed.
fn two_phones() -> FakeHost {
    let mut host = FakeHost::ready();
    host.device.install_runner(VERSION);
    let other = Other::new(OTHER);
    other.device.install_runner(VERSION);
    host.others.push(other);
    host
}

/// The runner's status on `target` once `done` holds, within two seconds.
fn runner_once(
    core: &Core<FakeHost>,
    target: &str,
    done: impl Fn(&RunnerStatus) -> bool,
) -> RunnerStatus {
    let deadline = Instant::now() + WAIT;
    loop {
        let status = core
            .status(target)
            .expect("status")
            .runner
            .expect("a runner");
        if done(&status) || Instant::now() >= deadline {
            return status;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn kind(error: &crate::error::Error) -> Option<Disconnect> {
    match error {
        crate::error::Error::Disconnected { kind, .. } => Some(*kind),
        _ => None,
    }
}

#[test]
fn a_pulled_cable_fails_the_phones_commands_as_unplugged_and_spares_the_other_phone() {
    let core = core(two_phones());
    for target in [PHONE, OTHER] {
        core.attach(target, &options()).expect("attaches");
        tap(&core, target).expect("taps");
    }
    core.host.device.unplug();
    let error = tap(&core, PHONE).expect_err("unplugged");
    assert_eq!(error.code(), "disconnected", "{error}");
    assert_eq!(kind(&error), Some(Disconnect::Unplugged), "{error}");
    assert!(error.to_string().contains("unplugged"), "{error}");
    let failed = runner_once(&core, PHONE, |status| {
        matches!(status.state, RunnerState::Failed { .. })
    });
    assert!(
        matches!(
            failed.state,
            RunnerState::Failed {
                code: "disconnected",
                ..
            }
        ),
        "{failed:?}"
    );
    let error = core
        .call(PHONE, snapshot(), WAIT)
        .expect_err("its runner is gone");
    assert_eq!(kind(&error), Some(Disconnect::Unplugged), "{error}");

    // The other phone goes on.
    tap(&core, OTHER).expect("the other phone takes input");
    assert!(matches!(
        core.call(OTHER, snapshot(), WAIT)
            .expect("its runner serves"),
        Reply::Snapshot(_)
    ));
    let stream = core
        .frames(OTHER, &FrameOptions::default())
        .expect("subscribed");
    drop(stream);
    assert_eq!(
        core.status(OTHER)
            .expect("status")
            .runner
            .map(|runner| runner.state),
        Some(RunnerState::Ready)
    );
}

#[test]
fn a_replugged_phone_attaches_again_and_starts_over() {
    let core = core(two_phones());
    core.attach(PHONE, &options()).expect("attaches");
    let stale = core.host.device.runner().expect("a runner").pid;
    let opens = core.host.device.opens();
    core.host.device.unplug();
    tap(&core, PHONE).expect_err("unplugged");
    core.host.device.plug_in();
    // The runner outlived the pull, as in S10; the new attach ends it.
    assert_eq!(
        core.host.device.runner().map(|runner| runner.pid),
        Some(stale)
    );
    let attached = core.attach(PHONE, &options()).expect("attaches again");
    assert_eq!(attached.runner.state, RunnerState::Ready);
    assert_eq!(attached.runner.restarts, 0, "a new session");
    assert_eq!(attached.runner.leftovers, vec![stale]);
    assert_ne!(
        core.host.device.runner().map(|runner| runner.pid),
        Some(stale)
    );
    assert!(core.host.device.opens() > opens, "a new tunnel");
    tap(&core, PHONE).expect("input again");
    core.call(PHONE, snapshot(), WAIT)
        .expect("the new runner serves");
}

#[test]
fn a_replugged_phone_takes_commands_again_under_its_new_usbmuxd_number() {
    let core = core(two_phones());
    tap(&core, PHONE).expect("taps");
    let first = core.host.device.usb_id();
    core.host.device.unplug();
    tap(&core, PHONE).expect_err("unplugged");
    core.host.device.plug_in();
    assert_ne!(core.host.device.usb_id(), first, "usbmuxd numbers it anew");
    tap(&core, PHONE).expect("input again, through a new tunnel, without an attach");
    core.attach(PHONE, &options())
        .expect("and the runner attaches under the new number");
}

#[test]
fn a_locked_phone_fails_input_as_locked_and_keeps_its_connections() {
    let core = core(two_phones());
    core.attach(PHONE, &options()).expect("attaches");
    tap(&core, PHONE).expect("taps");
    // The fake reads the release a moment after the tap returns.
    std::thread::sleep(Duration::from_millis(50));
    let events = core.host.device.hid_events().len();
    assert_eq!(events, 2);
    core.host.device.set_locked(true);
    let error = tap(&core, PHONE).expect_err("locked");
    assert_eq!(error.code(), "disconnected", "{error}");
    assert_eq!(kind(&error), Some(Disconnect::Locked), "{error}");
    assert!(error.to_string().contains("unlock"), "{error}");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        core.host.device.hid_events().len(),
        events,
        "no report reached the lock screen"
    );
    core.host.device.set_locked(false);
    tap(&core, PHONE).expect("unlocked, input goes on");
    assert_eq!(
        core.status(PHONE)
            .expect("status")
            .runner
            .map(|runner| runner.state),
        Some(RunnerState::Ready),
        "the session was kept"
    );
}

#[test]
fn a_rebooted_phone_is_told_apart_and_attaches_again_once_unlocked() {
    let mut host = two_phones();
    host.images = Images::Mounter;
    host.mounter = FakeMounter::new(true, Ok(()), Err("not called".into()));
    let core = core(host);
    core.attach(PHONE, &options()).expect("attaches");
    core.host.reboot();
    let error = tap(&core, PHONE).expect_err("rebooted");
    assert_eq!(kind(&error), Some(Disconnect::Rebooted), "{error}");
    assert!(error.to_string().contains("restarted"), "{error}");
    let error = core
        .attach(PHONE, &options())
        .expect_err("locked after the restart");
    assert_eq!(error.code(), "device_locked", "{error}");
    core.host.device.set_locked(false);
    let attached = core.attach(PHONE, &options()).expect("attaches again");
    assert_eq!(attached.runner.state, RunnerState::Ready);
    assert!(
        core.host.mounter.calls().contains(&"idevice"),
        "the disk image was mounted again: {:?}",
        core.host.mounter.calls()
    );
}

#[test]
fn a_command_made_while_the_runner_starts_again_waits_for_it_within_its_timeout() {
    let host = two_phones();
    host.test_manager
        .script_starts(&[Start::Normal, Start::Slow(Duration::from_millis(400))]);
    let core = core(host);
    core.attach(PHONE, &options()).expect("attaches");
    core.host.device.kill_runner();
    let restarting = runner_once(&core, PHONE, |status| {
        matches!(status.state, RunnerState::Restarting { .. })
    });
    assert!(
        matches!(restarting.state, RunnerState::Restarting { .. }),
        "{restarting:?}"
    );
    let error = core
        .call(PHONE, snapshot(), Duration::from_millis(50))
        .expect_err("its timeout is shorter than the restart");
    assert_eq!(error.code(), "timeout", "{error}");
    assert!(error.retryable());
    let waited = Instant::now();
    core.call(PHONE, snapshot(), WAIT)
        .expect("waits for the restart, then is served");
    assert!(waited.elapsed() < WAIT);
    let status = runner_once(&core, PHONE, |status| status.restarts == 1);
    assert_eq!(status.restarts, 1);
}

#[test]
fn no_hot_path_reads_the_process_list() {
    let core = core(two_phones());
    core.attach(PHONE, &options()).expect("attaches");
    let lists = |core: &Core<FakeHost>| {
        core.host
            .device
            .dvt_calls()
            .iter()
            .filter(|call| call.selector == RUNNING_PROCESSES)
            .count()
    };
    let before = lists(&core);
    for _ in 0..10 {
        tap(&core, PHONE).expect("taps");
        core.call(PHONE, snapshot(), WAIT).expect("served");
    }
    let stroke = Swipe::new(
        Point::new(100.0, 400.0),
        Point::new(100.0, 200.0),
        Duration::from_millis(100),
    )
    .resting();
    core.input(PHONE, Gesture::Swipe(stroke)).expect("swiped");
    core.host
        .device
        .show(crate::frames::tests::screen(|_, _| 120, false));
    let stream = core
        .frames(PHONE, &FrameOptions::default())
        .expect("subscribed");
    stream.next(WAIT).expect("a frame");
    drop(stream);
    core.screenshot(PHONE, &ScreenshotOptions::default())
        .expect("a screenshot");
    assert_eq!(lists(&core), before, "none of them read DVT's process list");
}

#[test]
fn a_device_that_refuses_lockdown_until_its_first_unlock_reads_as_rebooted() {
    let host = FakeHost::with(
        &[],
        crate::model::Trust::Unreachable(
            "the device is locked, and lockdown refuses a session until it is unlocked".into(),
        ),
        Some(Ok(recorded_images())),
    );
    let core = core(host);
    assert_eq!(
        core.runtime_block_on_diagnose(PHONE),
        Some(Disconnect::Rebooted)
    );
}
