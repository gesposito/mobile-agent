//! Talking to the daemon as agent-browser's CLI talks to its own: a fresh
//! connection to the Unix socket per request, one JSON line out, one back,
//! up to five attempts on transient errors with 200 ms times the attempt
//! between them, and a read timeout of 30 s or the command's own timeout
//! plus 10 s, whichever is longer, so waits report their own timeout.
//!
//! The first command starts the daemon. Before each request the CLI reads
//! the daemon's version file: when it names another build than the daemon
//! executable this CLI would start, the CLI starts that executable, which
//! replaces the running daemon (see the daemon's README). A request that
//! finds no daemon listening starts one and is sent once more. A process
//! that sends many requests, as the MCP server does, checks the version on
//! its first and again only after one that found no daemon listening
//! ([`call_checked`]).

use std::fs::{DirBuilder, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::code::{DAEMON_TIMEOUT, DAEMON_UNAVAILABLE};

/// The socket's file name in the socket directory.
pub const SOCKET_FILE: &str = "daemon.sock";
/// The file where the daemon records its version.
pub const VERSION_FILE: &str = "daemon.version";
/// Where a daemon the CLI starts writes its error output.
pub const LOG_FILE: &str = "daemon.log";
/// The daemon executable's name.
pub const DAEMON: &str = "mobile-agent-daemon";

/// How long a starting daemon gets to serve.
const STARTUP: Duration = Duration::from_secs(10);

const ATTEMPTS: u32 = 5;
const BACKOFF: Duration = Duration::from_millis(200);
const READ_FLOOR: Duration = Duration::from_secs(30);
const READ_MARGIN: Duration = Duration::from_secs(10);

/// The longest timeout the daemon accepts. A wait that names no timeout
/// ends at its session's default, which the CLI cannot know, so the CLI
/// waits this long for its reply.
const MAX_TIMEOUT: Duration = Duration::from_secs(600);

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

/// How a reply's `data` is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Data {
    /// Whole, as the CLI prints it with `--json`.
    Whole,
    /// Without the refs map of a listing or a delta, at the top of `data`
    /// and in its `snapshot` object, for a caller that shows the listing's
    /// text, which repeats the map's roles and names, as the MCP server
    /// does; the rest as it is.
    WithoutRefs,
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
    fn parse(line: &str, data: Data) -> Option<Reply> {
        let parsed = match data {
            Data::Whole => serde_json::from_str::<Value>(line).ok()?,
            Data::WithoutRefs => serde_json::from_str::<lean::Lean>(line).ok()?.0,
        };
        let Value::Object(mut reply) = parsed else {
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

/// Whether `request` may wait for an element for as long as its session's
/// default timeout: a wait for an element or text, an action on a selector
/// rather than a ref, a scroll until an element is on screen, or an open
/// waiting for the launched app's first tree.
fn waits_by_default(request: &Map<String, Value>) -> bool {
    let target = request.get("target").and_then(Value::as_str);
    match request.get("action").and_then(Value::as_str) {
        Some("wait") => !request.contains_key("settle"),
        Some("open") => true,
        Some("scroll") if request.contains_key("until") => true,
        Some("click" | "fill" | "type" | "get" | "scroll") => {
            target.is_some_and(|target| !crate::request::looks_like_ref(target))
        }
        _ => false,
    }
}

/// How long to wait for the reply to `request`: its own timeout or wait
/// plus a margin, at least 30 s.
fn read_timeout(request: &Map<String, Value>) -> Duration {
    let own = ["timeout", "ms"]
        .iter()
        .filter_map(|key| request.get(*key).and_then(Value::as_u64))
        .max()
        .map(Duration::from_millis);
    let own = match own {
        Some(own) => own,
        None if waits_by_default(request) => MAX_TIMEOUT,
        None => Duration::ZERO,
    };
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

fn attempt(socket: &Path, line: &str, timeout: Duration, data: Data) -> Attempt {
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
        Ok(_) => match Reply::parse(&reply, data) {
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

/// Why a request got no reply.
enum Unsent {
    /// No daemon listens on the socket.
    Unreachable(Failure),
    Failed(Failure),
}

impl From<Unsent> for Failure {
    fn from(unsent: Unsent) -> Failure {
        match unsent {
            Unsent::Unreachable(failure) | Unsent::Failed(failure) => failure,
        }
    }
}

/// Sends `request` to the daemon on `socket` and returns its reply, its
/// `data` read as `data` says.
fn send(socket: &Path, request: &Map<String, Value>, data: Data) -> Result<Reply, Unsent> {
    let mut line = Value::Object(request.clone()).to_string();
    line.push('\n');
    let timeout = read_timeout(request);
    let mut last = String::new();
    for attempt_number in 0..ATTEMPTS {
        if attempt_number > 0 {
            std::thread::sleep(BACKOFF * attempt_number);
        }
        match attempt(socket, &line, timeout, data) {
            Attempt::Reply(reply) => return Ok(reply),
            Attempt::Failed(failure) => return Err(Unsent::Failed(failure)),
            Attempt::Unreachable(reason) => {
                return Err(Unsent::Unreachable(Failure::new(
                    DAEMON_UNAVAILABLE,
                    format!("no daemon answers on {} ({reason})", socket.display()),
                )));
            }
            Attempt::Transient(reason) => last = reason,
        }
    }
    Err(Unsent::Failed(Failure::new(
        DAEMON_UNAVAILABLE,
        format!("{last} (after {ATTEMPTS} attempts; the daemon may be busy or unresponsive)"),
    )))
}

/// Where the daemon lives and how to start it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// The socket directory.
    pub dir: PathBuf,
    /// The daemon executable, when given by flag or environment.
    pub daemon: Option<PathBuf>,
    /// The idle timeout a daemon this CLI starts gets.
    pub idle_timeout: Option<String>,
    /// The emulator controller setting a daemon this CLI starts gets.
    pub emulator_controller: Option<String>,
}

impl Endpoint {
    pub fn socket(&self) -> PathBuf {
        self.dir.join(SOCKET_FILE)
    }

    fn version(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join(VERSION_FILE))
            .ok()
            .map(|version| version.trim().to_owned())
    }
}

/// The daemon executable: the one given, else `mobile-agent-daemon`
/// beside this executable, else on `PATH`.
pub fn daemon_executable(given: Option<&Path>) -> Result<PathBuf, Failure> {
    if let Some(path) = given {
        return Ok(path.to_path_buf());
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe = exe.canonicalize().unwrap_or(exe);
        let beside = exe.with_file_name(DAEMON);
        if beside.is_file() {
            return Ok(beside);
        }
    }
    if let Some(path) = std::env::var_os("PATH")
        && let Some(found) = std::env::split_paths(&path)
            .map(|dir| dir.join(DAEMON))
            .find(|candidate| candidate.is_file())
    {
        return Ok(found);
    }
    Err(Failure::new(
        DAEMON_UNAVAILABLE,
        format!(
            "cannot find {DAEMON} beside mobile-agent or on PATH; build it with cargo build -p daemon, or set MOBILE_AGENT_DAEMON_BIN"
        ),
    ))
}

/// The version a daemon started from `exe` records: the release, `+`, then
/// the executable's size and modification time in nanoseconds in hex, the
/// form the daemon writes, so a rebuilt daemon reads as another version.
pub fn daemon_version(exe: &Path) -> String {
    let build = std::fs::metadata(exe).map_or_else(
        |_| "unknown".to_owned(),
        |meta| {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_nanos());
            format!("{:x}.{:x}", meta.len(), mtime)
        },
    );
    format!("{}+{build}", env!("CARGO_PKG_VERSION"))
}

/// The end of the daemon log from byte `from`, for startup errors.
fn log_tail(path: &Path, from: u64) -> String {
    let mut text = String::new();
    if let Ok(mut file) = std::fs::File::open(path) {
        let _ = file.seek(SeekFrom::Start(from));
        let _ = file.read_to_string(&mut text);
    }
    let text = text.trim();
    let start = text
        .char_indices()
        .rev()
        .nth(499)
        .map_or(0, |(index, _)| index);
    text[start..].to_owned()
}

/// Starts the daemon executable `exe` on the endpoint and waits until a
/// daemon of `version` serves the socket: the one started, or another that
/// a concurrent command started first.
fn start(endpoint: &Endpoint, exe: &Path, version: &str) -> Result<(), Failure> {
    let unavailable = |message: String| Failure::new(DAEMON_UNAVAILABLE, message);
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&endpoint.dir)
        .map_err(|e| unavailable(format!("cannot create {}: {e}", endpoint.dir.display())))?;
    let log_path = endpoint.dir.join(LOG_FILE);
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();
    let log_start = log
        .as_ref()
        .and_then(|file| file.metadata().ok())
        .map_or(0, |meta| meta.len());
    let mut command = Command::new(exe);
    // The daemon outlives this process, so it keeps no working directory of
    // the caller's; paths it writes arrive absolute.
    command
        .arg("--socket-dir")
        .arg(&endpoint.dir)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log.map_or_else(Stdio::null, Stdio::from))
        .process_group(0);
    if let Some(idle) = &endpoint.idle_timeout {
        command.arg("--idle-timeout").arg(idle);
    }
    if let Some(controller) = &endpoint.emulator_controller {
        command.arg("--emulator-controller").arg(controller);
    }
    let mut child = command
        .spawn()
        .map_err(|e| unavailable(format!("cannot start the daemon {}: {e}", exe.display())))?;
    let socket = endpoint.socket();
    let deadline = Instant::now() + STARTUP;
    let mut exited = false;
    loop {
        if endpoint.version().as_deref() == Some(version) && UnixStream::connect(&socket).is_ok() {
            return Ok(());
        }
        if !exited && let Ok(Some(status)) = child.try_wait() {
            if !status.success() {
                return Err(unavailable(format!(
                    "the daemon exited during startup ({status}): {}",
                    log_tail(&log_path, log_start)
                )));
            }
            // A daemon of this version already serves; wait for it.
            exited = true;
        }
        if Instant::now() >= deadline {
            return Err(unavailable(format!(
                "the daemon did not serve {} within {} s; see {}",
                socket.display(),
                STARTUP.as_secs(),
                log_path.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Whether `request` is `shutdown`, which never starts a daemon.
fn stops(request: &Map<String, Value>) -> bool {
    request.get("action").and_then(Value::as_str) == Some("shutdown")
}

/// Sends `request` to the endpoint's daemon, starting one first when none
/// of this build serves, and returns the reply. `shutdown` never starts a
/// daemon.
pub fn call(endpoint: &Endpoint, request: &Map<String, Value>) -> Result<Reply, Failure> {
    call_reading(endpoint, request, Data::Whole)
}

/// [`call`], the reply's `data` read as `data` says.
fn call_reading(
    endpoint: &Endpoint,
    request: &Map<String, Value>,
    data: Data,
) -> Result<Reply, Failure> {
    let socket = endpoint.socket();
    check_socket(&socket)?;
    if stops(request) {
        return match send(&socket, request, data) {
            Err(Unsent::Unreachable(_)) => Ok(Reply {
                success: true,
                data: serde_json::json!({"stopped": false}),
                error: None,
                code: None,
                warning: Some("no daemon was running".to_owned()),
            }),
            other => other.map_err(Failure::from),
        };
    }
    let exe = daemon_executable(endpoint.daemon.as_deref())?;
    let version = daemon_version(&exe);
    if endpoint.version().as_deref() != Some(version.as_str()) {
        start(endpoint, &exe, &version)?;
    }
    match send(&socket, request, data) {
        Err(Unsent::Unreachable(_)) => {
            start(endpoint, &exe, &version)?;
            send(&socket, request, data).map_err(Failure::from)
        }
        other => other.map_err(Failure::from),
    }
}

/// Whether a process that sends the daemon many requests, such as the MCP
/// server, found a daemon of the build it would start serving the
/// endpoint, as [`call_checked`] keeps it from one request to the next.
#[derive(Debug, Default)]
pub struct Checked(AtomicBool);

/// Sends `request` as [`call`] does, for a process that sends the daemon
/// many requests, such as the MCP server: the daemon's version is checked
/// on the first request through `checked` and again only after a request
/// that found no daemon listening, which starts one as [`call`] does, and
/// the requests between go straight to the socket. A daemon another build
/// started meanwhile serves them as it is. The check costs a request the
/// version file's open, which the host's inspection of file opens can hold
/// for a millisecond or more (design D3 of `profiling`), and the
/// executable's lookup. The reply's `data` is read as `data` says.
pub fn call_checked(
    endpoint: &Endpoint,
    request: &Map<String, Value>,
    checked: &Checked,
    data: Data,
) -> Result<Reply, Failure> {
    if checked.0.load(Ordering::Acquire) {
        match send(&endpoint.socket(), request, data) {
            Err(Unsent::Unreachable(_)) => checked.0.store(false, Ordering::Release),
            sent => return sent.map_err(Failure::from),
        }
    }
    let reply = call_reading(endpoint, request, data)?;
    if !stops(request) {
        checked.0.store(true, Ordering::Release);
    }
    Ok(reply)
}

/// A reply read without the refs map of its listing or delta
/// ([`Data::WithoutRefs`]): the map's entries are skipped as the parser
/// meets them, never built into values, the rest read into values as
/// `serde_json` reads them.
mod lean {
    use std::fmt;

    use serde::de::{
        Deserialize, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor,
    };
    use serde_json::{Map, Value};

    /// Where a value lies in the envelope.
    #[derive(Clone, Copy)]
    enum Level {
        Envelope,
        Data,
        Snapshot,
    }

    /// How the value under a key is read.
    enum Child {
        Skipped,
        Whole,
        Lean(Level),
    }

    impl Level {
        fn child(self, key: &str) -> Child {
            match (self, key) {
                (Level::Envelope, "data") => Child::Lean(Level::Data),
                (Level::Data, "snapshot") => Child::Lean(Level::Snapshot),
                (Level::Data | Level::Snapshot, "refs") => Child::Skipped,
                _ => Child::Whole,
            }
        }
    }

    /// The envelope read without the refs map.
    pub struct Lean(pub Value);

    impl<'de> Deserialize<'de> for Lean {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Lean, D::Error> {
            Read(Level::Envelope).deserialize(deserializer).map(Lean)
        }
    }

    /// Reads the value at a level.
    struct Read(Level);

    impl<'de> DeserializeSeed<'de> for Read {
        type Value = Value;

        fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
            deserializer.deserialize_any(self)
        }
    }

    impl<'de> Visitor<'de> for Read {
        type Value = Value;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a JSON value")
        }

        fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
            Ok(Value::Bool(value))
        }

        fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
            Ok(value.into())
        }

        fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
            Ok(value.into())
        }

        fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
            Ok(value.into())
        }

        fn visit_str<E>(self, value: &str) -> Result<Value, E> {
            Ok(value.into())
        }

        fn visit_string<E>(self, value: String) -> Result<Value, E> {
            Ok(value.into())
        }

        fn visit_unit<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }

        fn visit_none<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }

        fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
            Value::deserialize(deserializer)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut items = Vec::new();
            while let Some(item) = seq.next_element::<Value>()? {
                items.push(item);
            }
            Ok(Value::Array(items))
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
            let mut object = Map::new();
            while let Some(key) = map.next_key::<String>()? {
                match self.0.child(&key) {
                    Child::Skipped => {
                        map.next_value::<IgnoredAny>()?;
                    }
                    Child::Whole => {
                        let value = map.next_value::<Value>()?;
                        object.insert(key, value);
                    }
                    Child::Lean(level) => {
                        let value = map.next_value_seed(Read(level))?;
                        object.insert(key, value);
                    }
                }
            }
            Ok(Value::Object(object))
        }
    }
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
        assert_eq!(
            read_timeout(&request(json!({"action": "wait", "text": "Welcome"}))),
            Duration::from_secs(610),
            "a session default can make the wait long"
        );
        assert_eq!(
            read_timeout(&request(json!({"action": "click", "target": "#submit"}))),
            Duration::from_secs(610)
        );
        assert_eq!(
            read_timeout(&request(json!({"action": "click", "target": "@e3"}))),
            Duration::from_secs(30),
            "a ref never waits"
        );
        assert_eq!(
            read_timeout(&request(
                json!({"action": "scroll", "direction": "down", "until": "@e40"})
            )),
            Duration::from_secs(610),
            "a scroll until an element shows waits, even for a ref"
        );
        assert_eq!(
            read_timeout(&request(json!({"action": "open", "app": "com.example"}))),
            Duration::from_secs(610),
            "an open waits for the app's first tree as long as the session's default"
        );
    }

    #[test]
    fn replies_keep_the_envelope_fields() {
        let reply = Reply::parse(
            r#"{"id":"r1","success":false,"data":null,"error":"gone","code":"stale_ref"}"#,
            Data::Whole,
        )
        .unwrap();
        assert!(!reply.success);
        assert_eq!(reply.code.as_deref(), Some("stale_ref"));
        assert_eq!(reply.error.as_deref(), Some("gone"));
        assert!(Reply::parse("[1]", Data::Whole).is_none());
        assert!(Reply::parse("[1]", Data::WithoutRefs).is_none());
    }

    /// A directory of its own for a test, removed on drop, with a short
    /// path so that its socket's fits in a Unix socket address.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let base = std::env::temp_dir();
            let base = if base.as_os_str().len() > 60 {
                PathBuf::from("/tmp")
            } else {
                base
            };
            let dir = base.join(format!("mac-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("the test directory can be created");
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A daemon stand-in on `socket` that answers every request line with
    /// a success, until [`Standin::stop`].
    struct Standin {
        socket: PathBuf,
        stopping: std::sync::Arc<std::sync::atomic::AtomicBool>,
        thread: std::thread::JoinHandle<()>,
    }

    impl Standin {
        fn serve(socket: &Path) -> Standin {
            let listener =
                std::os::unix::net::UnixListener::bind(socket).expect("the socket binds");
            let stopping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stop = stopping.clone();
            let thread = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    if stop.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    let mut line = String::new();
                    if BufReader::new(&stream).read_line(&mut line).is_ok() {
                        let _ =
                            stream.write_all(b"{\"success\":true,\"data\":{},\"error\":null}\n");
                    }
                }
            });
            Standin {
                socket: socket.to_path_buf(),
                stopping,
                thread,
            }
        }

        /// Stops answering and removes the socket, as a daemon that stopped.
        fn stop(self) {
            self.stopping
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = UnixStream::connect(&self.socket);
            self.thread.join().expect("the stand-in ends");
            std::fs::remove_file(&self.socket).expect("the socket is removed");
        }
    }

    /// Design D3 of `profiling`: a process that sends many requests checks
    /// the daemon's version on its first and again only after one that
    /// found no daemon listening. The daemon executable here cannot run,
    /// so a check that finds another version fails to start it.
    #[test]
    fn a_checked_caller_reads_the_version_once_and_again_after_a_call_finds_no_daemon() {
        let scratch = Scratch::new("checked");
        let exe = scratch.0.join("placeholder-daemon");
        std::fs::write(&exe, "not a program").expect("the stand-in is written");
        let endpoint = Endpoint {
            dir: scratch.0.clone(),
            daemon: Some(exe.clone()),
            idle_timeout: None,
            emulator_controller: None,
        };
        std::fs::write(scratch.0.join(VERSION_FILE), daemon_version(&exe)).expect("writable");
        let daemon = Standin::serve(&endpoint.socket());
        let request = json!({"action": "status"}).as_object().cloned().unwrap();
        let checked = Checked::default();

        let first =
            call_checked(&endpoint, &request, &checked, Data::Whole).expect("the daemon answers");
        assert!(first.success, "{first:?}");
        // Another build's version: a check would start this build's daemon,
        // which cannot run.
        std::fs::write(scratch.0.join(VERSION_FILE), "0.0.1+placeholder-other\n")
            .expect("writable");
        assert!(
            call(&endpoint, &request)
                .is_err_and(|failure| failure.message.starts_with("cannot start the daemon")),
            "a call that checks the version starts the daemon"
        );
        for _ in 0..3 {
            let reply = call_checked(&endpoint, &request, &checked, Data::Whole)
                .expect("the checked daemon answers without a check");
            assert!(reply.success, "{reply:?}");
        }

        // No daemon listens: the next call checks again, and starts one.
        daemon.stop();
        let failure = call_checked(&endpoint, &request, &checked, Data::Whole)
            .expect_err("no daemon answers and none can start");
        assert!(
            failure.message.starts_with("cannot start the daemon"),
            "{failure:?}"
        );
    }

    /// Design D3 of `profiling`: a reply read without refs is the whole
    /// reply less the refs map at the top of `data` and in its `snapshot`
    /// object, every other field as it is, a `refs` key elsewhere among
    /// them.
    #[test]
    fn a_reply_read_without_refs_is_the_whole_reply_less_the_listing_s_refs_map() {
        let line = json!({
            "id": "r1",
            "success": true,
            "data": {
                "snapshot": {"kind": "full", "revision": 3, "tree": "- button \"Sign in\" [ref=e1]",
                             "refs": {"e1": {"name": "Sign in", "role": "button"}}},
                "refs": {"e1": {"name": "Sign in", "role": "button"}, "e2": {"name": "Ok", "role": "button"}},
                "removedRefs": ["e0"],
                "readMs": 0.25,
                "replicaRevision": 3,
                "servedFromReplica": true,
                "target": "emulator-5554",
                "element": {"refs": ["kept"], "inView": null},
                "changes": [{"op": "add", "ref": "e2", "node": {"name": "Ok", "role": "button"}}],
            },
            "error": null,
            "warning": "a warning",
        })
        .to_string();
        let whole = Reply::parse(&line, Data::Whole).expect("an envelope");
        let lean = Reply::parse(&line, Data::WithoutRefs).expect("an envelope");
        let mut expected = whole.clone();
        let data = expected.data.as_object_mut().expect("an object");
        data.remove("refs");
        data["snapshot"]
            .as_object_mut()
            .expect("a delta object")
            .remove("refs");
        assert_eq!(lean, expected);
        assert_eq!(lean.data["element"]["refs"], json!(["kept"]));
        // A listing's text, a reply without data and a failure.
        for line in [
            r#"{"id":"r2","success":true,"data":{"snapshot":"- button","refs":{"e1":{"name":"a","role":"b"}},"origin":"app"},"error":null}"#,
            r#"{"id":"r3","success":true,"data":null,"error":null}"#,
            r#"{"id":"r4","success":false,"data":null,"error":"gone","code":"stale_ref"}"#,
            r#"{"success":true,"data":{"n":-1,"m":18446744073709551615,"f":1.5,"t":true,"a":[1,"x",null],"o":{}},"error":null}"#,
        ] {
            let mut whole = Reply::parse(line, Data::Whole).expect("an envelope");
            if let Some(data) = whole.data.as_object_mut() {
                data.remove("refs");
            }
            assert_eq!(Reply::parse(line, Data::WithoutRefs), Some(whole), "{line}");
        }
        assert!(Reply::parse(r#"{"success":true,"data":{"refs":{}}"#, Data::WithoutRefs).is_none());
        assert!(Reply::parse(r#"{"success":true} trailing"#, Data::WithoutRefs).is_none());
    }
}
