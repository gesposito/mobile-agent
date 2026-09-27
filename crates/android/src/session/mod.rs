//! The agent session: an attached agent behind the mirror's backend traits
//! (task 5.3; design D5 and D7).
//!
//! - [`Session::backend`] hands a `mirror::Backend` over: a tree source over
//!   the agent's `snapshot` requests, an event stream carrying the agent's
//!   events and, on an emulator with its controller, the controller's frames
//!   with status-bar-free hashes, an input sink over the agent's input and
//!   text requests or the controller's touches, and the system clock.
//! - The agent's snapshot passes through as it came: the window metadata it
//!   carries (whether each window sends change events, its owner, whether it
//!   is modal) and the scroll semantics are what the mirror reads.
//! - Input the agent did not inject, the controller's touches, leaves the
//!   agent's cache of Compose windows stale, so the next read waits until
//!   the input is a quiet window old and empties that cache first.
//! - The end of the connection or three seconds of silence (the protocol
//!   crate's liveness) reach the mirror as `Disconnected`.

mod controller;
mod events;
mod input;
mod tree;

use std::sync::Arc;
use std::time::Duration;

use mirror::backend::{Backend, ReadReason};
use protocol::message::{Screenshot, ScreenshotParams};
use protocol::session::DisconnectReason;

use crate::adb;
use crate::agent::{AgentInfo, Attached, Connection, DeployError, Deployer, Outcome, Stopped};

pub use controller::{
    ControllerLink, INPUT_QUIET, encode_jpeg, frame_meta, resize_rgb, rotate_rgb,
};
pub use events::stream_item;
pub use input::{agent_call, input_ack, swipe_touches};
pub use tree::{BACKEND, has_event_less_window, scale_from_density, snapshot_params};

/// How the session reads the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadOptions {
    /// Node cap per read; `None` takes the agent's (10,000).
    pub max_nodes: Option<u32>,
    /// Depth cap below each window root; `None` takes the agent's (256).
    pub max_depth: Option<u32>,
    /// Idle bound of reads the target's own changes trigger (events, polls,
    /// frames), in milliseconds: the mirror's settle replaces the platform's
    /// idle wait (design D8), so 0.
    pub idle_ms: u32,
    /// Idle bound of reads after an attach, an action or a resync, which
    /// should see the screen settled; `None` takes the agent's default
    /// (500 ms, ending 200 ms after the last event or action).
    pub settle_idle_ms: Option<u32>,
    /// Deadline of each snapshot request.
    pub deadline: Duration,
}

impl Default for ReadOptions {
    fn default() -> ReadOptions {
        ReadOptions {
            max_nodes: None,
            max_depth: None,
            idle_ms: 0,
            settle_idle_ms: None,
            deadline: Duration::from_secs(10),
        }
    }
}

/// How the session works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionOptions {
    pub read: ReadOptions,
    /// Deadline of input requests, added to a gesture's own duration.
    pub input_deadline: Duration,
    /// Width of the controller's frames (spike S7: 540 px at about 45 fps).
    pub frame_width: u32,
}

impl Default for SessionOptions {
    fn default() -> SessionOptions {
        SessionOptions {
            read: ReadOptions::default(),
            input_deadline: Duration::from_secs(5),
            frame_width: 540,
        }
    }
}

/// What the last tree read returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadStats {
    /// The screen's node count as the agent reported it.
    pub node_count: u32,
    /// Nodes the read carried.
    pub serialized: u32,
    pub windows: u32,
    /// From the start of the read, any wait after foreign input included, to
    /// its answer.
    pub took: Duration,
    pub reason: ReadReason,
    /// Whether the agent's node cache was emptied first, after input it did
    /// not inject.
    pub cleared_cache: bool,
}

/// Why a session could not start.
#[derive(Debug)]
pub enum SessionError {
    Adb(adb::Error),
    /// The display density could not be read.
    Density(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Adb(error) => write!(f, "adb: {error}"),
            SessionError::Density(output) => {
                write!(
                    f,
                    "cannot read the display density from `wm density`: {output}"
                )
            }
        }
    }
}

impl std::error::Error for SessionError {}

impl From<adb::Error> for SessionError {
    fn from(error: adb::Error) -> SessionError {
        SessionError::Adb(error)
    }
}

/// An attached agent serving a mirror. Clones share it.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

struct Inner {
    agent: AgentInfo,
    outcome: Outcome,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("agent", &self.inner.agent)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// A session over `attached`: reads the display density for the scale,
    /// and uses `controller`, when given, for touches and frames.
    pub fn start(
        attached: Attached,
        options: SessionOptions,
        controller: Option<ControllerLink>,
    ) -> Result<Session, SessionError> {
        let _ = (attached, options, controller);
        todo!("task 5.3")
    }

    /// A session over `connection`, whose handshake completed, at `scale`.
    /// Without a deployer, `stop` only asks the agent to shut down.
    pub(crate) fn over(
        connection: Connection,
        agent: AgentInfo,
        outcome: Outcome,
        deployer: Option<Deployer>,
        scale: f64,
        options: SessionOptions,
        controller: Option<ControllerLink>,
    ) -> Session {
        let _ = (connection, deployer, scale, options, controller);
        Session {
            inner: Arc::new(Inner { agent, outcome }),
        }
    }

    /// Notes input the agent did not inject at `at`, such as a controller
    /// touch: the next read waits until it is [`INPUT_QUIET`] old and first
    /// empties the agent's node cache when a window hosts Compose.
    pub(crate) fn note_foreign_input(&self, at: std::time::Instant) {
        let _ = at;
        todo!("task 5.3")
    }

    /// The backend a `mirror::Mirror` runs on. Taken once: the event stream
    /// has one reader.
    ///
    /// # Panics
    ///
    /// When called a second time.
    pub fn backend(&self) -> Backend {
        todo!("task 5.3")
    }

    pub fn agent(&self) -> &AgentInfo {
        &self.inner.agent
    }

    /// Whether the agent was launched or reused.
    pub fn outcome(&self) -> &Outcome {
        &self.inner.outcome
    }

    /// Physical pixels per device point.
    pub fn scale(&self) -> f64 {
        todo!("task 5.3")
    }

    /// Whether touches and frames go through the emulator's controller.
    pub fn uses_controller(&self) -> bool {
        todo!("task 5.3")
    }

    /// What the last tree read returned.
    pub fn last_read(&self) -> Option<ReadStats> {
        todo!("task 5.3")
    }

    /// Tree reads made so far.
    pub fn reads(&self) -> u64 {
        todo!("task 5.3")
    }

    /// Frames received from the controller so far.
    pub fn frames_received(&self) -> u64 {
        todo!("task 5.3")
    }

    /// A screenshot through the agent.
    pub fn screenshot(&self, params: ScreenshotParams) -> Result<Screenshot, protocol::Error> {
        let _ = params;
        todo!("task 5.3")
    }

    /// Why the connection to the agent closed, once it has.
    pub fn close_reason(&self) -> Option<DisconnectReason> {
        todo!("task 5.3")
    }

    /// Stops the agent: `shutdown`, then a kill by pid past `timeout`.
    pub fn stop(&self, timeout: Duration) -> Result<Stopped, DeployError> {
        let _ = timeout;
        todo!("task 5.3")
    }
}

#[cfg(test)]
mod tests;
