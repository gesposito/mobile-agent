//! The `mobile-agent` CLI (design D1, D9): a thin client of the daemon. It
//! parses agent-browser's grammar, sends one JSON request per command to the
//! daemon over its Unix socket, and prints the reply as text or, with
//! `--json`, as agent-browser's envelope. It links nothing of the daemon, so
//! a command costs little more than a process start and a socket round trip.

pub mod args;
pub mod batch;
pub mod client;
pub mod config;
pub mod output;
pub mod request;

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

use clap::Parser;
use clap::error::ErrorKind;
use serde_json::Value;

use crate::client::{Failure, Reply};

/// A command the grammar does not know.
pub const UNKNOWN_COMMAND: &str = "unknown_command";
/// A settings file that cannot be read.
pub const INVALID_CONFIG: &str = "invalid_config";

fn truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "y" | "t"
    )
}

/// Whether a command line that did not parse asked for JSON.
fn wants_json(args: &[OsString]) -> bool {
    args.iter().any(|arg| arg == "--json")
        || std::env::var("MOBILE_AGENT_JSON").is_ok_and(|v| truthy(&v))
        || config::load(
            std::env::var_os("MOBILE_AGENT_CONFIG")
                .map(std::path::PathBuf::from)
                .as_deref(),
        )
        .is_ok_and(|c| c.json == Some(true))
}

pub(crate) fn write(stream: &mut dyn Write, text: &str) {
    let _ = stream.write_all(text.as_bytes());
    let _ = stream.flush();
}

/// Prints `reply` and returns the exit status: 0 on success, 1 on failure.
pub(crate) fn finish(json: bool, action: &str, reply: &Reply) -> i32 {
    if json {
        write(
            &mut std::io::stdout().lock(),
            &format!("{}\n", output::json_line(reply)),
        );
    } else {
        let (out, err) = output::human(action, reply);
        write(&mut std::io::stdout().lock(), &out);
        write(&mut std::io::stderr().lock(), &err);
    }
    i32::from(!reply.success)
}

fn usage_error(error: &clap::Error, json: bool) -> i32 {
    match error.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            write(&mut std::io::stdout().lock(), &error.to_string());
            0
        }
        kind => {
            let code = if kind == ErrorKind::InvalidSubcommand {
                UNKNOWN_COMMAND
            } else {
                request::INVALID_ARGUMENT
            };
            let rendered = error.to_string();
            if json {
                let message = rendered.trim().trim_start_matches("error: ").to_owned();
                finish(true, "", &Reply::failed(Failure::new(code, message)))
            } else {
                write(&mut std::io::stderr().lock(), &rendered);
                1
            }
        }
    }
}

/// What a command runs with once flags, environment and config are merged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub json: bool,
    pub session: String,
    pub platform: Option<String>,
    pub device: Option<String>,
    pub force: bool,
    pub endpoint: client::Endpoint,
}

/// Runs one command against the daemon: its action and the reply.
pub(crate) fn execute(command: &args::Command, settings: &Settings, cwd: &Path) -> (String, Reply) {
    let mut request = match request::build(command, cwd) {
        Ok(request) => request,
        Err(failure) => return (String::new(), Reply::failed(failure)),
    };
    let action = request
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    request.insert("id".into(), client::request_id().into());
    request.insert("session".into(), settings.session.clone().into());
    if let Some(platform) = &settings.platform {
        request.insert("platform".into(), platform.to_ascii_lowercase().into());
    }
    if let Some(device) = &settings.device {
        request.insert("device".into(), device.clone().into());
    }
    if settings.force {
        request.insert("force".into(), Value::Bool(true));
    }
    let reply = client::call(&settings.endpoint, &request).unwrap_or_else(Reply::failed);
    (action, reply)
}

/// Runs the CLI with `args`, the program name first, and returns the exit
/// status.
pub fn run(args: Vec<OsString>) -> i32 {
    let cli = match args::Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => return usage_error(&error, wants_json(&args)),
    };
    let global = &cli.global;
    let config = match config::load(global.config.as_deref()) {
        Ok(config) => config,
        Err(message) => {
            let failure = Failure::new(INVALID_CONFIG, message);
            return finish(global.json, "", &Reply::failed(failure));
        }
    };
    let settings = Settings {
        json: global.json || config.json.unwrap_or(false),
        session: global
            .session
            .clone()
            .or(config.session)
            .unwrap_or_else(|| "default".to_owned()),
        platform: global.platform.clone().or(config.platform),
        device: global.device.clone().or(config.device),
        force: global.force,
        endpoint: client::Endpoint {
            dir: client::socket_dir(global.socket_dir.as_deref()),
            daemon: global.daemon_bin.clone(),
            idle_timeout: global.idle_timeout.clone().or(config.idle_timeout),
        },
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    if let args::Command::Batch(batch) = &cli.command {
        return batch::run(batch, &settings, &cwd);
    }
    let (action, reply) = execute(&cli.command, &settings, &cwd);
    finish(settings.json, &action, &reply)
}
