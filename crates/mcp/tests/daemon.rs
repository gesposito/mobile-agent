//! The daemon behind the server (design D3 of `profiling`): the server
//! checks the daemon's version on its first call and again only after a
//! call that finds no daemon listening, which starts one of the server's
//! build as the CLI's first command does, so the calls between go straight
//! to the socket.

mod support;

use std::time::{Duration, Instant};

use serde_json::json;
use support::{Raw, TestEnv, ok};

#[test]
fn the_server_checks_the_daemon_s_version_on_its_first_call_and_after_one_finds_no_daemon() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    ok(&client.call("status", json!({})));
    let first = env.daemon_pid().expect("a daemon serves");
    // Another build's version, which a check would replace with the
    // server's build.
    std::fs::write(
        env.path().join("daemon.version"),
        "0.0.1+placeholder-other\n",
    )
    .expect("the version file is writable");
    for _ in 0..3 {
        ok(&client.call("status", json!({})));
    }
    assert_eq!(
        env.daemon_pid(),
        Some(first),
        "the daemon the first call checked serves the calls after it"
    );

    // The daemon stops: the next call checks again and starts one.
    let stopped = env.request(json!({"action": "shutdown"}));
    assert_eq!(stopped["data"]["stopped"], true, "{stopped}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while env.socket().exists() {
        assert!(Instant::now() < deadline, "the daemon did not stop");
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = ok(&client.call("status", json!({})));
    assert!(status.contains("emulator-5554"), "{status}");
    let started = env.daemon_pid().expect("the call started a daemon");
    assert_ne!(started, first, "a daemon of its own started");
    let version = std::fs::read_to_string(env.path().join("daemon.version"))
        .expect("the daemon recorded its version");
    assert!(
        version.starts_with(&format!("{}+", env!("CARGO_PKG_VERSION"))),
        "the server's build: {version}"
    );
    client.close();
}
