//! Replies that name what the screen holds (design D5 of the
//! `real-app-fixes` change), so a model needs no snapshot or screenshot to
//! learn it: the refs a wait's timeout summary prints are the session's to
//! act on, `get value` gives a switch's state, `open` returns at once when
//! a system alert stands in front of the app, naming it, and a screenshot
//! names the screen's size in points beside its pixels, as a click on a
//! point off the screen does.

mod support;

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{APP, Daemon};

/// The ref of the line of `listing` whose identifier is `id`, as `@eN`.
#[track_caller]
fn ref_of(listing: &str, id: &str) -> String {
    let needle = format!("id={id},");
    let line = listing
        .lines()
        .find(|line| line.contains(&needle))
        .unwrap_or_else(|| panic!("no element {id} in:\n{listing}"));
    let start = line.find("ref=e").expect("the line has a ref") + 4;
    let digits: String = line[start + 1..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    format!("@e{digits}")
}

/// Design D4: on the demo pass a ref the screen summary of a
/// `wait_timeout` printed failed `unknown_ref`, since no snapshot of the
/// session had listed it. The summary's listing issues its refs to the
/// session, so a click on one lands; so does the summary an
/// `element_not_found` prints.
#[test]
fn a_ref_a_timeout_s_summary_prints_is_one_the_session_can_act_on() {
    let daemon = Daemon::start("ios");
    let opened = daemon.request(json!({"action": "open", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");
    let timed_out =
        daemon.request(json!({"action": "wait", "text": "Nobody shows this", "timeout": 300}));
    assert_eq!(timed_out["code"], "wait_timeout", "{timed_out}");
    let summary = timed_out["error"].as_str().unwrap_or_default();
    let list = ref_of(summary, "tab-list");
    let clicked = daemon.request(json!({"action": "click", "target": list}));
    assert_eq!(clicked["success"], true, "{clicked}");

    let missing =
        daemon.request(json!({"action": "click", "target": "#nobody-has-this", "timeout": 300}));
    assert_eq!(missing["code"], "element_not_found", "{missing}");
    let summary = missing["error"].as_str().unwrap_or_default();
    let login = ref_of(summary, "tab-login");
    let clicked = daemon.request(json!({"action": "click", "target": login}));
    assert_eq!(clicked["success"], true, "{clicked}");
}

/// Design D5: `get value` on a switch returned nothing on the demo pass. It
/// gives the switch's state as the listing has it, `checked`, beside the
/// value, which a switch does not have; an element without either has a
/// null value and no state.
#[test]
fn get_value_gives_a_switch_s_state_and_nothing_for_an_element_without_one() {
    for platform in ["ios", "android"] {
        let daemon = Daemon::start(platform);
        let settings = daemon.request(json!({"action": "click", "target": "#launcher-settings"}));
        assert_eq!(settings["success"], true, "{platform}: {settings}");
        let get = |target: &str| {
            daemon.request(json!({"action": "get", "what": "value", "target": target}))
        };
        let on = get("#settings-wifi");
        assert_eq!(on["data"]["checked"], true, "{platform}: {on}");
        assert_eq!(on["data"]["value"], Value::Null, "{platform}: {on}");
        let flipped = daemon.request(json!({"action": "click", "target": "#settings-wifi"}));
        assert_eq!(flipped["success"], true, "{platform}: {flipped}");
        let off = get("#settings-wifi");
        assert_eq!(off["data"]["checked"], false, "{platform}: {off}");
        let title = get("#settings-title");
        assert_eq!(title["success"], true, "{platform}: {title}");
        assert_eq!(title["data"]["value"], Value::Null, "{platform}: {title}");
        assert!(
            title["data"].get("checked").is_none(),
            "{platform}: {title}"
        );
    }
}

/// Design D5: on the demo pass `open com.apple.Maps` waited out the call's
/// bound behind the location alert in front of the app. `open` returns as
/// soon as a system alert stands in front of the app, the app opened and
/// the alert named, in the reply and its warning, as an action refused
/// under it names it; the next snapshot lists the alert. The fixture's
/// System tab brings its location prompt up, which stays in front while the
/// app is away.
#[test]
fn open_returns_at_once_when_a_system_alert_stands_in_front_of_the_app_naming_it() {
    for (platform, option, alert) in [
        (
            "ios",
            "FIXTURE_INITIAL_TAB=system",
            "alert \"Allow “FixtureSwiftUI” to use your location?\"",
        ),
        ("android", "initial_tab=system", "dialog \"Permission\""),
    ] {
        let daemon = Daemon::start(platform);
        let opened = daemon.request(json!({"action": "open", "app": APP, "env": [option]}));
        assert_eq!(opened["success"], true, "{platform}: {opened}");
        let asked = daemon.request(json!({"action": "click", "target": "#system-permission"}));
        assert_eq!(asked["success"], true, "{platform}: {asked}");
        let home = daemon.request(json!({"action": "press", "key": "Home"}));
        assert_eq!(home["success"], true, "{platform}: {home}");

        let started = Instant::now();
        let again = daemon.request(json!({"action": "open", "app": APP}));
        let took = started.elapsed();
        assert_eq!(again["success"], true, "{platform}: {again}");
        assert!(
            took < Duration::from_secs(3),
            "{platform}: {took:?}: {again}"
        );
        assert_eq!(again["data"]["opened"], APP, "{platform}: {again}");
        assert_eq!(again["data"]["alert"], alert, "{platform}: {again}");
        let warning = again["warning"].as_str().unwrap_or_default();
        assert!(warning.contains(alert), "{platform}: {warning}");
        let listing = daemon.request(json!({"action": "snapshot", "interactive": true}));
        let text = listing["data"]["snapshot"].as_str().unwrap_or_default();
        assert!(text.contains("id=permission-allow"), "{platform}: {text}");
    }
}

/// Design D5: on the demo pass a model took a screenshot's pixels, 460x1000,
/// for the 402x874 points a click takes. The screenshot reply names the
/// screen's size in points beside the image's pixels, and a click on a point
/// off the screen fails naming the screen's points and saying that a
/// screenshot's pixels are scaled; a point on the screen still taps.
#[test]
fn a_screenshot_and_a_click_off_the_screen_name_the_screen_s_points() {
    for (platform, pixels, points) in [
        ("ios", (460, 1000), (402.0, 874.0)),
        ("android", (447, 1000), (408.0, 912.0)),
    ] {
        let daemon = Daemon::start(platform);
        let shot = daemon.request(json!({"action": "screenshot", "inline": true}));
        assert_eq!(shot["success"], true, "{platform}: {shot}");
        let data = &shot["data"];
        assert_eq!(
            (data["width"].as_u64(), data["height"].as_u64()),
            (Some(pixels.0), Some(pixels.1)),
            "{platform}: {shot}"
        );
        assert_eq!(
            data["points"],
            json!({"width": points.0, "height": points.1}),
            "{platform}: {shot}"
        );

        let off = daemon.request(json!({"action": "click", "x": pixels.0, "y": pixels.1}));
        assert_eq!(off["code"], "invalid_argument", "{platform}: {off}");
        let message = off["error"].as_str().unwrap_or_default();
        let size = format!("{}x{}", points.0, points.1);
        assert!(
            message.contains(&size) && message.contains("points") && message.contains("pixels"),
            "{platform}: {message}"
        );
        let on = daemon.request(json!({"action": "click", "x": 200, "y": 400}));
        assert_eq!(on["success"], true, "{platform}: {on}");
    }
}
