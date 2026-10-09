//! `doctor`: each check against scripted command outputs, and the real
//! command listing every check with a status and, against fake tools,
//! asking nothing of a driver the run leaves out.

mod support;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cli::doctor::{Output, Probe, Report, Status, run_checks};
use serde_json::{Value, json};
use support::{Run, TestEnv};

/// A host described by scripted answers, recording what it was asked.
struct Host {
    os: &'static str,
    env: HashMap<&'static str, String>,
    commands: HashMap<String, Output>,
    files: Vec<PathBuf>,
    dirs: HashMap<PathBuf, Vec<(PathBuf, String)>>,
    adb: Result<u32, String>,
    daemon: Result<Value, String>,
    /// The daemon's `devices` data.
    devices: Result<Value, String>,
    /// Each command line run, `adb-server <port>` for each question to the
    /// adb server, and `devices` for each listing asked of the daemon.
    asked: RefCell<Vec<String>>,
}

fn ok(stdout: &str) -> Output {
    Output {
        success: true,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

fn failed(stderr: &str) -> Output {
    Output {
        success: false,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    }
}

const SDK: &str = "/sdk";

impl Host {
    /// A macOS host with Xcode, one runtime, the SDK, a running adb server
    /// with no emulator, sfw, just and a daemon with the fake driver.
    fn healthy() -> Host {
        let mut host = Host {
            os: "macos",
            env: HashMap::from([
                ("ANDROID_HOME", SDK.to_owned()),
                ("HOME", "/home/me".to_owned()),
            ]),
            commands: HashMap::new(),
            files: vec![PathBuf::from(SDK).join("platform-tools/adb")],
            dirs: HashMap::new(),
            adb: Ok(41),
            daemon: Ok(json!({
                "pid": 42,
                "version": "0.1.0+abc",
                "drivers": ["fake"],
                "compatibility": {"rows": [], "note": "the Simulator driver fills this table"}
            })),
            devices: Ok(json!({"devices": []})),
            asked: RefCell::default(),
        };
        host.command(
            "xcode-select -p",
            ok("/Applications/Xcode.app/Contents/Developer\n"),
        );
        host.command(
            "xcodebuild -version",
            ok("Xcode 27.0\nBuild version 27A266a\n"),
        );
        host.command(
            "xcrun simctl list runtimes --json",
            ok(r#"{"runtimes": [
                {"name": "iOS 26.5", "version": "26.5", "platform": "iOS", "isAvailable": true},
                {"name": "watchOS 26.5", "version": "26.5", "platform": "watchOS", "isAvailable": true},
                {"name": "iOS 18.4", "version": "18.4", "platform": "iOS", "isAvailable": false}
            ]}"#),
        );
        host.command(
            "/sdk/platform-tools/adb version",
            ok("Android Debug Bridge version 1.0.41\nVersion 36.0.0-13206524\nInstalled as /sdk/platform-tools/adb\n"),
        );
        host.command(
            "/sdk/platform-tools/adb devices",
            ok("List of devices attached\n\n"),
        );
        host.command("sfw --version", ok("sfw 1.4.2\n"));
        host.command("just --version", ok("just 1.43.0\n"));
        host
    }

    fn command(&mut self, line: &str, output: Output) {
        self.commands.insert(line.to_owned(), output);
    }

    fn check(&self, name: &str) -> (Status, String, Option<String>) {
        let report = run_checks(self);
        let check = report
            .checks
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no {name} check"));
        (check.status, check.message.clone(), check.hint.clone())
    }
}

impl Probe for Host {
    fn os(&self) -> &str {
        self.os
    }

    fn env(&self, key: &str) -> Option<String> {
        self.env.get(key).cloned()
    }

    fn run(&self, program: &str, args: &[&str]) -> Option<Output> {
        let mut line = vec![program];
        line.extend_from_slice(args);
        let line = line.join(" ");
        self.asked.borrow_mut().push(line.clone());
        self.commands.get(&line).cloned()
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|f| f == path)
    }

    fn files(&self, dir: &Path) -> Vec<(PathBuf, String)> {
        self.dirs.get(dir).cloned().unwrap_or_default()
    }

    fn adb_server(&self, port: u16) -> Result<u32, String> {
        self.asked.borrow_mut().push(format!("adb-server {port}"));
        self.adb.clone()
    }

    fn daemon(&self) -> Result<Value, String> {
        self.daemon.clone()
    }

    fn devices(&self) -> Result<Value, String> {
        self.asked.borrow_mut().push("devices".to_owned());
        self.devices.clone()
    }
}

#[test]
fn xcode_is_reported_with_its_version_build_and_path() {
    let (status, message, _) = Host::healthy().check("xcode");
    assert_eq!(status, Status::Ok);
    assert!(message.contains("Xcode 27.0 (27A266a)"), "{message}");
    assert!(message.contains("/Applications/Xcode.app"), "{message}");
}

#[test]
fn a_missing_xcode_warns_with_how_to_install_it() {
    let mut host = Host::healthy();
    host.commands.remove("xcode-select -p");
    host.commands.remove("xcodebuild -version");
    let (status, _, hint) = host.check("xcode");
    assert_eq!(status, Status::Warn);
    assert!(hint.unwrap_or_default().contains("xcode-select"));
    let (runtimes, message, _) = host.check("simulator-runtimes");
    assert_eq!(runtimes, Status::Skip, "{message}");
}

#[test]
fn command_line_tools_alone_warn_that_simulators_need_xcode() {
    let mut host = Host::healthy();
    host.command(
        "xcode-select -p",
        ok("/Library/Developer/CommandLineTools\n"),
    );
    host.command(
        "xcodebuild -version",
        failed("xcode-select: error: tool 'xcodebuild' requires Xcode"),
    );
    let (status, message, hint) = host.check("xcode");
    assert_eq!(status, Status::Warn);
    assert!(message.contains("Command Line Tools"), "{message}");
    assert!(hint.unwrap_or_default().contains("sudo xcode-select -s"));
}

#[test]
fn apple_checks_are_skipped_off_macos() {
    let mut host = Host::healthy();
    host.os = "linux";
    assert_eq!(host.check("xcode").0, Status::Skip);
    assert_eq!(host.check("simulator-runtimes").0, Status::Skip);
    assert_eq!(host.check("xcode-compatibility").0, Status::Skip);
}

#[test]
fn simulator_runtimes_list_the_available_ios_runtimes() {
    let (status, message, _) = Host::healthy().check("simulator-runtimes");
    assert_eq!(status, Status::Ok);
    assert_eq!(message, "iOS 26.5");
}

#[test]
fn no_ios_runtime_warns_with_the_download_command() {
    let mut host = Host::healthy();
    host.command(
        "xcrun simctl list runtimes --json",
        ok(r#"{"runtimes": []}"#),
    );
    let (status, _, hint) = host.check("simulator-runtimes");
    assert_eq!(status, Status::Warn);
    assert!(
        hint.unwrap_or_default()
            .contains("xcodebuild -downloadPlatform iOS")
    );
}

#[test]
fn the_android_sdk_is_found_from_android_home_with_adb_s_version() {
    let (status, message, _) = Host::healthy().check("android-sdk");
    assert_eq!(status, Status::Ok);
    assert!(message.contains("/sdk"), "{message}");
    assert!(
        message.contains("1.0.41") && message.contains("36.0.0"),
        "{message}"
    );
}

#[test]
fn the_default_sdk_location_is_used_without_android_home() {
    let mut host = Host::healthy();
    host.env.remove("ANDROID_HOME");
    let default = "/home/me/Library/Android/sdk";
    host.files = vec![PathBuf::from(default).join("platform-tools/adb")];
    host.command(
        "/home/me/Library/Android/sdk/platform-tools/adb version",
        ok("Android Debug Bridge version 1.0.41\nVersion 36.0.0-13206524\n"),
    );
    let (status, message, _) = host.check("android-sdk");
    assert_eq!(status, Status::Ok, "{message}");
    assert!(message.contains(default), "{message}");
}

#[test]
fn a_missing_android_sdk_warns_and_skips_the_emulator_check() {
    let mut host = Host::healthy();
    host.env.remove("ANDROID_HOME");
    host.files.clear();
    let (status, _, hint) = host.check("android-sdk");
    assert_eq!(status, Status::Warn);
    assert!(hint.unwrap_or_default().contains("ANDROID_HOME"));
    assert_eq!(host.check("emulator-control").0, Status::Skip);
}

#[test]
fn a_running_adb_server_reports_its_protocol_version() {
    let (status, message, _) = Host::healthy().check("adb-server");
    assert_eq!(status, Status::Ok);
    assert!(
        message.contains("5037") && message.contains("41"),
        "{message}"
    );
}

#[test]
fn a_stopped_adb_server_warns_with_the_start_command() {
    let mut host = Host::healthy();
    host.adb = Err("connection refused".to_owned());
    let (status, _, hint) = host.check("adb-server");
    assert_eq!(status, Status::Warn);
    assert!(hint.unwrap_or_default().contains("adb start-server"));
    host.env
        .insert("ANDROID_ADB_SERVER_PORT", "5038".to_owned());
    let (_, message, _) = host.check("adb-server");
    assert!(message.contains("5038"), "{message}");
}

fn with_emulators(discovery: Vec<(PathBuf, String)>) -> Host {
    let mut host = Host::healthy();
    host.command(
        "/sdk/platform-tools/adb devices",
        ok("List of devices attached\nemulator-5554\tdevice\nemulator-5556\tdevice\nR58M123\tdevice\n\n"),
    );
    host.dirs.insert(
        PathBuf::from("/home/me/Library/Caches/TemporaryItems/avd/running"),
        discovery,
    );
    host
}

#[test]
fn emulators_with_their_control_endpoint_pass() {
    let host = with_emulators(vec![
        (
            PathBuf::from("pid_101.ini"),
            "port.serial=5554\ngrpc.port=8554\ngrpc.token=abc=\n".to_owned(),
        ),
        (
            PathBuf::from("pid_102.ini"),
            "port.serial=5556\ngrpc.port=8556\n".to_owned(),
        ),
    ]);
    let (status, message, _) = host.check("emulator-control");
    assert_eq!(status, Status::Ok, "{message}");
    assert!(
        message.contains("emulator-5554") && message.contains("8554"),
        "{message}"
    );
}

#[test]
fn an_emulator_without_the_control_flag_gets_the_flag_hint() {
    let host = with_emulators(vec![(
        PathBuf::from("pid_101.ini"),
        "port.serial=5554\ngrpc.port=8554\n".to_owned(),
    )]);
    let (status, message, hint) = host.check("emulator-control");
    assert_eq!(status, Status::Warn);
    assert!(message.contains("emulator-5556"), "{message}");
    assert!(!message.contains("emulator-5554"), "{message}");
    let hint = hint.unwrap_or_default();
    assert!(hint.contains("-grpc 8556 -grpc-use-token"), "{hint}");
    assert!(
        hint.contains("adb -s emulator-5556 emu grpc 8556"),
        "{hint}"
    );
}

/// Task 10.12: a daemon whose emulator controller setting is off sends
/// everything through the agent, whatever the emulators serve, and the
/// check says so with how to turn the controller back on.
#[test]
fn a_daemon_that_leaves_the_controller_unused_says_so() {
    let mut host = with_emulators(vec![(
        PathBuf::from("pid_101.ini"),
        "port.serial=5554\ngrpc.port=8554\n".to_owned(),
    )]);
    if let Ok(data) = host.daemon.as_mut() {
        data["emulatorController"] = json!("off");
    }
    let (status, message, hint) = host.check("emulator-control");
    assert_eq!(status, Status::Warn, "{message}");
    assert!(
        message.contains("off") && message.contains("agent"),
        "{message}"
    );
    let hint = hint.unwrap_or_default();
    assert!(
        hint.contains("MOBILE_AGENT_EMULATOR_CONTROLLER") && hint.contains("shutdown"),
        "{hint}"
    );
    // Without an emulator the setting still shows.
    let mut idle = Host::healthy();
    if let Ok(data) = idle.daemon.as_mut() {
        data["emulatorController"] = json!("off");
    }
    assert_eq!(idle.check("emulator-control").0, Status::Warn);
}

#[test]
fn no_running_emulator_skips_the_control_check() {
    let (status, message, _) = Host::healthy().check("emulator-control");
    assert_eq!(status, Status::Skip, "{message}");
    assert!(message.contains("no emulator"), "{message}");
}

#[test]
fn sfw_and_just_are_found_or_hinted() {
    let host = Host::healthy();
    assert_eq!(
        host.check("sfw"),
        (Status::Ok, "sfw 1.4.2".to_owned(), None)
    );
    assert_eq!(
        host.check("just"),
        (Status::Ok, "just 1.43.0".to_owned(), None)
    );
    let mut bare = Host::healthy();
    bare.commands.remove("sfw --version");
    bare.commands.remove("just --version");
    let (sfw, _, sfw_hint) = bare.check("sfw");
    assert_eq!(sfw, Status::Warn);
    assert!(sfw_hint.unwrap_or_default().contains("npm i -g sfw"));
    let (just, _, just_hint) = bare.check("just");
    assert_eq!(just, Status::Warn);
    assert!(
        just_hint
            .unwrap_or_default()
            .contains("sfw cargo install just")
    );
}

#[test]
fn the_daemon_check_fails_when_the_daemon_cannot_start() {
    let mut host = Host::healthy();
    assert_eq!(host.check("daemon").0, Status::Ok);
    host.daemon = Err("cannot start the daemon".to_owned());
    let report = run_checks(&host);
    assert!(report.failed());
    let daemon = report
        .checks
        .iter()
        .find(|c| c.name == "daemon")
        .expect("a daemon check");
    assert_eq!(daemon.status, Status::Fail);
    assert!(daemon.message.contains("cannot start"));
}

#[test]
fn drivers_are_named_and_a_daemon_without_one_warns() {
    let host = Host::healthy();
    assert_eq!(host.check("drivers"), (Status::Ok, "fake".to_owned(), None));
    let mut none = Host::healthy();
    none.daemon =
        Ok(json!({"pid": 1, "version": "v", "drivers": [], "compatibility": {"rows": []}}));
    let (status, _, hint) = none.check("drivers");
    assert_eq!(status, Status::Warn);
    assert!(
        hint.unwrap_or_default()
            .contains("MOBILE_AGENT_FAKE_TARGETS")
    );
}

#[test]
fn the_xcode_compatibility_table_waits_for_the_simulator_driver() {
    let report: Report = run_checks(&Host::healthy());
    let check = report
        .checks
        .iter()
        .find(|c| c.name == "xcode-compatibility")
        .expect("a compatibility check");
    assert_eq!(check.status, Status::Skip);
    assert!(
        check.message.contains("Simulator driver"),
        "{}",
        check.message
    );
    assert!(report.compatibility.rows.is_empty());

    let mut filled = Host::healthy();
    filled.daemon = Ok(json!({
        "pid": 42, "version": "v", "drivers": ["simulator"],
        "compatibility": {"rows": [
            {"subject": "Xcode 27.0 / iOS 26.5", "feature": "dtuhidd XPC input", "status": "supported", "note": null}
        ]}
    }));
    let report = run_checks(&filled);
    assert_eq!(report.compatibility.rows.len(), 1);
    let text = report.to_text();
    assert!(
        text.contains("dtuhidd XPC input") && text.contains("supported"),
        "{text}"
    );
}

/// The checks of the Simulator's driver.
const SIMULATOR_CHECKS: [&str; 3] = ["xcode", "simulator-runtimes", "xcode-compatibility"];

/// The checks of Android's driver.
const ANDROID_CHECKS: [&str; 3] = ["android-sdk", "adb-server", "emulator-control"];

/// The checks of the physical iPhone driver.
const IOS_CHECKS: [&str; 3] = ["ios-signing", "ios-runner", "ios-devices"];

/// Task 10.24: doctor reads the run's `MOBILE_AGENT_DRIVERS` as the daemon
/// does, comma-separated names in any case, or `none`, and asks the host
/// nothing for a driver it leaves out, whose checks it skips as not
/// consulted; unset or blank, it leaves none of the default drivers out.
/// Task 5.1: `ios` names the physical iPhone driver, which registers only
/// when named, so its checks are consulted only then, and unset they say
/// so.
#[test]
fn the_run_s_drivers_choose_which_checks_ask_the_host() {
    for (drivers, simulator, android, ios) in [
        (None, true, true, false),
        (Some("  "), true, true, false),
        (Some("simulator"), true, false, false),
        (Some(" iOS "), false, false, true),
        (Some("android"), false, true, false),
        (Some("Android, simulator"), true, true, false),
        (Some("ios,android"), false, true, true),
        (Some("none"), false, false, false),
    ] {
        let mut host = Host::healthy();
        if let Some(value) = drivers {
            host.env.insert("MOBILE_AGENT_DRIVERS", value.to_owned());
        }
        let report = run_checks(&host);
        for (names, named) in [
            (SIMULATOR_CHECKS, simulator),
            (ANDROID_CHECKS, android),
            (IOS_CHECKS, ios),
        ] {
            for name in names {
                let check = report
                    .checks
                    .iter()
                    .find(|c| c.name == name)
                    .unwrap_or_else(|| panic!("no {name} check"));
                // The compatibility table serves the Simulator's and the
                // iPhone's drivers alike.
                let consulted = named || (name == "xcode-compatibility" && ios);
                assert_eq!(
                    check.message.contains("not consulted"),
                    !consulted,
                    "{drivers:?}: {check:?}"
                );
                if consulted {
                    continue;
                }
                assert_eq!(check.status, Status::Skip, "{drivers:?}: {check:?}");
                let unset = drivers.is_none_or(|value| value.trim().is_empty());
                let says = if unset {
                    "names ios"
                } else {
                    "MOBILE_AGENT_DRIVERS="
                };
                assert!(check.message.contains(says), "{drivers:?}: {check:?}");
            }
        }
        let asked = host.asked.borrow();
        assert_eq!(
            asked.iter().any(|line| line.contains("adb")),
            android,
            "{drivers:?} asked {asked:?}"
        );
        assert_eq!(
            asked.iter().any(|line| line.starts_with("xc")),
            simulator,
            "{drivers:?} asked {asked:?}"
        );
        assert_eq!(
            asked
                .iter()
                .any(|line| line.starts_with("security") || line == "devices"),
            ios,
            "{drivers:?} asked {asked:?}"
        );
    }
}

/// The healthy host with a daemon that registers the physical iPhone
/// driver alone, as `MOBILE_AGENT_DRIVERS=ios` makes it, its runner bundle
/// built and the team's identity in the keychain.
fn iphone_host() -> Host {
    let mut host = Host::healthy();
    host.env.insert("MOBILE_AGENT_DRIVERS", "ios".to_owned());
    host.env
        .insert("MOBILE_AGENT_IOS_TEAM", "TEAMPLACE1".to_owned());
    host.daemon = Ok(json!({
        "pid": 42,
        "version": "0.1.0+abc",
        "drivers": ["ios"],
        "compatibility": {"rows": [
            {"subject": "iOS 26.6.2, Xcode 27.0", "feature": "physical iPhones: iOS 17.4 up to but not including 27", "status": "supported", "note": null},
            {"subject": "iOS 26.6.2 (iPhone SE 3rd generation), Xcode 27.0", "feature": "ListDevices (usbmuxd)", "status": "validated", "note": null}
        ]},
        "iosRunner": {"bundle": RUNNER, "version": "0.2.0", "variable": "MOBILE_AGENT_IOS_RUNNER_APP"}
    }));
    host.files.push(PathBuf::from(RUNNER).join("Info.plist"));
    host.command(
        &format!("plutil -extract CFBundleShortVersionString raw -o - {RUNNER}/Info.plist"),
        ok("0.2.0\n"),
    );
    host.command(
        "security find-identity -v -p codesigning",
        ok("  1) 0123456789ABCDEF0123456789ABCDEF01234567 \"Apple Development: placeholder-developer (PLACEHOLD0)\"\n     1 valid identities found\n"),
    );
    host.devices = Ok(json!({"devices": [
        {"id": "FAKE-IOS-DEVICE-1", "platform": "ios", "class": "ios-device", "name": "placeholder-iphone",
         "model": "iPhone14,6", "os": "iOS 26.6.2", "state": "ready", "claimedBy": null},
        {"id": "emulator-5554", "platform": "android", "class": "android-emulator", "name": "Fake Android emulator",
         "model": "fake", "os": "Android 16", "state": "device", "claimedBy": null}
    ]}));
    host
}

/// Where the scripted daemon says its runner bundle is.
const RUNNER: &str = "/placeholder/agent-ios/dist/MobileAgentRunner-Runner.app";

/// Task 5.1: the signing check reads `MOBILE_AGENT_IOS_TEAM` and the
/// keychain's valid Apple Development identities, naming neither the team
/// nor the identity, and warns with the step when either is missing; which
/// team an identity belongs to each phone's readiness tells.
#[test]
fn the_signing_check_reads_the_team_and_the_development_identities() {
    let host = iphone_host();
    let (status, message, _) = host.check("ios-signing");
    assert_eq!(status, Status::Ok, "{message}");
    assert!(
        message.contains("MOBILE_AGENT_IOS_TEAM")
            && message.contains("1 valid Apple Development identity"),
        "{message}"
    );
    assert!(
        !message.contains("TEAMPLACE1") && !message.contains("placeholder-developer"),
        "{message}"
    );

    let mut no_team = iphone_host();
    no_team.env.remove("MOBILE_AGENT_IOS_TEAM");
    let (status, message, hint) = no_team.check("ios-signing");
    assert_eq!(status, Status::Warn, "{message}");
    assert!(message.contains("MOBILE_AGENT_IOS_TEAM"), "{message}");
    let hint = hint.unwrap_or_default();
    assert!(
        hint.contains("MOBILE_AGENT_IOS_TEAM") && hint.contains("Xcode"),
        "{hint}"
    );

    let mut no_identity = iphone_host();
    no_identity.command(
        "security find-identity -v -p codesigning",
        ok("     0 valid identities found\n"),
    );
    let (status, message, hint) = no_identity.check("ios-signing");
    assert_eq!(status, Status::Warn, "{message}");
    assert!(message.contains("Apple Development"), "{message}");
    assert!(hint.unwrap_or_default().contains("Xcode"));

    let mut unreadable = iphone_host();
    unreadable.command(
        "security find-identity -v -p codesigning",
        failed("security: SecKeychainSearchCopyNext: The specified item could not be found"),
    );
    let (status, message, _) = unreadable.check("ios-signing");
    assert_eq!(status, Status::Warn, "{message}");
    assert!(message.contains("security"), "{message}");
}

/// Task 5.1: the runner check finds the bundle the daemon installs and its
/// version, and names `just agent-ios-build` when it is missing or of
/// another version than the daemon expects.
#[test]
fn the_runner_check_finds_the_bundle_the_daemon_installs_and_its_version() {
    let (status, message, _) = iphone_host().check("ios-runner");
    assert_eq!(status, Status::Ok, "{message}");
    assert!(
        message.contains(RUNNER) && message.contains("0.2.0"),
        "{message}"
    );

    let mut older = iphone_host();
    older.command(
        &format!("plutil -extract CFBundleShortVersionString raw -o - {RUNNER}/Info.plist"),
        ok("0.1.0\n"),
    );
    let (status, message, hint) = older.check("ios-runner");
    assert_eq!(status, Status::Warn, "{message}");
    assert!(
        message.contains("0.1.0") && message.contains("0.2.0"),
        "{message}"
    );
    assert!(hint.unwrap_or_default().contains("just agent-ios-build"));

    let mut missing = iphone_host();
    missing.files.retain(|file| !file.starts_with(RUNNER));
    let (status, message, hint) = missing.check("ios-runner");
    assert_eq!(status, Status::Warn, "{message}");
    assert!(message.contains(RUNNER), "{message}");
    assert!(hint.unwrap_or_default().contains("just agent-ios-build"));

    let mut no_daemon = iphone_host();
    no_daemon.daemon = Err("cannot start the daemon".to_owned());
    assert_eq!(no_daemon.check("ios-runner").0, Status::Skip);
}

/// Task 5.1, spec `ios-device-driver` "Device not ready": doctor lists each
/// attached iPhone in the readiness state the daemon's `devices` gives it,
/// with the step the daemon names for a state that keeps it from a
/// session, word for word.
#[test]
fn each_iphone_is_listed_in_its_readiness_state_with_the_daemons_step() {
    let (status, message, hint) = iphone_host().check("ios-devices");
    assert_eq!(status, Status::Ok, "{message}");
    assert!(
        message.contains("FAKE-IOS-DEVICE-1") && message.contains("ready"),
        "{message}"
    );
    assert!(!message.contains("emulator-5554"), "{message}");
    assert_eq!(hint, None);

    let advice =
        "Developer Mode is off: on the device open Settings > Privacy & Security > Developer Mode";
    let mut unready = iphone_host();
    unready.devices = Ok(json!({"devices": [
        {"id": "FAKE-IOS-DEVICE-1", "platform": "ios", "class": "ios-device", "name": "placeholder-iphone",
         "model": "iPhone14,6", "os": "iOS 26.6.2", "state": "ready", "claimedBy": null},
        {"id": "FAKE-IOS-DEVICE-2", "platform": "ios", "class": "ios-device", "name": "placeholder-other",
         "model": "iPhone17,5", "os": "iOS 26.6.2", "state": "developer_mode_off", "advice": advice, "claimedBy": null}
    ]}));
    let (status, message, hint) = unready.check("ios-devices");
    assert_eq!(status, Status::Warn, "{message}");
    assert!(
        message.contains("FAKE-IOS-DEVICE-2") && message.contains("developer_mode_off"),
        "{message}"
    );
    let hint = hint.unwrap_or_default();
    assert!(
        hint.contains("FAKE-IOS-DEVICE-2") && hint.contains(advice),
        "{hint}"
    );
    assert!(!hint.contains("FAKE-IOS-DEVICE-1"), "{hint}");

    let mut none = iphone_host();
    none.devices = Ok(json!({"devices": []}));
    let (status, message, _) = none.check("ios-devices");
    assert_eq!(status, Status::Warn, "{message}");
    assert!(message.contains("no iPhone"), "{message}");

    let mut no_daemon = iphone_host();
    no_daemon.devices = Err("cannot start the daemon".to_owned());
    assert_eq!(no_daemon.check("ios-devices").0, Status::Skip);
}

/// Task 5.1: the iPhone checks need macOS, and the compatibility table's
/// device rows come from the daemon, which the check counts.
#[test]
fn the_iphone_checks_need_macos_and_the_device_rows_fill_the_table() {
    let report = run_checks(&iphone_host());
    let compatibility = report
        .checks
        .iter()
        .find(|c| c.name == "xcode-compatibility")
        .expect("a compatibility check");
    assert_eq!(compatibility.status, Status::Ok, "{compatibility:?}");
    assert!(compatibility.message.contains('2'), "{compatibility:?}");
    assert!(report.to_text().contains("ListDevices (usbmuxd)"));

    let mut linux = iphone_host();
    linux.os = "linux";
    for name in IOS_CHECKS {
        let (status, message, _) = linux.check(name);
        assert_eq!(status, Status::Skip, "{name}: {message}");
        assert!(message.contains("macOS"), "{name}: {message}");
    }
}

#[test]
fn doctor_lists_every_check_with_a_status_in_both_outputs() {
    let env = TestEnv::start("android");
    let human = env.cli(&["doctor"]);
    let names = [
        "daemon",
        "drivers",
        "xcode",
        "simulator-runtimes",
        "ios-signing",
        "ios-runner",
        "ios-devices",
        "android-sdk",
        "adb-server",
        "emulator-control",
        "sfw",
        "just",
        "xcode-compatibility",
    ];
    for name in names {
        let line = human
            .stdout
            .lines()
            .find(|line| line.split_whitespace().nth(1) == Some(name))
            .unwrap_or_else(|| panic!("no {name} line in:\n{}", human.stdout));
        let mark = line.split_whitespace().next().unwrap_or_default();
        assert!(["✓", "⚠", "✗", "-"].contains(&mark), "{line}");
    }
    assert!(
        human.success(),
        "only a failed check fails doctor:\n{}",
        human.stdout
    );

    let reply = env.cli(&["--json", "doctor"]).ok().json();
    let checks = reply["data"]["checks"].as_array().expect("checks");
    let listed: Vec<&str> = checks.iter().filter_map(|c| c["name"].as_str()).collect();
    assert_eq!(listed, names);
    for check in checks {
        let status = check["status"].as_str().unwrap_or_default();
        assert!(["ok", "warn", "fail", "skip"].contains(&status), "{check}");
    }
    assert!(reply["data"]["compatibility"]["rows"].is_array());
}

/// Writes an executable shell script at `path` that logs its arguments to
/// `path` with `.log` added, a line per run, then runs `body`.
fn script(path: &Path, body: &str) {
    let dir = path.parent().expect("the script has a directory");
    std::fs::create_dir_all(dir).expect("the script's directory can be created");
    let log = format!("{}.log", path.display());
    std::fs::write(
        path,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\n{body}\n"),
    )
    .expect("the script can be written");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("the script can be made executable");
}

/// Fake tools for the real command: `xcode-select`, `xcodebuild` and
/// `xcrun` alone on `PATH`, an SDK whose `adb` answers `version`, and
/// `devices` with nothing attached, and an adb server on a port of its own
/// that answers `host:version` with protocol 41 and counts its
/// connections. Each tool logs its arguments beside itself.
struct FakeTools {
    bin: PathBuf,
    sdk: PathBuf,
    port: u16,
    connections: Arc<AtomicUsize>,
}

impl FakeTools {
    fn new(dir: &Path) -> FakeTools {
        let bin = dir.join("fake-bin");
        let sdk = dir.join("fake-sdk");
        script(
            &bin.join("xcode-select"),
            "echo /Applications/Xcode.app/Contents/Developer",
        );
        script(
            &bin.join("xcodebuild"),
            r"printf 'Xcode 27.0\nBuild version 27A266a\n'",
        );
        script(
            &bin.join("xcrun"),
            r#"echo '{"runtimes": [{"name": "iOS 26.5", "isAvailable": true}]}'"#,
        );
        script(
            &sdk.join("platform-tools/adb"),
            r"case $1 in
version) printf 'Android Debug Bridge version 1.0.41\nVersion 36.0.0-13206524\n' ;;
devices) printf 'List of devices attached\n\n' ;;
esac",
        );
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a port for the adb server");
        let port = listener.local_addr().expect("the server's address").port();
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&connections);
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                counted.fetch_add(1, Ordering::SeqCst);
                let mut request = [0u8; 16];
                if stream.read_exact(&mut request).is_ok() && &request == b"000chost:version" {
                    let _ = stream.write_all(b"OKAY00040029");
                }
            }
        });
        FakeTools {
            bin,
            sdk,
            port,
            connections,
        }
    }

    /// Runs the CLI with `args` against the fakes, with `drivers` as the
    /// run's `MOBILE_AGENT_DRIVERS`.
    fn cli(&self, env: &TestEnv, args: &[&str], drivers: &str) -> Run {
        let path = self.bin.to_string_lossy().into_owned();
        let sdk = self.sdk.to_string_lossy().into_owned();
        let port = self.port.to_string();
        env.cli_env(
            args,
            &[
                ("PATH", path.as_str()),
                ("ANDROID_HOME", sdk.as_str()),
                ("ANDROID_SDK_ROOT", sdk.as_str()),
                ("ANDROID_ADB_SERVER_PORT", port.as_str()),
                ("MOBILE_AGENT_DRIVERS", drivers),
            ],
        )
    }

    /// The arguments of each run of `tool`, a line per run; empty when it
    /// never ran.
    fn asked(&self, tool: &str) -> String {
        let log = match tool {
            "adb" => self.sdk.join("platform-tools/adb.log"),
            _ => self.bin.join(format!("{tool}.log")),
        };
        std::fs::read_to_string(log).unwrap_or_default()
    }

    /// How many connections the adb server took.
    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

/// The line doctor's text gives check `name`.
fn line_of<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find(|line| line.split_whitespace().nth(1) == Some(name))
        .unwrap_or_else(|| panic!("no {name} line in:\n{text}"))
}

/// Check `name` in doctor's JSON reply.
fn check_of<'a>(reply: &'a Value, name: &str) -> &'a Value {
    reply["data"]["checks"]
        .as_array()
        .and_then(|checks| checks.iter().find(|c| c["name"] == name))
        .unwrap_or_else(|| panic!("no {name} check in {reply}"))
}

/// Asserts that both outputs skip each of `names` as not consulted, naming
/// the run's `MOBILE_AGENT_DRIVERS`.
#[track_caller]
fn assert_not_consulted(text: &str, reply: &Value, names: [&str; 3], drivers: &str) {
    let variable = format!("MOBILE_AGENT_DRIVERS={drivers}");
    for name in names {
        let line = line_of(text, name);
        assert!(
            line.starts_with("- ") && line.contains("not consulted") && line.contains(&variable),
            "{line}"
        );
        let check = check_of(reply, name);
        assert_eq!(check["status"], "skip", "{check}");
        assert!(
            check["message"]
                .as_str()
                .is_some_and(|m| m.contains("not consulted") && m.contains(&variable)),
            "{check}"
        );
    }
}

/// Task 10.24: a run that names the Simulator's driver alone asks nothing
/// of adb, as the daemon it starts does not. No adb runs and no adb server
/// is asked; both outputs skip the Android checks as not consulted, while
/// on macOS the Simulator's checks still ask Xcode.
#[test]
fn the_android_checks_are_not_consulted_when_the_run_names_the_simulator_alone() {
    let env = TestEnv::start("android");
    let tools = FakeTools::new(env.path());
    let human = tools.cli(&env, &["doctor"], "simulator").ok();
    let reply = tools
        .cli(&env, &["--json", "doctor"], "simulator")
        .ok()
        .json();
    assert_eq!(
        (tools.asked("adb").as_str(), tools.connections()),
        ("", 0),
        "doctor ran adb or asked its server:\n{}",
        human.stdout
    );
    assert_not_consulted(&human.stdout, &reply, ANDROID_CHECKS, "simulator");
    if cfg!(target_os = "macos") {
        assert_eq!(check_of(&reply, "xcode")["status"], "ok", "{reply}");
        assert_eq!(
            check_of(&reply, "simulator-runtimes")["message"],
            "iOS 26.5",
            "{reply}"
        );
        assert!(tools.asked("xcode-select").contains("-p"));
    }
}

/// Task 10.24: a run that names Android's driver alone asks nothing of
/// Xcode. Both outputs skip the Simulator's checks as not consulted, while
/// the Android checks run adb and ask its server.
#[test]
fn the_simulator_checks_are_not_consulted_when_the_run_names_android_alone() {
    let env = TestEnv::start("android");
    let tools = FakeTools::new(env.path());
    let human = tools.cli(&env, &["doctor"], "android").ok();
    let reply = tools
        .cli(&env, &["--json", "doctor"], "android")
        .ok()
        .json();
    for tool in ["xcode-select", "xcodebuild", "xcrun"] {
        assert_eq!(
            tools.asked(tool),
            "",
            "doctor ran {tool}:\n{}",
            human.stdout
        );
    }
    assert_not_consulted(&human.stdout, &reply, SIMULATOR_CHECKS, "android");
    assert_eq!(check_of(&reply, "android-sdk")["status"], "ok", "{reply}");
    assert_eq!(check_of(&reply, "adb-server")["status"], "ok", "{reply}");
    assert_eq!(tools.asked("adb"), "version\ndevices\nversion\ndevices\n");
    assert_eq!(tools.connections(), 2, "each run asks the adb server once");
}
