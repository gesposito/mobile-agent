//! `mobile-agent-mcp`: the Model Context Protocol server over the
//! mobile-agent daemon. See the crate's README.

use std::process::ExitCode;

use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt};

#[derive(Clone)]
struct Server;

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("mobile-agent-mcp", env!("CARGO_PKG_VERSION")),
        )
    }
}

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("mobile-agent-mcp: cannot start the runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async {
        match Server.serve(rmcp::transport::stdio()).await {
            Ok(running) => {
                let _ = running.waiting().await;
            }
            Err(error) => eprintln!("mobile-agent-mcp: {error}"),
        }
    });
    ExitCode::SUCCESS
}
