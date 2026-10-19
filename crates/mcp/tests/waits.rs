//! Waits (task 5.4 of the `mcp-server` change, design D7): every call is
//! bounded by `--call-timeout`, 55 s by default, which the server passes to
//! the daemon as the command's timeout less a margin, so the daemon's own
//! wait ends first with its `wait_timeout`; a timeout above the bound is
//! capped the same way and the text says so; while a call runs, a client
//! whose request carries a progress token gets a progress notification
//! every 5 s; a client's cancellation sends the daemon the session's
//! `cancel`, so the next call runs at once; and a call the daemon has not
//! answered 5 s past the bound is cancelled by the server itself.

mod support;

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{Raw, TestEnv, is_error, ok, text};

/// A text no fake screen shows, so a wait for it runs to its timeout.
const ABSENT: &str = "Welcome aboard";

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

#[test]
fn a_wait_past_the_bound_sends_progress_every_5_s_and_ends_with_the_daemon_s_timeout() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn_with(&env, &["--call-timeout", "12"]);
    let started = Instant::now();
    let id = client.send(
        "tools/call",
        json!({
            "name": "wait",
            "arguments": {"text": ABSENT},
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
        client
            .notifications
            .iter()
            .all(|notification| notification.message.get("id").and_then(Value::as_u64) != Some(id)),
        "the cancelled call was answered"
    );
    client.close();
}

/// The fake holds a tap in its device call for 9 s, which no cancel
/// interrupts: the server's cancel goes out 5 s past the bound of 3 s and
/// cuts the click's watch short, and the daemon's reply, within 5 s more,
/// is the call's answer.
#[test]
fn a_call_the_daemon_has_not_answered_past_the_bound_is_cancelled_by_the_server() {
    let env = TestEnv::start_with("android", &[("MOBILE_AGENT_FAKE_ACTIVATION_MS", "9000")]);
    let mut client = Raw::spawn_with(&env, &["--call-timeout", "3"]);
    let started = Instant::now();
    let result = client.call("click", json!({"selector": "#launcher-fixture"}));
    let elapsed = started.elapsed();
    let answer = ok(&result);
    let lines: Vec<&str> = answer.lines().collect();
    assert!(lines[0].starts_with("✓ Done ("), "{answer}");
    assert!(
        lines.len() == 2 && lines[1].starts_with("⚠ ") && lines[1].contains("cancel"),
        "the server's cancel cut the watch short: {answer}"
    );
    assert!(within(elapsed, 8.5, 12.5), "the call ended at {elapsed:?}");
    client.close();
}
