//! Daemon lifecycle from the CLI (spec `agent-cli`: Daemon lifecycle): the
//! first command starts the daemon, later ones reuse it, a daemon of
//! another build is replaced, and `shutdown` returns once the daemon has
//! let go of its targets.

mod support;

use std::time::{Duration, Instant};

use serde_json::json;
use support::TestEnv;

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn wait_dead(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(pid) {
        assert!(Instant::now() < deadline, "process {pid} is still alive");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_cold_devices_starts_the_daemon_and_the_next_command_reuses_it() {
    let env = TestEnv::new("android");
    assert!(!env.socket().exists(), "no daemon runs yet");
    let first = env.cli(&["devices"]).ok();
    assert!(first.stdout.contains("emulator-5554"), "{}", first.stdout);
    let pid = env
        .daemon_pid()
        .expect("the first command started a daemon");
    assert!(alive(pid));

    let second = env.cli(&["--json", "devices"]).ok().json();
    assert_eq!(second["data"]["devices"][0]["id"], "emulator-5554");
    assert_eq!(
        env.daemon_pid(),
        Some(pid),
        "the second command reused the daemon"
    );
    let status = env.cli(&["--json", "status"]).ok().json();
    assert_eq!(status["data"]["daemon"]["pid"], pid);
}

#[test]
fn a_daemon_of_another_version_is_replaced_by_the_next_command() {
    let env = TestEnv::new("android");
    env.cli(&["status"]).ok();
    let old = env.daemon_pid().expect("a daemon");
    std::fs::write(env.path().join("daemon.version"), "0.0.1+old\n").expect("writable");

    let status = env.cli(&["--json", "status"]).ok().json();
    let new = env.daemon_pid().expect("a daemon");
    assert_ne!(new, old, "the daemon was replaced");
    assert_eq!(status["data"]["daemon"]["pid"], new);
    wait_dead(old);
    let version =
        std::fs::read_to_string(env.path().join("daemon.version")).expect("a version file");
    assert!(
        version.starts_with(&format!("{}+", env!("CARGO_PKG_VERSION"))),
        "{version}"
    );
}

#[test]
fn files_left_by_a_killed_daemon_do_not_block_the_next_command() {
    let env = TestEnv::new("android");
    env.cli(&["status"]).ok();
    let killed = env.daemon_pid().expect("a daemon");
    std::process::Command::new("kill")
        .args(["-9", &killed.to_string()])
        .status()
        .expect("kill runs");
    wait_dead(killed);
    assert!(
        env.socket().exists(),
        "a killed daemon leaves its socket file"
    );

    env.cli(&["status"]).ok();
    let new = env.daemon_pid().expect("a new daemon");
    assert_ne!(new, killed);
}

#[test]
fn concurrent_first_commands_share_one_daemon() {
    let env = TestEnv::new("android");
    let children: Vec<_> = (0..4)
        .map(|_| {
            env.command(&["--json", "status"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("the CLI starts")
        })
        .collect();
    let mut pids = Vec::new();
    for child in children {
        let output = child.wait_with_output().expect("the CLI finishes");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reply: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("a JSON reply");
        pids.push(reply["data"]["daemon"]["pid"].as_u64().expect("a pid"));
    }
    pids.dedup();
    assert_eq!(pids.len(), 1, "one daemon served every command: {pids:?}");
}

#[test]
fn the_idle_timeout_flag_reaches_the_daemon_the_cli_starts() {
    let env = TestEnv::new("android");
    let status = env
        .cli(&["--json", "--idle-timeout", "2s", "status"])
        .ok()
        .json();
    assert_eq!(status["data"]["daemon"]["idleTimeoutMs"], 2000, "{status}");
}

/// Task 10.12: the emulator controller setting reaches the daemon the CLI
/// starts from the flag, the variable or a settings file, and a fake
/// emulator then attaches without a frame source, its input through the
/// agent.
#[test]
fn the_emulator_controller_setting_reaches_the_daemon_the_cli_starts() {
    let controller =
        |status: &serde_json::Value| status["data"]["daemon"]["emulatorController"].clone();
    let flag = TestEnv::new("android");
    let status = flag
        .cli(&["--json", "--emulator-controller", "off", "status"])
        .ok()
        .json();
    assert_eq!(controller(&status), "off", "{status}");
    flag.cli(&["snapshot"]).ok();
    let status = flag.cli(&["--json", "status"]).ok().json();
    let emulator = &status["data"]["targets"][0];
    assert_eq!(emulator["id"], "emulator-5554", "{status}");
    assert_eq!(emulator["frameSource"], false, "{status}");
    assert_eq!(emulator["driver"]["input"], "agent", "{status}");
    let human = flag.cli(&["status"]).ok().stdout;
    assert!(human.contains("emulator controller off"), "{human}");

    let variable = TestEnv::new("android");
    let status = variable
        .cli_env(
            &["--json", "status"],
            &[("MOBILE_AGENT_EMULATOR_CONTROLLER", "off")],
        )
        .ok()
        .json();
    assert_eq!(controller(&status), "off", "{status}");

    let settings = TestEnv::new("android");
    std::fs::write(
        settings.path().join("mobile-agent.json"),
        r#"{"emulatorController": "off"}"#,
    )
    .expect("the settings file is written");
    let status = settings.cli(&["--json", "status"]).ok().json();
    assert_eq!(controller(&status), "off", "{status}");

    let default = TestEnv::new("android");
    let status = default.cli(&["--json", "status"]).ok().json();
    assert_eq!(controller(&status), "auto", "{status}");
}

/// A fake target that takes 600 ms to stop, as stopping the Android agent
/// takes its time: the environment of the command that starts the daemon.
const SLOW_STOP: [(&str, &str); 1] = [("MOBILE_AGENT_FAKE_STOP_MS", "600")];

/// Task 8.16: `shutdown` returns once the daemon has stopped its targets
/// and removed its socket and sidecar files, so it takes as long as the
/// target's stop, and nothing of the daemon is left when it returns. A
/// second `shutdown` finds no daemon running.
#[test]
fn shutdown_returns_once_the_daemon_has_stopped_its_targets_and_removed_its_socket() {
    let env = TestEnv::new("android");
    env.cli_env(&["snapshot"], &SLOW_STOP).ok();
    let pid = env
        .daemon_pid()
        .expect("the first command started a daemon");
    let run = env.cli(&["--json", "shutdown"]).ok();
    let left: Vec<&str> = ["daemon.sock", "daemon.pid", "daemon.version"]
        .into_iter()
        .filter(|file| env.path().join(file).exists())
        .collect();
    assert_eq!(
        run.json(),
        json!({"success": true, "data": {"stopped": true}, "error": null})
    );
    assert!(
        run.elapsed >= Duration::from_millis(600),
        "shutdown returned after {:?}, before the target's 600 ms stop ended",
        run.elapsed
    );
    assert!(
        left.is_empty(),
        "shutdown returned with {left:?} still there"
    );
    wait_dead(pid);

    let again = env.cli(&["--json", "shutdown"]).ok().json();
    assert_eq!(
        again,
        json!({
            "success": true,
            "data": {"stopped": false},
            "error": null,
            "warning": "no daemon was running"
        })
    );
    let human = env.cli(&["shutdown"]).ok();
    assert_eq!(
        (human.stdout.as_str(), human.stderr.as_str()),
        ("✓ Daemon stopped\n", "⚠ no daemon was running\n")
    );
}

/// Task 8.16: a command right after `shutdown` starts a fresh daemon. The
/// old daemon's socket is gone once `shutdown` returns, so the command
/// neither reaches the old daemon nor finds its socket to remove as stale
/// while the old daemon still stops its targets, and it attaches its
/// target anew.
#[test]
fn a_command_right_after_shutdown_starts_a_fresh_daemon_without_meeting_the_old_socket() {
    let env = TestEnv::new("android");
    env.cli_env(&["snapshot"], &SLOW_STOP).ok();
    let old = env
        .daemon_pid()
        .expect("the first command started a daemon");
    env.cli(&["shutdown"]).ok();
    assert!(
        !env.socket().exists(),
        "the old daemon's socket was still there when shutdown returned"
    );
    let status = env.cli(&["--json", "status"]).ok().json();
    let new = status["data"]["daemon"]["pid"]
        .as_u64()
        .expect("a fresh daemon's pid");
    assert_ne!(new, u64::from(old), "the old daemon answered: {status}");
    assert_eq!(env.daemon_pid(), u32::try_from(new).ok());
    assert_eq!(
        status["data"]["targets"][0]["attached"], false,
        "the fresh daemon has not attached the target yet: {status}"
    );
    let snapshot = env.cli(&["--json", "snapshot"]).ok().json();
    assert_eq!(snapshot["data"]["target"], "emulator-5554", "{snapshot}");
    wait_dead(old);
}

#[test]
fn a_daemon_that_fails_to_start_is_reported_with_its_error() {
    let env = TestEnv::new("windows");
    let reply = env.cli(&["--json", "status"]).failed().json();
    assert_eq!(reply["code"], "daemon_unavailable", "{reply}");
    let message = reply["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("android or ios"),
        "the daemon's own error: {message}"
    );
}
