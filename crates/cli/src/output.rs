//! Printing replies as agent-browser does: in JSON mode one line on stdout,
//! `{"success", "data", "error"}` plus `code` and `warning` when set; in
//! human mode the listing or a short confirmation on stdout, and errors and
//! warnings on stderr marked ✗ and ⚠.

use std::fmt::Write as _;

use serde_json::{Map, Value};

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
            Some(app) if data["kept"] == Value::Bool(true) => {
                let _ = writeln!(
                    out,
                    "✓ Released {}; {app} keeps running",
                    str_of(data, "target")
                );
            }
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
        "cancel" if data["cancelled"] == Value::Bool(true) => {
            let _ = writeln!(
                out,
                "✓ Cancelled the {} in flight on {}",
                str_of(data, "action"),
                str_of(data, "target")
            );
        }
        "cancel" => {
            let _ = writeln!(out, "✓ Nothing in flight to cancel");
        }
        "get" => {
            if let Some(text) = data.get("text").and_then(Value::as_str) {
                let _ = writeln!(out, "{text}");
            } else if data.get("value").is_some() {
                let _ = writeln!(out, "{}", value(data));
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
            // An element the tree holds out of view (rn-app-fixes, design
            // D51).
            if data["inView"] == Value::Bool(false) {
                let _ = writeln!(out, "out of view: scroll it into view before acting on it");
            }
            if let Some(resolved) = resolution(data) {
                let _ = writeln!(out, "({resolved})");
            }
        }
        "screenshot" => {
            let path = str_of(data, "path");
            match sizes(data) {
                Some(sizes) => {
                    let _ = writeln!(out, "✓ Screenshot saved to {path} ({sizes})");
                }
                None => {
                    let _ = writeln!(out, "✓ Screenshot saved to {path}");
                }
            }
        }
        "wait" if str_of(data, "waited") == "settle" => settle(data, out),
        "wait" if str_of(data, "waited") == "change" => change(data, out),
        "wait" if data["element"].is_object() => found(data, out),
        "click" => click(data, out),
        "fill" => fill(data, out),
        "type" => typed(data, out),
        "scroll" => scroll(data, out),
        "shutdown" => {
            let _ = writeln!(out, "✓ Daemon stopped");
        }
        _ => {
            let _ = writeln!(out, "✓ Done");
        }
    }
}

/// What `get value` prints (real-app-fixes, design D5): the value, else a
/// switch's or a checkbox's state, `on` or `off` as the listing's `checked`
/// has it, else `no value`.
fn value(data: &Value) -> &str {
    match (data["value"].as_str(), &data["checked"]) {
        (Some(value), _) if !value.is_empty() => value,
        (_, Value::Bool(true)) => "on",
        (_, Value::Bool(false)) => "off",
        _ => "no value",
    }
}

/// The twin a stale ref stood for (design D3 of action-verdicts), as an
/// action's line names it: `@e221 now @e240`; `None` for a reply without
/// one.
fn resolution(data: &Value) -> Option<String> {
    let resolved = data.get("resolved")?;
    let (stale, twin) = (resolved["stale"].as_str()?, resolved["ref"].as_str()?);
    Some(format!("{stale} now {twin}"))
}

/// A screenshot's image size beside the screen's, which a click takes
/// points of (real-app-fixes, design D5): `460x1000 of a 402x874-point
/// screen`, the points to a tenth; `None` for a reply without them.
pub fn sizes(data: &Value) -> Option<String> {
    let points = |key: &str| {
        data["points"][key]
            .as_f64()
            .map(|value| (value * 10.0).round() / 10.0)
    };
    let (width, height) = (data["width"].as_u64()?, data["height"].as_u64()?);
    let (across, down) = (points("width")?, points("height")?);
    Some(format!(
        "{width}x{height} of a {across}x{down}-point screen"
    ))
}

/// A wait that found its element, or an element with its text, named as
/// the listing names it, by its role, name and ref (real-app-fixes, design
/// D5): `✓ Found text "Welcome, agent" @e12`, followed by get's line for an
/// element out of view.
fn found(data: &Value, out: &mut String) {
    let element = &data["element"];
    let mut line = format!("✓ Found {}", str_of(element, "role"));
    let name = str_of(element, "name");
    if !name.is_empty() {
        let _ = write!(line, " {name:?}");
    }
    if let Some(reference) = element["ref"].as_str() {
        let _ = write!(line, " {reference}");
    }
    let _ = writeln!(out, "{line}");
    if data["inView"] == Value::Bool(false) {
        let _ = writeln!(out, "out of view: scroll it into view before acting on it");
    }
}

/// A fill's read-back and entry: `✓ Done (read back "agent", typed)`; the
/// count of characters where the field showed as many mask characters, a
/// secure field's; `nothing to read back` beside the reply's warning for a
/// field that showed nothing, and `not read back` for one the fill did not
/// read back. `typed` is text that went in as keys and `entered in one
/// piece` a paste or a set, `direct` as the reply names it; `retried` says
/// the text went in again. The twin a stale ref stood for comes first
/// ([`resolution`]).
fn fill(data: &Value, out: &mut String) {
    let text = str_of(data, "text");
    let read = match &data["verified"] {
        Value::String(shown) if shown == "text" => format!("read back {text:?}"),
        Value::String(shown) if shown == "length" => masked(text),
        Value::Bool(false) => "nothing to read back".to_owned(),
        _ => "not read back".to_owned(),
    };
    let mut parts: Vec<String> = resolution(data).into_iter().chain([read]).collect();
    entry(data, &mut parts);
    if data["retried"] == Value::Bool(true) {
        parts.push("retried".to_owned());
    }
    let _ = writeln!(out, "✓ Done ({})", parts.join(", "));
}

/// A type's read-back and entry, as fill's line says them (real-app-fixes,
/// design D3): `✓ Done (read back "example.com", typed)` with what the field
/// holds once the keys went in, also when it differs from the text typed,
/// which the reply's warning names; the count of a secure field's masked
/// characters; `nothing to read back` for a field that showed nothing, and
/// `not read back` for one the read-back did not read, as when no field had
/// the focus; the twin a stale ref stood for first, as fill's line has it.
fn typed(data: &Value, out: &mut String) {
    let read = match (&data["verified"], data["text"].as_str()) {
        (Value::String(shown), Some(held)) if shown == "length" => masked(held),
        (_, Some(held)) => format!("read back {held:?}"),
        (Value::Bool(false), None) => "nothing to read back".to_owned(),
        _ => "not read back".to_owned(),
    };
    let mut parts: Vec<String> = resolution(data).into_iter().chain([read]).collect();
    entry(data, &mut parts);
    let _ = writeln!(out, "✓ Done ({})", parts.join(", "));
}

/// A read-back of `shown`, a secure field's text, by its count of masked
/// characters.
fn masked(shown: &str) -> String {
    let count = shown.chars().count();
    let plural = if count == 1 { "" } else { "s" };
    format!("read back {count} masked character{plural}")
}

/// How the text of a fill or a type went in, after the read-back in its
/// line: `typed` for keys, `entered in one piece` for `direct`.
fn entry(data: &Value, parts: &mut Vec<String>) {
    match str_of(data, "method") {
        "keys" => parts.push("typed".to_owned()),
        "direct" => parts.push("entered in one piece".to_owned()),
        "" => {}
        other => parts.push(other.to_owned()),
    }
}

/// A scroll's strokes, `✓ Done (2 strokes)`; without `until`, when the
/// content came to rest after the last, `✓ Done (2 strokes, at rest 812.3
/// ms after the last)`, or that it was `still moving` when the scroll
/// stopped waiting (design D6 of action-verdicts); with `until`, how much
/// of the element is in view, `✓ Done (3 strokes; "Row 57" is 100% in
/// view)`, the element named as the listing names it, or by the selector
/// when it has no name, and its share rounded as the daemon's messages
/// round it; the twin a stale ref stood for first, as fill's line has it.
fn scroll(data: &Value, out: &mut String) {
    let strokes = data["strokes"].as_u64().unwrap_or(0);
    let count = if strokes == 1 { "stroke" } else { "strokes" };
    let place = match (data["element"].as_object(), data["visible"].as_f64()) {
        (Some(element), Some(visible)) => {
            let named = match element.get("name").and_then(Value::as_str) {
                Some(name) if !name.is_empty() => format!("{name:?}"),
                _ => str_of(data, "until").to_owned(),
            };
            format!("; {named} is {}% in view", (visible * 100.0).round())
        }
        _ => match (data["settled"].as_bool(), data["movedOnMs"].as_f64()) {
            (Some(settled), Some(moved_on)) => {
                let state = if settled { "at rest" } else { "still moving" };
                let last = if strokes == 1 { "it" } else { "the last" };
                format!(", {state} {moved_on:.1} ms after {last}")
            }
            _ => String::new(),
        },
    };
    match resolution(data) {
        Some(resolved) => {
            let _ = writeln!(out, "✓ Done ({resolved}, {strokes} {count}{place})");
        }
        None => {
            let _ = writeln!(out, "✓ Done ({strokes} {count}{place})");
        }
    }
}

/// A click's spans: `✓ Done (dispatch 0.2 ms, first frame change 121.3 ms,
/// first tree delta 121.9 ms)`, leaving out the spans that never came,
/// after what the tap landed on as the listing names it, by its role and
/// any name, `hit button "Sign in"`, when the reply names it (design D1 of
/// action-verdicts), and first the twin a stale ref stood for, `@e221 now
/// @e240` ([`resolution`], design D3).
fn click(data: &Value, out: &mut String) {
    let span = |key: &str| data[key].as_f64().map(|ms| format!("{ms:.1} ms"));
    let mut parts: Vec<String> = resolution(data).into_iter().collect();
    if data["hit"].is_object() {
        let hit = &data["hit"];
        let role = str_of(hit, "role");
        match str_of(hit, "name") {
            "" => parts.push(format!("hit {role}")),
            name => parts.push(format!("hit {role} {name:?}")),
        }
    }
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

/// A change wait: what moved and when, then the delta as `snapshot
/// --delta` prints it.
fn change(data: &Value, out: &mut String) {
    let what = match str_of(data, "endedBy") {
        "tree" => "The tree",
        "frame" => "Frames",
        _ => "The tree and frames",
    };
    let _ = writeln!(
        out,
        "✓ {what} changed after {} ms",
        number(&data["elapsedMs"])
    );
    snapshot(data, out);
}

/// A settle wait and the signal it ended on; after an input whose first
/// effect it waited for, `nothing changed within 1 s of the click`, the
/// input named as a noun, when nothing changed since it (design D2 of
/// action-verdicts).
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
    let ended = match (str_of(data, "endedBy"), data["input"].as_str()) {
        ("tree", _) => ": the tree changed last".to_owned(),
        ("frame", _) => ": frames changed last".to_owned(),
        ("both", _) => ": the tree and frames changed last".to_owned(),
        ("none", Some("type")) => ": nothing changed within 1 s of the typing".to_owned(),
        ("none", Some(input)) => format!(": nothing changed within 1 s of the {input}"),
        ("none", None) => ": nothing changed".to_owned(),
        _ => String::new(),
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
            Some("delta") => changed_lines(delta, out),
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

/// A delta whose kind is `delta` (design D5 of action-verdicts): a header
/// naming the delta's revision, the revision it was taken against and the
/// lines of that revision's listing it replaces, counted from 1 where
/// `startLine` counts from 0, as many as `deleteCount`, `delta (revision 3
/// from 2): lines 2 to 4 replaced by`; then `treeChange`'s lines as the
/// listing prints them, and the refs the delta removed, `removed refs: @e5,
/// @e6`, in the form of a snapshot's `removedRefs`. A delta that replaces
/// no line says where its lines go in, `inserted after line 4`, and one
/// that brings no line which lines it removed, `lines 3 to 4 removed`.
fn changed_lines(delta: &Map<String, Value>, out: &mut String) {
    let count = |value: &Value| value.as_u64().unwrap_or(0);
    let splice = delta.get("treeChange").unwrap_or(&Value::Null);
    let (start, deleted) = (count(&splice["startLine"]), count(&splice["deleteCount"]));
    let lines: Vec<&str> = splice["lines"]
        .as_array()
        .map(|lines| lines.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let replaced = match deleted {
        0 => None,
        1 => Some(format!("line {}", start + 1)),
        _ => Some(format!("lines {} to {}", start + 1, start + deleted)),
    };
    let what = match replaced {
        Some(replaced) if lines.is_empty() => format!("{replaced} removed"),
        Some(replaced) => format!("{replaced} replaced by"),
        None if start == 0 => "inserted before line 1".to_owned(),
        None => format!("inserted after line {start}"),
    };
    let revision = |key: &str| delta.get(key).map_or(0, count);
    let _ = writeln!(
        out,
        "delta (revision {} from {}): {what}",
        revision("revision"),
        revision("baseRevision")
    );
    for line in lines {
        let _ = writeln!(out, "{line}");
    }
    let removed: Vec<&str> = delta
        .get("changes")
        .and_then(Value::as_array)
        .map(|changes| {
            changes
                .iter()
                .filter(|change| change["op"] == "remove")
                .filter_map(|change| change["ref"].as_str())
                .collect()
        })
        .unwrap_or_default();
    if !removed.is_empty() {
        let _ = writeln!(out, "removed refs: {}", removed.join(", "));
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
                str_of(d, "state").to_owned(),
            ];
            if let Some(holder) = d.get("claimedBy").and_then(Value::as_str) {
                row.push(format!("claimed by {holder}"));
            }
            row
        })
        .collect();
    let mut listing = String::new();
    table(&rows, &mut listing);
    // A target whose state keeps it from automation says what to do below
    // its line.
    for (line, device) in listing.lines().zip(&devices) {
        let _ = writeln!(out, "{line}");
        if let Some(advice) = device.get("advice").and_then(Value::as_str) {
            let _ = writeln!(out, "    {advice}");
        }
    }
}

fn status(data: &Value, out: &mut String) {
    let daemon = &data["daemon"];
    let idle = match daemon["idleTimeoutMs"].as_u64() {
        Some(ms) => format!("idle timeout {ms} ms"),
        None => "no idle timeout".to_owned(),
    };
    // The emulator controller setting is named when it leaves the
    // controller unused.
    let controller = if daemon["emulatorController"] == "off" {
        ", emulator controller off"
    } else {
        ""
    };
    let _ = writeln!(
        out,
        "daemon: running, pid {}, version {}, up {} s, {idle}{controller}",
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
                let mut state: Vec<String> = t
                    .get("state")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .into_iter()
                    .collect();
                state.push(if t["attached"] == Value::Bool(true) {
                    "attached".to_owned()
                } else {
                    "not attached".to_owned()
                });
                match t.get("frameSource").and_then(Value::as_bool) {
                    Some(true) => state.push("frames".to_owned()),
                    Some(false) => state.push("no frames".to_owned()),
                    None => {}
                }
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
        // What each attached target's driver reports: its paths, helpers
        // and failures.
        for target in &targets {
            let lines = target["driver"]["lines"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if lines.is_empty() {
                continue;
            }
            let _ = writeln!(out, "{}:", str_of(target, "id"));
            for line in lines.iter().filter_map(Value::as_str) {
                let _ = writeln!(out, "    {line}");
            }
        }
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
        assert!(
            lines[1].starts_with("R58M123 ") && lines[1].ends_with("  unauthorized"),
            "{out}"
        );
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

    /// Task 10.12: the daemon line names the emulator controller setting
    /// when it leaves the controller unused, and only then.
    #[test]
    fn status_names_the_emulator_controller_when_it_is_off() {
        let status = |controller: &str| {
            let reply = ok(json!({
                "daemon": {"pid": 1, "version": "0.1.0", "socket": "/s", "uptimeMs": 1000,
                           "idleTimeoutMs": 60000, "emulatorController": controller},
                "targets": [],
                "sessions": [],
            }));
            human("status", &reply).0
        };
        let off = status("off");
        assert!(
            off.lines().next().is_some_and(
                |line| line.ends_with("idle timeout 60000 ms, emulator controller off")
            ),
            "{off}"
        );
        let auto = status("auto");
        assert!(!auto.contains("emulator controller"), "{auto}");
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

    /// Task 6.4 of the mcp-server change: fill's line says what the
    /// read-back found, the text or a masked field's count of characters,
    /// how the text went in, and that it went in again when it did.
    #[test]
    fn fill_says_what_it_read_back_and_how_the_text_went_in() {
        let filled = |text: &str, verified: Value, method: Value, retried: bool| {
            let from = verified.is_string().then_some("value");
            let reason = (verified == Value::Bool(false)).then_some("no_value_to_read");
            ok(json!({
                "filled": "#login-username", "ref": "@e2", "text": text, "method": method,
                "verified": verified, "readFrom": from, "reason": reason,
                "readBackMs": 12.5, "retried": retried,
            }))
        };
        let line = |reply: &Reply| human("fill", reply).0;
        assert_eq!(
            line(&filled("agent", json!("text"), json!("keys"), false)),
            "✓ Done (read back \"agent\", typed)\n"
        );
        assert_eq!(
            line(&filled("hunter2", json!("length"), json!("direct"), false)),
            "✓ Done (read back 7 masked characters, entered in one piece)\n"
        );
        assert_eq!(
            line(&filled("x", json!("length"), json!("keys"), false)),
            "✓ Done (read back 1 masked character, typed)\n"
        );
        assert_eq!(
            line(&filled("nobel", json!("text"), json!("direct"), true)),
            "✓ Done (read back \"nobel\", entered in one piece, retried)\n"
        );
        // A target that names no method leaves the line to the read-back.
        assert_eq!(
            line(&filled("agent", json!("text"), Value::Null, false)),
            "✓ Done (read back \"agent\")\n"
        );
        // A field with nothing to read back: the line says so beside the
        // warning the reply carries. Null is a field not read back at all.
        let mut unread = filled("agent", json!(false), json!("keys"), false);
        let warning = "#login-username shows neither a value nor anything below it, so fill could not read its text back; a screenshot shows what it holds";
        unread.warning = Some(warning.to_owned());
        assert_eq!(
            human("fill", &unread),
            (
                "✓ Done (nothing to read back, typed)\n".to_owned(),
                format!("⚠ {warning}\n")
            )
        );
        assert_eq!(
            line(&filled("", Value::Null, json!("direct"), false)),
            "✓ Done (not read back, entered in one piece)\n"
        );
    }

    /// Design D3 of real-app-fixes: type's line says what the read-back
    /// found in the field, as fill's does, and how the text went in: what
    /// the field holds, also when it differs from the text typed, which the
    /// reply's warning names; a secure field's count of masked characters;
    /// and that nothing was read back, for a field that showed nothing or
    /// none that had the focus.
    #[test]
    fn type_says_what_it_read_back_from_the_field() {
        let typed = |text: Value, verified: Value, reason: Value| {
            ok(json!({
                "typed": "example.com", "ref": null, "method": "keys", "text": text,
                "verified": verified, "readFrom": "value", "reason": reason,
                "readBackMs": 201.5,
            }))
        };
        let line = |reply: &Reply| human("type", reply).0;
        assert_eq!(
            line(&typed(json!("example.com"), json!("text"), Value::Null)),
            "✓ Done (read back \"example.com\", typed)\n"
        );
        assert_eq!(
            line(&typed(
                json!("exampl"),
                json!(false),
                json!("text_mismatch")
            )),
            "✓ Done (read back \"exampl\", typed)\n"
        );
        assert_eq!(
            line(&typed(json!("•••••••"), json!("length"), Value::Null)),
            "✓ Done (read back 7 masked characters, typed)\n"
        );
        assert_eq!(
            line(&typed(Value::Null, json!(false), json!("no_value_to_read"))),
            "✓ Done (nothing to read back, typed)\n"
        );
        assert_eq!(
            line(&typed(Value::Null, Value::Null, json!("no_focused_field"))),
            "✓ Done (not read back, typed)\n"
        );
    }

    /// Design D5 of real-app-fixes: get value prints a switch's state, `on`
    /// or `off` as the listing's `checked` has it, the value where the
    /// element has one, and `no value` for an element without either.
    #[test]
    fn get_value_prints_a_switch_s_state_and_says_when_there_is_no_value() {
        let got = |data: Value| human("get", &ok(data)).0;
        assert_eq!(
            got(json!({"value": null, "checked": true, "inView": true})),
            "on\n"
        );
        assert_eq!(
            got(json!({"value": null, "checked": false, "inView": true})),
            "off\n"
        );
        assert_eq!(got(json!({"value": "agent", "inView": true})), "agent\n");
        assert_eq!(got(json!({"value": null, "inView": true})), "no value\n");
        assert_eq!(
            got(json!({"value": null, "inView": false})),
            "no value\nout of view: scroll it into view before acting on it\n"
        );
    }

    /// Design D5: a wait for a text or an element names what it found, its
    /// role, name and ref, where it answered `✓ Done` alone; a wait for an
    /// element to go keeps that.
    #[test]
    fn a_wait_names_the_element_it_found() {
        let found = |waited: &str, element: Value| {
            let mut data = json!({"waited": waited, "elapsedMs": 12.5, "element": element,
                                  "inView": true});
            data[waited] = "Welcome".into();
            human("wait", &ok(data)).0
        };
        let status = json!({"ref": "@e12", "role": "text", "name": "Welcome, agent",
                            "id": "login-status", "value": null, "enabled": true,
                            "bounds": [16.0, 342.0, 386.0, 366.0]});
        assert_eq!(
            found("text", status.clone()),
            "✓ Found text \"Welcome, agent\" @e12\n"
        );
        assert_eq!(
            found("selector", status),
            "✓ Found text \"Welcome, agent\" @e12\n"
        );
        let unnamed = json!({"ref": null, "role": "image", "name": "", "id": "logo",
                             "value": null, "enabled": true, "bounds": [0.0, 0.0, 8.0, 8.0]});
        assert_eq!(found("selector", unnamed), "✓ Found image\n");
        assert_eq!(found("selector", Value::Null), "✓ Done\n");
    }

    /// Design D5: a screenshot's line says the image's pixels beside the
    /// screen's points, which a click takes.
    #[test]
    fn a_screenshot_says_its_pixels_and_the_screen_s_points() {
        let shot = ok(json!({
            "path": "/tmp/shot.jpg", "format": "jpeg", "width": 460, "height": 1000,
            "quality": 80, "seq": 7, "ageMs": 3.5, "source": "frame",
            "points": {"width": 402.0, "height": 874.0},
        }));
        assert_eq!(
            human("screenshot", &shot).0,
            "✓ Screenshot saved to /tmp/shot.jpg (460x1000 of a 402x874-point screen)\n"
        );
    }

    /// Design D5 of action-verdicts: a delta whose kind is `delta` prints a
    /// header naming its revision and the one it was taken against, and the
    /// lines of that one's listing it replaces, from its start line counted
    /// from 1, as many as its count; then the lines that take their place,
    /// as the listing prints them, and the refs it removed, as `removedRefs`
    /// names them. A change wait prints its delta the same way.
    #[test]
    fn a_delta_prints_a_header_its_lines_and_the_refs_it_removed() {
        let delta = |start: u64, count: u64, lines: &[&str], changes: Value| {
            json!({
                "baseRevision": 2, "changes": changes, "kind": "delta", "revision": 3,
                "treeChange": {"deleteCount": count, "lines": lines, "startLine": start},
            })
        };
        let printed = |delta: Value| human("snapshot", &ok(json!({"snapshot": delta}))).0;
        let rows = [
            "- button \"Row 9\" [id=row-9, ref=e9]",
            "  - text \"Ninth\"",
        ];
        let changes = json!([
            {"op": "remove", "ref": "@e5"},
            {"op": "remove", "ref": "@e6"},
            {"field": "name", "op": "replace", "ref": "@e7", "value": "Row 7"},
            {"node": {"name": "Row 9", "role": "button"}, "op": "add", "ref": "@e9"},
        ]);
        assert_eq!(
            printed(delta(1, 3, &rows, changes)),
            "delta (revision 3 from 2): lines 2 to 4 replaced by\n\
             - button \"Row 9\" [id=row-9, ref=e9]\n  - text \"Ninth\"\n\
             removed refs: @e5, @e6\n"
        );
        let field = ["- textbox \"Username\" [id=login-username, ref=e5]: agent"];
        assert_eq!(
            printed(delta(1, 1, &field, json!([]))),
            "delta (revision 3 from 2): line 2 replaced by\n\
             - textbox \"Username\" [id=login-username, ref=e5]: agent\n"
        );
        let added =
            json!([{"node": {"name": "Row 9", "role": "button"}, "op": "add", "ref": "@e9"}]);
        assert_eq!(
            printed(delta(4, 0, &rows[..1], added.clone())),
            "delta (revision 3 from 2): inserted after line 4\n\
             - button \"Row 9\" [id=row-9, ref=e9]\n"
        );
        assert_eq!(
            printed(delta(0, 0, &rows[..1], added)),
            "delta (revision 3 from 2): inserted before line 1\n\
             - button \"Row 9\" [id=row-9, ref=e9]\n"
        );
        assert_eq!(
            printed(delta(2, 2, &[], json!([{"op": "remove", "ref": "@e5"}]))),
            "delta (revision 3 from 2): lines 3 to 4 removed\nremoved refs: @e5\n"
        );
        let waited = ok(json!({
            "waited": "change", "endedBy": "tree", "elapsedMs": 12.5,
            "snapshot": delta(1, 1, &field, json!([])),
        }));
        assert_eq!(
            human("wait", &waited).0,
            "✓ The tree changed after 12.5 ms\n\
             delta (revision 3 from 2): line 2 replaced by\n\
             - textbox \"Username\" [id=login-username, ref=e5]: agent\n"
        );
    }

    /// Design D1 of action-verdicts: a click's line names what its tap hit
    /// before its spans, by its role and its name, which an element without
    /// one leaves out, and is as it was for a reply that names nothing.
    #[test]
    fn a_click_names_what_its_tap_hit_before_its_spans() {
        let clicked = |hit: Value| {
            ok(json!({
                "clicked": "@e4", "ref": "@e4", "x": 201.0, "y": 164.0, "accepted": true,
                "hit": hit, "dispatchMs": 0.214, "treeDeltaMs": 351.04, "watchedMs": 551.3,
            }))
        };
        let line = |reply: &Reply| human("click", reply).0;
        assert_eq!(
            line(&clicked(
                json!({"role": "button", "name": "Sign in", "ref": "@e4"})
            )),
            "✓ Done (hit button \"Sign in\", dispatch 0.2 ms, first tree delta 351.0 ms)\n"
        );
        assert_eq!(
            line(&clicked(
                json!({"role": "keyboard", "name": "", "ref": null})
            )),
            "✓ Done (hit keyboard, dispatch 0.2 ms, first tree delta 351.0 ms)\n"
        );
        assert_eq!(
            line(&clicked(Value::Null)),
            "✓ Done (dispatch 0.2 ms, first tree delta 351.0 ms)\n"
        );
    }

    /// Design D2 of android-pass-fixes: a click whose tap landed on an
    /// element the session holds no ref for, as a plain text, says so after
    /// its name, while one on an element with a ref and one on the keyboard
    /// name them as before.
    #[test]
    fn a_click_whose_hit_has_no_ref_says_so() {
        let clicked = |hit: Value| {
            ok(json!({
                "clicked": "201 112", "ref": null, "x": 201.0, "y": 112.0, "accepted": true,
                "hit": hit, "dispatchMs": 0.214, "watchedMs": 201.3,
            }))
        };
        let line = |reply: &Reply| human("click", reply).0;
        assert_eq!(
            line(&clicked(
                json!({"role": "text", "name": "Address", "ref": null})
            )),
            "✓ Done (hit text \"Address\" (no ref), dispatch 0.2 ms, no change in 201 ms)\n"
        );
        assert_eq!(
            line(&clicked(json!({"role": "image", "name": "", "ref": null}))),
            "✓ Done (hit image (no ref), dispatch 0.2 ms, no change in 201 ms)\n"
        );
        assert_eq!(
            line(&clicked(
                json!({"role": "textbox", "name": "Address", "ref": "@e7"})
            )),
            "✓ Done (hit textbox \"Address\", dispatch 0.2 ms, no change in 201 ms)\n"
        );
        assert_eq!(
            line(&clicked(
                json!({"role": "keyboard", "name": "", "ref": null})
            )),
            "✓ Done (hit keyboard, dispatch 0.2 ms, no change in 201 ms)\n"
        );
    }

    /// Design D3 of action-verdicts: an action on a stale ref that the
    /// daemon resolved to its twin says which ref it took first in its line,
    /// `(@e5 now @e9, ...)`, and a get on a line of its own after what it
    /// read.
    #[test]
    fn an_action_on_a_stale_ref_resolved_to_its_twin_names_both_refs() {
        let resolved = json!({"stale": "@e5", "ref": "@e9"});
        let clicked = ok(json!({
            "clicked": "@e5", "ref": "@e9", "resolved": resolved, "x": 201.0, "y": 164.0,
            "hit": {"role": "textbox", "name": "Title", "ref": "@e9"}, "accepted": true,
            "dispatchMs": 0.214, "watchedMs": 201.3,
        }));
        assert_eq!(
            human("click", &clicked).0,
            "✓ Done (@e5 now @e9, hit textbox \"Title\", dispatch 0.2 ms, no change in 201 ms)\n"
        );
        let filled = ok(json!({
            "filled": "@e5", "text": "standup", "ref": "@e9", "resolved": resolved,
            "method": "keys", "verified": "text", "readFrom": "value", "reason": null,
            "readBackMs": 210.0, "retried": false,
        }));
        assert_eq!(
            human("fill", &filled).0,
            "✓ Done (@e5 now @e9, read back \"standup\", typed)\n"
        );
        let typed = ok(json!({
            "typed": "up", "ref": "@e9", "resolved": resolved, "method": "keys",
            "text": "standup", "verified": "text", "readFrom": "value", "reason": null,
            "readBackMs": 210.0,
        }));
        assert_eq!(
            human("type", &typed).0,
            "✓ Done (@e5 now @e9, read back \"standup\", typed)\n"
        );
        let got = ok(json!({"value": "standup", "inView": true, "resolved": resolved}));
        assert_eq!(human("get", &got).0, "standup\n(@e5 now @e9)\n");
        let scrolled = ok(json!({
            "scrolled": true, "direction": "down", "amount": 300.0, "strokes": 1,
            "from": [201.0, 600.0], "to": [201.0, 300.0], "resolved": resolved,
        }));
        assert_eq!(
            human("scroll", &scrolled).0,
            "✓ Done (@e5 now @e9, 1 stroke)\n"
        );
    }

    /// Design D2 of action-verdicts: a settle that waited for an input's
    /// first effect says when none came within a second of the input, naming
    /// it, and otherwise what changed last, as any settle does.
    #[test]
    fn a_settle_after_an_input_says_when_nothing_changed_within_1_s_of_it() {
        let settled = |input: Option<&str>, ended: &str| {
            let mut data = json!({
                "waited": "settle", "settled": true, "endedBy": ended, "elapsedMs": 795.25,
                "stillChanging": [], "treeChanges": 0, "frameChanges": 0,
            });
            if let Some(input) = input {
                data["input"] = input.into();
            }
            human("wait", &ok(data)).0
        };
        assert_eq!(
            settled(Some("click"), "none"),
            "✓ Settled after 795.25 ms: nothing changed within 1 s of the click\n"
        );
        assert_eq!(
            settled(Some("press"), "none"),
            "✓ Settled after 795.25 ms: nothing changed within 1 s of the press\n"
        );
        assert_eq!(
            settled(Some("type"), "none"),
            "✓ Settled after 795.25 ms: nothing changed within 1 s of the typing\n"
        );
        assert_eq!(
            settled(Some("click"), "tree"),
            "✓ Settled after 795.25 ms: the tree changed last\n"
        );
        assert_eq!(
            settled(None, "none"),
            "✓ Settled after 795.25 ms: nothing changed\n"
        );
    }

    /// Design D5 of action-verdicts: `--json` keeps a delta object as the
    /// mirror writes it, byte for byte.
    #[test]
    fn the_json_line_keeps_a_delta_object_byte_for_byte() {
        let object = r#"{"baseRevision":2,"changes":[{"op":"remove","ref":"@e5"},{"node":{"name":"Row 9","role":"button"},"op":"add","ref":"@e9"}],"kind":"delta","revision":3,"treeChange":{"deleteCount":3,"lines":["- button \"Row 9\" [id=row-9, ref=e9]"],"startLine":1}}"#;
        let delta: Value = serde_json::from_str(object).expect("the delta object is JSON");
        assert_eq!(
            json_line(&ok(json!({"snapshot": delta}))),
            format!(r#"{{"success":true,"data":{{"snapshot":{object}}},"error":null}}"#)
        );
    }

    /// Task 6.5: scroll's line counts its strokes and, with until, says how
    /// much of the element is in view, the element named as the listing
    /// names it, or by the selector when it has no name.
    #[test]
    fn scroll_counts_its_strokes_and_says_where_until_s_element_ended() {
        let moved = |strokes: u32| {
            ok(json!({
                "scrolled": strokes > 0, "direction": "down", "amount": 480.0,
                "strokes": strokes, "from": [200.0, 700.0], "to": [200.0, 220.0],
            }))
        };
        assert_eq!(human("scroll", &moved(2)).0, "✓ Done (2 strokes)\n");
        assert_eq!(human("scroll", &moved(1)).0, "✓ Done (1 stroke)\n");
        let brought = |strokes: u32, name: &str, visible: f64| {
            ok(json!({
                "scrolled": strokes > 0, "direction": "down", "amount": 600.0,
                "strokes": strokes, "until": "#row-57", "visible": visible,
                "element": {"ref": "@e57", "role": "button", "name": name, "id": "row-57",
                            "value": null, "enabled": true,
                            "bounds": [0.0, 512.0, 400.0, 560.0]},
                "from": [200.0, 700.0], "to": [200.0, 100.0],
            }))
        };
        assert_eq!(
            human("scroll", &brought(3, "Row 57", 1.0)).0,
            "✓ Done (3 strokes; \"Row 57\" is 100% in view)\n"
        );
        assert_eq!(
            human("scroll", &brought(0, "", 0.834)).0,
            "✓ Done (0 strokes; #row-57 is 83% in view)\n"
        );
    }

    /// Design D6 of action-verdicts: a plain scroll's line says when the
    /// content came to rest after its last stroke, or that the screen was
    /// still changing when the scroll stopped waiting for it.
    #[test]
    fn a_plain_scroll_says_when_the_content_came_to_rest_after_its_strokes() {
        let rested = |strokes: u32, settled: bool, moved_on: f64| {
            let data = ok(json!({
                "scrolled": true, "direction": "down", "amount": 600.0, "strokes": strokes,
                "from": [201.0, 612.0], "to": [201.0, 262.0], "settled": settled,
                "movedOnMs": moved_on,
            }));
            human("scroll", &data).0
        };
        assert_eq!(
            rested(2, true, 2341.27),
            "✓ Done (2 strokes, at rest 2341.3 ms after the last)\n"
        );
        assert_eq!(
            rested(1, true, 0.0),
            "✓ Done (1 stroke, at rest 0.0 ms after it)\n"
        );
        assert_eq!(
            rested(2, false, 2987.5),
            "✓ Done (2 strokes, still moving 2987.5 ms after the last)\n"
        );
    }
}
