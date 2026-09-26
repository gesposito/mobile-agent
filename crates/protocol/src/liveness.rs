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
}

impl Liveness {
    pub fn new(interval: Duration, timeout: Duration, now: Instant) -> Liveness {
        let _ = now;
        Liveness { interval, timeout }
    }

    /// Something arrived from the other side.
    pub fn heard(&mut self, now: Instant) {
        let _ = now;
        todo!("heard")
    }

    /// Checks the silence at `now`; pings only when `may_ping`.
    pub fn check(&mut self, now: Instant, may_ping: bool) -> Alive {
        let _ = (now, may_ping, self.interval, self.timeout);
        todo!("check")
    }

    /// When [`Liveness::check`] next has something to do.
    pub fn next_check(&self, may_ping: bool) -> Instant {
        let _ = may_ping;
        todo!("next_check")
    }
}
