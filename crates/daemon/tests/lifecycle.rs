//! The daemon's socket, sidecar files, version check and wire contract
//! (design D1, spec `agent-cli`: Daemon lifecycle).

mod support;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::json;
use support::{APP, Daemon, PID, SOCKET, TempDir, VERSION};

#[test]
fn the_daemon_writes_its_pid_and_version_files() {
    let daemon = Daemon::start("android");
    assert_eq!(
        daemon.file(PID).as_deref().map(str::trim),
        Some(daemon.pid().to_string().as_str())
    );
    assert_eq!(
        daemon.file(VERSION).as_deref().map(str::trim),
        Some(support::expected_version().as_str())
    );
    let status = daemon.request(json!({"action": "status"}));
    assert_eq!(status["data"]["daemon"]["pid"], daemon.pid(), "{status}");
    assert_eq!(
        status["data"]["daemon"]["version"],
        support::expected_version()
    );
    assert_eq!(
        status["data"]["daemon"]["socket"],
        daemon.socket().to_string_lossy().as_ref()
    );
}

#[test]
fn a_daemon_of_another_version_is_shut_down_and_replaced() {
    let dir = TempDir::new();
    let mut old = Daemon::builder()
        .fake("android")
        .socket_dir(dir.path())
        .start();
    std::fs::write(dir.path().join(VERSION), "0.0.1+old\n").expect("the version file is writable");

    let new = Daemon::builder()
        .fake("android")
        .socket_dir(dir.path())
        .start();
    let status = old
        .wait_exit(Duration::from_secs(10))
        .expect("the old daemon is asked to leave");
    assert!(status.success(), "the old daemon leaves cleanly: {status}");
    let reply = new.request(json!({"action": "status"}));
    assert_eq!(reply["data"]["daemon"]["pid"], new.pid(), "{reply}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(VERSION))
            .expect("a version file")
            .trim(),
        support::expected_version()
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join(PID))
            .expect("a pid file")
            .trim(),
        new.pid().to_string()
    );
}

#[test]
fn a_second_daemon_of_the_same_version_leaves_the_first_serving() {
    let dir = TempDir::new();
    let mut first = Daemon::builder()
        .fake("android")
        .socket_dir(dir.path())
        .start();
    let mut second = Daemon::builder()
        .fake("android")
        .socket_dir(dir.path())
        .spawn();
    let status = second
        .wait_exit(Duration::from_secs(10))
        .expect("the second daemon leaves");
    assert!(status.success(), "{status}: {}", second.stderr());
    assert!(first.is_running());
    let reply = first.request(json!({"action": "status"}));
    assert_eq!(reply["data"]["daemon"]["pid"], first.pid(), "{reply}");
}

#[test]
fn files_left_by_a_killed_daemon_do_not_block_a_new_one() {
    let dir = TempDir::new();
    let first = Daemon::builder()
        .fake("android")
        .socket_dir(dir.path())
        .start();
    let killed = first.pid();
    drop(first);
    assert!(
        dir.path().join(SOCKET).exists(),
        "a killed daemon leaves its socket file"
    );

    let second = Daemon::builder()
        .fake("android")
        .socket_dir(dir.path())
        .start();
    let reply = second.request(json!({"action": "status"}));
    assert_eq!(reply["data"]["daemon"]["pid"], second.pid(), "{reply}");
    assert_ne!(second.pid(), killed);
}

#[test]
fn shutdown_stops_the_daemon_and_removes_its_files() {
    let mut daemon = Daemon::start("android");
    let reply = daemon.request(json!({"action": "shutdown"}));
    assert_eq!(reply["success"], true, "{reply}");
    let status = daemon
        .wait_exit(Duration::from_secs(5))
        .expect("the daemon exits after shutdown");
    assert!(status.success(), "{status}");
    for file in [SOCKET, PID, VERSION] {
        assert!(!daemon.dir.join(file).exists(), "{file} is left behind");
    }
    // Task 8.14: nothing else was in the directory, so the lock file and
    // the directory go too.
    assert!(
        !daemon.dir.exists(),
        "the directory is left behind with {:?}",
        std::fs::read_dir(&daemon.dir)
            .map(|entries| entries.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
            .unwrap_or_default()
    );
}

/// Task 8.14: stopping does not need the socket's path. A daemon whose
/// socket file was removed, as a test's directory is when the test ends,
/// stops on shutdown, since its accept loop wakes on a pipe rather than on
/// a connection to that path, and leaves no directory holding its lock
/// behind.
#[test]
fn shutdown_completes_after_the_socket_file_was_removed_and_leaves_no_lock_directory() {
    let dir = TempDir::new();
    let run = dir.path().join("run");
    let mut daemon = Daemon::builder().fake("android").socket_dir(&run).start();
    let mut stream = UnixStream::connect(daemon.socket()).expect("the daemon serves");
    std::fs::remove_file(daemon.socket()).expect("the socket file can be removed");
    stream
        .write_all(b"{\"action\":\"shutdown\"}\n")
        .expect("the connection still carries requests");
    let mut reply = String::new();
    BufReader::new(&stream)
        .read_line(&mut reply)
        .expect("a reply arrives");
    assert!(reply.contains("\"stopped\":true"), "{reply}");
    let status = daemon
        .wait_exit(Duration::from_secs(5))
        .expect("the daemon stops without its socket file");
    assert!(status.success(), "{status}");
    let left: Vec<_> = std::fs::read_dir(&run)
        .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        !run.exists(),
        "the daemon's directory is left behind with {left:?}"
    );
}

/// Starts the daemon without `--socket-dir`, with `env` set and
/// `MOBILE_AGENT_SOCKET_DIR`, `XDG_RUNTIME_DIR` and `HOME` cleared first, and
/// waits for its socket at `expected`.
fn serves_at(env: &[(&str, &Path)], expected: &Path) {
    let mut command = Command::new(support::BIN);
    support::clean_env(&mut command);
    command
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("HOME")
        .env("MOBILE_AGENT_FAKE_TARGETS", "android")
        .env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "60000")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("the daemon starts");
    let mut up = false;
    for _ in 0..500 {
        if std::os::unix::net::UnixStream::connect(expected).is_ok() {
            up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if up {
        let reply = support::request_at(expected, &json!({"action": "shutdown"}));
        assert_eq!(reply["success"], true, "{reply}");
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(up, "no daemon socket at {}", expected.display());
}

#[test]
fn the_socket_lives_under_the_runtime_directory() {
    let runtime = TempDir::new();
    let home = TempDir::new();
    let explicit = TempDir::new();
    serves_at(
        &[("XDG_RUNTIME_DIR", runtime.path()), ("HOME", home.path())],
        &runtime.path().join("mobile-agent").join(SOCKET),
    );
    serves_at(
        &[("HOME", home.path())],
        &home.path().join(".mobile-agent").join(SOCKET),
    );
    serves_at(
        &[
            ("MOBILE_AGENT_SOCKET_DIR", explicit.path()),
            ("XDG_RUNTIME_DIR", runtime.path()),
        ],
        &explicit.path().join(SOCKET),
    );
}

#[test]
fn an_overlong_socket_path_is_refused_with_a_hint() {
    let dir = TempDir::new();
    let long = dir.path().join("x".repeat(110));
    let mut daemon = Daemon::builder().fake("android").socket_dir(&long).spawn();
    let status = daemon
        .wait_exit(Duration::from_secs(5))
        .expect("the daemon refuses to start");
    assert!(!status.success());
    let stderr = daemon.stderr();
    assert!(stderr.contains("103"), "{stderr}");
    assert!(stderr.contains("MOBILE_AGENT_SOCKET_DIR"), "{stderr}");
}

#[test]
fn every_reply_follows_the_agent_browser_envelope() {
    let daemon = Daemon::start("android");
    let ok = daemon.request(json!({"id": "r1", "action": "status"}));
    assert_eq!(ok["id"], "r1");
    assert_eq!(ok["success"], true);
    assert!(ok["data"].is_object(), "{ok}");
    assert_eq!(ok["error"], json!(null));
    assert!(
        ok.get("code").is_none(),
        "code only appears on failure: {ok}"
    );

    let failed = daemon.request(json!({"id": "r2", "action": "no-such-action"}));
    assert_eq!(failed["id"], "r2");
    assert_eq!(failed["success"], false);
    assert_eq!(failed["data"], json!(null));
    assert_eq!(failed["code"], "unknown_action", "{failed}");
    assert!(
        failed["error"]
            .as_str()
            .unwrap_or("")
            .contains("no-such-action")
    );
}

#[test]
fn malformed_and_http_looking_requests_are_refused_with_a_code() {
    let daemon = Daemon::start("android");
    let garbage = support::raw_request(&daemon.socket(), "{not json");
    let garbage: serde_json::Value = serde_json::from_str(&garbage).expect("a JSON reply");
    assert_eq!(garbage["success"], false);
    assert_eq!(garbage["code"], "invalid_request", "{garbage}");

    let http = support::raw_request(&daemon.socket(), "GET / HTTP/1.1");
    let http: serde_json::Value = serde_json::from_str(&http).expect("a JSON reply");
    assert_eq!(http["code"], "invalid_request", "{http}");
    // The daemon keeps serving afterwards.
    let status = daemon.request(json!({"action": "status"}));
    assert_eq!(status["success"], true, "{status}");
}

#[test]
fn devices_lists_every_fake_target_with_its_platform_and_class() {
    let daemon = Daemon::start("android:emulator-5554,ios:FAKE-IOS-1,android:R58M123");
    let reply = daemon.request(json!({"action": "devices"}));
    assert_eq!(reply["success"], true, "{reply}");
    let devices = reply["data"]["devices"].as_array().expect("a device list");
    let summary: Vec<(String, String, String)> = devices
        .iter()
        .map(|d| {
            (
                d["id"].as_str().unwrap_or("").to_owned(),
                d["platform"].as_str().unwrap_or("").to_owned(),
                d["class"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                "emulator-5554".into(),
                "android".into(),
                "android-emulator".into()
            ),
            ("FAKE-IOS-1".into(), "ios".into(), "simulator".into()),
            ("R58M123".into(), "android".into(), "android-device".into()),
        ]
    );
    assert!(
        devices
            .iter()
            .all(|d| d["name"].is_string() && d["claimedBy"].is_null())
    );
}

#[test]
fn a_snapshot_of_the_fake_launcher_lists_a_heading_and_two_buttons() {
    let daemon = Daemon::start("android");
    let reply = daemon.request(json!({"action": "snapshot", "interactive": true}));
    assert_eq!(reply["success"], true, "{reply}");
    let text = reply["data"]["snapshot"].as_str().expect("a listing");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "- heading \"Home\" [id=launcher-title, ref=e1]",
            "- button \"Fixture\" [id=launcher-fixture, ref=e2]",
            "- button \"Settings\" [id=launcher-settings, ref=e3]",
        ],
        "{text}"
    );
    assert_eq!(
        reply["data"]["refs"]["e2"],
        json!({"name": "Fixture", "role": "button"})
    );
    assert_eq!(reply["data"]["removedRefs"], json!([]));
    assert_eq!(reply["data"]["verdict"]["degraded"], false, "{reply}");
}

#[test]
fn open_launches_an_installed_app_and_refuses_an_unknown_one() {
    let daemon = Daemon::start("ios");
    let opened = daemon.request(json!({"action": "open", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");
    assert_eq!(opened["data"]["opened"], APP);
    let snapshot = daemon.request(json!({"action": "snapshot", "interactive": true}));
    let text = snapshot["data"]["snapshot"].as_str().unwrap_or("");
    assert!(text.contains("id=login-submit"), "{text}");
    assert_eq!(snapshot["data"]["origin"], APP, "{snapshot}");

    let missing = daemon.request(json!({"action": "open", "app": "com.example.missing"}));
    assert_eq!(missing["success"], false);
    assert_eq!(missing["code"], "app_not_found", "{missing}");
    assert!(
        missing["error"].as_str().unwrap_or("").contains(APP),
        "the error lists what is installed: {missing}"
    );
}
