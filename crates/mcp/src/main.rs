//! `mobile-agent-mcp`: the Model Context Protocol server over the
//! mobile-agent daemon (the `mcp-server` change). It serves MCP on its
//! standard input and output in both protocol eras, the 2025 revisions'
//! `initialize` and the 2026-07-28 revision's `server/discover`, with one
//! tool per daemon action an agent drives, each call one request on the
//! daemon's socket in a session of the server's own. It writes its own
//! lines to standard error alone.
//!
//! It ends when its input ends, at SIGTERM or SIGINT, and when the client
//! leaves before choosing a lifecycle, and on each of these ways out it
//! releases the targets its sessions hold with the apps left running
//! (design D9). See the crate's README.

mod results;
mod server;
mod sessions;
mod tools;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use cli::client::Endpoint;
use rmcp::ServiceExt;
use rmcp::service::ServerInitializeError;
use tokio::signal::unix::{SignalKind, signal};

use crate::server::{NAME, Server};

/// How long the server waits on its way out for the daemon to release its
/// sessions' targets. A release still queued behind a command in flight
/// when the bound passes goes ahead in the daemon all the same.
const RELEASE: Duration = Duration::from_secs(5);

/// The daemon's endpoint as the CLI resolves it: the socket directory from
/// `MOBILE_AGENT_SOCKET_DIR`, else the CLI's default; the daemon executable
/// from `MOBILE_AGENT_DAEMON_BIN`, else the one beside this executable or
/// on `PATH`; and for a daemon the server starts, the idle timeout and the
/// emulator controller from `MOBILE_AGENT_IDLE_TIMEOUT_MS` and
/// `MOBILE_AGENT_EMULATOR_CONTROLLER`, else from the CLI's settings files.
fn endpoint() -> Endpoint {
    let var = |key: &str| {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    };
    let config = cli::config::load(var("MOBILE_AGENT_CONFIG").map(PathBuf::from).as_deref())
        .unwrap_or_else(|message| {
            eprintln!("{NAME}: {message}; the server goes on without the settings file");
            cli::config::Config::default()
        });
    Endpoint {
        dir: cli::client::socket_dir(var("MOBILE_AGENT_SOCKET_DIR").map(PathBuf::from).as_deref()),
        daemon: var("MOBILE_AGENT_DAEMON_BIN").map(PathBuf::from),
        idle_timeout: var("MOBILE_AGENT_IDLE_TIMEOUT_MS").or(config.idle_timeout),
        emulator_controller: var("MOBILE_AGENT_EMULATOR_CONTROLLER").or(config.emulator_controller),
    }
}

/// Serves the client on standard input and output until it leaves or a
/// signal ends the server, and returns the exit status. The signals'
/// handlers are in place before the first request is read.
async fn serve(server: Server) -> i32 {
    let (mut terminate, mut interrupt) = match (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) {
        (Ok(terminate), Ok(interrupt)) => (terminate, interrupt),
        (Err(error), _) | (_, Err(error)) => {
            eprintln!("{NAME}: cannot handle signals: {error}");
            return 1;
        }
    };
    tokio::select! {
        status = serving(server) => status,
        _ = terminate.recv() => 0,
        _ = interrupt.recv() => 0,
    }
}

/// Serves the client until its input ends.
async fn serving(server: Server) -> i32 {
    match server.serve(rmcp::transport::stdio()).await {
        Ok(running) => {
            let _ = running.waiting().await;
            0
        }
        // The client left before a request that chooses a lifecycle, as
        // one that only discovered the server does.
        Err(ServerInitializeError::ConnectionClosed(_)) => 0,
        Err(error) => {
            eprintln!("{NAME}: {error}");
            1
        }
    }
}

/// Releases the targets the server's sessions hold, waiting at most
/// [`RELEASE`] for the daemon.
fn release(server: &Server) {
    let (done, finished) = mpsc::channel();
    let server = server.clone();
    std::thread::spawn(move || {
        let shared = server.shared();
        shared.sessions.release(&shared.endpoint);
        let _ = done.send(());
    });
    if finished.recv_timeout(RELEASE).is_err() {
        eprintln!(
            "{NAME}: the daemon had not released the session's targets within {} s",
            RELEASE.as_secs()
        );
    }
}

fn main() {
    let endpoint = endpoint();
    let session = format!("mcp-{}", std::process::id());
    let cwd = std::env::current_dir().unwrap_or_default();
    eprintln!(
        "{NAME} {}: session {session}, the daemon's socket {}",
        env!("CARGO_PKG_VERSION"),
        endpoint.socket().display()
    );
    let server = Server::new(endpoint, session, cwd);
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{NAME}: cannot start the runtime: {error}");
            std::process::exit(1);
        }
    };
    let status = runtime.block_on(serve(server.clone()));
    release(&server);
    // The runtime is left running: dropping it would wait for the read of
    // standard input that a signal interrupted, which never ends while the
    // client holds the pipe open.
    std::process::exit(status);
}
