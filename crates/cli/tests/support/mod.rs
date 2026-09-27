//! Runs the `mobile-agent` binary against a daemon with fake targets on a
//! private socket directory, with a clean environment: no `MOBILE_AGENT_*`
//! variables from the developer's shell, `HOME` and the working directory
//! inside the test directory so no user or project config applies.

#![allow(dead_code)]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

/// The CLI binary under test.
pub const CLI: &str = env!("CARGO_BIN_EXE_mobile-agent");

/// The fixture app every fake target has installed.
pub const APP: &str = "dev.mobileagent.fixture.expo";

static NEXT: AtomicU32 = AtomicU32::new(0);

/// The daemon binary, built beside the CLI: `cargo test --workspace` builds
/// it already, and a run of this crate's tests alone builds it here.
pub fn daemon_bin() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = Path::new(CLI)
            .parent()
            .expect("the CLI lives in a directory");
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
        let dir = base.join(format!("mac-{}-{n}", std::process::id()));
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

/// One CLI run.
#[derive(Debug)]
pub struct Run {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
}

impl Run {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Asserts the run succeeded.
    #[track_caller]
    pub fn ok(self) -> Run {
        assert!(
            self.success(),
            "exit {:?}\nstdout: {}\nstderr: {}",
            self.code,
            self.stdout,
            self.stderr
        );
        self
    }

    /// Asserts the run failed with exit status 1.
    #[track_caller]
    pub fn failed(self) -> Run {
        assert_eq!(
            self.code,
            Some(1),
            "expected exit status 1\nstdout: {}\nstderr: {}",
            self.stdout,
            self.stderr
        );
        self
    }

    /// Standard output as JSON.
    #[track_caller]
    pub fn json(&self) -> Value {
        serde_json::from_str(self.stdout.trim_end())
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {:?}", self.stdout))
    }
}

/// A socket directory with a daemon serving fake targets, and a way to run
/// the CLI against it.
pub struct TestEnv {
    pub dir: TempDir,
    pub fake: String,
    daemon: Option<Child>,
}

impl TestEnv {
    /// A directory without a daemon; the CLI's first command starts one.
    pub fn new(fake: &str) -> TestEnv {
        TestEnv {
            dir: TempDir::new(),
            fake: fake.to_owned(),
            daemon: None,
        }
    }

    /// A directory with a daemon already serving `fake`.
    pub fn start(fake: &str) -> TestEnv {
        let mut env = TestEnv::new(fake);
        env.start_daemon();
        env
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

    /// Starts a daemon on the directory and waits until it serves.
    pub fn start_daemon(&mut self) {
        let mut command = Command::new(daemon_bin());
        clean_env(&mut command);
        let mut child = command
            .arg("--socket-dir")
            .arg(self.path())
            .env("MOBILE_AGENT_FAKE_TARGETS", &self.fake)
            .env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "60000")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the daemon starts");
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.daemon_pid() != Some(child.id()) || UnixStream::connect(self.socket()).is_err() {
            assert!(
                child
                    .try_wait()
                    .expect("the daemon can be polled")
                    .is_none(),
                "the daemon exited before serving"
            );
            assert!(
                Instant::now() < deadline,
                "the daemon did not serve within 10 s"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        self.daemon = Some(child);
    }

    /// The CLI command with the test's environment.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(CLI);
        clean_env(&mut command);
        command
            .args(args)
            .current_dir(self.path())
            .env("HOME", self.path().join("home"))
            .env("MOBILE_AGENT_SOCKET_DIR", self.path())
            .env("MOBILE_AGENT_DAEMON_BIN", daemon_bin())
            .env("MOBILE_AGENT_FAKE_TARGETS", &self.fake)
            .env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "60000")
            .stdin(Stdio::null());
        command
    }

    /// Runs the CLI with `args`.
    pub fn cli(&self, args: &[&str]) -> Run {
        run(self.command(args), None)
    }

    /// Runs the CLI with `args` and `stdin`.
    pub fn cli_stdin(&self, args: &[&str], stdin: &str) -> Run {
        run(self.command(args), Some(stdin))
    }

    /// Runs the CLI with extra environment variables.
    pub fn cli_env(&self, args: &[&str], vars: &[(&str, &str)]) -> Run {
        let mut command = self.command(args);
        for (key, value) in vars {
            command.env(key, value);
        }
        run(command, None)
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if let Ok(mut stream) = UnixStream::connect(self.socket()) {
            let _ = stream.write_all(b"{\"action\":\"shutdown\"}\n");
            let _ = std::io::Read::read(&mut stream, &mut [0u8; 64]);
        }
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
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

fn run(mut command: Command, stdin: Option<&str>) -> Run {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let started = Instant::now();
    let mut child = command.spawn().expect("the CLI starts");
    if let Some(input) = stdin {
        let mut pipe = child.stdin.take().expect("a stdin pipe");
        pipe.write_all(input.as_bytes()).expect("stdin is written");
    }
    let output = child.wait_with_output().expect("the CLI finishes");
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        elapsed: started.elapsed(),
    }
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

/// Whether a snapshot line ends with its ref, as `[..., ref=eN]`.
pub fn ends_with_ref(line: &str) -> bool {
    let Some(inner) = line.strip_suffix(']') else {
        return false;
    };
    let last = inner.rsplit(", ").next().unwrap_or(inner);
    let last = last.rsplit('[').next().unwrap_or(last);
    last.strip_prefix("ref=e")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}
