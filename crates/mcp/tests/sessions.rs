//! Sessions and exit (task 5.2 of the `mcp-server` change, design D9): a
//! server mints its session, `mcp-<pid>`, a call may name another, and on
//! every way out, its input's end, SIGTERM, SIGINT or an opening that never
//! chose a lifecycle, the server releases the targets its sessions hold
//! with `close` and `keep`, so the app keeps running and the next session
//! attaches without `force`.

mod support;

use serde_json::{Value, json};
use support::{APP, Client, ERAS, Raw, TestEnv, is_error, ok, text};

/// The fake emulator every test drives.
const TARGET: &str = "emulator-5554";

/// The target as `status` reports it.
#[track_caller]
fn target(env: &TestEnv) -> Value {
    let status = env.status();
    status["targets"]
        .as_array()
        .and_then(|targets| targets.iter().find(|t| t["id"] == TARGET).cloned())
        .unwrap_or_else(|| panic!("no {TARGET} in {status}"))
}

/// A new server attaches to the target without `force`, finds the app as
/// the last one left it, and holds the target.
#[track_caller]
fn attaches(env: &TestEnv) {
    let mut next = Raw::spawn(env);
    let listing = ok(&next.call("snapshot", json!({})));
    assert!(
        listing.contains("id=login-username"),
        "the app the last server opened: {listing}"
    );
    assert_eq!(target(env)["claimedBy"], next.session().as_str());
    next.close();
}

#[test]
fn a_server_mints_its_session_and_a_call_may_name_another() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    let session = client.session();
    ok(&client.call("open", json!({"app": APP})));
    let held = target(&env);
    assert_eq!(held["claimedBy"], session.as_str(), "{held}");
    // Another session meets the server's claim.
    let other = client.call("snapshot", json!({"session": "resumed"}));
    assert!(is_error(&other), "{other}");
    let message = text(&other);
    assert!(
        message.starts_with("✗ target_claimed: ") && message.contains(&session),
        "{message}"
    );
    // Naming the server's own session is the same as naming none.
    let own = ok(&client.call("snapshot", json!({"session": session})));
    assert!(own.contains("id=login-username"), "{own}");
    client.close();
}

#[test]
fn a_server_whose_input_ends_releases_its_claim_and_leaves_the_app_running() {
    for era in ERAS {
        let env = TestEnv::start("android");
        let mut client = Client::spawn(&env, era);
        let session = client.session();
        ok(&client.call("open", json!({"app": APP})));
        assert_eq!(target(&env)["claimedBy"], session.as_str(), "{era}");
        let status = client.close();
        assert!(status.success(), "{era}: {status:?}\n{}", env.server_log());
        let left = target(&env);
        assert_eq!(left["claimedBy"], Value::Null, "{era}: released: {left}");
        assert_eq!(left["app"], APP, "{era}: the app keeps running: {left}");
        attaches(&env);
    }
}

#[test]
fn a_server_signalled_to_stop_releases_its_claim_and_leaves_the_app_running() {
    for signal in ["TERM", "INT"] {
        let env = TestEnv::start("android");
        let mut client = Raw::spawn(&env);
        ok(&client.call("open", json!({"app": APP})));
        let status = client.signal(signal);
        assert!(
            status.success(),
            "SIG{signal}: {status:?}\n{}",
            env.server_log()
        );
        let left = target(&env);
        assert_eq!(left["claimedBy"], Value::Null, "SIG{signal}: {left}");
        assert_eq!(left["app"], APP, "SIG{signal}: {left}");
        attaches(&env);
    }
}

/// A click on a selector claims the target without naming it in its reply;
/// the server finds the claim in `status` on its way out.
#[test]
fn a_server_releases_a_claim_no_reply_named() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    ok(&client.call("click", json!({"selector": "#launcher-fixture"})));
    assert_eq!(target(&env)["claimedBy"], client.session().as_str());
    assert!(client.close().success(), "{}", env.server_log());
    assert_eq!(target(&env)["claimedBy"], Value::Null);
    attaches(&env);
}

/// A client that discovers the server and leaves before any request that
/// chooses a lifecycle ends rmcp's initialization with an error: the
/// server exits all the same, with nothing to release.
#[test]
fn a_server_that_was_only_discovered_exits_when_its_input_ends() {
    let env = TestEnv::start("android");
    let client = Raw::spawn(&env);
    let status = client.close();
    assert!(status.success(), "{status:?}\n{}", env.server_log());
    assert_eq!(target(&env)["claimedBy"], Value::Null);
}
