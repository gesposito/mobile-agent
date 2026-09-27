//! Idle exit (spec `agent-cli`: Daemon lifecycle): the daemon leaves after a
//! configurable period without requests, one hour by default, never with 0.

mod support;

use std::time::{Duration, Instant};

use serde_json::json;
use support::{Daemon, PID, SOCKET, VERSION};

#[test]
fn the_daemon_exits_after_its_idle_timeout_and_removes_its_files() {
    let started = Instant::now();
    let mut daemon = Daemon::builder()
        .fake("android")
        .env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "400")
        .start();
    let status = daemon
        .wait_exit(Duration::from_secs(5))
        .expect("the daemon exits once idle");
    let lived = started.elapsed();
    assert!(status.success(), "idle exit is a clean exit: {status}");
    assert!(
        lived >= Duration::from_millis(400),
        "exited after {lived:?}"
    );
    for file in [SOCKET, PID, VERSION] {
        assert!(!daemon.dir.join(file).exists(), "{file} is left behind");
    }
}

#[test]
fn requests_postpone_the_idle_exit() {
    let mut daemon = Daemon::builder()
        .fake("android")
        .env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "600")
        .start();
    let busy_until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < busy_until {
        let status = daemon.request(json!({"action": "status"}));
        assert_eq!(status["success"], true, "{status}");
        std::thread::sleep(Duration::from_millis(150));
    }
    assert!(
        daemon.is_running(),
        "requests every 150 ms keep the daemon up"
    );
    let quiet = Instant::now();
    daemon
        .wait_exit(Duration::from_secs(5))
        .expect("the daemon exits once the requests stop");
    assert!(quiet.elapsed() >= Duration::from_millis(500));
}

#[test]
fn an_idle_timeout_of_zero_disables_the_exit() {
    let mut daemon = Daemon::builder()
        .fake("android")
        .env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "0")
        .start();
    std::thread::sleep(Duration::from_millis(1200));
    assert!(daemon.is_running());
    let status = daemon.request(json!({"action": "status"}));
    assert_eq!(
        status["data"]["daemon"]["idleTimeoutMs"],
        json!(null),
        "{status}"
    );
}

#[test]
fn the_idle_timeout_defaults_to_one_hour() {
    let daemon = Daemon::builder()
        .fake("android")
        .without_env("MOBILE_AGENT_IDLE_TIMEOUT_MS")
        .start();
    let status = daemon.request(json!({"action": "status"}));
    assert_eq!(
        status["data"]["daemon"]["idleTimeoutMs"], 3_600_000,
        "{status}"
    );
}

#[test]
fn idle_timeouts_take_the_units_agent_browser_takes() {
    for (given, ms) in [
        ("2500", 2500),
        ("3s", 3000),
        ("2m", 120_000),
        ("1h", 3_600_000),
    ] {
        let daemon = Daemon::builder()
            .fake("android")
            .arg("--idle-timeout")
            .arg(given)
            .start();
        let status = daemon.request(json!({"action": "status"}));
        assert_eq!(
            status["data"]["daemon"]["idleTimeoutMs"], ms,
            "{given}: {status}"
        );
    }
}
