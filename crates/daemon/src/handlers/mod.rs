//! What each action does. Target commands resolve their target, claim it
//! for the request's session, and run on the target's worker.

mod act;
mod aim;
mod view;

use std::sync::Arc;
use std::time::{Duration, Instant};

use mirror::backend::Owner;
use mirror::{Degradation, Element, Role, Snapshot, SnapshotOptions, Verdict};
use protocol::snapshot::WindowType;
use serde_json::{Map, Value, json};

use crate::driver::{AttachOptions, Launch, Platform, TargetInfo, TargetReport};
use crate::registry::Selection;
use crate::server::Daemon;
use crate::wire::{Command, Failure, Param, Request, Success, arg, code, needed};
use crate::worker::{Attached, InFlight, Label, LastInput, Outcome};

/// How long waiting commands wait when neither they, their session nor
/// the client name a timeout.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest timeout a command or session may set. The CLI keeps its
/// socket open this long plus a margin for a wait that names no timeout,
/// since it cannot know the session's default.
pub(crate) const MAX_TIMEOUT: Duration = Duration::from_secs(600);

/// What a command on a target's worker knows of its request beside its
/// parameters.
pub(crate) struct Call {
    pub session: String,
    /// The timeout of a waiting command that names none: the session's,
    /// else the one the request carries.
    pub default_timeout: Param<Option<u64>>,
}

/// A command that runs on a target's worker, with its parameters.
type OnTarget<A> = fn(&mut Attached, &Call, &A) -> Outcome;

pub(crate) fn handle(daemon: &Daemon, request: &Request) -> Outcome {
    match &request.command {
        Command::Status(_) => status(daemon),
        Command::Devices(_) => devices(daemon, request),
        Command::Close(args) => close(daemon, request, args.keep),
        Command::Cancel(_) => cancel(daemon, &request.session),
        Command::SetTimeout(args) => set_timeout(daemon, &request.session, args),
        Command::Doctor(_) => doctor(daemon),
        Command::Open(args) => run(daemon, request, args, open),
        Command::Snapshot(args) => run(daemon, request, args, view::snapshot),
        Command::DiffSnapshot(args) => run(daemon, request, args, view::diff_snapshot),
        Command::Get(args) => run(daemon, request, args, view::get),
        Command::Wait(args) => run(daemon, request, args, view::wait),
        Command::Screenshot(args) => run(daemon, request, args, view::screenshot),
        Command::Click(args) => run(daemon, request, args, act::click),
        Command::Fill(args) => run(daemon, request, args, act::fill),
        Command::Type(args) => run(daemon, request, args, act::type_text),
        Command::Press(args) => run(daemon, request, args, act::press),
        Command::Swipe(args) => run(daemon, request, args, act::swipe),
        Command::Scroll(args) => run(daemon, request, args, act::scroll),
        // The server answers shutdown itself.
        Command::Shutdown(_) => Err(Failure::new(
            code::UNKNOWN_ACTION,
            "unknown action \"shutdown\"; run mobile-agent --help for the commands",
        )),
    }
}

/// Runs the command of `request`, one that runs on a target, on `target` in
/// `session`, the request's own default timeout standing for the
/// session's: the handlers' tests call the handlers through it.
#[cfg(test)]
pub(crate) fn run_on(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let call = Call {
        session: session.to_owned(),
        default_timeout: request.default_timeout.clone(),
    };
    let input = input_of(&request.command);
    noting_input(target, &call, input, |target, call| {
        match &request.command {
            Command::Close(args) => close_app(target, call, args.keep),
            Command::Open(args) => open(target, call, args),
            Command::Snapshot(args) => view::snapshot(target, call, args),
            Command::DiffSnapshot(args) => view::diff_snapshot(target, call, args),
            Command::Get(args) => view::get(target, call, args),
            Command::Wait(args) => view::wait(target, call, args),
            Command::Screenshot(args) => view::screenshot(target, call, args),
            Command::Click(args) => act::click(target, call, args),
            Command::Fill(args) => act::fill(target, call, args),
            Command::Type(args) => act::type_text(target, call, args),
            Command::Press(args) => act::press(target, call, args),
            Command::Swipe(args) => act::swipe(target, call, args),
            Command::Scroll(args) => act::scroll(target, call, args),
            other => panic!("{} runs on no target", other.action()),
        }
    })
}

/// The action of `command` when its input is one whose first effect a
/// settle right after it waits for (design D2 of action-verdicts): a click,
/// a fill, a type, a press, a swipe or a scroll.
fn input_of(command: &Command) -> Option<&'static str> {
    match command {
        Command::Click(_)
        | Command::Fill(_)
        | Command::Type(_)
        | Command::Press(_)
        | Command::Swipe(_)
        | Command::Scroll(_) => Some(command.action()),
        _ => None,
    }
}

/// Runs `command` for `call`'s session on `target` and, when `input` names
/// its action, keeps for the session when the last input it injected went
/// in, as the mirror timed it, also when the command failed after it went
/// in (design D2 of action-verdicts).
fn noting_input(
    target: &mut Attached,
    call: &Call,
    input: Option<&'static str>,
    command: impl FnOnce(&mut Attached, &Call) -> Outcome,
) -> Outcome {
    let before = target.mirror.timestamps().last_action;
    let outcome = command(target, call);
    let last = target.mirror.timestamps().last_action;
    if let (Some(action), Some(at)) = (input, last)
        && last != before
    {
        target.session(&call.session).input = Some(LastInput { action, at });
    }
    outcome
}

/// Runs `command` with `args` on the request's target.
fn run<A: Clone + Send + 'static>(
    daemon: &Daemon,
    request: &Request,
    args: &A,
    command: OnTarget<A>,
) -> Outcome {
    let args = args.clone();
    let input = input_of(&request.command);
    on_target(daemon, request, move |attached, call| {
        noting_input(attached, call, input, |attached, call| {
            command(attached, call, &args)
        })
    })
    .map(|(success, _)| success)
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
/// the call carries (the session's, or else the client's), else
/// [`DEFAULT_TIMEOUT`].
pub(crate) fn timeout(own: &Param<Option<u64>>, call: &Call) -> Result<Duration, Failure> {
    if let Some(ms) = arg("timeout", own)? {
        return bounded("timeout", ms);
    }
    match arg("defaultTimeout", &call.default_timeout)? {
        Some(ms) => bounded("defaultTimeout", ms),
        None => Ok(DEFAULT_TIMEOUT),
    }
}

/// How long a settle wait may last: the command's own `timeout`, else the
/// maximum of design D8. The session's default does not apply.
pub(crate) fn settle_max(own: &Param<Option<u64>>) -> Result<Duration, Failure> {
    match arg("timeout", own)? {
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
fn set_timeout(daemon: &Daemon, session: &str, args: &::wire::SetTimeout) -> Outcome {
    let ms = arg("ms", &args.ms)?.ok_or_else(|| {
        Failure::new(
            code::INVALID_ARGUMENT,
            "set timeout needs a number of milliseconds",
        )
    })?;
    let timeout = bounded("the default timeout", ms)?;
    daemon.registry().set_default_timeout(session, timeout);
    Ok(Success::new(json!({
        "session": session,
        "defaultTimeoutMs": ms,
    })))
}

/// The target selection a request makes with `device` and `platform`.
fn selection(request: &Request) -> Result<Selection, Failure> {
    let platform = match request.platform.as_deref() {
        None => None,
        Some(text) => Some(Platform::parse(text).ok_or_else(|| {
            Failure::new(
                code::INVALID_ARGUMENT,
                format!("{text:?} is not a platform; use ios or android"),
            )
        })?),
    };
    Ok(Selection {
        device: request.device.clone(),
        platform,
    })
}

/// Resolves the request's target, claims it for the request's session and
/// runs `command` on its worker.
fn on_target<F>(
    daemon: &Daemon,
    request: &Request,
    command: F,
) -> Result<(Success, TargetInfo), Failure>
where
    F: FnOnce(&mut Attached, &Call) -> Outcome + Send + 'static,
{
    let (target, handle, call) = {
        let mut registry = daemon.registry();
        let target = registry.resolve(&request.session, &selection(request)?)?;
        registry.claim(&target, &request.session, request.force)?;
        // The session's default beats the one the client sends.
        let default_timeout = match registry.default_timeout(&request.session) {
            Some(default) => {
                Param::Valid(Some(u64::try_from(default.as_millis()).unwrap_or(u64::MAX)))
            }
            None => request.default_timeout.clone(),
        };
        let handle = registry.handle(&target)?;
        let call = Call {
            session: request.session.clone(),
            default_timeout,
        };
        (target, handle, call)
    };
    let options = AttachOptions {
        force: request.force,
    };
    let action = request.command.action();
    let label = Label {
        session: request.session.clone(),
        action,
    };
    let success = handle
        .run(options, label, move |attached| command(attached, &call))
        .map_err(|failure| ended_by_cancel(failure, action))?;
    Ok((success, target))
}

/// A failure the session's `cancel` caused, its message naming `action`,
/// the command it ended; any other failure as it is.
fn ended_by_cancel(failure: Failure, action: &str) -> Failure {
    if failure.code != code::CANCELLED {
        return failure;
    }
    Failure::new(
        code::CANCELLED,
        format!(
            "{action} was cancelled: its session's cancel ended it; take a snapshot to see the screen as it left it"
        ),
    )
}

/// How long `cancel` waits for the commands it ends to be over: a wait
/// sees the cancel within the mirror's cancel slice, and a device call in
/// flight runs to its end first.
const CANCEL_WAIT: Duration = Duration::from_secs(5);

/// Ends the commands of `session` in flight on the targets it holds and
/// waits for them to be over, up to [`CANCEL_WAIT`] (design D7 of
/// mcp-server). The reply names the first by target id; one that was not
/// over by then, held in a device call, is named in a warning.
fn cancel(daemon: &Daemon, session: &str) -> Outcome {
    let held: Vec<(String, Arc<InFlight>)> = {
        let registry = daemon.registry();
        registry
            .held_by(session)
            .into_iter()
            .filter_map(|target| registry.in_flight(&target).map(|flight| (target, flight)))
            .collect()
    };
    let raised: Vec<(String, Arc<InFlight>, u64, &'static str)> = held
        .into_iter()
        .filter_map(|(target, flight)| {
            let (number, action) = flight.raise(session)?;
            Some((target, flight, number, action))
        })
        .collect();
    let deadline = Instant::now() + CANCEL_WAIT;
    let unfinished: Vec<String> = raised
        .iter()
        .filter(|(_, flight, number, _)| !flight.wait_over(*number, deadline))
        .map(|(target, _, _, action)| format!("{action} on {target}"))
        .collect();
    let Some((target, _, _, action)) = raised.first() else {
        return Ok(Success::new(json!({
            "cancelled": false,
            "action": null,
            "target": null,
        })));
    };
    let warning = (!unfinished.is_empty()).then(|| {
        format!(
            "{} had not ended within {} s: a device call in flight runs to its end, and the command fails with cancelled once it returns",
            unfinished.join(", "),
            CANCEL_WAIT.as_secs()
        )
    });
    Ok(Success::new(json!({
        "cancelled": true,
        "action": action,
        "target": target,
    }))
    .warn(warning))
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

fn open(target: &mut Attached, call: &Call, args: &::wire::Open) -> Outcome {
    let session = call.session.as_str();
    let app = needed("open", "app", &args.app)?.to_owned();
    let mut env = Vec::new();
    for pair in arg("env", &args.env)? {
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            Failure::new(
                code::INVALID_ARGUMENT,
                format!("--env takes KEY=VALUE, not {pair:?}"),
            )
        })?;
        env.push((key.to_owned(), value.to_owned()));
    }
    let launch = Launch {
        relaunch: args.relaunch,
        env,
    };
    // An explicit timeout bounds the whole wait; without one, the app gets
    // the waiting commands' default to show its first tree, as a slow cold
    // read of a large screen needs, and the settle after it the maximum of
    // design D8.
    let explicit = arg("timeout", &args.timeout)?
        .map(|ms| bounded("timeout", ms))
        .transpose()?;
    let started = Instant::now();
    let first_wait = match explicit {
        Some(total) => total,
        None => timeout(&args.timeout, call)?,
    };
    // An input session that activates on the first input, as a Simulator's
    // dtuhidd session does, starts now and activates while the app
    // launches, so the first click does not wait for it; the click reports
    // a session that cannot start.
    if let Some(input) = target.input_session.as_mut() {
        let _ = input.start();
    }
    // Whatever the driver, the launch changes the window in front: every
    // window is read again when the tree is next needed, and the settle
    // below counts the launch as a tree change, so it waits for an app that
    // draws late. The change is noted before the launch, so that what the
    // target shows from the launch on is the launch's own effect.
    target.mirror.note_window_change();
    let launching = Instant::now();
    target.apps.launch(&app, &launch)?;
    let launched = Instant::now();
    target.session(session).app = Some(app.clone());
    target.set_app(Some(app.clone()));
    // A relaunched app's tree is the one a read after the launch shows: the
    // one before it was the process the relaunch stopped.
    let fresh = launch.relaunch.then_some(launched);
    let first = first_tree(target, &app, fresh, started + first_wait)?;
    let shown = match first {
        FirstTree::Shown(at) => Some(at),
        FirstTree::Splash | FirstTree::Alert(_) | FirstTree::NotShown => None,
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
        Some(_) => settle_launch(target, launching, (started, explicit))?,
        None => false,
    };
    // The app whose screen showed is in front once that screen settled,
    // unless a dialog of system UI stands in front of it (design D4 of
    // action-verdicts); an app that showed no screen yet, the launcher still
    // in front of a slow one, is named in a warning below.
    let showed = matches!(first, FirstTree::Shown(_)) && target.mirror.system_dialog().is_none();
    let (first, shown, settled) = if showed {
        in_front(
            target,
            (&app, &launch),
            (first, shown, settled),
            (started, explicit),
        )?
    } else {
        (first, shown, settled)
    };
    let session_deadline = match explicit {
        Some(total) => started + total.min(SESSION_WAIT),
        None => started + SESSION_WAIT,
    };
    input_active(target, session_deadline)?;
    // The dialog in front of the app: the one the wait for its screen met,
    // or one that came in front while the screen settled, as a permission
    // alert does once the app asks.
    let dialog = match &first {
        FirstTree::Alert(dialog) => Some(dialog.clone()),
        FirstTree::Shown(_) => target.mirror.system_dialog(),
        FirstTree::Splash | FirstTree::NotShown => None,
    };
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
        (FirstTree::Shown(_), false) if dialog.is_none() => warnings.push(format!(
            "{app} showed its screen, which did not settle; wait --settle before you act on it"
        )),
        (FirstTree::Alert(_) | FirstTree::Shown(_), _) => {}
    }
    if let Some(dialog) = &dialog {
        warnings.push(format!(
            "{dialog} stands in front of {app}; answer it before you act on the app: a snapshot lists its buttons"
        ));
    }
    if !primed {
        warnings.push(format!(
            "no frame came from the target's frame source within {} ms, so changes may show in the tree only",
            frame_deadline.saturating_duration_since(started).as_millis()
        ));
    }
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    let mut data = json!({
        "opened": app,
        "target": target.info.id,
        "settled": settled,
        "elapsedMs": millis(started.elapsed()),
        "firstTreeMs": shown.map(|at| millis(at.saturating_duration_since(started))),
    });
    if let Some(dialog) = dialog {
        data["alert"] = dialog.into();
    }
    Ok(Success::new(data).warn(warning))
}

/// Settles the screen after the launch made at `launching` as `open` does,
/// waiting for the launch's first effect as a settle after an input waits
/// for one (design D2 of action-verdicts): a replica that showed the app
/// before the launch, as when the app was in front, shows its screen at
/// once, before the target has shown what the launch did. The explicit
/// timeout from `started` bounds the wait when one is given, else the
/// settle's maximum does; says whether the screen settled.
fn settle_launch(
    target: &mut Attached,
    launching: Instant,
    (started, explicit): (Instant, Option<Duration>),
) -> Result<bool, Failure> {
    let max = match explicit {
        Some(total) => (started + total).saturating_duration_since(Instant::now()),
        None => mirror::DEFAULT_SETTLE_MAX,
    };
    let options = mirror::SettleOptions {
        max,
        ..mirror::SettleOptions::default()
    };
    let by = explicit.map(|total| started + total);
    Ok(view::settle_after(target, Some(launching), (&options, by))?
        .0
        .settled)
}

/// How long after the launch that brings a running app to the front `open`
/// waits for the app's screen: on this session's Simulator a second launch
/// of a running app showed its screen 166 to 177 ms after it
/// (action-verdicts, task 2.4).
const ACTIVATE_WAIT: Duration = Duration::from_secs(2);

/// The app in front as the replica has it now, as a snapshot's `origin`
/// names it.
fn front_of(target: &mut Attached) -> Result<String, Failure> {
    let probe = SnapshotOptions {
        interactive: true,
        max_depth: Some(1),
    };
    Ok(origin(&target.mirror.snapshot(&probe)?))
}

/// Leaves `app`, which `launch` launched and whose screen showed, in front
/// once that screen settled (design D4 of action-verdicts), as the
/// replica's foreground owner tells:
/// on this session's Simulator `open` right after `press home` launched
/// Settings during the home screen's go-home animation, its screen showed,
/// and the home screen took the front back as the animation ended, three
/// times in three, while a second launch of the running app brought it to
/// the front three times in three. So an app another holds the front from
/// is launched again, which brings a running app to the front on every
/// driver, and the wait for its screen, at most [`ACTIVATE_WAIT`] within the
/// explicit timeout from `started` when one is given, and the settle after
/// it run again; an app still not in front fails with `not_in_front`,
/// naming it and what is in front. Gives the first tree, when the app's
/// screen showed and whether it settled, those after the second launch
/// when one was made.
fn in_front(
    target: &mut Attached,
    (app, launch): (&str, &Launch),
    (first, shown, settled): (FirstTree, Option<Instant>, bool),
    (started, explicit): (Instant, Option<Duration>),
) -> Result<(FirstTree, Option<Instant>, bool), Failure> {
    if front_of(target)? == app {
        return Ok((first, shown, settled));
    }
    target.mirror.note_window_change();
    let launching = Instant::now();
    let again = Launch {
        relaunch: false,
        env: launch.env.clone(),
    };
    target.apps.launch(app, &again)?;
    let deadline = match explicit {
        Some(total) => (started + total).min(launching + ACTIVATE_WAIT),
        None => launching + ACTIVATE_WAIT,
    };
    let first = first_tree(target, app, None, deadline)?;
    let shown = match first {
        FirstTree::Shown(at) => Some(at),
        FirstTree::Splash | FirstTree::Alert(_) | FirstTree::NotShown => None,
    };
    let settled = match shown {
        Some(_) => settle_launch(target, launching, (started, explicit))?,
        None => false,
    };
    let front = front_of(target)?;
    if front == app || matches!(first, FirstTree::Alert(_)) {
        return Ok((first, shown, settled));
    }
    let front = if front.is_empty() {
        "no app".to_owned()
    } else {
        front
    };
    Err(Failure::new(
        code::NOT_IN_FRONT,
        format!(
            "{app} did not come to the front: {front} stays in front of it after its launch and a second one; take a snapshot to see what is on the screen"
        ),
    ))
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
    /// A dialog of system UI stood in front of it, named so (real-app-fixes,
    /// design D5).
    Alert(String),
    /// It did not show.
    NotShown,
}

/// Waits until `deadline` for the replica to show `app` in front with more
/// than its root and more than a launch splash, read after `fresh` when
/// given; says when it did. A dialog of system UI in front of the screen,
/// as a permission alert the app's launch brings up, ends the wait at once
/// (`Mirror::system_dialog`): on the demo pass `open` waited out the call's
/// bound behind the location alert in front of the app, while the next
/// action was the model's, on the alert.
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
        if read_after && let Some(dialog) = target.mirror.system_dialog() {
            return Ok(FirstTree::Alert(dialog));
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

/// Closes the session's app and releases its target; with `keep`, releases
/// the target and leaves the app running (design D9 of mcp-server).
fn close(daemon: &Daemon, request: &Request, keep: bool) -> Outcome {
    let (success, target) = on_target(daemon, request, move |attached, call| {
        close_app(attached, call, keep)
    })?;
    daemon.registry().release(&target.id, &request.session);
    Ok(success)
}

/// Stops the app the session's `open` launched, or with `keep` forgets it
/// and leaves it running, so that the session's next `close` stops nothing
/// and `status` still names the app on the target.
fn close_app(target: &mut Attached, call: &Call, keep: bool) -> Outcome {
    let app = target.session(&call.session).app.take();
    if let Some(app) = app.as_deref().filter(|_| !keep) {
        target.apps.terminate(app)?;
        // As for a launch: the window in front changed.
        target.mirror.note_window_change();
        target.set_app(None);
    }
    let mut data = json!({
        "closed": true,
        "app": app,
        "target": target.info.id,
    });
    if keep {
        data["kept"] = true.into();
    }
    Ok(Success::new(data))
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
