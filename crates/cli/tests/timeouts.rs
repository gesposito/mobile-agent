//! Timeouts (spec `agent-cli`: Configurable timeouts): every waiting command
//! takes one, its default is documented in help, and a session default is
//! settable.

mod support;

use std::time::Duration;

use support::{APP, TestEnv};

#[test]
fn a_2_s_wait_fails_at_2_s_not_after_the_session_default() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["set", "timeout", "30000"]).ok();
    let run = env
        .cli(&[
            "--json",
            "wait",
            "--text",
            "Never shown",
            "--timeout",
            "2000",
        ])
        .failed();
    let reply = run.json();
    assert_eq!(reply["code"], "wait_timeout", "{reply}");
    assert!(
        reply["error"].as_str().unwrap_or("").contains("waited 20"),
        "{reply}"
    );
    assert!(
        run.elapsed >= Duration::from_millis(2000),
        "failed after {:?}",
        run.elapsed
    );
    assert!(
        run.elapsed < Duration::from_millis(5000),
        "failed after {:?}",
        run.elapsed
    );
}

#[test]
fn the_session_default_applies_to_waits_that_name_no_timeout() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    let set = env.cli(&["--json", "set", "timeout", "1000"]).ok().json();
    assert_eq!(set["data"]["defaultTimeoutMs"], 1000, "{set}");
    assert_eq!(set["data"]["session"], "default");
    let run = env.cli(&["--json", "wait", "#never-there"]).failed();
    assert_eq!(run.json()["code"], "wait_timeout");
    assert!(
        run.elapsed >= Duration::from_millis(1000),
        "{:?}",
        run.elapsed
    );
    assert!(
        run.elapsed < Duration::from_millis(4000),
        "{:?}",
        run.elapsed
    );
    let missing = env.cli(&["--json", "click", "#never-there"]).failed();
    assert_eq!(missing.json()["code"], "element_not_found");
    assert!(
        missing.elapsed < Duration::from_millis(4000),
        "{:?}",
        missing.elapsed
    );
}

#[test]
fn session_defaults_belong_to_their_session() {
    let env = TestEnv::start("android");
    env.cli(&["--session", "alice", "set", "timeout", "1500"])
        .ok();
    let status = env.cli(&["--json", "status"]).ok().json();
    let sessions = status["data"]["sessions"].as_array().expect("sessions");
    let alice = sessions
        .iter()
        .find(|s| s["name"] == "alice")
        .expect("alice");
    assert_eq!(alice["defaultTimeoutMs"], 1500, "{status}");
    assert!(sessions.iter().all(|s| s["name"] != "default"), "{status}");
}

#[test]
fn the_environment_default_applies_when_the_session_sets_none() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    let run = env.cli_env(
        &["--json", "wait", "--text", "Never shown"],
        &[("MOBILE_AGENT_DEFAULT_TIMEOUT", "800")],
    );
    let run = run.failed();
    assert_eq!(run.json()["code"], "wait_timeout");
    assert!(
        run.elapsed >= Duration::from_millis(800),
        "{:?}",
        run.elapsed
    );
    assert!(
        run.elapsed < Duration::from_millis(3500),
        "{:?}",
        run.elapsed
    );
}

#[test]
fn timeouts_are_bounded_and_must_be_numbers() {
    let env = TestEnv::start("android");
    let long = env
        .cli(&["--json", "set", "timeout", "3600000"])
        .failed()
        .json();
    assert_eq!(long["code"], "invalid_argument", "{long}");
    let word = env
        .cli(&["--json", "set", "timeout", "soon"])
        .failed()
        .json();
    assert_eq!(word["code"], "invalid_argument", "{word}");
    for args in [
        ["--json", "wait", "--settle", "--timeout", "700000"],
        ["--json", "open", APP, "--timeout", "700000"],
        ["--json", "wait", "#x", "--timeout", "700000"],
    ] {
        let reply = env.cli(&args).failed().json();
        assert_eq!(reply["code"], "invalid_argument", "{args:?}: {reply}");
    }
}

#[test]
fn a_long_wait_outlasts_a_short_idle_timeout() {
    let env = TestEnv::new("android");
    env.cli_env(&["open", APP], &[("MOBILE_AGENT_IDLE_TIMEOUT_MS", "500")])
        .ok();
    let run = env
        .cli(&[
            "--json",
            "wait",
            "--text",
            "Never shown",
            "--timeout",
            "1500",
        ])
        .failed();
    assert_eq!(
        run.json()["code"],
        "wait_timeout",
        "the daemon stayed up during the wait"
    );
}
