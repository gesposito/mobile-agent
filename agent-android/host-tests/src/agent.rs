//! Starting the agent on the device the way the daemon will: the jar pushed
//! to `/data/local/tmp`, `app_process` run through `adb shell` as the shell
//! user, the per-launch token in the environment and the per-launch socket
//! name on the command line.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::adb::Device;
use crate::client::Client;
use crate::random_hex;

/// The class `app_process` starts.
pub const MAIN_CLASS: &str = "dev.mobileagent.agent.Main";

/// The tag of the agent's lines in logcat.
pub const LOG_TAG: &str = "mobile-agent";

/// How long the device keeps an agent a test started before killing it, so a
/// test that dies cannot leave it holding UI automation.
const DEVICE_TIMEOUT_S: u32 = 600;

/// The agent jar build.sh writes, or `$MOBILE_AGENT_JAR`.
pub fn jar_path() -> PathBuf {
    std::env::var_os("MOBILE_AGENT_JAR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist/mobile-agent-android.jar"),
        PathBuf::from,
    )
}

/// How to start an agent.
#[derive(Clone, Debug)]
pub struct Launch {
    /// Arguments after `serve --socket <name>`.
    pub args: Vec<String>,
    /// The per-launch token passed in the environment; `None` passes none.
    pub token: Option<String>,
    /// More environment variables for the agent.
    pub env: Vec<(String, String)>,
}

impl Default for Launch {
    fn default() -> Launch {
        Launch {
            args: Vec::new(),
            token: Some(random_hex(16)),
            env: Vec::new(),
        }
    }
}

impl Launch {
    pub fn arg(mut self, arg: impl Into<String>) -> Launch {
        self.args.push(arg.into());
        self
    }
}

/// An agent process started by a test, killed by pid when dropped unless it
/// already exited.
pub struct Agent {
    device: Device,
    pub socket: String,
    pub token: Option<String>,
    /// The agent's pid, from its ready line.
    pub pid: u32,
    /// The key=value fields of its ready line.
    pub ready: HashMap<String, String>,
    /// From the `adb shell` spawn to the ready line.
    pub ready_after: Duration,
    child: Child,
    output: Arc<Mutex<Vec<String>>>,
    stdout: Receiver<String>,
    remote_jar: String,
}

impl Agent {
    /// Pushes the jar, starts the agent and waits for its ready line.
    pub fn launch(device: &Device, launch: Launch) -> Agent {
        let mut agent = Agent::spawn(device, launch);
        let started = Instant::now();
        let line = agent
            .wait_line(Duration::from_secs(20), |l| l.starts_with("ready "))
            .unwrap_or_else(|| panic!("the agent printed no ready line:\n{}", agent.transcript()));
        agent.ready_after += started.elapsed();
        agent.ready = fields(&line);
        agent.pid = agent.ready["pid"].parse().expect("a numeric pid");
        agent
    }

    /// Pushes the jar and starts the agent without waiting for anything.
    pub fn spawn(device: &Device, launch: Launch) -> Agent {
        let jar = jar_path();
        assert!(
            jar.is_file(),
            "no agent jar at {}; run agent-android/build.sh first",
            jar.display()
        );
        let id = random_hex(6);
        // A jar left by a run whose adb transport dropped before it cleaned
        // up, once no agent runs that could still use it.
        if device.processes_matching("mobile-agent ").is_empty() {
            let _ = device.shell_output("rm -f /data/local/tmp/mobile-agent-host-tests-*.jar");
        }
        let remote_jar = format!("/data/local/tmp/mobile-agent-host-tests-{id}.jar");
        device.push(&jar, &remote_jar);
        let socket = format!("mobile-agent-{id}");
        let mut command = String::new();
        for (k, v) in &launch.env {
            command.push_str(&format!("{k}={v} "));
        }
        if let Some(token) = &launch.token {
            command.push_str(&format!("MOBILE_AGENT_TOKEN={token} "));
        }
        command.push_str(&format!(
            "CLASSPATH={remote_jar} exec timeout -s KILL {DEVICE_TIMEOUT_S} app_process / {MAIN_CLASS} serve --socket {socket}"
        ));
        // An agent a test lost track of, when the adb transport drops, exits
        // on its own soon after; tests that care pass their own bound.
        if !launch.args.iter().any(|a| a == "--idle-exit-ms") {
            command.push_str(" --idle-exit-ms 10000");
        }
        for arg in &launch.args {
            command.push(' ');
            command.push_str(arg);
        }
        let started = Instant::now();
        let mut child = device
            .adb()
            .arg("shell")
            .arg(&command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn adb shell");
        let output = Arc::new(Mutex::new(Vec::new()));
        let (tx, stdout) = mpsc::channel();
        let out = child.stdout.take().expect("piped stdout");
        let sink = Arc::clone(&output);
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let line = line.trim_end_matches('\r').to_owned();
                sink.lock().unwrap().push(format!("out: {line}"));
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let err = child.stderr.take().expect("piped stderr");
        let sink = Arc::clone(&output);
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                sink.lock()
                    .unwrap()
                    .push(format!("err: {}", line.trim_end_matches('\r')));
            }
        });
        Agent {
            device: device.clone(),
            socket,
            token: launch.token,
            pid: 0,
            ready: HashMap::new(),
            ready_after: started.elapsed(),
            child,
            output,
            stdout,
            remote_jar,
        }
    }

    /// Connects a client and completes the handshake with this agent's token.
    pub fn connect(&self) -> Client {
        let mut client = Client::connect(&self.device, &self.socket, self.token.clone())
            .unwrap_or_else(|e| panic!("cannot open the agent's socket: {e}"));
        client
            .handshake(Duration::from_secs(10))
            .unwrap_or_else(|e| panic!("handshake failed: {e}\n{}", self.transcript()));
        client
    }

    /// Takes the agent's pid from a line it printed, such as its conflict
    /// report, so that dropping it can still kill it.
    pub fn adopt_pid(&mut self, line: &str) {
        if let Some(pid) = fields(line).get("pid").and_then(|p| p.parse().ok()) {
            self.pid = pid;
        }
    }

    /// Opens the agent's socket without a handshake.
    pub fn open(&self, token: Option<String>) -> Client {
        Client::connect(&self.device, &self.socket, token)
            .unwrap_or_else(|e| panic!("cannot open the agent's socket: {e}"))
    }

    /// The next stdout line matching `pred`, skipping others, within `timeout`.
    pub fn wait_line(&mut self, timeout: Duration, pred: impl Fn(&str) -> bool) -> Option<String> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.checked_duration_since(Instant::now())?;
            match self.stdout.recv_timeout(left) {
                Ok(line) if pred(&line) => return Some(line),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    /// Drops the stdout lines received so far, so the next [`Agent::wait_line`]
    /// sees only later ones.
    pub fn drain_lines(&mut self) {
        while self.stdout.try_recv().is_ok() {}
    }

    /// Everything the agent printed so far, stdout and stderr interleaved.
    pub fn transcript(&self) -> String {
        self.output.lock().unwrap().join("\n")
    }

    /// Waits until the `adb shell` running the agent ends, which happens when
    /// the agent process exits.
    pub fn wait_exit(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let end = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            if Instant::now() >= end {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Whether the agent process still exists on the device.
    pub fn alive(&self) -> bool {
        self.is_ours(self.pid)
    }

    /// Whether `pid` is still this agent: its command line names our socket.
    fn is_ours(&self, pid: u32) -> bool {
        let out = self.device.shell_output(&format!(
            "cat /proc/{pid}/cmdline 2>/dev/null | tr '\\0' ' '"
        ));
        String::from_utf8_lossy(&out.stdout).contains(&self.socket)
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The agent's debug lines in logcat, once one of them contains
    /// `needle`; panics when none does within 3 s.
    pub fn debug_lines(&self, needle: &str) -> Vec<String> {
        let tagged = format!(" D {LOG_TAG}: ");
        crate::poll(Duration::from_secs(3), Duration::from_millis(100), || {
            let lines: Vec<String> = self
                .device
                .shell(&format!("logcat -d --pid={} -s {LOG_TAG}:D", self.pid))
                .lines()
                .filter(|l| l.contains(&tagged))
                .map(str::to_owned)
                .collect();
            lines.iter().any(|l| l.contains(needle)).then_some(lines)
        })
        .unwrap_or_else(|| {
            panic!(
                "no debug line of pid {} in logcat contains {needle:?}",
                self.pid
            )
        })
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // The agent outlives the adb shell that started it, so it is killed by
        // pid: its own, and any process whose command line names its socket,
        // which is unique to this launch (the agent and its timeout parent).
        let mut pids: Vec<u32> = self
            .device
            .processes_matching(&self.socket)
            .iter()
            .filter_map(|l| l.split_whitespace().next()?.parse().ok())
            .collect();
        if self.pid != 0 && self.is_ours(self.pid) {
            pids.push(self.pid);
        }
        pids.sort_unstable();
        pids.dedup();
        if !pids.is_empty() {
            let list: Vec<String> = pids.iter().map(u32::to_string).collect();
            let _ = self
                .device
                .shell_output(&format!("kill -9 {}", list.join(" ")));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self
            .device
            .shell_output(&format!("rm -f {}", self.remote_jar));
    }
}

/// The `key=value` words of a line; the value runs to the next space.
pub fn fields(line: &str) -> HashMap<String, String> {
    line.split(' ')
        .filter_map(|w| w.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}
