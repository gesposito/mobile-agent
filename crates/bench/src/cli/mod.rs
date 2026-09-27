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
//! the target cannot produce is reported as not measured, with the reason.
//! Snapshots count the refs in their reply as their nodes.

pub mod listing;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::adapter::{Adapter, AdapterError, Clock, Detection, Role, Sample, Scenario};
use crate::baseline::{exit_failure, find_executable, is_executable, search_path, version_from};
use crate::end_state::{Element, EndState};
use crate::fixture::Screen;
use crate::flow::{Direction, Flow, Selector, Step};
use crate::harness::Subject;
use crate::primitive::{Primitive, TEXT_ENTRY};
use crate::process;
use crate::target::{Target, TargetClass};

/// The tool's name in reports.
pub const TOOL: &str = "mobile-agent";

/// The session every command runs in.
pub const SESSION: &str = "mobile-agent-bench";

/// How long a selector, an element wait or a text wait may wait: 10 s, as
/// agent-device's `wait` steps do.
const WAIT_MS: u64 = 10_000;
const WAIT: &str = "10000";

/// How far one step of a scroll-to moves the content, in points: less than a
/// phone screen's content, so no element passes through the view between two
/// checks.
pub const SCROLL_STEP: u32 = 600;

/// The most steps a scroll-to takes before it fails.
pub const MAX_SCROLLS: u32 = 50;

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
}

/// A command's reply and when its process ran.
#[derive(Debug)]
struct Reply {
    success: bool,
    data: Value,
    error: Option<String>,
    code: Option<String>,
    started: Instant,
    elapsed: Duration,
}

impl Reply {
    fn ended(&self) -> Instant {
        self.started + self.elapsed
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
}

impl CliAdapter {
    pub fn new(config: CliConfig) -> Self {
        CliAdapter { config, path: None }
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
    /// starts a new one.
    fn stop_daemon(&self) -> Result<(), AdapterError> {
        for _ in 0..SHUTDOWNS {
            let reply = self.send(None, &["shutdown"])?;
            if !reply.success {
                return Err(failure(&["shutdown"], &reply));
            }
            if reply.data["stopped"] != true {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(AdapterError::new(format!(
            "a daemon still answered after {SHUTDOWNS} shutdowns"
        )))
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
        self.call(target, &["fill", "--timeout", WAIT, "--", &field, ""])?;
        self.call(target, &["wait", "--settle"])?;
        let method = reply.data["method"].as_str().map(|m| format!("method {m}"));
        Ok(wall(reply.elapsed, None, method))
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
    /// the first command's start to the last one's end, and how many scroll
    /// steps the flow took.
    fn run_steps(&self, target: &Target, flow: &Flow) -> Result<(Duration, u32), AdapterError> {
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
        let mut scrolls = 0;
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
                    self.call(
                        target,
                        &["fill", "--timeout", WAIT, "--", &selector(element), text],
                    )?;
                }
                Step::ScrollTo {
                    target: element,
                    direction,
                } => scrolls += self.scroll_to(target, element, *direction)?,
                Step::AssertVisible(element) => {
                    self.call(
                        target,
                        &["wait", "--timeout", WAIT, "--", &selector(element)],
                    )?;
                }
                Step::AssertText { id, text } => self.assert_text(target, id, text)?,
            }
        }
        Ok((started.elapsed(), scrolls))
    }

    /// Scrolls `direction` in steps of [`SCROLL_STEP`] points until `element`
    /// is visible. The screen settles before each check, so a check never
    /// races a moving list, and the check is a wait with no time to wait.
    /// Returns the number of steps.
    fn scroll_to(
        &self,
        target: &Target,
        element: &Selector,
        direction: Direction,
    ) -> Result<u32, AdapterError> {
        let element = selector(element);
        let direction = match direction {
            Direction::Down => "down",
            Direction::Up => "up",
        };
        let step = SCROLL_STEP.to_string();
        for steps in 0..=MAX_SCROLLS {
            self.call(target, &["wait", "--settle"])?;
            let check = ["wait", "--timeout", "0", "--", element.as_str()];
            let reply = self.send(Some(target), &check)?;
            if reply.success {
                return Ok(steps);
            }
            if reply.code.as_deref() != Some("wait_timeout") {
                return Err(failure(&check, &reply));
            }
            if steps < MAX_SCROLLS {
                self.call(target, &["scroll", "--", direction, &step])?;
            }
        }
        Err(AdapterError::new(format!(
            "{element} was not visible after {MAX_SCROLLS} scroll steps of {SCROLL_STEP} points \
             {direction}"
        )))
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
    /// screen, and takes one snapshot before warm snapshots. Tap to frame
    /// change on an Android device is not measured: it has no frame stream.
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
    /// final screen when the flow records its node count.
    fn run_flow(&mut self, target: &Target, flow: &Flow) -> Result<Sample, AdapterError> {
        let (elapsed, scrolls) = self.run_steps(target, flow)?;
        let nodes = if flow.count_nodes {
            Some(refs(&self.call(target, &["snapshot", "-i"])?)?)
        } else {
            None
        };
        let detail = (scrolls > 0).then(|| format!("{scrolls} scroll steps"));
        Ok(wall(elapsed, nodes, detail))
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
        .map(|device| Target {
            id: device.id,
            class: device.class,
            model: device.model,
            os: device.os,
            api_level: None,
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
            })
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

    #[test]
    fn screenshot_files_are_named_for_their_target() {
        assert_eq!(file_safe("emulator-5554"), "emulator-5554");
        assert_eq!(file_safe("192.168.1.7:5555"), "192_168_1_7_5555");
    }
}
