//! The adapter for `mobile-agent`, the tool under test, driven through its
//! command line as an agent drives it: one process per command in `--json`
//! mode, each reply in agent-browser's envelope (`{success, data, error,
//! code?, warning?}`), a failed reply an error carrying its code and message.
//! Every command names the bench's session and the target, then the command,
//! its flags, `--` and its operands:
//! `mobile-agent --json --session mobile-agent-bench --device <id> click
//! --timeout 10000 -- id=row-1`.
//!
//! Primitives are measured on the screens [`crate::plan::scenario`] names.
//! Process start, cold attach, snapshots, text entry and the screenshot are
//! timed by wall clock around the command's process; tap dispatch, tap to
//! frame change, tap to tree delta and settle detection are read from the
//! click's and the settle's replies, spans only the daemon observes. A span
//! the target cannot produce is reported as not measured, with the reason:
//! tap to frame change on a target that gives no frames, which `status` says.
//! Snapshots count the refs in their reply as their nodes. The daemon keeps
//! its agent, and on Android the device's one UI automation role with it,
//! until it exits, so the adapter's release stops the daemon, whose
//! `shutdown` answers once it has let go of its targets.

pub mod listing;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::adapter::{Adapter, AdapterError, Clock, Detection, Role, Sample, Scenario};
use crate::baseline::{
    exit_failure, find_executable, is_executable, maestro, search_path, version_from,
};
use crate::end_state::{Element, EndState};
use crate::fixture::Screen;
use crate::flow::{Direction, Flow, Selector, Step};
use crate::harness::Subject;
use crate::primitive::{Primitive, TEXT_ENTRY};
use crate::process;
use crate::provenance::Probe;
use crate::target::{Battery, Controller, Target, TargetClass};

/// The tool's name in reports.
pub const TOOL: &str = "mobile-agent";

/// The session every command runs in.
pub const SESSION: &str = "mobile-agent-bench";

/// How long a selector, an element wait or a text wait may wait: 10 s, as
/// agent-device's `wait` steps do.
const WAIT_MS: u64 = 10_000;
const WAIT: &str = "10000";

/// The AMOUNT a scroll-to passes to `scroll --until`: the length of each of
/// its strokes, in points, less than a phone screen's height.
pub const SCROLL_STEP: u32 = 600;

/// How long a scroll-to's `scroll --until` may scroll, in milliseconds: as
/// long as the Maestro flow's `scrollUntilVisible` may
/// ([`maestro::SCROLL_TIMEOUT_MS`]), within the 120 s a command gets by
/// default.
const SCROLL_TIMEOUT_MS: u64 = maestro::SCROLL_TIMEOUT_MS;

/// The element the tap primitives and settle detection tap: the List
/// screen's first row, which opens its detail.
pub const TAP: &str = "row-1";

/// The element that brings the list back after a tap on [`TAP`].
pub const TAP_BACK: &str = "detail-back";

/// The field text entry fills, on the Login screen.
pub const TEXT_FIELD: &str = "login-username";

/// How many times cold attach sends `shutdown` before it gives up on the
/// daemon stopping.
const SHUTDOWNS: u32 = 100;

/// Why a tap on an Android device has no frame change to time.
const NO_FRAME_STREAM: &str = "Android devices have no frame stream, only screenshots on demand \
    (design D7), so a tap on one has no frame change to time";

/// Where the CLI adapter finds `mobile-agent`, what environment its commands
/// get, and where it writes files.
#[derive(Clone, Debug)]
pub struct CliConfig {
    /// The executable: a path, or a name looked up in `search_path`.
    pub program: PathBuf,
    /// Directories searched for a program given by name: `PATH` by default.
    pub search_path: Vec<PathBuf>,
    /// Changes to the harness's environment for every command: `Some` sets a
    /// variable, `None` removes it.
    pub env: Vec<(OsString, Option<OsString>)>,
    /// The working directory of every command; `None` keeps the harness's.
    pub cwd: Option<PathBuf>,
    /// Where screenshots are written.
    pub work_dir: PathBuf,
    /// Deadline for one command's process.
    pub command_timeout: Duration,
}

impl CliConfig {
    /// `mobile-agent` from `PATH` with the harness's environment, writing
    /// under `work_dir`, 120 s per command.
    pub fn new(work_dir: impl Into<PathBuf>) -> Self {
        CliConfig {
            program: PathBuf::from(TOOL),
            search_path: search_path(),
            env: Vec::new(),
            cwd: None,
            work_dir: work_dir.into(),
            command_timeout: Duration::from_secs(120),
        }
    }
}

/// The `mobile-agent` to drive when none is named: the one beside the running
/// executable, where a build of the workspace puts it beside
/// `mobile-agent-bench`, else `mobile-agent`, looked up on `PATH`.
pub fn default_program() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(TOOL))
        .filter(|path| is_executable(path))
        .unwrap_or_else(|| PathBuf::from(TOOL))
}

/// `mobile-agent` driven through its CLI.
#[derive(Clone, Debug)]
pub struct CliAdapter {
    config: CliConfig,
    /// The executable, once detected.
    path: Option<PathBuf>,
    /// What `status` last said about each attached target, by target id.
    attached: HashMap<String, Attached>,
}

/// What `status` says about a target the daemon has attached: the target
/// entry's `frameSource`, and from its driver's report, whether an Android
/// emulator's controller answered and why the target gives no frames, and on
/// a physical iPhone its runner, its input and frame paths and its battery.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Attached {
    frame_source: Option<bool>,
    controller: Option<Controller>,
    /// The driver's `warning` and `frameError`, such as the note that the
    /// emulator's control endpoint did not answer and how to enable it.
    why_no_frames: Option<String>,
    /// On a physical iPhone, the runner's `agent`, as its hello named it.
    runner: Option<String>,
    /// On a physical iPhone, the driver's `input`.
    input_path: Option<String>,
    /// On a physical iPhone, the driver's `frames`.
    frame_path: Option<String>,
    /// On a physical iPhone, the driver's `battery`.
    battery: Option<Battery>,
}

/// A command's reply and when its process ran.
#[derive(Debug)]
struct Reply {
    success: bool,
    data: Value,
    error: Option<String>,
    code: Option<String>,
    warning: Option<String>,
    started: Instant,
    elapsed: Duration,
}

impl Reply {
    fn ended(&self) -> Instant {
        self.started + self.elapsed
    }

    /// The note of the fill this reply answers (see [`filled`]).
    fn filled(&self) -> Option<String> {
        filled(&self.data, self.warning.as_deref())
    }
}

/// Agent-browser's envelope, as `--json` prints it on one line.
#[derive(Debug, Deserialize, PartialEq)]
struct Envelope {
    success: bool,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    warning: Option<String>,
}

impl CliAdapter {
    pub fn new(config: CliConfig) -> Self {
        CliAdapter {
            config,
            path: None,
            attached: HashMap::new(),
        }
    }

    /// The targets `mobile-agent devices` lists, with the model and OS it
    /// reports for each. The first command starts a daemon if none serves.
    pub fn devices(&self) -> Result<Vec<Target>, AdapterError> {
        let reply = self.send(None, &["devices"])?;
        if !reply.success {
            return Err(failure(&["devices"], &reply));
        }
        read_devices(&reply.data).map_err(AdapterError::new)
    }

    fn executable(&self) -> Result<&Path, AdapterError> {
        self.path
            .as_deref()
            .ok_or_else(|| AdapterError::new(format!("{TOOL} was not detected")))
    }

    /// The executable at `path` with the configured environment and working
    /// directory.
    fn command(&self, path: &Path) -> Command {
        let mut command = Command::new(path);
        for (key, value) in &self.config.env {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        if let Some(dir) = &self.config.cwd {
            command.current_dir(dir);
        }
        command
    }

    /// Runs one command, on `target` when given, and returns its reply,
    /// successful or not. Fails when the process cannot run or prints no reply.
    fn send(&self, target: Option<&Target>, command: &[&str]) -> Result<Reply, AdapterError> {
        let mut process = self.command(self.executable()?);
        process.args(command_line(target, command));
        let finished =
            process::run(&mut process, self.config.command_timeout).map_err(AdapterError::new)?;
        let envelope = envelope(&finished.stdout).ok_or_else(|| {
            AdapterError::new(format!(
                "{TOOL} {} printed no JSON reply: {}",
                command.join(" "),
                exit_failure(&finished)
            ))
        })?;
        Ok(Reply {
            success: envelope.success,
            data: envelope.data,
            error: envelope.error,
            code: envelope.code,
            warning: envelope.warning,
            started: finished.started,
            elapsed: finished.elapsed,
        })
    }

    /// Runs one command on `target` and returns its successful reply.
    fn call(&self, target: &Target, command: &[&str]) -> Result<Reply, AdapterError> {
        let reply = self.send(Some(target), command)?;
        if reply.success {
            Ok(reply)
        } else {
            Err(failure(command, &reply))
        }
    }

    /// One `mobile-agent --version` process, its output discarded, as
    /// hyperfine times a command.
    fn process_start(&self) -> Result<Sample, AdapterError> {
        let mut command = self.command(self.executable()?);
        command.arg("--version");
        let (status, elapsed) =
            process::time(&mut command, self.config.command_timeout).map_err(AdapterError::new)?;
        if !status.success() {
            return Err(AdapterError::new(format!(
                "{TOOL} --version exited with {status}"
            )));
        }
        Ok(wall(elapsed, None, None))
    }

    /// Stops the daemon, then times the `open` of `app` that starts another,
    /// attaches the target and brings the app up.
    fn cold_attach(&self, target: &Target, app: &str) -> Result<Sample, AdapterError> {
        self.stop_daemon()?;
        let reply = self.call(target, &["open", "--", app])?;
        let detail = match (
            reply.data["settled"].as_bool(),
            reply.data["elapsedMs"].as_f64(),
        ) {
            (Some(true), Some(ms)) => format!("open settled in {ms} ms"),
            (Some(false), Some(ms)) => format!("open did not settle in {ms} ms"),
            _ => "open reported no settle".to_owned(),
        };
        Ok(wall(reply.elapsed, None, Some(detail)))
    }

    /// Sends `shutdown` until it finds no daemon to stop, so the next command
    /// starts a new one. The daemon answers only once it has stopped its
    /// targets, an Android target's agent with them, and removed its socket
    /// (task 8.16), so the next command never meets them. Returns whether it
    /// stopped one.
    fn stop_daemon(&self) -> Result<bool, AdapterError> {
        for sent in 0..SHUTDOWNS {
            let reply = self.send(None, &["shutdown"])?;
            if !reply.success {
                return Err(failure(&["shutdown"], &reply));
            }
            if reply.data["stopped"] != true {
                return Ok(sent > 0);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(AdapterError::new(format!(
            "a daemon still answered after {SHUTDOWNS} shutdowns"
        )))
    }

    /// Reads `status` and keeps what it says about `target` when the daemon
    /// has attached it. A `status` that fails or does not say leaves it
    /// unknown.
    fn observe(&mut self, target: &Target) {
        let Some(reply) = self.send(None, &["status"]).ok().filter(|r| r.success) else {
            return;
        };
        if let Some(attached) = read_attached(&reply.data, target) {
            self.keep(target, attached);
        }
    }

    /// Keeps what `status` said about `target`, with a physical iPhone's
    /// battery as `status` first gave it in the run.
    fn keep(&mut self, target: &Target, mut attached: Attached) {
        if let Some(first) = self.attached.get(&target.id).and_then(|kept| kept.battery) {
            attached.battery = Some(first);
        }
        self.attached.insert(target.id.clone(), attached);
    }

    /// Relaunches `app` on `screen`, then waits until the screen's marker is
    /// visible and the screen has settled.
    fn open_screen(&self, target: &Target, app: &str, screen: Screen) -> Result<(), AdapterError> {
        self.relaunch(target, app, screen)?;
        self.call(
            target,
            &["wait", "--timeout", WAIT, "--", &id(marker(screen))],
        )?;
        self.call(target, &["wait", "--settle"])?;
        Ok(())
    }

    /// Relaunches `app` on `screen`, through the launch option the fixtures
    /// of `target`'s platform read.
    fn relaunch(&self, target: &Target, app: &str, screen: Screen) -> Result<(), AdapterError> {
        let tab = screen.launch_option(target.class.platform());
        self.call(target, &["open", "--relaunch", "--env", &tab, "--", app])?;
        Ok(())
    }

    /// One `snapshot -i`, which must list `screen`'s marker.
    fn snapshot(&self, target: &Target, screen: Screen) -> Result<Reply, AdapterError> {
        let reply = self.call(target, &["snapshot", "-i"])?;
        if lists(&reply, marker(screen)) {
            Ok(reply)
        } else {
            Err(AdapterError::new(format!(
                "the snapshot does not list {}: the {screen} screen is not showing",
                marker(screen)
            )))
        }
    }

    /// Relaunches `app` on `screen`, then times `snapshot -i` from the first
    /// one's start until one lists the screen's marker.
    fn first_snapshot(
        &self,
        target: &Target,
        app: &str,
        screen: Screen,
    ) -> Result<Sample, AdapterError> {
        self.relaunch(target, app, screen)?;
        let mut reply = self.call(target, &["snapshot", "-i"])?;
        let started = reply.started;
        while !lists(&reply, marker(screen)) {
            if started.elapsed() >= Duration::from_millis(WAIT_MS) {
                return Err(AdapterError::new(format!(
                    "no snapshot listed {} within {WAIT} ms of the relaunch",
                    marker(screen)
                )));
            }
            reply = self.call(target, &["snapshot", "-i"])?;
        }
        Ok(wall(reply.ended() - started, Some(refs(&reply)?), None))
    }

    /// Taps [`TAP`] and reads `primitive`'s span from the click's reply, then
    /// brings the list back.
    fn tap(&self, target: &Target, primitive: Primitive) -> Result<Sample, AdapterError> {
        let reply = self.call(target, &["click", "--timeout", WAIT, "--", &id(TAP)])?;
        self.restore(target)?;
        let field = match primitive {
            Primitive::TapDispatch => "dispatchMs",
            Primitive::TapToFrameChange => "frameChangeMs",
            _ => "treeDeltaMs",
        };
        if let Some(value) = reply.data.get(field) {
            return Ok(tool(millis(value, field)?, None));
        }
        let watched = reply.data.get("watchedMs").and_then(Value::as_f64);
        Err(match (primitive, watched) {
            (Primitive::TapDispatch, _) => {
                AdapterError::new(format!("the click on {TAP} reported no dispatchMs"))
            }
            (_, None) => AdapterError::not_measured(format!(
                "this {TOOL}'s click does not watch a tap's effects: its reply has no \
                 watchedMs and so no {field}; the CLI reports tap spans from task 8.9 on"
            )),
            (Primitive::TapToFrameChange, Some(watched)) => AdapterError::new(format!(
                "the click on {TAP} reported no frameChangeMs: no frame changed within its \
                 {watched} ms watch"
            )),
            (_, Some(watched)) => AdapterError::new(format!(
                "the click on {TAP} reported no treeDeltaMs: the tree did not change within \
                 its {watched} ms watch"
            )),
        })
    }

    /// Brings the list back after a tap on [`TAP`] and lets the screen settle,
    /// so the next tap starts from a quiet list and its first frame change is
    /// its own.
    fn restore(&self, target: &Target) -> Result<(), AdapterError> {
        self.call(target, &["click", "--timeout", WAIT, "--", &id(TAP_BACK)])?;
        self.call(target, &["wait", "--settle"])?;
        Ok(())
    }

    /// Times the `fill` of [`TEXT_ENTRY`] into [`TEXT_FIELD`], which is empty
    /// and settled. After the span it waits until the field shows the string,
    /// which a target may show late, checks that the field holds exactly it,
    /// then empties the field and lets the screen settle for the next fill.
    /// The sample's detail holds the timed fill's note, then the emptying
    /// fill's after `cleared: ` (see [`filled`]).
    fn text_entry(&self, target: &Target) -> Result<Sample, AdapterError> {
        let field = id(TEXT_FIELD);
        let reply = self.call(
            target,
            &["fill", "--timeout", WAIT, "--", &field, TEXT_ENTRY],
        )?;
        let shown = format!("--text={TEXT_ENTRY}");
        self.call(target, &["wait", "--timeout", WAIT, &shown])?;
        let check = self.call(target, &["get", "value", "--timeout", WAIT, "--", &field])?;
        if check.data["value"] != TEXT_ENTRY {
            return Err(AdapterError::new(format!(
                "{TEXT_FIELD} holds {} after the fill, not the fixed string",
                check.data["value"]
            )));
        }
        let cleared = self.call(target, &["fill", "--timeout", WAIT, "--", &field, ""])?;
        self.call(target, &["wait", "--settle"])?;
        let notes: Vec<String> = [
            reply.filled(),
            cleared.filled().map(|note| format!("cleared: {note}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        Ok(wall(
            reply.elapsed,
            None,
            (!notes.is_empty()).then(|| notes.join("; ")),
        ))
    }

    /// Times one `screenshot` into the work directory.
    fn screenshot(&self, target: &Target) -> Result<Sample, AdapterError> {
        let dir = &self.config.work_dir;
        fs::create_dir_all(dir)
            .map_err(|e| AdapterError::new(format!("cannot create {}: {e}", dir.display())))?;
        let file =
            std::path::absolute(dir.join(format!("screenshot-{}.jpg", file_safe(&target.id))))
                .map_err(|e| {
                    AdapterError::new(format!("no absolute path for the screenshot: {e}"))
                })?;
        let reply = self.call(target, &["screenshot", "--", &file.to_string_lossy()])?;
        let size = match (reply.data["width"].as_u64(), reply.data["height"].as_u64()) {
            (Some(width), Some(height)) => Some(format!("{width}x{height}")),
            _ => None,
        };
        Ok(wall(reply.elapsed, None, size))
    }

    /// Taps [`TAP`], then reads how long `wait --settle` took to find the
    /// screen quiet, and brings the list back.
    fn settle(&self, target: &Target) -> Result<Sample, AdapterError> {
        self.call(target, &["click", "--timeout", WAIT, "--", &id(TAP)])?;
        let reply = self.call(target, &["wait", "--settle"])?;
        self.restore(target)?;
        if reply.data["settled"] != true {
            return Err(AdapterError::new(format!(
                "the screen did not settle: {} kept changing",
                reply.data["stillChanging"]
            )));
        }
        let ended = reply.data["endedBy"]
            .as_str()
            .map(|by| format!("ended by {by}"));
        Ok(tool(millis(&reply.data["elapsedMs"], "elapsedMs")?, ended))
    }

    /// Runs `flow`'s steps, each as its command; returns the wall time from
    /// the first command's start to the last one's end, and the notes of its
    /// steps in their order: each fill's, after `fill <selector>: ` (see
    /// [`filled`]), and each scroll-to's (see [`CliAdapter::scroll_to`]).
    fn run_steps(
        &self,
        target: &Target,
        flow: &Flow,
    ) -> Result<(Duration, Vec<String>), AdapterError> {
        let app = flow
            .fixture
            .app_id(target.class.platform())
            .ok_or_else(|| {
                AdapterError::new(format!(
                    "the {} fixture does not run on {}",
                    flow.fixture, target.class
                ))
            })?;
        let started = Instant::now();
        let mut notes = Vec::new();
        for step in &flow.steps {
            match step {
                Step::Launch => {
                    self.call(target, &["open", "--relaunch", "--", app])?;
                }
                Step::Tap(element) => {
                    self.call(
                        target,
                        &["click", "--timeout", WAIT, "--", &selector(element)],
                    )?;
                }
                Step::Fill {
                    target: element,
                    text,
                } => {
                    let field = selector(element);
                    let reply =
                        self.call(target, &["fill", "--timeout", WAIT, "--", &field, text])?;
                    if let Some(note) = reply.filled() {
                        notes.push(format!("fill {field}: {note}"));
                    }
                }
                Step::ScrollTo {
                    target: element,
                    direction,
                } => notes.push(self.scroll_to(target, element, *direction)?),
                Step::AssertVisible(element) => {
                    self.call(
                        target,
                        &["wait", "--timeout", WAIT, "--", &selector(element)],
                    )?;
                }
                Step::AssertText { id, text } => self.assert_text(target, id, text)?,
            }
        }
        Ok((started.elapsed(), notes))
    }

    /// Scrolls `direction` until `element` is on screen as an agent does, with
    /// one `scroll --until`: the daemon strokes [`SCROLL_STEP`] points at a
    /// time, lets the screen settle after each stroke and stops once the
    /// element is on screen by its own rule. Returns the scroll's note (see
    /// [`scrolled`]).
    fn scroll_to(
        &self,
        target: &Target,
        element: &Selector,
        direction: Direction,
    ) -> Result<String, AdapterError> {
        let until = selector(element);
        let direction = match direction {
            Direction::Down => "down",
            Direction::Up => "up",
        };
        let timeout = SCROLL_TIMEOUT_MS.to_string();
        let step = SCROLL_STEP.to_string();
        let reply = self.call(
            target,
            &[
                "scroll",
                "--timeout",
                &timeout,
                "--until",
                &until,
                "--",
                direction,
                &step,
            ],
        )?;
        Ok(scrolled(&reply.data))
    }

    /// Waits until an element shows `text`, then checks that the element
    /// `identifier` shows exactly it.
    fn assert_text(
        &self,
        target: &Target,
        identifier: &str,
        text: &str,
    ) -> Result<(), AdapterError> {
        let wait_for = format!("--text={text}");
        self.call(target, &["wait", "--timeout", WAIT, &wait_for])?;
        let reply = self.call(
            target,
            &["get", "text", "--timeout", WAIT, "--", &id(identifier)],
        )?;
        match reply.data["text"].as_str() {
            Some(shown) if shown == text => Ok(()),
            shown => Err(AdapterError::new(format!(
                "{identifier} shows {}, not {text:?}",
                shown.map_or_else(|| "no text".to_owned(), |s| format!("{s:?}"))
            ))),
        }
    }
}

impl Adapter for CliAdapter {
    fn name(&self) -> &str {
        TOOL
    }

    fn role(&self) -> Role {
        Role::UnderTest
    }

    fn detect(&mut self) -> Detection {
        let program = &self.config.program;
        let path = if program.components().count() > 1 {
            if !is_executable(program) {
                return Detection::Absent {
                    note: format!("{} is not an executable file", program.display()),
                };
            }
            program.clone()
        } else {
            match find_executable(&program.to_string_lossy(), &self.config.search_path) {
                Some(path) => path,
                None => {
                    return Detection::Absent {
                        note: format!("{} not found on PATH", program.display()),
                    };
                }
            }
        };
        let mut command = self.command(&path);
        command.arg("--version");
        let version = process::run(&mut command, self.config.command_timeout)
            .ok()
            .filter(|finished| finished.status.success())
            .and_then(|finished| version_from(&finished.stdout));
        self.path = Some(path);
        Detection::Available { version }
    }

    fn supports(&self, _target: &Target, scenario: &Scenario, subject: &Subject) -> bool {
        let Subject::Primitive(primitive) = subject else {
            return true;
        };
        let fixture = scenario.fixture.is_some();
        match primitive {
            Primitive::CliStart => true,
            Primitive::ColdAttach => fixture,
            Primitive::FirstSnapshot | Primitive::WarmSnapshot | Primitive::Screenshot => {
                fixture && scenario.screen.is_some()
            }
            Primitive::TextEntry => fixture && scenario.screen == Some(Screen::Login),
            Primitive::TapDispatch
            | Primitive::TapToFrameChange
            | Primitive::TapToTreeDelta
            | Primitive::Settle => fixture && scenario.screen == Some(Screen::List),
        }
    }

    /// Opens the scenario's screen for the primitives measured on an open
    /// screen, reads `status` for what the daemon says about the target, and
    /// takes one snapshot before warm snapshots. Tap to frame change is not
    /// measured on a target that gives no frames: an Android device, which
    /// has no frame stream, or a target whose `status` says it gives none,
    /// such as an emulator whose controller did not answer.
    fn prepare(
        &mut self,
        target: &Target,
        scenario: &Scenario,
        subject: &Subject,
    ) -> Result<(), AdapterError> {
        let Subject::Primitive(primitive) = *subject else {
            return Ok(());
        };
        if primitive == Primitive::TapToFrameChange && target.class == TargetClass::AndroidDevice {
            return Err(AdapterError::not_measured(NO_FRAME_STREAM));
        }
        match primitive {
            Primitive::CliStart | Primitive::ColdAttach | Primitive::FirstSnapshot => Ok(()),
            _ => {
                let screen = screen(scenario)?;
                self.open_screen(target, app(target, scenario)?, screen)?;
                self.observe(target);
                if primitive == Primitive::TapToFrameChange
                    && let Some(attached) = self.attached.get(&target.id)
                    && attached.frame_source == Some(false)
                {
                    return Err(AdapterError::not_measured(no_frames(
                        target,
                        attached.why_no_frames.as_deref(),
                    )));
                }
                if primitive == Primitive::WarmSnapshot {
                    self.snapshot(target, screen)?;
                }
                Ok(())
            }
        }
    }

    fn measure(
        &mut self,
        target: &Target,
        scenario: &Scenario,
        primitive: Primitive,
    ) -> Result<Sample, AdapterError> {
        match primitive {
            Primitive::CliStart => self.process_start(),
            Primitive::ColdAttach => self.cold_attach(target, app(target, scenario)?),
            Primitive::FirstSnapshot => {
                self.first_snapshot(target, app(target, scenario)?, screen(scenario)?)
            }
            Primitive::WarmSnapshot => {
                let reply = self.snapshot(target, screen(scenario)?)?;
                Ok(wall(reply.elapsed, Some(refs(&reply)?), None))
            }
            Primitive::TapDispatch | Primitive::TapToFrameChange | Primitive::TapToTreeDelta => {
                self.tap(target, primitive)
            }
            Primitive::TextEntry => self.text_entry(target),
            Primitive::Screenshot => self.screenshot(target),
            Primitive::Settle => self.settle(target),
        }
    }

    /// Reads the screen with a full `snapshot` and summarizes its listing in
    /// the terms of the flow's golden: each element line's identifier, and
    /// its value, else its name, as `get text` reads it.
    fn end_state(&mut self, target: &Target, flow: &Flow) -> Result<EndState, AdapterError> {
        let reply = self.call(target, &["snapshot"])?;
        let listing = reply.data["snapshot"]
            .as_str()
            .ok_or_else(|| AdapterError::new("the snapshot reply has no listing"))?;
        let elements: Vec<Element> = listing::parse(listing)
            .iter()
            .map(|line| Element::new(line.id.as_deref(), Some(line.text())))
            .collect();
        Ok(flow.golden.observe(&elements))
    }

    /// One run of the flow's steps, without the node count, then its end
    /// state.
    fn check_flow(&mut self, target: &Target, flow: &Flow) -> Result<EndState, AdapterError> {
        self.run_steps(target, flow)?;
        self.end_state(target, flow)
    }

    /// Runs the flow's steps, timed by wall clock, then counts the refs of the
    /// final screen when the flow records its node count. The sample's detail
    /// holds the notes of its fills and scroll-tos, if the flow has any.
    fn run_flow(&mut self, target: &Target, flow: &Flow) -> Result<Sample, AdapterError> {
        let (elapsed, notes) = self.run_steps(target, flow)?;
        let nodes = if flow.count_nodes {
            Some(refs(&self.call(target, &["snapshot", "-i"])?)?)
        } else {
            None
        };
        let detail = (!notes.is_empty()).then(|| notes.join("; "));
        Ok(wall(elapsed, nodes, detail))
    }

    /// Reads `status`, which keeps what the daemon says about `target`, then
    /// stops the daemon, whose agent holds an Android target's UI automation
    /// until the daemon exits; the `status` starts a daemon when none serves,
    /// which the shutdown then stops. Returns once the daemon has let go of
    /// its targets.
    fn release(&mut self, target: &Target, _: &dyn Probe) -> Result<Option<String>, AdapterError> {
        self.observe(target);
        Ok(Some(
            if self.stop_daemon()? {
                "stopped the daemon"
            } else {
                "no daemon ran"
            }
            .to_owned(),
        ))
    }

    /// Whether the emulator's controller answered and whether the target gave
    /// frames, as `status` last said while the daemon had it attached, and on
    /// a physical iPhone its runner and its input and frame paths, with its
    /// battery as `status` first gave it.
    fn describe(&self, target: &mut Target) {
        if let Some(attached) = self.attached.get(&target.id) {
            target.controller = target.controller.or(attached.controller);
            target.frame_source = target.frame_source.or(attached.frame_source);
            target.runner = target.runner.take().or_else(|| attached.runner.clone());
            target.input_path = target
                .input_path
                .take()
                .or_else(|| attached.input_path.clone());
            target.frame_path = target
                .frame_path
                .take()
                .or_else(|| attached.frame_path.clone());
            target.battery = target.battery.or(attached.battery);
        }
    }
}

/// What the data of a `status` reply says about `target`: its entry's
/// `frameSource`, and from its driver's report, on an Android emulator,
/// whether the controller answered (an `endpoint`) or not (none), on a
/// physical iPhone the runner's `agent`, the `input` and `frames` paths and
/// the `battery`, and the driver's `warning` and `frameError`. `None` when
/// the daemon does not list the target or says nothing about it, as for a
/// target it has not attached.
fn read_attached(data: &Value, target: &Target) -> Option<Attached> {
    let entry = data["targets"]
        .as_array()?
        .iter()
        .find(|entry| entry["id"] == target.id.as_str())?;
    let driver = entry.get("driver").filter(|driver| driver.is_object());
    let controller = match (target.class, driver) {
        (TargetClass::AndroidEmulator, Some(driver)) => Some(
            match driver["endpoint"].as_str().filter(|e| !e.is_empty()) {
                Some(_) => Controller::On,
                None => Controller::Off,
            },
        ),
        _ => None,
    };
    let why: Vec<&str> = driver
        .map(|driver| {
            ["warning", "frameError"]
                .into_iter()
                .filter_map(|key| driver[key].as_str())
                .filter(|text| !text.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut attached = Attached {
        frame_source: entry["frameSource"].as_bool(),
        controller,
        why_no_frames: (!why.is_empty()).then(|| why.join("; ")),
        ..Attached::default()
    };
    if let (TargetClass::IosDevice, Some(driver)) = (target.class, driver) {
        let text = |value: &Value| value.as_str().map(str::to_owned);
        attached.runner = text(&driver["runner"]["agent"]);
        attached.input_path = text(&driver["input"]);
        attached.frame_path = text(&driver["frames"]);
        let battery = &driver["battery"];
        attached.battery = battery.is_object().then(|| Battery {
            level_pct: battery["levelPct"]
                .as_u64()
                .and_then(|level| u8::try_from(level).ok()),
            charging: battery["charging"].as_bool(),
            external_power: battery["externalPower"].as_bool(),
        });
    }
    (attached.frame_source.is_some() || attached.controller.is_some()).then_some(attached)
}

/// Why a tap on `target`, which gives no frames, has no frame change to
/// time, with the driver's reason when it gave one.
fn no_frames(target: &Target, why: Option<&str>) -> String {
    let reason = format!(
        "{} gives {TOOL} no frames, so a tap on it has no frame change to time",
        target.id
    );
    match why {
        Some(why) => format!("{reason}: {why}"),
        None => reason,
    }
}

/// The arguments of one command: JSON mode, then the bench's session and
/// `target` when given, then `command`.
fn command_line<'a>(target: Option<&'a Target>, command: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["--json"];
    if let Some(target) = target {
        args.extend(["--session", SESSION, "--device", target.id.as_str()]);
    }
    args.extend_from_slice(command);
    args
}

/// The targets in the data of a `devices` reply: each device's id, class,
/// model and OS.
fn read_devices(data: &Value) -> Result<Vec<Target>, String> {
    #[derive(Deserialize)]
    struct Device {
        id: String,
        class: TargetClass,
        model: Option<String>,
        os: Option<String>,
    }
    let devices = data
        .get("devices")
        .ok_or("the devices reply lists no devices")?;
    let devices: Vec<Device> = serde_json::from_value(devices.clone())
        .map_err(|e| format!("the devices reply does not list devices: {e}"))?;
    Ok(devices
        .into_iter()
        .map(|device| {
            let mut target = Target::new(device.id, device.class);
            target.model = device.model;
            target.os = device.os;
            target
        })
        .collect())
}

/// The envelope `--json` printed, or `None` when stdout holds none.
fn envelope(stdout: &str) -> Option<Envelope> {
    serde_json::from_str(stdout.trim()).ok()
}

/// The error of a failed reply to `command`: the command, the code and the
/// first line of the message, at most 500 characters of it.
fn failure(command: &[&str], reply: &Reply) -> AdapterError {
    let message = reply.error.as_deref().unwrap_or("the reply gave no error");
    let first: String = message
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(500)
        .collect();
    let command = command.join(" ");
    AdapterError::new(match &reply.code {
        Some(code) => format!("{TOOL} {command}: {code}: {first}"),
        None => format!("{TOOL} {command}: {first}"),
    })
}

/// `id=<identifier>`, the CLI's selector for an element's test identifier.
fn id(identifier: &str) -> String {
    format!("id={identifier}")
}

/// `selector` as the CLI takes it: `id=<identifier>` for an identifier, and
/// `text=<text>` for a text, which the CLI matches as part of an element's
/// name or value.
fn selector(selector: &Selector) -> String {
    match selector {
        Selector::Id(identifier) => id(identifier),
        Selector::Text(text) => format!("text={text}"),
    }
}

/// The element each screen always shows, whose presence tells that the screen
/// is up.
fn marker(screen: Screen) -> &'static str {
    match screen {
        Screen::Login => "login-submit",
        Screen::List => TAP,
        Screen::Big => "cell-500",
        Screen::Anim => "anim-toggle",
        Screen::System => "system-alert",
    }
}

/// The app of the scenario's fixture on `target`'s platform.
fn app(target: &Target, scenario: &Scenario) -> Result<&'static str, AdapterError> {
    let fixture = scenario
        .fixture
        .ok_or_else(|| AdapterError::new("the scenario names no fixture"))?;
    fixture.app_id(target.class.platform()).ok_or_else(|| {
        AdapterError::new(format!(
            "the {fixture} fixture does not run on {}",
            target.class
        ))
    })
}

fn screen(scenario: &Scenario) -> Result<Screen, AdapterError> {
    scenario
        .screen
        .ok_or_else(|| AdapterError::new("the scenario names no screen"))
}

/// Whether a snapshot reply's listing has an element with identifier `id`.
fn lists(reply: &Reply, id: &str) -> bool {
    reply.data["snapshot"].as_str().is_some_and(|text| {
        listing::parse(text)
            .iter()
            .any(|line| line.id.as_deref() == Some(id))
    })
}

/// The number of refs a snapshot reply gives: its node count.
fn refs(reply: &Reply) -> Result<u32, AdapterError> {
    reply.data["refs"]
        .as_object()
        .map(|refs| u32::try_from(refs.len()).unwrap_or(u32::MAX))
        .ok_or_else(|| AdapterError::new("the snapshot reply has no refs"))
}

/// A span a reply gives in milliseconds, to the microsecond.
fn millis(value: &Value, field: &str) -> Result<Duration, AdapterError> {
    value
        .as_f64()
        .filter(|ms| ms.is_finite() && *ms >= 0.0)
        .map(|ms| Duration::from_micros((ms * 1000.0).round() as u64))
        .ok_or_else(|| {
            AdapterError::new(format!("{field} is not a number of milliseconds: {value}"))
        })
}

/// The note of a scroll-to, from the data of its `scroll --until` reply: the
/// strokes the scroll took, then the share of the element in view when the
/// reply gives it (`visible`, a number from 0 to 1), such as `7 strokes,
/// visible 0.62`.
fn scrolled(data: &Value) -> String {
    let strokes = match data["strokes"].as_u64() {
        Some(strokes) => format!("{strokes} strokes"),
        None => "strokes unknown".to_owned(),
    };
    match &data["visible"] {
        Value::Number(visible) => format!("{strokes}, visible {visible}"),
        _ => strokes,
    }
}

/// The note of a fill, from the data of its `fill` reply and the reply's
/// warning: the method the field's text went in by, `retried` when `fill`
/// entered the text again in one piece, how the read-back found the field
/// (`verified`, as the text or, for a secure field, by its length) and after
/// how long (`readBackMs`), or that it was not read back, and the warning,
/// such as `method keys, read back as text in 253.112 ms`. A reply without
/// `verified`, from a `mobile-agent` that does not read fields back (before
/// task 10.23), gives the method alone; `None` when the reply says none of
/// these.
fn filled(data: &Value, warning: Option<&str>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(method) = data["method"].as_str() {
        parts.push(format!("method {method}"));
    }
    if data["retried"] == true {
        parts.push("retried".to_owned());
    }
    match data.get("verified") {
        Some(Value::String(shown)) => parts.push(match &data["readBackMs"] {
            Value::Number(ms) => format!("read back as {shown} in {ms} ms"),
            _ => format!("read back as {shown}"),
        }),
        Some(Value::Null) => parts.push("not read back".to_owned()),
        _ => {}
    }
    if let Some(warning) = warning {
        parts.push(format!("warning: {warning}"));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn wall(elapsed: Duration, nodes: Option<u32>, detail: Option<String>) -> Sample {
    Sample {
        elapsed,
        clock: Clock::Wall,
        nodes,
        detail,
    }
}

fn tool(elapsed: Duration, detail: Option<String>) -> Sample {
    Sample {
        elapsed,
        clock: Clock::Tool,
        nodes: None,
        detail,
    }
}

/// `id` with every character but letters, digits and `-` replaced by `_`.
fn file_safe(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_fill_s_note_says_how_the_field_was_read_back() {
        let note = |data: Value, warning: Option<&str>| filled(&data, warning);
        assert_eq!(
            note(
                json!({"method": "keys", "verified": "length", "readBackMs": 262.0, "retried": false}),
                None
            )
            .as_deref(),
            Some("method keys, read back as length in 262.0 ms")
        );
        assert_eq!(
            note(
                json!({"method": "direct", "verified": "text", "readBackMs": 843.125, "retried": true}),
                Some("id=search showed \"nobe\" after fill typed the text, so fill pasted it instead")
            )
            .as_deref(),
            Some(
                "method direct, retried, read back as text in 843.125 ms, warning: id=search \
                 showed \"nobe\" after fill typed the text, so fill pasted it instead"
            )
        );
        assert_eq!(
            note(
                json!({"method": "direct", "verified": null, "readBackMs": null, "retried": false}),
                None
            )
            .as_deref(),
            Some("method direct, not read back")
        );
        // A mobile-agent from before task 10.23 does not read the field back.
        assert_eq!(
            note(json!({"method": "keys"}), None).as_deref(),
            Some("method keys")
        );
        assert_eq!(note(json!({}), None), None);
    }

    #[test]
    fn commands_name_json_mode_the_session_and_the_target_first() {
        let target = Target::new("emulator-5554", TargetClass::AndroidEmulator);
        assert_eq!(
            command_line(
                Some(&target),
                &["click", "--timeout", WAIT, "--", "id=row-1"]
            ),
            [
                "--json",
                "--session",
                "mobile-agent-bench",
                "--device",
                "emulator-5554",
                "click",
                "--timeout",
                "10000",
                "--",
                "id=row-1"
            ]
        );
        assert_eq!(command_line(None, &["shutdown"]), ["--json", "shutdown"]);
        assert_eq!(WAIT, WAIT_MS.to_string());
    }

    #[test]
    fn devices_give_each_target_s_id_class_model_and_os() {
        let data = json!({"devices": [
            {"claimedBy": null, "class": "android-emulator", "id": "emulator-5554", "model": "sdk_gphone64_arm64",
             "name": "Pixel 9", "os": "Android 16", "platform": "android"},
            {"claimedBy": "default", "class": "simulator", "id": "B0647CDD-575F-45E9-9398-F6F5AFF23C1F",
             "model": null, "name": "iPhone 17", "os": "iOS 26.5", "platform": "ios"}
        ]});
        let mut emulator = Target::new("emulator-5554", TargetClass::AndroidEmulator);
        emulator.model = Some("sdk_gphone64_arm64".into());
        emulator.os = Some("Android 16".into());
        let mut simulator = Target::new(
            "B0647CDD-575F-45E9-9398-F6F5AFF23C1F",
            TargetClass::Simulator,
        );
        simulator.os = Some("iOS 26.5".into());
        assert_eq!(read_devices(&data), Ok(vec![emulator, simulator]));
        // A physical iPhone, as the daemon's ios driver lists it.
        let data = json!({"devices": [
            {"claimedBy": null, "class": "ios-device", "id": "placeholder-udid-phone",
             "model": "iPhone14,6", "name": "placeholder-iphone", "os": "iOS 26.6.2",
             "platform": "ios", "state": "ready"}
        ]});
        let mut iphone = Target::new("placeholder-udid-phone", TargetClass::IosDevice);
        iphone.model = Some("iPhone14,6".into());
        iphone.os = Some("iOS 26.6.2".into());
        assert_eq!(read_devices(&data), Ok(vec![iphone]));
        assert_eq!(read_devices(&json!({"devices": []})), Ok(Vec::new()));
        let error = read_devices(&json!({"targets": []})).unwrap_err();
        assert!(error.contains("devices"), "{error}");
    }

    #[test]
    fn selectors_become_the_cli_s_identifier_and_text_forms() {
        assert_eq!(selector(&Selector::id("login-status")), "id=login-status");
        assert_eq!(selector(&Selector::text("Sign in")), "text=Sign in");
    }

    #[test]
    fn replies_read_agent_browser_s_envelope() {
        assert_eq!(
            envelope(r#"{"success":true,"data":{"text":"Welcome, agent"},"error":null}"#),
            Some(Envelope {
                success: true,
                data: json!({"text": "Welcome, agent"}),
                error: None,
                code: None,
                warning: None,
            })
        );
        assert_eq!(
            envelope(
                "{\"success\":false,\"data\":null,\"error\":\"e2 is stale\",\"code\":\"stale_ref\"}\n"
            ),
            Some(Envelope {
                success: false,
                data: Value::Null,
                error: Some("e2 is stale".into()),
                code: Some("stale_ref".into()),
                warning: None,
            })
        );
        assert_eq!(
            envelope(
                r#"{"success":true,"data":{"filled":"id=search"},"error":null,"warning":"careful"}"#
            )
            .and_then(|envelope| envelope.warning),
            Some("careful".to_owned())
        );
        assert_eq!(envelope("mobile-agent 0.1.0\n"), None);
        assert_eq!(envelope(r#"{"data": {}}"#), None);
    }

    #[test]
    fn every_screen_has_a_marker_its_screen_always_shows() {
        assert_eq!(marker(Screen::Big), "cell-500");
        assert_eq!(marker(Screen::List), "row-1");
        assert_eq!(marker(Screen::Login), "login-submit");
        assert_eq!(marker(Screen::Anim), "anim-toggle");
        assert_eq!(marker(Screen::System), "system-alert");
    }

    #[test]
    fn spans_in_milliseconds_become_whole_microseconds() {
        assert_eq!(
            millis(&json!(14.5), "frameChangeMs"),
            Ok(Duration::from_micros(14_500))
        );
        assert_eq!(
            millis(&json!(0.2216), "dispatchMs"),
            Ok(Duration::from_micros(222))
        );
        assert!(millis(&json!(-1.0), "treeDeltaMs").is_err());
        assert!(millis(&json!("12"), "treeDeltaMs").is_err());
        assert!(millis(&Value::Null, "elapsedMs").is_err());
    }

    /// `status` data as the daemon's README shows it for an emulator started
    /// with `-grpc 8554`, and for the same emulator without it.
    fn status(driver: Value, frame_source: Value) -> Value {
        json!({
            "daemon": {"pid": 12499, "version": "0.1.0", "socket": "/tmp/ma/daemon.sock"},
            "targets": [
                {"id": "emulator-5554", "platform": "android", "class": "android-emulator",
                 "attached": true, "frameSource": frame_source, "driver": driver},
                {"id": "emulator-5556", "platform": "android", "class": "android-emulator",
                 "attached": false, "frameSource": null}
            ],
            "sessions": []
        })
    }

    #[test]
    fn status_says_whether_the_emulator_s_controller_answered_and_whether_it_gives_frames() {
        let emulator = Target::new("emulator-5554", TargetClass::AndroidEmulator);
        let on = status(
            json!({"input": "emulator controller", "frames": "emulator controller",
                   "endpoint": "http://127.0.0.1:8554", "warning": null, "frameError": null}),
            json!(true),
        );
        assert_eq!(
            read_attached(&on, &emulator),
            Some(Attached {
                frame_source: Some(true),
                controller: Some(Controller::On),
                ..Attached::default()
            })
        );
        let off = status(
            json!({"input": "agent", "frames": null, "endpoint": null,
                   "warning": "emulator control endpoint unavailable for emulator-5554",
                   "frameError": null}),
            json!(false),
        );
        assert_eq!(
            read_attached(&off, &emulator),
            Some(Attached {
                frame_source: Some(false),
                controller: Some(Controller::Off),
                why_no_frames: Some(
                    "emulator control endpoint unavailable for emulator-5554".into()
                ),
                ..Attached::default()
            })
        );
        // A target the daemon has not attached, or does not list, says nothing.
        let other = Target::new("emulator-5556", TargetClass::AndroidEmulator);
        assert_eq!(read_attached(&on, &other), None);
        let absent = Target::new("emulator-5558", TargetClass::AndroidEmulator);
        assert_eq!(read_attached(&on, &absent), None);
        assert_eq!(read_attached(&json!({}), &emulator), None);
    }

    #[test]
    fn only_an_emulator_has_a_controller_to_report() {
        // An Android device and a Simulator give frames or not; neither has
        // a controller.
        let device = Target::new("R58M123", TargetClass::AndroidDevice);
        let data = json!({"targets": [{"id": "R58M123", "attached": true, "frameSource": false,
                                        "driver": {"input": "agent", "frames": null, "endpoint": null}}]});
        assert_eq!(
            read_attached(&data, &device),
            Some(Attached {
                frame_source: Some(false),
                ..Attached::default()
            })
        );
        let simulator = Target::new("SIM-1", TargetClass::Simulator);
        let data = json!({"targets": [{"id": "SIM-1", "attached": true, "frameSource": false,
                                        "driver": {"input": "dtuhidd", "frameError": "no display"}}]});
        assert_eq!(
            read_attached(&data, &simulator),
            Some(Attached {
                frame_source: Some(false),
                why_no_frames: Some("no display".into()),
                ..Attached::default()
            })
        );
        assert_eq!(
            no_frames(&simulator, Some("no display")),
            "SIM-1 gives mobile-agent no frames, so a tap on it has no frame change to time: no \
             display"
        );
    }

    /// `status` data for an attached iPhone, as the daemon's ios driver
    /// reports it with its runner ready (`crates/daemon`, `status_report`).
    fn iphone_status(battery: Value) -> Value {
        json!({"targets": [{
            "id": "placeholder-udid-phone", "platform": "ios", "class": "ios-device",
            "attached": true, "frameSource": true,
            "driver": {
                "input": "dtuhidd",
                "inputReason": "iOS 26.6.2 takes dtuhidd's reports from the host without a display stream",
                "lastInputMs": 54.5, "frames": "screenshots", "captureRate": 19.1,
                "framesReceived": 31, "frameError": null,
                "runner": {"agent": "mobile-agent-ios-runner 0.2.0", "state": "ready",
                           "restarts": 0, "handshakeRetries": 0, "replaced": false,
                           "leftovers": [], "startedMs": 781.0, "lastDeath": null},
                "battery": battery,
                "tunnel": null, "diskImage": null, "runnerBundle": null
            }
        }]})
    }

    #[test]
    fn an_iphone_s_status_names_its_runner_its_input_and_frame_paths_and_its_battery() {
        let iphone = Target::new("placeholder-udid-phone", TargetClass::IosDevice);
        let status =
            iphone_status(json!({"levelPct": 87, "charging": true, "externalPower": true}));
        assert_eq!(
            read_attached(&status, &iphone),
            Some(Attached {
                frame_source: Some(true),
                runner: Some("mobile-agent-ios-runner 0.2.0".into()),
                input_path: Some("dtuhidd".into()),
                frame_path: Some("screenshots".into()),
                battery: Some(Battery {
                    level_pct: Some(87),
                    charging: Some(true),
                    external_power: Some(true),
                }),
                ..Attached::default()
            })
        );
        // A driver that read no battery leaves it unknown.
        let without = read_attached(&iphone_status(Value::Null), &iphone).unwrap();
        assert_eq!(without.battery, None);
        assert_eq!(
            without.runner.as_deref(),
            Some("mobile-agent-ios-runner 0.2.0")
        );
    }

    #[test]
    fn only_an_iphone_s_runner_input_and_frame_paths_are_kept() {
        // An Android device's driver names its input and frames too; the
        // report keeps them for physical iPhones only.
        let device = Target::new("R58M123", TargetClass::AndroidDevice);
        let data = json!({"targets": [{"id": "R58M123", "attached": true, "frameSource": false,
            "driver": {"agent": "mobile-agent-android 0.2.0", "input": "agent", "frames": null}}]});
        let attached = read_attached(&data, &device).unwrap();
        assert_eq!(
            (attached.runner, attached.input_path, attached.frame_path),
            (None, None, None)
        );
    }

    #[test]
    fn describe_fills_in_an_iphone_s_runner_paths_and_its_first_battery() {
        let iphone = Target::new("placeholder-udid-phone", TargetClass::IosDevice);
        let mut adapter = CliAdapter::new(CliConfig::new("work"));
        let first = read_attached(
            &iphone_status(json!({"levelPct": 87, "charging": true, "externalPower": true})),
            &iphone,
        )
        .unwrap();
        let later = read_attached(
            &iphone_status(json!({"levelPct": 85, "charging": false, "externalPower": true})),
            &iphone,
        )
        .unwrap();
        adapter.keep(&iphone, first);
        adapter.keep(&iphone, later);
        let mut described = iphone.clone();
        adapter.describe(&mut described);
        assert_eq!(
            described.runner.as_deref(),
            Some("mobile-agent-ios-runner 0.2.0")
        );
        assert_eq!(described.input_path.as_deref(), Some("dtuhidd"));
        assert_eq!(described.frame_path.as_deref(), Some("screenshots"));
        assert_eq!(described.frame_source, Some(true));
        // The battery as the run first found it.
        assert_eq!(
            described.battery.and_then(|battery| battery.level_pct),
            Some(87)
        );
    }

    #[test]
    fn screenshot_files_are_named_for_their_target() {
        assert_eq!(file_safe("emulator-5554"), "emulator-5554");
        assert_eq!(file_safe("192.168.1.7:5555"), "192_168_1_7_5555");
    }
}
