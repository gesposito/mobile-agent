//! Daemon lifecycle from the CLI (spec `agent-cli`: Daemon lifecycle): the
//! first command starts the daemon, later ones reuse it, and a daemon of
//! another build is replaced.

mod support;

use std::time::{Duration, Instant};

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
