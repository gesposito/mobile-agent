//! `doctor`: checks of the host's toolchains and of the daemon, each with a
//! status (ok, warn, fail or skip), a message and, when something can be
//! done, a hint. Only a failed check fails doctor; a missing toolchain for
//! one platform is a warning, since the other platform may be all a user
//! drives. The checks of a device driver the run's `MOBILE_AGENT_DRIVERS`
//! leaves out ask the host nothing and are skipped as not consulted, so a
//! run that names the Simulator's driver alone runs no adb. The physical
//! iPhone driver registers only when the variable names `ios`, so its
//! checks are consulted only then: the signing identity and the team, the
//! runner bundle the daemon installs, and each attached iPhone's readiness
//! as the daemon's `devices` lists it, with the step the daemon names.
//!
//! The checks ask the host through a [`Probe`], which tests script. The
//! Xcode compatibility table comes from the daemon, whose drivers supply
//! its rows ([`System::daemon`]), the Simulator's and the iPhone's, so the
//! CLI links no driver code.

use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::client::{self, Endpoint};
use crate::request;

/// How long a probed command may run.
const COMMAND_LIMIT: Duration = Duration::from_secs(15);

/// The adb server's port unless `ANDROID_ADB_SERVER_PORT` names another.
const ADB_PORT: u16 = 5037;

/// Emulator gRPC ports follow the console port by this much, as the
/// emulator's own `-grpc` examples and spike S7 used them.
const GRPC_OFFSET: u16 = 3000;

/// The variable that names the device drivers a daemon registers; a daemon
/// the CLI starts inherits it.
const DRIVERS: &str = "MOBILE_AGENT_DRIVERS";

/// The variable that names the team the iPhone runner is signed for.
const TEAM: &str = "MOBILE_AGENT_IOS_TEAM";

/// A device driver as [`DRIVERS`] names it.
struct DeviceDriver {
    /// The names the variable gives it.
    names: &'static [&'static str],
    /// Its name in doctor's messages.
    title: &'static str,
    /// Whether it registers only when the variable names it.
    opt_in: bool,
}

const SIMULATOR: DeviceDriver = DeviceDriver {
    names: &["simulator"],
    title: "Simulator",
    opt_in: false,
};

const IOS_DEVICE: DeviceDriver = DeviceDriver {
    names: &["ios"],
    title: "physical iOS",
    opt_in: true,
};

const ANDROID: DeviceDriver = DeviceDriver {
    names: &["android"],
    title: "Android",
    opt_in: false,
};

/// What a command printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// What the checks ask of the host.
pub trait Probe {
    /// `macos` or `linux`.
    fn os(&self) -> &str;
    fn env(&self, key: &str) -> Option<String>;
    /// Runs a program; `None` when it cannot be started.
    fn run(&self, program: &str, args: &[&str]) -> Option<Output>;
    fn is_file(&self, path: &Path) -> bool;
    /// The regular files in `dir` with their text.
    fn files(&self, dir: &Path) -> Vec<(PathBuf, String)>;
    /// The protocol version of the adb server on `port`.
    fn adb_server(&self, port: u16) -> Result<u32, String>;
    /// The daemon's `doctor` data, starting the daemon if needed.
    fn daemon(&self) -> Result<Value, String>;
    /// The daemon's `devices` data: every target its drivers list now.
    fn devices(&self) -> Result<Value, String>;
}

/// A check's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Skip => "skip",
        }
    }

    fn mark(self) -> &'static str {
        match self {
            Status::Ok => "✓",
            Status::Warn => "⚠",
            Status::Fail => "✗",
            Status::Skip => "-",
        }
    }
}

/// One check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub message: String,
    pub hint: Option<String>,
}

fn check(name: &'static str, status: Status, message: impl Into<String>) -> Check {
    Check {
        name,
        status,
        message: message.into(),
        hint: None,
    }
}

fn hinted(
    name: &'static str,
    status: Status,
    message: impl Into<String>,
    hint: impl Into<String>,
) -> Check {
    Check {
        name,
        status,
        message: message.into(),
        hint: Some(hint.into()),
    }
}

/// One row of the Xcode compatibility table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub subject: String,
    pub feature: String,
    pub status: String,
    pub note: Option<String>,
}

/// The compatibility table the daemon's drivers supply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compatibility {
    pub rows: Vec<Row>,
    pub note: Option<String>,
}

/// Every check and the compatibility table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub checks: Vec<Check>,
    pub compatibility: Compatibility,
}

impl Report {
    /// Whether a check failed.
    pub fn failed(&self) -> bool {
        self.checks.iter().any(|c| c.status == Status::Fail)
    }

    /// The report as doctor prints it: a line per check, hints below, then
    /// the compatibility table.
    pub fn to_text(&self) -> String {
        let width = self.checks.iter().map(|c| c.name.len()).max().unwrap_or(0);
        let mut text = String::new();
        for check in &self.checks {
            let _ = writeln!(
                text,
                "{} {:width$}  {}",
                check.status.mark(),
                check.name,
                check.message
            );
            if let Some(hint) = &check.hint {
                let _ = writeln!(text, "    hint: {hint}");
            }
        }
        let _ = writeln!(text, "\nXcode compatibility:");
        if self.compatibility.rows.is_empty() {
            let note = self
                .compatibility
                .note
                .as_deref()
                .unwrap_or("no driver has supplied the table");
            let _ = writeln!(text, "  (no rows: {note})");
        }
        for row in &self.compatibility.rows {
            let note = row
                .note
                .as_deref()
                .map(|n| format!("  {n}"))
                .unwrap_or_default();
            let _ = writeln!(
                text,
                "  {}  {}  {}{note}",
                row.subject, row.feature, row.status
            );
        }
        text
    }

    /// The report as the JSON reply's data.
    pub fn to_json(&self) -> Value {
        json!({
            "checks": self.checks.iter().map(|c| json!({
                "name": c.name,
                "status": c.status.name(),
                "message": c.message,
                "hint": c.hint,
            })).collect::<Vec<_>>(),
            "compatibility": {
                "rows": self.compatibility.rows.iter().map(|r| json!({
                    "subject": r.subject,
                    "feature": r.feature,
                    "status": r.status,
                    "note": r.note,
                })).collect::<Vec<_>>(),
                "note": self.compatibility.note,
            },
            "failed": self.failed(),
        })
    }
}

/// Runs every check, in the order doctor prints them. The checks of a
/// device driver the run leaves out (see `left_out`) ask the host nothing.
pub fn run_checks(probe: &dyn Probe) -> Report {
    let daemon = probe.daemon();
    let mut checks = vec![daemon_check(&daemon), drivers_check(&daemon)];
    let simulator = left_out(probe, &SIMULATOR);
    if let Some(reason) = &simulator {
        checks.extend(skipped(&["xcode", "simulator-runtimes"], reason));
    } else {
        let xcode = xcode_check(probe);
        let has_xcode = xcode.status == Status::Ok;
        checks.push(xcode);
        checks.push(runtimes_check(probe, has_xcode));
    }
    let iphones = left_out(probe, &IOS_DEVICE);
    if let Some(reason) = &iphones {
        checks.extend(skipped(
            &["ios-signing", "ios-runner", "ios-devices"],
            reason,
        ));
    } else {
        checks.push(signing_check(probe));
        checks.push(runner_check(probe, &daemon));
        checks.push(iphones_check(probe));
    }
    if let Some(reason) = left_out(probe, &ANDROID) {
        checks.extend(skipped(
            &["android-sdk", "adb-server", "emulator-control"],
            &reason,
        ));
    } else {
        let (sdk, adb) = android_sdk_check(probe);
        checks.push(sdk);
        let (server, running) = adb_server_check(probe);
        checks.push(server);
        checks.push(emulator_check(
            probe,
            adb.as_deref().filter(|_| running),
            &daemon,
        ));
    }
    checks.push(tool_check(
        probe,
        "sfw",
        "install it with npm i -g sfw; every install goes through Socket Firewall",
    ));
    checks.push(tool_check(
        probe,
        "just",
        "install it with sfw cargo install just",
    ));
    let compatibility = compatibility(&daemon);
    // The Simulator's driver and the iPhone's both fill the table.
    checks.push(match (&simulator, &iphones) {
        (Some(reason), Some(_)) => check("xcode-compatibility", Status::Skip, reason.as_str()),
        _ => compatibility_check(probe, &compatibility),
    });
    Report {
        checks,
        compatibility,
    }
}

/// Why the checks of `driver` are not consulted: the run's
/// `MOBILE_AGENT_DRIVERS` leaves it out. The variable is read as the daemon
/// reads it, comma-separated names in any case or `none`; unset or blank,
/// it leaves out only a driver that registers when named.
fn left_out(probe: &dyn Probe, driver: &DeviceDriver) -> Option<String> {
    let Some(list) = probe.env(DRIVERS).filter(|list| !list.trim().is_empty()) else {
        return driver.opt_in.then(|| {
            format!(
                "not consulted: the {} driver registers only when {DRIVERS} names {}",
                driver.title, driver.names[0]
            )
        });
    };
    let named = list
        .split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .any(|entry| driver.names.contains(&entry.as_str()));
    (!named).then(|| {
        format!(
            "not consulted: {DRIVERS}={} leaves out the {} driver",
            list.trim(),
            driver.title
        )
    })
}

/// The checks `names`, each skipped for `reason` without asking the host.
fn skipped(names: &[&'static str], reason: &str) -> Vec<Check> {
    names
        .iter()
        .map(|&name| check(name, Status::Skip, reason))
        .collect()
}

fn daemon_check(daemon: &Result<Value, String>) -> Check {
    match daemon {
        Ok(data) => check(
            "daemon",
            Status::Ok,
            format!(
                "running, pid {}, version {}",
                data["pid"],
                data["version"].as_str().unwrap_or("unknown")
            ),
        ),
        Err(error) => hinted(
            "daemon",
            Status::Fail,
            error.clone(),
            "see daemon.log in the socket directory, or run mobile-agent status",
        ),
    }
}

fn drivers_check(daemon: &Result<Value, String>) -> Check {
    let Ok(data) = daemon else {
        return check("drivers", Status::Skip, "needs the daemon");
    };
    let drivers: Vec<&str> = data["drivers"]
        .as_array()
        .map(|d| d.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if drivers.is_empty() {
        return hinted(
            "drivers",
            Status::Warn,
            "no device driver is registered in this build",
            "set MOBILE_AGENT_FAKE_TARGETS=android to try the commands on a fake target",
        );
    }
    check("drivers", Status::Ok, drivers.join(", "))
}

const SELECT_XCODE: &str = "install Xcode, then run sudo xcode-select -s /Applications/Xcode.app";

fn xcode_check(probe: &dyn Probe) -> Check {
    if probe.os() != "macos" {
        return check("xcode", Status::Skip, "Xcode runs on macOS only");
    }
    let selected = probe
        .run("xcode-select", &["-p"])
        .filter(|out| out.success)
        .map(|out| out.stdout.trim().to_owned());
    let Some(path) = selected else {
        return hinted(
            "xcode",
            Status::Warn,
            "no Xcode is selected; iOS Simulators need Xcode",
            SELECT_XCODE,
        );
    };
    match probe.run("xcodebuild", &["-version"]) {
        Some(out) if out.success => {
            let mut lines = out.stdout.lines();
            let version = lines.next().unwrap_or("Xcode").trim().to_owned();
            let build = lines
                .next()
                .and_then(|line| line.trim().strip_prefix("Build version "))
                .map(|b| format!(" ({b})"))
                .unwrap_or_default();
            check("xcode", Status::Ok, format!("{version}{build} at {path}"))
        }
        _ if path.contains("CommandLineTools") => hinted(
            "xcode",
            Status::Warn,
            format!("{path} holds the Command Line Tools alone; iOS Simulators need Xcode"),
            SELECT_XCODE,
        ),
        out => hinted(
            "xcode",
            Status::Warn,
            format!(
                "xcodebuild -version failed: {}",
                out.map(|o| first_line(&o.stderr))
                    .unwrap_or_else(|| "not found".to_owned())
            ),
            SELECT_XCODE,
        ),
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_owned()
}

fn runtimes_check(probe: &dyn Probe, has_xcode: bool) -> Check {
    const NAME: &str = "simulator-runtimes";
    if probe.os() != "macos" {
        return check(NAME, Status::Skip, "iOS Simulators run on macOS only");
    }
    if !has_xcode {
        return check(NAME, Status::Skip, "needs Xcode");
    }
    let listing = match probe.run("xcrun", &["simctl", "list", "runtimes", "--json"]) {
        Some(out) if out.success => out.stdout,
        out => {
            return hinted(
                NAME,
                Status::Warn,
                format!(
                    "xcrun simctl list runtimes failed: {}",
                    out.map(|o| first_line(&o.stderr))
                        .unwrap_or_else(|| "not found".to_owned())
                ),
                SELECT_XCODE,
            );
        }
    };
    let runtimes: Value = serde_json::from_str(&listing).unwrap_or(Value::Null);
    let ios: Vec<&str> = runtimes["runtimes"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter(|r| r["isAvailable"] == Value::Bool(true))
                .filter_map(|r| r["name"].as_str())
                .filter(|name| name.starts_with("iOS "))
                .collect()
        })
        .unwrap_or_default();
    if ios.is_empty() {
        return hinted(
            NAME,
            Status::Warn,
            "no iOS Simulator runtime is installed",
            "install one with xcodebuild -downloadPlatform iOS",
        );
    }
    check(NAME, Status::Ok, ios.join(", "))
}

const NEEDS_MACOS: &str = "physical iPhones need macOS";

/// The step that gives the runner a signing identity: the team's
/// development certificate and the team's identifier in the environment.
const SIGNING_STEP: &str = "create the team's Apple Development certificate in Xcode (Settings > Accounts, Manage Certificates), then export MOBILE_AGENT_IOS_TEAM with the team's identifier";

/// Whether this Mac can sign the iPhone runner: `MOBILE_AGENT_IOS_TEAM` set
/// and a valid Apple Development identity in the keychain. Neither the team
/// nor the identity is named; which team an identity belongs to each
/// phone's readiness tells (`no_signing_identity`).
fn signing_check(probe: &dyn Probe) -> Check {
    const NAME: &str = "ios-signing";
    if probe.os() != "macos" {
        return check(NAME, Status::Skip, NEEDS_MACOS);
    }
    if probe.env(TEAM).is_none_or(|team| team.trim().is_empty()) {
        return hinted(
            NAME,
            Status::Warn,
            format!("{TEAM} is not set, so the runner cannot be signed"),
            SIGNING_STEP,
        );
    }
    match probe.run("security", &["find-identity", "-v", "-p", "codesigning"]) {
        Some(out) if out.success => {
            let identities = out
                .stdout
                .lines()
                .filter(|line| line.contains("\"Apple Development:"))
                .count();
            match identities {
                0 => hinted(
                    NAME,
                    Status::Warn,
                    format!(
                        "{TEAM} is set, and the keychain holds no valid Apple Development identity to sign the runner with"
                    ),
                    SIGNING_STEP,
                ),
                count => check(
                    NAME,
                    Status::Ok,
                    format!(
                        "{TEAM} is set, and the keychain holds {count} valid Apple Development {}; each iPhone's readiness below tells whether one is the team's",
                        if count == 1 { "identity" } else { "identities" }
                    ),
                ),
            }
        }
        out => hinted(
            NAME,
            Status::Warn,
            format!(
                "security find-identity failed: {}",
                out.map(|o| first_line(&o.stderr))
                    .unwrap_or_else(|| "not found".to_owned())
            ),
            SIGNING_STEP,
        ),
    }
}

/// How the runner bundle is built and reaches a phone.
const RUNNER_STEP: &str = "build it with just agent-ios-build (agent-ios/build.sh, with MOBILE_AGENT_IOS_TEAM set); an attach installs it on a phone whose runner is missing or of another version";

/// Whether the runner bundle the daemon installs (its `iosRunner`) is
/// built, and of the version the daemon expects.
fn runner_check(probe: &dyn Probe, daemon: &Result<Value, String>) -> Check {
    const NAME: &str = "ios-runner";
    if probe.os() != "macos" {
        return check(NAME, Status::Skip, NEEDS_MACOS);
    }
    let Ok(data) = daemon else {
        return check(NAME, Status::Skip, "needs the daemon");
    };
    let runner = &data["iosRunner"];
    let (Some(bundle), Some(expected)) = (runner["bundle"].as_str(), runner["version"].as_str())
    else {
        return check(
            NAME,
            Status::Skip,
            "the daemon does not register the physical iOS driver",
        );
    };
    let plist = Path::new(bundle).join("Info.plist");
    if !probe.is_file(&plist) {
        return hinted(
            NAME,
            Status::Warn,
            format!("no runner bundle is built at {bundle}"),
            RUNNER_STEP,
        );
    }
    let plist = plist.to_string_lossy();
    let version = probe
        .run(
            "plutil",
            &[
                "-extract",
                "CFBundleShortVersionString",
                "raw",
                "-o",
                "-",
                &plist,
            ],
        )
        .filter(|out| out.success)
        .map(|out| out.stdout.trim().to_owned());
    match version {
        Some(version) if version == expected => {
            check(NAME, Status::Ok, format!("{bundle}, version {version}"))
        }
        Some(version) => hinted(
            NAME,
            Status::Warn,
            format!(
                "the runner bundle at {bundle} is version {version}, and the daemon expects {expected}"
            ),
            RUNNER_STEP,
        ),
        None => hinted(
            NAME,
            Status::Warn,
            format!("the version of the runner bundle at {bundle} cannot be read"),
            RUNNER_STEP,
        ),
    }
}

/// Each iPhone the daemon's `devices` lists, in its readiness state, with
/// the step the daemon names for a state that keeps it from a session.
fn iphones_check(probe: &dyn Probe) -> Check {
    const NAME: &str = "ios-devices";
    if probe.os() != "macos" {
        return check(NAME, Status::Skip, NEEDS_MACOS);
    }
    let listing = match probe.devices() {
        Ok(listing) => listing,
        Err(error) => {
            return check(
                NAME,
                Status::Skip,
                format!("needs the daemon's devices: {error}"),
            );
        }
    };
    let phones: Vec<&Value> = listing["devices"]
        .as_array()
        .map(|all| all.iter().filter(|d| d["class"] == "ios-device").collect())
        .unwrap_or_default();
    if phones.is_empty() {
        return hinted(
            NAME,
            Status::Warn,
            "no iPhone is attached over USB",
            "connect one over USB, unlocked; devices lists it in its readiness state",
        );
    }
    let text = |phone: &Value, key: &str| phone[key].as_str().unwrap_or_default().to_owned();
    let listed: Vec<String> = phones
        .iter()
        .map(|phone| {
            format!(
                "{} ({}, {}): {}",
                text(phone, "id"),
                text(phone, "model"),
                text(phone, "os"),
                text(phone, "state")
            )
        })
        .collect();
    let steps: Vec<String> = phones
        .iter()
        .filter(|phone| phone["state"] != "ready")
        .map(|phone| {
            let advice = phone["advice"]
                .as_str()
                .unwrap_or("devices names what stands in the way");
            format!("{}: {advice}", text(phone, "id"))
        })
        .collect();
    if steps.is_empty() {
        check(NAME, Status::Ok, listed.join("; "))
    } else {
        hinted(NAME, Status::Warn, listed.join("; "), steps.join("; "))
    }
}

/// The Android SDK check and the SDK's adb.
fn android_sdk_check(probe: &dyn Probe) -> (Check, Option<PathBuf>) {
    const NAME: &str = "android-sdk";
    let mut candidates: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|key| probe.env(key))
        .map(PathBuf::from)
        .collect();
    if let Some(home) = probe.env("HOME") {
        let default = match probe.os() {
            "macos" => Path::new(&home).join("Library/Android/sdk"),
            _ => Path::new(&home).join("Android/Sdk"),
        };
        candidates.push(default);
    }
    let Some((sdk, adb)) = candidates.iter().find_map(|sdk| {
        let adb = sdk.join("platform-tools").join("adb");
        probe.is_file(&adb).then(|| (sdk.clone(), adb))
    }) else {
        return (
            hinted(
                NAME,
                Status::Warn,
                "no Android SDK with platform-tools/adb found",
                "install the Android SDK with its platform tools and set ANDROID_HOME",
            ),
            None,
        );
    };
    let adb_text = adb.to_string_lossy().into_owned();
    let version = probe
        .run(&adb_text, &["version"])
        .filter(|out| out.success)
        .map(|out| {
            let mut lines = out.stdout.lines();
            let bridge = lines
                .next()
                .and_then(|l| l.trim().strip_prefix("Android Debug Bridge version "))
                .unwrap_or("unknown")
                .to_owned();
            let tools = lines
                .next()
                .and_then(|l| l.trim().strip_prefix("Version "))
                .map(|v| format!(" ({v})"))
                .unwrap_or_default();
            format!("adb {bridge}{tools}")
        })
        .unwrap_or_else(|| "adb did not report its version".to_owned());
    (
        check(NAME, Status::Ok, format!("{}, {version}", sdk.display())),
        Some(adb),
    )
}

/// The adb server check and whether a server runs.
fn adb_server_check(probe: &dyn Probe) -> (Check, bool) {
    let port = probe
        .env("ANDROID_ADB_SERVER_PORT")
        .and_then(|p| p.parse().ok())
        .unwrap_or(ADB_PORT);
    match probe.adb_server(port) {
        Ok(version) => (
            check(
                "adb-server",
                Status::Ok,
                format!("running on port {port}, protocol {version}"),
            ),
            true,
        ),
        Err(error) => (
            hinted(
                "adb-server",
                Status::Warn,
                format!("no adb server answers on port {port} ({error})"),
                "start it with adb start-server",
            ),
            false,
        ),
    }
}

/// Where running emulators write their discovery files.
fn discovery_dirs(probe: &dyn Probe) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(runtime) = probe.env("XDG_RUNTIME_DIR") {
        dirs.push(Path::new(&runtime).join("avd/running"));
    }
    if probe.os() == "macos"
        && let Some(home) = probe.env("HOME")
    {
        dirs.push(Path::new(&home).join("Library/Caches/TemporaryItems/avd/running"));
    }
    let temp = probe.env("TMPDIR").unwrap_or_else(|| "/tmp".to_owned());
    dirs.push(Path::new(&temp).join("avd/running"));
    dirs
}

/// The gRPC port of the emulator with console port `console`, from its
/// discovery file.
fn grpc_port(files: &[(PathBuf, String)], console: u16) -> Option<u16> {
    let serial = format!("port.serial={console}");
    files
        .iter()
        .find(|(_, text)| text.lines().any(|line| line.trim() == serial))
        .and_then(|(_, text)| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix("grpc.port="))
                .and_then(|port| port.trim().parse().ok())
        })
}

fn emulator_check(probe: &dyn Probe, adb: Option<&Path>, daemon: &Result<Value, String>) -> Check {
    const NAME: &str = "emulator-control";
    // A daemon told to leave the controllers unused sends everything
    // through the agent, whatever the emulators serve.
    if daemon
        .as_ref()
        .is_ok_and(|data| data["emulatorController"] == "off")
    {
        return hinted(
            NAME,
            Status::Warn,
            "off in the daemon: taps, swipes and frames on every emulator go through the agent, as on a device, with no frame stream",
            "to use the gRPC controller again, drop --emulator-controller off, MOBILE_AGENT_EMULATOR_CONTROLLER=off or emulatorController from the config, then restart the daemon with mobile-agent shutdown",
        );
    }
    let Some(adb) = adb else {
        return check(
            NAME,
            Status::Skip,
            "needs the Android SDK's adb and a running adb server",
        );
    };
    let listing = match probe.run(&adb.to_string_lossy(), &["devices"]) {
        Some(out) if out.success => out.stdout,
        _ => return check(NAME, Status::Skip, "adb devices failed"),
    };
    let consoles: Vec<u16> = listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let state = fields.next()?;
            (state == "device")
                .then_some(serial)?
                .strip_prefix("emulator-")?
                .parse()
                .ok()
        })
        .collect();
    if consoles.is_empty() {
        return check(NAME, Status::Skip, "no emulator is running");
    }
    let files: Vec<(PathBuf, String)> = discovery_dirs(probe)
        .iter()
        .flat_map(|dir| probe.files(dir))
        .collect();
    let mut served = Vec::new();
    let mut missing = Vec::new();
    for console in consoles {
        match grpc_port(&files, console) {
            Some(port) => served.push(format!("emulator-{console} on port {port}")),
            None => missing.push(console),
        }
    }
    if missing.is_empty() {
        return check(
            NAME,
            Status::Ok,
            format!("gRPC control: {}", served.join(", ")),
        );
    }
    let names: Vec<String> = missing.iter().map(|c| format!("emulator-{c}")).collect();
    let hints: Vec<String> = missing
        .iter()
        .map(|&console| {
            let port = console.saturating_add(GRPC_OFFSET);
            format!(
                "start emulator-{console} with -grpc {port} -grpc-use-token, or enable it now with adb -s emulator-{console} emu grpc {port}"
            )
        })
        .collect();
    hinted(
        NAME,
        Status::Warn,
        format!(
            "{} {} no gRPC control endpoint, so frames and fast input fall back to the agent",
            names.join(", "),
            if names.len() == 1 { "has" } else { "have" }
        ),
        hints.join("; "),
    )
}

fn tool_check(probe: &dyn Probe, name: &'static str, install: &str) -> Check {
    match probe.run(name, &["--version"]) {
        Some(out) if out.success => check(name, Status::Ok, first_line(&out.stdout)),
        _ => hinted(
            name,
            Status::Warn,
            format!("{name} is not on PATH"),
            install,
        ),
    }
}

fn compatibility(daemon: &Result<Value, String>) -> Compatibility {
    let Ok(data) = daemon else {
        return Compatibility {
            rows: Vec::new(),
            note: Some("the daemon did not answer".to_owned()),
        };
    };
    let table = &data["compatibility"];
    let text = |row: &Value, key: &str| row[key].as_str().unwrap_or_default().to_owned();
    Compatibility {
        rows: table["rows"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| Row {
                        subject: text(row, "subject"),
                        feature: text(row, "feature"),
                        status: text(row, "status"),
                        note: row["note"].as_str().map(str::to_owned),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        note: table["note"].as_str().map(str::to_owned),
    }
}

fn compatibility_check(probe: &dyn Probe, table: &Compatibility) -> Check {
    const NAME: &str = "xcode-compatibility";
    if probe.os() != "macos" {
        return check(NAME, Status::Skip, "iOS Simulators run on macOS only");
    }
    if table.rows.is_empty() {
        return check(
            NAME,
            Status::Skip,
            "no Simulator driver has supplied the Xcode compatibility table yet",
        );
    }
    check(
        NAME,
        Status::Ok,
        format!("{} validated combinations, listed below", table.rows.len()),
    )
}

/// The host this process runs on, and its daemon.
pub struct System {
    pub endpoint: Endpoint,
    pub session: String,
}

impl Probe for System {
    fn os(&self) -> &str {
        std::env::consts::OS
    }

    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    }

    fn run(&self, program: &str, args: &[&str]) -> Option<Output> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        let reader = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut text = String::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_string(&mut text);
                }
                text
            })
        };
        let stdout = reader(
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let stderr = reader(
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let deadline = Instant::now() + COMMAND_LIMIT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let stdout = stdout.join().unwrap_or_default();
        let mut stderr = stderr.join().unwrap_or_default();
        if status.is_none() {
            stderr = format!("timed out after {} s", COMMAND_LIMIT.as_secs());
        }
        Some(Output {
            success: status.is_some_and(|s| s.success()),
            stdout,
            stderr,
        })
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn files(&self, dir: &Path) -> Vec<(PathBuf, String)> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut files: Vec<(PathBuf, String)> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter_map(|path| std::fs::read_to_string(&path).ok().map(|text| (path, text)))
            .collect();
        files.sort();
        files
    }

    fn adb_server(&self, port: u16) -> Result<u32, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500))
            .map_err(|e| e.to_string())?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        stream
            .write_all(b"000chost:version")
            .map_err(|e| e.to_string())?;
        let mut head = [0u8; 8];
        stream.read_exact(&mut head).map_err(|e| e.to_string())?;
        if &head[..4] != b"OKAY" {
            return Err(format!(
                "the server answered {:?}",
                String::from_utf8_lossy(&head)
            ));
        }
        let len = usize::from_str_radix(&String::from_utf8_lossy(&head[4..]), 16)
            .map_err(|e| e.to_string())?;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).map_err(|e| e.to_string())?;
        u32::from_str_radix(&String::from_utf8_lossy(&body), 16).map_err(|e| e.to_string())
    }

    fn daemon(&self) -> Result<Value, String> {
        self.ask(wire::Command::Doctor(wire::Doctor {}))
    }

    fn devices(&self) -> Result<Value, String> {
        self.ask(wire::Command::Devices(wire::Devices {}))
    }
}

impl System {
    /// The daemon's data for `command`, an action without parameters.
    fn ask(&self, command: wire::Command) -> Result<Value, String> {
        let request = request::bare(command, &self.session, client::request_id());
        match client::call(&self.endpoint, &request::line(&request)) {
            Ok(reply) if reply.success => Ok(reply.data),
            Ok(reply) => Err(reply
                .error
                .unwrap_or_else(|| "the daemon failed".to_owned())),
            Err(failure) => Err(failure.message),
        }
    }
}
