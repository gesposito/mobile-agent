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
/// inline, as image content; the path alone when the call named one, the
/// daemon having written the file; [`UNCHANGED`] instead of either when
/// `unchanged`. A warning follows, marked as the CLI marks it.
pub fn screenshot(reply: &Reply, unchanged: bool) -> CallToolResult {
    let warning = reply
        .warning
        .as_deref()
        .map(|warning| format!("⚠ {warning}"));
    let data = &reply.data;
    let said = if unchanged {
        UNCHANGED
    } else if let Some(path) = data["path"].as_str() {
        path
    } else {
        let (Some(image), Some(mime)) = (data["image"].as_str(), data["mimeType"].as_str()) else {
            return of("screenshot", reply);
        };
        let mut content = vec![ContentBlock::image(image, mime)];
        content.extend(warning.map(ContentBlock::text));
        return CallToolResult::success(content);
    };
    let text = match warning {
        Some(warning) => format!("{said}\n{warning}"),
        None => said.to_owned(),
    };
    CallToolResult::success(vec![ContentBlock::text(text)])
}

/// `text` without the newline that ends the CLI's last line.
pub fn lines(text: &str) -> String {
    text.trim_end_matches('\n').to_owned()
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
    use serde_json::{Value, json};

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
