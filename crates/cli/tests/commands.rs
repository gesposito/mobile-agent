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

/// Task 10.23: `fill` reads the field back. Keys a field took out of order,
/// as Bluesky's search field took "nobel" as "noelb", are replaced by the
/// text in one piece, and a warning names what the keys left.
#[test]
fn fill_reads_the_field_back_and_pastes_text_whose_keys_went_in_out_of_order() {
    let env = TestEnv::start_with("ios", &[("MOBILE_AGENT_FAKE_TYPING", "reorder")]);
    env.cli(&["open", APP]).ok();
    let reply = env
        .cli(&["--json", "fill", "#login-username", "nobel"])
        .ok()
        .json();
    let data = &reply["data"];
    assert_eq!(data["method"], "direct", "{reply}");
    assert_eq!(data["retried"], true, "{reply}");
    assert_eq!(data["verified"], "text", "{reply}");
    assert!(
        data["readBackMs"].as_f64().is_some_and(|ms| ms >= 0.0),
        "{reply}"
    );
    let warning = reply["warning"].as_str().unwrap_or_default();
    assert!(
        warning.contains("\"noelb\"") && warning.contains("pasted"),
        "{reply}"
    );
    assert_eq!(
        env.cli(&["get", "text", "#login-username"]).ok().stdout,
        "nobel\n"
    );
    let typed = env.cli(&["--json", "fill", "#login-username", "nob"]).ok();
    let reply = typed.json();
    assert_eq!(
        reply["data"]["method"], "keys",
        "three keys stay in order: {reply}"
    );
    assert_eq!(reply["data"]["retried"], false, "{reply}");
    assert!(reply.get("warning").is_none(), "{reply}");
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

/// Rn-app-fixes D51 (task 5.4): `get` reads an element the tree holds out
/// of view, such as a row the list has scrolled above, and says so: the
/// reply's `inView` is false, and the human output adds a line saying it is
/// out of view, which an element in view leaves out.
#[test]
fn get_reads_an_element_out_of_view_and_says_so() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["click", "#tab-list"]).ok();
    env.cli(&["scroll", "down", "600", "--until", "#row-150"])
        .ok();
    let reply = env.cli(&["--json", "get", "box", "#row-3"]).ok().json();
    assert_eq!(reply["data"]["inView"], false, "{reply}");
    let human = env.cli(&["get", "box", "#row-3"]).ok().stdout;
    assert!(
        human.ends_with("\nout of view: scroll it into view before acting on it\n"),
        "{human}"
    );
    let shown = env.cli(&["get", "box", "#row-150"]).ok().stdout;
    assert_eq!(shown.lines().count(), 4, "{shown}");
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

/// An exact-text selector (`text==`) matches an element whose whole name
/// or value is the text, where `text=` matches any that contains it.
#[test]
fn an_exact_text_selector_matches_the_whole_name_or_value_only() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["click", "#tab-list"]).ok();
    // Rows 11 to 26 on screen: "Row 1" is part of several names, the name
    // of none.
    env.cli(&["scroll", "down", "480"]).ok();
    let contains = env
        .cli(&["--json", "wait", "text=Row 1", "--timeout", "300"])
        .ok()
        .json();
    assert_eq!(contains["data"]["element"]["id"], "row-11", "{contains}");
    let exact = env
        .cli(&["--json", "wait", "text==Row 1", "--timeout", "300"])
        .failed()
        .json();
    assert_eq!(exact["code"], "wait_timeout", "{exact}");
    let row = env.cli(&["--json", "wait", "text==Row 12"]).ok().json();
    assert_eq!(row["data"]["element"]["id"], "row-12", "{row}");
    assert!(
        row["data"]["element"]["ref"].as_str().is_some(),
        "the element carries its ref: {row}"
    );
    let gone = env
        .cli(&["--json", "click", "text==Row", "--timeout", "300"])
        .failed()
        .json();
    assert_eq!(gone["code"], "element_not_found", "{gone}");

    env.cli(&["click", "text==Row 12"]).ok();
    assert_eq!(
        env.cli(&["get", "text", "#detail-back"]).ok().stdout,
        "Back\n"
    );
    env.cli(&["wait", "text==Detail 12"]).ok();
    env.cli(&["click", "text==Back"]).ok();
    env.cli(&["wait", "#row-12"]).ok();

    env.cli(&["click", "#tab-login"]).ok();
    env.cli(&["fill", "#login-username", "agent"]).ok();
    let value = env
        .cli(&["--json", "get", "box", "text==agent"])
        .ok()
        .json();
    assert!(
        value["data"]["width"].as_f64().is_some(),
        "a value matches: {value}"
    );
    env.cli(&["wait", "text==ag", "--state", "hidden", "--timeout", "300"])
        .ok();
}

/// `scroll --until` scrolls until the element is on screen, and gives up
/// once the content stops moving.
#[test]
fn scroll_until_moves_the_content_until_the_element_is_on_screen() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["click", "#tab-list"]).ok();
    let reply = env
        .cli(&["--json", "scroll", "down", "600", "--until", "#row-150"])
        .ok()
        .json();
    let data = &reply["data"];
    assert_eq!(data["until"], "#row-150", "{reply}");
    assert_eq!(data["element"]["id"], "row-150", "{reply}");
    assert!(
        data["strokes"].as_u64().is_some_and(|n| n > 1),
        "several strokes: {reply}"
    );
    env.cli(&["wait", "#row-150", "--timeout", "0"]).ok();
    let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
    assert!(listing.contains("id=row-150,"), "{listing}");

    // Already on screen: no stroke.
    let still = env
        .cli(&["--json", "scroll", "down", "--until", "#row-150"])
        .ok()
        .json();
    assert_eq!(still["data"]["strokes"], 0, "{still}");

    let started = std::time::Instant::now();
    let end = env
        .cli(&[
            "--json",
            "scroll",
            "down",
            "600",
            "--until",
            "#row-999",
            "--timeout",
            "20000",
        ])
        .failed()
        .json();
    assert_eq!(end["code"], "element_not_found", "{end}");
    let message = end["error"].as_str().unwrap_or_default();
    assert!(message.contains("stopped moving"), "{message}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "it gives up at the end of the list, not at the timeout"
    );
    let human = env.cli(&["scroll", "up", "--until", "#row-3"]).ok();
    assert!(human.stdout.starts_with("✓ "), "{}", human.stdout);
}

/// The bounds of the element a reply's data carries.
fn bounds_of(data: &Value) -> [f64; 4] {
    let bounds = data["element"]["bounds"]
        .as_array()
        .unwrap_or_else(|| panic!("an element with bounds: {data}"));
    let at = |i: usize| bounds[i].as_f64().expect("a coordinate");
    [at(0), at(1), at(2), at(3)]
}

/// Task 10.15: `scroll --until` stops only once the element is mostly in
/// view, not as soon as a sliver of it shows: in a frame-path run of the
/// harness's list-scroll flow it stopped with `row-200` 1.2 points tall at
/// the list's edge. On the fakes' lists, rows 48 points tall between 24 and
/// 840 (Android, which reports a row the list cuts off as the part that
/// shows) or 62 and 791 (iOS, whole frames), one stroke of 98 or 185 points
/// leaves row 20 two points into the list; one more, shorter stroke brings
/// it in, and the click on it opens its detail.
#[test]
fn scroll_until_stops_only_once_the_element_is_mostly_in_view() {
    for (fake, amount, (top, bottom)) in [
        ("android", "98", (24.0, 840.0)),
        ("ios", "185", (62.0, 791.0)),
    ] {
        let env = TestEnv::start(fake);
        env.cli(&["open", APP]).ok();
        env.cli(&["click", "#tab-list"]).ok();
        let reply = env
            .cli(&["--json", "scroll", "down", amount, "--until", "#row-20"])
            .ok()
            .json();
        let data = &reply["data"];
        let [_, row_top, _, row_bottom] = bounds_of(data);
        assert!(
            row_top >= top && row_bottom <= bottom && (row_bottom - row_top - 48.0).abs() < 0.5,
            "{fake}: the whole row inside the list: {reply}"
        );
        assert_eq!(
            data["strokes"], 2,
            "{fake}: a stroke of {amount} points, then one that brings the row in: {reply}"
        );
        assert_eq!(data["visible"], 1.0, "{fake}: all of it in view: {reply}");
        env.cli(&["click", "#row-20"]).ok();
        env.cli(&["wait", "text==Detail 20", "--timeout", "2000"])
            .ok();
    }
}

/// Task 10.15: an element on screen but not mostly in view, which no stroke
/// along the scroll's axis brings in, fails the scroll with
/// `element_not_found` rather than stopping there.
#[test]
fn scroll_until_fails_when_its_axis_cannot_bring_a_partly_shown_element_in() {
    for (fake, amount) in [("android", "98"), ("ios", "185")] {
        let env = TestEnv::start(fake);
        env.cli(&["open", APP]).ok();
        env.cli(&["click", "#tab-list"]).ok();
        env.cli(&["scroll", "down", amount]).ok();
        env.cli(&["wait", "#row-20", "--timeout", "0"]).ok();
        let failed = env
            .cli(&["--json", "scroll", "left", "--until", "#row-20"])
            .failed()
            .json();
        assert_eq!(failed["code"], "element_not_found", "{fake}: {failed}");
        let message = failed["error"].as_str().unwrap_or_default();
        assert!(
            message.contains("not mostly in view")
                && message.contains("scrolling left or right does not bring more of it in"),
            "{fake}: {message}"
        );
        assert_eq!(failed["data"]["strokes"], 0, "{fake}: {failed}");
        // Android reports the row's bounds cut to the two points that show,
        // all of them in view; iOS reports its whole frame.
        let visible = if fake == "android" { 1.0 } else { 0.04 };
        assert_eq!(failed["data"]["visible"], visible, "{fake}: {failed}");
    }
}

/// Task 10.16: `scroll --until` looks for its element between strokes, so a
/// stroke that carries the content farther than the list shows can carry a
/// row through the list unseen: on the emulator a 600-point stroke over
/// 300 ms whose finger lifted moving carried the fixtures' lists 1,103 to
/// 1,319 points, where they show about 780, and on the Simulator the Expo
/// list 1,522 to 1,608, where it shows about 730. With `MOBILE_AGENT_FAKE_FLING` the fakes' lists glide on as
/// their platforms' do, 1,247 points on Android and 1,598 on iOS for that
/// stroke, so rows 18 to 25 of the Android list and 17 to 33 of the iOS one
/// never show at a look. Each stroke that looks for the element comes to
/// rest before the finger lifts and carries the list at most three quarters
/// of what it shows, plus the tap threshold: 600 points of the Android
/// list's 816 (at most 620), 556.75 of the iOS list's 729. One stroke then
/// brings row 22 into view whole.
#[test]
fn scroll_until_finds_a_row_that_a_flinging_stroke_would_carry_through_the_list() {
    for (fake, step, top) in [("android", 600.0, 432.0), ("ios", 556.75, 513.25)] {
        let env = TestEnv::start_with(fake, &[("MOBILE_AGENT_FAKE_FLING", "1")]);
        env.cli(&["open", APP]).ok();
        env.cli(&["click", "#tab-list"]).ok();
        let reply = env
            .cli(&["--json", "scroll", "down", "600", "--until", "#row-22"])
            .ok()
            .json();
        let data = &reply["data"];
        assert_eq!(data["strokes"], 1, "{fake}: {reply}");
        assert_eq!(data["amount"], step, "{fake}: the stroke's length: {reply}");
        let [_, row_top, _, row_bottom] = bounds_of(data);
        assert!(
            (row_top - top).abs() < 0.5 && (row_bottom - row_top - 48.0).abs() < 0.5,
            "{fake}: row 22 {step} points up, whole: {reply}"
        );
        assert_eq!(data["visible"], 1.0, "{fake}: {reply}");
    }
}

/// Task 10.16: every stroke of `scroll --until` comes to rest before the
/// finger lifts, the one that brings a partly shown element in too, so the
/// content stops where the stroke aimed even on lists that fling. On the
/// fakes' lists with `MOBILE_AGENT_FAKE_FLING`, which move as far as the
/// finger, the tap threshold included: a stroke of 98 points (Android) or
/// 185 (iOS) leaves row 20 two points into the list, and a second stroke of
/// 98 points (the step; Android cuts the row's bounds to what shows, so the
/// stroke aims its middle at the list's) or 64 (the 46 points it sticks out,
/// 8 more and the threshold) brings it in, its top at 740 or 725. Strokes
/// that lifted moving would glide on 28 points each on Android, leaving the
/// row at 684, and 308 on iOS after the first, bringing the row in at 481.
#[test]
fn scroll_until_strokes_come_to_rest_so_a_row_stops_where_they_aim() {
    for (fake, amount, top) in [("android", "98", 740.0), ("ios", "185", 725.0)] {
        let env = TestEnv::start_with(fake, &[("MOBILE_AGENT_FAKE_FLING", "1")]);
        env.cli(&["open", APP]).ok();
        env.cli(&["click", "#tab-list"]).ok();
        let reply = env
            .cli(&["--json", "scroll", "down", amount, "--until", "#row-20"])
            .ok()
            .json();
        let data = &reply["data"];
        let [_, row_top, _, row_bottom] = bounds_of(data);
        assert!(
            data["strokes"] == 2
                && (row_top - top).abs() < 0.5
                && (row_bottom - row_top - 48.0).abs() < 0.5,
            "{fake}: two strokes, row 20 whole at {top}: {reply}"
        );
    }
}

/// Task 10.15 and design D37 of rn-app-fixes: a click on an element mostly
/// out of view taps the part of it in view, not the centre of its bounds,
/// which may lie on something else: on the iOS fake, row 20 twenty points
/// into the list has its centre under the tab bar, and the tap goes to the
/// centre of those twenty points; Android reports the row clipped to the
/// two points that show. The click warns, and says how to bring the element
/// in; a click on an element in view does not.
#[test]
fn a_click_on_an_element_mostly_out_of_view_warns() {
    for (fake, amount, shown) in [
        ("android", "98", 838.0..840.0),
        ("ios", "203", 771.0..791.0),
    ] {
        let env = TestEnv::start(fake);
        env.cli(&["open", APP]).ok();
        let in_view = env.cli(&["--json", "click", "#tab-list"]).ok().json();
        assert_eq!(in_view["warning"], Value::Null, "{fake}: {in_view}");
        env.cli(&["scroll", "down", amount]).ok();
        let reply = env.cli(&["--json", "click", "#row-20"]).ok().json();
        let warning = reply["warning"].as_str().unwrap_or_default();
        assert!(
            warning.contains("#row-20 was not mostly in view")
                && warning.contains("scroll --until #row-20"),
            "{fake}: {reply}"
        );
        let y = reply["data"]["y"].as_f64().unwrap_or_default();
        assert!(
            shown.contains(&y),
            "{fake}: on what shows of the row: {reply}"
        );
    }
}

/// Task 8.15: a React Native app shows a launch splash first (Bluesky's
/// listed only `image [id=SplashScreenLegacy]` for seconds), which is not
/// the app's screen: `open` waits past it for the app's first screen, so
/// the snapshot right after it lists the app's elements.
#[test]
fn open_waits_past_a_launch_splash_for_the_app_s_first_screen() {
    for fake in ["android", "ios"] {
        let env = TestEnv::start_with(fake, &[("MOBILE_AGENT_FAKE_SPLASH_MS", "800")]);
        let reply = env.cli(&["--json", "open", APP]).ok().json();
        assert_eq!(reply["data"]["settled"], true, "{fake}: {reply}");
        assert!(
            reply["data"]["firstTreeMs"]
                .as_f64()
                .is_some_and(|ms| ms >= 800.0),
            "{fake}: the app's first screen came after the 800 ms splash: {reply}"
        );
        let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
        assert!(
            listing.contains("id=login-submit"),
            "{fake}: the snapshot right after open shows the app:\n{listing}"
        );
    }
}

/// Task 8.13: an app takes a stroke no longer than its platform's tap
/// threshold for a tap (`scroll down 2` on the Simulator opened what lay
/// under the screen's center), so `scroll` refuses one with
/// `stroke_too_short` before it touches the screen: 8 points or less on
/// Android, whose touch slop is 8 dp, and 10 points or less on iOS.
#[test]
fn scroll_refuses_strokes_the_platform_takes_for_a_tap() {
    for (fake, slop) in [("android", 8), ("ios", 10)] {
        let env = TestEnv::start(fake);
        env.cli(&["open", APP]).ok();
        env.cli(&["click", "#tab-list"]).ok();
        for amount in ["2".to_owned(), slop.to_string()] {
            let refused = env
                .cli(&["--json", "scroll", "down", &amount])
                .failed()
                .json();
            assert_eq!(
                refused["code"], "stroke_too_short",
                "{fake}, {amount} points: {refused}"
            );
            let message = refused["error"].as_str().unwrap_or_default();
            assert!(
                message.contains(&format!("{slop} points")),
                "{fake}: the message names the threshold: {message}"
            );
        }
        let until = env
            .cli(&["--json", "scroll", "down", "5", "--until", "#row-150"])
            .failed()
            .json();
        assert_eq!(until["code"], "stroke_too_short", "{fake}: {until}");
        let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
        assert!(listing.contains("id=row-1,"), "{fake}:\n{listing}");
        let longer = (slop + 1).to_string();
        let moved = env.cli(&["--json", "scroll", "down", &longer]).ok().json();
        assert_eq!(moved["data"]["strokes"], 1, "{fake}: {moved}");
    }
}

/// The fake fixtures read the initial tab where the real ones do
/// (fixtures/CONTRACT.md, Launch options): an iOS fixture from the launch
/// environment variable `FIXTURE_INITIAL_TAB`, an Android one from the launch
/// intent's string extra `initial_tab`, which an `--env` entry becomes there.
/// Each ignores the other platform's option and opens on its Login tab.
#[test]
fn open_passes_the_initial_tab_the_way_each_platform_s_fixtures_read_it() {
    for (fake, read, ignored) in [
        ("android", "initial_tab=big", "FIXTURE_INITIAL_TAB=big"),
        ("ios", "FIXTURE_INITIAL_TAB=big", "initial_tab=big"),
    ] {
        let env = TestEnv::start(fake);
        env.cli(&["open", "--relaunch", "--env", read, APP]).ok();
        let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
        assert!(
            listing.contains("id=cell-1,") && !listing.contains("id=login-submit"),
            "{fake}: --env {read} opens the Big tab:\n{listing}"
        );
        env.cli(&["open", "--relaunch", "--env", ignored, APP]).ok();
        let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
        assert!(
            listing.contains("id=login-submit"),
            "{fake}: --env {ignored} leaves the Login tab:\n{listing}"
        );
    }
}

/// `open --relaunch` reports the screen settled only once the relaunched
/// app has shown its first tree, so a snapshot right after it sees the app.
#[test]
fn open_relaunch_reports_settled_only_once_the_relaunched_app_shows_its_screen() {
    for (fake, list) in [
        ("android", "initial_tab=list"),
        ("ios", "FIXTURE_INITIAL_TAB=list"),
    ] {
        let env = TestEnv::start_with(fake, &[("MOBILE_AGENT_FAKE_LATENCY_MS", "400")]);
        env.cli(&["open", APP]).ok();
        env.cli(&["wait", "#login-submit"]).ok();
        let reply = env
            .cli(&["--json", "open", "--relaunch", "--env", list, APP])
            .ok()
            .json();
        let data = &reply["data"];
        assert_eq!(data["settled"], true, "{fake}: {reply}");
        assert!(
            data["elapsedMs"].as_f64().is_some_and(|ms| ms >= 400.0),
            "{fake}: the relaunched app shows its screen 400 ms late: {reply}"
        );
        assert!(
            data["firstTreeMs"].as_f64().is_some_and(|ms| ms >= 400.0),
            "{fake}: {reply}"
        );
        let listing = env.cli(&["snapshot", "-i"]).ok().stdout;
        assert!(
            listing.contains("id=row-1,") && !listing.contains("id=login-submit"),
            "{fake}: the snapshot right after the open shows the relaunched app:\n{listing}"
        );
    }
}

/// An app that takes long to show its first tree, as the Compose fixture's
/// 500-cell screen does on an emulator, still has 3 s to settle once it
/// shows: the wait for its first tree has a budget of its own, the
/// session's default timeout.
#[test]
fn open_gives_a_slow_app_its_first_tree_before_the_settle_budget_starts() {
    let env = TestEnv::start_with("android", &[("MOBILE_AGENT_FAKE_LATENCY_MS", "2900")]);
    let reply = env
        .cli(&["--json", "open", "--env", "initial_tab=list", APP])
        .ok()
        .json();
    let data = &reply["data"];
    assert!(
        data["firstTreeMs"].as_f64().is_some_and(|ms| ms >= 2900.0),
        "{reply}"
    );
    assert_eq!(
        data["settled"], true,
        "settled after its first tree: {reply}"
    );
    let quick = env
        .cli(&[
            "--json",
            "open",
            "--relaunch",
            "--timeout",
            "1000",
            "--env",
            "initial_tab=big",
            APP,
        ])
        .ok()
        .json();
    assert_eq!(
        quick["data"]["settled"], false,
        "an explicit timeout bounds the whole wait: {quick}"
    );
    assert!(quick["warning"].as_str().is_some(), "{quick}");
}

/// Task 8.12: `open` and `close` note the launch and the stop to the mirror
/// as a change of the window in front, whatever the driver, so a settle
/// counts it even before the app draws. On targets that show every change
/// 120 ms late, the settle after an `open` of the app already in front
/// waits for it to draw again, and a settle right after `close` waits for
/// the launcher.
#[test]
fn open_and_close_are_window_changes_that_a_settle_waits_out() {
    for fake in ["android", "ios"] {
        let env = TestEnv::start_with(fake, &[("MOBILE_AGENT_FAKE_LATENCY_MS", "120")]);
        env.cli(&["open", APP]).ok();
        let again = env.cli(&["--json", "open", APP]).ok().json();
        assert_eq!(again["data"]["settled"], true, "{fake}: {again}");
        assert!(
            again["data"]["elapsedMs"]
                .as_f64()
                .is_some_and(|ms| ms >= 120.0),
            "{fake}: the app drew again 120 ms after the launch: {again}"
        );
        let quiet = env.cli(&["--json", "wait", "--settle"]).ok().json();
        assert_eq!(
            quiet["data"]["endedBy"], "none",
            "{fake}: open waited the app out: {quiet}"
        );

        let closed = env
            .cli(&["--json", "batch", "--bail", "close", "wait --settle"])
            .ok()
            .json();
        let settle = &closed[1]["result"];
        assert!(
            settle["elapsedMs"].as_f64().is_some_and(|ms| ms >= 120.0),
            "{fake}: the launcher showed 120 ms after the stop: {closed}"
        );
        assert_eq!(settle["settled"], true, "{fake}: {closed}");
        assert_ne!(settle["endedBy"], "none", "{fake}: {closed}");
    }
}

/// `status` names, for each attached target, whether it has a frame source:
/// emulators and Simulators do, Android devices do not (design D7).
#[test]
fn status_names_whether_each_attached_target_has_a_frame_source() {
    let env = TestEnv::start("android,android:R58M123,ios,ios");
    for device in ["emulator-5554", "R58M123", "FAKE-IOS-1"] {
        env.cli(&["--device", device, "snapshot"]).ok();
    }
    let reply = env.cli(&["--json", "status"]).ok().json();
    let frames = |id: &str| {
        reply["data"]["targets"]
            .as_array()
            .and_then(|targets| targets.iter().find(|t| t["id"] == id))
            .map(|t| t["frameSource"].clone())
            .unwrap_or_else(|| panic!("no {id} in {reply}"))
    };
    assert_eq!(frames("emulator-5554"), true, "{reply}");
    assert_eq!(frames("R58M123"), false, "{reply}");
    assert_eq!(frames("FAKE-IOS-1"), true, "{reply}");
    assert_eq!(frames("FAKE-IOS-2"), Value::Null, "not attached: {reply}");
    let human = env.cli(&["status"]).ok().stdout;
    let row = |id: &str| {
        human
            .lines()
            .find(|line| line.trim_start().starts_with(id))
            .unwrap_or_else(|| panic!("no {id} in {human}"))
            .to_owned()
    };
    assert!(row("emulator-5554").contains("attached, frames"), "{human}");
    assert!(row("R58M123").contains("attached, no frames"), "{human}");
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

/// `wait --change` (design D8 of the `mcp-server` change) prints what moved
/// and the delta as `snapshot --delta` prints it, and fails with
/// `wait_timeout` on a still screen as other waits do, and on frames that
/// move alone unless `--frames` asks for them.
#[test]
fn wait_change_prints_what_moved_and_the_delta_and_times_out_on_a_still_screen() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    env.cli(&["snapshot", "-i", "--delta"]).ok();
    env.cli(&["click", "#tab-list"]).ok();
    let changed = env.cli(&["wait", "--change"]).ok().stdout;
    let mut lines = changed.lines();
    let first = lines.next().unwrap_or_default();
    assert!(
        first.starts_with("✓ The tree and frames changed after ") && first.ends_with(" ms"),
        "{changed}"
    );
    assert!(changed.contains("id=row-1,"), "the delta: {changed}");

    // The tab's transition animates on after its tree changed: a still
    // screen is one the session's last snapshot took once it settled.
    env.cli(&["wait", "--settle"]).ok();
    env.cli(&["snapshot", "-i", "--delta"]).ok();
    let still = env
        .cli(&["--json", "wait", "--change", "--timeout", "300"])
        .failed()
        .json();
    assert_eq!(still["code"], "wait_timeout", "{still}");
    let human = env.cli(&["wait", "--change", "--timeout", "300"]).failed();
    assert!(
        human.stderr.starts_with("✗ waited 300 ms"),
        "{}",
        human.stderr
    );

    // The Anim tab's spinner moves the frames alone, which end a change
    // wait only with --frames.
    env.cli(&["click", "#tab-anim"]).ok();
    env.cli(&["snapshot", "-i", "--delta"]).ok();
    let running = env
        .cli(&["--json", "wait", "--change", "--timeout", "300"])
        .failed()
        .json();
    assert_eq!(running["code"], "wait_timeout", "{running}");
    let reply = env
        .cli(&["--json", "wait", "--change", "--frames"])
        .ok()
        .json();
    assert_eq!(reply["data"]["waited"], "change", "{reply}");
    assert_eq!(reply["data"]["endedBy"], "frame", "{reply}");
    let frames = env.cli(&["wait", "--change", "--frames"]).ok().stdout;
    assert!(
        frames.starts_with("✓ Frames changed after ") && frames.contains("\nunchanged (revision "),
        "{frames}"
    );
    for refused in [
        vec!["wait", "--change", "@e1"],
        vec!["wait", "--change", "--text", "Welcome"],
        vec!["wait", "--change", "--settle"],
    ] {
        let run = env.cli(&refused).failed();
        assert!(
            run.stderr.contains("cannot be used with"),
            "{refused:?}: {}",
            run.stderr
        );
    }
    // --frames needs --change, as --quiet and --tree-only need --settle.
    for needing in [
        vec!["wait", "--frames"],
        vec!["wait", "--frames", "--timeout", "300"],
    ] {
        let run = env.cli(&needing).failed();
        assert!(
            run.stderr.starts_with(
                "error: the following required arguments were not provided:\n  --change\n"
            ),
            "{needing:?}: {}",
            run.stderr
        );
    }
    let reply = env.cli(&["--json", "wait", "--frames"]).failed().json();
    assert_eq!(reply["code"], "invalid_argument", "{reply}");
}

/// `close --keep` (design D9) releases the target and leaves the app
/// running, so another session works on the app as it is.
#[test]
fn close_keep_releases_the_target_and_leaves_the_app_running() {
    let env = TestEnv::start("android");
    env.cli(&["--session", "alice", "open", APP]).ok();
    let kept = env.cli(&["--session", "alice", "close", "--keep"]).ok();
    assert_eq!(
        kept.stdout,
        format!("✓ Released emulator-5554; {APP} keeps running\n")
    );
    let listing = env.cli(&["--session", "bob", "snapshot", "-i"]).ok().stdout;
    assert!(listing.contains("id=login-submit,"), "{listing}");
    let reply = env
        .cli(&["--json", "--session", "bob", "close", "--keep"])
        .ok()
        .json();
    assert_eq!(reply["data"]["kept"], true, "{reply}");
    assert_eq!(reply["data"]["app"], Value::Null, "{reply}");
}

/// `cancel` (design D7) ends the session's command in flight, here a wait
/// another shell started, which fails with `cancelled`, and says when
/// nothing was in flight.
#[test]
fn cancel_ends_the_session_s_wait_that_another_shell_started() {
    let env = TestEnv::start("android");
    env.cli(&["open", APP]).ok();
    let waiting = env
        .command(&["wait", "--text", "Nobody shows this"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the wait starts");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let cancelled = env.cli(&["cancel"]).ok();
    assert_eq!(
        cancelled.stdout,
        "✓ Cancelled the wait in flight on emulator-5554\n"
    );
    let ended = waiting.wait_with_output().expect("the wait ends");
    assert_eq!(ended.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&ended.stderr);
    assert!(stderr.starts_with("✗ wait was cancelled"), "{stderr}");

    let idle = env.cli(&["cancel"]).ok();
    assert_eq!(idle.stdout, "✓ Nothing in flight to cancel\n");
    let reply = env.cli(&["--json", "cancel"]).ok().json();
    assert_eq!(
        reply["data"],
        serde_json::json!({"cancelled": false, "action": null, "target": null}),
        "{reply}"
    );
    env.cli(&["cancel", "now"]).failed();
}
