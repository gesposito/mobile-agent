//! The request types against the lines the CLI sends (task 3.1 of the
//! `mcp-server` change). `lines.json` holds, for each command and option,
//! the line `crates/cli/src/request.rs` built for it before the types
//! existed, with the fields the CLI adds to every line (`id`, `session`, and
//! `platform`, `device`, `force` and `defaultTimeout` when set), and the
//! line `doctor` asks the daemon with (`crates/cli/src/doctor.rs`).

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};
use wire::{Command, Request};

const LINES: &str = include_str!("lines.json");

/// The lines that give a parameter at its default, by their command line,
/// with those parameters: a request leaves a parameter at its default out
/// of the line it serializes to, which the daemon reads the same.
const AT_DEFAULT: [(&str, &[&str]); 8] = [
    ("swipe up", &["direction"]),
    ("swipe down 300", &["distance"]),
    ("scroll down", &["direction"]),
    ("scroll up 300", &["amount"]),
    (
        "scroll down 600 -s @e7 --until #row-150 --timeout 9000",
        &["direction"],
    ),
    (
        "screenshot --quality 80 --max-dimension 1000",
        &["maxDimension", "quality"],
    ),
    ("wait #x --state visible", &["state"]),
    ("wait --settle --quiet 200", &["quiet"]),
];

/// One line of `lines.json`.
struct Case {
    /// The command line, its words joined by spaces.
    command: String,
    line: Map<String, Value>,
}

fn cases() -> Vec<Case> {
    let entries: Vec<Value> = serde_json::from_str(LINES).expect("lines.json is JSON");
    entries
        .iter()
        .map(|entry| Case {
            command: entry["argv"]
                .as_array()
                .expect("argv is a list")
                .iter()
                .map(|word| word.as_str().expect("argv holds strings"))
                .collect::<Vec<_>>()
                .join(" "),
            line: entry["line"]
                .as_object()
                .cloned()
                .expect("line is an object"),
        })
        .collect()
}

#[track_caller]
fn parse(line: &Map<String, Value>) -> Request {
    serde_json::from_value(Value::Object(line.clone()))
        .unwrap_or_else(|e| panic!("{line:?} does not parse: {e}"))
}

fn line(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("an object")
}

/// The parameters `line` gives at their default: those with which the
/// action alone parses to the same request as without them. The envelope's
/// `id` and `session` are written back as given.
fn at_default(line: &Map<String, Value>) -> BTreeSet<String> {
    let alone = |entry: Option<(&String, &Value)>| {
        let mut alone = Map::new();
        alone.insert("action".into(), line["action"].clone());
        if let Some((key, value)) = entry {
            alone.insert(key.clone(), value.clone());
        }
        parse(&alone)
    };
    let bare = alone(None);
    line.iter()
        .filter(|(key, _)| !matches!(key.as_str(), "action" | "id" | "session"))
        .filter(|entry| alone(Some(*entry)) == bare)
        .map(|(key, _)| key.clone())
        .collect()
}

#[test]
fn every_line_the_cli_sends_parses_into_its_action_and_serializes_back_to_the_same_json() {
    for case in cases() {
        let request = parse(&case.line);
        let excused: BTreeSet<String> = AT_DEFAULT
            .iter()
            .find(|(command, _)| *command == case.command)
            .map(|(_, keys)| keys.iter().map(|key| (*key).to_owned()).collect())
            .unwrap_or_default();
        assert_eq!(
            at_default(&case.line),
            excused,
            "{}: the parameters at their default",
            case.command
        );
        let mut expected = case.line.clone();
        expected.retain(|key, _| !excused.contains(key));
        let written = serde_json::to_value(&request).expect("a request serializes");
        assert_eq!(
            written,
            Value::Object(expected),
            "{}: the line written back",
            case.command
        );
        let again: Request = serde_json::from_value(written).expect("the line written back parses");
        assert_eq!(again, request, "{}: the line read back", case.command);
    }
}

#[test]
fn the_cli_lines_cover_every_action() {
    let covered: BTreeSet<String> = cases()
        .iter()
        .map(|case| case.line["action"].as_str().expect("an action").to_owned())
        .collect();
    let all: BTreeSet<String> = Command::ACTIONS
        .iter()
        .map(|action| (*action).to_owned())
        .collect();
    assert_eq!(covered, all);
}

#[test]
fn every_action_s_name_reads_as_its_command() {
    for action in Command::ACTIONS {
        let request = parse(&line(json!({"action": action})));
        assert_eq!(request.command.action(), action);
    }
}

#[test]
fn a_key_the_action_does_not_take_is_ignored() {
    for case in cases() {
        let mut extended = case.line.clone();
        extended.insert("frobnicate".into(), json!({"any": ["value"]}));
        assert_eq!(parse(&extended), parse(&case.line), "{}", case.command);
    }
}

/// The daemon reads a parameter only where a command uses it, so a value of
/// another type passes the parse and is kept as it came, for the daemon to
/// refuse when it reads it.
#[test]
fn a_number_a_point_or_a_list_of_another_type_is_kept_as_it_came() {
    for given in [
        json!({"action": "wait", "ms": "soon", "quiet": -1, "timeout": 2.5}),
        json!({"action": "snapshot", "maxDepth": "deep"}),
        json!({"action": "open", "app": "a", "env": "A=1"}),
        json!({"action": "open", "app": "a", "env": ["A=1", 2]}),
        json!({"action": "click", "x": "1", "y": [2], "hold": true}),
        json!({"action": "swipe", "from": [1, 2, 3], "to": "up", "distance": "far", "duration": -5}),
        json!({"action": "scroll", "amount": "a lot"}),
        json!({"action": "screenshot", "quality": "high", "maxDimension": 1.5}),
        json!({"action": "set_timeout", "ms": "soon"}),
        json!({"action": "status", "defaultTimeout": "soon"}),
    ] {
        let written = serde_json::to_value(parse(&line(given.clone()))).expect("serializes");
        for (key, value) in given.as_object().expect("an object") {
            assert_eq!(&written[key], value, "{given}: {key}");
        }
    }
}

/// The daemon has always read a string given another type as no string,
/// and a flag as set only by `true`; each case holds the same keys with
/// values that count, which parse to another request.
#[test]
fn a_string_of_another_type_is_absent_and_only_true_sets_a_flag() {
    let cases = [
        (
            json!({"action": "snapshot", "interactive": "yes", "delta": 1, "full": null}),
            json!({"action": "snapshot", "interactive": true, "delta": true, "full": true}),
        ),
        (
            json!({"action": "wait", "text": 5, "target": ["#a"], "state": false, "settle": "true"}),
            json!({"action": "wait", "text": "a", "target": "#a", "state": "hidden", "settle": true}),
        ),
        (
            json!({"action": "scroll", "direction": 1, "target": {}, "until": 2}),
            json!({"action": "scroll", "direction": "up", "target": "@e1", "until": "#a"}),
        ),
        (
            json!({"action": "swipe", "direction": null}),
            json!({"action": "swipe", "direction": "left"}),
        ),
        (
            json!({"action": "screenshot", "path": 7}),
            json!({"action": "screenshot", "path": "/tmp/a.jpg"}),
        ),
        (
            json!({"action": "open", "app": 5, "relaunch": "yes"}),
            json!({"action": "open", "app": "a", "relaunch": true}),
        ),
        (
            json!({"action": "get", "what": 1, "target": null}),
            json!({"action": "get", "what": "text", "target": "@e1"}),
        ),
        (
            json!({"action": "fill", "target": 1, "text": 2}),
            json!({"action": "fill", "target": "@e1", "text": ""}),
        ),
        (
            json!({"action": "press", "key": ["Enter"]}),
            json!({"action": "press", "key": "Enter"}),
        ),
        (
            json!({"action": "devices", "platform": 1, "device": true, "force": "yes"}),
            json!({"action": "devices", "platform": "ios", "device": "a", "force": true}),
        ),
    ];
    for (ignored, counted) in cases {
        let mut alone = Map::new();
        alone.insert("action".into(), ignored["action"].clone());
        let alone = parse(&alone);
        assert_eq!(parse(&line(ignored.clone())), alone, "{ignored}");
        assert_ne!(parse(&line(counted.clone())), alone, "{counted}");
    }
}

/// The daemon has always read a parameter given as null as one not given;
/// the same keys with values parse to another request.
#[test]
fn null_is_a_parameter_not_given() {
    for (action, keys) in [
        (
            "wait",
            &[
                ("ms", json!(5)),
                ("settle", json!(true)),
                ("quiet", json!(5)),
                ("treeOnly", json!(true)),
                ("text", json!("a")),
                ("target", json!("#a")),
                ("state", json!("hidden")),
                ("timeout", json!(5)),
                ("defaultTimeout", json!(5)),
            ][..],
        ),
        (
            "open",
            &[
                ("env", json!(["A=1"])),
                ("timeout", json!(5)),
                ("relaunch", json!(true)),
            ][..],
        ),
        (
            "swipe",
            &[
                ("direction", json!("left")),
                ("distance", json!(5)),
                ("from", json!([1, 2])),
                ("to", json!([3, 4])),
                ("duration", json!(5)),
            ][..],
        ),
        (
            "scroll",
            &[
                ("direction", json!("up")),
                ("amount", json!(5)),
                ("target", json!("@e1")),
                ("until", json!("#a")),
            ][..],
        ),
        (
            "screenshot",
            &[
                ("path", json!("/tmp/a.jpg")),
                ("quality", json!(5)),
                ("maxDimension", json!(5)),
            ][..],
        ),
        (
            "click",
            &[
                ("target", json!("@e1")),
                ("x", json!(1)),
                ("y", json!(2)),
                ("hold", json!(5)),
            ][..],
        ),
    ] {
        let mut alone = Map::new();
        alone.insert("action".into(), action.into());
        let (mut nulls, mut values) = (alone.clone(), alone.clone());
        for (key, value) in keys {
            nulls.insert((*key).into(), Value::Null);
            values.insert((*key).into(), value.clone());
        }
        let alone = parse(&alone);
        assert_eq!(parse(&nulls), alone, "{action}");
        assert_ne!(parse(&values), alone, "{action}");
    }
}

/// A command's `Default` is the request that gives none of its parameters,
/// as a tool called with `{}` gives none.
#[test]
fn an_action_s_default_parameters_are_those_of_a_request_that_gives_none() {
    let defaults = [
        Command::Status(Default::default()),
        Command::Devices(Default::default()),
        Command::Close(Default::default()),
        Command::SetTimeout(Default::default()),
        Command::Doctor(Default::default()),
        Command::Open(Default::default()),
        Command::Snapshot(Default::default()),
        Command::DiffSnapshot(Default::default()),
        Command::Get(Default::default()),
        Command::Wait(Default::default()),
        Command::Screenshot(Default::default()),
        Command::Click(Default::default()),
        Command::Fill(Default::default()),
        Command::Type(Default::default()),
        Command::Press(Default::default()),
        Command::Swipe(Default::default()),
        Command::Scroll(Default::default()),
        Command::Shutdown(Default::default()),
    ];
    for command in defaults {
        let request = Request::new(command);
        let action = request.command.action();
        assert_eq!(parse(&line(json!({"action": action}))), request, "{action}");
        assert_eq!(
            serde_json::to_value(&request).expect("serializes"),
            json!({"action": action, "session": "default"}),
            "{action}"
        );
    }
}
