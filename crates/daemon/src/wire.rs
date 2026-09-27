//! The CLI-daemon protocol: newline-delimited JSON over the Unix socket in
//! agent-browser's shape (design D1, research note `protocol.md`).
//!
//! A request is one line `{"id", "action", "session", ...parameters}`; the
//! reply is one line `{"id", "success", "data", "error"}` plus `code` and
//! `warning` when set. `error` is null on success; `data` is null on failure
//! unless the error carries details, such as the holder of a claimed
//! target. Every failure has a snake_case `code`.

use serde_json::{Map, Value};

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
    /// `press` got a key the target's platform has no code for.
    pub const UNKNOWN_KEY: &str = "unknown_key";
    /// Writing a file, such as a screenshot, failed.
    pub const IO_ERROR: &str = "io_error";
    /// The daemon failed unexpectedly.
    pub const INTERNAL: &str = "internal";
}

/// One parsed request.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// Echoed in the reply; agent-browser's CLI sends `r` and a number.
    pub id: Value,
    pub action: String,
    /// The session, `default` when the request names none.
    pub session: String,
    /// Everything else in the request object.
    pub params: Map<String, Value>,
}

impl Request {
    /// Parses one request line.
    pub fn parse(line: &str) -> Result<Request, Failure> {
        let value: Value = serde_json::from_str(line).map_err(|e| {
            Failure::new(
                code::INVALID_REQUEST,
                format!("the request is not JSON ({e}); send one JSON object per line"),
            )
        })?;
        let Value::Object(mut params) = value else {
            return Err(Failure::new(
                code::INVALID_REQUEST,
                "the request is not a JSON object",
            ));
        };
        let id = params.remove("id").unwrap_or(Value::Null);
        let action = match params.remove("action") {
            Some(Value::String(action)) => action,
            _ => {
                return Err(Failure::new(
                    code::INVALID_REQUEST,
                    "the request has no \"action\" string",
                )
                .with_id(id));
            }
        };
        let session = match params.remove("session") {
            Some(Value::String(session)) if !session.is_empty() => session,
            None | Some(Value::Null) => "default".to_owned(),
            Some(_) => {
                return Err(Failure::new(
                    code::INVALID_ARGUMENT,
                    "\"session\" must be a non-empty string",
                )
                .with_id(id));
            }
        };
        Ok(Request {
            id,
            action,
            session,
            params,
        })
    }

    /// A string parameter, if present and a string.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(Value::as_str)
    }

    /// A required string parameter.
    pub fn required_str(&self, key: &str) -> Result<&str, Failure> {
        match self.params.get(key) {
            Some(Value::String(s)) if !s.is_empty() => Ok(s),
            _ => Err(Failure::new(
                code::INVALID_ARGUMENT,
                format!("{} needs a \"{key}\" string", self.action),
            )),
        }
    }

    /// A flag: true only when the parameter is `true`.
    pub fn flag(&self, key: &str) -> bool {
        self.params.get(key).and_then(Value::as_bool) == Some(true)
    }

    /// An unsigned integer parameter, if present.
    pub fn u64(&self, key: &str) -> Result<Option<u64>, Failure> {
        match self.params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value.as_u64().map(Some).ok_or_else(|| {
                Failure::new(
                    code::INVALID_ARGUMENT,
                    format!("\"{key}\" must be a non-negative integer, not {value}"),
                )
            }),
        }
    }

    /// A number parameter, if present.
    pub fn f64(&self, key: &str) -> Result<Option<f64>, Failure> {
        match self.params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value.as_f64().map(Some).ok_or_else(|| {
                Failure::new(
                    code::INVALID_ARGUMENT,
                    format!("\"{key}\" must be a number, not {value}"),
                )
            }),
        }
    }

    /// A list of strings, empty when absent.
    pub fn strings(&self, key: &str) -> Result<Vec<String>, Failure> {
        match self.params.get(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_owned).ok_or_else(|| {
                        Failure::new(
                            code::INVALID_ARGUMENT,
                            format!("\"{key}\" must hold strings, not {item}"),
                        )
                    })
                })
                .collect(),
            Some(value) => Err(Failure::new(
                code::INVALID_ARGUMENT,
                format!("\"{key}\" must be a list of strings, not {value}"),
            )),
        }
    }
}

/// A successful reply's payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Success {
    pub data: Value,
    pub warning: Option<String>,
}

impl Success {
    pub fn new(data: Value) -> Success {
        Success {
            data,
            warning: None,
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
/// order, `code` and `warning` only when set.
pub fn reply_line(id: &Value, result: &Result<Success, Failure>) -> String {
    let mut line = String::from("{\"id\":");
    line.push_str(&id.to_string());
    match result {
        Ok(success) => {
            line.push_str(",\"success\":true,\"data\":");
            line.push_str(&success.data.to_string());
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
    use serde_json::json;

    #[test]
    fn a_request_defaults_its_session_and_keeps_its_parameters() {
        let request =
            Request::parse(r#"{"id":"r1","action":"snapshot","interactive":true}"#).unwrap();
        assert_eq!(request.id, json!("r1"));
        assert_eq!(request.action, "snapshot");
        assert_eq!(request.session, "default");
        assert!(request.flag("interactive"));
        assert!(!request.flag("delta"));
    }

    #[test]
    fn replies_follow_agent_browser_field_order() {
        let ok = reply_line(&json!("r1"), &Ok(Success::new(json!({"a": 1}))));
        assert_eq!(
            ok,
            "{\"id\":\"r1\",\"success\":true,\"data\":{\"a\":1},\"error\":null}\n"
        );
        let failed = reply_line(&json!("r2"), &Err(Failure::new("stale_ref", "gone \"x\"")));
        assert_eq!(
            failed,
            "{\"id\":\"r2\",\"success\":false,\"data\":null,\"error\":\"gone \\\"x\\\"\",\"code\":\"stale_ref\"}\n"
        );
    }
}
