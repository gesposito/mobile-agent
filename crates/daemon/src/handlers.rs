//! What each action does. Target commands resolve their target, claim it
//! for the request's session, and run on the target's worker.

use std::time::Duration;

use mirror::backend::Owner;
use mirror::{Degradation, Snapshot, SnapshotOptions, Verdict};
use serde_json::{Map, Value, json};

use crate::driver::{Launch, TargetInfo};
use crate::server::Daemon;
use crate::wire::{Failure, Request, Success, code};
use crate::worker::{Attached, Outcome};

/// A command that runs on a target's worker.
type Command = fn(&mut Attached, &str, &Request) -> Outcome;

pub(crate) fn handle(daemon: &Daemon, request: &Request) -> Outcome {
    match request.action.as_str() {
        "status" => status(daemon),
        "devices" => devices(daemon),
        "open" => on_target(daemon, request, open).map(|(success, _)| success),
        "close" => close(daemon, request),
        "snapshot" => on_target(daemon, request, snapshot).map(|(success, _)| success),
        other => Err(Failure::new(
            code::UNKNOWN_ACTION,
            format!("unknown action {other:?}; run mobile-agent --help for the commands"),
        )),
    }
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

fn millis(duration: Duration) -> f64 {
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
    let settled = target.mirror.settle(&mirror::SettleOptions::default())?;
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

/// The snapshot options a request asks for.
fn snapshot_options(request: &Request) -> Result<SnapshotOptions, Failure> {
    let max_depth = request
        .u64("maxDepth")?
        .map(|depth| usize::try_from(depth).unwrap_or(usize::MAX));
    Ok(SnapshotOptions {
        interactive: request.flag("interactive"),
        max_depth,
    })
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

fn snapshot(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let options = snapshot_options(request)?;
    let snapshot = target.mirror.snapshot(&options)?;
    let mut data = Map::new();
    if request.flag("delta") {
        let full = request.flag("full") || target.delta_session.as_deref() != Some(session);
        let delta = target.mirror.delta_snapshot(&options, full)?;
        target.delta_session = Some(session.to_owned());
        let delta: Value = serde_json::from_str(&delta.to_json())
            .map_err(|e| Failure::internal(format!("the delta snapshot is not JSON: {e}")))?;
        data.insert("snapshot".into(), delta);
    } else {
        let refs: Map<String, Value> = snapshot
            .elements
            .iter()
            .filter_map(|element| {
                element.reference.map(|reference| {
                    (
                        reference.to_string(),
                        json!({"name": element.name, "role": element.role.as_str()}),
                    )
                })
            })
            .collect();
        let removed: Vec<String> = snapshot
            .removed_refs
            .iter()
            .map(|r| format!("@{r}"))
            .collect();
        data.insert("snapshot".into(), snapshot.text.clone().into());
        data.insert("refs".into(), Value::Object(refs));
        data.insert("removedRefs".into(), removed.into());
    }
    data.insert("origin".into(), origin(&snapshot).into());
    data.insert("revision".into(), snapshot.revision.into());
    data.insert("target".into(), target.info.id.clone().into());
    data.insert("verdict".into(), verdict_json(&snapshot.verdict));
    let warning = verdict_warning(&snapshot.verdict);
    let entry = target.session(session);
    entry.seen.extend(snapshot.refs());
    entry.last = Some(snapshot);
    Ok(Success::new(Value::Object(data)).warn(warning))
}
