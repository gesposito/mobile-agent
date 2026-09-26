//! Connection state machines for both ends of the device link, without I/O.
//!
//! A [`HostSession`] runs in the daemon and an [`AgentSession`] in a device
//! agent. The caller feeds bytes read from the socket into `receive`, writes
//! whatever `poll_transmit` returns, calls `handle_timeout` at `poll_timeout`,
//! and reads what happened from `poll_event`. Every method that depends on
//! time takes `now`; the sessions never read the clock themselves.
//!
//! The agent speaks first: its hello carries the versions it speaks, its
//! capabilities and its maximum frame size. The host answers with a welcome
//! naming the highest version both speak and the smaller maximum frame size,
//! or with the reason it refuses the connection before closing it.

mod agent;
mod host;

use core::fmt;
use std::time::Duration;

pub use agent::{AgentConfig, AgentEvent, AgentSession, Responded};
pub use host::{HostConfig, HostEvent, HostSession};

use crate::capability::Capabilities;
use crate::error::{Error, code};
use crate::frame::MIN_MAX_FRAME;

/// How long a peer may stay silent before a ping goes out.
pub const PING_INTERVAL: Duration = Duration::from_secs(1);

/// How long a peer may stay silent before it is declared dead. With pings
/// every second, three unanswered ones; the spec's bound for reporting a
/// killed agent is five seconds.
pub const PEER_TIMEOUT: Duration = Duration::from_secs(3);

/// How long past a request's deadline the host waits for the agent's own
/// deadline error before reporting one itself.
pub const DEADLINE_GRACE: Duration = Duration::from_millis(500);

/// The highest version in both lists, whatever their order.
pub fn select_version(ours: &[u16], theirs: &[u16]) -> Option<u16> {
    ours.iter().filter(|v| theirs.contains(v)).max().copied()
}

/// Where a session is, without the data each state holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Handshake,
    Ready,
    Closed,
}

/// What the handshake settled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Negotiated {
    /// The protocol version both sides use.
    pub version: u16,
    /// The maximum frame payload both sides accept.
    pub max_frame: u32,
    /// What the agent advertised.
    pub capabilities: Capabilities,
    /// The agent's name and version from its hello.
    pub agent: String,
    pub host_versions: Vec<u16>,
    pub agent_versions: Vec<u16>,
}

/// Why a handshake failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandshakeError {
    /// The two sides share no protocol version.
    NoCommonVersion { host: Vec<u16>, agent: Vec<u16> },
    /// The hello's token is not the one the host started the agent with.
    TokenMismatch,
    /// The negotiated maximum frame size is below [`MIN_MAX_FRAME`].
    FrameLimitTooSmall { limit: u32 },
    /// The host selected a version the agent does not speak.
    VersionNotOffered { version: u16, offered: Vec<u16> },
    /// The host refused the connection (seen by the agent).
    Refused(Error),
}

impl HandshakeError {
    /// The error the host sends in its welcome.
    pub fn to_error(&self) -> Error {
        match self {
            HandshakeError::NoCommonVersion { .. } | HandshakeError::VersionNotOffered { .. } => {
                Error::new(code::VERSION_MISMATCH, self.to_string())
            }
            HandshakeError::TokenMismatch => Error::new(code::TOKEN_MISMATCH, self.to_string()),
            HandshakeError::FrameLimitTooSmall { .. } => {
                Error::new(code::FRAME_LIMIT_TOO_SMALL, self.to_string())
            }
            HandshakeError::Refused(error) => error.clone(),
        }
    }
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandshakeError::NoCommonVersion { host, agent } => write!(
                f,
                "no common protocol version: the daemon speaks {host:?} and the agent speaks {agent:?}; redeploy the agent that matches this daemon"
            ),
            HandshakeError::TokenMismatch => f.write_str(
                "the agent's hello carries another per-launch token, so this daemon did not start it",
            ),
            HandshakeError::FrameLimitTooSmall { limit } => write!(
                f,
                "the negotiated maximum frame size of {limit} bytes is below the protocol minimum of {MIN_MAX_FRAME} bytes"
            ),
            HandshakeError::VersionNotOffered { version, offered } => write!(
                f,
                "the daemon selected protocol version {version}, which the agent does not speak (it speaks {offered:?})"
            ),
            HandshakeError::Refused(error) => write!(f, "the daemon refused the connection: {error}"),
        }
    }
}

impl std::error::Error for HandshakeError {}

/// Why a session closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisconnectReason {
    /// The other side closed the connection.
    Eof,
    /// Nothing arrived from the other side for this long.
    Silent(Duration),
    /// The handshake failed.
    Handshake(HandshakeError),
    /// A message could not be decoded or was not expected.
    Protocol(String),
    /// This side closed the session.
    Closed,
}

impl fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DisconnectReason::Eof => f.write_str("the other side closed the connection"),
            DisconnectReason::Silent(silence) => write!(
                f,
                "nothing arrived from the other side for {:.1} s",
                silence.as_secs_f64()
            ),
            DisconnectReason::Handshake(e) => write!(f, "handshake failed: {e}"),
            DisconnectReason::Protocol(detail) => write!(f, "protocol error: {detail}"),
            DisconnectReason::Closed => f.write_str("the session was closed"),
        }
    }
}
