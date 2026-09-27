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

use std::ffi::OsString;
use std::path::Path;

use serde_json::{Value, json};

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
    let device_flag = match platform {
        Platform::Ios => "--udid",
        Platform::Android => "--serial",
    };
    vec![
        "batch".into(),
        "--steps-file".into(),
        file.as_os_str().to_owned(),
        "--on-error".into(),
        "stop".into(),
        "--json".into(),
        "--platform".into(),
        platform.id().into(),
        device_flag.into(),
        target.id.clone().into(),
        "--session".into(),
        session.into(),
    ]
}

/// Reads the batch's JSON reply. Success gives a note of the duration
/// agent-device reported; failure gives the failing step and its error.
pub fn outcome(stdout: &str) -> Result<Option<String>, String> {
    let Some(reply) = reply(stdout) else {
        let last = stdout.lines().map(str::trim).rfind(|l| !l.is_empty());
        return Err(format!(
            "agent-device printed no JSON reply{}",
            last.map(|l| format!(": {l}")).unwrap_or_default()
        ));
    };
    if reply["success"] == true {
        return Ok(reply["data"]["totalDurationMs"]
            .as_u64()
            .map(|ms| format!("agent-device reported {ms} ms")));
    }
    let error = &reply["error"];
    let message = error.as_str().map(str::to_owned).or_else(|| {
        let message = error["message"].as_str()?;
        Some(match error["code"].as_str() {
            Some(code) => format!("{code}: {message}"),
            None => message.to_owned(),
        })
    });
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

/// The JSON object on stdout: all of it, or else its last line that is one.
fn reply(stdout: &str) -> Option<Value> {
    let object = |text: &str| {
        serde_json::from_str::<Value>(text.trim())
            .ok()
            .filter(Value::is_object)
    };
    object(stdout).or_else(|| stdout.lines().rev().find_map(object))
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
