//! Starts the daemon binary on a private socket directory with fake targets
//! and talks to it as the CLI does: one JSON line per request and one per
//! reply, over the Unix socket.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde_json::Value;

/// The daemon binary under test.
pub const BIN: &str = env!("CARGO_BIN_EXE_mobile-agent-daemon");

pub const SOCKET: &str = "daemon.sock";
pub const PID: &str = "daemon.pid";
pub const VERSION: &str = "daemon.version";

/// The fixture app every fake target has installed.
pub const APP: &str = "dev.mobileagent.fixture.expo";

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A private directory, removed on drop. Its path stays short, because a
/// socket path must fit in 103 bytes.
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
        let dir = base.join(format!("mad-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the test directory can be created");
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// How to start a daemon.
pub struct Builder {
    socket_dir: Option<PathBuf>,
    env: Vec<(String, String)>,
    removed: Vec<String>,
    args: Vec<String>,
}

impl Builder {
    /// Fake targets as `MOBILE_AGENT_FAKE_TARGETS` lists them.
    pub fn fake(mut self, spec: &str) -> Builder {
        self.env
            .push(("MOBILE_AGENT_FAKE_TARGETS".into(), spec.into()));
        self
    }

    pub fn env(mut self, key: &str, value: &str) -> Builder {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Starts the daemon without `key` in its environment.
    pub fn without_env(mut self, key: &str) -> Builder {
        self.removed.push(key.into());
        self
    }

    pub fn arg(mut self, arg: &str) -> Builder {
        self.args.push(arg.into());
        self
    }

    /// Serves on `dir` instead of a fresh directory.
    pub fn socket_dir(mut self, dir: &Path) -> Builder {
        self.socket_dir = Some(dir.to_path_buf());
        self
    }

    /// Starts the daemon without waiting for its socket.
    pub fn spawn(self) -> Daemon {
        let (dir, temp) = match self.socket_dir {
            Some(dir) => (dir, None),
            None => {
                let temp = TempDir::new();
                (temp.path().to_path_buf(), Some(temp))
            }
        };
        let mut command = Command::new(BIN);
        clean_env(&mut command);
        command
            .arg("--socket-dir")
            .arg(&dir)
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        // A daemon a failed test leaves behind exits on its own.
        command.env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "60000");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        for key in &self.removed {
            command.env_remove(key);
        }
        let child = command.spawn().expect("the daemon binary starts");
        Daemon {
            dir,
            child,
            _temp: temp,
        }
    }

    /// Starts the daemon and waits until it accepts connections.
    pub fn start(self) -> Daemon {
        let mut daemon = self.spawn();
        daemon.wait_ready();
        daemon
    }
}

/// Removes every `MOBILE_AGENT_*` variable the developer's shell may carry,
/// so a test sees only what it sets.
pub fn clean_env(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("MOBILE_AGENT_") {
            command.env_remove(key);
        }
    }
}

/// A daemon process on its own socket directory, killed on drop.
pub struct Daemon {
    pub dir: PathBuf,
    child: Child,
    _temp: Option<TempDir>,
}

impl Daemon {
    pub fn builder() -> Builder {
        Builder {
            socket_dir: None,
            env: Vec::new(),
            removed: Vec::new(),
            args: Vec::new(),
        }
    }

    /// A daemon serving the fake targets `spec`.
    pub fn start(spec: &str) -> Daemon {
        Daemon::builder().fake(spec).start()
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.join(SOCKET)
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Waits until the socket accepts connections, failing the test with the
    /// daemon's error output if it exits first.
    pub fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if UnixStream::connect(self.socket()).is_ok() {
                return;
            }
            if let Some(status) = self.child.try_wait().expect("the daemon can be polled") {
                panic!(
                    "the daemon exited with {status} before accepting connections: {}",
                    self.stderr()
                );
            }
            assert!(
                Instant::now() < deadline,
                "the daemon did not accept connections within 10 s"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Sends one request and returns the reply.
    pub fn request(&self, request: Value) -> Value {
        request_at(&self.socket(), &request)
    }

    /// Waits up to `within` for the process to exit.
    pub fn wait_exit(&mut self, within: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().expect("the daemon can be polled") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn is_running(&mut self) -> bool {
        self.child
            .try_wait()
            .expect("the daemon can be polled")
            .is_none()
    }

    /// What the daemon wrote to its error output, once it has exited.
    pub fn stderr(&mut self) -> String {
        let mut text = String::new();
        if let Some(mut stderr) = self.child.stderr.take() {
            use std::io::Read;
            let _ = stderr.read_to_string(&mut text);
        }
        text
    }

    pub fn file(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(name)).ok()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Sends one request line on a fresh connection and reads one reply line.
pub fn request_at(socket: &Path, request: &Value) -> Value {
    let reply = raw_request(socket, &request.to_string());
    serde_json::from_str(&reply).unwrap_or_else(|e| panic!("reply {reply:?} is not JSON: {e}"))
}

/// Sends `line` as it is and returns the reply line.
pub fn raw_request(socket: &Path, line: &str) -> String {
    let mut stream = UnixStream::connect(socket)
        .unwrap_or_else(|e| panic!("cannot connect to {}: {e}", socket.display()));
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("a read timeout can be set");
    stream
        .write_all(format!("{line}\n").as_bytes())
        .expect("the request is written");
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .expect("a reply line arrives");
    reply
}

/// The daemon's version as its version file records it: the crate version
/// and the executable's size and modification time in hex, so a rebuilt
/// binary reads as another version.
pub fn expected_version() -> String {
    let meta = std::fs::metadata(BIN).expect("the daemon binary exists");
    let mtime = meta
        .modified()
        .expect("the file system records modification times")
        .duration_since(UNIX_EPOCH)
        .expect("the binary is newer than 1970")
        .as_nanos();
    format!("{}+{:x}.{:x}", env!("CARGO_PKG_VERSION"), meta.len(), mtime)
}

/// Whether process `pid` exists.
pub fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
