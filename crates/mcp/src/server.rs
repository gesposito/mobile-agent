//! The server: rmcp's handler over the tool table (design D2 and D4 of the
//! `mcp-server` change), written against the handler trait rather than
//! rmcp's tool macros, since every tool's definition comes from the table.
//! Each call is one request on the daemon's socket through the CLI's client
//! (`crates/cli/src/client.rs`): a connection of its own, the daemon started
//! first when none of this build serves, the version check and the retries
//! the CLI makes, and the reply awaited on the runtime's blocking pool, so
//! the runtime's one thread keeps serving the other requests meanwhile.

use std::path::PathBuf;
use std::sync::Arc;

use cli::client::{Endpoint, Reply};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value};

use crate::results;
use crate::sessions::Sessions;
use crate::tools::{Call, IF_CHANGED, Tool};

/// The server's name, as `initialize` and `server/discover` answer it.
pub const NAME: &str = "mobile-agent-mcp";

/// What the server tells a client about itself, in a paragraph that stands
/// alone within the 512 characters Codex keeps of it.
pub const INSTRUCTIONS: &str = "Drives Android devices, emulators, iOS Simulators and iPhones through the mobile-agent daemon. Call snapshot for the screen's elements and their refs (@e3), then act by ref or selector with click, fill, type, press, swipe and scroll; replies verify each action, and an error names its code and what to do next. This server acts in a session of its own and, when it exits, releases its target with the app left running.";

/// The server: the daemon's endpoint, its sessions and the tools'
/// definitions, shared by every call.
#[derive(Clone)]
pub struct Server(Arc<Shared>);

pub struct Shared {
    pub endpoint: Endpoint,
    /// The session the server minted, `mcp-<pid>`, which a call's
    /// `session` overrides, and the targets its calls' sessions hold.
    pub sessions: Sessions,
    /// The directory relative paths are taken from.
    cwd: PathBuf,
    /// Each tool with its definition.
    tools: Vec<(&'static Tool, rmcp::model::Tool)>,
}

impl Server {
    pub fn new(endpoint: Endpoint, session: String, cwd: PathBuf) -> Server {
        let tools = crate::tools::TOOLS
            .iter()
            .map(|tool| (tool, tool.definition()))
            .collect();
        Server(Arc::new(Shared {
            endpoint,
            sessions: Sessions::new(session),
            cwd,
            tools,
        }))
    }

    pub fn shared(&self) -> &Shared {
        &self.0
    }

    /// Runs `call` of `tool` and answers with its result.
    async fn run(&self, tool: &'static Tool, mut call: Call) -> Result<CallToolResult, ErrorData> {
        if tool.action == "doctor" {
            let probe = cli::doctor::System {
                endpoint: self.0.endpoint.clone(),
                session: call.session.unwrap_or_else(|| self.0.sessions.own.clone()),
            };
            let report = blocking(move || cli::doctor::run_checks(&probe)).await?;
            return Ok(results::doctor(&report));
        }
        let reply = self.send(std::mem::take(&mut call.line)).await?;
        let Some(session) = &call.session else {
            return Ok(results::of(tool.action, &reply));
        };
        self.0.sessions.record(session, tool.action, &reply);
        if tool.action == "screenshot" && reply.success {
            let unchanged = reply.data["image"].as_str().is_some_and(|image| {
                self.0
                    .sessions
                    .screenshot(session, call.device.as_deref(), image)
            });
            return Ok(results::screenshot(
                &reply,
                unchanged && call.flag(IF_CHANGED),
            ));
        }
        Ok(results::of(tool.action, &reply))
    }

    /// Sends one request line to the daemon and waits for its reply, as
    /// the CLI sends a command's. This is where a call waits.
    async fn send(&self, line: Map<String, Value>) -> Result<Reply, ErrorData> {
        let endpoint = self.0.endpoint.clone();
        blocking(move || cli::client::call(&endpoint, &line).unwrap_or_else(Reply::failed)).await
    }
}

/// Runs `work`, which blocks, on the runtime's blocking pool and awaits
/// it.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ErrorData> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ErrorData::internal_error(format!("the call failed: {error}"), None))
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self.0.tools.iter().map(|(_, tool)| tool.clone()).collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some((tool, definition)) = self
            .0
            .tools
            .iter()
            .find(|(tool, _)| tool.name == request.name)
        else {
            return Err(ErrorData::invalid_params(
                format!("no tool is named {}", request.name),
                None,
            ));
        };
        let arguments = request.arguments.unwrap_or_default();
        let call = tool.call(definition, &arguments, &self.0.sessions.own, &self.0.cwd);
        Ok(self.run(tool, call).await?.into())
    }
}
