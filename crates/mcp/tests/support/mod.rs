//! Runs the `mobile-agent-mcp` binary over its standard input and output
//! against a daemon with fake targets on a private socket directory, with a
//! clean environment as the CLI's tests have it (`crates/cli/tests/support`):
//! no `MOBILE_AGENT_*` variables from the developer's shell, no device
//! driver, and `HOME` and the working directory inside the test directory.
//!
//! Two clients speak to the server, one per protocol era: rmcp's own client
//! ([`Rmcp`]), which opens with `initialize` as clients of the 2025
//! revisions do, and a JSON-RPC exchange written out here ([`Raw`]) in the
//! 2026-07-28 revision, which opens with `server/discover` and carries the
//! protocol version, the client's info and its capabilities in every
//! request's `_meta`, as spike S13's client sent them.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use rmcp::model::CallToolRequestParams;
use rmcp::serve_client;
use rmcp::service::{RoleClient, RunningService};
use serde_json::{Value, json};

/// The server binary under test.
pub const SERVER: &str = env!("CARGO_BIN_EXE_mobile-agent-mcp");

/// The fixture app every fake target has installed.
pub const APP: &str = "dev.mobileagent.fixture.expo";

/// The revision of the stateless era.
pub const STATELESS: &str = "2026-07-28";

/// How long a test waits for the server's answer to one message.
const ANSWER: Duration = Duration::from_secs(60);

/// How long a server gets to exit once its input ends or it is signalled.
const EXIT: Duration = Duration::from_secs(20);

static NEXT: AtomicU32 = AtomicU32::new(0);

/// The daemon binary, built beside the server: `cargo test --workspace`
/// builds it already, and a run of this crate's tests alone builds it here.
pub fn daemon_bin() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = Path::new(SERVER)
            .parent()
            .expect("the server lives in a directory");
        let mut cargo = Command::new(env!("CARGO"));
        cargo
            .args([
                "build",
                "--quiet",
                "-p",
                "daemon",
                "--bin",
                "mobile-agent-daemon",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        if dir.file_name().is_some_and(|name| name == "release") {
            cargo.arg("--release");
        }
        let status = cargo.status().expect("cargo runs");
        assert!(status.success(), "cargo build -p daemon failed");
        dir.join("mobile-agent-daemon")
    })
    .clone()
}

/// The CLI binary, built beside the server as [`daemon_bin`] builds the
/// daemon, to compare the server's text with what the CLI prints.
pub fn cli_bin() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = Path::new(SERVER)
            .parent()
            .expect("the server lives in a directory");
        let mut cargo = Command::new(env!("CARGO"));
        cargo
            .args(["build", "--quiet", "-p", "cli", "--bin", "mobile-agent"])
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        if dir.file_name().is_some_and(|name| name == "release") {
            cargo.arg("--release");
        }
        let status = cargo.status().expect("cargo runs");
        assert!(status.success(), "cargo build -p cli failed");
        dir.join("mobile-agent")
    })
    .clone()
}

/// A private directory, removed on drop, with a short path so that the
/// socket path fits in 103 bytes.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir();
        let base = if base.as_os_str().len() > 60 {
            PathBuf::from("/tmp")
        } else {
            base
        };
        let dir = base.join(format!("mam-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).expect("the test directory can be created");
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Removes the `MOBILE_AGENT_*` variables the developer's shell may carry.
pub fn clean_env(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("MOBILE_AGENT_") {
            command.env_remove(key);
        }
    }
    command.env_remove("XDG_RUNTIME_DIR");
}

/// A socket directory with a daemon serving fake targets, the environment
/// every server of the test gets, and the daemon's socket to ask directly.
pub struct TestEnv {
    pub dir: TempDir,
    pub fake: String,
    /// Variables the daemon runs with beyond the fake's, such as the fake
    /// driver's `MOBILE_AGENT_FAKE_ACTIVATION_MS`.
    extra: Vec<(String, String)>,
    daemon: Option<Child>,
}

impl TestEnv {
    /// A directory with a daemon serving `fake`, as
    /// `MOBILE_AGENT_FAKE_TARGETS` lists targets.
    pub fn start(fake: &str) -> TestEnv {
        TestEnv::start_with(fake, &[])
    }

    /// As [`TestEnv::start`], with `extra` variables for the daemon.
    pub fn start_with(fake: &str, extra: &[(&str, &str)]) -> TestEnv {
        let env = TestEnv {
            dir: TempDir::new(),
            fake: fake.to_owned(),
            extra: extra
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            daemon: None,
        };
        env.started()
    }

    fn started(mut self) -> TestEnv {
        let mut command = Command::new(daemon_bin());
        clean_env(&mut command);
        let child = command
            .arg("--socket-dir")
            .arg(self.path())
            .envs(self.daemon_vars())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the daemon starts");
        let pid = child.id();
        self.daemon = Some(child);
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.daemon_pid() != Some(pid) || UnixStream::connect(self.socket()).is_err() {
            assert!(
                Instant::now() < deadline,
                "the daemon did not serve within 10 s"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        self
    }

    pub fn path(&self) -> &Path {
        &self.dir.0
    }

    pub fn socket(&self) -> PathBuf {
        self.path().join("daemon.sock")
    }

    /// The pid the daemon recorded, if one serves.
    pub fn daemon_pid(&self) -> Option<u32> {
        std::fs::read_to_string(self.path().join("daemon.pid"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// What a daemon started for the test, or by a server, runs with.
    fn daemon_vars(&self) -> Vec<(String, String)> {
        let mut vars = vec![
            ("MOBILE_AGENT_FAKE_TARGETS".into(), self.fake.clone()),
            ("MOBILE_AGENT_DRIVERS".into(), "none".into()),
            ("MOBILE_AGENT_IDLE_TIMEOUT_MS".into(), "60000".into()),
        ];
        vars.extend(self.extra.iter().cloned());
        vars
    }

    /// The environment every server gets.
    pub fn server_vars(&self) -> Vec<(String, String)> {
        let mut vars = self.daemon_vars();
        vars.extend([
            (
                "HOME".to_owned(),
                self.path().join("home").to_string_lossy().into_owned(),
            ),
            (
                "MOBILE_AGENT_SOCKET_DIR".to_owned(),
                self.path().to_string_lossy().into_owned(),
            ),
            (
                "MOBILE_AGENT_DAEMON_BIN".to_owned(),
                daemon_bin().to_string_lossy().into_owned(),
            ),
        ]);
        vars
    }

    /// The server command with the test's environment, its error output
    /// appended to `server.log` in the test directory.
    pub fn server_command(&self) -> Command {
        self.server_command_with(&[])
    }

    /// As [`TestEnv::server_command`], with `args` on the server's command
    /// line.
    pub fn server_command_with(&self, args: &[&str]) -> Command {
        let mut command = Command::new(SERVER);
        clean_env(&mut command);
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path().join("server.log"))
            .expect("the server's log opens");
        command
            .args(args)
            .envs(self.server_vars())
            .current_dir(self.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        command
    }

    /// What the servers of the test wrote on their error output.
    pub fn server_log(&self) -> String {
        std::fs::read_to_string(self.path().join("server.log")).unwrap_or_default()
    }

    /// Sends one request line to the daemon, as the CLI does, and returns
    /// its reply.
    pub fn request(&self, request: Value) -> Value {
        let mut stream = UnixStream::connect(self.socket()).expect("the daemon accepts");
        stream
            .set_read_timeout(Some(ANSWER))
            .expect("a read timeout");
        let mut line = request.to_string();
        line.push('\n');
        stream.write_all(line.as_bytes()).expect("the request goes");
        let mut reply = String::new();
        BufReader::new(stream)
            .read_line(&mut reply)
            .expect("a reply comes");
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply:?}"))
    }

    /// Runs the CLI with `args` in the test's environment and returns its
    /// standard output and error.
    pub fn cli(&self, args: &[&str]) -> (String, String) {
        let mut command = Command::new(cli_bin());
        clean_env(&mut command);
        let output = command
            .args(args)
            .envs(self.server_vars())
            .current_dir(self.path())
            .stdin(Stdio::null())
            .output()
            .expect("the CLI runs");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// The daemon's `status` data.
    pub fn status(&self) -> Value {
        let reply = self.request(json!({"action": "status"}));
        assert_eq!(reply["success"], true, "{reply}");
        reply["data"].clone()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if let Ok(mut stream) = UnixStream::connect(self.socket()) {
            let _ = stream.write_all(b"{\"action\":\"shutdown\"}\n");
            let _ = stream.read(&mut [0u8; 64]);
        }
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Sends `pid` the signal `name`, such as `TERM`.
pub fn signal(pid: u32, name: &str) {
    let status = Command::new("kill")
        .args([&format!("-{name}"), &pid.to_string()])
        .status()
        .expect("kill runs");
    assert!(status.success(), "kill -{name} {pid}");
}

/// Waits for `child` to exit, up to [`EXIT`].
#[track_caller]
fn exited(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + EXIT;
    loop {
        if let Some(status) = child.try_wait().expect("the server can be polled") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the server did not exit within {} s", EXIT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A message the server sent that answered no request of the client's: a
/// notification, or a response to a request the client cancelled.
pub struct Notification {
    /// When it arrived.
    pub at: Instant,
    pub message: Value,
}

/// The 2026-07-28 era's client: JSON-RPC lines over the server's standard
/// input and output, one request in flight, every request's `_meta` naming
/// the revision, the client and its capabilities.
pub struct Raw {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next: u64,
    /// The messages that answered no awaited request, in the order they
    /// came: notifications.
    pub notifications: Vec<Notification>,
    /// The result of `server/discover`, for a server opened with it.
    pub discovered: Value,
}

impl Raw {
    /// Starts a server and opens it with `server/discover`.
    pub fn spawn(env: &TestEnv) -> Raw {
        Raw::spawn_with(env, &[])
    }

    /// Starts a server with `args` on its command line and opens it with
    /// `server/discover`.
    pub fn spawn_with(env: &TestEnv, args: &[&str]) -> Raw {
        let mut raw = Raw::start_with(env, args);
        let discovered = raw.request("server/discover", json!({}));
        assert!(
            discovered.get("result").is_some(),
            "server/discover: {discovered}"
        );
        raw.discovered = discovered["result"].clone();
        raw
    }

    /// Starts a server and sends it nothing.
    pub fn start(env: &TestEnv) -> Raw {
        Raw::start_with(env, &[])
    }

    /// Starts a server with `args` on its command line and sends it
    /// nothing.
    pub fn start_with(env: &TestEnv, args: &[&str]) -> Raw {
        let mut child = env
            .server_command_with(args)
            .spawn()
            .expect("the server starts");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("a stdout pipe");
        let (send, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Raw {
            child,
            stdin,
            lines,
            next: 1,
            notifications: Vec::new(),
            discovered: Value::Null,
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The session the server mints.
    pub fn session(&self) -> String {
        format!("mcp-{}", self.pid())
    }

    fn meta() -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion": STATELESS,
            "io.modelcontextprotocol/clientInfo": {"name": "mcp-tests", "version": "0.1.0"},
            "io.modelcontextprotocol/clientCapabilities": {},
        })
    }

    /// Sends `method` with `params` and the era's `_meta`, and returns the
    /// response: its `result` or its `error`.
    #[track_caller]
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        self.response(id)
    }

    /// Writes one message to the server.
    fn write(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("the server's input is open");
        let mut line = message.to_string();
        line.push('\n');
        stdin.write_all(line.as_bytes()).expect("the server reads");
        stdin.flush().expect("the server reads");
    }

    /// Sends `method` with `params`, whose `_meta` gains the era's keys
    /// beside what it holds, such as a `progressToken`, and returns the
    /// request's id without waiting for the response.
    pub fn send(&mut self, method: &str, mut params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        let mut meta = Raw::meta();
        if let Some(given) = params.get("_meta").and_then(Value::as_object) {
            for (key, value) in given {
                meta[key] = value.clone();
            }
        }
        params["_meta"] = meta;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    /// Sends the notification `method` with `params`.
    pub fn notify(&mut self, method: &str, params: Value) {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Waits for the response to the request `id`, keeping every other
    /// message that comes meanwhile in `notifications`.
    #[track_caller]
    pub fn response(&mut self, id: u64) -> Value {
        let deadline = Instant::now() + ANSWER;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => panic!("no answer to request {id} within 60 s"),
                Err(RecvTimeoutError::Disconnected) => panic!("the server closed its output"),
            };
            let message: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("the server wrote no JSON ({e}): {line}"));
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return message;
            }
            self.notifications.push(Notification {
                at: Instant::now(),
                message,
            });
        }
    }

    /// The progress notifications that came for `token`: each one's
    /// `progress`, its `message` and when it arrived.
    pub fn progress(&self, token: &str) -> Vec<(f64, String, Instant)> {
        self.notifications
            .iter()
            .filter(|notification| {
                notification.message["method"] == "notifications/progress"
                    && notification.message["params"]["progressToken"] == token
            })
            .map(|notification| {
                let params = &notification.message["params"];
                (
                    params["progress"].as_f64().unwrap_or(-1.0),
                    params["message"].as_str().unwrap_or_default().to_owned(),
                    notification.at,
                )
            })
            .collect()
    }

    /// The tools `tools/list` gives.
    #[track_caller]
    pub fn list_tools(&mut self) -> Vec<Value> {
        let listed = self.request("tools/list", json!({}));
        listed["result"]["tools"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| panic!("tools/list: {listed}"))
    }

    /// Calls `name` with `arguments` and returns the call's result.
    #[track_caller]
    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        let called = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        called
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("tools/call {name}: {called}"))
    }

    /// Closes the server's input and waits for it to exit.
    pub fn close(mut self) -> ExitStatus {
        drop(self.stdin.take());
        exited(&mut self.child)
    }

    /// Sends the server the signal `name`, such as `TERM`, its input left
    /// open, and waits for it to exit.
    pub fn signal(mut self, name: &str) -> ExitStatus {
        signal(self.pid(), name);
        exited(&mut self.child)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The 2025 era's client: rmcp's own, which opens with `initialize`.
pub struct Rmcp {
    runtime: tokio::runtime::Runtime,
    service: Option<RunningService<RoleClient, ()>>,
    child: tokio::process::Child,
}

impl Rmcp {
    /// Starts a server and initializes it.
    pub fn spawn(env: &TestEnv) -> Rmcp {
        Rmcp::spawn_with(env, &[])
    }

    /// Starts a server with `args` on its command line and initializes it.
    pub fn spawn_with(env: &TestEnv, args: &[&str]) -> Rmcp {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let mut command = tokio::process::Command::from(env.server_command_with(args));
        command.kill_on_drop(true);
        let (child, service) = runtime.block_on(async {
            let mut child = command.spawn().expect("the server starts");
            let stdin = child.stdin.take().expect("a stdin pipe");
            let stdout = child.stdout.take().expect("a stdout pipe");
            let service = serve_client((), (stdout, stdin))
                .await
                .expect("the server answers initialize");
            (child, service)
        });
        Rmcp {
            runtime,
            service: Some(service),
            child,
        }
    }

    fn service(&self) -> &RunningService<RoleClient, ()> {
        self.service.as_ref().expect("the client is open")
    }

    pub fn pid(&self) -> u32 {
        self.child.id().expect("the server runs")
    }

    /// The session the server mints.
    pub fn session(&self) -> String {
        format!("mcp-{}", self.pid())
    }

    /// The server's answer to `initialize`.
    pub fn initialized(&self) -> Value {
        serde_json::to_value(self.service().peer_info()).expect("the answer serializes")
    }

    /// The tools the server lists, as JSON.
    pub fn list_tools(&self) -> Vec<Value> {
        let tools = self
            .runtime
            .block_on(self.service().list_all_tools())
            .expect("tools/list answers");
        tools
            .iter()
            .map(|tool| serde_json::to_value(tool).expect("a tool serializes"))
            .collect()
    }

    /// Calls `name` with `arguments` and returns the call's result as JSON.
    #[track_caller]
    pub fn call(&self, name: &str, arguments: Value) -> Value {
        let Value::Object(arguments) = arguments else {
            panic!("arguments are an object");
        };
        let params = CallToolRequestParams::new(name.to_owned()).with_arguments(arguments);
        let result = self
            .runtime
            .block_on(self.service().call_tool(params))
            .unwrap_or_else(|e| panic!("tools/call {name}: {e}"));
        serde_json::to_value(result).expect("a result serializes")
    }

    /// Ends the session, which closes the server's input, and waits for
    /// the server to exit.
    pub fn close(mut self) -> ExitStatus {
        let service = self.service.take().expect("the client is open");
        let _ = self.runtime.block_on(service.cancel());
        let deadline = Instant::now() + EXIT;
        loop {
            if let Some(status) = self.child.try_wait().expect("the server can be polled") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the server did not exit within {} s",
                EXIT.as_secs()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The text of a result's content, its text blocks joined by newlines.
#[track_caller]
pub fn text(result: &Value) -> String {
    let content = result["content"]
        .as_array()
        .unwrap_or_else(|| panic!("no content: {result}"));
    content
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a result is an execution error.
pub fn is_error(result: &Value) -> bool {
    result["isError"] == Value::Bool(true)
}

/// Asserts that a result succeeded and returns its text.
#[track_caller]
pub fn ok(result: &Value) -> String {
    assert!(!is_error(result), "an execution error: {result}");
    text(result)
}

/// The ref a listing gives the element with identifier `id`, as `@eN`.
#[track_caller]
pub fn ref_of(listing: &str, id: &str) -> String {
    let needle = format!("id={id},");
    let line = listing
        .lines()
        .find(|line| line.contains(&needle))
        .unwrap_or_else(|| panic!("no element {id} in:\n{listing}"));
    let start = line.find("ref=e").expect("the line has a ref") + 4;
    let digits: String = line[start + 1..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    format!("@e{digits}")
}

/// Whether a listing line ends with its ref, as `[..., ref=eN]`.
pub fn ends_with_ref(line: &str) -> bool {
    let Some(inner) = line.strip_suffix(']') else {
        return false;
    };
    let last = inner.rsplit(", ").next().unwrap_or(inner);
    let last = last.rsplit('[').next().unwrap_or(last);
    last.strip_prefix("ref=e")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Starts the client of `era`: `legacy` through rmcp, `stateless` written
/// out.
pub enum Client {
    Legacy(Box<Rmcp>),
    Stateless(Raw),
}

impl Client {
    pub fn spawn(env: &TestEnv, era: &str) -> Client {
        Client::spawn_with(env, era, &[])
    }

    /// Starts the client of `era` on a server with `args` on its command
    /// line.
    pub fn spawn_with(env: &TestEnv, era: &str, args: &[&str]) -> Client {
        match era {
            "legacy" => Client::Legacy(Box::new(Rmcp::spawn_with(env, args))),
            "stateless" => Client::Stateless(Raw::spawn_with(env, args)),
            other => panic!("no era {other}"),
        }
    }

    /// The instructions the server's opening carried.
    pub fn instructions(&self) -> String {
        let opened = match self {
            Client::Legacy(client) => client.initialized(),
            Client::Stateless(client) => client.discovered.clone(),
        };
        opened["instructions"]
            .as_str()
            .unwrap_or_else(|| panic!("no instructions in {opened}"))
            .to_owned()
    }

    pub fn list_tools(&mut self) -> Vec<Value> {
        match self {
            Client::Legacy(client) => client.list_tools(),
            Client::Stateless(client) => client.list_tools(),
        }
    }

    #[track_caller]
    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        match self {
            Client::Legacy(client) => client.call(name, arguments),
            Client::Stateless(client) => client.call(name, arguments),
        }
    }

    pub fn session(&self) -> String {
        match self {
            Client::Legacy(client) => client.session(),
            Client::Stateless(client) => client.session(),
        }
    }

    pub fn close(self) -> ExitStatus {
        match self {
            Client::Legacy(client) => client.close(),
            Client::Stateless(client) => client.close(),
        }
    }
}

/// Every protocol era a test runs in.
pub const ERAS: [&str; 2] = ["legacy", "stateless"];
