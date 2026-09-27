//! Sessions (spec `agent-cli`: Sessions): claims and refs are isolated
//! between sessions, and a force flag takes a target over.

mod support;

use support::{APP, TestEnv};

#[test]
fn a_claimed_target_refuses_another_session_naming_the_holder_until_forced() {
    let env = TestEnv::start("android");
    env.cli(&["--session", "alice", "open", APP]).ok();
    let refused = env.cli(&["--session", "bob", "snapshot"]).failed();
    assert!(refused.stderr.contains("\"alice\""), "{}", refused.stderr);
    let reply = env
        .cli(&["--json", "--session", "bob", "snapshot"])
        .failed()
        .json();
    assert_eq!(reply["code"], "target_claimed", "{reply}");
    assert_eq!(reply["data"]["holder"], "alice");

    env.cli(&["--session", "bob", "--force", "snapshot"]).ok();
    let back = env
        .cli(&["--json", "--session", "alice", "snapshot"])
        .failed()
        .json();
    assert_eq!(back["data"]["holder"], "bob", "{back}");
}

#[test]
fn refs_are_isolated_between_sessions() {
    let env = TestEnv::start("android");
    env.cli(&["--session", "alice", "snapshot", "-i"]).ok();
    env.cli(&["--session", "alice", "close"]).ok();
    let refused = env
        .cli(&["--json", "--session", "bob", "click", "@e2"])
        .failed()
        .json();
    assert_eq!(refused["code"], "unknown_ref", "{refused}");
    assert!(
        refused["error"].as_str().unwrap_or("").contains("session"),
        "{refused}"
    );
    env.cli(&["--session", "bob", "snapshot", "-i"]).ok();
    env.cli(&["--session", "bob", "click", "@e2"]).ok();
}

#[test]
fn the_session_and_json_flags_have_environment_twins() {
    let env = TestEnv::start("android");
    let run = env
        .cli_env(
            &["open", APP],
            &[
                ("MOBILE_AGENT_SESSION", "alice"),
                ("MOBILE_AGENT_JSON", "1"),
            ],
        )
        .ok();
    assert_eq!(run.json()["data"]["opened"], APP);
    let reply = env
        .cli_env(&["snapshot"], &[("MOBILE_AGENT_JSON", "true")])
        .failed()
        .json();
    assert_eq!(reply["data"]["holder"], "alice", "{reply}");
    let forced = env
        .cli_env(&["snapshot"], &[("MOBILE_AGENT_FORCE", "1")])
        .ok();
    assert!(forced.stdout.contains("login-submit"), "{}", forced.stdout);
}
