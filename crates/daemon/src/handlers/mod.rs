//! What each action does. Target commands resolve their target, claim it
//! for the request's session, and run on the target's worker.

mod act;
mod aim;
mod view;

use std::time::{Duration, Instant};

use mirror::backend::Owner;
use mirror::{Degradation, Snapshot, SnapshotOptions, Verdict};
use serde_json::{Map, Value, json};

use crate::driver::{AttachOptions, Launch, Platform, TargetInfo, TargetReport};
use crate::registry::Selection;
use crate::server::Daemon;
use crate::wire::{Failure, Request, Success, code};
use crate::worker::{Attached, Outcome};

/// How long waiting commands wait when neither they, their session nor
/// the client name a timeout.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest timeout a command or session may set. The CLI keeps its
/// socket open this long plus a margin for a wait that names no timeout,
/// since it cannot know the session's default.
pub(crate) const MAX_TIMEOUT: Duration = Duration::from_secs(600);

/// A command that runs on a target's worker.
type Command = fn(&mut Attached, &str, &Request) -> Outcome;

pub(crate) fn handle(daemon: &Daemon, request: &Request) -> Outcome {
    let command: Command = match request.action.as_str() {
        "status" => return status(daemon),
        "devices" => return devices(daemon, request),
        "close" => return close(daemon, request),
        "set_timeout" => return set_timeout(daemon, request),
        "doctor" => return doctor(daemon),
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

/// A timeout in milliseconds, at most [`MAX_TIMEOUT`].
fn bounded(key: &str, ms: u64) -> Result<Duration, Failure> {
    let timeout = Duration::from_millis(ms);
    if timeout > MAX_TIMEOUT {
        return Err(Failure::new(
            code::INVALID_ARGUMENT,
            format!(
                "{key} of {ms} ms is longer than the {} ms a timeout may be",
                MAX_TIMEOUT.as_millis()
            ),
        ));
    }
    Ok(timeout)
}

/// The timeout of a waiting command: its own `timeout`, else the default
/// the request carries (the session's, or else the client's), else
/// [`DEFAULT_TIMEOUT`].
pub(crate) fn timeout(request: &Request) -> Result<Duration, Failure> {
    if let Some(ms) = request.u64("timeout")? {
        return bounded("timeout", ms);
    }
    match request.u64("defaultTimeout")? {
        Some(ms) => bounded("defaultTimeout", ms),
        None => Ok(DEFAULT_TIMEOUT),
    }
}

/// How long a settle wait may last: the command's own `timeout`, else the
/// maximum of design D8. The session's default does not apply.
pub(crate) fn settle_max(request: &Request) -> Result<Duration, Failure> {
    match request.u64("timeout")? {
        Some(ms) => bounded("timeout", ms),
        None => Ok(mirror::DEFAULT_SETTLE_MAX),
    }
}

/// What `doctor` asks of the daemon: its identity, drivers and the rows
/// the drivers contribute to the compatibility table.
fn doctor(daemon: &Daemon) -> Outcome {
    let registry = daemon.registry();
    let rows: Vec<Value> = registry
        .compatibility()
        .into_iter()
        .map(|row| {
            json!({
                "subject": row.subject,
                "feature": row.feature,
                "status": row.status,
                "note": row.note,
            })
        })
        .collect();
    let note = rows.is_empty().then_some(
        "no driver has supplied compatibility data; the Simulator driver fills this table with each Xcode and iOS runtime it was validated on",
    );
    Ok(Success::new(json!({
        "pid": std::process::id(),
        "version": daemon.version,
        "drivers": registry.driver_names(),
        "compatibility": {"rows": rows, "note": note},
    })))
}

/// Sets the session's default timeout for waiting commands.
fn set_timeout(daemon: &Daemon, request: &Request) -> Outcome {
    let ms = request.u64("ms")?.ok_or_else(|| {
        Failure::new(
            code::INVALID_ARGUMENT,
            "set timeout needs a number of milliseconds",
        )
    })?;
    let timeout = bounded("the default timeout", ms)?;
    daemon
        .registry()
        .set_default_timeout(&request.session, timeout);
    Ok(Success::new(json!({
        "session": request.session,
        "defaultTimeoutMs": ms,
    })))
}

/// The target selection a request makes with `device` and `platform`.
fn selection(request: &Request) -> Result<Selection, Failure> {
    let platform = match request.str("platform") {
        None => None,
        Some(text) => Some(Platform::parse(text).ok_or_else(|| {
            Failure::new(
                code::INVALID_ARGUMENT,
                format!("{text:?} is not a platform; use ios or android"),
            )
        })?),
    };
    Ok(Selection {
        device: request.str("device").map(str::to_owned),
        platform,
    })
}

/// Resolves the request's target, claims it for the request's session and
/// runs `command` on its worker.
fn on_target(
    daemon: &Daemon,
    request: &Request,
    command: Command,
) -> Result<(Success, TargetInfo), Failure> {
    let mut request = request.clone();
    let (target, handle) = {
        let mut registry = daemon.registry();
        let target = registry.resolve(&request.session, &selection(&request)?)?;
        registry.claim(&target, &request.session, request.flag("force"))?;
        // The session's default beats the one the client sends.
        if let Some(default) = registry.default_timeout(&request.session) {
            let ms = u64::try_from(default.as_millis()).unwrap_or(u64::MAX);
            request.params.insert("defaultTimeout".into(), ms.into());
        }
        let handle = registry.handle(&target)?;
        (target, handle)
    };
    let options = AttachOptions {
        force: request.flag("force"),
    };
    let success = handle.run(options, move |attached| {
        command(attached, &request.session, &request)
    })?;
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
    entry.insert("state".into(), target.state.clone().into());
    entry
}

fn status(daemon: &Daemon) -> Outcome {
    let (mut targets, reports, sessions, drivers, warning) = {
        let mut registry = daemon.registry();
        let listed = registry.targets(false);
        let mut targets = Vec::with_capacity(listed.len());
        let mut reports = Vec::with_capacity(listed.len());
        for target in &listed {
            let worker = registry.worker_status(&target.id).unwrap_or_default();
            let mut entry = target_json(target);
            entry.insert("attached".into(), worker.attached.into());
            entry.insert("frameSource".into(), worker.frames.into());
            entry.insert("app".into(), worker.app.into());
            entry.insert("claimedBy".into(), registry.holder(&target.id).into());
            if let Some(error) = worker.error {
                entry.insert("error".into(), error.into());
            }
            targets.push(entry);
            reports.push(registry.worker_report(&target.id));
        }
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
        let warning = discovery_warning(registry.failures());
        (targets, reports, sessions, registry.driver_names(), warning)
    };
    // Drivers answer without the registry held: a Simulator's status asks
    // CoreSimulator.
    for (entry, report) in targets.iter_mut().zip(reports) {
        if let Some(report) = report {
            let TargetReport { lines, mut data } = report.report();
            data.insert("lines".into(), lines.into());
            entry.insert("driver".into(), Value::Object(data));
        }
    }
    let data = json!({
        "daemon": {
            "pid": std::process::id(),
            "version": daemon.version,
            "socket": daemon.paths.socket().to_string_lossy(),
            "uptimeMs": daemon.started.elapsed().as_millis() as u64,
            "idleTimeoutMs": daemon.idle.map(|idle| idle.as_millis() as u64),
            "drivers": drivers,
        },
        "targets": targets,
        "sessions": sessions,
    });
    Ok(Success::new(data).warn(warning))
}

fn discovery_warning(failures: &[String]) -> Option<String> {
    (!failures.is_empty()).then(|| {
        format!(
            "some drivers could not list their targets: {}",
            failures.join("; ")
        )
    })
}

fn devices(daemon: &Daemon, request: &Request) -> Outcome {
    let selection = selection(request)?;
    let mut registry = daemon.registry();
    let devices: Vec<Value> = registry
        .targets(true)
        .iter()
        .filter(|target| selection.platform.is_none_or(|p| target.platform == p))
        .map(|target| {
            let mut entry = target_json(target);
            entry.insert("model".into(), target.model.clone().into());
            entry.insert("os".into(), target.os.clone().into());
            if let Some(advice) = &target.advice {
                entry.insert("advice".into(), advice.clone().into());
            }
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
    // An explicit timeout bounds the whole wait; without one, the app gets
    // the waiting commands' default to show its first tree, as a slow cold
    // read of a large screen needs, and the settle after it the maximum of
    // design D8.
    let explicit = request
        .u64("timeout")?
        .map(|ms| bounded("timeout", ms))
        .transpose()?;
    let started = Instant::now();
    let first_wait = match explicit {
        Some(total) => total,
        None => timeout(request)?,
    };
    target.apps.launch(&app, &launch)?;
    let launched = Instant::now();
    target.session(session).app = Some(app.clone());
    target.set_app(Some(app.clone()));
    // A relaunched app's tree is the one a read after the launch shows: the
    // one before it was the process the relaunch stopped.
    let fresh = launch.relaunch.then_some(launched);
    let shown = first_tree(target, &app, fresh, started + first_wait)?;
    let settled = match shown {
        Some(at) => {
            let max = match explicit {
                Some(total) => (started + total).saturating_duration_since(at),
                None => mirror::DEFAULT_SETTLE_MAX,
            };
            target
                .mirror
                .settle(&mirror::SettleOptions {
                    max,
                    ..mirror::SettleOptions::default()
                })?
                .settled
        }
        None => false,
    };
    let warning = match (shown, settled) {
        (None, _) => Some(format!(
            "{app} did not show its screen within {} ms; take a snapshot to see what is in front",
            first_wait.as_millis()
        )),
        (Some(_), false) => Some(format!(
            "{app} showed its screen, which did not settle; wait --settle before you act on it"
        )),
        (Some(_), true) => None,
    };
    Ok(Success::new(json!({
        "opened": app,
        "target": target.info.id,
        "settled": settled,
        "elapsedMs": millis(started.elapsed()),
        "firstTreeMs": shown.map(|at| millis(at.saturating_duration_since(started))),
    }))
    .warn(warning))
}

/// How often `open` looks for the launched app's screen; the mirror keeps
/// the replica current in between.
const OPEN_SLICE: Duration = Duration::from_millis(10);

/// Waits until `deadline` for the replica to show `app` in front with more
/// than its root, read after `fresh` when given; returns when it did.
fn first_tree(
    target: &mut Attached,
    app: &str,
    fresh: Option<Instant>,
    deadline: Instant,
) -> Result<Option<Instant>, Failure> {
    // Only what is in front and whether it is sparse matter, not the
    // listing, so one level is enough.
    let probe = SnapshotOptions {
        interactive: true,
        max_depth: Some(1),
    };
    loop {
        let snapshot = target.mirror.snapshot(&probe)?;
        let read_after = fresh.is_none_or(|launched| {
            target
                .observer
                .last_read_started()
                .is_some_and(|started| started >= launched)
        });
        if read_after && shows(&snapshot, app) {
            return Ok(Some(Instant::now()));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        target.mirror.run_until((now + OPEN_SLICE).min(deadline))?;
    }
}

/// Whether `snapshot` shows `app` in front with more than its root.
fn shows(snapshot: &Snapshot, app: &str) -> bool {
    origin(snapshot) == app
        && !snapshot
            .verdict
            .reasons
            .iter()
            .any(|reason| matches!(reason, Degradation::SparseTree { .. }))
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
