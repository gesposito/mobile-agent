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
mod paths;
mod tree;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use mirror::backend::{Backend, FrameSource, ReadReason, StreamItem, SystemClock, Timed};
use protocol::code;
use protocol::event::Stream;
use protocol::message::{Call, Reply, Screenshot, ScreenshotParams};
use protocol::session::{DisconnectReason, HostEvent};

use self::controller::{ControllerFrames, FramePump, FrameStore};
use self::events::AgentEvents;
use self::input::SessionInput;
use self::tree::AgentTree;
use crate::adb;
use crate::agent::{AgentInfo, Attached, Connection, DeployError, Deployer, Outcome, Stopped};

pub use controller::{
    ControllerLink, INPUT_QUIET, encode_jpeg, frame_meta, resize_rgb, rotate_rgb,
};
pub use events::stream_item;
pub use input::{agent_call, input_ack, swipe_touches};
pub use paths::{ControllerChoice, Path, Paths, Selection, Status, select_paths};
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

/// What the session's backend parts share.
struct Inner {
    connection: Connection,
    agent: AgentInfo,
    outcome: Outcome,
    deployer: Option<Deployer>,
    scale: f64,
    options: SessionOptions,
    /// The paths chosen when the session started.
    paths: Paths,
    controller: Option<ControllerLink>,
    frames: Arc<Mutex<FrameStore>>,
    frames_received: Arc<AtomicU64>,
    frame_error: Arc<Mutex<Option<String>>>,
    pump_stop: Arc<AtomicBool>,
    /// The event stream's receiving end, until the backend takes it.
    items: Mutex<Option<Receiver<Timed<StreamItem>>>>,
    /// Whether the last read showed a window that sends no change events.
    event_less: AtomicBool,
    /// Input the agent did not inject since the last read.
    foreign_input: Mutex<Option<Instant>>,
    /// Streams whose resync failed, to retry on the next read.
    failed_resyncs: Arc<Mutex<Vec<Stream>>>,
    last_read: Mutex<Option<ReadStats>>,
    reads: AtomicU64,
    controller_inputs: AtomicU64,
    agent_inputs: AtomicU64,
    /// Why the controller stopped taking input, once it failed.
    controller_failed: Mutex<Option<String>>,
}

impl Inner {
    fn note_foreign_input(&self, at: Instant) {
        *self
            .foreign_input
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(at);
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.pump_stop.store(true, Ordering::SeqCst);
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("agent", &self.inner.agent)
            .field("scale", &self.inner.scale)
            .field("controller", &self.inner.controller.is_some())
            .finish_non_exhaustive()
    }
}

/// How long `shutdown` may take to be answered.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(2);

/// Deadline of a screenshot through the agent.
const SCREENSHOT_DEADLINE: Duration = Duration::from_secs(10);

impl Session {
    /// A session over `attached`: reads the display density for the scale,
    /// and uses `controller`, when given, for touches and frames.
    pub fn start(
        attached: Attached,
        options: SessionOptions,
        controller: Option<ControllerLink>,
    ) -> Result<Session, SessionError> {
        let selection = Selection {
            paths: controller
                .as_ref()
                .map_or_else(|| Paths::agent(false, None), Paths::controller),
            link: controller,
        };
        Session::start_selected(attached, options, selection)
    }

    /// A session over `attached` on the paths `selection` chose (see
    /// [`select_paths`]).
    pub fn start_selected(
        attached: Attached,
        options: SessionOptions,
        selection: Selection,
    ) -> Result<Session, SessionError> {
        let density = attached.deployer().device().shell("wm density")?;
        let text = density.stdout_text();
        let scale = scale_from_density(&text)
            .ok_or_else(|| SessionError::Density(format!("{text}{}", density.stderr_text())))?;
        let (connection, agent, outcome, deployer) = attached.into_parts();
        Ok(Session::over(
            connection,
            agent,
            outcome,
            Some(deployer),
            scale,
            options,
            selection,
        ))
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
        selection: Selection,
    ) -> Session {
        let Selection {
            link: controller,
            paths,
        } = selection;
        let (sender, items) = mpsc::channel();
        let failed_resyncs = Arc::new(Mutex::new(Vec::new()));
        let events = sender.clone();
        let failed = Arc::clone(&failed_resyncs);
        connection.set_sink(Box::new(move |at, event| {
            if let HostEvent::ResyncFailed { stream, .. } = &event {
                failed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(*stream);
            }
            if let Some(item) = stream_item(event) {
                let _ = events.send(Timed { at, item });
            }
        }));
        let pump = FramePump {
            store: Arc::new(Mutex::new(FrameStore::default())),
            items: sender,
            received: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
        };
        if let Some(link) = &controller {
            link.start_frames(options.frame_width, pump.clone());
        }
        Session {
            inner: Arc::new(Inner {
                connection,
                agent,
                outcome,
                deployer,
                scale,
                options,
                paths,
                controller,
                frames: pump.store,
                frames_received: pump.received,
                frame_error: pump.error,
                pump_stop: pump.stop,
                items: Mutex::new(Some(items)),
                event_less: AtomicBool::new(false),
                foreign_input: Mutex::new(None),
                failed_resyncs,
                last_read: Mutex::new(None),
                reads: AtomicU64::new(0),
                controller_inputs: AtomicU64::new(0),
                agent_inputs: AtomicU64::new(0),
                controller_failed: Mutex::new(None),
            }),
        }
    }

    /// Notes input the agent did not inject at `at`, such as a controller
    /// touch: the next read waits until it is [`INPUT_QUIET`] old and first
    /// empties the agent's node cache when a window sends no change events.
    #[cfg(test)]
    pub(crate) fn note_foreign_input(&self, at: Instant) {
        self.inner.note_foreign_input(at);
    }

    /// What the session reports: the agent, the active paths, the keyboard
    /// capability, the fallback warning and what failed since.
    pub fn status(&self) -> Status {
        let inner = &*self.inner;
        let controller_error = self.controller_error();
        let mut paths = inner.paths.clone();
        if controller_error.is_some() {
            paths.input = Path::Agent;
        }
        Status {
            serial: inner.agent.serial.clone(),
            agent: inner.agent.agent.clone(),
            protocol: inner.agent.protocol,
            pid: inner.agent.pid,
            socket: inner.agent.socket.clone(),
            reused: inner.outcome == Outcome::Reused,
            paths,
            inputs: self.inputs(),
            frames_received: self.frames_received(),
            controller_error,
            frame_error: self.frame_error(),
            disconnected: self.close_reason().map(|reason| reason.to_string()),
        }
    }

    /// The backend a `mirror::Mirror` runs on. Taken once: the event stream
    /// has one reader.
    ///
    /// # Panics
    ///
    /// When called a second time.
    pub fn backend(&self) -> Backend {
        let items = self
            .inner
            .items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .expect("the backend of a session is taken once");
        let frames = self.inner.controller.as_ref().map(|_| {
            Box::new(ControllerFrames {
                store: Arc::clone(&self.inner.frames),
            }) as Box<dyn FrameSource>
        });
        Backend {
            tree: Box::new(AgentTree {
                inner: Arc::clone(&self.inner),
            }),
            events: Box::new(AgentEvents { items }),
            frames,
            input: Box::new(SessionInput {
                inner: Arc::clone(&self.inner),
            }),
            clock: Box::new(SystemClock),
        }
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
        self.inner.scale
    }

    /// Whether touches and frames go through the emulator's controller.
    pub fn uses_controller(&self) -> bool {
        self.inner.controller.is_some()
    }

    /// What the last tree read returned.
    pub fn last_read(&self) -> Option<ReadStats> {
        *self
            .inner
            .last_read
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Tree reads made so far.
    pub fn reads(&self) -> u64 {
        self.inner.reads.load(Ordering::SeqCst)
    }

    /// Frames received from the controller so far.
    pub fn frames_received(&self) -> u64 {
        self.inner.frames_received.load(Ordering::SeqCst)
    }

    /// Inputs sent so far: through the controller, and through the agent.
    pub fn inputs(&self) -> (u64, u64) {
        (
            self.inner.controller_inputs.load(Ordering::SeqCst),
            self.inner.agent_inputs.load(Ordering::SeqCst),
        )
    }

    /// Why the controller stopped taking input, once it failed; input then
    /// goes through the agent.
    pub fn controller_error(&self) -> Option<String> {
        self.inner
            .controller_failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The last failure of the controller's frame stream, which the session
    /// subscribes to again.
    pub fn frame_error(&self) -> Option<String> {
        self.inner
            .frame_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// A screenshot through the agent.
    pub fn screenshot(&self, params: ScreenshotParams) -> Result<Screenshot, protocol::Error> {
        match self
            .inner
            .connection
            .call(Call::Screenshot(params), SCREENSHOT_DEADLINE)?
        {
            Reply::Screenshot(shot) => Ok(shot),
            _ => Err(protocol::Error::new(
                code::PROTOCOL_ERROR,
                "the agent answered a screenshot request with another result",
            )),
        }
    }

    /// Why the connection to the agent closed, once it has.
    pub fn close_reason(&self) -> Option<DisconnectReason> {
        self.inner.connection.close_reason()
    }

    /// Stops the agent: `shutdown`, then a kill by pid past `timeout`.
    pub fn stop(&self, timeout: Duration) -> Result<Stopped, DeployError> {
        let inner = &*self.inner;
        inner.pump_stop.store(true, Ordering::SeqCst);
        match &inner.deployer {
            Some(deployer) => deployer.stop_agent(
                &inner.connection,
                inner.agent.pid,
                &inner.agent.socket,
                timeout,
            ),
            None => {
                let _ = inner.connection.call(Call::Shutdown, SHUTDOWN_DEADLINE);
                inner.connection.close();
                Ok(Stopped::Exited)
            }
        }
    }
}

#[cfg(test)]
mod tests;
