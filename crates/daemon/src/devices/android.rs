//! The Android driver (`crates/android`, design D2, D3, D5 and D7) behind
//! the daemon's driver contract.
//!
//! - Discovery lists every target the local adb server knows, in every
//!   state, with the advice for a state that keeps it from automation; no
//!   adb server means no Android targets.
//! - Attaching deploys the agent, reuses the one this daemon or its
//!   predecessor started, or replaces an incompatible one, with the
//!   records kept in the daemon's directory; selects the emulator's
//!   controller when it answers; and starts the agent session, whose
//!   backend is what the mirror runs on.
//! - Apps launch by package through their launcher activity, each launch
//!   environment entry a string extra of the intent under its own name,
//!   stopped first for a relaunch.
//! - Screenshots and status come from the session; dropping the
//!   attachment stops the agent, so no agent outlives a detached target.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use android::adb::{self, AdbServer, Device};
use android::agent::Attached;
use android::agent::{
    AGENT_VERSION, AgentJar, DeployError, DeployOptions, Deployer, JAR_VAR, Records,
};
use android::emulator;
use android::session::{
    ControllerChoice, Session, SessionError, SessionOptions, Status, select_paths,
};
use mirror::MirrorConfig;
use protocol::message::{ImageFormat, ScreenshotParams};
use serde_json::{Map, Value, json};

use crate::driver::{
    AppControl, AttachOptions, Attachment, Capture, CaptureRequest, Driver, DriverError, Image,
    Launch, Platform, Report, Screen, Size, TargetClass, TargetInfo, TargetReport,
};
use crate::wire::code::{APP_NOT_FOUND, IO_ERROR};

/// The driver's name, as `status`, `doctor` and `--drivers` give it.
pub(crate) const NAME: &str = "android";

/// The adb server or the device failed.
const ADB_FAILED: &str = "adb_failed";
/// Another process holds the device's UI automation.
const AUTOMATION_CONFLICT: &str = "automation_conflict";
/// The agent could not be started, or its jar read.
const AGENT_FAILED: &str = "agent_failed";
/// The agent jar is of another build than this daemon deploys.
const AGENT_MISMATCH: &str = "agent_mismatch";
/// The target's state keeps it from automation.
const TARGET_NOT_READY: &str = "target_not_ready";
/// `am start` could not open the app.
const LAUNCH_FAILED: &str = "launch_failed";

/// Where in the daemon's directory the records of the agents it started
/// are kept, one file per serial.
const RECORDS: &str = "android-agents";

/// How long a target listing may wait on the adb server: the listing runs
/// with the registry held, so a device that stops answering must not hold
/// every other command up for the client's default 10 s.
const LISTING_TIMEOUT: Duration = Duration::from_secs(3);

/// How long a stopped agent may take to exit before it is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// The agent jar `build.rs` built with `agent-android/build.sh` and this
/// daemon embeds; empty when the build was told to skip it
/// (`SKIP_AGENT_BUILD=1`).
pub(crate) static EMBEDDED_JAR: &[u8] = include_bytes!(env!("MOBILE_AGENT_EMBEDDED_JAR"));

/// The build of [`EMBEDDED_JAR`], as `AgentJar::build` names it, recorded
/// by `build.rs`.
pub(crate) const EMBEDDED_BUILD: &str = env!("MOBILE_AGENT_EMBEDDED_JAR_BUILD");

/// Where the agent jar a daemon deploys comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum JarSource {
    /// The file `MOBILE_AGENT_JAR` names, for development.
    File(PathBuf),
    /// The jar built into this daemon.
    Embedded,
}

/// Where the jar comes from, given `MOBILE_AGENT_JAR`'s value.
pub(crate) fn jar_source(variable: Option<&std::ffi::OsStr>) -> JarSource {
    match variable.filter(|value| !value.is_empty()) {
        Some(path) => JarSource::File(PathBuf::from(path)),
        None => JarSource::Embedded,
    }
}

/// The jar to deploy from `source`: the file, or the embedded jar, refused
/// with its reason when this daemon was built without one.
pub(crate) fn agent_jar(source: &JarSource) -> Result<AgentJar, DriverError> {
    match source {
        JarSource::File(path) => {
            AgentJar::from_path(path).map_err(|e| DriverError::new(AGENT_FAILED, e.to_string()))
        }
        JarSource::Embedded if EMBEDDED_JAR.is_empty() => Err(DriverError::new(
            AGENT_FAILED,
            "this daemon was built with SKIP_AGENT_BUILD=1 and embeds no Android agent; set \
             MOBILE_AGENT_JAR to a jar agent-android/build.sh wrote, or rebuild the daemon \
             without SKIP_AGENT_BUILD",
        )),
        JarSource::Embedded => Ok(AgentJar::from_bytes(EMBEDDED_JAR, AGENT_VERSION)),
    }
}

impl JarSource {
    /// The source as `status` names it: `embedded`, or the file's path.
    fn name(&self) -> String {
        match self {
            JarSource::File(path) => path.display().to_string(),
            JarSource::Embedded => "embedded".to_owned(),
        }
    }

    /// The build of `jar`, which came from this source: for the embedded
    /// jar, the digest its build recorded.
    fn build(&self, jar: &AgentJar) -> String {
        match self {
            JarSource::File(_) => jar.build(),
            JarSource::Embedded => EMBEDDED_BUILD.to_owned(),
        }
    }
}

/// The targets of the local adb server.
pub(crate) struct AndroidDevices {
    server: AdbServer,
    state_dir: PathBuf,
    /// The agents this daemon started, in the daemon's directory, opened on
    /// the first attach.
    records: OnceLock<Records>,
}

impl AndroidDevices {
    pub(crate) fn new(state_dir: &Path) -> AndroidDevices {
        AndroidDevices {
            server: AdbServer::from_env(),
            state_dir: state_dir.to_owned(),
            records: OnceLock::new(),
        }
    }

    /// The records, in a file per serial so a restarted daemon reuses the
    /// agent its predecessor left running; in memory when the directory
    /// cannot be written.
    fn records(&self) -> Records {
        self.records
            .get_or_init(|| {
                let dir = self.state_dir.join(RECORDS);
                Records::in_dir(&dir).unwrap_or_else(|failure| {
                    eprintln!(
                        "mobile-agent-daemon: cannot keep the Android agent records in {}: {failure}; they last as long as this daemon",
                        dir.display()
                    );
                    Records::in_memory()
                })
            })
            .clone()
    }
}

impl Driver for AndroidDevices {
    fn name(&self) -> &str {
        NAME
    }

    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError> {
        match self
            .server
            .clone()
            .with_io_timeout(LISTING_TIMEOUT)
            .targets()
        {
            Ok(targets) => Ok(targets.into_iter().map(target_info).collect()),
            // No adb server runs, so no Android target is connected;
            // doctor reports the server.
            Err(adb::Error::ServerUnavailable { .. }) => Ok(Vec::new()),
            Err(failure) => Err(DriverError::new(ADB_FAILED, failure.to_string())),
        }
    }

    fn attach(
        &self,
        target: &TargetInfo,
        options: &AttachOptions,
    ) -> Result<Attachment, DriverError> {
        ready(target)?;
        let device = self.server.device(&target.id);
        let source = jar_source(std::env::var_os(JAR_VAR).as_deref());
        let jar = agent_jar(&source)?;
        let build = source.build(&jar);
        let emulator = target.class == TargetClass::AndroidEmulator;
        let selection = select_paths(
            &target.id,
            emulator,
            &ControllerChoice::Auto,
            emulator::Options::default(),
        );
        // A forced command takes UI automation over from another holder,
        // which the new agent stops.
        let deploy = DeployOptions {
            force: options.force,
            ..DeployOptions::default()
        };
        let attached = Deployer::new(device.clone(), jar, self.records())
            .attach(&deploy)
            .map_err(deploy_error)?;
        let session = start_or_stop(attached, |attached| {
            Session::start_selected(attached, SessionOptions::default(), selection)
        })?;
        // Created before anything else can fail, so the agent is stopped
        // on a failure below as on a detach.
        let stop = StopAgent(session.clone());
        let mut display = AndroidScreen {
            device: device.clone(),
            scale: session.scale(),
        };
        // Read once here, so a display that cannot be read fails the attach.
        display.size()?;
        Ok(Attachment {
            backend: session.backend(),
            apps: Box::new(Apps {
                device,
                _stop: stop,
            }),
            display: Box::new(display),
            config: MirrorConfig::default(),
            capture: Some(Box::new(AgentScreenshots(session.clone()))),
            report: Some(Arc::new(SessionReport {
                session,
                jar: source.name(),
                build,
            })),
        })
    }
}

fn adb_error(failure: adb::Error) -> DriverError {
    DriverError::new(ADB_FAILED, failure.to_string())
}

/// Starts the session over `attached` with `start`; when it cannot start,
/// stops the agent the attach started or reused, so no agent outlives a
/// failed attach.
pub(crate) fn start_or_stop(
    attached: Attached,
    start: impl FnOnce(Attached) -> Result<Session, SessionError>,
) -> Result<Session, DriverError> {
    let deployer = attached.deployer().clone();
    match start(attached) {
        Ok(session) => Ok(session),
        Err(failure) => {
            // The session took the connection, so the agent is stopped from
            // its record: `shutdown` on a new connection, else a kill.
            let stopped = match deployer.stop_recorded(STOP_TIMEOUT) {
                Ok(_) => "the agent was stopped".to_owned(),
                Err(error) => format!("stopping the agent failed too: {error}"),
            };
            Err(DriverError::new(
                ADB_FAILED,
                format!("the agent session could not start ({failure}); {stopped}"),
            ))
        }
    }
}

/// Stops the agent when dropped: `shutdown`, then a kill past
/// [`STOP_TIMEOUT`], and its record forgotten.
struct StopAgent(Session);

impl Drop for StopAgent {
    fn drop(&mut self) {
        if let Err(failure) = self.0.stop(STOP_TIMEOUT) {
            eprintln!(
                "mobile-agent-daemon: stopping the agent on {}: {failure}",
                self.0.agent().serial
            );
        }
    }
}

struct Apps {
    device: Device,
    _stop: StopAgent,
}

impl AppControl for Apps {
    fn launch(&mut self, app: &str, launch: &Launch) -> Result<(), DriverError> {
        let resolved = self
            .device
            .shell(&format!(
                "cmd package resolve-activity --brief -a android.intent.action.MAIN \
                 -c android.intent.category.LAUNCHER {}",
                quote(app)
            ))
            .map_err(adb_error)?;
        let Some(component) = launcher_activity(&resolved.stdout_text()) else {
            let installed = self
                .device
                .shell(&format!("pm path {}", quote(app)))
                .map_err(adb_error)?
                .success();
            let serial = self.device.serial();
            return Err(DriverError::new(
                APP_NOT_FOUND,
                if installed {
                    format!("{app} has no launcher activity on {serial} to open")
                } else {
                    format!(
                        "{app} is not installed on {serial}; list the packages with adb shell pm list packages"
                    )
                },
            ));
        };
        let started = self
            .device
            .shell(&start_command(&component, launch))
            .map_err(adb_error)?;
        let output = format!("{}{}", started.stdout_text(), started.stderr_text());
        match start_failure(&output) {
            Some(failure) => Err(DriverError::new(
                LAUNCH_FAILED,
                format!("am start could not open {app}: {failure}"),
            )),
            None => Ok(()),
        }
    }

    fn terminate(&mut self, app: &str) -> Result<(), DriverError> {
        self.device
            .shell(&format!("am force-stop {}", quote(app)))
            .map_err(adb_error)?;
        Ok(())
    }
}

/// Screenshots through the agent, which takes them upright in every
/// display orientation.
struct AgentScreenshots(Session);

impl Capture for AgentScreenshots {
    fn screenshot(&mut self, request: &CaptureRequest) -> Result<Image, DriverError> {
        let shot = self.0.screenshot(ScreenshotParams {
            format: Some(ImageFormat::JPEG),
            quality: Some(request.quality),
            max_dimension: Some(request.max_dimension),
        })?;
        Ok(Image {
            format: if shot.format == ImageFormat::PNG {
                "png"
            } else {
                "jpeg"
            },
            width: shot.width,
            height: shot.height,
            data: shot.data,
        })
    }
}

struct SessionReport {
    session: Session,
    /// Where the deployed jar came from: `embedded`, or a path.
    jar: String,
    /// Its build.
    build: String,
}

impl Report for SessionReport {
    fn report(&self) -> TargetReport {
        let mut report = status_report(&self.session.status());
        report
            .data
            .insert("agentJar".into(), self.jar.clone().into());
        report
            .data
            .insert("agentBuild".into(), self.build.clone().into());
        report
            .lines
            .push(format!("agent jar: {}, build {}", self.jar, self.build));
        report
    }
}

/// `text` quoted for the device's shell.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// An adb target as `devices` lists it, in whatever state the adb server
/// reports it, with the advice for a state that keeps it from automation.
pub(crate) fn target_info(target: adb::Target) -> TargetInfo {
    let advice = target.advice();
    TargetInfo {
        platform: Platform::Android,
        class: if target.emulator {
            TargetClass::AndroidEmulator
        } else {
            TargetClass::AndroidDevice
        },
        name: target
            .model
            .clone()
            .unwrap_or_else(|| target.serial.clone()),
        os: target.api_level.map(|level| format!("Android API {level}")),
        state: state_name(&target.state),
        advice,
        driver: NAME.to_owned(),
        model: target.model,
        id: target.serial,
    }
}

/// The adb server's name for a state: `device`, `offline`, `unauthorized`,
/// `authorizing`, `connecting`, `no permissions`, or the mode it names.
pub(crate) fn state_name(state: &adb::State) -> String {
    match state {
        adb::State::Device => "device",
        adb::State::Offline => "offline",
        adb::State::Unauthorized => "unauthorized",
        adb::State::Authorizing => "authorizing",
        adb::State::Connecting => "connecting",
        adb::State::NoPermissions(_) => "no permissions",
        adb::State::Other(mode) => mode,
    }
    .to_owned()
}

/// Refuses a target whose state keeps it from automation, with its advice.
pub(crate) fn ready(target: &TargetInfo) -> Result<(), DriverError> {
    if target.state == "device" {
        return Ok(());
    }
    let advice = target
        .advice
        .as_deref()
        .unwrap_or("wait until adb lists it as device");
    Err(DriverError::new(
        TARGET_NOT_READY,
        format!("{} is {}: {advice}", target.id, target.state),
    ))
}

/// The screen in points from `wm size`, the override size first, at
/// `scale` pixels per point.
pub(crate) fn display_size(wm_size: &str, scale: f64) -> Option<Size> {
    let size = |label: &str| {
        let text = wm_size
            .lines()
            .find_map(|line| line.trim().strip_prefix(label))?;
        let (width, height) = text.trim().split_once('x')?;
        Some((
            width.trim().parse::<f64>().ok()?,
            height.trim().parse::<f64>().ok()?,
        ))
    };
    let (width, height) = size("Override size:").or_else(|| size("Physical size:"))?;
    (scale > 0.0).then(|| Size {
        width: width / scale,
        height: height / scale,
    })
}

/// The display's size in points now, from the `cur=WIDTHxHEIGHT` of
/// `dumpsys window displays`, which follows the current rotation, at
/// `scale` pixels per point.
pub(crate) fn current_display(dumpsys: &str, scale: f64) -> Option<Size> {
    let (width, height) = dumpsys
        .split_whitespace()
        .find_map(|word| word.strip_prefix("cur="))?
        .split_once('x')?;
    let (width, height) = (width.parse::<f64>().ok()?, height.parse::<f64>().ok()?);
    (scale > 0.0 && width > 0.0 && height > 0.0).then(|| Size {
        width: width / scale,
        height: height / scale,
    })
}

/// An Android display, read as it is now: its size follows the rotation.
struct AndroidScreen {
    device: Device,
    /// Pixels per point, which a rotation leaves alone.
    scale: f64,
}

impl Screen for AndroidScreen {
    fn size(&mut self) -> Result<Size, DriverError> {
        let shown = self
            .device
            .shell("dumpsys window displays")
            .map_err(adb_error)?;
        if let Some(size) = current_display(&shown.stdout_text(), self.scale) {
            return Ok(size);
        }
        // A build without the cur= size: the natural orientation's.
        let size = self.device.shell("wm size").map_err(adb_error)?;
        display_size(&size.stdout_text(), self.scale).ok_or_else(|| {
            DriverError::new(
                ADB_FAILED,
                format!(
                    "cannot read the display size of {} from dumpsys window displays or wm size: {}",
                    self.device.serial(),
                    size.stdout_text().trim()
                ),
            )
        })
    }
}

/// The launcher activity `cmd package resolve-activity --brief` printed,
/// as `package/activity`; `None` when it found none.
pub(crate) fn launcher_activity(resolved: &str) -> Option<String> {
    let line = resolved
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())?;
    (line.contains('/') && !line.contains(char::is_whitespace)).then(|| line.to_owned())
}

/// The `am start` command that launches `component` as the launcher does,
/// waiting for its first frame: stopped first when `launch.relaunch` is
/// set, with each environment entry as a string extra of the same name.
pub(crate) fn start_command(component: &str, launch: &Launch) -> String {
    let mut command = String::from("am start -W");
    if launch.relaunch {
        command.push_str(" -S");
    }
    command.push_str(" -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -n ");
    command.push_str(&quote(component));
    for (key, value) in &launch.env {
        command.push_str(&format!(" --es {} {}", quote(key), quote(value)));
    }
    command
}

/// The error `am start` printed, when it failed.
pub(crate) fn start_failure(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("Error"))
        .map(str::to_owned)
}

/// What `status` shows about an attached Android target: the agent, the
/// active path of each kind of input and output, and what failed since.
pub(crate) fn status_report(status: &Status) -> TargetReport {
    let paths = &status.paths;
    let mut data = Map::new();
    let mut put = |key: &str, value: Value| {
        data.insert(key.to_owned(), value);
    };
    put("agent", status.agent.clone().into());
    put("protocol", status.protocol.into());
    put("pid", status.pid.into());
    put("socket", status.socket.clone().into());
    put("reused", status.reused.into());
    put("input", paths.input.to_string().into());
    put("frames", paths.frames.map(|path| path.to_string()).into());
    put("screenshots", paths.screenshots.to_string().into());
    put("keys", paths.keys.to_string().into());
    put("endpoint", paths.endpoint.clone().into());
    put("keyboard", paths.keyboard.into());
    put("warning", paths.warning.clone().into());
    put(
        "inputs",
        json!({"controller": status.inputs.0, "agent": status.inputs.1}),
    );
    put("framesReceived", status.frames_received.into());
    put("controllerError", status.controller_error.clone().into());
    put("frameError", status.frame_error.clone().into());
    put("disconnected", status.disconnected.clone().into());
    TargetReport {
        lines: status.lines(),
        data,
    }
}

/// A deployment failure with a stable code: another holder of UI
/// automation, the adb server, the agent, or its record.
pub(crate) fn deploy_error(error: DeployError) -> DriverError {
    if let DeployError::Conflict(conflict) = &error {
        let who = conflict.holder.as_ref().map_or_else(
            || "a process the device does not name".to_owned(),
            |h| format!("likely {h}"),
        );
        return DriverError::new(
            AUTOMATION_CONFLICT,
            format!(
                "another process holds UI automation, {who}: {}. Run the command again with \
                 --force to take it over, which stops that holder",
                conflict.detail
            ),
        );
    }
    let code = match &error {
        DeployError::Conflict(_) => AUTOMATION_CONFLICT,
        DeployError::Adb(_) => ADB_FAILED,
        DeployError::Exited { .. }
        | DeployError::NoReadyLine { .. }
        | DeployError::Handshake { .. } => AGENT_FAILED,
        DeployError::WrongVersion { .. } => AGENT_MISMATCH,
        DeployError::Records(_) => IO_ERROR,
    };
    DriverError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use android::agent::Conflict;
    use android::session::{Path, Paths};
    use serde_json::Value;

    use crate::driver::{Platform, TargetClass};

    fn adb_target(serial: &str, state: adb::State, emulator: bool) -> adb::Target {
        let online = state == adb::State::Device;
        adb::Target {
            serial: serial.into(),
            state,
            model: online.then(|| "sdk_gphone64_arm64".into()),
            api_level: online.then_some(36),
            emulator,
            transport_id: Some(6),
        }
    }

    /// Android-driver spec, Target discovery: every state is listed, an
    /// unusable one with what to do about it.
    #[test]
    fn adb_targets_are_listed_in_every_state_with_advice_for_the_unusable_ones() {
        let emulator = target_info(adb_target("emulator-5554", adb::State::Device, true));
        assert_eq!(emulator.id, "emulator-5554");
        assert_eq!(emulator.platform, Platform::Android);
        assert_eq!(emulator.class, TargetClass::AndroidEmulator);
        assert_eq!(emulator.name, "sdk_gphone64_arm64");
        assert_eq!(emulator.model.as_deref(), Some("sdk_gphone64_arm64"));
        assert_eq!(emulator.os.as_deref(), Some("Android API 36"));
        assert_eq!(emulator.state, "device");
        assert_eq!(emulator.advice, None);
        assert_eq!(emulator.driver, NAME);
        assert!(ready(&emulator).is_ok());

        let phone = target_info(adb_target("R58M123", adb::State::Unauthorized, false));
        assert_eq!(phone.class, TargetClass::AndroidDevice);
        assert_eq!(
            phone.name, "R58M123",
            "a target without a model shows its serial"
        );
        assert_eq!(phone.state, "unauthorized");
        let advice = phone.advice.clone().expect("advice");
        assert!(advice.contains("Allow USB debugging"), "{advice}");
        let refused = ready(&phone).unwrap_err();
        assert_eq!(refused.code, "target_not_ready");
        assert!(
            refused.message.contains("unauthorized") && refused.message.contains(&advice),
            "{refused:?}"
        );

        assert_eq!(state_name(&adb::State::Offline), "offline");
        assert_eq!(
            state_name(&adb::State::NoPermissions("udev".into())),
            "no permissions"
        );
        assert_eq!(
            state_name(&adb::State::Other("recovery".into())),
            "recovery"
        );
    }

    #[test]
    fn the_display_is_the_override_size_or_the_physical_one_in_points() {
        let size = display_size("Physical size: 1080x2400\n", 2.625).unwrap();
        assert!((size.width - 411.428_571).abs() < 1e-3, "{size:?}");
        assert!((size.height - 914.285_714).abs() < 1e-3, "{size:?}");
        let over =
            display_size("Physical size: 1080x2400\nOverride size: 720x1600\n", 2.0).unwrap();
        assert_eq!((over.width, over.height), (360.0, 800.0));
        assert_eq!(display_size("wm: not found", 2.625), None);
    }

    /// Task 8.11: the display's size follows the current rotation.
    #[test]
    fn the_current_display_is_read_in_the_current_rotation() {
        let portrait = "  Display: mDisplayId=0 rotation 0\n    init=1080x2400 420dpi \
                        mMinSizeOfResizeableTaskDp=220 cur=1080x2400 app=1080x2400 \
                        rng=1080x1080-2400x2400\n";
        let size = current_display(portrait, 2.625).unwrap();
        assert!((size.width - 411.428_571).abs() < 1e-3, "{size:?}");
        assert!((size.height - 914.285_714).abs() < 1e-3, "{size:?}");
        let landscape =
            portrait.replace("cur=1080x2400 app=1080x2400", "cur=2400x1080 app=2400x1017");
        let size = current_display(&landscape, 2.625).unwrap();
        assert!(size.width > size.height, "{size:?}");
        assert!((size.width - 914.285_714).abs() < 1e-3, "{size:?}");
        assert_eq!(current_display("init=1080x2400 420dpi", 2.625), None);
        assert_eq!(current_display("", 2.625), None);
    }

    #[test]
    fn the_launcher_activity_is_what_resolve_activity_names() {
        let resolved = "priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false\n\
                        dev.mobileagent.fixture.compose/.MainActivity\n";
        assert_eq!(
            launcher_activity(resolved).as_deref(),
            Some("dev.mobileagent.fixture.compose/.MainActivity")
        );
        assert_eq!(launcher_activity("No activity found\n"), None);
        assert_eq!(launcher_activity(""), None);
    }

    /// Fixture contract, Launch options: on Android the initial tab is the
    /// string extra `initial_tab`, which `--env initial_tab=big` passes.
    #[test]
    fn a_launch_passes_its_environment_as_quoted_string_extras() {
        let component = "dev.mobileagent.fixture.compose/.MainActivity";
        let launch = Launch {
            relaunch: false,
            env: vec![("initial_tab".into(), "big".into())],
        };
        assert_eq!(
            start_command(component, &launch),
            "am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER \
             -n 'dev.mobileagent.fixture.compose/.MainActivity' --es 'initial_tab' 'big'"
        );
        let relaunch = Launch {
            relaunch: true,
            env: vec![("name".into(), "it's; rm -rf /".into())],
        };
        let command = start_command(component, &relaunch);
        assert!(command.starts_with("am start -W -S "), "{command}");
        assert!(
            command.ends_with("--es 'name' 'it'\\''s; rm -rf /'"),
            "{command}"
        );
    }

    #[test]
    fn a_failed_start_is_told_from_one_that_brought_the_app_to_the_front() {
        let failed = "Starting: Intent { cmp=x/.Main }\n\
                      Error: Activity class {x/x.Main} does not exist.\n";
        assert_eq!(
            start_failure(failed).as_deref(),
            Some("Error: Activity class {x/x.Main} does not exist.")
        );
        let front = "Starting: Intent { cmp=x/.Main }\nWarning: Activity not started, intent has \
                     been delivered to currently running top-most instance.\nStatus: ok\n\
                     LaunchState: UNKNOWN (0)\nComplete\n";
        assert_eq!(start_failure(front), None);
    }

    fn status(paths: Paths) -> Status {
        Status {
            serial: "emulator-5554".into(),
            agent: "mobile-agent-android 0.1.0".into(),
            protocol: 1,
            pid: 812,
            socket: "mobile-agent-0.1.0-p1-00ff".into(),
            reused: false,
            paths,
            inputs: (3, 1),
            frames_received: 40,
            controller_error: None,
            frame_error: None,
            disconnected: None,
        }
    }

    /// Android-driver spec, Emulator fast path: the status names the active
    /// path, in JSON and in the lines.
    #[test]
    fn the_status_names_the_input_path_and_the_agent() {
        let report = status_report(&status(Paths {
            emulator: true,
            input: Path::Controller,
            frames: Some(Path::Controller),
            screenshots: Path::Agent,
            keys: Path::Agent,
            keyboard: Some(false),
            endpoint: Some("http://127.0.0.1:8554".into()),
            warning: None,
        }));
        let data = Value::Object(report.data.clone());
        assert_eq!(data["input"], "emulator controller", "{data}");
        assert_eq!(data["frames"], "emulator controller", "{data}");
        assert_eq!(data["screenshots"], "agent", "{data}");
        assert_eq!(data["keys"], "agent", "{data}");
        assert_eq!(data["endpoint"], "http://127.0.0.1:8554", "{data}");
        assert_eq!(data["agent"], "mobile-agent-android 0.1.0", "{data}");
        assert_eq!(data["pid"], 812, "{data}");
        assert_eq!(data["socket"], "mobile-agent-0.1.0-p1-00ff", "{data}");
        assert_eq!(data["reused"], false, "{data}");
        assert_eq!(data["inputs"]["controller"], 3, "{data}");
        assert_eq!(data["framesReceived"], 40, "{data}");
        assert!(
            report
                .lines
                .iter()
                .any(|line| line == "input: emulator controller at http://127.0.0.1:8554 (taps, long presses and swipes)"),
            "{:?}",
            report.lines
        );

        let fallback = status_report(&status(Paths::agent(true, Some("start with -grpc".into()))));
        let data = Value::Object(fallback.data.clone());
        assert_eq!(data["input"], "agent", "{data}");
        assert_eq!(data["frames"], Value::Null, "{data}");
        assert_eq!(data["warning"], "start with -grpc", "{data}");
    }

    /// Task 8.11: an installed daemon needs neither the source tree nor
    /// `MOBILE_AGENT_JAR`: the jar `agent-android/build.sh` builds is in the
    /// binary, with its digest.
    #[test]
    fn the_daemon_embeds_the_agent_jar_with_its_digest() {
        if EMBEDDED_JAR.is_empty() {
            // Built with SKIP_AGENT_BUILD=1, as the macOS CI job does.
            let refused = agent_jar(&JarSource::Embedded).unwrap_err();
            assert!(refused.message.contains("SKIP_AGENT_BUILD"), "{refused:?}");
            return;
        }
        assert_eq!(&EMBEDDED_JAR[..4], b"PK\x03\x04", "a jar");
        assert!(
            EMBEDDED_JAR.windows(11).any(|w| w == b"classes.dex"),
            "holding classes.dex"
        );
        let jar = agent_jar(&JarSource::Embedded).unwrap();
        assert_eq!(jar.build(), EMBEDDED_BUILD, "the digest the build recorded");
        assert_eq!(jar.version(), android::agent::AGENT_VERSION);
        assert_eq!(jar.path(), None, "read from no file");
    }

    #[test]
    fn the_jar_comes_from_mobile_agent_jar_else_from_the_binary() {
        use std::ffi::OsStr;
        assert_eq!(jar_source(None), JarSource::Embedded);
        assert_eq!(jar_source(Some(OsStr::new(""))), JarSource::Embedded);
        assert_eq!(
            jar_source(Some(OsStr::new("/opt/agent.jar"))),
            JarSource::File("/opt/agent.jar".into())
        );
        let missing = agent_jar(&JarSource::File("/nonexistent/agent.jar".into())).unwrap_err();
        assert_eq!(missing.code, "agent_failed");
        assert!(
            missing.message.contains("/nonexistent/agent.jar"),
            "{missing:?}"
        );
    }

    /// Task 8.11, on the device `MOBILE_AGENT_ANDROID_SERIAL` names: an
    /// agent whose session cannot start is stopped and forgotten, not left
    /// to its idle exit.
    #[test]
    fn an_agent_whose_session_cannot_start_is_stopped() {
        let Some(serial) = std::env::var("MOBILE_AGENT_ANDROID_SERIAL")
            .ok()
            .filter(|s| !s.trim().is_empty())
        else {
            eprintln!(
                "skipped an_agent_whose_session_cannot_start_is_stopped: set \
                 MOBILE_AGENT_ANDROID_SERIAL to run it on a device"
            );
            return;
        };
        let device = AdbServer::from_env().device(serial.trim());
        let agents = || {
            device
                .shell(android::agent::PS_COMMAND)
                .map(|out| android::agent::parse_agents(&out.stdout_text()))
                .unwrap_or_default()
        };
        let quiet = std::time::Instant::now();
        while !agents().is_empty() {
            assert!(
                quiet.elapsed() < Duration::from_secs(60),
                "the shared device kept an agent for a minute: {:?}",
                agents()
            );
            std::thread::sleep(Duration::from_millis(250));
        }
        let dir = std::env::temp_dir().join(format!("daemon-stop-{}", std::process::id()));
        let records = Records::in_dir(&dir).unwrap();
        let attached = Deployer::new(
            device.clone(),
            agent_jar(&JarSource::Embedded).expect("an agent jar"),
            records.clone(),
        )
        .attach(&DeployOptions {
            idle_exit: Duration::from_secs(10),
            ..DeployOptions::default()
        })
        .expect("the agent starts");
        let (pid, socket) = (attached.agent.pid, attached.agent.socket.clone());
        assert!(agents().iter().any(|a| a.pid == pid));

        let failed = start_or_stop(attached, |_| {
            Err(SessionError::Density("injected for the test".into()))
        })
        .expect_err("the session cannot start");
        assert!(
            failed.message.contains("injected for the test"),
            "{failed:?}"
        );
        let gone = std::time::Instant::now();
        while agents().iter().any(|a| a.pid == pid && a.socket == socket) {
            assert!(
                gone.elapsed() < Duration::from_secs(5),
                "the agent is still there: {:?}",
                agents()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(records.get(serial.trim()).is_none(), "its record is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deployment_failures_keep_a_code_that_says_whose_they_are() {
        let conflict = deploy_error(DeployError::Conflict(Conflict {
            holder: None,
            detail: "uiautomator events runs".into(),
        }));
        assert_eq!(conflict.code, "automation_conflict");
        assert!(
            conflict.message.contains("uiautomator events"),
            "{conflict:?}"
        );
        assert!(
            conflict.message.contains("--force"),
            "the conflict says how to take UI automation over: {conflict:?}"
        );
        let silent = deploy_error(DeployError::NoReadyLine {
            waited: std::time::Duration::from_secs(20),
            output: "nothing".into(),
        });
        assert_eq!(silent.code, "agent_failed");
        let records = deploy_error(DeployError::Records(std::io::Error::other("disk full")));
        assert_eq!(records.code, "io_error");
    }
}
