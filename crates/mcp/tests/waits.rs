//! Waits (task 5.4 of the `mcp-server` change, design D7, and design D7 of
//! `real-app-fixes`): every call is bounded by `--call-timeout`, 55 s by
//! default, which caps a timeout the call names at the bound less a margin,
//! so the daemon's own wait ends first with its `wait_timeout`, the text
//! saying so, and gives no timeout to a call that names none, which keeps
//! the daemon's default of 10 s: a wait, or a click on a selector that
//! matches nothing, ends there, and `wait`'s description says to pass
//! `timeout` for longer; while a call runs, a client whose request carries
//! a progress token gets a progress notification every 5 s; a client's
//! cancellation sends the daemon the session's `cancel`, so the next call
//! runs at once; and a call the daemon has not answered 5 s past the bound
//! is cancelled by the server itself.

mod support;

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{Raw, TestEnv, is_error, ok, text};

/// A text no fake screen shows, so a wait for it runs to its timeout.
const ABSENT: &str = "Welcome aboard";

/// A selector no element of a fake screen matches, so a click on it waits
/// its timeout for the element.
const MISSING: &str = "#placeholder-nothing";

/// Whether `elapsed` lies between `from` and `to` seconds.
fn within(elapsed: Duration, from: f64, to: f64) -> bool {
    let seconds = elapsed.as_secs_f64();
    seconds >= from && seconds <= to
}

#[test]
fn the_server_takes_its_bound_its_profile_and_structured_with_their_defaults() {
    let env = TestEnv::start("android");
    let output = env
        .server_command_with(&["--help"])
        .stdin(Stdio::null())
        .output()
        .expect("the server runs");
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{help}\n{}", env.server_log());
    for needle in [
        "--call-timeout",
        "[default: 55]",
        "--tools",
        "read-only",
        "[default: all]",
        "--structured",
    ] {
        assert!(help.contains(needle), "the help lacks {needle:?}:\n{help}");
    }
    // A bound under the margin would leave the daemon no time to wait.
    let refused = env
        .server_command_with(&["--call-timeout", "2"])
        .stdin(Stdio::null())
        .output()
        .expect("the server runs");
    assert!(!refused.status.success(), "a bound of 2 s is refused");
}

/// A wait whose timeout lies past the bound, capped to the bound less the
/// margin, as the spec's scenario Wait past the bound has it.
#[test]
fn a_wait_past_the_bound_sends_progress_every_5_s_and_ends_with_the_daemon_s_timeout() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn_with(&env, &["--call-timeout", "12"]);
    let started = Instant::now();
    let id = client.send(
        "tools/call",
        json!({
            "name": "wait",
            "arguments": {"text": ABSENT, "timeout": 60000},
            "_meta": {"progressToken": "p1"},
        }),
    );
    let answer = client.response(id);
    let elapsed = started.elapsed();
    let result = answer
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("tools/call wait: {answer}"));
    assert!(is_error(&result), "{result}");
    let message = text(&result);
    assert!(
        message.starts_with("✗ wait_timeout: waited ")
            && message.contains(ABSENT)
            && message.contains("the screen now shows"),
        "the daemon's own timeout: {message}"
    );
    assert!(
        within(elapsed, 9.5, 11.5),
        "the wait ended at {elapsed:?}: the bound of 12 s less the margin of 2 s"
    );
    // One notification at 5 s, and one at 10 s when the tick beats the
    // daemon's reply, due at the same moment.
    let progress = client.progress("p1");
    assert!(
        !progress.is_empty() && progress.len() <= 2,
        "a notification every 5 s: {progress:?}"
    );
    for (n, (seconds, message, at)) in progress.iter().enumerate() {
        let due = 5.0 * (n as f64 + 1.0);
        assert_eq!(*seconds, due, "the seconds elapsed: {message}");
        assert!(
            message.contains("wait")
                && message.contains(ABSENT)
                && message.contains(&format!("{} s", due as u64)),
            "names the tool, the seconds and what it waits for: {message}"
        );
        let came = at.duration_since(started);
        assert!(
            within(came, due - 0.5, due + 1.5),
            "notification {n} came at {came:?}, due at {due} s"
        );
    }
    client.close();
}

#[test]
fn a_timeout_above_the_bound_is_capped_and_said_and_no_progress_goes_without_a_token() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn_with(&env, &["--call-timeout", "12"]);
    let started = Instant::now();
    let result = client.call("wait", json!({"text": ABSENT, "timeout": 60000}));
    let elapsed = started.elapsed();
    assert!(is_error(&result), "{result}");
    let message = text(&result);
    assert!(
        message.starts_with("✗ wait_timeout: ") && message.contains(ABSENT),
        "{message}"
    );
    assert!(
        message.contains("60000") && message.contains("capped") && message.contains("10000"),
        "the text says the timeout was capped to the bound less the margin: {message}"
    );
    assert!(within(elapsed, 9.5, 11.5), "the wait ended at {elapsed:?}");
    let came: Vec<&Value> = client
        .notifications
        .iter()
        .map(|notification| &notification.message)
        .collect();
    assert!(came.is_empty(), "no token, no progress: {came:?}");
    client.close();
}

/// Design D7 of real-app-fixes, scenario A click on a missing element: a
/// call that names no timeout sends none, so a click on a selector that
/// matches nothing fails with `element_not_found` at the daemon's default
/// of 10 s, not at the bound, 30 s here, less the margin.
#[test]
fn a_click_on_a_missing_element_without_a_timeout_fails_at_the_daemon_s_default() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn_with(&env, &["--call-timeout", "30"]);
    let started = Instant::now();
    let result = client.call("click", json!({"selector": MISSING}));
    let elapsed = started.elapsed();
    assert!(is_error(&result), "{result}");
    let message = text(&result);
    assert!(
        message
            .starts_with("✗ element_not_found: no element matches id placeholder-nothing after ")
            && message.contains("the screen shows"),
        "the daemon's own failure: {message}"
    );
    assert!(!message.contains("capped"), "nothing was capped: {message}");
    assert!(
        within(elapsed, 9.5, 12.5),
        "the click failed at {elapsed:?}: the daemon's default of 10 s, not the bound of 30 s less 2 s"
    );
    client.close();
}

/// Design D7 of real-app-fixes: a wait that names no timeout ends at the
/// daemon's default of 10 s with `wait_timeout` and the screen's summary,
/// not at the bound, 30 s here, less the margin.
#[test]
fn a_wait_without_a_timeout_ends_at_the_daemon_s_default() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn_with(&env, &["--call-timeout", "30"]);
    let started = Instant::now();
    let result = client.call("wait", json!({"text": ABSENT}));
    let elapsed = started.elapsed();
    assert!(is_error(&result), "{result}");
    let message = text(&result);
    assert!(
        message.starts_with("✗ wait_timeout: waited ")
            && message.contains(ABSENT)
            && message.contains("the screen now shows"),
        "the daemon's own timeout: {message}"
    );
    assert!(!message.contains("capped"), "nothing was capped: {message}");
    assert!(
        within(elapsed, 9.5, 12.5),
        "the wait ended at {elapsed:?}: the daemon's default of 10 s, not the bound of 30 s less 2 s"
    );
    client.close();
}

/// Design D7 of real-app-fixes: `wait`'s description says to pass
/// `timeout` for a wait past the daemon's default of 10 s, which a call
/// without one keeps.
#[test]
fn wait_s_description_says_to_pass_timeout_for_a_wait_past_the_default() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    let tools = client.list_tools();
    let wait = tools
        .iter()
        .find(|tool| tool["name"] == "wait")
        .unwrap_or_else(|| panic!("no wait tool: {tools:?}"));
    let description = wait["description"].as_str().unwrap_or_default();
    assert!(
        description.contains("pass `timeout`") && description.contains("10 s"),
        "{description}"
    );
    client.close();
}

#[test]
fn a_cancelled_call_ends_the_daemon_s_wait_so_the_next_call_runs_at_once() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    let id = client.send(
        "tools/call",
        json!({"name": "wait", "arguments": {"text": ABSENT, "timeout": 20000}}),
    );
    std::thread::sleep(Duration::from_secs(1));
    client.notify(
        "notifications/cancelled",
        json!({"requestId": id, "reason": "the client gave up"}),
    );
    let started = Instant::now();
    let listing = ok(&client.call("snapshot", json!({})));
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "the snapshot ran behind the wait for {elapsed:?}"
    );
    assert!(listing.contains("Fixture"), "{listing}");
    // The cancelled call gets no answer.
    assert!(
        client.notifications.iter().all(|notification| notification
            .message
            .get("id")
            .and_then(Value::as_u64)
            != Some(id)),
        "the cancelled call was answered"
    );
    client.close();
}

/// The fake holds a tap in its device call for 9 s, which no cancel
/// interrupts: the server sends the session's cancel 5 s past the bound of
/// 3 s, saying so on its standard error, and the daemon's reply, within
/// 5 s more, is the call's answer. The click's watch, whose time is counted
/// from the tap's dispatch, has none left after the hold, so the reply
/// carries no warning of the cancel.
#[test]
fn a_call_the_daemon_has_not_answered_past_the_bound_is_cancelled_by_the_server() {
    let env = TestEnv::start_with("android", &[("MOBILE_AGENT_FAKE_ACTIVATION_MS", "9000")]);
    let mut client = Raw::spawn_with(&env, &["--call-timeout", "3"]);
    let started = Instant::now();
    let result = client.call("click", json!({"selector": "#launcher-fixture"}));
    let elapsed = started.elapsed();
    let answer = ok(&result);
    assert!(
        answer.starts_with("✓ Done (hit button \"Fixture\", dispatch 9") && !answer.contains('\n'),
        "the daemon's reply: {answer}"
    );
    assert!(within(elapsed, 8.5, 12.5), "the call ended at {elapsed:?}");
    client.close();
    let log = env.server_log();
    assert!(
        log.contains(
            "click had no answer 5 s past the bound of 3 s; the session's cancel was sent"
        ),
        "the server said it cancelled the call: {log}"
    );
}
