//! The device agent's end of the connection.

use std::time::{Duration, Instant};

use super::{DisconnectReason, Negotiated, PEER_TIMEOUT, PING_INTERVAL};
use crate::capability::Capabilities;
use crate::error::Error;
use crate::frame::DEFAULT_MAX_FRAME;
use crate::message::{Reply, Request};

/// Settings for an [`AgentSession`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConfig {
    /// Protocol versions the agent speaks.
    pub versions: Vec<u16>,
    pub capabilities: Capabilities,
    /// The largest frame payload the agent accepts.
    pub max_frame: u32,
    /// Agent name and version for the hello.
    pub agent: String,
    /// The per-launch token the host started the agent with.
    pub token: Option<String>,
    pub ping_interval: Duration,
    pub peer_timeout: Duration,
}

impl Default for AgentConfig {
    fn default() -> AgentConfig {
        AgentConfig {
            versions: crate::SUPPORTED_VERSIONS.to_vec(),
            capabilities: Capabilities::NONE,
            max_frame: DEFAULT_MAX_FRAME,
            agent: String::new(),
            token: None,
            ping_interval: PING_INTERVAL,
            peer_timeout: PEER_TIMEOUT,
        }
    }
}

/// What happened on an [`AgentSession`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// The host accepted the hello.
    Ready(Negotiated),
    /// A request to work on; answer it with [`AgentSession::respond`].
    Request(Request),
    /// The session closed.
    Closed(DisconnectReason),
}

/// What became of an answer passed to [`AgentSession::respond`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Responded {
    /// Sent as given.
    Sent,
    /// Over the negotiated frame size: a `frame_too_large` error went instead.
    TooLarge,
    /// Not sent: the request was already answered, by its deadline for
    /// instance, or never arrived.
    NotInFlight,
}

/// The device agent's end of the connection.
#[derive(Debug)]
pub struct AgentSession {
    config: AgentConfig,
}

impl AgentSession {
    /// Starts the session; the hello is ready to send at once.
    pub fn new(config: AgentConfig, now: Instant) -> AgentSession {
        let _ = (config, now);
        todo!("AgentSession::new")
    }

    /// Feeds bytes read from the connection.
    pub fn receive(&mut self, bytes: &[u8], now: Instant) {
        let _ = (bytes, now, &self.config);
        todo!("receive")
    }

    /// The connection reached its end.
    pub fn receive_eof(&mut self, now: Instant) {
        let _ = now;
        todo!("receive_eof")
    }

    /// Answers requests whose deadline passed and keeps the connection alive.
    pub fn handle_timeout(&mut self, now: Instant) {
        let _ = now;
        todo!("handle_timeout")
    }

    /// When `handle_timeout` must next run; `None` once closed.
    pub fn poll_timeout(&self) -> Option<Instant> {
        todo!("poll_timeout")
    }

    /// The instant request `id` must be answered by, if it has a deadline.
    pub fn deadline(&self, id: u32) -> Option<Instant> {
        let _ = id;
        todo!("deadline")
    }

    /// Answers request `id`.
    pub fn respond(&mut self, id: u32, result: Result<Reply, Error>) -> Responded {
        let _ = (id, result);
        todo!("respond")
    }

    /// Bytes to write to the connection.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        todo!("poll_transmit")
    }

    pub fn poll_event(&mut self) -> Option<AgentEvent> {
        todo!("poll_event")
    }

    pub fn is_closed(&self) -> bool {
        todo!("is_closed")
    }
}
