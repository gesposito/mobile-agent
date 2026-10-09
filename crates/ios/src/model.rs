//! The values the driver takes and returns. Nothing here depends on the
//! `idevice` crate, so these types exist on every platform.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::readiness::Readiness;

/// The environment variable that names the development team the runner is
/// signed for. It stays in the environment, never in a tracked file.
pub const TEAM_VARIABLE: &str = "MOBILE_AGENT_IOS_TEAM";

/// How usbmuxd reaches a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    Usb,
    /// Wi-Fi sync: usbmuxd lists a phone with it on a second time, on the
    /// network. This driver uses USB only (design D18).
    Network,
    /// A connection type this driver does not know, as usbmuxd named it.
    Other(String),
}

/// One entry of usbmuxd's device listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuxEntry {
    pub udid: String,
    /// usbmuxd's number for the device on this connection, which its
    /// `Connect` takes.
    pub device_id: u32,
    pub connection: Connection,
}

/// Whether this Mac holds a pairing record the device accepts: what a
/// lockdown session with it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    /// A lockdown session started with the host's pairing record.
    Trusted,
    /// usbmuxd holds no pairing record for the device: it never trusted
    /// this Mac, or its record was deleted.
    NotPaired,
    /// The device refused the host's pairing record (`InvalidHostID`): its
    /// trust settings were reset, or it was paired with another record.
    Refused,
    /// Lockdown did not answer, or answered something else; the reason.
    Unreachable(String),
}

/// The developer disk image, as the image mounter's `CopyDevices` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskImage {
    /// A developer disk image is mounted: its `DiskImageType` (`Personalized`
    /// from iOS 17, `Developer` before) and mount path.
    Mounted {
        image_type: String,
        mount_path: Option<String>,
    },
    /// The image mounter lists no developer disk image.
    NotMounted,
    /// The image mounter could not be asked; the reason.
    Unreadable(String),
}

/// What the driver read from a device to tell its readiness. A value is
/// `None` where an earlier check stopped the reading: Developer Mode and the
/// lock need a lockdown session, the disk image Developer Mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub trust: Trust,
    /// Lockdown's `DeveloperModeStatus` in the `com.apple.security.mac.amfi`
    /// domain.
    pub developer_mode: Option<bool>,
    pub disk_image: Option<DiskImage>,
    /// Lockdown's `PasswordProtected`, which follows the lock on iOS 26.6.2
    /// (spike S9). CoreDevice's `getlockstate` answers that it is not
    /// implemented there, so the driver never asks it.
    pub password_protected: Option<bool>,
}

impl Probe {
    /// A device whose lockdown session did not start: nothing past the trust
    /// was read.
    pub fn untrusted(trust: Trust) -> Probe {
        Probe {
            trust,
            developer_mode: None,
            disk_image: None,
            password_protected: None,
        }
    }
}

/// One iOS device attached over USB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The device identifier, which every command takes; names are not
    /// unique, so the driver never addresses a device by name.
    pub udid: String,
    /// Lockdown's `DeviceName`, as the owner named the phone.
    pub name: Option<String>,
    /// Lockdown's `ProductType`, such as `iPhone14,6`.
    pub model: Option<String>,
    /// Lockdown's `ProductVersion`, such as `26.6.2`.
    pub ios_version: Option<String>,
    /// Lockdown's `BuildVersion`, such as `23G90`.
    pub build: Option<String>,
    pub probe: Probe,
    /// What stands between the device and a session, or `Ready`.
    pub readiness: Readiness,
}

/// The CoreDevice tunnel to a device and the services its RSD handshake
/// listed (design D18).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelStatus {
    /// Tunnels opened to the device so far: one, unless one dropped and the
    /// driver opened another.
    pub opens: u32,
    /// How long the last one took to open, `CoreDeviceProxy` and the RSD
    /// handshake together: 60 ms at p50 in spike S9.
    pub opened_in: Duration,
    /// Every service the handshake listed, by name, sorted.
    pub services: Vec<String>,
    /// The services the driver needs that the handshake did not list.
    pub missing: Vec<&'static str>,
}

/// How the driver mounted the developer disk image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountPath {
    /// The image mounter's personalized mount through `idevice`, with the
    /// ticket from Apple's signing server.
    Idevice,
    /// `xcrun devicectl device info ddiServices`, which mounts the image
    /// the host's CoreDevice holds; the fallback, since `idevice`'s mount
    /// hung once in spike S10.
    Devicectl,
}

impl fmt::Display for MountPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MountPath::Idevice => "idevice's personalized mount",
            MountPath::Devicectl => "devicectl device info ddiServices",
        })
    }
}

/// One try at mounting the developer disk image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountAttempt {
    pub path: MountPath,
    pub elapsed: Duration,
    /// `None` when the path reported success; why it failed otherwise. A
    /// path that reported success still counts only once the image mounter
    /// lists the image.
    pub failure: Option<String>,
}

/// What `connect` found of the developer disk image and did about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountReport {
    /// The image as the image mounter lists it once the attempts are over.
    pub image: DiskImage,
    /// The mounts tried, none when the image was already mounted.
    pub attempts: Vec<MountAttempt>,
}

/// What `connect` found and opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connected {
    /// The device as its probe read it after any mount.
    pub device: DeviceInfo,
    pub tunnel: TunnelStatus,
    pub disk_image: MountReport,
}

/// What the driver holds for a device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetStatus {
    /// The tunnel, once `connect` opened one and until it is reset.
    pub tunnel: Option<TunnelStatus>,
    /// The disk image as the last `connect` found it.
    pub disk_image: Option<MountReport>,
    /// The service connections kept open over the tunnel, by RSD name.
    pub services_open: Vec<&'static str>,
    /// The runner's session, from `attach` until `release`.
    pub runner: Option<RunnerStatus>,
}

/// The bundle identifier of the runner app (design D20).
pub const RUNNER_BUNDLE_ID: &str = "dev.mobileagent.runner";
/// The name the runner's hello puts before its version.
pub const RUNNER_NAME: &str = "mobile-agent-ios-runner";
/// The loopback port the runner listens on, which the host reaches through
/// usbmuxd's `Connect`.
pub const RUNNER_PORT: u16 = 27183;

/// The runner a session starts: the installed app, what its hello must
/// say, and the built bundle that replaces a runner of another version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerSpec {
    pub bundle_id: String,
    /// The name its hello puts before its version.
    pub name: String,
    /// The version its hello must name: that of the bundle built beside the
    /// daemon.
    pub version: String,
    /// The built bundle, installed when the runner is missing or says hello
    /// with another version; without it either fails.
    pub bundle: Option<PathBuf>,
    pub port: u16,
}

impl RunnerSpec {
    /// The runner `dev.mobileagent.runner` of `version` on port 27183, with
    /// no bundle to install.
    pub fn new(version: impl Into<String>) -> RunnerSpec {
        RunnerSpec {
            bundle_id: RUNNER_BUNDLE_ID.to_owned(),
            name: RUNNER_NAME.to_owned(),
            version: version.into(),
            bundle: None,
            port: RUNNER_PORT,
        }
    }

    /// The bundle built beside the daemon, installed when needed.
    pub fn with_bundle(mut self, bundle: impl Into<PathBuf>) -> RunnerSpec {
        self.bundle = Some(bundle.into());
        self
    }

    /// The agent its hello must name: its name and version.
    pub fn agent(&self) -> String {
        format!("{} {}", self.name, self.version)
    }
}

/// How long an attach may take by default: the runner's hello comes 0.8 s
/// after the start at p50 (S10), but XCTest waits 60 s for an approval of
/// "Enable UI Automation", and the test manager stopped answering its
/// handshake for a minute or two after runners were ended mid-start.
pub const ATTACH_TIMEOUT: Duration = Duration::from_secs(150);

/// How a runner session starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachOptions {
    pub runner: RunnerSpec,
    /// The application under test the test manager names to the runner, or
    /// none: the runner reads whatever is in front either way.
    pub target_app: Option<String>,
    /// The longest the start may take, every retry included.
    pub timeout: Duration,
}

impl AttachOptions {
    pub fn new(runner: RunnerSpec) -> AttachOptions {
        AttachOptions {
            runner,
            target_app: None,
            timeout: ATTACH_TIMEOUT,
        }
    }
}

/// Where a runner session is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunnerState {
    /// The test manager is starting it; what the start waits for.
    Starting { step: String },
    /// XCTest waits in "enabling automation mode" for the device owner to
    /// approve "Enable UI Automation" (readiness `automation_not_enabled`).
    AwaitingApproval,
    /// It said hello with the expected version and serves.
    Ready,
    /// It died, and the driver starts it again; why it died.
    Restarting { cause: String },
    /// It cannot serve until a new attach: the error's code and message.
    Failed { code: &'static str, message: String },
    /// It was released.
    Stopped,
}

impl RunnerState {
    /// A stable name for the state.
    pub fn name(&self) -> &'static str {
        match self {
            RunnerState::Starting { .. } => "starting",
            RunnerState::AwaitingApproval => "awaiting_approval",
            RunnerState::Ready => "ready",
            RunnerState::Restarting { .. } => "restarting",
            RunnerState::Failed { .. } => "failed",
            RunnerState::Stopped => "stopped",
        }
    }
}

/// What the driver knows of a target's runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerStatus {
    pub state: RunnerState,
    /// The agent its hello named, its name and version, once it said hello.
    pub agent: Option<String>,
    /// What its hello advertised.
    pub capabilities: protocol::Capabilities,
    /// Times it died and was started again in this session.
    pub restarts: u32,
    /// Starts repeated because the test manager did not answer its
    /// handshake.
    pub handshake_retries: u32,
    /// Whether the built bundle was installed: the runner was missing or
    /// said hello with another version.
    pub replaced: bool,
    /// Runner processes of an earlier session ended before a start, by pid.
    pub leftovers: Vec<u32>,
    /// From the last start's request to the runner's hello.
    pub started_in: Option<Duration>,
    /// Why it died last.
    pub last_death: Option<String>,
}

impl RunnerStatus {
    /// A session whose start is under way.
    pub fn starting(step: impl Into<String>) -> RunnerStatus {
        RunnerStatus {
            state: RunnerState::Starting { step: step.into() },
            agent: None,
            capabilities: protocol::Capabilities::NONE,
            restarts: 0,
            handshake_retries: 0,
            replaced: false,
            leftovers: Vec::new(),
            started_in: None,
            last_death: None,
        }
    }
}

/// A started runner session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    pub runner: RunnerStatus,
    /// From the request to the hello: the device's readiness, ending
    /// leftover runners, any install and the start.
    pub elapsed: Duration,
}

/// What `release` ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Released {
    /// Runner processes that were still running and were signalled, by pid.
    pub ended: Vec<u32>,
    pub elapsed: Duration,
}

/// Arguments and environment for a launch (design D19).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    pub arguments: Vec<String>,
    /// The process's environment, as given.
    pub environment: Vec<(String, String)>,
    /// End a running instance first (DVT's `KillExisting`), so the launch
    /// starts a new process that sees these arguments and this environment.
    pub terminate_running: bool,
}

impl LaunchOptions {
    pub fn new() -> LaunchOptions {
        LaunchOptions::default()
    }

    /// Adds one argument.
    pub fn argument(mut self, argument: impl Into<String>) -> LaunchOptions {
        self.arguments.push(argument.into());
        self
    }

    /// Adds one environment variable.
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> LaunchOptions {
        self.environment.push((name.into(), value.into()));
        self
    }

    /// Ends a running instance before launching.
    pub fn terminating_running(mut self) -> LaunchOptions {
        self.terminate_running = true;
        self
    }
}

/// A launched application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Launched {
    /// The process identifier DVT's reply carries.
    pub pid: u32,
    /// From the request, the lock's read included, to the reply.
    pub elapsed: Duration,
}

/// An installed application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// The bundle identifier its `Info.plist` names.
    pub bundle_id: String,
    /// From the request to the installation proxy's completion: the upload
    /// through AFC and the install.
    pub elapsed: Duration,
}

/// What `terminate` ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminated {
    /// The processes signalled, none when the application was not running.
    pub pids: Vec<u32>,
    /// From the request until the processes left DVT's process list.
    pub elapsed: Duration,
}
