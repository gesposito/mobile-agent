//! agent-device. A flow becomes one `batch` request, the tool's documented way
//! to run several commands in one invocation:
//! `agent-device batch --steps-file <file> --on-error stop --json
//! --platform <ios|android> --udid <udid> | --serial <serial> --session <name>`.
//! Each step is `{"command", "input"}`, where the input holds the command's
//! structured fields (`agent-device help batch`), and the JSON reply carries
//! `success` and the batch's own `totalDurationMs`.
//!
//! Assertions wait rather than check once, as Maestro's do: a visible element
//! becomes a `wait` on its selector, and a text becomes a `wait` for the text
//! followed by an `is text` on the element.
//!
//! Between commands agent-device keeps its session, and on Android with it a
//! snapshot helper, an instrumentation that holds the device's one UI
//! automation role, and the test input method it selects for text entry.
//! `agent-device close` ends the session: it stops the helper and selects the
//! input method it found again. `close --shutdown` would also shut the target
//! down, which the harness never asks for.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use serde_json::{Value, json};

use super::{json_document, last_line};
use crate::end_state::{Element, Reading};
use crate::flow::{Direction, Flow, Selector, Step};
use crate::target::{Platform, Target};

/// How long a `wait` step waits.
pub const ASSERT_TIMEOUT_MS: u64 = 10_000;

/// The batch steps for `flow` against `app_id` on `platform`, as JSON.
pub fn steps(flow: &Flow, app_id: &str, platform: Platform) -> String {
    let mut steps = Vec::new();
    for step in &flow.steps {
        match step {
            Step::Launch => steps.push(json!({
                "command": "open",
                "input": {"app": app_id, "platform": platform.id(), "relaunch": true}
            })),
            Step::Tap(target) => steps.push(json!({
                "command": "press",
                "input": {"target": element(target)}
            })),
            Step::Fill { target, text } => steps.push(json!({
                "command": "fill",
                "input": {"target": element(target), "text": text}
            })),
            Step::ScrollTo { target, direction } => steps.push(json!({
                "command": "scroll",
                "input": {
                    "direction": match direction {
                        Direction::Down => "down",
                        Direction::Up => "up",
                    },
                    "until": selector(target)
                }
            })),
            Step::AssertVisible(target) => steps.push(json!({
                "command": "wait",
                "input": {"kind": "selector", "selector": selector(target), "timeoutMs": ASSERT_TIMEOUT_MS}
            })),
            Step::AssertText { id, text } => {
                steps.push(json!({
                    "command": "wait",
                    "input": {"kind": "text", "text": text, "timeoutMs": ASSERT_TIMEOUT_MS}
                }));
                steps.push(json!({
                    "command": "is",
                    "input": {
                        "predicate": "text",
                        "selector": selector(&Selector::Id(id.clone())),
                        "value": text
                    }
                }));
            }
        }
    }
    serde_json::to_string_pretty(&Value::Array(steps)).expect("batch steps always serialize")
}

/// A step's target: the element a selector finds.
fn element(target: &Selector) -> Value {
    json!({"kind": "selector", "selector": selector(target)})
}

/// The agent-device selector for `selector`: `id="…"` or `text="…"`, with
/// backslashes and quotes escaped.
pub fn selector(selector: &Selector) -> String {
    let (key, value) = match selector {
        Selector::Id(id) => ("id", id),
        Selector::Text(text) => ("text", text),
    };
    format!(
        "{key}=\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// Arguments for `agent-device` running the steps in `file`.
pub fn args(target: &Target, session: &str, file: &Path) -> Vec<OsString> {
    let platform = target.class.platform();
    vec![
        "batch".into(),
        "--steps-file".into(),
        file.as_os_str().to_owned(),
        "--on-error".into(),
        "stop".into(),
        "--json".into(),
        "--platform".into(),
        platform.id().into(),
        device_flag(platform).into(),
        target.id.clone().into(),
        "--session".into(),
        session.into(),
    ]
}

/// Reads the batch's JSON reply. Success gives a note of the duration
/// agent-device reported; failure gives the failing step and its error.
pub fn outcome(stdout: &str) -> Result<Option<String>, String> {
    let reply = json_document(stdout).ok_or_else(|| no_reply(stdout))?;
    if reply["success"] == true {
        return Ok(reply["data"]["totalDurationMs"]
            .as_u64()
            .map(|ms| format!("agent-device reported {ms} ms")));
    }
    let message = error_message(&reply);
    let step = reply["data"]["results"]
        .as_array()
        .and_then(|results| results.iter().find(|r| r["ok"] == false))
        .map(|r| {
            format!(
                "step {} ({})",
                r["step"]
                    .as_u64()
                    .map_or_else(|| "?".to_owned(), |n| n.to_string()),
                r["command"].as_str().unwrap_or("?")
            )
        });
    Err(match (step, message) {
        (Some(step), Some(message)) => format!("{step} failed: {message}"),
        (Some(step), None) => format!("{step} failed"),
        (None, Some(message)) => message,
        (None, None) => "agent-device reported a failure without an error".to_owned(),
    })
}

/// Arguments for `agent-device close` of the session on `target`, as JSON.
pub fn close_args(target: &Target, session: &str) -> Vec<OsString> {
    let platform = target.class.platform();
    vec![
        "close".into(),
        "--json".into(),
        "--platform".into(),
        platform.id().into(),
        device_flag(platform).into(),
        target.id.clone().into(),
        "--session".into(),
        session.into(),
    ]
}

/// Reads the reply of `agent-device close`: `Ok(true)` when it closed the
/// session, `Ok(false)` when there was no session to close
/// (`SESSION_NOT_FOUND`), else the failure.
pub fn closed(stdout: &str) -> Result<bool, String> {
    let reply = json_document(stdout).ok_or_else(|| no_reply(stdout))?;
    if reply["success"] == true {
        return Ok(true);
    }
    if reply["error"]["code"] == "SESSION_NOT_FOUND" {
        return Ok(false);
    }
    Err(error_message(&reply)
        .unwrap_or_else(|| "agent-device reported a failure without an error".to_owned()))
}

/// Arguments for `agent-device snapshot` of the session's screen, as JSON:
/// the whole tree, not only interactive elements.
pub fn snapshot_args(target: &Target, session: &str) -> Vec<OsString> {
    let platform = target.class.platform();
    vec![
        "snapshot".into(),
        "--json".into(),
        "--platform".into(),
        platform.id().into(),
        device_flag(platform).into(),
        target.id.clone().into(),
        "--session".into(),
        session.into(),
    ]
}

/// Reads the reply `agent-device snapshot --json` prints: `data.nodes`, a
/// flat list whose nodes name their parent by `parentIndex`. An element's
/// identifier is its `identifier`, and its text is the `value` a field holds,
/// else its `label`. A node marked `inheritsLabel` or `inheritsIdentifier`
/// had that value left out because it equals its nearest ancestor's, which
/// it takes. Every node counts.
pub fn read_snapshot(stdout: &str) -> Result<Reading, String> {
    let reply = json_document(stdout).ok_or_else(|| no_reply(stdout))?;
    if reply["success"] != true {
        return Err(error_message(&reply)
            .unwrap_or_else(|| "agent-device reported a failure without an error".to_owned()));
    }
    let nodes = reply["data"]["nodes"]
        .as_array()
        .ok_or("agent-device's snapshot reply has no nodes")?;
    let by_index: HashMap<u64, &Value> = nodes
        .iter()
        .filter_map(|node| Some((node["index"].as_u64()?, node)))
        .collect();
    let parent = |node: &Value| {
        node["parentIndex"]
            .as_u64()
            .and_then(|i| by_index.get(&i).copied())
    };
    // The node's own value for `key`, or when `inherits` marks it, the nearest
    // ancestor's; at most as many steps up as there are nodes.
    let own_or_inherited = |node: &Value, key: &str, inherits: &str| -> Option<String> {
        let value = |node: &Value| {
            node[key]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        if let Some(own) = value(node) {
            return Some(own);
        }
        if node[inherits] != true {
            return None;
        }
        let mut ancestor = parent(node);
        for _ in 0..nodes.len() {
            let current = ancestor?;
            if let Some(found) = value(current) {
                return Some(found);
            }
            ancestor = parent(current);
        }
        None
    };
    let elements: Vec<Element> = nodes
        .iter()
        .map(|node| {
            let label = own_or_inherited(node, "label", "inheritsLabel");
            let value = node["value"].as_str().filter(|v| !v.is_empty());
            Element {
                id: own_or_inherited(node, "identifier", "inheritsIdentifier"),
                text: value.map(str::to_owned).or(label),
            }
        })
        .collect();
    Ok(Reading {
        nodes: u32::try_from(elements.len()).unwrap_or(u32::MAX),
        elements,
    })
}

/// `--udid` for a Simulator, `--serial` for an Android target.
fn device_flag(platform: Platform) -> &'static str {
    match platform {
        Platform::Ios => "--udid",
        Platform::Android => "--serial",
    }
}

/// The error a failed reply carries: a string, or `code: message`.
fn error_message(reply: &Value) -> Option<String> {
    let error = &reply["error"];
    error.as_str().map(str::to_owned).or_else(|| {
        let message = error["message"].as_str()?;
        Some(match error["code"].as_str() {
            Some(code) => format!("{code}: {message}"),
            None => message.to_owned(),
        })
    })
}

fn no_reply(stdout: &str) -> String {
    format!(
        "agent-device printed no JSON reply{}",
        last_line(stdout)
            .map(|line| format!(": {line}"))
            .unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;
    use crate::flow::{Direction, Step};
    use crate::target::TargetClass;
    use serde_json::{Value, json};

    #[test]
    fn every_step_becomes_its_batch_command() {
        let flow = Flow {
            name: "every-step".into(),
            fixture: Fixture::Expo,
            steps: vec![
                Step::Launch,
                Step::Tap(Selector::id("tab-list")),
                Step::ScrollTo {
                    target: Selector::id("row-200"),
                    direction: Direction::Down,
                },
                Step::AssertVisible(Selector::text("Detail 200")),
                Step::Fill {
                    target: Selector::id("login-username"),
                    text: "agent".into(),
                },
                Step::AssertText {
                    id: "login-status".into(),
                    text: "Welcome, agent".into(),
                },
            ],
            count_nodes: false,
            golden: Default::default(),
        };
        let steps: Value = serde_json::from_str(&steps(
            &flow,
            "dev.mobileagent.fixture.expo",
            Platform::Android,
        ))
        .unwrap();
        let by_id = |id: &str| json!({"kind": "selector", "selector": format!("id=\"{id}\"")});
        assert_eq!(
            steps,
            json!([
                {"command": "open", "input": {"app": "dev.mobileagent.fixture.expo", "platform": "android", "relaunch": true}},
                {"command": "press", "input": {"target": by_id("tab-list")}},
                {"command": "scroll", "input": {"direction": "down", "until": "id=\"row-200\""}},
                {"command": "wait", "input": {"kind": "selector", "selector": "text=\"Detail 200\"", "timeoutMs": 10000}},
                {"command": "fill", "input": {"target": by_id("login-username"), "text": "agent"}},
                {"command": "wait", "input": {"kind": "text", "text": "Welcome, agent", "timeoutMs": 10000}},
                {"command": "is", "input": {"predicate": "text", "selector": "id=\"login-status\"", "value": "Welcome, agent"}}
            ])
        );
    }

    #[test]
    fn selectors_escape_quotes_and_backslashes() {
        assert_eq!(selector(&Selector::id("row-1")), r#"id="row-1""#);
        assert_eq!(
            selector(&Selector::text(r#"say "hi" \o/"#)),
            r#"text="say \"hi\" \\o/""#
        );
    }

    #[test]
    fn a_simulator_is_named_by_udid_and_an_android_target_by_serial() {
        let file = Path::new("/work/steps.json");
        let strings = |args: Vec<OsString>| -> Vec<String> {
            args.into_iter().map(|a| a.into_string().unwrap()).collect()
        };
        assert_eq!(
            strings(args(
                &Target::new("SIM-1", TargetClass::Simulator),
                "mobile-agent-bench",
                file
            )),
            [
                "batch",
                "--steps-file",
                "/work/steps.json",
                "--on-error",
                "stop",
                "--json",
                "--platform",
                "ios",
                "--udid",
                "SIM-1",
                "--session",
                "mobile-agent-bench"
            ]
        );
        let android = strings(args(
            &Target::new("R5CT", TargetClass::AndroidDevice),
            "s",
            file,
        ));
        assert_eq!(
            &android[6..10],
            ["--platform", "android", "--serial", "R5CT"]
        );
    }

    #[test]
    fn a_close_ends_the_session_on_the_target_it_ran_on_and_never_shuts_it_down() {
        let strings = |args: Vec<OsString>| -> Vec<String> {
            args.into_iter().map(|a| a.into_string().unwrap()).collect()
        };
        assert_eq!(
            strings(close_args(
                &Target::new("emulator-5554", TargetClass::AndroidEmulator),
                "mobile-agent-bench"
            )),
            [
                "close",
                "--json",
                "--platform",
                "android",
                "--serial",
                "emulator-5554",
                "--session",
                "mobile-agent-bench"
            ]
        );
        let ios = strings(close_args(
            &Target::new("SIM-1", TargetClass::Simulator),
            "s",
        ));
        assert_eq!(&ios[2..6], ["--platform", "ios", "--udid", "SIM-1"]);
        assert!(!ios.iter().any(|arg| arg == "--shutdown"));
    }

    #[test]
    fn a_close_reads_whether_it_ended_a_session() {
        assert_eq!(
            closed("{\"success\": true, \"data\": {\"session\": \"mobile-agent-bench\"}}\n"),
            Ok(true)
        );
        assert_eq!(
            closed(
                r#"{"success": false, "error": {"code": "SESSION_NOT_FOUND", "message": "no session mobile-agent-bench"}}"#
            ),
            Ok(false)
        );
        assert_eq!(
            closed(
                r#"{"success": false, "error": {"code": "COMMAND_FAILED", "message": "Android test IME could not be restored on emulator-5554."}}"#
            ),
            Err("COMMAND_FAILED: Android test IME could not be restored on emulator-5554.".into())
        );
        assert!(closed("daemon unavailable\n").is_err());
    }

    #[test]
    fn a_successful_reply_notes_the_reported_duration() {
        let reply = r#"{"success": true, "data": {"total": 4, "executed": 4, "totalDurationMs": 1810, "results": []}}"#;
        assert_eq!(
            outcome(reply),
            Ok(Some("agent-device reported 1810 ms".into()))
        );
        assert_eq!(outcome(r#"{"success": true, "data": {}}"#), Ok(None));
    }

    #[test]
    fn a_failed_reply_names_the_step_and_its_error() {
        let reply = r#"{"success": false,
            "error": {"code": "ELEMENT_NOT_FOUND", "message": "no element matches id=\"row-1\""},
            "data": {"results": [{"step": 1, "command": "open", "ok": true},
                                 {"step": 2, "command": "press", "ok": false}]}}"#;
        assert_eq!(
            outcome(reply),
            Err(
                r#"step 2 (press) failed: ELEMENT_NOT_FOUND: no element matches id="row-1""#.into()
            )
        );
        assert_eq!(
            outcome(r#"{"success": false, "error": "daemon unavailable"}"#),
            Err("daemon unavailable".into())
        );
    }

    #[test]
    fn a_snapshot_names_the_session_s_target() {
        let strings = |args: Vec<OsString>| -> Vec<String> {
            args.into_iter().map(|a| a.into_string().unwrap()).collect()
        };
        assert_eq!(
            strings(snapshot_args(
                &Target::new("emulator-5554", TargetClass::AndroidEmulator),
                "mobile-agent-bench"
            )),
            [
                "snapshot",
                "--json",
                "--platform",
                "android",
                "--serial",
                "emulator-5554",
                "--session",
                "mobile-agent-bench"
            ]
        );
        let ios = strings(snapshot_args(
            &Target::new("SIM-1", TargetClass::Simulator),
            "s",
        ));
        assert_eq!(&ios[2..6], ["--platform", "ios", "--udid", "SIM-1"]);
    }

    #[test]
    fn a_snapshot_gives_identifiers_values_or_labels_and_its_node_count() {
        let reply = r#"Starting daemon...
{
  "success": true,
  "data": {
    "nodes": [
      {"index": 0, "type": "Application", "label": "FixtureExpo", "depth": 0, "ref": "e1"},
      {"index": 1, "type": "Other", "depth": 1, "parentIndex": 0, "ref": "e2"},
      {"index": 2, "type": "StaticText", "label": "Welcome, agent", "identifier": "login-status", "depth": 2, "parentIndex": 1, "ref": "e3"},
      {"index": 3, "type": "TextField", "label": "Username", "value": "agent", "identifier": "login-username", "depth": 2, "parentIndex": 1, "ref": "e4"},
      {"index": 4, "type": "Button", "label": "A label twenty-four long", "identifier": "an-identifier-this-long-too", "depth": 2, "parentIndex": 1, "ref": "e5"},
      {"index": 5, "type": "StaticText", "inheritsLabel": true, "inheritsIdentifier": true, "depth": 3, "parentIndex": 4, "ref": "e6"}
    ],
    "truncated": false,
    "appName": "FixtureExpo"
  }
}"#;
        let element =
            |id: Option<&str>, text: Option<&str>| crate::end_state::Element::new(id, text);
        assert_eq!(
            read_snapshot(reply),
            Ok(Reading {
                elements: vec![
                    element(None, Some("FixtureExpo")),
                    element(None, None),
                    element(Some("login-status"), Some("Welcome, agent")),
                    element(Some("login-username"), Some("agent")),
                    element(
                        Some("an-identifier-this-long-too"),
                        Some("A label twenty-four long")
                    ),
                    element(
                        Some("an-identifier-this-long-too"),
                        Some("A label twenty-four long")
                    ),
                ],
                nodes: 6,
            })
        );
    }

    #[test]
    fn a_failed_snapshot_is_an_error_with_its_code() {
        assert_eq!(
            read_snapshot(
                r#"{"success": false, "error": {"code": "SESSION_NOT_FOUND", "message": "no session mobile-agent-bench"}}"#
            ),
            Err("SESSION_NOT_FOUND: no session mobile-agent-bench".into())
        );
        let error = read_snapshot(r#"{"success": true, "data": {}}"#).unwrap_err();
        assert!(error.contains("nodes"), "{error}");
        let error = read_snapshot(
            "daemon unavailable
",
        )
        .unwrap_err();
        assert!(error.contains("no JSON reply"), "{error}");
    }

    #[test]
    fn output_without_a_reply_is_a_failure() {
        let error = outcome("Starting daemon...\n").unwrap_err();
        assert!(error.contains("no JSON reply"), "{error}");
        // Progress lines before the reply are skipped.
        assert_eq!(
            outcome(
                "Starting daemon...\n{\"success\": true, \"data\": {\"totalDurationMs\": 5}}\n"
            ),
            Ok(Some("agent-device reported 5 ms".into()))
        );
    }
}
