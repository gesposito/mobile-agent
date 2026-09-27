//! Every command of the grammar end to end on fake targets (spec
//! `agent-cli`: Command grammar, Daemon lifecycle's status).

mod support;

use serde_json::Value;
use support::{APP, TestEnv, ref_of};

#[test]
fn devices_lists_the_connected_targets() {
    let env = TestEnv::start("android,ios");
    let human = env.cli(&["devices"]).ok().stdout;
    let lines: Vec<&str> = human.lines().collect();
    assert_eq!(lines.len(), 2, "{human}");
    assert!(
        lines[0].contains("emulator-5554") && lines[0].contains("android"),
        "{human}"
    );
    assert!(
        lines[1].contains("FAKE-IOS-1") && lines[1].contains("ios"),
        "{human}"
    );
    let reply = env.cli(&["--json", "devices"]).ok().json();
    let devices = reply["data"]["devices"].as_array().expect("devices");
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[1]["class"], "simulator");
}

#[test]
fn open_launches_an_app_and_close_stops_it_and_releases_the_target() {
    let env = TestEnv::start("android");
    let opened = env.cli(&["open", APP]).ok();
    assert!(opened.stdout.contains(APP), "{}", opened.stdout);
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(listing.contains("id=login-submit"), "{listing}");
    let status = env.cli(&["--json", "status"]).ok().json();
    assert_eq!(status["data"]["targets"][0]["app"], APP);
    assert_eq!(status["data"]["targets"][0]["claimedBy"], "default");

    let closed = env.cli(&["--json", "close"]).ok().json();
    assert_eq!(closed["data"]["closed"], true, "{closed}");
    assert_eq!(closed["data"]["app"], APP);
    let status = env.cli(&["--json", "status"]).ok().json();
    assert_eq!(
        status["data"]["targets"][0]["claimedBy"],
        Value::Null,
        "{status}"
    );
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(
        listing.contains("- heading \"Home\""),
        "back on the launcher: {listing}"
    );
}

#[test]
fn click_by_coordinates_taps_the_point_in_device_points() {
    let env = TestEnv::start("android");
    // The launcher's Fixture button spans [16, 96, 392, 144] in points.
    let reply = env.cli(&["--json", "click", "204", "120"]).ok().json();
    assert_eq!(reply["data"]["x"], 204.0, "{reply}");
    assert_eq!(reply["data"]["y"], 120.0);
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(listing.contains("id=login-submit"), "{listing}");
}

#[test]
fn tap_is_an_alias_of_click() {
    let env = TestEnv::start("ios");
    env.cli(&["snapshot", "-i"]).ok();
    let reply = env.cli(&["--json", "tap", "@e3"]).ok().json();
    assert_eq!(reply["data"]["clicked"], "@e3", "{reply}");
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(
        listing.contains("- switch \"Wi-Fi\" [checked=true"),
        "{listing}"
    );
}

#[test]
fn a_click_reports_where_it_landed_and_its_dispatch_time() {
    let env = TestEnv::start("android");
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    let human = env
        .cli(&["click", &ref_of(&listing, "launcher-settings")])
        .ok()
        .stdout;
    assert!(human.starts_with("✓ Done (dispatch "), "{human}");
    env.cli(&["press", "Back"]).ok();
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    let settings = ref_of(&listing, "launcher-settings");
    let reply = env.cli(&["--json", "click", &settings]).ok().json();
    let data = &reply["data"];
    assert_eq!(data["ref"], settings.as_str(), "{reply}");
    assert!(
        data["dispatchMs"].as_f64().is_some_and(|ms| ms >= 0.0),
        "{reply}"
    );
    assert_eq!(data["accepted"], true);
}

#[test]
fn fill_replaces_a_field_and_type_appends_to_it() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    let username = ref_of(&listing, "login-username");
    env.cli(&["fill", &username, "agent"]).ok();
    env.cli(&["type", &username, "007"]).ok();
    assert_eq!(
        env.cli(&["get", "text", &username]).ok().stdout,
        "agent007\n"
    );
    env.cli(&["fill", &username, "bond"]).ok();
    assert_eq!(env.cli(&["get", "text", &username]).ok().stdout, "bond\n");
    env.cli(&["type", "!"]).ok();
    let reply = env.cli(&["--json", "get", "value", &username]).ok().json();
    assert_eq!(
        reply["data"]["value"], "bond!",
        "type without a target goes to the focused field"
    );
}

#[test]
fn press_enter_submits_the_login_form() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["fill", "#login-username", "agent"]).ok();
    env.cli(&["fill", "#login-password", "hunter2"]).ok();
    let reply = env.cli(&["--json", "press", "Enter"]).ok().json();
    assert_eq!(reply["data"]["pressed"], "Enter", "{reply}");
    assert_eq!(
        env.cli(&["get", "text", "#login-status"]).ok().stdout,
        "Welcome, agent\n"
    );
}

#[test]
fn scroll_and_swipe_move_the_list() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["click", "#tab-list"]).ok();
    let top = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(top.contains("id=row-1,"), "{top}");
    assert!(
        top.contains("more actionable items below, scroll down to reveal)"),
        "{top}"
    );

    env.cli(&["scroll", "down", "480"]).ok();
    let scrolled = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(!scrolled.contains("id=row-1,"), "{scrolled}");
    assert!(scrolled.contains("id=row-11,"), "{scrolled}");
    assert!(
        scrolled.contains("more actionable items above, scroll up to reveal)"),
        "{scrolled}"
    );

    let reply = env.cli(&["--json", "swipe", "down", "480"]).ok().json();
    assert_eq!(reply["data"]["swiped"], "down", "{reply}");
    let back = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(back.contains("id=row-1,"), "{back}");
}

#[test]
fn screenshot_saves_the_latest_frame() {
    let env = TestEnv::start("android");
    let run = env.cli(&["screenshot", "shot.jpg"]).ok();
    let saved = env.path().join("shot.jpg");
    assert!(run.stdout.contains("Screenshot saved to"), "{}", run.stdout);
    assert!(
        run.stdout.contains(&*saved.to_string_lossy()),
        "an absolute path: {}",
        run.stdout
    );
    assert!(std::fs::metadata(&saved).expect("the file exists").len() > 0);

    let reply = env.cli(&["--json", "screenshot"]).ok().json();
    let data = &reply["data"];
    let path = data["path"].as_str().expect("a path");
    assert!(std::path::Path::new(path).exists(), "{reply}");
    let _ = std::fs::remove_file(path);
    // 1071 x 2394 pixels fit to a 1000-pixel long edge.
    assert_eq!(data["width"], 447, "{reply}");
    assert_eq!(data["height"], 1000);
    assert_eq!(data["quality"], 80);
}

#[test]
fn wait_returns_when_an_element_or_text_appears_and_when_the_screen_settles() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["wait", "#login-status"]).ok();
    env.cli(&["fill", "#login-username", "agent"]).ok();
    env.cli(&["fill", "#login-password", "hunter2"]).ok();
    env.cli(&["click", "#login-submit"]).ok();
    let reply = env
        .cli(&["--json", "wait", "--text", "Welcome, agent"])
        .ok()
        .json();
    assert_eq!(reply["data"]["waited"], "text", "{reply}");
    assert_eq!(reply["data"]["element"]["id"], "login-status");

    let settled = env.cli(&["--json", "wait", "--settle"]).ok().json();
    assert_eq!(settled["data"]["settled"], true, "{settled}");
    let slept = env.cli(&["--json", "wait", "150"]).ok();
    assert_eq!(slept.json()["data"]["ms"], 150);
    assert!(slept.elapsed.as_millis() >= 150);

    env.cli(&["click", "#tab-list"]).ok();
    let gone = env
        .cli(&["--json", "wait", "#login-status", "--state", "hidden"])
        .ok()
        .json();
    assert_eq!(gone["data"]["waited"], "selector", "{gone}");
}

#[test]
fn an_endless_animation_does_not_settle_and_says_what_keeps_changing() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["click", "#tab-anim"]).ok();
    let reply = env.cli(&["--json", "wait", "--settle"]).ok().json();
    assert_eq!(reply["data"]["settled"], false, "{reply}");
    assert_eq!(reply["data"]["stillChanging"], serde_json::json!(["frame"]));
    assert!(
        reply["warning"]
            .as_str()
            .unwrap_or("")
            .contains("frame changes"),
        "{reply}"
    );
}

#[test]
fn get_reads_an_element_s_text_value_and_box() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    assert_eq!(
        env.cli(&["get", "text", "#login-status"]).ok().stdout,
        "Signed out\n"
    );
    let reply = env
        .cli(&["--json", "get", "box", "#login-submit"])
        .ok()
        .json();
    let data = &reply["data"];
    assert_eq!(
        (
            data["x"].as_f64(),
            data["y"].as_f64(),
            data["width"].as_f64(),
            data["height"].as_f64()
        ),
        (Some(16.0), Some(232.0), Some(376.0), Some(48.0)),
        "{reply}"
    );
    let human = env.cli(&["get", "box", "#login-submit"]).ok().stdout;
    assert_eq!(human, "x:      16\ny:      232\nwidth:  376\nheight: 48\n");
}

#[test]
fn diff_snapshot_lists_what_changed_since_the_session_s_last_snapshot() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["snapshot", "-i"]).ok();
    env.cli(&["click", "#tab-list"]).ok();
    let reply = env.cli(&["--json", "diff", "snapshot"]).ok().json();
    let data = &reply["data"];
    assert_eq!(data["changed"], true, "{reply}");
    assert!(data["additions"].as_u64().unwrap_or(0) > 0);
    assert!(data["removals"].as_u64().unwrap_or(0) >= 4);
    let removed: Vec<&str> = data["removed"]
        .as_array()
        .expect("removed elements")
        .iter()
        .filter_map(|e| e["id"].as_str())
        .collect();
    assert!(removed.contains(&"login-submit"), "{reply}");
    let diff = data["diff"].as_str().unwrap_or_default();
    assert!(
        diff.contains("removed:") && diff.contains("added:"),
        "{diff}"
    );

    let unchanged = env.cli(&["diff", "snapshot"]).ok().stdout;
    assert_eq!(unchanged, "✓ No changes detected\n");
}

#[test]
fn status_reports_the_daemon_and_its_targets() {
    let env = TestEnv::start("android,ios");
    let reply = env.cli(&["--json", "status"]).ok().json();
    let daemon = &reply["data"]["daemon"];
    assert_eq!(
        daemon["pid"].as_u64(),
        env.daemon_pid().map(u64::from),
        "{reply}"
    );
    assert!(
        daemon["version"]
            .as_str()
            .unwrap_or("")
            .starts_with(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(daemon["idleTimeoutMs"], 60_000);
    let ids: Vec<&str> = reply["data"]["targets"]
        .as_array()
        .expect("targets")
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert_eq!(ids, ["emulator-5554", "FAKE-IOS-1"]);
    let human = env.cli(&["status"]).ok().stdout;
    assert!(human.starts_with("daemon: running"), "{human}");
    assert!(
        human.contains("emulator-5554") && human.contains("FAKE-IOS-1"),
        "{human}"
    );
}

#[test]
fn selectors_address_elements_by_identifier_and_text() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["click", "text=Sign in"]).ok();
    assert_eq!(
        env.cli(&["get", "text", "id=login-status"]).ok().stdout,
        "Invalid credentials\n"
    );
    let missing = env
        .cli(&["--json", "click", "#nothing-here", "--timeout", "300"])
        .failed()
        .json();
    assert_eq!(missing["code"], "element_not_found", "{missing}");
}

#[test]
fn shutdown_stops_the_daemon() {
    let env = TestEnv::start("android");
    let run = env.cli(&["shutdown"]).ok();
    assert_eq!(run.stdout, "✓ Daemon stopped\n");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while env.socket().exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the socket is still there"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
