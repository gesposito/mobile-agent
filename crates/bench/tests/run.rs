//! Task 9.6: `mobile-agent-bench run --target <id>` measures `mobile-agent`
//! through the CLI adapter against a daemon, on the fixtures of each target's
//! platform: the golden checks first, then the timed series, with the
//! baselines by default, into both reports. The tests run it on the daemon's
//! fake targets, with fake baseline tools, or none, on a `PATH` of their own.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bench::adapter::Role;
use bench::fixture::Fixture;
use bench::flow;
use bench::gates::GateSet;
use bench::harness::Subject;
use bench::plan;
use bench::provenance::ToolStatus;
use bench::report::Report;
use bench::target::{Platform, Target, TargetClass};
use common::FakeDevices;
use common::tools::{Tools, end_screen};
use serde_json::Value;

/// `PATH` holding `bin` and the system directories the harness's probes need,
/// and none of the places a real tool is installed.
fn hermetic_path(bin: &Path) -> String {
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
}

/// A gate file no fake run misses: every default gate at a minute.
fn relaxed_gates(dir: &Path) -> PathBuf {
    let mut gates = GateSet::defaults();
    for gate in &mut gates.gates {
        gate.limit_ms = 60_000.0;
    }
    let path = dir.join("gates.json");
    fs::write(&path, gates.to_json()).unwrap();
    path
}

/// `mobile-agent-bench run <args>` in `devices`' environment, whose daemon
/// serves its fake targets, with `path` as `PATH`.
fn bench_run(devices: &FakeDevices, path: &str, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"));
    for (key, value) in devices.env() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    command
        .env("PATH", path)
        .arg("run")
        .args(args)
        .current_dir(&devices.dir)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The report `run` wrote into `out`, checking that the markdown agrees with
/// the JSON on every number.
fn written(out: &Path) -> Report {
    let json = fs::read_to_string(out.join("report.json")).unwrap();
    let md = fs::read_to_string(out.join("report.md")).unwrap();
    let value: Value = serde_json::from_str(&json).unwrap();
    common::assert_every_markdown_number_is_in_the_json(&md, &value);
    Report::from_json(&json).unwrap()
}

/// Asserts that `report` measured mobile-agent on `target` with `fixtures`,
/// every series the plan lists, and read each flow's golden before timing it.
fn assert_measured_as_planned(report: &Report, target: &Target, fixtures: &[Fixture]) {
    let under_test: Vec<_> = report
        .measurements
        .iter()
        .filter(|m| m.tool == "mobile-agent")
        .collect();
    let measured: Vec<(Option<Fixture>, String)> = under_test
        .iter()
        .map(|m| (m.fixture, m.subject.to_string()))
        .collect();
    let planned: Vec<(Option<Fixture>, String)> = plan::series(target, fixtures)
        .iter()
        .map(|s| (s.fixture, s.subject.to_string()))
        .collect();
    assert_eq!(measured, planned);
    for m in under_test {
        assert_eq!(m.target, target.id);
        assert_eq!(m.error, None, "{} on {:?}", m.subject, m.fixture);
        assert_eq!(m.not_measured, None, "{} on {:?}", m.subject, m.fixture);
        assert_eq!(
            m.stats.map(|s| s.n),
            Some(1),
            "{} on {:?}: {:?}",
            m.subject,
            m.fixture,
            m.samples
        );
        if let Subject::Flow(name) = &m.subject {
            let golden = flow::reference(Fixture::Expo)
                .into_iter()
                .find(|f| f.name == *name)
                .unwrap()
                .golden;
            assert_eq!(m.end_state.as_ref(), Some(&golden), "{name}");
        }
    }
}

#[test]
fn a_run_measures_mobile_agent_on_a_fake_emulator_and_the_baselines_beside_it() {
    let devices = FakeDevices::new("android:emulator-5590");
    let tools = Tools::new("run-emulator", Platform::Android, end_screen);
    let out = tools.dir.join("report");
    let cli = common::mobile_agent();
    let gates = relaxed_gates(&tools.dir);
    let output = bench_run(
        &devices,
        &hermetic_path(&tools.bin),
        &[
            "--target",
            "emulator-5590",
            "--fixture",
            "expo",
            "--tool",
            "mobile-agent",
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
            "--gates",
            gates.to_str().unwrap(),
        ],
    );
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains(&format!(
            "wrote {} and {}",
            out.join("report.json").display(),
            out.join("report.md").display()
        )),
        "{stdout}"
    );
    assert!(stdout.contains("gate checks passed"), "{stdout}");
    let report = written(&out);

    let mut target = Target::new("emulator-5590", TargetClass::AndroidEmulator);
    target.model = Some("fake".into());
    target.os = Some("Android 16".into());
    assert_eq!(report.provenance.targets, [target.clone()]);
    let tool = |name: &str| {
        report
            .provenance
            .tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("no tool {name}"))
    };
    let mobile_agent = tool("mobile-agent");
    assert_eq!(
        (
            mobile_agent.role,
            mobile_agent.status,
            mobile_agent.version.as_deref()
        ),
        (
            Role::UnderTest,
            ToolStatus::Available,
            Some(env!("CARGO_PKG_VERSION"))
        )
    );
    assert_measured_as_planned(&report, &target, &[Fixture::Expo]);

    // The baselines run the same flows by default, each checked against its
    // golden before any of them is timed.
    for (name, big_nodes) in [
        ("maestro", 509),
        ("maestro-runner", 508),
        ("agent-device", 509),
    ] {
        assert_eq!(tool(name).status, ToolStatus::Available, "{name}");
        let flows: Vec<_> = report
            .measurements
            .iter()
            .filter(|m| m.tool == name)
            .collect();
        assert_eq!(flows.len(), 3, "{name}");
        for m in flows {
            let Subject::Flow(flow) = &m.subject else {
                panic!("{name} measured {}", m.subject)
            };
            assert!(m.end_state.is_some(), "{name} {flow}");
            assert_eq!(m.error, None, "{name} {flow}");
            assert_eq!(m.stats.map(|s| s.n), Some(1), "{name} {flow}");
            let nodes = (flow == "big-screen").then_some((big_nodes, big_nodes));
            assert_eq!(m.nodes.map(|n| (n.min, n.max)), nodes, "{name} {flow}");
        }
        let log = tools.log(name);
        assert_eq!(
            log[..6],
            [
                "run login",
                "read",
                "run list-scroll",
                "read",
                "run big-screen",
                "read"
            ],
            "{name}: {log:?}"
        );
    }
}

#[test]
fn a_run_on_a_fake_simulator_runs_swiftui_and_expo_through_the_cli_beside_the_bench() {
    let devices = FakeDevices::new("ios:FAKE-IOS-9");
    let dir = common::scratch("run-simulator");
    // No mobile-agent and no baseline on PATH: the run drives the
    // mobile-agent built beside mobile-agent-bench, and skips the baselines.
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    common::mobile_agent();
    let out = dir.join("report");
    let gates = relaxed_gates(&dir);
    let output = bench_run(
        &devices,
        &hermetic_path(&bin),
        &[
            "--target",
            "FAKE-IOS-9",
            "--out",
            out.to_str().unwrap(),
            "--iterations",
            "1",
            "--warmup",
            "0",
            "--retries",
            "0",
            "--gates",
            gates.to_str().unwrap(),
        ],
    );
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let report = written(&out);
    let mut target = Target::new("FAKE-IOS-9", TargetClass::Simulator);
    target.model = Some("fake".into());
    target.os = Some("iOS 26.5".into());
    assert_eq!(report.provenance.targets, [target.clone()]);
    assert_measured_as_planned(&report, &target, &[Fixture::SwiftUi, Fixture::Expo]);
    for name in ["maestro", "maestro-runner", "agent-device"] {
        let tool = report
            .provenance
            .tools
            .iter()
            .find(|t| t.name == name)
            .unwrap();
        assert_eq!(tool.status, ToolStatus::Skipped, "{name}");
    }
    assert!(report.measurements.iter().all(|m| m.tool == "mobile-agent"));
}

/// A failed run: its exit status and standard error.
fn refused(devices: &FakeDevices, args: &[&str]) -> (Option<i32>, String) {
    let dir = devices.dir.join("empty-bin");
    fs::create_dir_all(&dir).unwrap();
    let output = bench_run(devices, &hermetic_path(&dir), args);
    (output.status.code(), text(&output.stderr))
}

#[test]
fn a_target_mobile_agent_does_not_list_is_refused_naming_the_connected_ones() {
    let devices = FakeDevices::new("android:emulator-5590");
    let cli = common::mobile_agent();
    let out = common::scratch("run-unknown-target").join("report");
    let (code, stderr) = refused(
        &devices,
        &[
            "--target",
            "emulator-5592",
            "--cli",
            cli.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains(
            "no connected target is emulator-5592; mobile-agent devices lists emulator-5590 \
             (android-emulator)"
        ),
        "{stderr}"
    );
    assert!(!out.join("report.json").exists());
}

#[test]
fn a_fixture_the_target_s_platform_lacks_is_refused() {
    let devices = FakeDevices::new("android:emulator-5590");
    let cli = common::mobile_agent();
    let (code, stderr) = refused(
        &devices,
        &[
            "--target",
            "emulator-5590",
            "--fixture",
            "swiftui",
            "--cli",
            cli.to_str().unwrap(),
        ],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains(
            "the swiftui fixture does not run on emulator-5590, an android-emulator; its \
             fixtures are expo and compose"
        ),
        "{stderr}"
    );
}

#[test]
fn a_cli_that_is_not_there_is_refused() {
    let devices = FakeDevices::new("android:emulator-5590");
    let missing = devices.dir.join("nowhere").join("mobile-agent");
    let (code, stderr) = refused(
        &devices,
        &[
            "--target",
            "emulator-5590",
            "--cli",
            missing.to_str().unwrap(),
        ],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains(&format!("{} is not an executable file", missing.display())),
        "{stderr}"
    );
}

#[test]
fn run_takes_targets_or_a_fake_script_but_not_both() {
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
            .arg("run")
            .args(args)
            .output()
            .unwrap();
        (output.status.code(), text(&output.stderr))
    };
    let (code, stderr) = run(&[]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("--target <ID>"), "{stderr}");
    let (code, stderr) = run(&["--fake", "script.json", "--target", "emulator-5554"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("cannot be used with"), "{stderr}");
    let (code, stderr) = run(&["--target", "emulator-5554", "--tool", "maestro"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("invalid value 'maestro'"), "{stderr}");
}
