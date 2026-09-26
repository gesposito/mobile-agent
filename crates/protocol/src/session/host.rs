//! The daemon's end of a device connection.

use std::time::{Duration, Instant};

use super::{DEADLINE_GRACE, DisconnectReason, Negotiated, PEER_TIMEOUT, PING_INTERVAL};
use crate::error::Error;
use crate::event::Event;
use crate::frame::{DEFAULT_MAX_FRAME, Oversized};
use crate::message::{Call, Method, Reply};

/// Settings for a [`HostSession`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostConfig {
    /// Protocol versions the daemon speaks.
    pub versions: Vec<u16>,
    /// The largest frame payload the daemon accepts.
    pub max_frame: u32,
    /// The per-launch token the agent's hello must carry, if any.
    pub token: Option<String>,
    pub ping_interval: Duration,
    pub peer_timeout: Duration,
    pub deadline_grace: Duration,
}

impl Default for HostConfig {
    fn default() -> HostConfig {
        HostConfig {
            versions: crate::SUPPORTED_VERSIONS.to_vec(),
            max_frame: DEFAULT_MAX_FRAME,
            token: None,
            ping_interval: PING_INTERVAL,
            peer_timeout: PEER_TIMEOUT,
            deadline_grace: DEADLINE_GRACE,
        }
    }
}

/// What happened on a [`HostSession`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    /// The handshake completed.
    Ready(Negotiated),
    /// The answer to a request: the agent's, or an error the session reports
    /// on its behalf (a passed deadline, an oversized frame, a lost connection).
    Response {
        id: u32,
        method: Method,
        result: Result<Reply, Error>,
    },
    /// An event, in order.
    Event(Event),
    /// A frame over the limit that answered no pending request.
    FrameTooLarge(Oversized),
    /// The session closed; later requests fail at once.
    Closed(DisconnectReason),
}

/// The daemon's end of a device connection.
#[derive(Debug)]
pub struct HostSession {
    config: HostConfig,
}

impl HostSession {
    pub fn new(config: HostConfig, now: Instant) -> HostSession {
        let _ = now;
        let _ = config;
        todo!("HostSession::new")
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

    /// Acts on deadlines and liveness that are due at `now`.
    pub fn handle_timeout(&mut self, now: Instant) {
        let _ = now;
        todo!("handle_timeout")
    }

    /// When `handle_timeout` must next run; `None` once closed.
    pub fn poll_timeout(&self) -> Option<Instant> {
        todo!("poll_timeout")
    }

    /// Sends a request, returning its id. Fails at once, sending nothing, when
    /// the session is not ready or closed, when the agent did not advertise
    /// the capability the method needs, or when the request exceeds the
    /// negotiated frame size.
    pub fn send_request(
        &mut self,
        call: Call,
        deadline: Option<Duration>,
        now: Instant,
    ) -> Result<u32, Error> {
        let _ = (call, deadline, now);
        todo!("send_request")
    }

    /// Bytes to write to the connection.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        todo!("poll_transmit")
    }

    pub fn poll_event(&mut self) -> Option<HostEvent> {
        todo!("poll_event")
    }

    /// What the handshake settled, once it has.
    pub fn negotiated(&self) -> Option<&Negotiated> {
        todo!("negotiated")
    }

    pub fn is_closed(&self) -> bool {
        todo!("is_closed")
    }
}
