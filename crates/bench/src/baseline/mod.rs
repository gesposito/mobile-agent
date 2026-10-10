//! Baseline tools the harness compares `mobile-agent` with on the same flows:
//! Maestro, maestro-runner and agent-device, and on a physical iPhone
//! WebDriverAgent through Appium's XCUITest driver ([`wda`]). Each adapter
//! runs a flow through the tool's own CLI in the shape the tool's
//! documentation gives, or for WebDriverAgent as WebDriver requests, reads
//! the screen through the tool's hierarchy, snapshot or source for the
//! flow's golden check and node count, and a tool that is not on `PATH` is
//! skipped with a note, as is a tool that runs on none of the run's targets.
//! Only flows run through them; none of them exposes the spans the
//! primitives measure.

pub mod agent_device;
pub mod iphone;
pub mod maestro;
pub mod wda;
#[cfg(target_os = "macos")]
pub mod wda_xctest;

use std::cell::OnceCell;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::adapter::{Adapter, AdapterError, Clock, Detection, Left, Role, Sample, Scenario};
use crate::end_state::{EndState, Reading};
use crate::flow::Flow;
use crate::harness::Subject;
use crate::process::{self, Finished};
use crate::provenance::Probe;
use crate::target::{Platform, Target, TargetClass};

/// The agent-device session every flow runs in.
pub const SESSION: &str = "mobile-agent-bench";

/// A baseline tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineTool {
    Maestro,
    MaestroRunner,
    AgentDevice,
}

impl BaselineTool {
    /// Every baseline, in report order.
    pub const ALL: [BaselineTool; 3] = [
        BaselineTool::Maestro,
        BaselineTool::MaestroRunner,
        BaselineTool::AgentDevice,
    ];

    /// The executable's name, which is also the tool's name in reports.
    pub fn name(self) -> &'static str {
        match self {
            BaselineTool::Maestro => "maestro",
            BaselineTool::MaestroRunner => "maestro-runner",
            BaselineTool::AgentDevice => "agent-device",
        }
    }
}

/// The team's signing for the runners the baseline tools build for a
/// physical iPhone: the team `MOBILE_AGENT_IOS_TEAM` names, and the identity
/// and profile `MOBILE_AGENT_IOS_IDENTITY` and `MOBILE_AGENT_IOS_PROFILE`
/// name, `Apple Development` and `mobile-agent development` by default, as
/// for `mobile-agent`'s runner and the fixtures' device builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IosSigning {
    pub team: String,
    pub identity: String,
    pub profile: String,
}

impl IosSigning {
    /// The signing the variables `var` reads name; `None` without a team.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let named = |name: &str| var(name).filter(|value| !value.is_empty());
        Some(IosSigning {
            team: named("MOBILE_AGENT_IOS_TEAM")?,
            identity: named("MOBILE_AGENT_IOS_IDENTITY")
                .unwrap_or_else(|| "Apple Development".to_owned()),
            profile: named("MOBILE_AGENT_IOS_PROFILE")
                .unwrap_or_else(|| "mobile-agent development".to_owned()),
        })
    }

    /// The signing the environment names.
    pub fn from_env() -> Option<Self> {
        IosSigning::from_vars(|name| std::env::var(name).ok())
    }
}

/// Where a baseline adapter looks for its tool, where it writes flow files,
/// how long it waits, and what it needs to know about the run's targets.
#[derive(Clone, Debug)]
pub struct BaselineConfig {
    /// Directories searched for the executable, in order: `PATH` by default.
    pub search_path: Vec<PathBuf>,
    /// Where flow files are written for the tools to read.
    pub work_dir: PathBuf,
    /// Deadline for one run of a flow.
    pub flow_timeout: Duration,
    /// Deadline for `--version`; Maestro starts a JVM to answer it.
    pub version_timeout: Duration,
    /// The classes of the run's targets; empty when not known, as when
    /// `mobile-agent-bench baselines` lists the tools. A tool that runs on
    /// none of them is skipped with the reason.
    pub targets: Vec<TargetClass>,
    /// The team's signing for runners built for a physical iPhone, if the
    /// environment names a team.
    pub ios_signing: Option<IosSigning>,
    /// The program, with its first arguments, that starts WebDriverAgent on
    /// a physical iPhone ([`wda`]): `mobile-agent-bench wda-session`.
    pub wda_helper: Vec<OsString>,
    /// The port Appium serves on for WebDriverAgent ([`wda::APPIUM_PORT`]).
    pub appium_port: u16,
}

impl BaselineConfig {
    /// Searches `PATH`, writes flow files under `work_dir`, gives a flow
    /// 120 s and `--version` 30 s, takes the team's signing from the
    /// environment, and starts WebDriverAgent with this program's own
    /// `wda-session`; the run's targets are not known.
    pub fn new(work_dir: impl Into<PathBuf>) -> Self {
        BaselineConfig {
            search_path: search_path(),
            work_dir: work_dir.into(),
            flow_timeout: Duration::from_secs(120),
            version_timeout: Duration::from_secs(30),
            targets: Vec::new(),
            ios_signing: IosSigning::from_env(),
            wda_helper: std::env::current_exe()
                .map(|exe| vec![exe.into_os_string(), "wda-session".into()])
                .unwrap_or_default(),
            appium_port: wda::APPIUM_PORT,
        }
    }

    /// This configuration for a run on targets of `classes`.
    pub fn for_targets(mut self, classes: impl IntoIterator<Item = TargetClass>) -> Self {
        self.targets = classes.into_iter().collect();
        self
    }
}

/// The directories of the `PATH` environment variable.
pub fn search_path() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

/// The first executable file named `name` in `dirs`, following symbolic links.
pub fn find_executable(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|path| is_executable(path))
}

/// Whether `path` is a file anyone may execute.
pub(crate) fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The version in a tool's `--version` output: the first word of its first
/// non-empty line that starts with a digit, or with `v` and a digit, else the
/// whole line.
pub fn version_from(output: &str) -> Option<String> {
    let line = output.lines().map(str::trim).find(|l| !l.is_empty())?;
    let version = line.split_whitespace().find_map(|word| {
        let bare = word.strip_prefix('v').unwrap_or(word);
        bare.starts_with(|c: char| c.is_ascii_digit())
            .then_some(bare)
    });
    Some(version.unwrap_or(line).to_owned())
}

/// The JSON object a tool printed: all of `stdout`, or else the first object
/// that starts a line, read to its end with whatever follows ignored, so
/// progress lines before a reply and pretty-printed replies both read.
pub(crate) fn json_document(stdout: &str) -> Option<Value> {
    if let Some(value) = serde_json::from_str::<Value>(stdout.trim())
        .ok()
        .filter(Value::is_object)
    {
        return Some(value);
    }
    let mut offset = 0;
    for line in stdout.split_inclusive('\n') {
        if line.trim_start().starts_with('{')
            && let Some(Ok(value)) = serde_json::Deserializer::from_str(&stdout[offset..])
                .into_iter::<Value>()
                .next()
            && value.is_object()
        {
            return Some(value);
        }
        offset += line.len();
    }
    None
}

/// The last line `output` holds, for errors about output that was not what
/// the tool should have printed.
pub(crate) fn last_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).rfind(|line| !line.is_empty())
}

/// A reader of what a tool prints for its screen.
type ReadScreen = fn(&str) -> Result<Reading, String>;

/// One adapter per baseline tool, in report order: the CLI tools, then
/// WebDriverAgent.
pub fn baselines(config: &BaselineConfig) -> Vec<Box<dyn Adapter>> {
    BaselineTool::ALL
        .into_iter()
        .map(|tool| Box::new(Baseline::new(tool, config.clone())) as Box<dyn Adapter>)
        .chain(std::iter::once(
            Box::new(wda::WebDriverAgent::new(config.clone())) as Box<dyn Adapter>,
        ))
        .collect()
}

/// Whether every target of a run on `classes` is a physical iPhone; false
/// when the classes are not known.
fn iphones_only(classes: &[TargetClass]) -> bool {
    !classes.is_empty() && classes.iter().all(|class| *class == TargetClass::IosDevice)
}

/// The processes of the iPhone `udid`, listed through the `xcrun` found in
/// `search_path`: what a baseline reads before it starts there.
fn phone_processes(search_path: &[PathBuf], udid: &str) -> Result<Vec<iphone::Process>, String> {
    let xcrun = find_executable("xcrun", search_path)
        .ok_or("xcrun not found on PATH, so the iPhone's processes cannot be listed")?;
    let finished = process::run(
        Command::new(xcrun).args(iphone::processes_args(udid)),
        Duration::from_secs(30),
    )?;
    if !finished.status.success() {
        return Err(format!(
            "devicectl did not list the processes of {udid}: {}",
            exit_failure(&finished)
        ));
    }
    iphone::processes(&finished.stdout)
}

/// Fails, before `tool` starts on the iPhone `target`, when another tool's
/// runner holds the phone's automation session, which would keep `tool`'s
/// own runner waiting rather than failing.
pub(crate) fn check_phone_is_free(
    search_path: &[PathBuf],
    target: &Target,
    tool: &str,
) -> Result<(), AdapterError> {
    let processes = phone_processes(search_path, &target.id).map_err(AdapterError::new)?;
    match iphone::held_by_another(&processes, tool) {
        Some(held) => Err(AdapterError::new(format!("{tool} did not start: {held}"))),
        None => Ok(()),
    }
}

/// How long a release waits for the runner app it ended on an iPhone to
/// leave the phone's process list: devicectl's termination returned in
/// 0.26 s and the runner was gone at the next listing.
const RUNNER_EXIT: Duration = Duration::from_secs(5);

/// Ends `tool`'s runner app on the iPhone `udid` through `probe` and waits
/// up to [`RUNNER_EXIT`] for it to leave the process list; returns whether
/// it ended one.
pub(crate) fn end_phone_runner(probe: &dyn Probe, udid: &str, tool: &str) -> Result<bool, String> {
    let runner = iphone::runner_of(tool).ok_or_else(|| format!("{tool} keeps no runner"))?;
    let listed = |probe: &dyn Probe| -> Result<Vec<u32>, String> {
        let json = probe
            .output("xcrun", &iphone::processes_args(udid))
            .ok_or_else(|| format!("cannot list the processes of {udid} with devicectl"))?;
        Ok(iphone::processes(&json)?
            .into_iter()
            .filter(|process| process.executable == runner)
            .map(|process| process.pid)
            .collect())
    };
    let pids = listed(probe)?;
    if pids.is_empty() {
        return Ok(false);
    }
    for pid in &pids {
        probe
            .output(
                "xcrun",
                &iphone::terminate_args(udid, &pid.to_string(), false),
            )
            .ok_or_else(|| format!("devicectl did not end {runner} (pid {pid}) on {udid}"))?;
    }
    let started = std::time::Instant::now();
    while !listed(probe)?.is_empty() {
        if started.elapsed() >= RUNNER_EXIT {
            return Err(format!(
                "{runner} was still running on {udid} {} s after devicectl ended it",
                RUNNER_EXIT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(true)
}

/// A baseline tool driven through its CLI.
#[derive(Clone, Debug)]
pub struct Baseline {
    tool: BaselineTool,
    config: BaselineConfig,
    /// The executable, once detected.
    path: Option<PathBuf>,
    /// agent-device's state directory, once asked for: the directory of the
    /// daemon its commands share.
    state_dir: OnceCell<String>,
}

impl Baseline {
    pub fn new(tool: BaselineTool, config: BaselineConfig) -> Self {
        Baseline {
            tool,
            config,
            path: None,
            state_dir: OnceCell::new(),
        }
    }

    /// The flow file for `flow` on `target`, rewritten before every run.
    fn flow_file(&self, flow: &Flow, target: &Target, extension: &str) -> PathBuf {
        self.config.work_dir.join(format!(
            "{}-{}-{}.{extension}",
            self.tool.name(),
            flow.name,
            target.id
        ))
    }
}

impl Adapter for Baseline {
    fn name(&self) -> &str {
        self.tool.name()
    }

    fn role(&self) -> Role {
        Role::Baseline
    }

    fn detect(&mut self) -> Detection {
        let name = self.tool.name();
        if self.tool == BaselineTool::Maestro && iphones_only(&self.config.targets) {
            return Detection::Absent {
                note: maestro::IPHONE_NOTE.to_owned(),
            };
        }
        let Some(path) = find_executable(name, &self.config.search_path) else {
            return Detection::Absent {
                note: format!("{name} not found on PATH"),
            };
        };
        let version = process::run(
            Command::new(&path).arg("--version"),
            self.config.version_timeout,
        )
        .ok()
        .filter(|finished| finished.status.success())
        .and_then(|finished| {
            version_from(&finished.stdout).or_else(|| version_from(&finished.stderr))
        });
        self.path = Some(path);
        Detection::Available { version }
    }

    /// Flows only, and Maestro's never on a physical iPhone, which 2.11.0
    /// refuses.
    fn supports(&self, target: &Target, _scenario: &Scenario, subject: &Subject) -> bool {
        matches!(subject, Subject::Flow(_))
            && !(self.tool == BaselineTool::Maestro && target.class == TargetClass::IosDevice)
    }

    /// On a physical iPhone, agent-device fails the series at once when
    /// another tool's runner holds the phone.
    fn prepare(
        &mut self,
        target: &Target,
        _scenario: &Scenario,
        _subject: &Subject,
    ) -> Result<(), AdapterError> {
        self.check_phone(target)
    }

    /// Runs the flow through the tool, timed, then, for a flow that records
    /// its node count, reads the screen once more outside the timing and
    /// counts the nodes the tool's output held.
    fn run_flow(&mut self, target: &Target, flow: &Flow) -> Result<Sample, AdapterError> {
        let mut sample = self.run_tool(target, flow)?;
        if flow.count_nodes {
            let reading = self.read_screen(target).map_err(|e| {
                AdapterError::new(format!(
                    "the flow ran, but its node count was not read: {e}"
                ))
            })?;
            sample.nodes = Some(reading.nodes);
        }
        Ok(sample)
    }

    /// Reads the screen through the tool and summarizes it in the terms of
    /// the flow's golden.
    fn end_state(&mut self, target: &Target, flow: &Flow) -> Result<EndState, AdapterError> {
        Ok(flow.golden.observe(&self.read_screen(target)?.elements))
    }

    /// One run of the flow through the tool, then one read of the screen;
    /// on a physical iPhone, agent-device first checks that no other tool's
    /// runner holds the phone.
    fn check_flow(&mut self, target: &Target, flow: &Flow) -> Result<EndState, AdapterError> {
        self.check_phone(target)?;
        self.run_tool(target, flow)?;
        self.end_state(target, flow)
    }

    /// Closes agent-device's session on the target, which stops the snapshot
    /// helper it keeps holding UI automation between commands and selects
    /// the input method it found again. It then stops agent-device's daemon,
    /// which would otherwise idle for five minutes into the next tool's
    /// series, on a Simulator with its XCTest runner and its snapshot bridge;
    /// it does so even when the close failed, since the daemon ends its
    /// sessions as it stops. On Android it then force-stops agent-device's
    /// helper apps still running on the device ([`stop_helpers`]); on a
    /// physical iPhone it ends agent-device's runner app on the phone,
    /// through CoreDevice, should it outlive its `xcodebuild`
    /// ([`end_phone_runner`]). Maestro and maestro-runner hold nothing:
    /// their driver ends with the process that runs a flow.
    fn release(
        &mut self,
        target: &Target,
        probe: &dyn Probe,
    ) -> Result<Option<String>, AdapterError> {
        if self.tool != BaselineTool::AgentDevice {
            return Ok(None);
        }
        let path = self
            .path
            .clone()
            .ok_or_else(|| AdapterError::new("agent-device was not detected"))?;
        let mut errors = Vec::new();
        let closed = self
            .agent_device(
                &path,
                Some(target),
                &agent_device::close_args(target, SESSION),
                "close",
                agent_device::closed,
            )
            .unwrap_or_else(|e| {
                errors.push(e.message);
                false
            });
        let stopped = self
            .agent_device(
                &path,
                None,
                &agent_device::daemon_stop_args(),
                "daemon stop",
                agent_device::stopped,
            )
            .unwrap_or_else(|e| {
                errors.push(e.message);
                false
            });
        let helpers = match target.class.platform() {
            Platform::Android => stop_helpers(probe, &target.id).unwrap_or_else(|e| {
                errors.push(e);
                Vec::new()
            }),
            Platform::Ios => Vec::new(),
        };
        let runner = target.class == TargetClass::IosDevice
            && end_phone_runner(probe, &target.id, self.tool.name()).unwrap_or_else(|e| {
                errors.push(e);
                false
            });
        if !errors.is_empty() {
            return Err(AdapterError::new(errors.join("; ")));
        }
        let released = agent_device::released(closed, stopped, &helpers, SESSION);
        Ok(match (released, runner) {
            (released, false) => released,
            (Some(done), true) => Some(match done.rsplit_once(" and ") {
                Some((first, last)) => {
                    format!("{first}, {last} and ended its runner app on the iPhone")
                }
                None => format!("{done} and ended its runner app on the iPhone"),
            }),
            (None, true) => Some("ended agent-device's runner app on the iPhone".to_owned()),
        })
    }

    /// What agent-device still runs for the target on the host: its daemon
    /// of the state directory its commands use, a stray once the release's
    /// stop has returned, on a Simulator its XCTest runner and its snapshot
    /// bridge, and on a physical iPhone its runner's `xcodebuild`
    /// ([`agent_device::left_running`]); the runner app on the phone is the
    /// hand-off's to look for. For Maestro and maestro-runner, nothing.
    fn left_running(&self, target: &Target, probe: &dyn Probe) -> Result<Vec<Left>, String> {
        if self.tool != BaselineTool::AgentDevice {
            return Ok(Vec::new());
        }
        let state_dir = match self.state_dir.get() {
            Some(dir) => dir,
            None => {
                let path = self.path.as_ref().ok_or("agent-device was not detected")?;
                let dir = self
                    .agent_device(
                        path,
                        None,
                        &agent_device::state_dir_args(),
                        "session state-dir",
                        agent_device::state_dir,
                    )
                    .map_err(|e| e.message)?;
                self.state_dir.get_or_init(|| dir)
            }
        };
        let programs = probe
            .output("ps", &agent_device::PS_PROGRAMS)
            .ok_or("cannot list the host's processes with ps")?;
        Ok(agent_device::left_running(
            &programs,
            target,
            state_dir,
            |pids| {
                let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
                probe
                    .output("ps", &agent_device::ps_details(&pids.join(",")))
                    .unwrap_or_default()
            },
        ))
    }
}

impl Baseline {
    /// The tool's process at `path` for a command on `target`: on a
    /// physical iPhone agent-device's gets the team's signing for the runner
    /// it builds there ([`agent_device::signing_env`]), and maestro-runner
    /// its team as an argument instead ([`maestro::runner_args`]).
    fn command(&self, path: &Path, target: Option<&Target>) -> Command {
        let mut command = Command::new(path);
        if self.tool == BaselineTool::AgentDevice
            && target.is_some_and(|t| t.class == TargetClass::IosDevice)
            && let Some(signing) = &self.config.ios_signing
        {
            command.envs(agent_device::signing_env(signing, |name| {
                std::env::var_os(name).is_some()
            }));
        }
        command
    }

    /// The team maestro-runner's iOS agent is signed for on a physical
    /// iPhone.
    fn team(&self) -> Option<&str> {
        self.config.ios_signing.as_ref().map(|s| s.team.as_str())
    }

    /// On a physical iPhone, agent-device's check that no other tool's
    /// runner holds the phone; nothing elsewhere and for the other tools.
    fn check_phone(&self, target: &Target) -> Result<(), AdapterError> {
        if self.tool != BaselineTool::AgentDevice || target.class != TargetClass::IosDevice {
            return Ok(());
        }
        check_phone_is_free(&self.config.search_path, target, self.tool.name())
    }

    /// Runs agent-device with `args`, for `target` when it names one, and
    /// reads what it printed with `read`; a failure names the command,
    /// `what`, and how the process ended.
    fn agent_device<T>(
        &self,
        path: &Path,
        target: Option<&Target>,
        args: &[OsString],
        what: &str,
        read: fn(&str) -> Result<T, String>,
    ) -> Result<T, AdapterError> {
        let finished = process::run(
            self.command(path, target).args(args),
            self.config.flow_timeout,
        )
        .map_err(AdapterError::new)?;
        read(&finished.stdout).map_err(|e| {
            AdapterError::new(format!(
                "agent-device {what} failed: {e}; {}",
                exit_failure(&finished)
            ))
        })
    }

    /// Runs `flow` through the tool's process, which the sample times.
    fn run_tool(&self, target: &Target, flow: &Flow) -> Result<Sample, AdapterError> {
        let name = self.tool.name();
        let path = self
            .path
            .clone()
            .ok_or_else(|| AdapterError::new(format!("{name} was not detected")))?;
        let platform = target.class.platform();
        let app = flow.fixture.app_id(platform).ok_or_else(|| {
            AdapterError::new(format!(
                "the {} fixture does not run on {}",
                flow.fixture, target.class
            ))
        })?;
        let (file, contents, args) = match self.tool {
            BaselineTool::Maestro | BaselineTool::MaestroRunner => {
                let file = self.flow_file(flow, target, "yaml");
                let args = if self.tool == BaselineTool::Maestro {
                    maestro::maestro_args(target, &file)
                } else {
                    maestro::runner_args(target, &file, self.team())
                };
                (file, maestro::flow_yaml(flow, app), args)
            }
            BaselineTool::AgentDevice => {
                let file = self.flow_file(flow, target, "json");
                let args = agent_device::args(target, SESSION, &file);
                (file, agent_device::steps(flow, app, platform), args)
            }
        };
        fs::create_dir_all(&self.config.work_dir)
            .and_then(|()| fs::write(&file, contents))
            .map_err(|e| AdapterError::new(format!("cannot write {}: {e}", file.display())))?;
        let finished = process::run(
            self.command(&path, Some(target)).args(&args),
            self.config.flow_timeout,
        )
        .map_err(AdapterError::new)?;
        let detail = match self.tool {
            BaselineTool::AgentDevice => {
                let detail = agent_device::outcome(&finished.stdout).map_err(AdapterError::new)?;
                if !finished.status.success() {
                    return Err(AdapterError::new(exit_failure(&finished)));
                }
                detail
            }
            BaselineTool::Maestro | BaselineTool::MaestroRunner => {
                if !finished.status.success() {
                    return Err(AdapterError::new(exit_failure(&finished)));
                }
                None
            }
        };
        Ok(Sample {
            elapsed: finished.elapsed,
            clock: Clock::Wall,
            nodes: None,
            detail,
            read: None,
        })
    }

    /// Reads the screen through the tool: `maestro hierarchy`,
    /// `maestro-runner hierarchy` or `agent-device snapshot --json`, in the
    /// session the flows run in.
    fn read_screen(&self, target: &Target) -> Result<Reading, AdapterError> {
        let name = self.tool.name();
        let path = self
            .path
            .clone()
            .ok_or_else(|| AdapterError::new(format!("{name} was not detected")))?;
        let (args, read): (Vec<OsString>, ReadScreen) = match self.tool {
            BaselineTool::Maestro => (
                maestro::maestro_hierarchy_args(target),
                maestro::read_hierarchy,
            ),
            BaselineTool::MaestroRunner => (
                maestro::runner_hierarchy_args(target, self.team()),
                maestro::read_runner_hierarchy,
            ),
            BaselineTool::AgentDevice => (
                agent_device::snapshot_args(target, SESSION),
                agent_device::read_snapshot,
            ),
        };
        let finished = process::run(
            self.command(&path, Some(target)).args(&args),
            self.config.flow_timeout,
        )
        .map_err(AdapterError::new)?;
        read(&finished.stdout).map_err(|e| {
            AdapterError::new(if finished.status.success() {
                format!("{name} read no screen: {e}")
            } else {
                format!("{name} read no screen: {e}; {}", exit_failure(&finished))
            })
        })
    }
}

/// Force-stops each of agent-device's helper apps that has a process on the
/// Android device `serial`, as `adb` lists them through `probe`, and
/// returns what each was ([`agent_device::HELPERS`]). Neither `close` nor
/// `daemon stop` ends the input-method helper, which stays in the guest as
/// a cached process, and a daemon killed without its session closed leaves
/// the snapshot helper's instrumentation running.
fn stop_helpers(probe: &dyn Probe, serial: &str) -> Result<Vec<&'static str>, String> {
    let listing = probe
        .output("adb", &agent_device::guest_ps_args(serial))
        .ok_or_else(|| format!("cannot list the processes of {serial} with adb"))?;
    let mut stopped = Vec::new();
    let mut failed = Vec::new();
    for (package, what) in agent_device::running_helpers(&listing) {
        match probe.output("adb", &agent_device::force_stop_args(serial, package)) {
            Some(_) => stopped.push(what),
            None => failed.push(format!("cannot force-stop {package} on {serial} with adb")),
        }
    }
    if failed.is_empty() {
        Ok(stopped)
    } else {
        Err(failed.join("; "))
    }
}

/// The exit status and the last three lines the tool printed, at most 500
/// characters of them.
pub(crate) fn exit_failure(finished: &Finished) -> String {
    let status = finished.status.code().map_or_else(
        || "killed by a signal".to_owned(),
        |code| format!("exit status {code}"),
    );
    let lines: Vec<&str> = finished
        .stdout
        .lines()
        .chain(finished.stderr.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let tail: String = lines[lines.len().saturating_sub(3)..]
        .join(" / ")
        .chars()
        .take(500)
        .collect();
    if tail.is_empty() {
        status
    } else {
        format!("{status}: {tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mobile-agent-bench-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file(path: &Path, mode: u32) {
        fs::write(path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn an_executable_is_found_in_the_first_directory_holding_it() {
        let dir = scratch("find-first");
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        file(&a.join("maestro"), 0o755);
        file(&b.join("maestro"), 0o755);
        assert_eq!(
            find_executable("maestro", &[b.clone(), a.clone()]),
            Some(b.join("maestro"))
        );
        assert_eq!(find_executable("maestro-runner", &[a, b]), None);
    }

    #[test]
    fn files_nobody_may_execute_and_directories_are_not_tools() {
        let dir = scratch("find-skip");
        let (plain, holder, real) = (dir.join("plain"), dir.join("holder"), dir.join("real"));
        for d in [&plain, &holder, &real] {
            fs::create_dir_all(d).unwrap();
        }
        file(&plain.join("agent-device"), 0o644);
        fs::create_dir_all(holder.join("agent-device")).unwrap();
        file(&real.join("agent-device"), 0o755);
        assert_eq!(
            find_executable("agent-device", &[plain, holder, real.clone()]),
            Some(real.join("agent-device"))
        );
    }

    #[test]
    fn a_symbolic_link_to_an_executable_is_a_tool() {
        let dir = scratch("find-link");
        file(&dir.join("agent-device.mjs"), 0o755);
        std::os::unix::fs::symlink(dir.join("agent-device.mjs"), dir.join("agent-device")).unwrap();
        assert_eq!(
            find_executable("agent-device", std::slice::from_ref(&dir)),
            Some(dir.join("agent-device"))
        );
    }

    #[test]
    fn versions_are_read_from_the_first_line() {
        assert_eq!(version_from("2.10.0\n").as_deref(), Some("2.10.0"));
        assert_eq!(
            version_from("\nmaestro-runner version v1.1.0\nbuilt 2026-09-01\n").as_deref(),
            Some("1.1.0")
        );
        assert_eq!(version_from("0.21.14").as_deref(), Some("0.21.14"));
        assert_eq!(
            version_from("development build").as_deref(),
            Some("development build")
        );
        assert_eq!(version_from(" \n"), None);
    }

    #[test]
    fn baselines_are_flow_only_and_in_report_order() {
        let adapters = baselines(&BaselineConfig::new("/nonexistent"));
        let names: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
        assert_eq!(
            names,
            [
                "maestro",
                "maestro-runner",
                "agent-device",
                "webdriveragent"
            ]
        );
        let target = Target::new("emulator-5554", crate::target::TargetClass::AndroidEmulator);
        let adapter = &adapters[0];
        assert_eq!(adapter.role(), Role::Baseline);
        assert!(adapter.supports(
            &target,
            &Scenario::default(),
            &Subject::Flow("login".into())
        ));
        assert!(!adapter.supports(
            &target,
            &Scenario::default(),
            &Subject::Primitive(crate::primitive::Primitive::WarmSnapshot)
        ));
    }

    /// Each tool runs its flows where it can: Maestro never on a physical
    /// iPhone, which 2.11.0 refuses, WebDriverAgent only on one, the others
    /// on every class.
    #[test]
    fn each_baseline_runs_flows_on_the_classes_it_supports() {
        use crate::target::TargetClass::*;
        let adapters = baselines(&BaselineConfig::new("/nonexistent"));
        let login = Subject::Flow("login".into());
        let supported = |name: &str| -> Vec<TargetClass> {
            let adapter = adapters.iter().find(|a| a.name() == name).unwrap();
            [Simulator, AndroidEmulator, AndroidDevice, IosDevice]
                .into_iter()
                .filter(|class| {
                    adapter.supports(&Target::new("T-1", *class), &Scenario::default(), &login)
                })
                .collect()
        };
        assert_eq!(
            supported("maestro"),
            [Simulator, AndroidEmulator, AndroidDevice]
        );
        assert_eq!(
            supported("maestro-runner"),
            [Simulator, AndroidEmulator, AndroidDevice, IosDevice]
        );
        assert_eq!(
            supported("agent-device"),
            [Simulator, AndroidEmulator, AndroidDevice, IosDevice]
        );
        assert_eq!(supported("webdriveragent"), [IosDevice]);
    }

    #[test]
    fn the_team_s_signing_comes_from_the_runner_s_variables() {
        let vars = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        assert_eq!(
            IosSigning::from_vars(vars(&[("MOBILE_AGENT_IOS_TEAM", "TEAM123456")])),
            Some(IosSigning {
                team: "TEAM123456".into(),
                identity: "Apple Development".into(),
                profile: "mobile-agent development".into(),
            })
        );
        assert_eq!(
            IosSigning::from_vars(vars(&[
                ("MOBILE_AGENT_IOS_TEAM", "TEAM123456"),
                ("MOBILE_AGENT_IOS_IDENTITY", "Apple Development: someone"),
                ("MOBILE_AGENT_IOS_PROFILE", "another profile"),
            ])),
            Some(IosSigning {
                team: "TEAM123456".into(),
                identity: "Apple Development: someone".into(),
                profile: "another profile".into(),
            })
        );
        assert_eq!(IosSigning::from_vars(vars(&[])), None);
        assert_eq!(
            IosSigning::from_vars(vars(&[("MOBILE_AGENT_IOS_TEAM", "")])),
            None
        );
    }

    /// On a run whose targets are all physical iPhones Maestro is skipped
    /// with the reason, without being asked anything; on a run with any
    /// other target it is detected as ever.
    #[test]
    fn maestro_is_skipped_with_its_reason_on_a_run_of_iphones() {
        let dir = scratch("maestro-iphones");
        fs::write(
            dir.join("maestro"),
            "#!/bin/sh\necho \"$@\" >> \"${0%/*}/maestro.log\"\necho 2.10.0\n",
        )
        .unwrap();
        fs::set_permissions(dir.join("maestro"), fs::Permissions::from_mode(0o755)).unwrap();
        let config = BaselineConfig {
            search_path: vec![dir.clone()],
            ..BaselineConfig::new(dir.join("work"))
        };
        let mut on_phones = Baseline::new(
            BaselineTool::Maestro,
            config.clone().for_targets([TargetClass::IosDevice]),
        );
        assert_eq!(
            on_phones.detect(),
            Detection::Absent {
                note: maestro::IPHONE_NOTE.to_owned()
            }
        );
        assert!(!dir.join("maestro.log").exists());
        let mut mixed = Baseline::new(
            BaselineTool::Maestro,
            config.for_targets([TargetClass::IosDevice, TargetClass::Simulator]),
        );
        assert_eq!(
            mixed.detect(),
            Detection::Available {
                version: Some("2.10.0".into())
            }
        );
    }
}
