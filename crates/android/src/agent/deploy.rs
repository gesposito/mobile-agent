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
use std::time::Duration;

use protocol::Capabilities;
use protocol::session::{DisconnectReason, Negotiated};

use super::connection::Connection;
use super::jar::AgentJar;
use super::launch::{Holder, ReadyLine, RunningAgent};
use super::records::Records;
use crate::adb::{self, Device};

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
    /// The per-launch token its hello carries.
    pub fn token(&self) -> &str {
        &self.token
    }
}

/// How an attach went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A new agent was started.
    Launched(Launched),
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
}

impl fmt::Display for Incompatibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!("task 5.2")
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
    let _ = (handshake, expected_agent);
    todo!("task 5.2")
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

/// The state `agents` add up to for a driver deploying `version` over
/// `protocol`: running when one of them is that build, else incompatible
/// when any runs.
pub fn agent_state(agents: &[RunningAgent], version: &str, protocol: u16) -> AgentState {
    let _ = (agents, version, protocol);
    todo!("task 5.2")
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
        let _ = agent;
        todo!("task 5.2")
    }

    /// What the user reads: the holder, the detail and how to take over.
    pub fn message(&self) -> String {
        todo!("task 5.2")
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
        let _ = f;
        todo!("task 5.2")
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
    let _ = (device, jar, remote);
    todo!("task 5.2")
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
        let _ = timeout;
        todo!("task 5.2")
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
        todo!("task 5.2")
    }

    /// The target's agent state, from the process list.
    pub fn state(&self) -> Result<AgentState, adb::Error> {
        todo!("task 5.2")
    }

    /// Connects to an agent: the running one this host started when it is
    /// compatible, else a new one. Any connection the reused agent had is
    /// replaced by this one.
    pub fn attach(&self, options: &DeployOptions) -> Result<Attached, DeployError> {
        let _ = options;
        todo!("task 5.2")
    }

    /// Stops the agent this host recorded for the device, when it still
    /// runs, without a connection to it: `shutdown` over a new connection,
    /// else a kill by pid. `None` when there was nothing to stop.
    pub fn stop_recorded(&self, timeout: Duration) -> Result<Option<Stopped>, DeployError> {
        let _ = timeout;
        todo!("task 5.2")
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

    #[test]
    fn no_agent_process_is_absent() {
        assert_eq!(agent_state(&[], "0.1.0", 1), AgentState::Absent);
    }

    #[test]
    fn an_agent_named_for_this_build_is_running() {
        let agent = running(812, "mobile-agent-0.1.0-p1-00ff");
        assert_eq!(
            agent_state(&[agent.clone()], "0.1.0", 1),
            AgentState::Running(agent)
        );
    }

    #[test]
    fn another_version_protocol_or_an_unknown_name_is_incompatible() {
        for socket in [
            "mobile-agent-0.0.9-p1-00ff",
            "mobile-agent-0.1.0-p2-00ff",
            "mobile-agent-3f9a1c2b7d4e",
        ] {
            let agent = running(812, socket);
            assert_eq!(
                agent_state(&[agent.clone()], "0.1.0", 1),
                AgentState::Incompatible(agent),
                "{socket}"
            );
        }
    }

    #[test]
    fn a_compatible_agent_wins_over_an_incompatible_one() {
        let old = running(700, "mobile-agent-0.0.9-p1-aa");
        let current = running(812, "mobile-agent-0.1.0-p1-bb");
        assert_eq!(
            agent_state(&[old, current.clone()], "0.1.0", 1),
            AgentState::Running(current)
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
