//! Sessions and target claims (spec `agent-cli`: Sessions).

mod support;

use serde_json::json;
use support::{APP, Daemon};

#[test]
fn a_claimed_target_refuses_another_session_naming_the_holder() {
    let daemon = Daemon::start("android:emulator-5554");
    let opened = daemon.request(json!({"action": "open", "session": "alice", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");

    let refused = daemon.request(json!({"action": "snapshot", "session": "bob"}));
    assert_eq!(refused["success"], false, "{refused}");
    assert_eq!(refused["code"], "target_claimed");
    let message = refused["error"].as_str().expect("an error message");
    assert!(message.contains("\"alice\""), "{message}");
    assert!(message.contains("emulator-5554"), "{message}");
    assert!(message.contains("--force"), "{message}");
    assert_eq!(refused["data"]["holder"], "alice");
    assert_eq!(refused["data"]["target"], "emulator-5554");
}

#[test]
fn force_takes_the_target_over_and_the_previous_holder_is_refused() {
    let daemon = Daemon::start("android:emulator-5554");
    let opened = daemon.request(json!({"action": "open", "session": "alice", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");

    let forced = daemon.request(json!({"action": "snapshot", "session": "bob", "force": true}));
    assert_eq!(forced["success"], true, "{forced}");
    assert_eq!(forced["data"]["target"], "emulator-5554");

    let refused = daemon.request(json!({"action": "snapshot", "session": "alice"}));
    assert_eq!(refused["code"], "target_claimed", "{refused}");
    assert_eq!(refused["data"]["holder"], "bob");
}

#[test]
fn the_holder_keeps_using_its_target() {
    let daemon = Daemon::start("android:emulator-5554");
    for action in ["open", "snapshot", "snapshot"] {
        let reply = daemon.request(json!({"action": action, "session": "alice", "app": APP}));
        assert_eq!(reply["success"], true, "{action}: {reply}");
    }
}

#[test]
fn every_target_command_claims_an_unclaimed_target() {
    let daemon = Daemon::start("android:emulator-5554");
    let first = daemon.request(json!({"action": "snapshot", "session": "bob"}));
    assert_eq!(first["success"], true, "{first}");
    let refused = daemon.request(json!({"action": "open", "session": "alice", "app": APP}));
    assert_eq!(refused["code"], "target_claimed", "{refused}");
    assert_eq!(refused["data"]["holder"], "bob");
}

#[test]
fn a_released_target_serves_another_session_without_force() {
    let daemon = Daemon::start("android:emulator-5554");
    let opened = daemon.request(json!({"action": "open", "session": "alice", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");
    let closed = daemon.request(json!({"action": "close", "session": "alice"}));
    assert_eq!(closed["success"], true, "{closed}");
    assert_eq!(closed["data"]["app"], APP);

    let snapshot = daemon.request(json!({"action": "snapshot", "session": "bob"}));
    assert_eq!(snapshot["success"], true, "{snapshot}");
}

#[test]
fn status_names_the_holder_of_each_target() {
    let daemon = Daemon::start("android:emulator-5554");
    let before = daemon.request(json!({"action": "status"}));
    assert_eq!(before["success"], true, "{before}");
    assert_eq!(before["data"]["targets"][0]["id"], "emulator-5554");
    assert_eq!(before["data"]["targets"][0]["claimedBy"], json!(null));

    daemon.request(json!({"action": "open", "session": "alice", "app": APP}));
    let after = daemon.request(json!({"action": "status"}));
    assert_eq!(after["data"]["targets"][0]["claimedBy"], "alice", "{after}");
    assert_eq!(after["data"]["targets"][0]["app"], APP, "{after}");
    assert_eq!(after["data"]["sessions"][0]["name"], "alice", "{after}");
}

#[test]
fn a_request_without_a_session_uses_the_default_session() {
    let daemon = Daemon::start("android:emulator-5554");
    let snapshot = daemon.request(json!({"action": "snapshot"}));
    assert_eq!(snapshot["success"], true, "{snapshot}");
    let refused = daemon.request(json!({"action": "snapshot", "session": "bob"}));
    assert_eq!(refused["data"]["holder"], "default", "{refused}");
}
