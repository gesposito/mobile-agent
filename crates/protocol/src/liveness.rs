//! Application-level liveness (spec: Liveness detection).
//!
//! Over `adb forward` the TCP hop ends at the local adb server, so TCP
//! keepalive proves nothing about the agent. Instead each side pings after
//! [`crate::session::PING_INTERVAL`] without hearing from the other and
//! declares it dead after [`crate::session::PEER_TIMEOUT`].

use std::time::{Duration, Instant};

/// What a [`Liveness`] check asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alive {
    /// Nothing to do.
    Yes,
    /// Send a ping now.
    Ping,
    /// Nothing arrived for this long: the other side is dead.
    Dead(Duration),
}

/// Silence tracking for one side of a connection.
#[derive(Clone, Debug)]
pub struct Liveness {
    interval: Duration,
    timeout: Duration,
    last_heard: Instant,
    last_ping: Option<Instant>,
}

impl Liveness {
    /// Starts counting silence at `now`.
    pub fn new(interval: Duration, timeout: Duration, now: Instant) -> Liveness {
        Liveness {
            interval,
            timeout,
            last_heard: now,
            last_ping: None,
        }
    }

    /// Something arrived from the other side.
    pub fn heard(&mut self, now: Instant) {
        self.last_heard = self.last_heard.max(now);
    }

    /// Checks the silence at `now`; pings only when `may_ping`.
    pub fn check(&mut self, now: Instant, may_ping: bool) -> Alive {
        let silence = now.saturating_duration_since(self.last_heard);
        if silence >= self.timeout {
            return Alive::Dead(silence);
        }
        if may_ping && silence >= self.interval && now >= self.next_ping() {
            self.last_ping = Some(now);
            return Alive::Ping;
        }
        Alive::Yes
    }

    /// When [`Liveness::check`] next has something to do.
    pub fn next_check(&self, may_ping: bool) -> Instant {
        let dead_at = self.last_heard + self.timeout;
        if may_ping {
            dead_at.min(self.next_ping())
        } else {
            dead_at
        }
    }

    /// One interval after the later of the last ping and the last sign of life.
    fn next_ping(&self) -> Instant {
        let since = match self.last_ping {
            Some(ping) if ping > self.last_heard => ping,
            _ => self.last_heard,
        };
        since + self.interval
    }
}
