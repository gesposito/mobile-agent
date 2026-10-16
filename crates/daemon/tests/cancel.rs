//! `cancel` (design D7 of the `mcp-server` change): the session's command
//! in flight on a target, which holds the target's worker, is ended from
//! another connection at its next wakeup and fails with `cancelled`, so the
//! target serves the next command at once; a device call in flight runs to
//! its end.

mod support;

use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{APP, Daemon, request_at};

/// Sends `request` on a connection of its own, returning the reply and
/// when it came.
fn in_flight(daemon: &Daemon, request: Value) -> JoinHandle<(Value, Instant)> {
    let socket: PathBuf = daemon.socket();
    std::thread::spawn(move || {
        let reply = request_at(&socket, &request);
        (reply, Instant::now())
    })
}

/// A daemon on a fake emulator with the fixture app open on its Login tab.
fn opened() -> Daemon {
    let daemon = Daemon::start("android");
    let opened = daemon.request(json!({"action": "open", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");
    daemon
}

#[test]
fn a_cancel_ends_the_session_s_wait_with_cancelled_and_the_next_command_runs_at_once() {
    let daemon = opened();
    let waiting = in_flight(
        &daemon,
        json!({"action": "wait", "text": "Nobody shows this"}),
    );
    std::thread::sleep(Duration::from_millis(300));

    // Another session's cancel leaves the wait alone.
    let other = daemon.request(json!({"action": "cancel", "session": "bob"}));
    assert_eq!(other["success"], true, "{other}");
    assert_eq!(other["data"]["cancelled"], false, "{other}");
    assert!(!waiting.is_finished(), "bob's cancel ended alice's wait");

    let sent = Instant::now();
    let cancel = daemon.request(json!({"action": "cancel"}));
    assert_eq!(cancel["success"], true, "{cancel}");
    assert_eq!(cancel["data"]["cancelled"], true, "{cancel}");
    assert_eq!(cancel["data"]["action"], "wait", "{cancel}");
    assert_eq!(cancel["data"]["target"], "emulator-5554", "{cancel}");

    let (reply, at) = waiting.join().expect("the wait's connection");
    assert_eq!(reply["success"], false, "{reply}");
    assert_eq!(reply["code"], "cancelled", "{reply}");
    assert!(reply["data"].is_null(), "{reply}");
    assert!(
        reply["error"]
            .as_str()
            .is_some_and(|error| error.starts_with("wait ")),
        "the message names the command: {reply}"
    );
    assert!(
        at.duration_since(sent) < Duration::from_millis(200),
        "the wait ended within 200 ms of the cancel: {:?}",
        at.duration_since(sent)
    );

    // The worker is free and the flag is down: the next command runs at
    // once, and a wait after it waits.
    let next = Instant::now();
    let snapshot = daemon.request(json!({"action": "snapshot", "interactive": true}));
    assert_eq!(snapshot["success"], true, "{snapshot}");
    assert!(
        next.elapsed() < Duration::from_millis(500),
        "{:?}",
        next.elapsed()
    );
    let slept = daemon.request(json!({"action": "wait", "ms": 200}));
    assert_eq!(slept["success"], true, "{slept}");
}

#[test]
fn a_cancel_with_nothing_in_flight_says_so_and_changes_nothing() {
    let daemon = Daemon::start("android");
    let idle = daemon.request(json!({"action": "cancel"}));
    assert_eq!(idle["success"], true, "{idle}");
    assert_eq!(
        idle["data"],
        json!({"cancelled": false, "action": null, "target": null}),
        "{idle}"
    );
    let opened = daemon.request(json!({"action": "open", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");
    let held = daemon.request(json!({"action": "cancel"}));
    assert_eq!(held["data"]["cancelled"], false, "{held}");
    let started = Instant::now();
    let slept = daemon.request(json!({"action": "wait", "ms": 300}));
    assert_eq!(slept["success"], true, "{slept}");
    assert!(started.elapsed() >= Duration::from_millis(300));
}

/// A cancel that comes while the tap's input is held, as a Simulator holds
/// the first input until its input session is active, lets the tap go in:
/// the click completes, its watch cut short with a warning, and the screen
/// shows what the tap did.
#[test]
fn a_cancel_during_a_tap_s_device_call_leaves_the_click_completing() {
    let daemon = Daemon::builder()
        .fake("android")
        .env("MOBILE_AGENT_FAKE_ACTIVATION_MS", "1500")
        .start();
    let launcher = daemon.request(json!({"action": "snapshot", "interactive": true}));
    assert_eq!(launcher["success"], true, "{launcher}");
    let clicking = in_flight(
        &daemon,
        json!({"action": "click", "target": "#launcher-fixture"}),
    );
    std::thread::sleep(Duration::from_millis(400));
    let sent = Instant::now();
    let cancel = daemon.request(json!({"action": "cancel"}));
    assert_eq!(cancel["data"]["cancelled"], true, "{cancel}");
    assert_eq!(cancel["data"]["action"], "click", "{cancel}");
    assert!(
        sent.elapsed() > Duration::from_millis(600),
        "the cancel waited for the tap held in its device call: {:?}",
        sent.elapsed()
    );
    let (clicked, _) = clicking.join().expect("the click's connection");
    assert_eq!(clicked["success"], true, "{clicked}");
    assert_eq!(clicked["data"]["accepted"], true, "{clicked}");
    assert!(
        clicked["warning"]
            .as_str()
            .is_some_and(|warning| warning.contains("cancel")),
        "{clicked}"
    );
    let after = daemon.request(json!({"action": "snapshot", "interactive": true}));
    assert!(
        after["data"]["snapshot"]
            .as_str()
            .is_some_and(|listing| listing.contains("id=tab-login,")),
        "the tap opened the fixture app: {after}"
    );
}

/// Every wait of a command sees the cancel: the waits for time, for a
/// settled screen, for a change, for a selector's element, and `open`'s
/// for the launched app.
#[test]
fn every_kind_of_wait_ends_at_a_cancel() {
    let splash = Daemon::builder()
        .fake("android")
        .env("MOBILE_AGENT_FAKE_SPLASH_MS", "8000")
        .start();
    let animating = opened();
    let clicked = animating.request(json!({"action": "click", "target": "#tab-anim"}));
    assert_eq!(clicked["success"], true, "{clicked}");
    let still = opened();
    let snapshot = still.request(json!({"action": "snapshot", "interactive": true}));
    assert_eq!(snapshot["success"], true, "{snapshot}");
    for (daemon, request) in [
        (&still, json!({"action": "wait", "ms": 8000})),
        (
            &animating,
            json!({"action": "wait", "settle": true, "timeout": 8000}),
        ),
        (
            &still,
            json!({"action": "wait", "change": true, "timeout": 8000}),
        ),
        (
            &still,
            json!({"action": "get", "what": "text", "target": "#nothing", "timeout": 8000}),
        ),
        (&splash, json!({"action": "open", "app": APP})),
    ] {
        let waiting = in_flight(daemon, request.clone());
        std::thread::sleep(Duration::from_millis(300));
        let sent = Instant::now();
        let cancel = daemon.request(json!({"action": "cancel"}));
        assert_eq!(cancel["data"]["cancelled"], true, "{request}: {cancel}");
        let (reply, at) = waiting.join().expect("the command's connection");
        assert_eq!(reply["code"], "cancelled", "{request}: {reply}");
        assert!(
            at.duration_since(sent) < Duration::from_millis(200),
            "{request}: {:?}",
            at.duration_since(sent)
        );
    }
}
