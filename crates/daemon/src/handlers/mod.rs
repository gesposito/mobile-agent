//! What each action does. Target commands resolve their target, claim it
//! for the request's session, and run on the target's worker.

mod act;
mod aim;
mod view;

use std::time::{Duration, Instant};

use mirror::backend::Owner;
use mirror::{Degradation, Element, Role, Snapshot, SnapshotOptions, Verdict};
use protocol::snapshot::WindowType;
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

/// What `doctor` asks of the daemon: its identity, drivers, emulator
/// controller setting, the rows the drivers contribute to the compatibility
/// table and what else they add, such as the physical iPhone driver's
/// runner bundle (`iosRunner`).
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
    let mut data = Map::new();
    data.insert("pid".into(), std::process::id().into());
    data.insert("version".into(), daemon.version.clone().into());
    data.insert("drivers".into(), registry.driver_names().into());
    data.insert(
        "emulatorController".into(),
        daemon.emulator_controller.name().into(),
    );
    data.insert("compatibility".into(), json!({"rows": rows, "note": note}));
    data.extend(registry.doctor());
    Ok(Success::new(Value::Object(data)))
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
            entry.insert("attaches".into(), worker.attaches.into());
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
            "emulatorController": daemon.emulator_controller.name(),
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
        .devices()
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
    // An input session that activates on the first input, as a Simulator's
    // dtuhidd session does, starts now and activates while the app
    // launches, so the first click does not wait for it; the click reports
    // a session that cannot start.
    if let Some(input) = target.input_session.as_mut() {
        let _ = input.start();
    }
    target.apps.launch(&app, &launch)?;
    // Whatever the driver, the window in front changed: every window is
    // read again when the tree is next needed, and the settle below counts
    // the launch as a tree change, so it waits for an app that draws late.
    target.mirror.note_window_change();
    let launched = Instant::now();
    target.session(session).app = Some(app.clone());
    target.set_app(Some(app.clone()));
    // A relaunched app's tree is the one a read after the launch shows: the
    // one before it was the process the relaunch stopped.
    let fresh = launch.relaunch.then_some(launched);
    let first = first_tree(target, &app, fresh, started + first_wait)?;
    let shown = match first {
        FirstTree::Shown(at) => Some(at),
        FirstTree::Splash | FirstTree::NotShown => None,
    };
    // The first frame is no change, so the first click would report no
    // frame change if its effects came before the frame stream's first
    // frame, which an emulator delivers a while after the attach.
    let frame_deadline = match explicit {
        Some(total) => started + total.min(FIRST_FRAME_WAIT),
        None => started + FIRST_FRAME_WAIT,
    };
    let primed = first_frame(target, frame_deadline)?;
    let settled = match shown {
        Some(_) => {
            let max = match explicit {
                Some(total) => (started + total).saturating_duration_since(Instant::now()),
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
    let session_deadline = match explicit {
        Some(total) => started + total.min(SESSION_WAIT),
        None => started + SESSION_WAIT,
    };
    input_active(target, session_deadline)?;
    let mut warnings = Vec::new();
    match (&first, settled) {
        (FirstTree::NotShown, _) => warnings.push(format!(
            "{app} did not show its screen within {} ms; take a snapshot to see what is in front",
            first_wait.as_millis()
        )),
        (FirstTree::Splash, _) => warnings.push(format!(
            "{app} showed only its launch splash for {} ms; wait for one of its elements before you act",
            first_wait.as_millis()
        )),
        (FirstTree::Shown(_), false) => warnings.push(format!(
            "{app} showed its screen, which did not settle; wait --settle before you act on it"
        )),
        (FirstTree::Shown(_), true) => {}
    }
    if !primed {
        warnings.push(format!(
            "no frame came from the target's frame source within {} ms, so changes may show in the tree only",
            frame_deadline.saturating_duration_since(started).as_millis()
        ));
    }
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
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

/// How long after its start `open` waits for the target's frame stream to
/// deliver its first frame. An emulator's arrived within 500 ms of the
/// attach on a still launcher (spike S7: 22 to 146 ms after subscribing),
/// a Simulator's with the subscription.
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(2);

/// How long after its start `open` lets the target's input session
/// activate: dtuhidd's first session of a boot took 270 to 330 ms (spike
/// S3), and 957 ms under load in the Bluesky run of the React Native smoke test
/// (reports/10.4-rn-smoke.md).
const SESSION_WAIT: Duration = Duration::from_secs(2);

/// Waits until `deadline` for the target's input session, if it has one,
/// to be active, so the first input does not wait for it.
fn input_active(target: &mut Attached, deadline: Instant) -> Result<(), Failure> {
    loop {
        let Some(input) = target.input_session.as_mut() else {
            return Ok(());
        };
        if input.active() {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(());
        }
        target.mirror.run_until((now + OPEN_SLICE).min(deadline))?;
    }
}

/// Waits until `deadline` for the mirror to take in the first frame of a
/// target with a frame source; says whether it has taken one in.
fn first_frame(target: &mut Attached, deadline: Instant) -> Result<bool, Failure> {
    if !target.frames {
        return Ok(true);
    }
    loop {
        target.mirror.drain();
        if target.mirror.timestamps().last_frame.is_some() {
            return Ok(true);
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        target.mirror.run_until((now + OPEN_SLICE).min(deadline))?;
    }
}

/// What `open` found of the launched app.
enum FirstTree {
    /// Its screen showed at this instant.
    Shown(Instant),
    /// Only its launch splash showed within the budget.
    Splash,
    /// It did not show.
    NotShown,
}

/// Waits until `deadline` for the replica to show `app` in front with more
/// than its root and more than a launch splash, read after `fresh` when
/// given; says when it did.
fn first_tree(
    target: &mut Attached,
    app: &str,
    fresh: Option<Instant>,
    deadline: Instant,
) -> Result<FirstTree, Failure> {
    // Only what is in front and whether it is sparse matter, not the
    // listing, so one level is enough.
    let probe = SnapshotOptions {
        interactive: true,
        max_depth: Some(1),
    };
    let mut splash = false;
    loop {
        let snapshot = target.mirror.snapshot(&probe)?;
        let read_after = fresh.is_none_or(|launched| {
            target
                .mirror
                .timestamps()
                .last_read
                .is_some_and(|read| read.started >= launched)
        });
        if read_after && shows(&snapshot, app) {
            // A splash lists nothing interactive; only then does the full
            // listing say whether the app shows more than one.
            let listed = in_app_windows(&snapshot).next().is_some();
            if listed || !is_splash(&target.mirror.snapshot(&SnapshotOptions::full())?) {
                return Ok(FirstTree::Shown(Instant::now()));
            }
            splash = true;
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(if splash {
                FirstTree::Splash
            } else {
                FirstTree::NotShown
            });
        }
        target.mirror.run_until((now + OPEN_SLICE).min(deadline))?;
    }
}

/// The elements `snapshot` lists in the target's own application windows.
fn in_app_windows(snapshot: &Snapshot) -> impl Iterator<Item = &Element> {
    let windows: Vec<u32> = snapshot
        .windows
        .iter()
        .filter(|w| w.kind == WindowType::APPLICATION && w.owner == Owner::Target)
        .map(|w| w.id)
        .collect();
    snapshot
        .elements
        .iter()
        .filter(move |element| windows.contains(&element.window))
}

/// Whether `listing`, a full listing, shows only a launch splash in the
/// app's windows: nothing listed, as the Expo fixture's empty React Native
/// root view reads, or a single image, as Bluesky's
/// `image [id=SplashScreenLegacy]`, or a single element whose identifier or
/// class says splash.
fn is_splash(listing: &Snapshot) -> bool {
    let mut listed = in_app_windows(listing);
    match (listed.next(), listed.next()) {
        (None, _) => true,
        (Some(only), None) => {
            let says_splash = |text: &str| text.to_ascii_lowercase().contains("splash");
            only.role == Role::Image
                || only.identifier.as_deref().is_some_and(says_splash)
                || says_splash(&only.class)
        }
        _ => false,
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
        // As for a launch: the window in front changed.
        target.mirror.note_window_change();
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
