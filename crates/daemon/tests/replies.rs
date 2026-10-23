//! The reply types of `crates/wire` held to the handlers (design D3 of the
//! `mcp-server` change): each action's reply on a fake target parses into
//! the envelope and its action's data, every field known, so a field a
//! handler adds fails here until the type gains it; and so do the details
//! of the failures a fake target can give.

mod support;

use serde_json::{Value, json};
use support::{APP, Daemon, TempDir};
use wire::reply::{self, Reply};

/// `$reply`, a success, read as the envelope with `$data` as its data,
/// unknown fields refused at every level; evaluates to the reply.
macro_rules! success {
    ($reply:expr, $data:ty) => {{
        let reply: Value = $reply;
        assert_eq!(reply["success"], true, "{reply}");
        let read: Reply<$data> = serde_json::from_value(reply.clone())
            .unwrap_or_else(|e| panic!("{reply} is no {}: {e}", stringify!($data)));
        assert!(read.data.is_some(), "{reply}");
        reply
    }};
}

/// `$reply`, a failure with `$code`, read as the envelope with the failure
/// details, if any, as its data; evaluates to the reply.
macro_rules! failure {
    ($reply:expr, $code:expr) => {{
        let reply: Value = $reply;
        assert_eq!(reply["success"], false, "{reply}");
        assert_eq!(reply["code"], $code, "{reply}");
        let read: Reply<reply::Details> = serde_json::from_value(reply.clone())
            .unwrap_or_else(|e| panic!("{reply} is no failure: {e}"));
        assert_eq!(read.data.is_some(), !reply["data"].is_null(), "{reply}");
        reply
    }};
}

#[test]
fn every_action_s_reply_on_a_fake_target_parses_into_its_reply_type() {
    let dir = TempDir::new();
    let daemon = Daemon::start("android");
    let ask = |request: Value| daemon.request(request);

    let status = success!(ask(json!({"action": "status"})), reply::Status);
    assert_eq!(status["data"]["targets"][0]["attached"], false, "{status}");
    success!(ask(json!({"action": "devices"})), reply::Devices);
    success!(ask(json!({"action": "doctor"})), reply::Doctor);
    success!(
        ask(json!({"action": "set_timeout", "ms": 5000})),
        reply::SetTimeout
    );
    success!(ask(json!({"action": "open", "app": APP})), reply::Open);
    let status = success!(ask(json!({"action": "status"})), reply::Status);
    assert_eq!(status["data"]["targets"][0]["attached"], true, "{status}");

    // A listing with its refs, the full one, and the delta kinds a session's
    // first and second delta snapshots give.
    for request in [
        json!({"action": "snapshot", "interactive": true}),
        json!({"action": "snapshot"}),
        json!({"action": "snapshot", "interactive": true, "delta": true}),
        json!({"action": "snapshot", "interactive": true, "delta": true}),
    ] {
        success!(ask(request), reply::Snapshot);
    }
    success!(ask(json!({"action": "diff_snapshot"})), reply::DiffSnapshot);
    for what in ["text", "value", "box"] {
        success!(
            ask(json!({"action": "get", "what": what, "target": "#login-username"})),
            reply::Get
        );
    }
    success!(
        ask(json!({"action": "fill", "target": "#login-username", "text": "agent"})),
        reply::Fill
    );
    // The fill changed the field's value, which the diff names.
    let diff = success!(ask(json!({"action": "diff_snapshot"})), reply::DiffSnapshot);
    let fields = diff["data"]["modified"][0]["fields"].as_array().cloned();
    assert!(
        fields.is_some_and(|fields| fields.contains(&json!("value"))),
        "{diff}"
    );
    for request in [
        json!({"action": "type", "target": "#login-username", "text": "s"}),
        json!({"action": "type", "text": "s"}),
    ] {
        success!(ask(request), reply::TypeText);
    }
    // With the keyboard up, the tabs are out of view.
    failure!(
        ask(json!({"action": "click", "target": "#tab-list"})),
        "element_offscreen"
    );
    success!(ask(json!({"action": "press", "key": "Back"})), reply::Press);
    // A click names the element its tap landed on, by a selector or at a
    // point (design D1 of action-verdicts).
    let clicked = success!(
        ask(json!({"action": "click", "target": "#login-submit"})),
        reply::Click
    );
    assert_eq!(clicked["data"]["hit"]["name"], "Sign in", "{clicked}");
    let pressed = success!(
        ask(json!({"action": "click", "x": 200, "y": 50, "hold": 300})),
        reply::Click
    );
    assert_eq!(pressed["data"]["hit"]["role"], "heading", "{pressed}");
    for request in [
        json!({"action": "wait", "ms": 10}),
        json!({"action": "wait", "settle": true}),
        json!({"action": "wait", "target": "#login-title"}),
        json!({"action": "wait", "text": "Login"}),
        json!({"action": "wait", "target": "#nothing", "state": "hidden"}),
    ] {
        success!(ask(request), reply::Wait);
    }
    let path = dir.path().join("shot.jpg");
    for request in [
        json!({"action": "screenshot", "path": path.to_string_lossy()}),
        json!({"action": "screenshot", "path": path.to_string_lossy(), "inline": true}),
        json!({"action": "screenshot", "inline": true}),
    ] {
        success!(ask(request), reply::Screenshot);
    }
    for request in [
        json!({"action": "swipe", "direction": "left"}),
        json!({"action": "swipe", "from": [200, 700], "to": [200, 600], "duration": 100}),
    ] {
        success!(ask(request), reply::Swipe);
    }

    success!(
        ask(json!({"action": "click", "target": "#tab-list"})),
        reply::Click
    );
    // A settle right after an input names it (design D2 of action-verdicts).
    let settled = success!(ask(json!({"action": "wait", "settle": true})), reply::Wait);
    assert_eq!(settled["data"]["input"], "click", "{settled}");
    // The list came in since the session's last snapshot: a change wait
    // answers at once with the delta, and then waits out its bound.
    let changed = success!(ask(json!({"action": "wait", "change": true})), reply::Wait);
    assert_eq!(changed["data"]["waited"], "change", "{changed}");
    failure!(
        ask(json!({"action": "wait", "change": true, "timeout": 300})),
        "wait_timeout"
    );
    // A plain scroll answers once the screen has come to rest after its
    // strokes, saying how long the content moved on (design D6 of
    // action-verdicts).
    let scrolled = success!(
        ask(json!({"action": "scroll", "direction": "down", "amount": 300})),
        reply::Scroll
    );
    assert_eq!(scrolled["data"]["settled"], true, "{scrolled}");
    assert!(scrolled["data"]["movedOnMs"].is_number(), "{scrolled}");
    success!(
        ask(json!({"action": "scroll", "direction": "up", "amount": 600, "until": "#row-2"})),
        reply::Scroll
    );
    let stopped = failure!(
        ask(json!({"action": "scroll", "amount": 300, "until": "#row-none", "timeout": 1500})),
        "element_not_found"
    );
    assert_eq!(stopped["data"]["until"], "#row-none", "{stopped}");
    let short = failure!(
        ask(json!({"action": "scroll", "amount": 2})),
        "stroke_too_short"
    );
    assert_eq!(short["data"]["stroke"], 2.0, "{short}");

    let claimed = failure!(
        ask(json!({"action": "snapshot", "session": "bob"})),
        "target_claimed"
    );
    assert_eq!(claimed["data"]["holder"], "default", "{claimed}");
    let missing = failure!(
        ask(json!({"action": "snapshot", "device": "nope"})),
        "target_not_found"
    );
    assert_eq!(
        missing["data"]["candidates"].as_array().map(Vec::len),
        Some(1)
    );
    for (request, code) in [
        (
            json!({"action": "snapshot", "platform": "windows"}),
            "invalid_argument",
        ),
        (json!({"action": "open"}), "invalid_argument"),
        (json!({"action": "frobnicate"}), "unknown_action"),
        (json!({"id": "r1"}), "invalid_request"),
    ] {
        let refused = failure!(ask(request), code);
        assert!(refused["data"].is_null(), "{refused}");
    }

    for request in [
        json!({"action": "close", "keep": true}),
        json!({"action": "close"}),
        json!({"action": "close"}),
    ] {
        success!(ask(request), reply::Close);
    }
    let stopped = success!(ask(json!({"action": "shutdown"})), reply::Shutdown);
    assert_eq!(stopped["data"]["stopped"], true, "{stopped}");
}

/// `cancel`'s reply with a command in flight and without, and the failure
/// of the command it ends, which carries no details.
#[test]
fn a_cancel_s_reply_and_the_failure_it_causes_parse_into_their_types() {
    let daemon = Daemon::start("android");
    let idle = success!(daemon.request(json!({"action": "cancel"})), reply::Cancel);
    assert_eq!(idle["data"]["cancelled"], false, "{idle}");
    success!(
        daemon.request(json!({"action": "open", "app": APP})),
        reply::Open
    );
    let socket = daemon.socket();
    let waiting = std::thread::spawn(move || {
        support::request_at(
            &socket,
            &json!({"action": "wait", "text": "Nobody shows this"}),
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    let cancel = success!(daemon.request(json!({"action": "cancel"})), reply::Cancel);
    assert_eq!(cancel["data"]["cancelled"], true, "{cancel}");
    let ended = waiting.join().expect("the wait's connection");
    let ended = failure!(ended, "cancelled");
    assert!(ended["data"].is_null(), "{ended}");
}

/// The replies of a get, a click, a fill, a type and a scroll on a stale
/// ref that the daemon resolved to its twin (design D3 of action-verdicts),
/// a field of the placeholder app's rebuilt board that its text put in a
/// new node, with the stale ref and the twin's in `resolved`.
#[test]
fn replies_on_a_stale_ref_resolved_to_its_twin_parse_into_their_types() {
    let daemon = Daemon::start("ios");
    let ask = |request: Value| daemon.request(request);
    success!(
        ask(
            json!({"action": "open", "app": "dev.mobileagent.placeholder",
                   "env": ["PLACEHOLDER_SCREEN=rebuilt"]})
        ),
        reply::Open
    );
    let listing = success!(
        ask(json!({"action": "snapshot", "interactive": true})),
        reply::Snapshot
    );
    let field = listing["data"]["refs"]
        .as_object()
        .and_then(|refs| {
            refs.iter()
                .find(|(_, entry)| entry["name"] == "placeholder-textbox-1")
                .map(|(reference, _)| format!("@{reference}"))
        })
        .unwrap_or_else(|| panic!("no field in {listing}"));
    success!(
        ask(json!({"action": "fill", "target": "#placeholder-id-1", "text": "standup"})),
        reply::Fill
    );
    let resolved = |reply: &Value| {
        assert_eq!(
            reply["data"]["resolved"]["stale"],
            field.as_str(),
            "{reply}"
        );
    };
    resolved(&success!(
        ask(json!({"action": "get", "what": "value", "target": field})),
        reply::Get
    ));
    resolved(&success!(
        ask(json!({"action": "click", "target": field})),
        reply::Click
    ));
    resolved(&success!(
        ask(json!({"action": "fill", "target": field, "text": "again"})),
        reply::Fill
    ));
    resolved(&success!(
        ask(json!({"action": "type", "target": field, "text": "s"})),
        reply::TypeText
    ));
    resolved(&success!(
        ask(json!({"action": "scroll", "direction": "down", "amount": 100, "target": field})),
        reply::Scroll
    ));
}

#[test]
fn several_targets_give_the_candidates_and_a_short_typing_target_a_mismatch() {
    let several = Daemon::start("android,ios");
    let ambiguous = failure!(
        several.request(json!({"action": "snapshot"})),
        "ambiguous_target"
    );
    assert_eq!(
        ambiguous["data"]["candidates"].as_array().map(Vec::len),
        Some(2)
    );

    let short = Daemon::builder()
        .fake("android")
        .env("MOBILE_AGENT_FAKE_TYPING", "max:3")
        .start();
    success!(
        short.request(json!({"action": "open", "app": APP})),
        reply::Open
    );
    let mismatch = failure!(
        short.request(json!({"action": "fill", "target": "#login-username", "text": "agent"})),
        "text_mismatch"
    );
    assert_eq!(mismatch["data"]["value"], "age", "{mismatch}");
}

#[test]
fn a_warning_rides_on_the_envelope_and_a_driver_s_report_on_its_target() {
    // A launch splash longer than the open's timeout warns.
    let splash = Daemon::builder()
        .fake("android")
        .env("MOBILE_AGENT_FAKE_SPLASH_MS", "3000")
        .start();
    let opened = success!(
        splash.request(json!({"action": "open", "app": APP, "timeout": 300})),
        reply::Open
    );
    assert!(opened["warning"].is_string(), "{opened}");

    // A fake emulator kept off its controller reports its paths in status.
    let agent = Daemon::builder()
        .fake("android")
        .env("MOBILE_AGENT_EMULATOR_CONTROLLER", "off")
        .start();
    success!(
        agent.request(json!({"action": "open", "app": APP})),
        reply::Open
    );
    let status = success!(agent.request(json!({"action": "status"})), reply::Status);
    assert!(
        status["data"]["targets"][0]["driver"].is_object(),
        "{status}"
    );
}

/// The delta snapshot's kinds as the mirror writes them, each change kind
/// among them: the fake gives the full and unchanged kinds above, and the
/// delta kind only where the mirror's own tests patch a screen.
#[test]
fn every_delta_kind_the_mirror_writes_parses_into_the_delta_type() {
    use mirror::{DeltaSnapshot, Ref, RefChange, RefEntry, RefField, Role, TreeSplice};
    let entry = |n: u32, name: &str| RefEntry {
        reference: Ref(n),
        role: Role::Button,
        name: name.to_owned(),
    };
    for delta in [
        DeltaSnapshot::Full {
            revision: 1,
            tree: "- button \"A\" [ref=e1]".to_owned(),
            refs: vec![entry(1, "A")],
        },
        DeltaSnapshot::Unchanged {
            revision: 2,
            base_revision: 1,
        },
        DeltaSnapshot::Delta {
            revision: 3,
            base_revision: 2,
            changes: vec![
                RefChange::Add(entry(2, "B")),
                RefChange::Remove(Ref(1)),
                RefChange::Replace {
                    reference: Ref(3),
                    field: RefField::Name,
                    value: "C".to_owned(),
                },
            ],
            tree_change: TreeSplice {
                start_line: 0,
                delete_count: 1,
                lines: vec!["- button \"B\" [ref=e2]".to_owned()],
            },
        },
    ] {
        let json = delta.to_json();
        let read: Result<reply::DeltaSnapshot, _> = serde_json::from_str(&json);
        assert!(read.is_ok(), "{json}: {read:?}");
    }
}
