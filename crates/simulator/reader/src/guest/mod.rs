//! The reader as it runs inside the Simulator.

pub mod extract;
pub mod objc;
pub mod process;
pub mod runtime;
pub mod server;

use std::os::unix::net::UnixStream;
use std::path::Path;

use protocol::Capabilities;
use protocol::message::SnapshotParams;
use protocol::session::AgentConfig;
use protocol::session::DisconnectReason;
use protocol::snapshot::Snapshot;

use crate::elements::{self, Limits};
use crate::guest::runtime::Runtime;
use crate::guest::server::{Source, Worker};

/// The runtime as a snapshot source, at the screen scale the host passed.
pub struct RuntimeSource {
    runtime: Runtime,
    scale: f64,
}

impl Source for RuntimeSource {
    fn read(&self, params: &SnapshotParams) -> Result<Snapshot, protocol::Error> {
        self.runtime
            .read(Limits::from_params(params), self.scale)
            .map(|read| elements::snapshot(&read))
            .map_err(|error| error.into_protocol())
    }
}

/// Why the reader stopped before serving.
#[derive(Debug)]
pub enum Stop {
    /// The private runtime could not be loaded.
    Runtime(String),
    /// The host's socket could not be reached.
    Connect(String),
}

/// Loads the runtime, then connects to the host's socket at `socket` and
/// serves it until the host shuts the reader down or goes away. Connecting
/// only once the runtime is loaded makes the connection itself the sign that
/// the reader can read.
pub fn run(socket: &Path, token: Option<String>, scale: f64) -> Result<DisconnectReason, Stop> {
    let worker = Worker::start(move || {
        Runtime::load()
            .map(|runtime| RuntimeSource { runtime, scale })
            .map_err(|failure| failure.to_string())
    })
    .map_err(Stop::Runtime)?;
    let stream = UnixStream::connect(socket)
        .map_err(|error| Stop::Connect(format!("{}: {error}", socket.display())))?;
    let config = AgentConfig {
        capabilities: Capabilities::TREE,
        agent: crate::AGENT.to_owned(),
        token,
        ..AgentConfig::default()
    };
    server::serve(stream, config, worker).map_err(|error| Stop::Connect(error.to_string()))
}
