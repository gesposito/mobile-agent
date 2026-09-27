//! What each action does. Target commands resolve their target, claim it
//! for the request's session, and run on the target's worker.

mod act;
mod aim;
mod view;

use std::time::Duration;

use mirror::backend::Owner;
use mirror::{Degradation, Snapshot, Verdict};
use serde_json::{Map, Value, json};

use crate::driver::{Launch, TargetInfo};
use crate::server::Daemon;
use crate::wire::{Failure, Request, Success, code};
use crate::worker::{Attached, Outcome};

/// How long waiting commands wait when they name no timeout.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// A command that runs on a target's worker.
type Command = fn(&mut Attached, &str, &Request) -> Outcome;

pub(crate) fn handle(daemon: &Daemon, request: &Request) -> Outcome {
    let command: Command = match request.action.as_str() {
        "status" => return status(daemon),
        "devices" => return devices(daemon),
        "close" => return close(daemon, request),
        "open" => open,
        "snapshot" => view::snapshot,
        "diff_snapshot" => view::diff_snapshot,
        "get" => view::get,
        "wait" => view::wait,
        "screenshot" => view::screenshot,
        "click" => act::click,
        "fill" => act::fill,
        "type" => act::type_text,
        "press" => act::press,
        "swipe" => act::swipe,
        "scroll" => act::scroll,
        other => {
            return Err(Failure::new(
                code::UNKNOWN_ACTION,
                format!("unknown action {other:?}; run mobile-agent --help for the commands"),
            ));
        }
    };
    on_target(daemon, request, command).map(|(success, _)| success)
}

/// The timeout of a waiting command.
pub(crate) fn timeout(request: &Request) -> Result<Duration, Failure> {
    Ok(request
        .u64("timeout")?
        .map_or(DEFAULT_TIMEOUT, Duration::from_millis))
}

/// Resolves the request's target, claims it for the request's session and
/// runs `command` on its worker.
fn on_target(
    daemon: &Daemon,
    request: &Request,
    command: Command,
) -> Result<(Success, TargetInfo), Failure> {
    let (target, handle) = {
        let mut registry = daemon.registry();
        let target = registry.resolve(&request.session)?;
        registry.claim(&target, &request.session, request.flag("force"))?;
        let handle = registry.handle(&target)?;
        (target, handle)
    };
    let request = request.clone();
    let success =
        handle.run(move |attached| command(attached, &request.session.clone(), &request))?;
    Ok((success, target))
}

pub(crate) fn millis(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1e6).round() / 1e3
}

fn target_json(target: &TargetInfo) -> Map<String, Value> {
    let mut entry = Map::new();
    entry.insert("id".into(), target.id.clone().into());
    entry.insert("platform".into(), target.platform.id().into());
    entry.insert("class".into(), target.class.id().into());
    entry.insert("name".into(), target.name.clone().into());
    entry
}

fn status(daemon: &Daemon) -> Outcome {
    let mut registry = daemon.registry();
    let targets: Vec<Value> = registry
        .targets(false)
        .iter()
        .map(|target| {
            let worker = registry.worker_status(&target.id).unwrap_or_default();
            let mut entry = target_json(target);
            entry.insert("attached".into(), worker.attached.into());
            entry.insert("app".into(), worker.app.into());
            entry.insert("claimedBy".into(), registry.holder(&target.id).into());
            if let Some(error) = worker.error {
                entry.insert("error".into(), error.into());
            }
            Value::Object(entry)
        })
        .collect();
    let sessions: Vec<Value> = registry
        .sessions()
        .iter()
        .map(|(name, settings)| {
            json!({
                "name": name,
                "targets": registry.held_by(name),
                "defaultTimeoutMs": settings.default_timeout.map(|t| t.as_millis() as u64),
            })
        })
        .collect();
    let data = json!({
        "daemon": {
            "pid": std::process::id(),
            "version": daemon.version,
            "socket": daemon.paths.socket().to_string_lossy(),
            "uptimeMs": daemon.started.elapsed().as_millis() as u64,
            "idleTimeoutMs": daemon.idle.map(|idle| idle.as_millis() as u64),
            "drivers": registry.driver_names(),
        },
        "targets": targets,
        "sessions": sessions,
    });
    Ok(Success::new(data).warn(discovery_warning(registry.failures())))
}

fn discovery_warning(failures: &[String]) -> Option<String> {
    (!failures.is_empty()).then(|| {
        format!(
            "some drivers could not list their targets: {}",
            failures.join("; ")
        )
    })
}

fn devices(daemon: &Daemon) -> Outcome {
    let mut registry = daemon.registry();
    let devices: Vec<Value> = registry
        .targets(true)
        .iter()
        .map(|target| {
            let mut entry = target_json(target);
            entry.insert("model".into(), target.model.clone().into());
            entry.insert("os".into(), target.os.clone().into());
            entry.insert("claimedBy".into(), registry.holder(&target.id).into());
            Value::Object(entry)
        })
        .collect();
    Ok(Success::new(json!({"devices": devices})).warn(discovery_warning(registry.failures())))
}

fn open(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let app = request.required_str("app")?.to_owned();
    let mut env = Vec::new();
    for pair in request.strings("env")? {
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            Failure::new(
                code::INVALID_ARGUMENT,
                format!("--env takes KEY=VALUE, not {pair:?}"),
            )
        })?;
        env.push((key.to_owned(), value.to_owned()));
    }
    let launch = Launch {
        relaunch: request.flag("relaunch"),
        env,
    };
    target.apps.launch(&app, &launch)?;
    target.session(session).app = Some(app.clone());
    target.set_app(Some(app.clone()));
    let settled = target.mirror.settle(&mirror::SettleOptions {
        max: request
            .u64("timeout")?
            .map_or(mirror::DEFAULT_SETTLE_MAX, Duration::from_millis),
        ..mirror::SettleOptions::default()
    })?;
    Ok(Success::new(json!({
        "opened": app,
        "target": target.info.id,
        "settled": settled.settled,
        "elapsedMs": millis(settled.elapsed),
    })))
}

fn close(daemon: &Daemon, request: &Request) -> Outcome {
    let (success, target) = on_target(daemon, request, close_app)?;
    daemon.registry().release(&target.id, &request.session);
    Ok(success)
}

fn close_app(target: &mut Attached, session: &str, _request: &Request) -> Outcome {
    let app = target.session(session).app.take();
    if let Some(app) = &app {
        target.apps.terminate(app)?;
        target.set_app(None);
    }
    Ok(Success::new(json!({
        "closed": true,
        "app": app,
        "target": target.info.id,
    })))
}

/// The app in front, as agent-browser's `origin` names the page: system UI
/// that owns the foreground, else the package of the top-most application
/// window.
pub(crate) fn origin(snapshot: &Snapshot) -> String {
    match &snapshot.foreground {
        Owner::System(name) => name.clone(),
        Owner::Target => snapshot
            .windows
            .iter()
            .find(|w| {
                w.kind == protocol::snapshot::WindowType::APPLICATION && w.owner == Owner::Target
            })
            .and_then(|w| w.package.clone())
            .unwrap_or_default(),
    }
}

pub(crate) fn verdict_json(verdict: &Verdict) -> Value {
    json!({
        "backend": verdict.backend.name,
        "fallback": verdict.backend.fallback,
        "degraded": verdict.degraded,
        "reasons": verdict
            .reasons
            .iter()
            .map(|reason| json!({"code": reason.code(), "advice": reason.advice()}))
            .collect::<Vec<_>>(),
    })
}

/// The warning a degraded snapshot carries.
pub(crate) fn verdict_warning(verdict: &Verdict) -> Option<String> {
    if !verdict.degraded {
        return None;
    }
    let codes: Vec<&str> = verdict.reasons.iter().map(Degradation::code).collect();
    Some(format!(
        "degraded snapshot ({}): {}",
        codes.join(", "),
        verdict.advice().join("; ")
    ))
}
