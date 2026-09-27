//! Deploying the agent (android-driver requirements "Agent deployment
//! without installation", "Single ownership of UI automation" and "Agent
//! lifecycle"; design D2 and D3).
//!
//! [`Deployer::attach`] lists the agent processes on the device, then:
//!
//! 1. reuses the agent this host recorded for the device when it still runs,
//!    its hello carries the recorded token, and it speaks the protocol
//!    version and is the agent build this driver deploys;
//! 2. replaces a recorded agent that is not: one whose protocol the host
//!    refuses exits by itself (the host's welcome says `version_mismatch`),
//!    another build is asked to `shutdown`, and one that does not answer as
//!    recorded is killed by pid;
//! 3. reports any other agent as a [`Conflict`], since it serves another
//!    host, unless the client passes force;
//! 4. otherwise pushes the jar read-only to a per-launch path, starts the
//!    agent detached as the shell user with a per-launch socket name and
//!    token, waits for its ready line (or its conflict report, which names
//!    the likely holder of UI automation), deletes the jar, connects and
//!    checks the versions its hello reports.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use protocol::message::Call;
use protocol::session::{DisconnectReason, HandshakeError, HostConfig, Negotiated};
use protocol::{Capabilities, PROTOCOL_VERSION};

use super::connection::Connection;
use super::jar::AgentJar;
use super::launch::{
    ConflictLine, Holder, JAR_MODE, PROCESS_NAME, PS_COMMAND, ReadyLine, RunningAgent, SocketName,
    launch_command, parse_agents, parse_conflict, parse_ready, random_hex, remote_jar,
};
use super::records::{Record, Records};
use crate::adb::{self, Device, ShellPacket};

/// How long `shutdown` may take to be answered.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(2);

/// How long the stream of a refused launch is read for the agent's exit.
const CONFLICT_EXIT_WAIT: Duration = Duration::from_secs(3);

/// How to attach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployOptions {
    /// Take UI automation over: from a known automation client (the agent's
    /// `--force`), or from an agent this host did not start.
    pub force: bool,
    /// How long the agent outlives a host that went away without stopping it
    /// (`--idle-exit-ms`).
    pub idle_exit: Duration,
    /// Longest wait for the ready line. The agent retries a refused role for
    /// 2 s and waits up to 2 s for an active window.
    pub ready_timeout: Duration,
    /// Longest wait for the agent's hello.
    pub handshake_timeout: Duration,
    /// How long a replaced or stopped agent may take to exit before it is
    /// killed.
    pub exit_timeout: Duration,
    /// More arguments for `serve`, such as `--trace`.
    pub agent_args: Vec<String>,
}

impl Default for DeployOptions {
    fn default() -> DeployOptions {
        DeployOptions {
            force: false,
            idle_exit: Duration::from_secs(30),
            ready_timeout: Duration::from_secs(20),
            handshake_timeout: Duration::from_secs(5),
            exit_timeout: Duration::from_secs(3),
            agent_args: Vec::new(),
        }
    }
}

/// The agent a connection reaches.
#[derive(Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub serial: String,
    pub socket: String,
    pub pid: u32,
    /// The agent name and version its hello carries.
    pub agent: String,
    /// The negotiated protocol version.
    pub protocol: u16,
    pub capabilities: Capabilities,
    token: String,
}

impl fmt::Debug for AgentInfo {
    /// Leaves the token out.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentInfo")
            .field("serial", &self.serial)
            .field("socket", &self.socket)
            .field("pid", &self.pid)
            .field("agent", &self.agent)
            .field("protocol", &self.protocol)
            .field("capabilities", &self.capabilities)
            .finish()
    }
}

impl AgentInfo {
    /// The agent reached on `socket`, with what its handshake settled.
    #[cfg(test)]
    pub(crate) fn new(
        serial: &str,
        socket: &str,
        pid: u32,
        negotiated: &Negotiated,
        token: &str,
    ) -> AgentInfo {
        AgentInfo {
            serial: serial.to_owned(),
            socket: socket.to_owned(),
            pid,
            agent: negotiated.agent.clone(),
            protocol: negotiated.version,
            capabilities: negotiated.capabilities,
            token: token.to_owned(),
        }
    }

    /// The per-launch token its hello carries.
    pub fn token(&self) -> &str {
        &self.token
    }
}

/// How an attach went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A new agent was started.
    Launched(Box<Launched>),
    /// The running agent this host started was reused: no process started.
    Reused,
}

/// A launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launched {
    pub ready: ReadyLine,
    /// The incompatible agent the launch replaced.
    pub replaced: Option<Replaced>,
    /// The holder of UI automation that force stopped.
    pub took_over: Option<Holder>,
}

/// An incompatible agent that was stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replaced {
    pub pid: u32,
    pub socket: String,
    pub reason: Incompatibility,
    pub stopped: Stopped,
}

/// How an agent ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// It exited: on `shutdown`, or by itself after a refused hello.
    Exited,
    /// It was still there after the exit timeout and was killed by pid.
    Killed,
    /// It was gone already.
    Gone,
}

/// Why a running agent cannot be reused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incompatibility {
    /// No common protocol version: the agent speaks `agent`, this driver
    /// `host`.
    Protocol { agent: Vec<u16>, host: Vec<u16> },
    /// Another build of the agent.
    Version { reported: String, expected: String },
    /// It does not answer as recorded: no connection, another token, or no
    /// hello in time.
    Unreachable(String),
    /// Another build reporting the same version, as the agent before and
    /// after a change to its payload can.
    Build { running: String, deploying: String },
}

impl fmt::Display for Incompatibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Incompatibility::Protocol { agent, host } => write!(
                f,
                "it speaks protocol versions {agent:?} and this driver {host:?}"
            ),
            Incompatibility::Version { reported, expected } => {
                write!(f, "it is {reported} and this driver deploys {expected}")
            }
            Incompatibility::Unreachable(why) => {
                write!(f, "it does not answer as this host recorded it: {why}")
            }
            Incompatibility::Build { .. } => todo!("build identity"),
        }
    }
}

/// Whether an agent can be reused, from its handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compatibility {
    Compatible,
    Incompatible(Incompatibility),
}

/// Judges a running agent by its handshake: compatible when the host
/// accepted its hello and it is the build `expected_agent` names
/// (`mobile-agent-android <version>`).
pub fn compatibility(
    handshake: &Result<Negotiated, DisconnectReason>,
    expected_agent: &str,
) -> Compatibility {
    match handshake {
        Ok(negotiated) if negotiated.agent == expected_agent => Compatibility::Compatible,
        Ok(negotiated) => Compatibility::Incompatible(Incompatibility::Version {
            reported: negotiated.agent.clone(),
            expected: expected_agent.to_owned(),
        }),
        Err(DisconnectReason::Handshake(HandshakeError::NoCommonVersion { host, agent })) => {
            Compatibility::Incompatible(Incompatibility::Protocol {
                agent: agent.clone(),
                host: host.clone(),
            })
        }
        Err(reason) => {
            Compatibility::Incompatible(Incompatibility::Unreachable(reason.to_string()))
        }
    }
}

/// Judges a recorded agent's build against the one being deployed: an
/// empty recorded build, from a record written before builds were
/// recorded, is another build.
pub fn build_compatibility(recorded: &str, deploying: &str) -> Compatibility {
    let _ = (recorded, deploying);
    todo!("build identity")
}

/// Where an attach spent its time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timings {
    pub total: Duration,
    /// Listing the agent processes.
    pub listing: Duration,
    /// Checking and stopping an incompatible agent.
    pub replace: Option<Duration>,
    pub push: Option<Duration>,
    /// From opening the launch stream to the ready line.
    pub spawn_to_ready: Option<Duration>,
    /// Connecting and completing the handshake.
    pub handshake: Duration,
}

/// Whether an agent runs on a target, from the process list alone: an
/// attach would take the agent's connection away from whoever holds it, so
/// the state is read from the socket name, which says the version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentState {
    Absent,
    /// An agent of the version and protocol this driver deploys.
    Running(RunningAgent),
    /// An agent of another version or protocol, or whose name does not say.
    Incompatible(RunningAgent),
}

/// The state `agents` add up to for a driver deploying `build` of
/// `version` over `protocol`: running when one of them is that build, else
/// incompatible when any runs.
pub fn agent_state(
    agents: &[RunningAgent],
    version: &str,
    protocol: u16,
    build: &str,
) -> AgentState {
    let _ = (agents, version, protocol, build);
    todo!("build identity")
}

/// Another process holds UI automation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The likely holder, when the device's process and instrumentation
    /// lists name one.
    pub holder: Option<Holder>,
    /// The agent's own explanation, or the driver's for an agent it did not
    /// start.
    pub detail: String,
}

impl Conflict {
    /// A conflict with an agent this host did not start.
    pub fn foreign_agent(agent: &RunningAgent) -> Conflict {
        let build = match &agent.name {
            Some(name) => format!("version {} over protocol {}", name.version, name.protocol),
            None => "a version its socket name does not say".to_owned(),
        };
        Conflict {
            holder: Some(Holder {
                name: PROCESS_NAME.to_owned(),
                pid: Some(agent.pid),
            }),
            detail: format!(
                "an agent this host did not start runs on the device (pid {}, socket {}, {build}); \
                 another daemon or an earlier run of this one may be using it, and it exits \
                 by itself once no host has been connected for its idle bound",
                agent.pid, agent.socket
            ),
        }
    }

    /// What the user reads: the holder, the detail and how to take over.
    pub fn message(&self) -> String {
        let who = match &self.holder {
            Some(holder) => format!("likely {holder}"),
            None => "by a process the device does not name".to_owned(),
        };
        format!(
            "another process holds UI automation, {who}: {}. Attach again with force to take \
             it over, which stops that holder.",
            self.detail
        )
    }

    fn from_line(line: ConflictLine) -> Conflict {
        Conflict {
            holder: line.holder,
            detail: line.message,
        }
    }
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

/// Why an attach or a stop failed.
#[derive(Debug)]
pub enum DeployError {
    Adb(adb::Error),
    /// Another process holds UI automation; nothing was taken over.
    Conflict(Conflict),
    /// The agent ended before its ready line.
    Exited {
        output: String,
        exit: Option<u8>,
    },
    /// No ready line within the timeout; the agent was killed.
    NoReadyLine {
        waited: Duration,
        output: String,
    },
    /// The launched agent's handshake failed.
    Handshake {
        socket: String,
        reason: DisconnectReason,
    },
    /// The launched jar reports another build than this driver deploys; it
    /// was stopped.
    WrongVersion {
        reported: String,
        expected: String,
        jar: Option<PathBuf>,
    },
    /// The record of the agent could not be written.
    Records(io::Error),
}

impl fmt::Display for DeployError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeployError::Adb(error) => write!(f, "adb: {error}"),
            DeployError::Conflict(conflict) => f.write_str(&conflict.message()),
            DeployError::Exited { output, exit } => {
                let status = exit.map_or_else(|| "unknown".to_owned(), |code| code.to_string());
                write!(
                    f,
                    "the agent ended before it was ready (exit status {status}); its output: {output}"
                )
            }
            DeployError::NoReadyLine { waited, output } => write!(
                f,
                "the agent printed no ready line within {:.1} s and was stopped; its output: {output}",
                waited.as_secs_f64()
            ),
            DeployError::Handshake { socket, reason } => write!(
                f,
                "the agent listening on {socket} did not complete its handshake: {reason}"
            ),
            DeployError::WrongVersion {
                reported,
                expected,
                jar,
            } => {
                let jar = jar
                    .as_ref()
                    .map_or_else(|| "in memory".to_owned(), |p| p.display().to_string());
                write!(
                    f,
                    "the agent jar ({jar}) reports {reported}, but this driver deploys {expected}; \
                     rebuild it with agent-android/build.sh"
                )
            }
            DeployError::Records(error) => write!(f, "cannot record the agent: {error}"),
        }
    }
}

impl std::error::Error for DeployError {}

impl From<adb::Error> for DeployError {
    fn from(error: adb::Error) -> DeployError {
        DeployError::Adb(error)
    }
}

/// Pushes `jar` to `remote` read-only ([`super::JAR_MODE`]).
pub fn push_jar(device: &Device, jar: &AgentJar, remote: &str) -> Result<(), adb::Error> {
    device.push_bytes(jar.bytes(), remote, JAR_MODE)
}

/// What the launch stream showed.
enum Spawned {
    Ready(ReadyLine),
    Conflict(ConflictLine),
    Exited { output: String, exit: Option<u8> },
    Timeout { output: String },
}

/// Stdout lines as they complete, and everything else the stream carried.
#[derive(Default)]
struct Transcript {
    partial: Vec<u8>,
    lines: Vec<String>,
    stderr: String,
}

impl Transcript {
    /// Adds stdout bytes; returns the lines they completed.
    fn stdout(&mut self, data: &[u8]) -> Vec<String> {
        self.partial.extend_from_slice(data);
        let mut done = Vec::new();
        while let Some(end) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line).trim_end().to_owned();
            self.lines.push(line.clone());
            done.push(line);
        }
        done
    }

    fn text(&self) -> String {
        let mut text: Vec<String> = self.lines.iter().map(|l| format!("out: {l}")).collect();
        if !self.partial.is_empty() {
            text.push(format!("out: {}", String::from_utf8_lossy(&self.partial)));
        }
        text.extend(self.stderr.lines().map(|l| format!("err: {l}")));
        text.join("\n")
    }
}

/// Deploys one agent build on one device. Clones share the records.
#[derive(Clone, Debug)]
pub struct Deployer {
    device: Device,
    jar: AgentJar,
    records: Records,
}

/// A connection to an agent, with how it was obtained.
pub struct Attached {
    pub connection: Connection,
    pub agent: AgentInfo,
    pub outcome: Outcome,
    pub timings: Timings,
    deployer: Deployer,
}

impl fmt::Debug for Attached {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attached")
            .field("agent", &self.agent)
            .field("outcome", &self.outcome)
            .field("timings", &self.timings)
            .finish_non_exhaustive()
    }
}

impl Attached {
    /// The deployer that attached, to stop the agent later.
    pub fn deployer(&self) -> &Deployer {
        &self.deployer
    }

    /// Stops the agent: `shutdown`, then a kill by pid if it has not exited
    /// within `timeout`; forgets its record.
    pub fn stop(self, timeout: Duration) -> Result<Stopped, DeployError> {
        let Attached {
            connection,
            agent,
            deployer,
            ..
        } = self;
        deployer.stop_agent(&connection, agent.pid, &agent.socket, timeout)
    }

    /// The connection, the agent, the outcome and the deployer.
    pub(crate) fn into_parts(self) -> (Connection, AgentInfo, Outcome, Deployer) {
        (self.connection, self.agent, self.outcome, self.deployer)
    }
}

impl Deployer {
    pub fn new(device: Device, jar: AgentJar, records: Records) -> Deployer {
        Deployer {
            device,
            jar,
            records,
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn jar(&self) -> &AgentJar {
        &self.jar
    }

    pub fn records(&self) -> &Records {
        &self.records
    }

    /// The agent processes on the device.
    pub fn running(&self) -> Result<Vec<RunningAgent>, adb::Error> {
        let listing = self.device.shell(PS_COMMAND)?;
        Ok(parse_agents(&listing.stdout_text()))
    }

    /// The target's agent state, from the process list.
    pub fn state(&self) -> Result<AgentState, adb::Error> {
        Ok(agent_state(
            &self.running()?,
            self.jar.version(),
            PROTOCOL_VERSION,
            &self.jar.build(),
        ))
    }

    /// Connects to an agent: the running one this host started when it is
    /// compatible, else a new one. Any connection the reused agent had is
    /// replaced by this one.
    pub fn attach(&self, options: &DeployOptions) -> Result<Attached, DeployError> {
        let started = Instant::now();
        let agents = self.running()?;
        let mut timings = Timings {
            listing: started.elapsed(),
            ..Timings::default()
        };
        let serial = self.device.serial();
        let mut replaced = None;
        let mut recorded_pid = None;
        if let Some(record) = self.records.get(serial) {
            let running = agents
                .iter()
                .any(|a| a.pid == record.pid && a.socket == record.socket);
            if running {
                recorded_pid = Some(record.pid);
                let checking = Instant::now();
                match self.check(&record, options) {
                    Ok((connection, negotiated)) => {
                        timings.handshake = checking.elapsed();
                        timings.total = started.elapsed();
                        return Ok(Attached {
                            agent: AgentInfo {
                                serial: serial.to_owned(),
                                socket: record.socket.clone(),
                                pid: record.pid,
                                agent: negotiated.agent,
                                protocol: negotiated.version,
                                capabilities: negotiated.capabilities,
                                token: record.token.clone(),
                            },
                            connection,
                            outcome: Outcome::Reused,
                            timings,
                            deployer: self.clone(),
                        });
                    }
                    Err((reason, connection)) => {
                        let stopped = self.replace(&record, &reason, connection, options)?;
                        timings.replace = Some(checking.elapsed());
                        replaced = Some(Replaced {
                            pid: record.pid,
                            socket: record.socket.clone(),
                            reason,
                            stopped,
                        });
                    }
                }
            }
            self.records.remove(serial).map_err(DeployError::Records)?;
        }
        let foreign = agents.iter().find(|a| Some(a.pid) != recorded_pid);
        if let Some(other) = foreign
            && !options.force
        {
            return Err(DeployError::Conflict(Conflict::foreign_agent(other)));
        }
        self.launch(options, replaced, timings, started)
    }

    /// Connects to the recorded agent with its token and judges it.
    fn check(
        &self,
        record: &Record,
        options: &DeployOptions,
    ) -> Result<(Connection, Negotiated), (Incompatibility, Option<Connection>)> {
        let unreachable = |why: String| (Incompatibility::Unreachable(why), None);
        let stream = self
            .device
            .open_abstract(&record.socket)
            .map_err(|e| unreachable(e.to_string()))?;
        let connection = Connection::open(stream, self.host_config(&record.token))
            .map_err(|e| unreachable(e.to_string()))?;
        let handshake = connection.handshake(options.handshake_timeout);
        match (compatibility(&handshake, &self.jar.agent_name()), handshake) {
            (Compatibility::Compatible, Ok(negotiated)) => Ok((connection, negotiated)),
            (Compatibility::Incompatible(reason), _) => Err((reason, Some(connection))),
            (Compatibility::Compatible, Err(reason)) => Err(unreachable(reason.to_string())),
        }
    }

    /// Stops the agent `pid` on `socket` that `connection` reaches:
    /// `shutdown`, answered before it exits, then a kill past `timeout`; and
    /// forgets its record.
    pub(crate) fn stop_agent(
        &self,
        connection: &Connection,
        pid: u32,
        socket: &str,
        timeout: Duration,
    ) -> Result<Stopped, DeployError> {
        let _ = connection.call(Call::Shutdown, SHUTDOWN_DEADLINE);
        connection.close();
        let stopped = self.finish(pid, socket, timeout)?;
        self.forget(pid)?;
        Ok(stopped)
    }

    /// Stops an agent that cannot be reused: one whose protocol was refused
    /// exits by itself, another build is asked to `shutdown`, and anything
    /// still there after the exit timeout is killed.
    fn replace(
        &self,
        record: &Record,
        reason: &Incompatibility,
        connection: Option<Connection>,
        options: &DeployOptions,
    ) -> Result<Stopped, DeployError> {
        if let (Incompatibility::Version { .. }, Some(connection)) = (reason, &connection) {
            let _ = connection.call(Call::Shutdown, SHUTDOWN_DEADLINE);
        }
        drop(connection);
        self.finish(record.pid, &record.socket, options.exit_timeout)
    }

    /// Waits up to `timeout` for the agent `pid` on `socket` to exit, then
    /// kills it.
    fn finish(&self, pid: u32, socket: &str, timeout: Duration) -> Result<Stopped, DeployError> {
        if self.wait_gone(pid, socket, timeout)? {
            return Ok(Stopped::Exited);
        }
        self.kill(pid, socket)?;
        if self.wait_gone(pid, socket, Duration::from_secs(2))? {
            Ok(Stopped::Killed)
        } else {
            Err(DeployError::Exited {
                output: format!("agent {pid} on {socket} survived SIGKILL"),
                exit: None,
            })
        }
    }

    /// Whether the agent `pid` on `socket` is gone within `timeout`, polled
    /// on the device every 20 ms.
    fn wait_gone(&self, pid: u32, socket: &str, timeout: Duration) -> Result<bool, adb::Error> {
        let polls = (timeout.as_millis() / 20).max(1);
        let script = format!(
            "i=0; while grep -qF {socket} /proc/{pid}/cmdline 2>/dev/null; do \
             i=$((i+1)); [ $i -gt {polls} ] && exit 1; sleep 0.02; done; exit 0"
        );
        Ok(self.device.shell(&script)?.success())
    }

    /// Kills `pid` if its command line still names `socket`.
    fn kill(&self, pid: u32, socket: &str) -> Result<(), adb::Error> {
        self.device.shell(&format!(
            "grep -qF {socket} /proc/{pid}/cmdline 2>/dev/null && kill -9 {pid}"
        ))?;
        Ok(())
    }

    /// Kills every agent process listening on `socket`.
    fn kill_socket(&self, socket: &str) -> Result<(), adb::Error> {
        for agent in self.running()? {
            if agent.socket == socket {
                self.kill(agent.pid, socket)?;
            }
        }
        Ok(())
    }

    /// Forgets the record of the agent `pid`, and only that one.
    fn forget(&self, pid: u32) -> Result<(), DeployError> {
        let serial = self.device.serial();
        if self.records.get(serial).is_some_and(|r| r.pid == pid) {
            self.records.remove(serial).map_err(DeployError::Records)?;
        }
        Ok(())
    }

    fn host_config(&self, token: &str) -> HostConfig {
        HostConfig {
            token: Some(token.to_owned()),
            ..HostConfig::default()
        }
    }

    /// Pushes the jar, starts the agent detached, waits for its ready line,
    /// deletes the jar, connects and checks the build its hello names.
    fn launch(
        &self,
        options: &DeployOptions,
        replaced: Option<Replaced>,
        mut timings: Timings,
        started: Instant,
    ) -> Result<Attached, DeployError> {
        let socket = SocketName::generate(self.jar.version(), PROTOCOL_VERSION, &self.jar.build())
            .to_string();
        let token = random_hex(16);
        let remote = remote_jar(&socket);
        let pushing = Instant::now();
        push_jar(&self.device, &self.jar, &remote)?;
        timings.push = Some(pushing.elapsed());
        let command = launch_command(
            &remote,
            &socket,
            options.idle_exit,
            options.force,
            &options.agent_args,
        );
        let spawning = Instant::now();
        let spawned = self.spawn(&command, &token, options.ready_timeout);
        timings.spawn_to_ready = Some(spawning.elapsed());
        // The running agent keeps the dex it loaded; nothing needs the file.
        let _ = self.device.shell(&format!("rm -f {remote}"));
        let ready = match spawned? {
            Spawned::Ready(ready) => ready,
            Spawned::Conflict(line) => {
                return Err(DeployError::Conflict(Conflict::from_line(line)));
            }
            Spawned::Exited { output, exit } => {
                return Err(DeployError::Exited { output, exit });
            }
            Spawned::Timeout { output } => {
                let _ = self.kill_socket(&socket);
                return Err(DeployError::NoReadyLine {
                    waited: options.ready_timeout,
                    output,
                });
            }
        };
        let connecting = Instant::now();
        let connection = self.device.open_abstract(&socket).and_then(|stream| {
            Connection::open(stream, self.host_config(&token)).map_err(|source| adb::Error::Io {
                request: format!("connect to the agent on {socket}"),
                source,
            })
        });
        let connection = match connection {
            Ok(connection) => connection,
            Err(error) => {
                let _ = self.kill(ready.pid, &socket);
                return Err(error.into());
            }
        };
        let negotiated = match connection.handshake(options.handshake_timeout) {
            Ok(negotiated) => negotiated,
            Err(reason) => {
                drop(connection);
                let _ = self.kill(ready.pid, &socket);
                return Err(DeployError::Handshake { socket, reason });
            }
        };
        timings.handshake = connecting.elapsed();
        let expected = self.jar.agent_name();
        if negotiated.agent != expected {
            let _ = connection.call(Call::Shutdown, SHUTDOWN_DEADLINE);
            drop(connection);
            let _ = self.finish(ready.pid, &socket, options.exit_timeout);
            return Err(DeployError::WrongVersion {
                reported: negotiated.agent,
                expected,
                jar: self.jar.path().map(PathBuf::from),
            });
        }
        let agent = AgentInfo {
            serial: self.device.serial().to_owned(),
            socket: socket.clone(),
            pid: ready.pid,
            agent: negotiated.agent,
            protocol: negotiated.version,
            capabilities: negotiated.capabilities,
            token,
        };
        self.records
            .put(Record {
                serial: agent.serial.clone(),
                socket,
                pid: agent.pid,
                token: agent.token.clone(),
                agent: agent.agent.clone(),
                protocol: agent.protocol,
                build: self.jar.build(),
            })
            .map_err(DeployError::Records)?;
        timings.total = started.elapsed();
        let took_over = ready.forced.clone();
        Ok(Attached {
            connection,
            agent,
            outcome: Outcome::Launched(Box::new(Launched {
                ready,
                replaced,
                took_over,
            })),
            timings,
            deployer: self.clone(),
        })
    }

    /// Runs the launch command, hands it the token, and reads the stream
    /// until the ready or conflict line. The stream is dropped on return,
    /// which leaves a ready agent running (see [`launch_command`]).
    fn spawn(&self, command: &str, token: &str, timeout: Duration) -> Result<Spawned, DeployError> {
        let mut stream = self.device.shell_stream(command)?;
        stream.write_stdin(format!("{token}\n").as_bytes())?;
        stream.close_stdin()?;
        let deadline = Instant::now() + timeout;
        let mut transcript = Transcript::default();
        let mut exit = None;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(Spawned::Timeout {
                    output: transcript.text(),
                });
            }
            stream.set_read_timeout(Some(left))?;
            let packet = match stream.read_packet() {
                Ok(packet) => packet,
                Err(error) if error.is_timeout() => continue,
                Err(error) => return Err(error.into()),
            };
            match packet {
                Some(ShellPacket::Stdout(data)) => {
                    for line in transcript.stdout(&data) {
                        if let Some(ready) = parse_ready(&line) {
                            return Ok(Spawned::Ready(ready));
                        }
                        if let Some(conflict) = parse_conflict(&line) {
                            // The agent exits after its report; its end
                            // closes the stream.
                            let _ = stream.set_read_timeout(Some(CONFLICT_EXIT_WAIT));
                            while let Ok(Some(_)) = stream.read_packet() {}
                            return Ok(Spawned::Conflict(conflict));
                        }
                    }
                }
                Some(ShellPacket::Stderr(data)) => {
                    transcript.stderr.push_str(&String::from_utf8_lossy(&data));
                }
                Some(ShellPacket::Exit(code)) => exit = Some(code),
                None => {
                    return Ok(Spawned::Exited {
                        output: transcript.text(),
                        exit,
                    });
                }
            }
        }
    }

    /// Stops the agent this host recorded for the device, when it still
    /// runs, without a connection to it: `shutdown` over a new connection,
    /// else a kill by pid. `None` when there was nothing to stop.
    pub fn stop_recorded(&self, timeout: Duration) -> Result<Option<Stopped>, DeployError> {
        let serial = self.device.serial();
        let Some(record) = self.records.get(serial) else {
            return Ok(None);
        };
        let running = self
            .running()?
            .into_iter()
            .any(|a| a.pid == record.pid && a.socket == record.socket);
        let stopped = if running {
            if let Ok(stream) = self.device.open_abstract(&record.socket)
                && let Ok(connection) = Connection::open(stream, self.host_config(&record.token))
                && connection.handshake(timeout).is_ok()
            {
                let _ = connection.call(Call::Shutdown, SHUTDOWN_DEADLINE);
            }
            self.finish(record.pid, &record.socket, timeout)?
        } else {
            Stopped::Gone
        };
        self.records.remove(serial).map_err(DeployError::Records)?;
        Ok(Some(stopped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::launch::SocketName;
    use protocol::session::HandshakeError;

    const EXPECTED: &str = "mobile-agent-android 0.1.0";

    fn negotiated(agent: &str) -> Negotiated {
        Negotiated {
            version: 1,
            max_frame: 1 << 24,
            capabilities: Capabilities::TREE,
            agent: agent.into(),
            host_versions: vec![1],
            agent_versions: vec![1],
        }
    }

    fn running(pid: u32, socket: &str) -> RunningAgent {
        RunningAgent {
            pid,
            socket: socket.into(),
            name: SocketName::parse(socket),
        }
    }

    #[test]
    fn the_same_build_on_the_same_protocol_is_compatible() {
        assert_eq!(
            compatibility(&Ok(negotiated(EXPECTED)), EXPECTED),
            Compatibility::Compatible
        );
    }

    /// Spec scenario "Incompatible agent replaced": the running agent reports
    /// an older protocol version than the daemon requires.
    #[test]
    fn an_agent_on_an_older_protocol_is_incompatible() {
        let refused = Err(DisconnectReason::Handshake(
            HandshakeError::NoCommonVersion {
                host: vec![1],
                agent: vec![0],
            },
        ));
        assert_eq!(
            compatibility(&refused, EXPECTED),
            Compatibility::Incompatible(Incompatibility::Protocol {
                agent: vec![0],
                host: vec![1],
            })
        );
    }

    #[test]
    fn another_build_is_incompatible() {
        assert_eq!(
            compatibility(&Ok(negotiated("mobile-agent-android 0.0.9")), EXPECTED),
            Compatibility::Incompatible(Incompatibility::Version {
                reported: "mobile-agent-android 0.0.9".into(),
                expected: EXPECTED.into(),
            })
        );
    }

    #[test]
    fn an_agent_that_does_not_answer_as_recorded_is_unreachable() {
        for reason in [
            DisconnectReason::Handshake(HandshakeError::TokenMismatch),
            DisconnectReason::Eof,
            DisconnectReason::Silent(Duration::from_secs(3)),
        ] {
            let text = reason.to_string();
            assert_eq!(
                compatibility(&Err(reason), EXPECTED),
                Compatibility::Incompatible(Incompatibility::Unreachable(text))
            );
        }
    }

    #[test]
    fn incompatibilities_say_what_differs() {
        let protocol = Incompatibility::Protocol {
            agent: vec![0],
            host: vec![1],
        }
        .to_string();
        assert!(
            protocol.contains("[0]") && protocol.contains("[1]"),
            "{protocol}"
        );
        let version = Incompatibility::Version {
            reported: "mobile-agent-android 0.0.9".into(),
            expected: EXPECTED.into(),
        }
        .to_string();
        assert!(
            version.contains("0.0.9") && version.contains("0.1.0"),
            "{version}"
        );
    }

    const BUILD: &str = "0123456789ab";

    #[test]
    fn no_agent_process_is_absent() {
        assert_eq!(agent_state(&[], "0.1.0", 1, BUILD), AgentState::Absent);
    }

    #[test]
    fn an_agent_named_for_this_build_is_running() {
        let agent = running(812, "mobile-agent-0.1.0-p1-0123456789ab-00ff");
        assert_eq!(
            agent_state(std::slice::from_ref(&agent), "0.1.0", 1, BUILD),
            AgentState::Running(agent)
        );
    }

    #[test]
    fn another_version_protocol_build_or_an_unknown_name_is_incompatible() {
        for socket in [
            "mobile-agent-0.0.9-p1-0123456789ab-00ff",
            "mobile-agent-0.1.0-p2-0123456789ab-00ff",
            "mobile-agent-0.1.0-p1-ba9876543210-00ff",
            "mobile-agent-0.1.0-p1-00ff",
            "mobile-agent-3f9a1c2b7d4e",
        ] {
            let agent = running(812, socket);
            assert_eq!(
                agent_state(std::slice::from_ref(&agent), "0.1.0", 1, BUILD),
                AgentState::Incompatible(agent),
                "{socket}"
            );
        }
    }

    #[test]
    fn a_compatible_agent_wins_over_an_incompatible_one() {
        let old = running(700, "mobile-agent-0.1.0-p1-ba9876543210-aa");
        let current = running(812, "mobile-agent-0.1.0-p1-0123456789ab-bb");
        assert_eq!(
            agent_state(&[old, current.clone()], "0.1.0", 1, BUILD),
            AgentState::Running(current)
        );
    }

    /// The agent's payload gained window metadata without a version bump:
    /// a recorded agent of another build reporting the same version is
    /// replaced, as is one recorded before builds were.
    #[test]
    fn a_recorded_agent_of_another_build_is_incompatible() {
        assert_eq!(build_compatibility(BUILD, BUILD), Compatibility::Compatible);
        assert_eq!(
            build_compatibility("ba9876543210", BUILD),
            Compatibility::Incompatible(Incompatibility::Build {
                running: "ba9876543210".into(),
                deploying: BUILD.into(),
            })
        );
        assert_eq!(
            build_compatibility("", BUILD),
            Compatibility::Incompatible(Incompatibility::Build {
                running: String::new(),
                deploying: BUILD.into(),
            })
        );
        let text = Incompatibility::Build {
            running: "ba9876543210".into(),
            deploying: BUILD.into(),
        }
        .to_string();
        assert!(
            text.contains("ba9876543210") && text.contains(BUILD),
            "{text}"
        );
    }

    /// Spec scenario "Another tool holds automation": the error names the
    /// likely holder and how to take over.
    #[test]
    fn a_conflict_names_the_holder_and_the_force_option() {
        let conflict = Conflict {
            holder: Some(Holder {
                name: "uiautomator".into(),
                pid: Some(4702),
            }),
            detail: "UI automation is held by another process, likely uiautomator (pid 4702: \
                     uiautomator events); stop it, or run the agent with --force to take the \
                     role over"
                .into(),
        };
        let message = conflict.message();
        assert!(message.contains("uiautomator (pid 4702)"), "{message}");
        assert!(message.contains("force"), "{message}");
        assert!(
            DeployError::Conflict(conflict)
                .to_string()
                .contains("uiautomator")
        );
    }

    #[test]
    fn an_agent_this_host_did_not_start_is_a_conflict_naming_it() {
        let conflict = Conflict::foreign_agent(&running(900, "mobile-agent-0.1.0-p1-cc"));
        assert_eq!(
            conflict.holder,
            Some(Holder {
                name: "mobile-agent".into(),
                pid: Some(900),
            })
        );
        let message = conflict.message();
        assert!(message.contains("mobile-agent-0.1.0-p1-cc"), "{message}");
        assert!(message.contains("force"), "{message}");
    }

    #[test]
    fn deploy_errors_explain_themselves() {
        let exited = DeployError::Exited {
            output: "err: cannot listen on @s: Address already in use".into(),
            exit: Some(1),
        }
        .to_string();
        assert!(exited.contains("cannot listen"), "{exited}");
        let wrong = DeployError::WrongVersion {
            reported: "mobile-agent-android 0.0.9".into(),
            expected: EXPECTED.into(),
            jar: Some(PathBuf::from("/tmp/a.jar")),
        }
        .to_string();
        assert!(wrong.contains("agent-android/build.sh"), "{wrong}");
        assert!(wrong.contains("/tmp/a.jar"), "{wrong}");
    }

    #[test]
    fn the_defaults_match_the_agents_own() {
        let options = DeployOptions::default();
        assert!(!options.force);
        assert_eq!(options.idle_exit, Duration::from_secs(30));
        assert!(options.ready_timeout >= Duration::from_secs(5));
    }
}
