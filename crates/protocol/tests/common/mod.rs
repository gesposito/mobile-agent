//! A host and an agent session joined in memory, with time passed in
//! explicitly: the sessions never read the clock, so tests control it.

#![allow(dead_code)]

use std::time::{Duration, Instant};

use protocol::Capabilities;
use protocol::frame::DEFAULT_MAX_FRAME;
use protocol::session::{
    AgentConfig, AgentEvent, AgentSession, HostConfig, HostEvent, HostSession,
};

pub fn host_config(versions: &[u16]) -> HostConfig {
    HostConfig {
        versions: versions.to_vec(),
        ..HostConfig::default()
    }
}

pub fn agent_config(versions: &[u16], capabilities: Capabilities) -> AgentConfig {
    AgentConfig {
        versions: versions.to_vec(),
        capabilities,
        max_frame: DEFAULT_MAX_FRAME,
        agent: "placeholder-agent 0.0.0".to_owned(),
        token: None,
        ..AgentConfig::default()
    }
}

pub fn every_capability() -> Capabilities {
    Capabilities::TREE
        | Capabilities::INPUT
        | Capabilities::SCREENSHOT
        | Capabilities::FRAMES
        | Capabilities::EVENTS
        | Capabilities::TEXT_SET
}

pub struct Link {
    pub host: HostSession,
    pub agent: AgentSession,
    pub t0: Instant,
}

impl Link {
    /// Starts both sessions at the same instant; nothing has moved yet.
    pub fn new(host: HostConfig, agent: AgentConfig) -> Link {
        let t0 = Instant::now();
        Link {
            host: HostSession::new(host, t0),
            agent: AgentSession::new(agent, t0),
            t0,
        }
    }

    /// Starts both sessions and completes the handshake, draining its events.
    pub fn ready(host: HostConfig, agent: AgentConfig) -> Link {
        let mut link = Link::new(host, agent);
        link.pump(link.t0);
        let host_events = link.host_events();
        assert!(
            matches!(host_events.as_slice(), [HostEvent::Ready(_)]),
            "host: {host_events:?}"
        );
        let agent_events = link.agent_events();
        assert!(
            matches!(agent_events.as_slice(), [AgentEvent::Ready(_)]),
            "agent: {agent_events:?}"
        );
        link
    }

    pub fn at(&self, ms: u64) -> Instant {
        self.t0 + Duration::from_millis(ms)
    }

    /// Moves bytes both ways until neither side has anything left to send.
    pub fn pump(&mut self, now: Instant) {
        loop {
            let mut moved = false;
            while let Some(bytes) = self.agent.poll_transmit() {
                self.host.receive(&bytes, now);
                moved = true;
            }
            while let Some(bytes) = self.host.poll_transmit() {
                self.agent.receive(&bytes, now);
                moved = true;
            }
            if !moved {
                break;
            }
        }
    }

    /// Delivers what the agent sent to the host, but nothing the other way.
    pub fn agent_to_host(&mut self, now: Instant) {
        while let Some(bytes) = self.agent.poll_transmit() {
            self.host.receive(&bytes, now);
        }
    }

    pub fn host_events(&mut self) -> Vec<HostEvent> {
        std::iter::from_fn(|| self.host.poll_event()).collect()
    }

    pub fn agent_events(&mut self) -> Vec<AgentEvent> {
        std::iter::from_fn(|| self.agent.poll_event()).collect()
    }
}
