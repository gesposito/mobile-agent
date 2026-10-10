//! Task 6.2: the harness's `ios-device` class. A physical iPhone is a target
//! `mobile-agent devices` lists in the `ios-device` class. Its run takes the
//! iOS fixtures, installs their device builds through CoreDevice when `run
//! --ios-app` names them, measures tap to frame change from the frames the
//! phone gives, and records the runner, the input and frame paths, the
//! battery and the Auto-Lock the person running stated. The tests run the
//! bench against the daemon's fake iPhone, with a fake `xcrun` on a `PATH`
//! of their own.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bench::fixture::Fixture;
use bench::gates::GateStatus;
use bench::harness::Subject;
use bench::primitive::Primitive;
use bench::report::Report;
use bench::target::{Battery, TargetClass};
use common::FakeDevices;
use serde_json::Value;

/// The fake iPhone the daemon serves.
const IPHONE: &str = "FAKE-IOS-DEVICE-7";

/// The runner the fake iPhone's status names: the one `agent-ios` builds.
fn runner() -> String {
    format!(
        "mobile-agent-ios-runner {}",
        include_str!("../../../agent-ios/VERSION").trim()
    )
}

/// `PATH` holding `bin` and the system directories, and none of the places a
/// real tool is installed.
fn hermetic_path(bin: &Path) -> String {
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
}

/// A fake `xcrun` in `bin` that logs its arguments to `xcrun.log` and answers
/// `devicectl device install app` as CoreDevice does with `--json-output -`:
/// the bundle identifier of the fixture the bundle's name gives, or, while
/// `install-fails` exists beside it, CoreDevice's error. Anything else goes
/// to the system's `xcrun`.
fn fake_xcrun(bin: &Path) {
    fs::create_dir_all(bin).unwrap();
    let path = bin.join("xcrun");
    fs::write(
        &path,
        r#"#!/bin/sh
dir="${0%/*}"
echo "$@" >> "$dir/xcrun.log"
if [ "$1 $2 $3 $4" != "devicectl device install app" ]; then
  exec /usr/bin/xcrun "$@"
fi
for bundle; do :; done
case "$bundle" in
  *FixtureSwiftUI.app) id=dev.mobileagent.fixture.swiftui ;;
  *FixtureExpo.app) id=dev.mobileagent.fixture.expo ;;
  *) id=dev.example.other ;;
esac
if [ -e "$dir/install-fails" ]; then
  echo '{"error": {"code": 3002, "domain": "com.apple.dt.CoreDeviceError", "userInfo": {"NSLocalizedDescription": {"string": "Failed to install the app on the device."}, "NSUnderlyingError": {"error": {"code": 3000, "domain": "com.apple.dt.CoreDeviceError", "userInfo": {"NSLocalizedDescription": {"string": "The device is locked."}}}}}}, "info": {"outcome": "failed"}}'
  echo "ERROR: Failed to install the app on the device. (com.apple.dt.CoreDeviceError error 3002 (0xBBA))" >&2
  exit 1
fi
sleep 0.05
echo "{\"info\": {\"outcome\": \"success\"}, \"result\": {\"installedApplications\": [{\"bundleID\": \"$id\"}]}}"
"#,
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A device build's bundle, `name` (such as `FixtureSwiftUI.app`), under `dir`.
fn bundle(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join("Build/Products/Debug-iphoneos").join(name);
    fs::create_dir_all(&path).unwrap();
    path
}

/// `mobile-agent-bench run <args>` in `devices`' environment with `bin`
/// first on `PATH`.
fn bench_run(devices: &FakeDevices, bin: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"));
    for (key, value) in devices.env() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    command
        .env("PATH", hermetic_path(bin))
        .arg("run")
        .args(args)
        .current_dir(&devices.dir)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The lines the fake `xcrun` logged.
fn xcrun_log(bin: &Path) -> Vec<String> {
    fs::read_to_string(bin.join("xcrun.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// A run on the fake iPhone of the SwiftUI fixture's warm snapshot and taps,
/// with the fixture's device build installed first and Auto-Lock stated off.
#[test]
fn a_run_on_an_iphone_installs_the_fixture_and_records_the_phone_s_provenance() {
    let devices = FakeDevices::new(&format!("ios-device:{IPHONE}"));
    let dir = common::scratch("ios-device-run");
    let bin = dir.join("bin");
    fake_xcrun(&bin);
    let app = bundle(&dir, "FixtureSwiftUI.app");
    let out = dir.join("report");
    let cli = common::mobile_agent();
    let ios_app = format!("swiftui={}", app.display());
    let output = bench_run(
        &devices,
        &bin,
        &[
            "--target",
            IPHONE,
            "--fixture",
            "swiftui",
            "--primitive",
            "warm-snapshot",
            "--primitive",
            "tap-dispatch",
            "--primitive",
            "tap-to-frame-change",
            "--ios-app",
            &ios_app,
            "--auto-lock-off",
            "--no-baselines",
            "--cli",
            cli.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--iterations",
            "1",
            "--warmup",
            "0",
            "--retries",
            "0",
        ],
    );
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {stderr}"
    );

    // The device build went in once, through CoreDevice, before the run.
    let installs: Vec<String> = xcrun_log(&bin)
        .into_iter()
        .filter(|line| line.starts_with("devicectl device install app"))
        .collect();
    assert_eq!(
        installs,
        [format!(
            "devicectl device install app --device {IPHONE} --json-output - {}",
            app.display()
        )]
    );

    let json = fs::read_to_string(out.join("report.json")).unwrap();
    let md = fs::read_to_string(out.join("report.md")).unwrap();
    let value: Value = serde_json::from_str(&json).unwrap();
    common::assert_every_markdown_number_is_in_the_json(&md, &value);
    let report = Report::from_json(&json).unwrap();

    let target = &report.provenance.targets[0];
    assert_eq!(target.class, TargetClass::IosDevice);
    assert_eq!(target.model.as_deref(), Some("fake"));
    assert_eq!(target.os.as_deref(), Some("iOS 26.6.2"));
    assert_eq!(target.frame_source, Some(true));
    assert_eq!(target.runner.as_deref(), Some(runner().as_str()));
    assert_eq!(target.input_path.as_deref(), Some("dtuhidd"));
    assert_eq!(target.frame_path.as_deref(), Some("screenshots"));
    assert_eq!(
        target.battery,
        Some(Battery {
            level_pct: Some(76),
            charging: Some(true),
            external_power: Some(true),
        })
    );
    assert_eq!(target.auto_lock_off, Some(true));
    assert_eq!(target.installs.len(), 1, "{:?}", target.installs);
    let install = &target.installs[0];
    assert_eq!(install.fixture, Fixture::SwiftUi);
    assert_eq!(install.bundle, app.display().to_string());
    assert!(install.elapsed_us >= 50_000, "{install:?}");
    assert_eq!(value["provenance"]["targets"][0]["class"], "ios-device");

    // The phone gives frames, so a tap's frame change is measured.
    let frames = report
        .measurements
        .iter()
        .find(|m| m.subject == Subject::Primitive(Primitive::TapToFrameChange))
        .unwrap();
    assert_eq!(frames.not_measured, None);
    assert_eq!(frames.target_class, TargetClass::IosDevice);
    assert_eq!(frames.stats.map(|s| s.n), Some(1), "{:?}", frames.samples);

    // Physical-ios, task 9.4: the warm snapshot's sample carries what the
    // daemon's reply says it cost, served from the replica, and the gate
    // table shows it beside the gates.
    let warm = report
        .measurements
        .iter()
        .find(|m| m.subject == Subject::Primitive(Primitive::WarmSnapshot))
        .unwrap();
    let sample = warm.samples.iter().find(|s| !s.warmup).unwrap();
    assert_eq!(sample.served_from_replica, Some(true), "{sample:?}");
    assert!(sample.read_us.is_some(), "{sample:?}");
    let reads = &report.gates.as_ref().unwrap().reads;
    assert_eq!(reads.len(), 1, "{reads:?}");
    assert_eq!(reads[0].primitive, Primitive::WarmSnapshot);
    assert_eq!(reads[0].reads.replica.map(|s| s.n), Some(1));
    assert_eq!(reads[0].reads.device, None);
    let read_table = common::table(&md, "Snapshot reads on physical iPhones");
    assert_eq!(read_table.rows.len(), 1, "{md}");

    // The gates are the iPhone's, tap to frame change without a verdict.
    let gates = report.gates.as_ref().unwrap();
    assert!(gates.passed);
    let status = |name: &str| {
        gates
            .rows
            .iter()
            .find(|r| r.gate == name)
            .map(|r| r.status)
            .unwrap_or_else(|| panic!("no gate {name}"))
    };
    assert_eq!(
        status("ios-device tap-to-frame-change"),
        GateStatus::Reported
    );
    assert_eq!(status("ios-device tap-dispatch"), GateStatus::Pass);
    assert_eq!(status("ios-device warm-snapshot big"), GateStatus::Pass);
    assert!(
        stdout.contains("ios-device tap-to-frame-change"),
        "{stdout}"
    );

    // The summary's table of physical iPhones.
    let table = common::table(&md, "Physical iPhones");
    assert_eq!(table.rows.len(), 1, "{md}");
    let row = &table.rows[0];
    assert_eq!(table.cell(row, "Target"), IPHONE);
    assert_eq!(table.cell(row, "Runner"), runner());
    assert_eq!(table.cell(row, "Input"), "dtuhidd");
    assert_eq!(table.cell(row, "Frames"), "screenshots");
    assert_eq!(
        table.cell(row, "Battery"),
        "76%, charging, on external power"
    );
    assert_eq!(table.cell(row, "Auto-Lock"), "off, as stated");
    assert_eq!(
        table.cell(row, "Installed before the run"),
        format!("swiftui in {} ms", common::ms(install.elapsed_us))
    );
}

/// A run whose device build does not install measures nothing: it stops
/// with CoreDevice's reason before any series.
#[test]
fn a_build_that_does_not_install_refuses_the_run_before_anything_is_measured() {
    let devices = FakeDevices::new(&format!("ios-device:{IPHONE}"));
    let dir = common::scratch("ios-device-install-fails");
    let bin = dir.join("bin");
    fake_xcrun(&bin);
    fs::write(bin.join("install-fails"), "").unwrap();
    let app = bundle(&dir, "FixtureSwiftUI.app");
    let out = dir.join("report");
    let cli = common::mobile_agent();
    let ios_app = format!("swiftui={}", app.display());
    let output = bench_run(
        &devices,
        &bin,
        &[
            "--target",
            IPHONE,
            "--primitive",
            "warm-snapshot",
            "--ios-app",
            &ios_app,
            "--no-baselines",
            "--cli",
            cli.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ],
    );
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "cannot install the swiftui fixture's build {} on {IPHONE}: Failed to install the \
             app on the device.: The device is locked.",
            app.display()
        )),
        "{stderr}"
    );
    assert!(!out.join("report.json").exists());
}

/// A bundle of another app installs, but the run stops before it measures
/// the wrong app under the fixture's name.
#[test]
fn a_bundle_of_another_app_refuses_the_run() {
    let devices = FakeDevices::new(&format!("ios-device:{IPHONE}"));
    let dir = common::scratch("ios-device-other-app");
    let bin = dir.join("bin");
    fake_xcrun(&bin);
    let app = bundle(&dir, "FixtureExpo.app");
    let cli = common::mobile_agent();
    let ios_app = format!("swiftui={}", app.display());
    let output = bench_run(
        &devices,
        &bin,
        &[
            "--target",
            IPHONE,
            "--primitive",
            "warm-snapshot",
            "--ios-app",
            &ios_app,
            "--no-baselines",
            "--cli",
            cli.to_str().unwrap(),
        ],
    );
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains(
            "installed dev.mobileagent.fixture.expo, not the swiftui fixture's \
             dev.mobileagent.fixture.swiftui"
        ),
        "{stderr}"
    );
}

/// `--ios-app` takes a fixture an iPhone runs and a bundle, and a run with a
/// physical iPhone to install it on; anything else is refused before the
/// run.
#[test]
fn ios_app_takes_an_iphone_fixture_and_a_bundle_and_a_run_with_an_iphone() {
    let devices = FakeDevices::new(&format!("ios-device:{IPHONE},ios:FAKE-IOS-9"));
    let dir = common::scratch("ios-device-refusals");
    let bin = dir.join("bin");
    fake_xcrun(&bin);
    let app = bundle(&dir, "FixtureSwiftUI.app");
    let cli = common::mobile_agent();
    let refused = |target: &str, ios_app: &str| {
        let output = bench_run(
            &devices,
            &bin,
            &[
                "--target",
                target,
                "--primitive",
                "warm-snapshot",
                "--ios-app",
                ios_app,
                "--no-baselines",
                "--cli",
                cli.to_str().unwrap(),
            ],
        );
        (output.status.code(), text(&output.stderr))
    };
    let swiftui = format!("swiftui={}", app.display());
    for (target, ios_app, says) in [
        (
            IPHONE,
            format!("compose={}", app.display()),
            "the compose fixture has no iPhone build".to_owned(),
        ),
        (
            IPHONE,
            "swiftui".to_owned(),
            "--ios-app takes <fixture>=<path>, such as swiftui=FixtureSwiftUI.app".to_owned(),
        ),
        (
            IPHONE,
            format!("expo={}", dir.join("missing.app").display()),
            format!("{} is not a directory", dir.join("missing.app").display()),
        ),
        (
            "FAKE-IOS-9",
            swiftui.clone(),
            "--ios-app installs on a physical iPhone, and the run has no ios-device target"
                .to_owned(),
        ),
    ] {
        let (code, stderr) = refused(target, &ios_app);
        assert_eq!(code, Some(2), "{ios_app}: {stderr}");
        assert!(stderr.contains(&says), "{ios_app}: {stderr}");
    }
    assert!(
        xcrun_log(&bin)
            .iter()
            .all(|line| !line.starts_with("devicectl device install app")),
        "{:?}",
        xcrun_log(&bin)
    );
}
