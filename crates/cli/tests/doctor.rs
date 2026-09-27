//! `doctor`: each check against scripted command outputs, and the real
//! command listing every check with a status.

mod support;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cli::doctor::{Output, Probe, Report, Status, run_checks};
use serde_json::{Value, json};
use support::TestEnv;

/// A host described by scripted answers.
struct Host {
    os: &'static str,
    env: HashMap<&'static str, String>,
    commands: HashMap<String, Output>,
    files: Vec<PathBuf>,
    dirs: HashMap<PathBuf, Vec<(PathBuf, String)>>,
    adb: Result<u32, String>,
    daemon: Result<Value, String>,
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
        self.commands.get(&line.join(" ")).cloned()
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|f| f == path)
    }

    fn files(&self, dir: &Path) -> Vec<(PathBuf, String)> {
        self.dirs.get(dir).cloned().unwrap_or_default()
    }

    fn adb_server(&self, _port: u16) -> Result<u32, String> {
        self.adb.clone()
    }

    fn daemon(&self) -> Result<Value, String> {
        self.daemon.clone()
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

#[test]
fn doctor_lists_every_check_with_a_status_in_both_outputs() {
    let env = TestEnv::start("android");
    let human = env.cli(&["doctor"]);
    let names = [
        "daemon",
        "drivers",
        "xcode",
        "simulator-runtimes",
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
