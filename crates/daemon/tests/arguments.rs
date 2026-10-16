//! What the daemon answers a request whose envelope or parameters are
//! missing, malformed or of another type than they take (design D3 of the
//! `mcp-server` change: the requests' types change no code, message or
//! reply). Each case is a raw request line on a fake target and the code and
//! message the reply carries, or its success where the daemon does not read
//! the parameter on the command's path. Several cases give two bad values,
//! the reply naming the one the command reads first.

mod support;

use serde_json::{Value, json};
use support::{APP, Daemon, raw_request};

/// What a request line should get.
enum Answer {
    /// A failure with this code and message, and no data.
    Refused(&'static str, String),
    /// A failure with this code, whatever its message and data.
    Failed(&'static str),
    /// A success.
    Done,
}

use Answer::{Done, Failed, Refused};

fn refused(code: &'static str, message: impl Into<String>) -> Answer {
    Refused(code, message.into())
}

fn invalid(message: impl Into<String>) -> Answer {
    refused("invalid_argument", message)
}

/// The message of a number parameter given `value`.
fn whole(key: &str, value: Value) -> Answer {
    invalid(format!(
        "\"{key}\" must be a non-negative integer, not {value}"
    ))
}

/// The message of a distance or coordinate given `value`.
fn number(key: &str, value: Value) -> Answer {
    invalid(format!("\"{key}\" must be a number, not {value}"))
}

/// Sends each line on `daemon` and checks its answer.
#[track_caller]
fn check(daemon: &Daemon, cases: &[(Value, Answer)]) {
    for (request, answer) in cases {
        let line = request.to_string();
        let reply: Value = serde_json::from_str(&raw_request(&daemon.socket(), &line))
            .unwrap_or_else(|e| panic!("{line}: the reply is not JSON: {e}"));
        match answer {
            Refused(code, message) => {
                assert_eq!(reply["success"], false, "{line}: {reply}");
                assert_eq!(reply["code"], *code, "{line}: {reply}");
                assert_eq!(reply["error"], message.as_str(), "{line}: {reply}");
                assert!(reply["data"].is_null(), "{line}: {reply}");
            }
            Failed(code) => {
                assert_eq!(reply["success"], false, "{line}: {reply}");
                assert_eq!(reply["code"], *code, "{line}: {reply}");
            }
            Done => assert_eq!(reply["success"], true, "{line}: {reply}"),
        }
    }
}

/// A daemon on a fake emulator with the fixture app open on its Login tab.
fn opened() -> Daemon {
    let daemon = Daemon::start("android");
    let opened = daemon.request(json!({"action": "open", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");
    daemon
}

#[test]
fn a_line_that_is_no_request_is_refused_as_before() {
    let daemon = Daemon::start("android");
    let ask = |line: &str| -> Value {
        serde_json::from_str(&raw_request(&daemon.socket(), line)).expect("the reply is JSON")
    };
    for line in [
        "not json",
        r#"{"action": "status""#,
        r#"{"action": "status"} trailing"#,
    ] {
        let parse = serde_json::from_str::<Value>(line).expect_err("not JSON");
        let reply = ask(line);
        assert_eq!(reply["code"], "invalid_request", "{line}: {reply}");
        assert_eq!(
            reply["error"],
            format!("the request is not JSON ({parse}); send one JSON object per line"),
            "{line}"
        );
        assert!(reply["id"].is_null(), "{reply}");
    }
    for line in ["[1, 2]", r#""status""#, "7"] {
        let reply = ask(line);
        assert_eq!(reply["code"], "invalid_request", "{line}: {reply}");
        assert_eq!(reply["error"], "the request is not a JSON object", "{line}");
        assert!(reply["id"].is_null(), "{reply}");
    }
    // What an envelope's id, action and session must be, in that order;
    // the id read so far comes back.
    for (line, id, code, message) in [
        (
            r#"{"id": "r1"}"#,
            json!("r1"),
            "invalid_request",
            "the request has no \"action\" string",
        ),
        (
            r#"{"id": 7, "action": 5, "session": 5}"#,
            json!(7),
            "invalid_request",
            "the request has no \"action\" string",
        ),
        (
            r#"{"id": "r2", "action": "snapshot", "session": ""}"#,
            json!("r2"),
            "invalid_argument",
            "\"session\" must be a non-empty string",
        ),
        (
            r#"{"id": [3], "action": "status", "session": 5}"#,
            json!([3]),
            "invalid_argument",
            "\"session\" must be a non-empty string",
        ),
        (
            r#"{"id": "r4", "action": "frobnicate", "session": "alice"}"#,
            json!("r4"),
            "unknown_action",
            "unknown action \"frobnicate\"; run mobile-agent --help for the commands",
        ),
    ] {
        let reply = ask(line);
        assert_eq!(reply["id"], id, "{line}: {reply}");
        assert_eq!(reply["code"], code, "{line}: {reply}");
        assert_eq!(reply["error"], message, "{line}: {reply}");
    }
    // A session given null is the default session, and of a key given
    // twice the last counts.
    for line in [
        r#"{"action": "status", "session": null}"#,
        r#"{"action": "frobnicate", "action": "status"}"#,
    ] {
        let reply = ask(line);
        assert_eq!(reply["success"], true, "{line}: {reply}");
    }
}

#[test]
fn the_target_s_selection_is_read_as_before() {
    let daemon = opened();
    let windows = || invalid("\"windows\" is not a platform; use ios or android");
    check(
        &daemon,
        &[
            (
                json!({"action": "snapshot", "platform": "windows"}),
                windows(),
            ),
            (
                json!({"action": "devices", "platform": "windows"}),
                windows(),
            ),
            // Neither status nor set_timeout selects a target.
            (json!({"action": "status", "platform": "windows"}), Done),
            (
                json!({"action": "set_timeout", "ms": 5000, "platform": "windows"}),
                Done,
            ),
            // A platform or device of another type than a string is none, and
            // only true forces.
            (json!({"action": "snapshot", "platform": 5}), Done),
            (json!({"action": "snapshot", "device": true}), Done),
            (json!({"action": "snapshot", "force": "yes"}), Done),
            // The target is resolved before the command reads its parameters.
            (
                json!({"action": "snapshot", "device": "nope", "maxDepth": "deep"}),
                Failed("target_not_found"),
            ),
            (
                json!({"action": "snapshot", "session": "bob", "maxDepth": "deep"}),
                Failed("target_claimed"),
            ),
        ],
    );
}

#[test]
fn timeouts_and_counts_are_whole_numbers_read_where_the_command_uses_them() {
    let daemon = opened();
    check(
        &daemon,
        &[
            (
                json!({"action": "set_timeout"}),
                invalid("set timeout needs a number of milliseconds"),
            ),
            (
                json!({"action": "set_timeout", "ms": "soon"}),
                whole("ms", json!("soon")),
            ),
            (
                json!({"action": "set_timeout", "ms": -1}),
                whole("ms", json!(-1)),
            ),
            (
                json!({"action": "set_timeout", "ms": 5.0}),
                whole("ms", json!(5.0)),
            ),
            (
                json!({"action": "set_timeout", "ms": 700_000}),
                invalid(
                    "the default timeout of 700000 ms is longer than the 600000 ms a timeout may be",
                ),
            ),
            (
                json!({"action": "snapshot", "maxDepth": "deep"}),
                whole("maxDepth", json!("deep")),
            ),
            (
                json!({"action": "snapshot", "timeout": "x", "quiet": "x"}),
                Done,
            ),
            (
                json!({"action": "screenshot", "quality": "high"}),
                whole("quality", json!("high")),
            ),
            (
                json!({"action": "screenshot", "maxDimension": 1.5, "quality": "high"}),
                whole("maxDimension", json!(1.5)),
            ),
            (
                json!({"action": "wait", "ms": "soon"}),
                whole("ms", json!("soon")),
            ),
            (
                json!({"action": "wait", "ms": 10, "timeout": "x", "quiet": "x", "defaultTimeout": "x"}),
                Done,
            ),
            (
                json!({"action": "wait", "settle": true, "timeout": "x", "quiet": "x"}),
                whole("timeout", json!("x")),
            ),
            (
                json!({"action": "wait", "settle": true, "quiet": [1]}),
                whole("quiet", json!([1])),
            ),
            (
                json!({"action": "wait", "settle": true, "timeout": 700_000}),
                invalid("timeout of 700000 ms is longer than the 600000 ms a timeout may be"),
            ),
            (
                json!({"action": "wait", "settle": true, "timeout": 50, "defaultTimeout": "x"}),
                Done,
            ),
            (
                json!({"action": "wait", "target": "#login-title", "timeout": "x"}),
                whole("timeout", json!("x")),
            ),
            (
                json!({"action": "wait", "target": "#login-title", "defaultTimeout": "x"}),
                whole("defaultTimeout", json!("x")),
            ),
            (
                json!({"action": "wait", "target": "#login-title", "defaultTimeout": 700_000}),
                invalid(
                    "defaultTimeout of 700000 ms is longer than the 600000 ms a timeout may be",
                ),
            ),
            (
                json!({"action": "wait", "target": "#login-title", "timeout": 100, "defaultTimeout": "x", "quiet": "x"}),
                Done,
            ),
            (
                json!({"action": "click", "target": "#login-title", "hold": "long", "x": "1"}),
                whole("hold", json!("long")),
            ),
            (
                json!({"action": "click", "target": "#login-title", "timeout": "x"}),
                whole("timeout", json!("x")),
            ),
            (
                json!({"action": "swipe", "duration": "slow", "from": "up"}),
                whole("duration", json!("slow")),
            ),
            (
                json!({"action": "open", "app": APP, "timeout": "x"}),
                whole("timeout", json!("x")),
            ),
            (
                json!({"action": "open", "app": APP, "timeout": 700_000}),
                invalid("timeout of 700000 ms is longer than the 600000 ms a timeout may be"),
            ),
            (
                json!({"action": "get", "what": "text", "target": "#login-title", "timeout": -5}),
                whole("timeout", json!(-5)),
            ),
            (
                json!({"action": "fill", "target": "#login-username", "text": "a", "timeout": true}),
                whole("timeout", json!(true)),
            ),
            (
                json!({"action": "scroll", "target": "#login-title", "timeout": "x"}),
                whole("timeout", json!("x")),
            ),
        ],
    );
}

#[test]
fn a_session_s_default_timeout_takes_the_place_of_the_one_a_request_gives() {
    let daemon = opened();
    check(
        &daemon,
        &[
            (json!({"action": "set_timeout", "ms": 5000}), Done),
            (
                json!({"action": "wait", "target": "#login-title", "defaultTimeout": "x"}),
                Done,
            ),
        ],
    );
}

#[test]
fn points_coordinates_and_distances_are_numbers_read_where_the_command_uses_them() {
    let daemon = opened();
    check(
        &daemon,
        &[
            (
                json!({"action": "click", "x": "1", "y": [2]}),
                number("x", json!("1")),
            ),
            (
                json!({"action": "click", "x": 1, "y": [2]}),
                number("y", json!([2])),
            ),
            (
                json!({"action": "click", "x": 1}),
                invalid("click needs a \"target\" string"),
            ),
            (
                json!({"action": "click", "x": 200, "y": 50, "timeout": "x", "target": 5}),
                Done,
            ),
            (
                json!({"action": "swipe", "from": [1, 2, 3], "to": "up"}),
                invalid("\"from\" must be [x, y] in points"),
            ),
            (
                json!({"action": "swipe", "from": [1, 2], "to": [1, "x"]}),
                invalid("\"to\" must be [x, y] in points"),
            ),
            (
                json!({"action": "swipe", "direction": "left", "distance": "far"}),
                number("distance", json!("far")),
            ),
            (
                json!({"action": "swipe", "from": [200, 500], "to": [200, 450], "distance": "far", "direction": 5}),
                Done,
            ),
            (
                json!({"action": "swipe", "direction": "diagonal", "distance": "far"}),
                invalid("\"diagonal\" is not a direction; use up, down, left or right"),
            ),
            (
                json!({"action": "scroll", "amount": "a lot"}),
                number("amount", json!("a lot")),
            ),
            (
                json!({"action": "scroll", "direction": "sideways", "amount": "a lot"}),
                invalid("\"sideways\" is not a direction; use up, down, left or right"),
            ),
            (
                json!({"action": "scroll", "direction": 5, "amount": 100, "timeout": "x"}),
                Done,
            ),
        ],
    );
}

#[test]
fn strings_a_command_needs_are_refused_by_name_and_others_of_another_type_are_none() {
    let daemon = opened();
    check(
        &daemon,
        &[
            (
                json!({"action": "open"}),
                invalid("open needs a \"app\" string"),
            ),
            (
                json!({"action": "open", "app": ""}),
                invalid("open needs a \"app\" string"),
            ),
            (
                json!({"action": "open", "app": 5, "env": "A=1", "timeout": "x"}),
                invalid("open needs a \"app\" string"),
            ),
            (
                json!({"action": "open", "app": APP, "env": "A=1"}),
                invalid("\"env\" must be a list of strings, not \"A=1\""),
            ),
            (
                json!({"action": "open", "app": APP, "env": ["A=1", 2]}),
                invalid("\"env\" must hold strings, not 2"),
            ),
            (
                json!({"action": "open", "app": APP, "env": ["A"], "timeout": "x"}),
                invalid("--env takes KEY=VALUE, not \"A\""),
            ),
            (
                json!({"action": "get"}),
                invalid("get needs a \"what\" string"),
            ),
            (
                json!({"action": "get", "what": "text", "target": 1}),
                invalid("get needs a \"target\" string"),
            ),
            (
                json!({"action": "get", "what": "color", "target": "#login-title", "timeout": "x"}),
                invalid("get takes text, value or box, not \"color\""),
            ),
            (
                json!({"action": "get", "what": "color", "target": "@x1"}),
                invalid("\"@x1\" is not a ref; use a ref such as @e3, #id, text=... or role=..."),
            ),
            (
                json!({"action": "fill", "target": "#login-username"}),
                invalid("fill needs a \"text\" string"),
            ),
            (
                json!({"action": "fill", "text": 5, "target": 6}),
                invalid("fill needs a \"text\" string"),
            ),
            (
                json!({"action": "fill", "text": "a", "target": ""}),
                invalid("fill needs a \"target\" string"),
            ),
            (
                json!({"action": "type", "target": "#login-username"}),
                invalid("type needs a \"text\" string"),
            ),
            (
                json!({"action": "press"}),
                invalid("press needs a \"key\" string"),
            ),
            (
                json!({"action": "press", "key": ["Enter"]}),
                invalid("press needs a \"key\" string"),
            ),
            (
                json!({"action": "wait", "target": 5, "state": "gone"}),
                invalid(
                    "wait takes an element, a number of milliseconds, --text, --settle or --change",
                ),
            ),
            (
                json!({"action": "wait", "target": "#login-title", "state": "gone", "timeout": "x"}),
                invalid(
                    "--state takes visible, attached, hidden, detached or enabled, not \"gone\"",
                ),
            ),
            (
                json!({"action": "wait", "target": "@e999", "state": "gone"}),
                refused(
                    "unknown_ref",
                    "@e999 is not a ref from session \"default\"'s snapshots of emulator-5554; take a snapshot and use the refs it prints",
                ),
            ),
            (
                json!({"action": "wait", "text": 5, "target": "#login-title", "state": 7}),
                Done,
            ),
            (
                json!({"action": "screenshot", "path": 5, "interactive": "yes"}),
                Done,
            ),
        ],
    );
}

#[test]
fn a_diff_reads_its_listing_options_only_without_a_last_snapshot() {
    let daemon = opened();
    check(
        &daemon,
        &[
            (
                json!({"action": "diff_snapshot", "maxDepth": "deep"}),
                whole("maxDepth", json!("deep")),
            ),
            (json!({"action": "snapshot"}), Done),
            (json!({"action": "diff_snapshot", "maxDepth": "deep"}), Done),
        ],
    );
}
