//! Errors (spec `agent-cli`: Output formats, Ref syntax): exit status 1,
//! human messages on stderr, and in JSON mode an envelope with a code.

mod support;

use serde_json::Value;
use support::{APP, TestEnv, ref_of};

fn is_snake_case(code: &str) -> bool {
    !code.is_empty()
        && code
            .split('_')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase()))
}

#[test]
fn a_failed_command_in_json_mode_prints_an_error_object_with_a_code_and_exits_non_zero() {
    let env = TestEnv::start("android");
    let run = env.cli(&["--json", "click", "@e9"]).failed();
    assert_eq!(run.stdout.lines().count(), 1, "{}", run.stdout);
    let reply = run.json();
    assert_eq!(reply["success"], false);
    assert_eq!(reply["data"], Value::Null);
    assert_eq!(reply["code"], "unknown_ref", "{reply}");
    let message = reply["error"].as_str().expect("an error message");
    assert!(message.contains("e9"), "{message}");
    assert!(
        message.contains("snapshot"),
        "the message says what to do: {message}"
    );
}

#[test]
fn a_failed_command_in_human_mode_writes_its_error_to_stderr_and_exits_1() {
    let env = TestEnv::start("android");
    let run = env.cli(&["click", "@e9"]).failed();
    assert_eq!(run.stdout, "");
    assert!(run.stderr.starts_with("✗ "), "{}", run.stderr);
    assert!(run.stderr.contains("e9"), "{}", run.stderr);
}

#[test]
fn clicking_a_ref_from_before_navigation_fails_with_a_stale_ref_error_and_guidance() {
    let env = TestEnv::start("android");
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(listing.contains("ref=e2]"), "{listing}");
    env.cli(&["click", "@e2"]).ok();
    let after = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(
        after.contains("id=login-submit"),
        "the click opened the app: {after}"
    );

    let run = env.cli(&["click", "@e2"]).failed();
    assert!(run.stderr.contains("stale"), "{}", run.stderr);
    let reply = env.cli(&["--json", "click", "@e2"]).failed().json();
    assert_eq!(reply["code"], "stale_ref", "{reply}");
    let message = reply["error"].as_str().unwrap_or_default();
    assert!(message.contains("Take a new snapshot"), "{message}");
}

/// Design D3 of action-verdicts: a ref whose field the app has put in a
/// new node, as the placeholder app's rebuilt board does once the field
/// holds text, acts on its twin, and the line names the ref it took, as
/// `--json`'s `resolved` does.
#[test]
fn a_ref_whose_field_the_app_rebuilt_acts_on_its_twin_and_names_it() {
    let env = TestEnv::start("ios");
    let screen = "PLACEHOLDER_SCREEN=rebuilt";
    env.cli(&["open", "dev.mobileagent.placeholder", "--env", screen])
        .ok();
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    let field = ref_of(&listing, "placeholder-id-1");
    env.cli(&["fill", "#placeholder-id-1", "standup"]).ok();
    let clicked = env.cli(&["click", &field]).ok().stdout;
    assert!(
        clicked.starts_with(&format!("✓ Done ({field} now @e")),
        "{clicked}"
    );
    let reply = env.cli(&["--json", "get", "value", &field]).ok().json();
    assert_eq!(reply["data"]["value"], "standup", "{reply}");
    assert_eq!(
        reply["data"]["resolved"]["stale"],
        field.as_str(),
        "{reply}"
    );
    let twin = reply["data"]["resolved"]["ref"]
        .as_str()
        .unwrap_or_default();
    assert!(
        clicked.contains(&format!("{field} now {twin}")),
        "{clicked}"
    );
}

#[test]
fn refs_are_accepted_as_at_ref_equals_and_bare_forms() {
    for form in ["@e2", "ref=e2", "e2"] {
        let env = TestEnv::start("android");
        env.cli(&["snapshot", "-i"]).ok();
        env.cli(&["click", form]).ok();
        let after = env.cli(&["snapshot", "-i"]).ok().stdout;
        assert!(after.contains("id=login-submit"), "{form}: {after}");
    }
}

#[test]
fn every_failure_carries_a_snake_case_code() {
    let env = TestEnv::start("android");
    let usage = env.cli(&["--json", "click"]).failed().json();
    assert_eq!(usage["code"], "invalid_argument", "{usage}");
    let unknown = env.cli(&["--json", "frobnicate"]).failed().json();
    assert_eq!(unknown["code"], "unknown_command", "{unknown}");
    let missing = env
        .cli(&["--json", "open", "com.example.none"])
        .failed()
        .json();
    assert_eq!(missing["code"], "app_not_found", "{missing}");
    let bad_ref = env.cli(&["--json", "click", "@x1"]).failed().json();
    assert_eq!(bad_ref["code"], "invalid_argument", "{bad_ref}");
    env.cli(&["open", APP]).ok();
    let key = env.cli(&["--json", "press", "NoSuchKey"]).failed().json();
    assert_eq!(key["code"], "unknown_key", "{key}");
    for reply in [&usage, &unknown, &missing, &bad_ref, &key] {
        let code = reply["code"].as_str().unwrap_or_default();
        assert!(is_snake_case(code), "{code}");
        assert_eq!(reply["success"], false);
        assert!(reply["error"].is_string());
    }
}

/// Task 10.23: a fill that leaves other text in the field than it entered,
/// typed and then entered again in one piece, fails with `text_mismatch`,
/// whose message names what the field holds.
#[test]
fn a_fill_whose_field_keeps_other_text_fails_with_text_mismatch() {
    let env = TestEnv::start_with("ios", &[("MOBILE_AGENT_FAKE_TYPING", "max:4")]);
    env.cli(&["open", APP]).ok();
    let reply = env
        .cli(&["--json", "fill", "#login-username", "nobel"])
        .failed()
        .json();
    assert_eq!(reply["code"], "text_mismatch", "{reply}");
    assert_eq!(reply["data"]["value"], "nobe", "{reply}");
    let message = reply["error"].as_str().unwrap_or_default();
    assert!(message.contains("\"nobe\""), "{message}");
    assert!(
        message.contains("snapshot"),
        "the message says what to do: {message}"
    );
    let human = env.cli(&["fill", "#login-username", "nobel"]).failed();
    assert!(human.stderr.contains("\"nobe\""), "{}", human.stderr);
}

#[test]
fn an_unreachable_daemon_is_reported_with_a_code() {
    let env = TestEnv::new("android");
    let run = env
        .cli_env(
            &["--json", "status"],
            &[(
                "MOBILE_AGENT_DAEMON_BIN",
                "/nonexistent/mobile-agent-daemon",
            )],
        )
        .failed();
    let reply = run.json();
    assert_eq!(reply["code"], "daemon_unavailable", "{reply}");
}

#[test]
fn a_usage_error_in_human_mode_goes_to_stderr() {
    let env = TestEnv::start("android");
    let run = env.cli(&["click"]).failed();
    assert_eq!(run.stdout, "");
    assert!(run.stderr.contains("Usage"), "{}", run.stderr);
}
