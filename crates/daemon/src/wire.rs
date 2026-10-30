//! The CLI-daemon protocol: newline-delimited JSON over the Unix socket in
//! agent-browser's shape (design D1, research note `protocol.md`).
//!
//! A request is one line `{"id", "action", "session", ...parameters}`, read
//! into the request types of `crates/wire` ([`parse`]); the reply is one
//! line `{"id", "success", "data", "error"}` plus `code` and `warning` when
//! set. `error` is null on success; `data` is null on failure
//! unless the error carries details, such as the holder of a claimed
//! target. Every failure has a snake_case `code`.

use serde_json::Value;

/// The codes the daemon raises itself. Mirror and driver errors keep their
/// own codes, such as `stale_ref`.
pub mod code {
    /// The line is not a JSON object, or looks like an HTTP request.
    pub const INVALID_REQUEST: &str = "invalid_request";
    /// A parameter is missing, of the wrong type or out of range.
    pub const INVALID_ARGUMENT: &str = "invalid_argument";
    /// The daemon does not know the action.
    pub const UNKNOWN_ACTION: &str = "unknown_action";
    /// No target is connected.
    pub const NO_TARGET: &str = "no_target";
    /// Several targets are connected and none was chosen.
    pub const AMBIGUOUS_TARGET: &str = "ambiguous_target";
    /// No connected target has the id or name `--device` gave.
    pub const TARGET_NOT_FOUND: &str = "target_not_found";
    /// Another session holds the target.
    pub const TARGET_CLAIMED: &str = "target_claimed";
    /// The target has no such app installed.
    pub const APP_NOT_FOUND: &str = "app_not_found";
    /// No element matched a selector within the command's timeout.
    pub const ELEMENT_NOT_FOUND: &str = "element_not_found";
    /// The target refused a tap, text, key or gesture.
    pub const INPUT_REJECTED: &str = "input_rejected";
    /// A scroll's strokes would be no longer than the platform takes for a
    /// tap, so they would tap what lies under them.
    pub const STROKE_TOO_SHORT: &str = "stroke_too_short";
    /// The keyboard covers what a scroll strokes, but for too little to
    /// take a stroke longer than a tap, and a stroke that came down on the
    /// keyboard would type.
    pub const COVERED_BY_KEYBOARD: &str = "covered_by_keyboard";
    /// `fill` left other text in the field than it entered, typed and then
    /// entered again in one piece; the message names what the field holds.
    pub const TEXT_MISMATCH: &str = "text_mismatch";
    /// `press` got a key the target's platform has no code for.
    pub const UNKNOWN_KEY: &str = "unknown_key";
    /// Writing a file, such as a screenshot, failed.
    pub const IO_ERROR: &str = "io_error";
    /// The daemon is shutting down and attaches no more targets.
    pub const DAEMON_STOPPING: &str = "daemon_stopping";
    /// `open` launched an app that runs, twice, and another app or system UI
    /// stays in front of it; the message names what is in front.
    pub const NOT_IN_FRONT: &str = "not_in_front";
    /// The session's `cancel` ended the command at its next wakeup, as the
    /// mirror's waits report it (design D7 of mcp-server).
    pub const CANCELLED: &str = mirror::code::CANCELLED;
    /// The daemon failed unexpectedly.
    pub const INTERNAL: &str = "internal";
}

pub use ::wire::{Command, Param, Request};

/// Parses one request line into its request (`crates/wire`).
///
/// The types read every line that is a request; a line they refuse, one
/// that is not JSON or not an object, whose envelope is not one, that gives
/// a key twice or names an action they do not know, is read as a map, which
/// words the refusal as the daemon always has and counts the last of a key
/// given twice.
pub fn parse(line: &str) -> Result<Request, Failure> {
    serde_json::from_str(line).or_else(|_| parse_map(line))
}

/// Parses `line` read as a map: the envelope's `id`, `action` and
/// `session`, in that order, then the request.
fn parse_map(line: &str) -> Result<Request, Failure> {
    let value: Value = serde_json::from_str(line).map_err(|e| {
        Failure::new(
            code::INVALID_REQUEST,
            format!("the request is not JSON ({e}); send one JSON object per line"),
        )
    })?;
    let Value::Object(fields) = value else {
        return Err(Failure::new(
            code::INVALID_REQUEST,
            "the request is not a JSON object",
        ));
    };
    let id = fields.get("id").cloned().unwrap_or(Value::Null);
    let action = match fields.get("action") {
        Some(Value::String(action)) => action.clone(),
        _ => {
            return Err(Failure::new(
                code::INVALID_REQUEST,
                "the request has no \"action\" string",
            )
            .with_id(id));
        }
    };
    match fields.get("session") {
        Some(Value::String(session)) if !session.is_empty() => {}
        None | Some(Value::Null) => {}
        Some(_) => {
            return Err(Failure::new(
                code::INVALID_ARGUMENT,
                "\"session\" must be a non-empty string",
            )
            .with_id(id));
        }
    }
    if !Command::ACTIONS.contains(&action.as_str()) {
        return Err(Failure::new(
            code::UNKNOWN_ACTION,
            format!("unknown action {action:?}; run mobile-agent --help for the commands"),
        )
        .with_id(id));
    }
    serde_json::from_value(Value::Object(fields)).map_err(|e| {
        Failure::internal(format!("the request's parameters do not read: {e}")).with_id(id)
    })
}

/// A parameter's value, or, for a value of another type, the failure that
/// names `key` and the value given: a command reads each parameter where
/// it uses it, so the first it reads is the one a request is refused for.
pub fn arg<T: Clone + Refusal>(key: &str, param: &Param<T>) -> Result<T, Failure> {
    match param {
        Param::Valid(value) => Ok(value.clone()),
        Param::Invalid(given) => Err(Failure::new(code::INVALID_ARGUMENT, T::refusal(key, given))),
    }
}

/// A string `action` needs, which a request gives empty, of another type or
/// not at all when it is refused.
pub fn needed<'a>(action: &str, key: &str, value: &'a str) -> Result<&'a str, Failure> {
    if value.is_empty() {
        return Err(Failure::new(
            code::INVALID_ARGUMENT,
            format!("{action} needs a \"{key}\" string"),
        ));
    }
    Ok(value)
}

/// What a parameter's value of another type is refused with, by the type.
pub trait Refusal {
    /// The message for `given` in the place of parameter `key`.
    fn refusal(key: &str, given: &Value) -> String;
}

impl Refusal for u64 {
    fn refusal(key: &str, given: &Value) -> String {
        format!("\"{key}\" must be a non-negative integer, not {given}")
    }
}

impl Refusal for f64 {
    fn refusal(key: &str, given: &Value) -> String {
        format!("\"{key}\" must be a number, not {given}")
    }
}

/// A point, `[x, y]`.
impl Refusal for [f64; 2] {
    fn refusal(key: &str, _given: &Value) -> String {
        format!("\"{key}\" must be [x, y] in points")
    }
}

/// A list of strings: the first item that is no string, or the value that
/// is no list.
impl Refusal for Vec<String> {
    fn refusal(key: &str, given: &Value) -> String {
        match given {
            Value::Array(items) => match items.iter().find(|item| !item.is_string()) {
                Some(item) => format!("\"{key}\" must hold strings, not {item}"),
                None => format!("\"{key}\" must be a list of strings, not {given}"),
            },
            _ => format!("\"{key}\" must be a list of strings, not {given}"),
        }
    }
}

impl<T: Refusal> Refusal for Option<T> {
    fn refusal(key: &str, given: &Value) -> String {
        T::refusal(key, given)
    }
}

/// A successful reply's payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Success {
    /// The data, null when it is `written`.
    pub data: Value,
    /// The data written as JSON text before the reply, which the reply line
    /// carries as it is: the listing's, whose refs map and text
    /// [`crate::listing::reply`] writes straight from the mirror's snapshot.
    pub written: Option<String>,
    pub warning: Option<String>,
}

impl Success {
    pub fn new(data: Value) -> Success {
        Success {
            data,
            written: None,
            warning: None,
        }
    }

    /// A success whose data is `json`, JSON text the reply line carries as
    /// it is.
    pub fn written(json: String) -> Success {
        Success {
            data: Value::Null,
            written: Some(json),
            warning: None,
        }
    }

    /// The data as a value: `data`, or the written text read back.
    #[cfg(test)]
    pub(crate) fn into_data(self) -> Value {
        match self.written {
            Some(json) => serde_json::from_str(&json).expect("the written data is JSON"),
            None => self.data,
        }
    }

    pub fn warn(mut self, warning: Option<String>) -> Success {
        self.warning = warning;
        self
    }
}

/// A failed request: a code, a message that says what to do, and details.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub code: String,
    pub message: String,
    pub data: Option<Value>,
    /// The request id, when the request could be read that far.
    pub id: Option<Value>,
}

impl Failure {
    pub fn new(code: &str, message: impl Into<String>) -> Failure {
        Failure {
            code: code.to_owned(),
            message: message.into(),
            data: None,
            id: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Failure {
        Failure::new(code::INTERNAL, message)
    }

    pub fn with_data(mut self, data: Value) -> Failure {
        self.data = Some(data);
        self
    }

    fn with_id(mut self, id: Value) -> Failure {
        self.id = Some(id);
        self
    }
}

impl From<mirror::MirrorError> for Failure {
    fn from(error: mirror::MirrorError) -> Failure {
        Failure::new(error.code(), error.to_string())
    }
}

impl From<protocol::Error> for Failure {
    fn from(error: protocol::Error) -> Failure {
        Failure::new(&error.code, error.message)
    }
}

/// The reply line for `result`, newline included: agent-browser's field
/// order, `code` and `warning` only when set; written data goes in as it
/// is, into a line sized for it.
pub fn reply_line(id: &Value, result: &Result<Success, Failure>) -> String {
    let written = result
        .as_ref()
        .ok()
        .and_then(|success| success.written.as_deref());
    let mut line = String::with_capacity(written.map_or(0, str::len) + 128);
    line.push_str("{\"id\":");
    line.push_str(&id.to_string());
    match result {
        Ok(success) => {
            line.push_str(",\"success\":true,\"data\":");
            match written {
                Some(written) => line.push_str(written),
                None => line.push_str(&success.data.to_string()),
            }
            line.push_str(",\"error\":null");
            if let Some(warning) = &success.warning {
                line.push_str(",\"warning\":");
                line.push_str(&Value::from(warning.as_str()).to_string());
            }
        }
        Err(failure) => {
            line.push_str(",\"success\":false,\"data\":");
            line.push_str(&failure.data.clone().unwrap_or(Value::Null).to_string());
            line.push_str(",\"error\":");
            line.push_str(&Value::from(failure.message.as_str()).to_string());
            line.push_str(",\"code\":");
            line.push_str(&Value::from(failure.code.as_str()).to_string());
        }
    }
    line.push_str("}\n");
    line
}

/// Whether a line looks like the start of an HTTP request, which the daemon
/// refuses as agent-browser's does.
pub fn looks_like_http(line: &str) -> bool {
    const VERBS: [&str; 9] = [
        "GET ", "POST ", "PUT ", "DELETE ", "HEAD ", "OPTIONS ", "PATCH ", "CONNECT ", "TRACE ",
    ];
    VERBS.iter().any(|verb| line.starts_with(verb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::wire::Command;
    use serde_json::json;

    #[test]
    fn a_request_defaults_its_session_and_keeps_its_parameters() {
        let request = parse(r#"{"id":"r1","action":"snapshot","interactive":true}"#).unwrap();
        assert_eq!(request.id, json!("r1"));
        assert_eq!(request.session, "default");
        let Command::Snapshot(snapshot) = &request.command else {
            panic!("{request:?}");
        };
        assert!(snapshot.interactive);
        assert!(!snapshot.delta);
    }

    /// The lines the CLI sent before the types existed (`crates/wire`).
    const LINES: &str = include_str!("../../wire/tests/lines.json");

    #[test]
    fn every_line_the_cli_sends_parses_as_the_request_types_read_it() {
        let cases: Vec<Value> = serde_json::from_str(LINES).unwrap();
        for case in cases {
            let line = case["line"].to_string();
            let typed: Request = serde_json::from_str(&line).unwrap();
            assert_eq!(parse(&line), Ok(typed), "{line}");
        }
    }

    /// Of a key given twice the last counts, as when the line was read into
    /// a map, whichever of the line's keys it is.
    #[test]
    fn a_key_given_twice_counts_once_its_last_value() {
        for (line, expected) in [
            (
                r#"{"action":"snapshot","interactive":false,"interactive":true}"#,
                r#"{"action":"snapshot","interactive":true}"#,
            ),
            (
                r#"{"action":"frobnicate","id":1,"action":"status","id":2}"#,
                r#"{"action":"status","id":2}"#,
            ),
            (
                r#"{"session":"alice","action":"close","session":"bob"}"#,
                r#"{"action":"close","session":"bob"}"#,
            ),
        ] {
            assert_eq!(parse(line), parse(expected), "{line}");
            assert!(parse(expected).is_ok(), "{expected}");
        }
    }

    #[test]
    fn a_value_of_another_type_is_refused_naming_the_parameter_and_the_value() {
        let refusal = |result: Result<(), Failure>| result.unwrap_err();
        let given = |value: Value| Param::<Option<u64>>::Invalid(value);
        assert_eq!(
            refusal(arg("timeout", &given(json!("x"))).map(drop)),
            Failure::new(
                code::INVALID_ARGUMENT,
                "\"timeout\" must be a non-negative integer, not \"x\""
            )
        );
        assert_eq!(
            refusal(arg("quiet", &Param::<u64>::Invalid(json!(-1))).map(drop)).message,
            "\"quiet\" must be a non-negative integer, not -1"
        );
        assert_eq!(
            refusal(arg("amount", &Param::<f64>::Invalid(json!("far"))).map(drop)).message,
            "\"amount\" must be a number, not \"far\""
        );
        assert_eq!(
            refusal(arg("x", &Param::<Option<f64>>::Invalid(json!([1]))).map(drop)).message,
            "\"x\" must be a number, not [1]"
        );
        assert_eq!(
            refusal(
                arg(
                    "from",
                    &Param::<Option<[f64; 2]>>::Invalid(json!([1, 2, 3]))
                )
                .map(drop)
            )
            .message,
            "\"from\" must be [x, y] in points"
        );
        assert_eq!(
            refusal(arg("env", &Param::<Vec<String>>::Invalid(json!(["A=1", 2]))).map(drop))
                .message,
            "\"env\" must hold strings, not 2"
        );
        assert_eq!(
            refusal(arg("env", &Param::<Vec<String>>::Invalid(json!("A=1"))).map(drop)).message,
            "\"env\" must be a list of strings, not \"A=1\""
        );
        assert_eq!(arg("timeout", &Param::Valid(Some(5_u64))), Ok(Some(5)));
        assert_eq!(
            refusal(needed("open", "app", "").map(drop)),
            Failure::new(code::INVALID_ARGUMENT, "open needs a \"app\" string")
        );
        assert_eq!(needed("open", "app", "a"), Ok("a"));
    }

    /// The defaults the request types give the parameters are the mirror's.
    #[test]
    fn the_parameters_defaults_are_the_mirror_s() {
        assert_eq!(
            std::time::Duration::from_millis(::wire::QUIET_MS),
            mirror::DEFAULT_QUIET_WINDOW
        );
        assert_eq!(::wire::QUALITY, u64::from(mirror::DEFAULT_QUALITY));
        assert_eq!(
            ::wire::MAX_DIMENSION,
            u64::from(mirror::DEFAULT_MAX_DIMENSION)
        );
    }

    #[test]
    fn replies_follow_agent_browser_field_order() {
        let ok = reply_line(&json!("r1"), &Ok(Success::new(json!({"a": 1}))));
        assert_eq!(
            ok,
            "{\"id\":\"r1\",\"success\":true,\"data\":{\"a\":1},\"error\":null}\n"
        );
        let written = Success::written(r#"{"a":1}"#.to_owned()).warn(Some("w".to_owned()));
        assert_eq!(
            reply_line(&json!("r3"), &Ok(written)),
            "{\"id\":\"r3\",\"success\":true,\"data\":{\"a\":1},\"error\":null,\"warning\":\"w\"}\n"
        );
        let failed = reply_line(&json!("r2"), &Err(Failure::new("stale_ref", "gone \"x\"")));
        assert_eq!(
            failed,
            "{\"id\":\"r2\",\"success\":false,\"data\":null,\"error\":\"gone \\\"x\\\"\",\"code\":\"stale_ref\"}\n"
        );
    }
}
