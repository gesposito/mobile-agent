//! The server: rmcp's handler over the tool table (design D2 and D4 of the
//! `mcp-server` change), written against the handler trait rather than
//! rmcp's tool macros, since every tool's definition comes from the table.
//! Each call is one request on the daemon's socket through the CLI's client
//! (`crates/cli/src/client.rs`): a connection of its own, the daemon started
//! first when none of this build serves, the version check and the retries
//! the CLI makes, and the reply awaited on the runtime's blocking pool, so
//! the runtime's one thread keeps serving the other requests meanwhile.
//!
//! While a call waits for the daemon (design D7), the handler watches three
//! things beside the reply, the same for every tool: a tick every 5 s, on
//! which a client whose request carries a progress token gets a progress
//! notification naming the tool, the seconds elapsed and what the call waits
//! for; the request's cancellation token, which a client's cancellation
//! fires, on which the server sends the daemon the session's `cancel` and
//! lets the call go; and the bound, [`Options::bound`], 5 s past which a
//! call the daemon has not answered gets the same `cancel`, the daemon's
//! reply within 5 s more being its answer, else an execution error.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cli::client::{Endpoint, Reply};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, ServerCapabilities,
    ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value};

use crate::sessions::{self, Sessions};
use crate::tools::{Call, IF_CHANGED, Profile, Tool};
use crate::{instructions, results};

/// The server's name, as `initialize` and `server/discover` answer it.
pub const NAME: &str = "mobile-agent-mcp";

/// How often a running call reports its progress.
const PROGRESS: Duration = Duration::from_secs(5);

/// How long past the bound the server waits for the daemon's reply before
/// it cancels the call, and how long after the cancel before it gives the
/// call up.
const GRACE: Duration = Duration::from_secs(5);

/// What the command line set.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The bound of a call (design D7): less a margin, the cap of a timeout
    /// the call names, a call that names none sending none (design D7 of
    /// `real-app-fixes`); and the point past which a call the daemon has
    /// not answered is cancelled.
    pub bound: Duration,
    /// The tools served (design D10).
    pub profile: Profile,
    /// Whether each tool carries an `outputSchema`, its reply struct's, and
    /// each successful result a `structuredContent`, the reply's data
    /// (design D5).
    pub structured: bool,
}

/// The server: the daemon's endpoint, its sessions and the tools'
/// definitions, shared by every call.
#[derive(Clone)]
pub struct Server(Arc<Shared>);

pub struct Shared {
    pub endpoint: Endpoint,
    /// The session the server minted, `mcp-<pid>`, which a call's
    /// `session` overrides, and the targets its calls' sessions hold.
    pub sessions: Sessions,
    pub options: Options,
    /// The directory relative paths are taken from.
    cwd: PathBuf,
    /// Each tool the profile serves, with its definition.
    tools: Vec<(&'static Tool, rmcp::model::Tool)>,
    /// The instructions card, as `initialize` and `server/discover` carry
    /// it.
    card: String,
}

impl Server {
    pub fn new(endpoint: Endpoint, session: String, cwd: PathBuf, options: Options) -> Server {
        let tools = crate::tools::TOOLS
            .iter()
            .filter(|tool| options.profile.serves(tool))
            .map(|tool| (tool, tool.definition(options.structured)))
            .collect();
        Server(Arc::new(Shared {
            endpoint,
            sessions: Sessions::new(session),
            options,
            cwd,
            tools,
            card: instructions::card(options.profile, options.bound),
        }))
    }

    pub fn shared(&self) -> &Shared {
        &self.0
    }

    /// Runs `call` of `tool` and answers with its result, a successful one
    /// carrying the reply's data as structured content when the server
    /// runs with `--structured`.
    async fn run(&self, tool: &'static Tool, call: Call) -> Result<CallToolResult, ErrorData> {
        let (mut result, data) = self.answer(tool, call).await?;
        if self.0.options.structured && result.is_error != Some(true) {
            result.structured_content = Some(results::structured(data));
        }
        Ok(result)
    }

    /// Runs `call` of `tool`: its result, and the data a structured result
    /// carries.
    async fn answer(
        &self,
        tool: &'static Tool,
        mut call: Call,
    ) -> Result<(CallToolResult, Value), ErrorData> {
        if tool.action == "help" {
            let text = results::lines(&instructions::help());
            let data = serde_json::json!({"help": text});
            let result = CallToolResult::success(vec![ContentBlock::text(text)]);
            return Ok((result, data));
        }
        if tool.action == "doctor" {
            let probe = cli::doctor::System {
                endpoint: self.0.endpoint.clone(),
                session: call.session.unwrap_or_else(|| self.0.sessions.own.clone()),
            };
            let report = blocking(move || cli::doctor::run_checks(&probe)).await?;
            return Ok((results::doctor(&report), report.to_json()));
        }
        let mut reply = self.send(std::mem::take(&mut call.line)).await?;
        if !call.capped.is_empty() {
            let capped = call.capped.join("; ");
            reply.warning = Some(match reply.warning.take() {
                Some(warning) => format!("{warning}; {capped}"),
                None => capped,
            });
        }
        let Some(session) = &call.session else {
            return Ok((results::of(tool.action, &reply), reply.data));
        };
        self.0.sessions.record(session, tool.action, &reply);
        if tool.action == "screenshot" && reply.success {
            let unchanged = reply.data["image"].as_str().is_some_and(|image| {
                self.0
                    .sessions
                    .screenshot(session, call.device.as_deref(), image)
            });
            let result = results::screenshot(&reply, unchanged && call.flag(IF_CHANGED));
            return Ok((result, reply.data));
        }
        Ok((results::of(tool.action, &reply), reply.data))
    }

    /// Sends one request line to the daemon and waits for its reply, as
    /// the CLI sends a command's. This is where a call waits.
    async fn send(&self, line: Map<String, Value>) -> Result<Reply, ErrorData> {
        let endpoint = self.0.endpoint.clone();
        blocking(move || cli::client::call(&endpoint, &line).unwrap_or_else(Reply::failed)).await
    }

    /// Sends the daemon `session`'s `cancel` on a connection of its own,
    /// detached from the call it ends, so that the command in flight fails
    /// with `cancelled` and the target is free for the next call.
    fn cancel(&self, session: Option<&str>) {
        let Some(session) = session else {
            return;
        };
        let endpoint = self.0.endpoint.clone();
        let session = session.to_owned();
        tokio::task::spawn_blocking(move || sessions::cancel(&endpoint, &session));
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
            .with_instructions(self.0.card.clone())
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
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some((tool, definition)) = self
            .0
            .tools
            .iter()
            .find(|(tool, _)| tool.name == request.name)
        else {
            let message = match crate::tools::TOOLS
                .iter()
                .find(|tool| tool.name == request.name)
            {
                Some(tool) => format!(
                    "{} is not served: this server runs with --tools read-only, which serves {}",
                    tool.name,
                    self.0
                        .tools
                        .iter()
                        .map(|(tool, _)| tool.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                None => format!("no tool is named {}", request.name),
            };
            return Err(ErrorData::invalid_params(message, None));
        };
        let arguments = request.arguments.unwrap_or_default();
        let bound = self.0.options.bound;
        let call = tool.call(
            definition,
            &arguments,
            &self.0.sessions.own,
            &self.0.cwd,
            bound,
        );
        let session = call.session.clone();
        let waits_for = call.waits_for();
        let token = context.meta.get_progress_token();
        let work = self.run(tool, call);
        tokio::pin!(work);
        let mut ticks = tokio::time::interval(PROGRESS);
        ticks.tick().await;
        let mut elapsed = 0;
        let deadline = tokio::time::sleep(bound + GRACE);
        tokio::pin!(deadline);
        let mut cancelled = false;
        loop {
            tokio::select! {
                result = &mut work => return Ok(result?.into()),
                _ = ticks.tick() => {
                    elapsed += PROGRESS.as_secs();
                    if let Some(token) = &token {
                        let progress = ProgressNotificationParam::new(token.clone(), elapsed as f64)
                            .with_message(format!(
                                "{}: {elapsed} s elapsed, waiting for {waits_for}",
                                tool.name
                            ));
                        let _ = context.peer.notify_progress(progress).await;
                    }
                }
                () = &mut deadline => {
                    if cancelled {
                        return Ok(results::bound(tool.name, bound).into());
                    }
                    cancelled = true;
                    eprintln!(
                        "{NAME}: {} had no answer {} s past the bound of {} s; the session's cancel was sent",
                        tool.name,
                        GRACE.as_secs(),
                        bound.as_secs()
                    );
                    self.cancel(session.as_deref());
                    deadline.as_mut().reset(tokio::time::Instant::now() + GRACE);
                }
                () = context.ct.cancelled() => {
                    // The client gave the call up: rmcp drops the response.
                    eprintln!(
                        "{NAME}: the client cancelled {}; the session's cancel was sent",
                        tool.name
                    );
                    self.cancel(session.as_deref());
                    return Err(ErrorData::internal_error(
                        format!("{} was cancelled", tool.name),
                        None,
                    ));
                }
            }
        }
    }
}
