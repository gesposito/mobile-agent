//! Talking to the daemon as agent-browser's CLI talks to its own: a fresh
//! connection to the Unix socket per request, one JSON line out, one back,
//! up to five attempts on transient errors with 200 ms times the attempt
//! between them, and a read timeout of 30 s or the command's own timeout
//! plus 10 s, whichever is longer, so waits report their own timeout.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

/// The socket's file name in the socket directory.
pub const SOCKET_FILE: &str = "daemon.sock";

const ATTEMPTS: u32 = 5;
const BACKOFF: Duration = Duration::from_millis(200);
const READ_FLOOR: Duration = Duration::from_secs(30);
const READ_MARGIN: Duration = Duration::from_secs(10);

/// The daemon could not be reached or started.
pub const DAEMON_UNAVAILABLE: &str = "daemon_unavailable";
/// The daemon did not answer in time.
pub const DAEMON_TIMEOUT: &str = "daemon_timeout";

/// The socket directory: the flag or `MOBILE_AGENT_SOCKET_DIR`, else
/// `$XDG_RUNTIME_DIR/mobile-agent`, else `~/.mobile-agent`, else
/// `mobile-agent` in the temporary directory, as the daemon resolves it.
pub fn socket_dir(flag: Option<&Path>) -> PathBuf {
    if let Some(dir) = flag.filter(|dir| !dir.as_os_str().is_empty()) {
        return dir.to_path_buf();
    }
    let var = |key: &str| {
        std::env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(runtime) = var("XDG_RUNTIME_DIR") {
        return runtime.join("mobile-agent");
    }
    if let Some(home) = var("HOME") {
        return home.join(".mobile-agent");
    }
    std::env::temp_dir().join("mobile-agent")
}

/// A failure before or instead of a daemon reply.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub code: String,
    pub message: String,
}

impl Failure {
    pub fn new(code: &str, message: impl Into<String>) -> Failure {
        Failure {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

/// A daemon reply, agent-browser's envelope.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub success: bool,
    pub data: Value,
    pub error: Option<String>,
    pub code: Option<String>,
    pub warning: Option<String>,
}

impl Reply {
    fn parse(line: &str) -> Option<Reply> {
        let Value::Object(mut reply) = serde_json::from_str::<Value>(line).ok()? else {
            return None;
        };
        let text = |value: Option<Value>| value.and_then(|v| v.as_str().map(str::to_owned));
        Some(Reply {
            success: reply.get("success").and_then(Value::as_bool)?,
            data: reply.remove("data").unwrap_or(Value::Null),
            error: text(reply.remove("error")),
            code: text(reply.remove("code")),
            warning: text(reply.remove("warning")),
        })
    }

    /// A failed reply for a failure on this side of the socket.
    pub fn failed(failure: Failure) -> Reply {
        Reply {
            success: false,
            data: Value::Null,
            error: Some(failure.message),
            code: Some(failure.code),
            warning: None,
        }
    }
}

/// A request id as agent-browser makes them: `r` and the microseconds.
pub fn request_id() -> String {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_micros());
    format!("r{micros}")
}

/// How long to wait for the reply to `request`.
fn read_timeout(request: &Map<String, Value>) -> Duration {
    let own = ["timeout", "ms"]
        .iter()
        .filter_map(|key| request.get(*key).and_then(Value::as_u64))
        .max()
        .map_or(Duration::ZERO, Duration::from_millis);
    (own + READ_MARGIN).max(READ_FLOOR)
}

enum Attempt {
    Reply(Reply),
    /// Nothing listens: no socket, or a refused connection.
    Unreachable(String),
    /// Worth another attempt on the same daemon.
    Transient(String),
    Failed(Failure),
}

fn attempt(socket: &Path, line: &str, timeout: Duration) -> Attempt {
    let mut stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
            return Attempt::Unreachable(e.to_string());
        }
        Err(e) if e.kind() == ErrorKind::WouldBlock => return Attempt::Transient(e.to_string()),
        Err(e) => return Attempt::Unreachable(e.to_string()),
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    if let Err(e) = stream.write_all(line.as_bytes()) {
        return Attempt::Transient(format!("cannot send the request: {e}"));
    }
    let mut reply = String::new();
    match BufReader::new(stream).read_line(&mut reply) {
        Ok(0) => Attempt::Transient("the daemon closed the connection without a reply".into()),
        Ok(_) => match Reply::parse(&reply) {
            Some(reply) => Attempt::Reply(reply),
            None => Attempt::Failed(Failure::new(
                DAEMON_UNAVAILABLE,
                format!("the daemon's reply is not an envelope: {}", reply.trim()),
            )),
        },
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            Attempt::Failed(Failure::new(
                DAEMON_TIMEOUT,
                format!("the daemon did not answer within {} s", timeout.as_secs()),
            ))
        }
        Err(e) if matches!(e.kind(), ErrorKind::ConnectionReset | ErrorKind::BrokenPipe) => {
            Attempt::Transient(e.to_string())
        }
        Err(e) => Attempt::Failed(Failure::new(DAEMON_UNAVAILABLE, e.to_string())),
    }
}

/// The longest socket path: a Unix socket address holds 104 bytes on
/// macOS, the terminating NUL included.
pub const MAX_SOCKET_PATH: usize = 103;

/// Fails when `socket` does not fit in a Unix socket address.
pub fn check_socket(socket: &Path) -> Result<(), Failure> {
    let len = socket.as_os_str().len();
    if len > MAX_SOCKET_PATH {
        return Err(Failure::new(
            DAEMON_UNAVAILABLE,
            format!(
                "the socket path {} is {len} bytes, more than the {MAX_SOCKET_PATH} a Unix socket address holds; set MOBILE_AGENT_SOCKET_DIR to a shorter directory",
                socket.display()
            ),
        ));
    }
    Ok(())
}

/// Sends `request` to the daemon on `socket` and returns its reply.
pub fn send(socket: &Path, request: &Map<String, Value>) -> Result<Reply, Failure> {
    check_socket(socket)?;
    let mut line = Value::Object(request.clone()).to_string();
    line.push('\n');
    let timeout = read_timeout(request);
    let mut last = String::new();
    for attempt_number in 0..ATTEMPTS {
        if attempt_number > 0 {
            std::thread::sleep(BACKOFF * attempt_number);
        }
        match attempt(socket, &line, timeout) {
            Attempt::Reply(reply) => return Ok(reply),
            Attempt::Failed(failure) => return Err(failure),
            Attempt::Unreachable(reason) => {
                return Err(Failure::new(
                    DAEMON_UNAVAILABLE,
                    format!("no daemon answers on {} ({reason})", socket.display()),
                ));
            }
            Attempt::Transient(reason) => last = reason,
        }
    }
    Err(Failure::new(
        DAEMON_UNAVAILABLE,
        format!("{last} (after {ATTEMPTS} attempts; the daemon may be busy or unresponsive)"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn waits_get_their_own_timeout_plus_a_margin_and_other_commands_30_s() {
        let request = |value: Value| value.as_object().cloned().unwrap();
        assert_eq!(
            read_timeout(&request(json!({"action": "snapshot"}))),
            Duration::from_secs(30)
        );
        assert_eq!(
            read_timeout(&request(json!({"action": "wait", "timeout": 45_000}))),
            Duration::from_secs(55)
        );
        assert_eq!(
            read_timeout(&request(json!({"action": "wait", "ms": 2_000}))),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn replies_keep_the_envelope_fields() {
        let reply = Reply::parse(
            r#"{"id":"r1","success":false,"data":null,"error":"gone","code":"stale_ref"}"#,
        )
        .unwrap();
        assert!(!reply.success);
        assert_eq!(reply.code.as_deref(), Some("stale_ref"));
        assert_eq!(reply.error.as_deref(), Some("gone"));
        assert!(Reply::parse("[1]").is_none());
    }
}
