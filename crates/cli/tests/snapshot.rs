//! Snapshot output (spec `agent-cli`: Command grammar, Output formats).

mod support;

use serde_json::Value;
use support::{APP, TestEnv, ends_with_ref};

#[test]
fn an_interactive_snapshot_lists_a_heading_and_two_buttons_each_ending_with_a_ref() {
    let env = TestEnv::start("android");
    let run = env.cli(&["snapshot", "-i"]).ok();
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{}", run.stdout);
    assert!(lines[0].starts_with("- heading \"Home\" ["), "{}", lines[0]);
    assert!(
        lines[1].starts_with("- button \"Fixture\" ["),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].starts_with("- button \"Settings\" ["),
        "{}",
        lines[2]
    );
    for line in &lines {
        assert!(ends_with_ref(line), "{line} does not end with a ref");
    }
    assert_eq!(run.stderr, "");
}

#[test]
fn the_json_snapshot_follows_agent_browser_shape() {
    let env = TestEnv::start("android");
    let run = env.cli(&["--json", "snapshot", "-i"]).ok();
    assert_eq!(
        run.stdout.lines().count(),
        1,
        "one line of JSON: {}",
        run.stdout
    );
    assert!(
        run.stdout.starts_with("{\"success\":true,\"data\":{"),
        "the envelope keeps agent-browser's field order: {}",
        run.stdout
    );
    let reply = run.json();
    assert_eq!(reply["error"], Value::Null);
    assert!(reply.get("code").is_none());
    let data = &reply["data"];
    let listing = data["snapshot"].as_str().expect("the listing");
    assert!(listing.contains("- button \"Fixture\" [id=launcher-fixture, ref=e2]"));
    assert_eq!(data["refs"]["e1"]["role"], "heading");
    assert_eq!(data["refs"]["e1"]["name"], "Home");
    assert_eq!(data["refs"]["e3"]["name"], "Settings");
    assert_eq!(data["removedRefs"], serde_json::json!([]));
    assert_eq!(data["origin"], "com.android.launcher3");
    assert_eq!(data["target"], "emulator-5554");
    assert_eq!(data["verdict"]["degraded"], false);
    assert_eq!(data["verdict"]["backend"], "fake");
}

#[test]
fn removed_refs_name_the_refs_the_previous_snapshot_listed() {
    let env = TestEnv::start("android");
    env.cli(&["snapshot", "-i"]).ok();
    env.cli(&["click", "@e2"]).ok();
    let reply = env.cli(&["--json", "snapshot", "-i"]).ok().json();
    let removed: Vec<&str> = reply["data"]["removedRefs"]
        .as_array()
        .expect("removed refs")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(removed, ["@e1", "@e2", "@e3"], "{reply}");
}

#[test]
fn a_full_snapshot_also_lists_text_and_system_bars() {
    let env = TestEnv::start("android");
    let full = env.cli(&["snapshot"]).ok().stdout;
    assert!(full.contains("- heading \"Home\""), "{full}");
    assert!(
        full.contains("- text \"12:30\""),
        "the status bar is listed: {full}"
    );
    let interactive = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(!interactive.contains("12:30"), "{interactive}");
}

#[test]
fn depth_limits_the_listing() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["click", "#tab-list"]).ok();
    let full = env.cli(&["snapshot"]).ok().stdout;
    assert!(
        full.contains("\n  - button \"Row 1\""),
        "rows nest under the list: {full}"
    );
    let shallow = env.cli(&["snapshot", "-d", "0"]).ok().stdout;
    assert!(shallow.contains("- list [id=list"), "{shallow}");
    assert!(
        !shallow.contains("  - "),
        "nothing below depth 0: {shallow}"
    );
    assert!(shallow.lines().count() < full.lines().count());
}

#[test]
fn delta_snapshots_start_full_then_report_unchanged_and_changes() {
    let env = TestEnv::start("android");
    let first = env
        .cli(&["--json", "snapshot", "-i", "--delta"])
        .ok()
        .json();
    assert_eq!(first["data"]["snapshot"]["kind"], "full", "{first}");
    assert_eq!(first["data"]["snapshot"]["revision"], 1);
    assert!(
        first["data"]["snapshot"]["tree"]
            .as_str()
            .unwrap_or("")
            .contains("Fixture")
    );

    let same = env
        .cli(&["--json", "snapshot", "-i", "--delta"])
        .ok()
        .json();
    assert_eq!(same["data"]["snapshot"]["kind"], "unchanged", "{same}");
    assert_eq!(same["data"]["snapshot"]["baseRevision"], 1);
    assert_eq!(same["data"]["snapshot"]["revision"], 2);
    let human = env.cli(&["snapshot", "-i", "--delta"]).ok().stdout;
    assert_eq!(human.trim(), "unchanged (revision 3)");

    let forced = env
        .cli(&["--json", "snapshot", "-i", "--delta", "--full"])
        .ok()
        .json();
    assert_eq!(forced["data"]["snapshot"]["kind"], "full", "{forced}");
    assert_eq!(forced["data"]["snapshot"]["revision"], 4);
}

#[test]
fn an_ios_target_lists_the_same_launcher() {
    let env = TestEnv::start("ios");
    let run = env.cli(&["snapshot", "-i"]).ok();
    assert_eq!(
        run.stdout,
        "- heading \"Home\" [id=launcher-title, ref=e1]\n- button \"Fixture\" [id=launcher-fixture, ref=e2]\n- button \"Settings\" [id=launcher-settings, ref=e3]\n"
    );
}
