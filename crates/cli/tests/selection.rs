//! Target selection (spec `agent-cli`: Target selection): platform and
//! device flags; one connected target is selected by default, several
//! without a flag fail listing the candidates.

mod support;

use serde_json::{Value, json};
use support::{APP, TestEnv};

fn candidate_ids(reply: &Value) -> Vec<String> {
    reply["data"]["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .filter_map(|c| c["id"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn two_targets_without_a_device_flag_fail_listing_both() {
    let env = TestEnv::start("android,ios");
    let run = env.cli(&["snapshot"]).failed();
    assert!(run.stderr.contains("emulator-5554"), "{}", run.stderr);
    assert!(run.stderr.contains("FAKE-IOS-1"), "{}", run.stderr);
    assert!(run.stderr.contains("--device"), "{}", run.stderr);

    let reply = env.cli(&["--json", "snapshot"]).failed().json();
    assert_eq!(reply["code"], "ambiguous_target", "{reply}");
    assert_eq!(candidate_ids(&reply), ["emulator-5554", "FAKE-IOS-1"]);
    assert_eq!(reply["data"]["candidates"][1]["platform"], "ios");
    let message = reply["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("emulator-5554") && message.contains("FAKE-IOS-1"),
        "{message}"
    );
}

#[test]
fn one_connected_target_is_selected_by_default() {
    let env = TestEnv::start("ios");
    let reply = env.cli(&["--json", "snapshot", "-i"]).ok().json();
    assert_eq!(reply["data"]["target"], "FAKE-IOS-1");
}

#[test]
fn the_device_flag_selects_a_target_by_id_or_name() {
    let env = TestEnv::start("android,ios");
    let by_id = env
        .cli(&["--json", "--device", "FAKE-IOS-1", "snapshot", "-i"])
        .ok()
        .json();
    assert_eq!(by_id["data"]["target"], "FAKE-IOS-1");
    assert_eq!(by_id["data"]["origin"], "com.apple.springboard");
    let by_name = env
        .cli(&[
            "--json",
            "--device",
            "fake android emulator",
            "snapshot",
            "-i",
        ])
        .ok()
        .json();
    assert_eq!(by_name["data"]["target"], "emulator-5554", "{by_name}");
}

#[test]
fn the_platform_flag_selects_the_only_target_of_its_platform() {
    let env = TestEnv::start("android,ios");
    let reply = env
        .cli(&["--json", "--platform", "android", "snapshot"])
        .ok()
        .json();
    assert_eq!(reply["data"]["target"], "emulator-5554");
    let devices = env
        .cli(&["--json", "--platform", "ios", "devices"])
        .ok()
        .json();
    assert_eq!(devices["data"]["devices"].as_array().map(Vec::len), Some(1));
    assert_eq!(devices["data"]["devices"][0]["id"], "FAKE-IOS-1");
}

#[test]
fn a_platform_with_several_targets_is_still_ambiguous_among_them() {
    let env = TestEnv::start("android,android,ios");
    let reply = env
        .cli(&["--json", "--platform", "android", "snapshot"])
        .failed()
        .json();
    assert_eq!(reply["code"], "ambiguous_target", "{reply}");
    assert_eq!(candidate_ids(&reply), ["emulator-5554", "emulator-5556"]);
}

#[test]
fn a_device_flag_naming_no_target_fails_listing_the_connected_ones() {
    let env = TestEnv::start("android,ios");
    let reply = env
        .cli(&["--json", "--device", "nope", "snapshot"])
        .failed()
        .json();
    assert_eq!(reply["code"], "target_not_found", "{reply}");
    assert_eq!(candidate_ids(&reply), ["emulator-5554", "FAKE-IOS-1"]);
    let wrong_platform = env
        .cli(&[
            "--json",
            "--device",
            "FAKE-IOS-1",
            "--platform",
            "android",
            "snapshot",
        ])
        .failed()
        .json();
    assert_eq!(
        wrong_platform["code"], "target_not_found",
        "{wrong_platform}"
    );
    let bad = env
        .cli(&["--json", "--platform", "windows", "snapshot"])
        .failed()
        .json();
    assert_eq!(bad["code"], "invalid_argument", "{bad}");
}

#[test]
fn the_device_and_platform_flags_have_environment_twins() {
    let env = TestEnv::start("android,ios");
    let by_device = env
        .cli_env(
            &["--json", "snapshot", "-i"],
            &[("MOBILE_AGENT_DEVICE", "FAKE-IOS-1")],
        )
        .ok()
        .json();
    assert_eq!(by_device["data"]["target"], "FAKE-IOS-1");
    let by_platform = env
        .cli_env(
            &["--json", "snapshot", "-i"],
            &[("MOBILE_AGENT_PLATFORM", "android")],
        )
        .ok()
        .json();
    assert_eq!(by_platform["data"]["target"], "emulator-5554");
}

#[test]
fn a_session_that_holds_one_target_keeps_using_it_without_a_flag() {
    let env = TestEnv::start("android,ios");
    env.cli(&["--device", "FAKE-IOS-1", "open", APP]).ok();
    let reply = env.cli(&["--json", "snapshot", "-i"]).ok().json();
    assert_eq!(reply["data"]["target"], "FAKE-IOS-1", "{reply}");
    let other = env
        .cli(&["--json", "--session", "bob", "snapshot"])
        .failed()
        .json();
    assert_eq!(other["code"], "ambiguous_target", "{other}");
}

#[test]
fn claims_are_per_target() {
    let env = TestEnv::start("android,ios");
    env.cli(&[
        "--session",
        "alice",
        "--device",
        "emulator-5554",
        "open",
        APP,
    ])
    .ok();
    env.cli(&["--session", "bob", "--device", "FAKE-IOS-1", "open", APP])
        .ok();
    let alice = env
        .cli(&["--json", "--session", "alice", "snapshot", "-i"])
        .ok()
        .json();
    assert_eq!(alice["data"]["target"], "emulator-5554");
    let crossed = env
        .cli(&[
            "--json",
            "--session",
            "alice",
            "--device",
            "FAKE-IOS-1",
            "snapshot",
        ])
        .failed()
        .json();
    assert_eq!(crossed["code"], "target_claimed");
    assert_eq!(crossed["data"]["holder"], "bob");
    assert_eq!(
        env.cli(&["--json", "status"]).ok().json()["data"]["sessions"],
        json!([
            {"name": "alice", "targets": ["emulator-5554"], "defaultTimeoutMs": null},
            {"name": "bob", "targets": ["FAKE-IOS-1"], "defaultTimeoutMs": null}
        ])
    );
}

#[test]
fn no_connected_target_is_reported_with_a_code() {
    let env = TestEnv::start("");
    assert_eq!(
        env.cli(&["devices"]).ok().stdout,
        "(no targets connected)\n"
    );
    let reply = env.cli(&["--json", "snapshot"]).failed().json();
    assert_eq!(reply["code"], "no_target", "{reply}");
}
