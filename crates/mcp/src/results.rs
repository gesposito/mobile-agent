//! A reply as a tool's result (design D5 of the `mcp-server` change): the
//! text the CLI prints for it without `--json`, which the CLI's `output.rs`
//! writes, so that a model reads what the CLI's user reads. A snapshot is
//! its listing, without the refs map that repeats it; an action is the
//! CLI's line, such as `✓ Done (dispatch 0.3 ms, no change in 202 ms)` for
//! a click; a warning follows on its own line, marked `⚠` as the CLI marks
//! it. A failure is an execution error (`isError`), the CLI's `✗` line
//! with the daemon's code named after the mark, whose message says what to
//! do next.

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

/// `text` without the newline that ends the CLI's last line.
pub fn lines(text: &str) -> String {
    text.trim_end_matches('\n').to_owned()
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
