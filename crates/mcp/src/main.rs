//! `mobile-agent-mcp`: the Model Context Protocol server over the
//! mobile-agent daemon (the `mcp-server` change). It serves MCP on its
//! standard input and output in both protocol eras, the 2025 revisions'
//! `initialize` and the 2026-07-28 revision's `server/discover`, with one
//! tool per daemon action an agent drives, each call one request on the
//! daemon's socket in a session of the server's own. It writes its own
//! lines to standard error alone. See the crate's README.

mod results;
mod server;
mod tools;

use std::path::PathBuf;
use std::process::ExitCode;

use cli::client::Endpoint;
use rmcp::ServiceExt;
use rmcp::service::ServerInitializeError;

use crate::server::{NAME, Server};

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

/// Serves the client on standard input and output until it leaves, and
/// returns the exit status.
async fn serve(server: Server) -> ExitCode {
    match server.serve(rmcp::transport::stdio()).await {
        Ok(running) => {
            let _ = running.waiting().await;
            ExitCode::SUCCESS
        }
        // The client left before a request that chooses a lifecycle, as
        // one that only discovered the server does.
        Err(ServerInitializeError::ConnectionClosed(_)) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{NAME}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
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
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(serve(server))
}
