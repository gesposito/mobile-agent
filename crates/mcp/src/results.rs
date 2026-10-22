//! A reply as a tool's result (design D5 of the `mcp-server` change): the
//! text the CLI prints for it without `--json`, which the CLI's `output.rs`
//! writes, so that a model reads what the CLI's user reads. A snapshot is
//! its listing, without the refs map that repeats it; an action is the
//! CLI's line, such as `✓ Done (dispatch 0.3 ms, no change in 202 ms)` for
//! a click; a warning follows on its own line, marked `⚠` as the CLI marks
//! it. A failure is an execution error (`isError`), the CLI's `✗` line
//! with the daemon's code named after the mark, whose message says what to
//! do next.

use std::time::Duration;

use cli::client::Reply;
use cli::output;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

/// The result of `action`'s `reply`.
pub fn of(action: &str, reply: &Reply) -> CallToolResult {
    let (out, err) = output::human(action, reply);
    if reply.success {
        CallToolResult::success(vec![ContentBlock::text(lines(&format!("{out}{err}")))])
    } else {
        CallToolResult::error(vec![ContentBlock::text(failure(reply, &err))])
    }
}

/// The CLI's lines for a failure, its `✗` line and any warning, with the
/// failure's code after the mark: `✗ stale_ref: e2 is stale: ...`.
fn failure(reply: &Reply, printed: &str) -> String {
    let Some(code) = reply.code.as_deref() else {
        return lines(printed);
    };
    match printed.strip_prefix("✗ ") {
        Some(rest) => lines(&format!("✗ {code}: {rest}")),
        None => lines(&format!("✗ {code}\n{printed}")),
    }
}

/// The line a screenshot with `ifChanged` answers when its image is the
/// session's last.
pub const UNCHANGED: &str = "unchanged since the last screenshot";

/// The result of a screenshot (design D6): the image the daemon returned
/// inline, as image content, with the text of its pixels beside the
/// screen's points, as the CLI's line says them, `460x1000 of a
/// 402x874-point screen`, since a click takes points (real-app-fixes,
/// design D5); the path, and the sizes on the next line, when the call
/// named one, the daemon having written the file; [`UNCHANGED`] instead of
/// either when `unchanged`. A warning follows, marked as the CLI marks it.
pub fn screenshot(reply: &Reply, unchanged: bool) -> CallToolResult {
    let warning = reply
        .warning
        .as_deref()
        .map(|warning| format!("⚠ {warning}"));
    let data = &reply.data;
    let sizes = output::sizes(data);
    let joined = |said: Vec<String>| said.join("\n");
    let said = if unchanged {
        vec![UNCHANGED.to_owned()]
    } else if let Some(path) = data["path"].as_str() {
        std::iter::once(path.to_owned()).chain(sizes).collect()
    } else {
        let (Some(image), Some(mime)) = (data["image"].as_str(), data["mimeType"].as_str()) else {
            return of("screenshot", reply);
        };
        let mut content = vec![ContentBlock::image(image, mime)];
        let text: Vec<String> = sizes.into_iter().chain(warning).collect();
        if !text.is_empty() {
            content.push(ContentBlock::text(joined(text)));
        }
        return CallToolResult::success(content);
    };
    let text: Vec<String> = said.into_iter().chain(warning).collect();
    CallToolResult::success(vec![ContentBlock::text(joined(text))])
}

/// `text` without the newline that ends the CLI's last line.
pub fn lines(text: &str) -> String {
    text.trim_end_matches('\n').to_owned()
}

/// A reply's `data` as a result's structured content (design D5): the data
/// less what the result's content carries already, the refs map a listing
/// or a delta repeats (`refs`, at the top and in `snapshot`) and a
/// screenshot's image (`image`), each optional in the reply's schema.
pub fn structured(mut data: Value) -> Value {
    if let Some(object) = data.as_object_mut() {
        object.remove("refs");
        object.remove("image");
        if let Some(delta) = object.get_mut("snapshot").and_then(Value::as_object_mut) {
            delta.remove("refs");
        }
    }
    data
}

/// The result of a call of `tool` the daemon had not answered 5 s past the
/// server's `bound`, nor within 5 s of the cancel the server sent then
/// (design D7): an execution error with the CLI's code for a daemon that
/// did not answer in time.
pub fn bound(tool: &str, bound: Duration) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "✗ {}: {tool} reached the server's bound of {} s and was cancelled",
        cli::code::DAEMON_TIMEOUT,
        bound.as_secs()
    ))])
}

/// The result of doctor's checks: the report as the CLI prints it, and
/// when a check failed, an execution error ending with the failure's line
/// as `--json` names it.
pub fn doctor(report: &cli::doctor::Report) -> CallToolResult {
    let text = report.to_text();
    if !report.failed() {
        return CallToolResult::success(vec![ContentBlock::text(lines(&text))]);
    }
    let failed: Vec<&str> = report
        .checks
        .iter()
        .filter(|check| check.status == cli::doctor::Status::Fail)
        .map(|check| check.name)
        .collect();
    CallToolResult::error(vec![ContentBlock::text(format!(
        "{text}\n✗ {}: checks failed: {}",
        cli::code::DOCTOR_FAILED,
        failed.join(", ")
    ))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reply(success: bool, data: Value, error: Option<&str>, code: Option<&str>) -> Reply {
        Reply {
            success,
            data,
            error: error.map(str::to_owned),
            code: code.map(str::to_owned),
            warning: None,
        }
    }

    fn text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|block| block.as_text().map(|text| text.text.clone()))
            .collect()
    }

    #[test]
    fn a_success_is_the_cli_s_lines_and_its_warning() {
        let mut clicked = reply(
            true,
            json!({"dispatchMs": 0.31, "watchedMs": 202.4}),
            None,
            None,
        );
        assert_eq!(
            text(&of("click", &clicked)),
            "✓ Done (dispatch 0.3 ms, no change in 202 ms)"
        );
        clicked.warning = Some("#row-20 was not mostly in view".into());
        let result = of("click", &clicked);
        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            text(&result),
            "✓ Done (dispatch 0.3 ms, no change in 202 ms)\n⚠ #row-20 was not mostly in view"
        );
        // The element the tap hit leads the line (design D1 of
        // action-verdicts).
        let hit = reply(
            true,
            json!({"hit": {"role": "tab", "name": "List", "ref": "@e8"},
                   "dispatchMs": 0.31, "watchedMs": 202.4}),
            None,
            None,
        );
        assert_eq!(
            text(&of("click", &hit)),
            "✓ Done (hit tab \"List\", dispatch 0.3 ms, no change in 202 ms)"
        );
    }

    #[test]
    fn structured_content_leaves_the_refs_and_the_image_to_the_content() {
        let snapshot = json!({
            "snapshot": {"kind": "full", "revision": 3, "tree": "- button", "refs": {"e1": {}}},
            "refs": {"e1": {"name": "Sign in", "role": "button"}},
            "target": "emulator-5554",
        });
        assert_eq!(
            structured(snapshot),
            json!({"snapshot": {"kind": "full", "revision": 3, "tree": "- button"}, "target": "emulator-5554"})
        );
        let shot =
            json!({"path": null, "format": "jpeg", "image": "aGVsbG8=", "mimeType": "image/jpeg"});
        assert_eq!(
            structured(shot),
            json!({"path": null, "format": "jpeg", "mimeType": "image/jpeg"})
        );
        assert_eq!(
            structured(json!({"waited": "time", "ms": 500})),
            json!({"waited": "time", "ms": 500})
        );
    }

    #[test]
    fn a_failure_names_its_code_after_the_cli_s_mark() {
        let mut failed = reply(
            false,
            Value::Null,
            Some("e2 is stale: the screen changed. Take a new snapshot"),
            Some("stale_ref"),
        );
        let result = of("click", &failed);
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            text(&result),
            "✗ stale_ref: e2 is stale: the screen changed. Take a new snapshot"
        );
        failed.warning = Some("a cancel ended the watch".into());
        assert_eq!(
            text(&of("click", &failed)),
            "✗ stale_ref: e2 is stale: the screen changed. Take a new snapshot\n⚠ a cancel ended the watch"
        );
    }
}
