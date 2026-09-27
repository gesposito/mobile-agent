//! The resident agent from the host side (design D2 and D3): deploying it,
//! reusing or replacing a running one, and the connection a session drives.
//!
//! - [`AgentJar`] is the single-dex jar `agent-android/build.sh` writes and
//!   the version its hello must name.
//! - [`Deployer::attach`] reuses a running agent this host started when its
//!   protocol and agent versions match, replaces one that does not, and
//!   otherwise pushes the jar read-only, starts it detached as the shell user
//!   with a per-launch socket name and token, and waits for its ready line.
//!   Another holder of UI automation is reported as a [`Conflict`] naming the
//!   likely holder, and taken over only with [`DeployOptions::force`].
//! - [`Connection`] drives the protocol crate's `HostSession` over the
//!   agent's abstract socket, opened as an adb service stream, on a reader
//!   thread that answers pings and delivers responses and events.
//!
//! Nothing is installed on the device: no package, no instrumentation, no
//! forward rule. The pushed jar is deleted once the agent has started, and
//! the agent exits on `shutdown`, on its idle bound without a host, or when a
//! host refuses its protocol version.

mod connection;
mod deploy;
mod jar;
mod launch;
mod records;

pub use connection::{Connection, EventSink};
pub use deploy::{
    AgentInfo, AgentState, Attached, Compatibility, Conflict, DeployError, DeployOptions, Deployer,
    Incompatibility, Launched, Outcome, Replaced, Stopped, Timings, agent_state, compatibility,
    push_jar,
};
pub use jar::{AGENT_NAME, AGENT_VERSION, AgentJar, JAR_VAR, JarError, default_jar_path};
pub use launch::{
    ConflictLine, Holder, JAR_MODE, MAIN_CLASS, PROCESS_NAME, PS_COMMAND, ReadyLine, RunningAgent,
    SocketName, TOKEN_VARIABLE, launch_command, parse_agents, parse_conflict, parse_ready,
    random_hex, remote_jar,
};
pub use records::{Record, Records};
