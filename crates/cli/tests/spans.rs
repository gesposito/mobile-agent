//! Timing a tap's effects and a settle's end (task 8.9): the spans the
//! benchmark harness reads as tap-dispatch, tap-to-frame-change,
//! tap-to-tree-delta and settle, taken from the daemon's own clock.

mod support;

use serde_json::Value;
use support::{APP, TestEnv, ref_of};

/// How long the fake targets in these tests take to show a change.
const LATENCY: &str = "120";
const LATENCY_MS: f64 = 120.0;

/// A point on the launcher where no element is.
const EMPTY: [&str; 2] = ["204", "600"];

fn slow(fake: &str) -> TestEnv {
    TestEnv::start_with(fake, &[("MOBILE_AGENT_FAKE_LATENCY_MS", LATENCY)])
}

#[track_caller]
fn ms(data: &Value, key: &str) -> f64 {
    data[key]
        .as_f64()
        .unwrap_or_else(|| panic!("{key} is not a number of ms in {data}"))
}

/// Taps the launcher's Fixture button, which opens the fixture app, and
/// returns the reply's data.
fn open_fixture(env: &TestEnv, command: &str) -> Value {
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    let fixture = ref_of(&listing, "launcher-fixture");
    env.cli(&["--json", command, &fixture]).ok().json()["data"].clone()
}

#[test]
fn a_click_reports_the_first_frame_change_and_the_first_tree_delta() {
    let env = slow("android");
    let data = open_fixture(&env, "click");
    let dispatch = ms(&data, "dispatchMs");
    let frame = ms(&data, "frameChangeMs");
    let tree = ms(&data, "treeDeltaMs");
    assert!(dispatch < LATENCY_MS, "dispatch {dispatch} ms: {data}");
    assert!(
        (LATENCY_MS..1000.0).contains(&frame),
        "the first changed frame comes with the change, {LATENCY} ms after the tap: {data}"
    );
    assert!(
        (LATENCY_MS..1000.0).contains(&tree),
        "the first tree delta comes with the change: {data}"
    );
    assert!(
        ms(&data, "watchedMs") >= frame.max(tree),
        "the daemon watched until both arrived: {data}"
    );
}

#[test]
fn a_tap_reports_the_same_spans_as_a_click() {
    let env = slow("android");
    let data = open_fixture(&env, "tap");
    assert!(ms(&data, "frameChangeMs") >= LATENCY_MS, "{data}");
    assert!(ms(&data, "treeDeltaMs") >= LATENCY_MS, "{data}");
    assert!(ms(&data, "dispatchMs") < LATENCY_MS, "{data}");
}

#[test]
fn an_ios_target_reports_the_tree_delta_its_re_reads_find() {
    let env = slow("ios");
    let data = open_fixture(&env, "click");
    let frame = ms(&data, "frameChangeMs");
    let tree = ms(&data, "treeDeltaMs");
    assert!((LATENCY_MS..1000.0).contains(&frame), "{data}");
    // Simulator windows send no events: the re-read right after the tap
    // comes too early, and the next one follows the changed frames.
    assert!((LATENCY_MS..1000.0).contains(&tree), "{data}");
}

#[test]
fn a_target_without_frames_reports_no_frame_change_rather_than_zero() {
    let env = slow("android:R58M123");
    let data = open_fixture(&env, "click");
    assert!(data.get("frameChangeMs").is_none(), "{data}");
    assert!(ms(&data, "treeDeltaMs") >= LATENCY_MS, "{data}");
}

/// Task 8.13: an emulator's frame stream delivers its first frame a while
/// after the attach, and the first frame is no change, so a tap whose
/// effects came before it would report no frame change. `open` waits for
/// the first frame on a target with a frame source, so the first click
/// after it reports its first frame change.
#[test]
fn the_first_click_after_open_reports_its_first_frame_change() {
    let env = TestEnv::start_with("android", &[("MOBILE_AGENT_FAKE_FRAME_DELAY_MS", "1200")]);
    env.cli(&["open", APP]).ok();
    let data = env.cli(&["--json", "click", "#tab-list"]).ok().json()["data"].clone();
    assert!(data["frameChangeMs"].is_number(), "{data}");
    assert!(data["treeDeltaMs"].is_number(), "{data}");
}

/// Task 8.15: a Simulator's input session activates on its first input
/// (Bluesky's first click took 957 ms to dispatch while dtuhidd
/// activated), so `open` starts the session and lets it activate: the
/// first click after it dispatches at once.
#[test]
fn the_first_click_after_open_does_not_wait_for_the_input_session() {
    let env = TestEnv::start_with("ios", &[("MOBILE_AGENT_FAKE_ACTIVATION_MS", "800")]);
    env.cli(&["open", APP]).ok();
    let data = env.cli(&["--json", "click", "#tab-list"]).ok().json()["data"].clone();
    assert!(ms(&data, "dispatchMs") < 400.0, "{data}");
}

#[test]
fn a_tap_that_changes_nothing_reports_neither_span() {
    let env = slow("android");
    let data = env
        .cli(&["--json", "click", EMPTY[0], EMPTY[1]])
        .ok()
        .json()["data"]
        .clone();
    assert!(data.get("frameChangeMs").is_none(), "{data}");
    assert!(data.get("treeDeltaMs").is_none(), "{data}");
    assert!(data["dispatchMs"].is_number(), "{data}");
    assert!(
        ms(&data, "watchedMs") >= 200.0,
        "the daemon watched for the quiet window: {data}"
    );
}

#[test]
fn a_click_prints_its_spans() {
    let env = TestEnv::start("android");
    // On the quiet launcher first: a tap during another tap's transition
    // would see that transition's frames change.
    let nothing = env.cli(&["click", EMPTY[0], EMPTY[1]]).ok();
    assert!(nothing.stdout.contains("no change"), "{}", nothing.stdout);

    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    let run = env
        .cli(&["click", &ref_of(&listing, "launcher-fixture")])
        .ok();
    let line = run.stdout.trim_end();
    assert!(line.starts_with("✓ Done (dispatch "), "{line}");
    assert!(line.contains(" ms, first frame change "), "{line}");
    assert!(line.contains(" ms, first tree delta "), "{line}");
    assert!(line.ends_with(" ms)"), "{line}");
}

/// Runs `action`, then `wait --settle` in the same batch, so the wait
/// starts well within the quiet window after the action however slowly
/// processes start, and returns the wait's data.
fn settle_after(env: &TestEnv, action: &str) -> Value {
    let results = env
        .cli(&["--json", "batch", "--bail", action, "wait --settle"])
        .ok()
        .json();
    assert_eq!(results[0]["success"], true, "{results}");
    results[1]["result"].clone()
}

/// The edit is a key pressed into the field in focus, which replies once
/// the key went in: `fill` and `type` reply only once the field has held
/// their text for the quiet window (task 10.23; real-app-fixes, design D3),
/// so a settle after either finds the screen quiet already.
#[test]
fn a_settle_after_an_edit_ended_on_both_signals() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["fill", "#login-username", "agen"]).ok();
    let data = settle_after(&env, "press t");
    assert_eq!(data["settled"], true, "{data}");
    assert_eq!(
        data["endedBy"], "both",
        "the edit changed the tree and a frame at once: {data}"
    );
    assert_eq!(data["stillChanging"], serde_json::json!([]));
}

#[test]
fn a_settle_after_a_transition_ended_on_the_frames() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    let data = settle_after(&env, "click #tab-list");
    assert_eq!(data["settled"], true, "{data}");
    assert_eq!(
        data["endedBy"], "frame",
        "the transition's frames outlast the tree change: {data}"
    );
}

#[test]
fn a_settle_on_a_target_without_frames_ended_on_the_tree() {
    let env = TestEnv::start("android:R58M123");
    env.cli(&["open", APP]).ok();
    let data = settle_after(&env, "click #tab-list");
    assert_eq!(data["settled"], true, "{data}");
    assert_eq!(data["endedBy"], "tree", "{data}");
}

#[test]
fn a_settle_that_times_out_names_the_signal_still_changing() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["click", "#tab-anim"]).ok();
    let data = env
        .cli(&["--json", "wait", "--settle", "--timeout", "800"])
        .ok()
        .json()["data"]
        .clone();
    assert_eq!(data["settled"], false, "{data}");
    assert_eq!(data["endedBy"], "timeout", "{data}");
    assert_eq!(
        data["stillChanging"],
        serde_json::json!(["frame"]),
        "{data}"
    );
}

#[test]
fn a_settle_on_a_screen_already_quiet_ended_on_nothing() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    let data = env.cli(&["--json", "wait", "--settle"]).ok().json()["data"].clone();
    assert_eq!(data["settled"], true, "{data}");
    assert_eq!(
        data["endedBy"], "none",
        "open waited the changes out: {data}"
    );
}

#[test]
fn a_settle_prints_the_signal_it_ended_on() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    let run = env.cli(&["batch", "click #tab-list", "wait --settle"]).ok();
    let settled = run.stdout.split("\n\n").nth(1).unwrap_or_default();
    assert!(settled.starts_with("✓ Settled after "), "{}", run.stdout);
    assert!(settled.contains(": frames changed last"), "{}", run.stdout);

    env.cli(&["click", "#tab-anim"]).ok();
    let timeout = env.cli(&["wait", "--settle", "--timeout", "500"]).ok();
    assert!(
        timeout.stdout.starts_with("✓ Stopped waiting after "),
        "{}",
        timeout.stdout
    );
    assert!(
        timeout.stdout.contains(": frames kept changing"),
        "{}",
        timeout.stdout
    );
}
