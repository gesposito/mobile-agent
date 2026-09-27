//! Batch execution (spec `agent-cli`: Batch execution), as agent-browser's
//! `batch`: quoted command strings or a JSON array of argv arrays on stdin,
//! `--bail` to stop at the first failure, and in JSON mode one array of
//! `{command, success, result, error, code?, warning?}`.

mod support;

use serde_json::{Value, json};
use support::{APP, TestEnv};

/// Whether the fake target shows its launcher, which `click @e2` leaves.
fn on_launcher(env: &TestEnv) -> bool {
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    listing.contains("- heading \"Home\"") && !listing.contains("login-submit")
}

#[test]
fn a_bailing_batch_stops_after_the_failing_second_command() {
    let env = TestEnv::start("android");
    let run = env
        .cli(&[
            "--json",
            "batch",
            "--bail",
            "snapshot -i",
            "click @e99",
            "click @e2",
        ])
        .failed();
    let results = run.json();
    let results = results.as_array().expect("an array of results");
    assert_eq!(
        results.len(),
        2,
        "the third command did not run: {results:?}"
    );

    assert_eq!(results[0]["command"], json!(["snapshot", "-i"]));
    assert_eq!(results[0]["success"], true);
    assert!(
        results[0]["result"]["snapshot"]
            .as_str()
            .unwrap_or("")
            .contains("Fixture")
    );
    assert_eq!(results[0]["error"], Value::Null);

    assert_eq!(results[1]["command"], json!(["click", "@e99"]));
    assert_eq!(results[1]["success"], false);
    assert_eq!(results[1]["code"], "unknown_ref");
    assert!(results[1]["error"].as_str().unwrap_or("").contains("e99"));
    assert_eq!(results[1]["result"], Value::Null);

    assert!(on_launcher(&env), "click @e2 never ran");
}

#[test]
fn a_batch_without_bail_runs_every_command_and_still_fails() {
    let env = TestEnv::start("android");
    let run = env
        .cli(&["--json", "batch", "snapshot -i", "click @e99", "click @e2"])
        .failed();
    let results = run.json();
    let successes: Vec<&Value> = results
        .as_array()
        .expect("an array")
        .iter()
        .map(|r| &r["success"])
        .collect();
    assert_eq!(successes, [&json!(true), &json!(false), &json!(true)]);
    assert!(!on_launcher(&env), "click @e2 ran and opened the app");
}

#[test]
fn a_batch_reads_a_json_array_of_argv_arrays_from_stdin() {
    let env = TestEnv::start("android");
    let run = env
        .cli_stdin(
            &["--json", "batch"],
            r##"[["snapshot", "-i"], ["click", "@e2"], ["get", "text", "#login-status"]]"##,
        )
        .ok();
    let results = run.json();
    let results = results.as_array().expect("an array");
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r["success"] == true), "{results:?}");
    assert_eq!(results[2]["result"]["text"], "Signed out");
}

#[test]
fn quoted_arguments_keep_their_spaces() {
    let env = TestEnv::start("android");
    let run = env
        .cli(&[
            "--json",
            "batch",
            &format!("open {APP}"),
            r##"fill #login-username "agent smith""##,
            "get text '#login-username'",
        ])
        .ok();
    let results = run.json();
    assert_eq!(
        results[1]["command"],
        json!(["fill", "#login-username", "agent smith"])
    );
    assert_eq!(results[2]["result"]["text"], "agent smith", "{results}");
}

#[test]
fn a_human_batch_prints_each_result_with_a_blank_line_between() {
    let env = TestEnv::start("android");
    let run = env.cli(&["batch", "snapshot -i", "click @e2"]).ok();
    let (listing, click) = run
        .stdout
        .split_once("\n\n")
        .expect("a blank line between the results");
    assert_eq!(
        listing,
        "- heading \"Home\" [id=launcher-title, ref=e1]\n- button \"Fixture\" [id=launcher-fixture, ref=e2]\n- button \"Settings\" [id=launcher-settings, ref=e3]"
    );
    assert!(click.starts_with("✓ Done (dispatch "), "{click}");
    assert_eq!(click.lines().count(), 1, "{click}");
    let failed = env
        .cli(&["batch", "--bail", "click @e2", "snapshot -i"])
        .failed();
    assert_eq!(failed.stdout, "");
    assert!(failed.stderr.contains("✗ "), "{}", failed.stderr);
}

#[test]
fn a_command_that_does_not_parse_is_a_failed_entry_with_a_code() {
    let env = TestEnv::start("android");
    let run = env
        .cli(&[
            "--json",
            "batch",
            "frobnicate now",
            "snapshot -i",
            "fill 'x",
        ])
        .failed();
    let results = run.json();
    assert_eq!(results[0]["command"], json!(["frobnicate", "now"]));
    assert_eq!(results[0]["success"], false);
    assert_eq!(results[0]["code"], "unknown_command", "{results}");
    assert_eq!(results[1]["success"], true);
    assert_eq!(results[2]["success"], false);
    assert_eq!(
        results[2]["code"], "invalid_argument",
        "an unclosed quote: {results}"
    );
}

#[test]
fn a_batch_applies_its_global_flags_to_every_command() {
    let env = TestEnv::start("android");
    env.cli(&["--session", "alice", "batch", &format!("open {APP}")])
        .ok();
    let status = env.cli(&["--json", "status"]).ok().json();
    assert_eq!(
        status["data"]["targets"][0]["claimedBy"], "alice",
        "{status}"
    );
    let nested = env
        .cli(&["--json", "batch", "batch status"])
        .failed()
        .json();
    assert_eq!(nested[0]["code"], "invalid_argument", "{nested}");
}

#[test]
fn an_empty_batch_prints_an_empty_array() {
    let env = TestEnv::start("android");
    let run = env.cli_stdin(&["--json", "batch"], "[]").ok();
    assert_eq!(run.stdout, "[]\n");
    let invalid = env
        .cli_stdin(&["--json", "batch"], "{\"not\": \"an array\"}")
        .failed();
    assert_eq!(invalid.json()["code"], "invalid_argument");
}
