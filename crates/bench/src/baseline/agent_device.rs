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
//! down, which the harness never asks for. The input method's app stays in
//! the guest as a cached process, which no command of agent-device ends, so
//! the release force-stops it with `adb` ([`HELPERS`]).
//!
//! `close` leaves agent-device's daemon running until five idle minutes
//! have passed, and on a Simulator its XCTest runner and its snapshot bridge
//! with it (`crates/bench/README.md`, Hand-offs between tools). So the
//! release then runs `agent-device daemon stop --clean --json`, which stops
//! the daemon of the state directory the bench's commands use and kills the
//! runner the daemon leaves running for its successor, and [`left_running`]
//! finds whichever of them the host still runs.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use serde_json::{Value, json};

use super::{json_document, last_line};
use crate::adapter::Left;
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

/// Arguments for `agent-device daemon stop` of the daemon of the state
/// directory the bench's commands use, as JSON. `--clean` also kills the
/// runner the daemon leaves running, with a lease, for the next daemon.
pub fn daemon_stop_args() -> Vec<OsString> {
    ["daemon", "stop", "--clean", "--json"]
        .into_iter()
        .map(OsString::from)
        .collect()
}

/// Reads the reply of `agent-device daemon stop`: whether it stopped a
/// daemon (`data.stopped`), else the failure.
pub fn stopped(stdout: &str) -> Result<bool, String> {
    let reply = json_document(stdout).ok_or_else(|| no_reply(stdout))?;
    if reply["success"] != true {
        return Err(error_message(&reply)
            .unwrap_or_else(|| "agent-device reported a failure without an error".to_owned()));
    }
    reply["data"]["stopped"]
        .as_bool()
        .ok_or_else(|| "agent-device's reply does not say whether it stopped a daemon".to_owned())
}

/// Arguments for `agent-device session state-dir`, which prints the state
/// directory of the bench's commands without starting a daemon, as JSON.
pub fn state_dir_args() -> Vec<OsString> {
    ["session", "state-dir", "--json"]
        .into_iter()
        .map(OsString::from)
        .collect()
}

/// Reads the reply of `agent-device session state-dir`: `data.stateDir`.
pub fn state_dir(stdout: &str) -> Result<String, String> {
    let reply = json_document(stdout).ok_or_else(|| no_reply(stdout))?;
    if reply["success"] != true {
        return Err(error_message(&reply)
            .unwrap_or_else(|| "agent-device reported a failure without an error".to_owned()));
    }
    reply["data"]["stateDir"]
        .as_str()
        .filter(|dir| !dir.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "agent-device's reply names no state directory".to_owned())
}

/// The arguments of `ps` that list the host's processes, one per line, as
/// the pid and the program's path, which may hold spaces.
pub const PS_PROGRAMS: [&str; 3] = ["-A", "-o", "pid=,comm="];

/// The arguments of `ps` that list the processes `pids` names, comma
/// separated, one per line, as the pid, the whole command line and then the
/// process's environment.
pub fn ps_details(pids: &str) -> [&str; 6] {
    ["-E", "-ww", "-o", "pid=,command=", "-p", pids]
}

/// What agent-device still runs on the host for `target`, one entry per
/// process, named with its pid:
///
/// - its daemon: a `node` running `…/dist/src/internal/daemon.js`, the path
///   agent-device itself checks a daemon's command line for, whose
///   environment names `state_dir` as `AGENT_DEVICE_STATE_DIR`, which
///   agent-device sets for each daemon it starts. Once the release has run
///   `daemon stop`, which returns only after the daemon `daemon.json` names
///   has exited, a daemon still running is one the stop did not reach, a
///   stray;
///
/// and for a Simulator, whose UDID is the target's id:
///
/// - its XCTest runner: the `xcodebuild` building or running
///   `AgentDeviceRunner` for the Simulator, and the runner app under the
///   Simulator's directory;
/// - its snapshot bridge: the `simctl spawn <udid>` that starts it, and the
///   `snapshot-bridge serve` in the Simulator, whose environment names the
///   Simulator as `SIMULATOR_UDID`.
///
/// On Android the daemon reaches the device through `adb` commands, the
/// longest of which, the snapshot helper's `am instrument -w`, ends with
/// the session's `close`, so the daemon is all there is to look for.
///
/// `programs` is the host's processes as `ps` lists them with
/// [`PS_PROGRAMS`]. `details` lists the processes whose pids it is given as
/// `ps` does with [`ps_details`]; it is asked only about the processes whose
/// program is one of those, and a process missing from what it gives has
/// ended.
pub fn left_running(
    programs: &str,
    target: &Target,
    state_dir: &str,
    details: impl FnOnce(&[u32]) -> String,
) -> Vec<Left> {
    let udid = target.id.as_str();
    let simulator = target.class.platform() == Platform::Ios;
    let devices = format!("/Devices/{udid}/");
    let mut left: Vec<(u32, &str)> = Vec::new();
    // The processes whose command line or environment tells whether they
    // are agent-device's for this target and state directory.
    let mut unsure: Vec<(u32, &str)> = Vec::new();
    for (pid, program) in programs.lines().filter_map(pid_and_rest) {
        match program.rsplit('/').next().unwrap_or(program) {
            "AgentDeviceRunnerUITests-Runner" if simulator && program.contains(&devices) => {
                left.push((pid, "runner app"));
            }
            "node" => unsure.push((pid, "node")),
            name @ ("xcodebuild" | "simctl" | "snapshot-bridge") if simulator => {
                unsure.push((pid, name));
            }
            _ => {}
        }
    }
    if !unsure.is_empty() {
        let pids: Vec<u32> = unsure.iter().map(|(pid, _)| *pid).collect();
        let details = details(&pids);
        let lines: Vec<(u32, &str)> = details.lines().filter_map(pid_and_rest).collect();
        let spawn = format!(" spawn {udid} ");
        for (pid, program) in unsure {
            let Some((_, line)) = lines.iter().find(|(listed, _)| *listed == pid) else {
                continue;
            };
            let named = match program {
                "node" => (ends_word(line, "/dist/src/internal/daemon.js")
                    && sets(line, "AGENT_DEVICE_STATE_DIR", state_dir))
                .then_some("daemon"),
                "xcodebuild" => (line.contains("AgentDeviceRunner") && line.contains(udid))
                    .then_some("runner's xcodebuild"),
                "simctl" => (line.contains(&spawn) && line.contains("/snapshot-bridge serve "))
                    .then_some("bridge's simctl spawn"),
                _ => (line.contains("/snapshot-bridge serve ")
                    && sets(line, "SIMULATOR_UDID", udid))
                .then_some("bridge"),
            };
            if let Some(name) = named {
                left.push((pid, name));
            }
        }
    }
    left.sort_by_key(|(pid, _)| *pid);
    left.into_iter()
        .map(|(pid, name)| Left {
            name: name.to_owned(),
            pid,
            stray: name == "daemon",
        })
        .collect()
}

/// A line `ps` printed for `pid=,…`, as the pid and the rest.
fn pid_and_rest(line: &str) -> Option<(u32, &str)> {
    let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
    Some((pid.parse().ok()?, rest.trim_start()))
}

/// Whether `line` holds `suffix` at the end of a word.
fn ends_word(line: &str, suffix: &str) -> bool {
    line.match_indices(suffix).any(|(at, _)| {
        line[at + suffix.len()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
    })
}

/// Whether `line`, a command and its environment as `ps -E` prints them,
/// sets `variable` to `value`.
fn sets(line: &str, variable: &str, value: &str) -> bool {
    ends_word(line, &format!(" {variable}={value}"))
}

/// agent-device's helper apps on an Android device, by package, and what
/// each is: the input method it selects for text entry, which stays in the
/// guest as a cached process after `close` and `daemon stop`, and the
/// snapshot helper, whose instrumentation `close` ends but a daemon killed
/// without its session closed leaves running.
pub const HELPERS: [(&str, &str); 2] = [
    ("com.callstack.agentdevice.imehelper", "input-method helper"),
    (
        "com.callstack.agentdevice.snapshothelper",
        "snapshot helper",
    ),
];

/// The arguments of `adb` that list the processes of the Android device
/// `serial`, one per line after a header, as the pid and the process's name.
pub fn guest_ps_args(serial: &str) -> [&str; 7] {
    ["-s", serial, "shell", "ps", "-A", "-o", "PID,NAME"]
}

/// The arguments of `adb` that force-stop `package` on the Android device
/// `serial`, which ends its processes and prints nothing.
pub fn force_stop_args<'a>(serial: &'a str, package: &'a str) -> [&'a str; 6] {
    ["-s", serial, "shell", "am", "force-stop", package]
}

/// The [`HELPERS`] that have a process in `listing`, the guest's processes
/// as `adb` lists them with [`guest_ps_args`], in the order of
/// [`HELPERS`]. A process is a package's when it is named after it, alone
/// or followed by `:` and the name of one of its processes.
pub fn running_helpers(listing: &str) -> Vec<(&'static str, &'static str)> {
    let names: Vec<&str> = listing
        .lines()
        .filter_map(pid_and_rest)
        .map(|(_, name)| name.trim_end())
        .collect();
    HELPERS
        .into_iter()
        .filter(|(package, _)| {
            names.iter().any(|name| {
                name.strip_prefix(package)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
            })
        })
        .collect()
}

/// What the release did, from whether `close` ended a session, whether
/// `daemon stop` stopped a daemon, and which helper apps it force-stopped on
/// an Android device; `None` when it did none of these.
pub fn released(closed: bool, stopped: bool, helpers: &[&str], session: &str) -> Option<String> {
    let mut done = Vec::new();
    if closed {
        done.push(format!("closed agent-device session {session}"));
    }
    // The first thing done names agent-device, the others say "its".
    let whose = |done: &Vec<String>| {
        if done.is_empty() {
            "agent-device's"
        } else {
            "its"
        }
    };
    if stopped {
        done.push(format!("stopped {} daemon", whose(&done)));
    }
    if !helpers.is_empty() {
        done.push(format!(
            "force-stopped {} {}",
            whose(&done),
            helpers.join(" and ")
        ));
    }
    let (last, rest) = done.split_last()?;
    Some(if rest.is_empty() {
        last.clone()
    } else {
        format!("{} and {last}", rest.join(", "))
    })
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
    use std::cell::RefCell;

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
    fn a_daemon_stop_also_kills_the_runner_the_daemon_leaves_and_names_no_target() {
        let args: Vec<String> = daemon_stop_args()
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(args, ["daemon", "stop", "--clean", "--json"]);
        let args: Vec<String> = state_dir_args()
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(args, ["session", "state-dir", "--json"]);
    }

    #[test]
    fn a_daemon_stop_reads_whether_it_stopped_a_daemon() {
        assert_eq!(
            stopped(
                r#"{"success": true, "data": {"stopped": true, "mode": "graceful", "cleanupConfidence": "known", "warnings": [], "clean": true}}"#
            ),
            Ok(true)
        );
        assert_eq!(
            stopped(
                r#"{"success": true, "data": {"stopped": false, "mode": "not-running", "clean": false}}"#
            ),
            Ok(false)
        );
        assert_eq!(
            stopped(
                r#"{"success": false, "error": {"code": "COMMAND_FAILED", "message": "Daemon did not exit after SIGKILL."}}"#
            ),
            Err("COMMAND_FAILED: Daemon did not exit after SIGKILL.".into())
        );
        let error = stopped(r#"{"success": true, "data": {"total": 3}}"#).unwrap_err();
        assert!(error.contains("stopped a daemon"), "{error}");
        assert!(stopped("Daemon stopped (graceful).\n").is_err());
    }

    #[test]
    fn the_state_directory_is_read_from_its_reply() {
        assert_eq!(
            state_dir(
                "{\n  \"success\": true,\n  \"data\": {\n    \"stateDir\": \"/tmp/ad\"\n  }\n}\n"
            ),
            Ok("/tmp/ad".into())
        );
        assert!(state_dir(r#"{"success": true, "data": {}}"#).is_err());
        assert!(state_dir(r#"{"success": false, "error": "unknown command"}"#).is_err());
    }

    /// The host's processes as `ps -A -o pid=,comm=` lists them while
    /// agent-device drives `SIM-1` from the state directory `/tmp/ad`, with
    /// processes that are not its own for them: another state directory's
    /// daemon, another Simulator's runner and bridge, another `node`, and a
    /// shell whose command line names everything agent-device's do.
    const PROGRAMS: &str = "    1 /sbin/launchd
  401 /opt/homebrew/bin/node
  402 /Applications/Xcode.app/Contents/Developer/usr/bin/xcodebuild
  403 /Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/Resources/bin/simctl
  404 /Users/<user>/.agent-device/snapshot-source/1/snapshot-bridge
  405 /Users/<user>/Library/Developer/CoreSimulator/Devices/SIM-1/data/Containers/Bundle/Application/1/AgentDeviceRunnerUITests-Runner.app/AgentDeviceRunnerUITests-Runner
  501 /opt/homebrew/bin/node
  502 /Applications/Xcode 27.app/Contents/Developer/usr/bin/xcodebuild
  503 /Users/<user>/.agent-device/snapshot-source/1/snapshot-bridge
  504 /bin/zsh
  505 /Users/<user>/Library/Developer/CoreSimulator/Devices/SIM-2/data/Containers/Bundle/Application/2/AgentDeviceRunnerUITests-Runner.app/AgentDeviceRunnerUITests-Runner
  506 node";

    /// What `ps -E -ww -o pid=,command= -p <pids>` prints for the processes
    /// of [`PROGRAMS`] whose program may be agent-device's, and the shell.
    const DETAILS: &str = "  401 /opt/homebrew/bin/node /opt/homebrew/lib/node_modules/agent-device/dist/src/internal/daemon.js AGENT_DEVICE_STATE_DIR=/tmp/ad AGENT_DEVICE_DAEMON_SERVER_MODE=socket
  402 /Applications/Xcode.app/Contents/Developer/usr/bin/xcodebuild test-without-building -only-testing AgentDeviceRunnerUITests/RunnerTests/testCommand -parallel-testing-enabled NO -xctestrun /Users/<user>/.agent-device/apple-runner/derived/ios-simulator/cache-1/Build/Products/AgentDeviceRunner.env.session-SIM-1-owner-401-1-1.xctestrun -destination platform=iOS Simulator,id=SIM-1 AGENT_DEVICE_STATE_DIR=/tmp/ad
  403 /Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/Resources/bin/simctl spawn SIM-1 /Users/<user>/.agent-device/snapshot-source/1/snapshot-bridge serve /tmp/agent-device-ax-1-401-1/snapshot.sock --idle-timeout 60 --exit-on-disconnect false AGENT_DEVICE_STATE_DIR=/tmp/ad
  404 /Users/<user>/.agent-device/snapshot-source/1/snapshot-bridge serve /tmp/agent-device-ax-1-401-1/snapshot.sock --idle-timeout 60 --exit-on-disconnect false SIMULATOR_DEVICE_NAME=bench SIMULATOR_UDID=SIM-1
  501 /opt/homebrew/bin/node /opt/homebrew/lib/node_modules/agent-device/dist/src/internal/daemon.js AGENT_DEVICE_STATE_DIR=/tmp/ad-other AGENT_DEVICE_DAEMON_SERVER_MODE=socket
  502 /Applications/Xcode 27.app/Contents/Developer/usr/bin/xcodebuild test-without-building -only-testing AgentDeviceRunnerUITests/RunnerTests/testCommand -destination platform=iOS Simulator,id=SIM-2 AGENT_DEVICE_STATE_DIR=/tmp/ad-other
  503 /Users/<user>/.agent-device/snapshot-source/1/snapshot-bridge serve /tmp/agent-device-ax-2-501-1/snapshot.sock --idle-timeout 60 --exit-on-disconnect false SIMULATOR_UDID=SIM-2
  504 /bin/zsh -c grep -e /dist/src/internal/daemon.js -e AgentDeviceRunner -e SIM-1 -e '/snapshot-bridge serve ' AGENT_DEVICE_STATE_DIR=/tmp/ad SIMULATOR_UDID=SIM-1
  506 node /usr/local/lib/node_modules/typescript/lib/tsserver.js --serverMode partialSemantic AGENT_DEVICE_STATE_DIR=/tmp/ad";

    fn simulator(udid: &str) -> Target {
        Target::new(udid, TargetClass::Simulator)
    }

    /// Each process left, as the hand-off names it.
    fn named(left: &[Left]) -> Vec<String> {
        left.iter().map(Left::to_string).collect()
    }

    #[test]
    fn what_is_left_running_is_the_daemon_runner_and_bridge_of_the_state_directory_and_simulator() {
        let asked = RefCell::new(Vec::new());
        let left = left_running(PROGRAMS, &simulator("SIM-1"), "/tmp/ad", |pids| {
            asked.borrow_mut().extend_from_slice(pids);
            DETAILS.to_owned()
        });
        assert_eq!(
            named(&left),
            [
                "daemon (pid 401)",
                "runner's xcodebuild (pid 402)",
                "bridge's simctl spawn (pid 403)",
                "bridge (pid 404)",
                "runner app (pid 405)"
            ]
        );
        // The daemon is a stray, which the release's stop should have ended;
        // the runner and the bridge end after it.
        let strays: Vec<u32> = left.iter().filter(|l| l.stray).map(|l| l.pid).collect();
        assert_eq!(strays, [401]);
        // Only the processes whose program may be agent-device's are looked
        // at closer; the shell that names them all is not one of them.
        assert_eq!(asked.into_inner(), [401, 402, 403, 404, 501, 502, 503, 506]);
        // Another state directory's daemon and another Simulator's runner
        // and bridge are not this run's.
        assert_eq!(
            named(&left_running(
                PROGRAMS,
                &simulator("SIM-2"),
                "/tmp/ad-other",
                |_| DETAILS.to_owned()
            )),
            [
                "daemon (pid 501)",
                "runner's xcodebuild (pid 502)",
                "bridge (pid 503)",
                "runner app (pid 505)"
            ]
        );
    }

    #[test]
    fn what_is_left_running_beside_an_emulator_is_the_daemon_of_the_state_directory() {
        // On Android agent-device runs neither a runner nor a bridge on the
        // host, so only its daemon is looked for: a `node` process.
        let asked = RefCell::new(Vec::new());
        let emulator = Target::new("emulator-5554", TargetClass::AndroidEmulator);
        let left = left_running(PROGRAMS, &emulator, "/tmp/ad", |pids| {
            asked.borrow_mut().extend_from_slice(pids);
            DETAILS.to_owned()
        });
        assert_eq!(named(&left), ["daemon (pid 401)"]);
        assert!(left[0].stray);
        assert_eq!(asked.into_inner(), [401, 501, 506]);
    }

    #[test]
    fn processes_that_end_before_they_are_looked_at_closer_are_not_left() {
        // `ps -p` lists none of the pids it was asked about once they have
        // ended; the runner app is told by its program alone.
        let left = left_running(PROGRAMS, &simulator("SIM-1"), "/tmp/ad", |_| String::new());
        assert_eq!(named(&left), ["runner app (pid 405)"]);
        let none = left_running(
            "    1 /sbin/launchd\n  504 /bin/zsh",
            &simulator("SIM-1"),
            "/tmp/ad",
            |_| panic!("no process needed a closer look"),
        );
        assert!(none.is_empty());
    }

    #[test]
    fn a_release_says_what_it_ended() {
        let s = "mobile-agent-bench";
        assert_eq!(
            released(true, true, &[], s).as_deref(),
            Some("closed agent-device session mobile-agent-bench and stopped its daemon")
        );
        assert_eq!(
            released(true, false, &[], s).as_deref(),
            Some("closed agent-device session mobile-agent-bench")
        );
        assert_eq!(
            released(false, true, &[], s).as_deref(),
            Some("stopped agent-device's daemon")
        );
        assert_eq!(released(false, false, &[], s), None);
        // On Android the release also force-stops the helpers it finds.
        let ime = "input-method helper";
        assert_eq!(
            released(true, true, &[ime], s).as_deref(),
            Some(
                "closed agent-device session mobile-agent-bench, stopped its daemon and \
                 force-stopped its input-method helper"
            )
        );
        assert_eq!(
            released(false, false, &[ime, "snapshot helper"], s).as_deref(),
            Some("force-stopped agent-device's input-method helper and snapshot helper")
        );
        assert_eq!(
            released(false, true, &[ime], s).as_deref(),
            Some("stopped agent-device's daemon and force-stopped its input-method helper")
        );
    }

    #[test]
    fn the_helpers_running_in_a_guest_are_read_from_its_process_listing() {
        let listing = "  PID NAME
    1 init
30694 com.callstack.agentdevice.imehelper
30737 app_process
30749 com.callstack.agentdevice.snapshothelper:remote
31002 com.callstack.agentdevice.imehelper.other
";
        assert_eq!(
            running_helpers(listing),
            [
                ("com.callstack.agentdevice.imehelper", "input-method helper"),
                (
                    "com.callstack.agentdevice.snapshothelper",
                    "snapshot helper"
                )
            ]
        );
        assert!(running_helpers("  PID NAME\n    1 init\n").is_empty());
        assert!(running_helpers("").is_empty());
    }

    #[test]
    fn a_helper_is_force_stopped_on_the_device_it_runs_on() {
        assert_eq!(
            guest_ps_args("emulator-5554"),
            ["-s", "emulator-5554", "shell", "ps", "-A", "-o", "PID,NAME"]
        );
        assert_eq!(
            force_stop_args("emulator-5554", "com.callstack.agentdevice.imehelper"),
            [
                "-s",
                "emulator-5554",
                "shell",
                "am",
                "force-stop",
                "com.callstack.agentdevice.imehelper"
            ]
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
