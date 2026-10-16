//! `wait` with `change` (design D8 of the `mcp-server` change): a wait that
//! returns once the session's listing, in the options of its last snapshot,
//! differs from the one that snapshot showed, or with `frames` once the
//! frames moved too, with the delta `snapshot` with `delta` gives and what
//! moved, or fails with `wait_timeout` at its bound. A change the listing
//! does not show, such as the Login tab's status, a text an interactive
//! listing leaves out, leaves it waiting.

mod support;

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{APP, Daemon};

/// A daemon on the fake target `spec`, each change showing `latency_ms`
/// late, with the fixture app open on its Login tab.
fn opened(spec: &str, latency_ms: u64) -> Daemon {
    let daemon = Daemon::builder()
        .fake(spec)
        .env("MOBILE_AGENT_FAKE_LATENCY_MS", &latency_ms.to_string())
        .start();
    let opened = daemon.request(json!({"action": "open", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");
    daemon
}

/// Takes the session's baseline, an interactive delta snapshot, and returns
/// the replica's revision it listed.
#[track_caller]
fn baseline(daemon: &Daemon) -> u64 {
    let snapshot =
        daemon.request(json!({"action": "snapshot", "interactive": true, "delta": true}));
    assert_eq!(snapshot["success"], true, "{snapshot}");
    snapshot["data"]["replicaRevision"]
        .as_u64()
        .expect("replicaRevision")
}

/// Taps the Login tab's Sign in with no password. Its effect, the status
/// "Invalid credentials" in place of "Signed out", is a change of the tree
/// that an interactive listing does not show: the status is a text, which
/// gets no ref.
#[track_caller]
fn sign_in(daemon: &Daemon) {
    let clicked = daemon.request(json!({"action": "click", "target": "#login-submit"}));
    assert_eq!(clicked["success"], true, "{clicked}");
}

/// The Login tab's status as the replica holds it.
#[track_caller]
fn status(daemon: &Daemon) -> String {
    let reply = daemon.request(json!({"action": "get", "what": "text", "target": "#login-status"}));
    assert_eq!(reply["success"], true, "{reply}");
    reply["data"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Waits for a change for at most `ms` and checks that it failed with
/// `wait_timeout` at that bound.
#[track_caller]
fn times_out(daemon: &Daemon, request: Value, ms: u64, what: &str) {
    let started = Instant::now();
    let reply = daemon.request(request);
    let took = started.elapsed();
    assert_eq!(reply["code"], "wait_timeout", "{what}: {reply}");
    assert!(
        took >= Duration::from_millis(ms) && took < Duration::from_millis(ms + 1500),
        "{what}: it waited to its bound: {took:?}"
    );
}

/// The listing lines a delta object carries: the whole tree of a `full`
/// delta, the replaced lines of a `delta`, none for `unchanged`.
fn delta_lines(delta: &Value) -> String {
    let mut lines = delta["tree"].as_str().unwrap_or_default().to_owned();
    for line in delta["treeChange"]["lines"]
        .as_array()
        .into_iter()
        .flatten()
    {
        lines.push('\n');
        lines.push_str(line.as_str().unwrap_or_default());
    }
    lines
}

/// Checks that `reply` is a change wait that saw the List tab come in,
/// moved by `moved`, and returns its data.
#[track_caller]
fn saw_the_list(reply: &Value, moved: &str) -> Value {
    assert_eq!(reply["success"], true, "{reply}");
    let data = &reply["data"];
    assert_eq!(data["waited"], "change", "{reply}");
    assert_eq!(data["endedBy"], moved, "{reply}");
    let kind = data["snapshot"]["kind"].as_str().unwrap_or_default();
    assert!(kind == "full" || kind == "delta", "{reply}");
    assert!(
        delta_lines(&data["snapshot"]).contains("id=row-1,"),
        "the delta shows the list: {reply}"
    );
    data.clone()
}

/// The spec's "Wait for the next change": on a target without frames a
/// tap's effect that arrives after the tap's reply ends the wait at the
/// tree's change, with the delta.
#[test]
fn a_change_wait_returns_at_the_tree_change_a_tap_s_late_effect_brings() {
    let daemon = opened("android:R58M123", 400);
    baseline(&daemon);
    let clicked = daemon.request(json!({"action": "click", "target": "#tab-list"}));
    assert_eq!(clicked["success"], true, "{clicked}");
    assert!(
        clicked["data"]["treeDeltaMs"].is_null(),
        "the tap's effect comes after its reply: {clicked}"
    );
    let started = Instant::now();
    let reply = daemon.request(json!({"action": "wait", "change": true}));
    let took = started.elapsed();
    let data = saw_the_list(&reply, "tree");
    assert!(
        took < Duration::from_secs(2),
        "it returned at the change, not at its 10 s bound: {took:?}"
    );
    let elapsed = data["elapsedMs"].as_f64().expect("elapsedMs");
    assert!(
        elapsed > 50.0 && elapsed < 2000.0,
        "it waited for the change: {reply}"
    );
    // The reply moved the session's baseline as a delta snapshot does.
    let again = daemon.request(json!({"action": "wait", "change": true, "timeout": 300}));
    assert_eq!(again["code"], "wait_timeout", "{again}");
}

/// On targets with frames the same late effect moves the tree and the
/// frames, which the reply names as `both`.
#[test]
fn on_targets_with_frames_a_tap_s_late_effect_moves_the_tree_and_the_frames() {
    for spec in ["android", "ios"] {
        let daemon = opened(spec, 400);
        baseline(&daemon);
        let clicked = daemon.request(json!({"action": "click", "target": "#tab-list"}));
        assert_eq!(clicked["success"], true, "{spec}: {clicked}");
        let reply = daemon.request(json!({"action": "wait", "change": true}));
        saw_the_list(&reply, "both");
    }
}

/// A change that came between the session's last snapshot and the wait
/// returns at once: the wait is for an effect that may still be arriving,
/// not for the next change from now.
#[test]
fn a_change_since_the_session_s_last_snapshot_returns_at_once() {
    let daemon = opened("android", 0);
    baseline(&daemon);
    let clicked = daemon.request(json!({"action": "click", "target": "#tab-list"}));
    assert!(
        clicked["data"]["treeDeltaMs"].is_number(),
        "the tap's effect came before its reply: {clicked}"
    );
    let reply = daemon.request(json!({"action": "wait", "change": true}));
    let data = saw_the_list(&reply, "both");
    let elapsed = data["elapsedMs"].as_f64().expect("elapsedMs");
    assert!(elapsed < 100.0, "it returned at once: {reply}");
}

/// The spec's "A change outside the listing": the status, which the
/// session's interactive listing leaves out, changes during the wait, which
/// waits on to its bound, on a target without frames and on targets whose
/// frames move with the change.
#[test]
fn a_change_outside_the_session_s_listing_leaves_the_wait_running_to_its_bound() {
    for spec in ["android:R58M123", "android", "ios"] {
        let daemon = opened(spec, 400);
        let revision = baseline(&daemon);
        sign_in(&daemon);
        times_out(
            &daemon,
            json!({"action": "wait", "change": true, "timeout": 1000}),
            1000,
            spec,
        );
        // The tree changed, outside the listing.
        assert_eq!(status(&daemon), "Invalid credentials", "{spec}");
        let after =
            daemon.request(json!({"action": "snapshot", "interactive": true, "delta": true}));
        assert_eq!(
            after["data"]["snapshot"]["kind"], "unchanged",
            "{spec}: {after}"
        );
        assert!(
            after["data"]["replicaRevision"]
                .as_u64()
                .is_some_and(|now| now > revision),
            "{spec}: the replica moved past the baseline's revision {revision}: {after}"
        );
    }
}

/// The wait compares the listing again each time the revision moves: the
/// status that changed after the session's last snapshot is passed over,
/// and the List tab a later tap brings in ends the wait with its delta.
#[test]
fn a_change_inside_the_listing_ends_the_wait_after_one_outside_it() {
    for (spec, moved) in [("android:R58M123", "tree"), ("ios", "both")] {
        let daemon = opened(spec, 400);
        baseline(&daemon);
        sign_in(&daemon);
        let waited = daemon.request(json!({"action": "wait", "ms": 600}));
        assert_eq!(waited["success"], true, "{spec}: {waited}");
        assert_eq!(status(&daemon), "Invalid credentials", "{spec}");
        let clicked = daemon.request(json!({"action": "click", "target": "#tab-list"}));
        assert_eq!(clicked["success"], true, "{spec}: {clicked}");
        let reply = daemon.request(json!({"action": "wait", "change": true}));
        let data = saw_the_list(&reply, moved);
        let elapsed = data["elapsedMs"].as_f64().expect("elapsedMs");
        assert!(
            elapsed > 50.0 && elapsed < 2000.0,
            "{spec}: it waited past the status for the list: {reply}"
        );
    }
}

/// Frames that move alone, as the Anim tab's spinner's, leave a wait
/// without `frames` waiting to its bound, though they moved since the
/// session's last snapshot, as a wait with `frames` then shows at once.
#[test]
fn frames_that_move_alone_leave_a_wait_without_frames_running_to_its_bound() {
    for spec in ["android", "ios"] {
        let daemon = opened(spec, 0);
        let clicked = daemon.request(json!({"action": "click", "target": "#tab-anim"}));
        assert_eq!(clicked["success"], true, "{spec}: {clicked}");
        baseline(&daemon);
        times_out(
            &daemon,
            json!({"action": "wait", "change": true, "timeout": 1000}),
            1000,
            spec,
        );
        let started = Instant::now();
        let framed = daemon.request(json!({"action": "wait", "change": true, "frames": true}));
        assert_eq!(framed["data"]["endedBy"], "frame", "{spec}: {framed}");
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{spec}: the frames had moved: {framed}"
        );
    }
}

/// The spec's "Frames on request": with `frames`, frames that move alone, as
/// the Anim tab's spinner's, end the wait with `frame` and the listing
/// `unchanged`, as for a canvas or a map the tree does not describe.
#[test]
fn with_frames_frames_that_move_alone_end_the_wait_with_the_listing_unchanged() {
    for spec in ["android", "ios"] {
        let daemon = opened(spec, 0);
        let clicked = daemon.request(json!({"action": "click", "target": "#tab-anim"}));
        assert_eq!(clicked["success"], true, "{spec}: {clicked}");
        baseline(&daemon);
        let reply = daemon.request(json!({"action": "wait", "change": true, "frames": true}));
        assert_eq!(reply["success"], true, "{spec}: {reply}");
        assert_eq!(reply["data"]["endedBy"], "frame", "{spec}: {reply}");
        assert_eq!(
            reply["data"]["snapshot"]["kind"], "unchanged",
            "{spec}: {reply}"
        );
    }
}

/// With `frames` a change outside the listing ends the wait where frames
/// show it, as `frame` with the listing `unchanged`, `both` naming a
/// listing that changed; on a target without frames the listing alone
/// counts, so the wait runs to its bound.
#[test]
fn with_frames_a_change_outside_the_listing_ends_the_wait_where_frames_show_it() {
    for spec in ["android", "ios"] {
        let daemon = opened(spec, 400);
        baseline(&daemon);
        sign_in(&daemon);
        let reply = daemon
            .request(json!({"action": "wait", "change": true, "frames": true, "timeout": 2000}));
        assert_eq!(reply["success"], true, "{spec}: {reply}");
        assert_eq!(reply["data"]["endedBy"], "frame", "{spec}: {reply}");
        assert_eq!(
            reply["data"]["snapshot"]["kind"], "unchanged",
            "{spec}: {reply}"
        );
    }
    let daemon = opened("android:R58M123", 400);
    baseline(&daemon);
    sign_in(&daemon);
    times_out(
        &daemon,
        json!({"action": "wait", "change": true, "frames": true, "timeout": 1000}),
        1000,
        "a device without frames",
    );
    assert_eq!(status(&daemon), "Invalid credentials");
}

/// A still screen ends the wait at its bound with `wait_timeout`: the
/// command's timeout, else the session's default.
#[test]
fn a_still_screen_fails_with_wait_timeout_at_the_wait_s_bound() {
    let daemon = opened("android", 0);
    baseline(&daemon);
    for (request, bound) in [
        (
            json!({"action": "wait", "change": true, "timeout": 500}),
            500,
        ),
        (json!({"action": "set_timeout", "ms": 300}), 0),
        (json!({"action": "wait", "change": true}), 300),
    ] {
        let started = Instant::now();
        let reply = daemon.request(request.clone());
        let took = started.elapsed();
        if bound == 0 {
            assert_eq!(reply["success"], true, "{reply}");
            continue;
        }
        assert_eq!(reply["success"], false, "{request}: {reply}");
        assert_eq!(reply["code"], "wait_timeout", "{request}: {reply}");
        assert!(reply["data"].is_null(), "{reply}");
        assert!(
            reply["error"]
                .as_str()
                .is_some_and(|error| error.contains(&format!("{bound} ms"))),
            "{reply}"
        );
        assert!(
            took >= Duration::from_millis(bound) && took < Duration::from_millis(bound + 1500),
            "{request}: {took:?}"
        );
    }
}

/// A session that took no snapshot has seen no screen, so the wait answers
/// at once with the whole listing, as its first delta snapshot would.
#[test]
fn a_session_without_a_snapshot_gets_the_whole_listing_at_once() {
    let daemon = opened("android", 0);
    let reply = daemon.request(json!({"action": "wait", "change": true, "timeout": 2000}));
    assert_eq!(reply["success"], true, "{reply}");
    assert_eq!(reply["data"]["endedBy"], "tree", "{reply}");
    assert_eq!(reply["data"]["snapshot"]["kind"], "full", "{reply}");
    assert!(
        delta_lines(&reply["data"]["snapshot"]).contains("id=login-submit,"),
        "{reply}"
    );
    let still = daemon.request(json!({"action": "wait", "change": true, "timeout": 300}));
    assert_eq!(still["code"], "wait_timeout", "{still}");
}
