//! Printing replies as agent-browser does: in JSON mode one line on stdout,
//! `{"success", "data", "error"}` plus `code` and `warning` when set; in
//! human mode the listing or a short confirmation on stdout, and errors and
//! warnings on stderr marked ✗ and ⚠.

use std::fmt::Write as _;

use serde_json::Value;

use crate::client::Reply;

/// The reply as one JSON line, fields in agent-browser's order.
pub fn json_line(reply: &Reply) -> String {
    let text = |value: &Option<String>| {
        value
            .as_deref()
            .map_or_else(|| "null".to_owned(), |s| Value::from(s).to_string())
    };
    let mut line = format!(
        "{{\"success\":{},\"data\":{},\"error\":{}",
        reply.success,
        reply.data,
        text(&reply.error)
    );
    if reply.code.is_some() {
        let _ = write!(line, ",\"code\":{}", text(&reply.code));
    }
    if reply.warning.is_some() {
        let _ = write!(line, ",\"warning\":{}", text(&reply.warning));
    }
    line.push('}');
    line
}

/// What human mode prints for a reply: standard output and standard error.
pub fn human(action: &str, reply: &Reply) -> (String, String) {
    let mut out = String::new();
    let mut err = String::new();
    if !reply.success {
        let _ = writeln!(
            err,
            "✗ {}",
            reply.error.as_deref().unwrap_or("unknown error")
        );
    } else {
        human_success(action, &reply.data, &mut out);
    }
    if let Some(warning) = &reply.warning {
        let _ = writeln!(err, "⚠ {warning}");
    }
    (out, err)
}

fn str_of<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn number(value: &Value) -> String {
    match value.as_f64() {
        Some(n) => format!("{}", (n * 100.0).round() / 100.0),
        None => value.to_string(),
    }
}

fn human_success(action: &str, data: &Value, out: &mut String) {
    match action {
        "snapshot" => snapshot(data, out),
        "devices" => devices(data, out),
        "status" => status(data, out),
        "diff_snapshot" => diff(data, out),
        "open" => {
            let _ = writeln!(
                out,
                "✓ Opened {} on {}",
                str_of(data, "opened"),
                str_of(data, "target")
            );
        }
        "close" => match data.get("app").and_then(Value::as_str) {
            Some(app) => {
                let _ = writeln!(
                    out,
                    "✓ Closed {app} and released {}",
                    str_of(data, "target")
                );
            }
            None => {
                let _ = writeln!(out, "✓ Released {}", str_of(data, "target"));
            }
        },
        "get" => {
            if let Some(text) = data.get("text").and_then(Value::as_str) {
                let _ = writeln!(out, "{text}");
            } else if data.get("value").is_some() {
                let _ = writeln!(out, "{}", str_of(data, "value"));
            } else {
                for (label, key) in [
                    ("x:     ", "x"),
                    ("y:     ", "y"),
                    ("width: ", "width"),
                    ("height:", "height"),
                ] {
                    let _ = writeln!(out, "{label} {}", number(&data[key]));
                }
            }
        }
        "screenshot" => {
            let _ = writeln!(out, "✓ Screenshot saved to {}", str_of(data, "path"));
        }
        "wait" if str_of(data, "waited") == "settle" => settle(data, out),
        "click" => click(data, out),
        "shutdown" => {
            let _ = writeln!(out, "✓ Daemon stopped");
        }
        _ => {
            let _ = writeln!(out, "✓ Done");
        }
    }
}

/// A click's spans: `✓ Done (dispatch 0.2 ms, first frame change 121.3 ms,
/// first tree delta 121.9 ms)`, leaving out the spans that never came.
fn click(data: &Value, out: &mut String) {
    let span = |key: &str| data[key].as_f64().map(|ms| format!("{ms:.1} ms"));
    let mut parts = Vec::new();
    if let Some(dispatch) = span("dispatchMs") {
        parts.push(format!("dispatch {dispatch}"));
    }
    let frame = span("frameChangeMs");
    let tree = span("treeDeltaMs");
    if let Some(frame) = &frame {
        parts.push(format!("first frame change {frame}"));
    }
    if let Some(tree) = &tree {
        parts.push(format!("first tree delta {tree}"));
    }
    if frame.is_none()
        && tree.is_none()
        && let Some(watched) = data["watchedMs"].as_f64()
    {
        parts.push(format!("no change in {watched:.0} ms"));
    }
    if parts.is_empty() {
        let _ = writeln!(out, "✓ Done");
    } else {
        let _ = writeln!(out, "✓ Done ({})", parts.join(", "));
    }
}

/// A settle wait and the signal it ended on.
fn settle(data: &Value, out: &mut String) {
    let elapsed = number(&data["elapsedMs"]);
    if data["settled"] != Value::Bool(true) {
        let still: Vec<&str> = data["stillChanging"]
            .as_array()
            .map(|s| s.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let what = match still.as_slice() {
            ["tree"] => "the tree kept changing",
            ["frame"] => "frames kept changing",
            [_, _] => "the tree and frames kept changing",
            _ => "the screen kept changing",
        };
        let _ = writeln!(out, "✓ Stopped waiting after {elapsed} ms: {what}");
        return;
    }
    let ended = match str_of(data, "endedBy") {
        "tree" => ": the tree changed last",
        "frame" => ": frames changed last",
        "both" => ": the tree and frames changed last",
        "none" => ": nothing changed",
        _ => "",
    };
    let _ = writeln!(out, "✓ Settled after {elapsed} ms{ended}");
}

fn snapshot(data: &Value, out: &mut String) {
    match &data["snapshot"] {
        Value::String(listing) => {
            let _ = writeln!(out, "{listing}");
        }
        Value::Object(delta) => match delta.get("kind").and_then(Value::as_str) {
            Some("full") => {
                let _ = writeln!(
                    out,
                    "{}",
                    delta
                        .get("tree")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                );
            }
            Some("unchanged") => {
                let revision = delta.get("revision").and_then(Value::as_u64).unwrap_or(0);
                let _ = writeln!(out, "unchanged (revision {revision})");
            }
            _ => {
                let pretty = serde_json::to_string_pretty(delta).unwrap_or_default();
                let _ = writeln!(out, "{pretty}");
            }
        },
        other => {
            let _ = writeln!(out, "{other}");
        }
    }
}

/// Rows as columns separated by two spaces.
fn table(rows: &[Vec<String>], out: &mut String) {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in rows {
        let mut line = String::new();
        for (c, cell) in row.iter().enumerate() {
            if c + 1 == row.len() {
                line.push_str(cell);
            } else {
                let _ = write!(line, "{cell:width$}  ", width = widths[c]);
            }
        }
        let _ = writeln!(out, "{}", line.trim_end());
    }
}

fn devices(data: &Value, out: &mut String) {
    let devices = data["devices"].as_array().cloned().unwrap_or_default();
    if devices.is_empty() {
        let _ = writeln!(out, "(no targets connected)");
        return;
    }
    let rows: Vec<Vec<String>> = devices
        .iter()
        .map(|d| {
            let mut row = vec![
                str_of(d, "id").to_owned(),
                str_of(d, "platform").to_owned(),
                str_of(d, "class").to_owned(),
                str_of(d, "name").to_owned(),
            ];
            if let Some(holder) = d.get("claimedBy").and_then(Value::as_str) {
                row.push(format!("claimed by {holder}"));
            }
            row
        })
        .collect();
    table(&rows, out);
}

fn status(data: &Value, out: &mut String) {
    let daemon = &data["daemon"];
    let idle = match daemon["idleTimeoutMs"].as_u64() {
        Some(ms) => format!("idle timeout {ms} ms"),
        None => "no idle timeout".to_owned(),
    };
    let _ = writeln!(
        out,
        "daemon: running, pid {}, version {}, up {} s, {idle}",
        daemon["pid"],
        str_of(daemon, "version"),
        daemon["uptimeMs"].as_u64().unwrap_or(0) / 1000,
    );
    let _ = writeln!(out, "socket: {}", str_of(daemon, "socket"));
    let targets = data["targets"].as_array().cloned().unwrap_or_default();
    if targets.is_empty() {
        let _ = writeln!(out, "targets: none connected");
    } else {
        let _ = writeln!(out, "targets:");
        let rows: Vec<Vec<String>> = targets
            .iter()
            .map(|t| {
                let mut state = vec![if t["attached"] == Value::Bool(true) {
                    "attached".to_owned()
                } else {
                    "not attached".to_owned()
                }];
                if let Some(holder) = t.get("claimedBy").and_then(Value::as_str) {
                    state.push(format!("claimed by {holder}"));
                }
                if let Some(app) = t.get("app").and_then(Value::as_str) {
                    state.push(format!("app {app}"));
                }
                vec![
                    format!("  {}", str_of(t, "id")),
                    str_of(t, "platform").to_owned(),
                    str_of(t, "name").to_owned(),
                    state.join(", "),
                ]
            })
            .collect();
        table(&rows, out);
    }
    if let Some(sessions) = data["sessions"].as_array().filter(|s| !s.is_empty()) {
        let _ = writeln!(out, "sessions:");
        for session in sessions {
            let held: Vec<&str> = session["targets"]
                .as_array()
                .map(|t| t.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "  {}: {}",
                str_of(session, "name"),
                if held.is_empty() {
                    "no target".to_owned()
                } else {
                    held.join(", ")
                }
            );
        }
    }
}

fn diff(data: &Value, out: &mut String) {
    if data["changed"] != Value::Bool(true) {
        let _ = writeln!(out, "✓ No changes detected");
        return;
    }
    let _ = writeln!(out, "{}", str_of(data, "diff"));
    let count = |key: &str| data[key].as_u64().unwrap_or(0);
    let _ = writeln!(
        out,
        "\n{} additions, {} removals, {} changes, {} unchanged",
        count("additions"),
        count("removals"),
        count("modifications"),
        count("unchanged")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_json_line_keeps_agent_browser_field_order_and_omits_unset_fields() {
        let ok = Reply {
            success: true,
            data: json!({"a": 1}),
            error: None,
            code: None,
            warning: None,
        };
        assert_eq!(
            json_line(&ok),
            r#"{"success":true,"data":{"a":1},"error":null}"#
        );
        let failed = Reply {
            success: false,
            data: Value::Null,
            error: Some("gone".into()),
            code: Some("stale_ref".into()),
            warning: Some("careful".into()),
        };
        assert_eq!(
            json_line(&failed),
            r#"{"success":false,"data":null,"error":"gone","code":"stale_ref","warning":"careful"}"#
        );
    }

    fn ok(data: Value) -> Reply {
        Reply {
            success: true,
            data,
            error: None,
            code: None,
            warning: None,
        }
    }

    #[test]
    fn devices_list_each_target_with_its_state_and_what_makes_it_usable() {
        let reply = ok(json!({"devices": [
            {"id": "7B292E1E", "platform": "ios", "class": "simulator", "name": "mobile-agent-spike",
             "state": "booted", "claimedBy": null},
            {"id": "R58M123", "platform": "android", "class": "android-device", "name": "R58M123",
             "state": "unauthorized", "advice": "accept the prompt on the device", "claimedBy": null},
        ]}));
        let (out, err) = human("devices", &reply);
        assert_eq!(err, "");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3, "{out}");
        assert!(
            lines[0].starts_with("7B292E1E") && lines[0].ends_with("mobile-agent-spike  booted"),
            "{out}"
        );
        assert!(lines[1].ends_with("R58M123  unauthorized"), "{out}");
        assert_eq!(lines[2], "    accept the prompt on the device");
    }

    #[test]
    fn status_prints_what_the_driver_reports_under_each_attached_target() {
        let reply = ok(json!({
            "daemon": {"pid": 1, "version": "0.1.0", "socket": "/s", "uptimeMs": 1000, "idleTimeoutMs": null},
            "targets": [
                {"id": "emulator-5554", "platform": "android", "name": "sdk_gphone64_arm64",
                 "state": "device", "attached": true, "app": null, "claimedBy": "default",
                 "driver": {"input": "agent", "lines": ["input: agent (taps, long presses and swipes)", "screenshots: agent"]}},
                {"id": "7B292E1E", "platform": "ios", "name": "mobile-agent-spike",
                 "state": "booted", "attached": false, "app": null, "claimedBy": null},
            ],
            "sessions": [],
        }));
        let (out, _) = human("status", &reply);
        let expected = "emulator-5554:\n    input: agent (taps, long presses and swipes)\n    screenshots: agent\n";
        assert!(out.contains(expected), "{out}");
        assert!(!out.contains("7B292E1E:"), "{out}");
        let row = out
            .lines()
            .find(|line| line.trim_start().starts_with("emulator-5554 "))
            .unwrap_or_else(|| panic!("no row in {out}"));
        assert!(
            row.ends_with("device, attached, claimed by default"),
            "{row}"
        );
    }

    #[test]
    fn human_errors_and_warnings_go_to_stderr() {
        let failed = Reply {
            success: false,
            data: Value::Null,
            error: Some("gone".into()),
            code: Some("stale_ref".into()),
            warning: None,
        };
        assert_eq!(
            human("click", &failed),
            (String::new(), "✗ gone\n".to_owned())
        );
    }
}
