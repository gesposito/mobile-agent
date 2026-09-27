//! Task 9.3: baseline adapters for Maestro, maestro-runner and agent-device.
//! An absent tool is noted as skipped and the tools that ran are still
//! compared. Fake executables stand in for the tools, found through a search
//! path the test controls, never the host's `PATH`.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use bench::adapter::{Clock, Role};
use bench::baseline::{BaselineConfig, agent_device, baselines, maestro};
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::Fixture;
use bench::flow::{Flow, Selector, Step};
use bench::harness::{Harness, RunConfig, Subject};
use bench::provenance::ToolStatus;
use bench::report::{Outcome, Report};
use bench::target::{Target, TargetClass};
use serde_json::Value;

const ITERATIONS: u32 = 5;
const WARMUP: u32 = 1;

/// A tool that answers `--version`, logs its arguments next to itself and
/// then runs `body`.
fn fake_tool(dir: &Path, name: &str, version: &str, body: &str) {
    let path = dir.join(name);
    fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then echo '{version}'; exit 0; fi\n\
             echo \"$@\" >> \"${{0%/*}}/{name}.log\"\n\
             {body}\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

const MAESTRO_PASSES: &str = "echo '[Passed] tab-list'";
const AGENT_DEVICE_PASSES: &str =
    r#"echo '{"success":true,"data":{"total":3,"executed":3,"totalDurationMs":42,"results":[]}}'"#;

/// Opens the List tab of the Expo fixture.
fn tab_flow() -> Flow {
    Flow {
        name: "tab-list".into(),
        fixture: Fixture::Expo,
        steps: vec![
            Step::Launch,
            Step::Tap(Selector::id("tab-list")),
            Step::AssertVisible(Selector::id("row-1")),
        ],
        count_nodes: false,
    }
}

/// `mobile-agent`, scripted, running the flow on the emulator.
fn script() -> FakeScript {
    FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![Target::new("emulator-5554", TargetClass::AndroidEmulator)],
        series: vec![FakeSeries::new(
            "emulator-5554",
            Some(Fixture::Expo),
            None,
            Subject::Flow("tab-list".into()),
            vec![300.0, 310.0, 320.0],
        )],
        flows: vec![tab_flow()],
    }
}

struct Setup {
    dir: PathBuf,
    bin: PathBuf,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = common::scratch(name);
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        Setup { dir, bin }
    }

    fn config(&self) -> BaselineConfig {
        BaselineConfig {
            search_path: vec![self.bin.clone()],
            work_dir: self.dir.join("work"),
            flow_timeout: Duration::from_secs(30),
            version_timeout: Duration::from_secs(10),
        }
    }

    fn run(&self) -> Report {
        let fake = FakeAdapter::new(script());
        let mut harness = Harness::new(
            RunConfig {
                iterations: ITERATIONS,
                warmup: WARMUP,
                retries: 0,
            },
            fake.targets(),
            fake.series(),
        )
        .flows(fake.flows())
        .tool(Box::new(fake));
        for baseline in baselines(&self.config()) {
            harness = harness.tool(baseline);
        }
        harness
            .probe(Box::new(common::probe()), "macos")
            .run()
            .unwrap()
    }

    fn log(&self, tool: &str) -> String {
        fs::read_to_string(self.bin.join(format!("{tool}.log"))).unwrap_or_default()
    }
}

#[test]
fn an_absent_tool_is_noted_as_skipped_and_the_tools_that_ran_are_still_compared() {
    let setup = Setup::new("baselines-one-missing");
    fake_tool(&setup.bin, "maestro", "2.10.0", MAESTRO_PASSES);
    fake_tool(&setup.bin, "agent-device", "0.21.14", AGENT_DEVICE_PASSES);
    let report = setup.run();

    let tools = &report.provenance.tools;
    let tool = |name: &str| tools.iter().find(|t| t.name == name).unwrap();
    assert_eq!(tool("maestro-runner").status, ToolStatus::Skipped);
    assert_eq!(
        tool("maestro-runner").note.as_deref(),
        Some("maestro-runner not found on PATH")
    );
    assert_eq!(tool("maestro").status, ToolStatus::Available);
    assert_eq!(tool("maestro").version.as_deref(), Some("2.10.0"));
    assert_eq!(tool("maestro").role, Role::Baseline);
    assert_eq!(tool("agent-device").version.as_deref(), Some("0.21.14"));

    let measured: Vec<&str> = report
        .measurements
        .iter()
        .map(|m| m.tool.as_str())
        .collect();
    assert_eq!(measured, ["mobile-agent", "maestro", "agent-device"]);
    for m in &report.measurements {
        assert_eq!(m.subject, Subject::Flow("tab-list".into()));
        assert_eq!(m.stats.map(|s| s.n), Some(ITERATIONS), "{}", m.tool);
    }
    let agent_device = &report.measurements[2];
    assert_eq!(agent_device.clock, Some(Clock::Wall));
    assert_eq!(
        agent_device.samples[0].detail.as_deref(),
        Some("agent-device reported 42 ms")
    );

    let md = report.to_markdown();
    let tools_table = common::table(&md, "Tools");
    let runner = tools_table
        .rows
        .iter()
        .find(|r| r[0] == "maestro-runner")
        .unwrap();
    assert_eq!(tools_table.cell(runner, "Status"), "skipped");
    assert_eq!(
        tools_table.cell(runner, "Note"),
        "maestro-runner not found on PATH"
    );
    let comparison = common::table(&md, "Comparison");
    assert_eq!(
        comparison.header,
        [
            "Target",
            "Fixture",
            "Screen",
            "Subject",
            "mobile-agent",
            "maestro",
            "agent-device"
        ]
    );
    assert_eq!(comparison.rows.len(), 1);
    assert_eq!(comparison.rows[0][3], "flow tab-list");
    assert!(
        md.contains("Skipped: maestro-runner (maestro-runner not found on PATH)."),
        "{md}"
    );
    let json: Value = serde_json::from_str(&report.to_json()).unwrap();
    common::assert_every_markdown_number_is_in_the_json(&md, &json);

    // Each tool received the flow in its documented shape, once per iteration.
    let work = setup.dir.join("work");
    let yaml = work.join("maestro-tab-list-emulator-5554.yaml");
    let maestro_log = setup.log("maestro");
    assert_eq!(maestro_log.lines().count(), (WARMUP + ITERATIONS) as usize);
    assert_eq!(
        maestro_log.lines().next().unwrap(),
        format!("--device emulator-5554 test {}", yaml.display())
    );
    assert_eq!(
        fs::read_to_string(&yaml).unwrap(),
        maestro::flow_yaml(&tab_flow(), "dev.mobileagent.fixture.expo")
    );
    let steps = work.join("agent-device-tab-list-emulator-5554.json");
    assert_eq!(
        setup.log("agent-device").lines().next().unwrap(),
        format!(
            "batch --steps-file {} --on-error stop --json --platform android \
             --serial emulator-5554 --session mobile-agent-bench",
            steps.display()
        )
    );
    assert_eq!(
        fs::read_to_string(&steps).unwrap(),
        agent_device::steps(
            &tab_flow(),
            "dev.mobileagent.fixture.expo",
            bench::target::Platform::Android
        )
    );
}

#[test]
fn every_baseline_is_skipped_when_none_is_installed() {
    let setup = Setup::new("baselines-none");
    let report = setup.run();
    for name in ["maestro", "maestro-runner", "agent-device"] {
        let tool = report
            .provenance
            .tools
            .iter()
            .find(|t| t.name == name)
            .unwrap();
        assert_eq!(tool.status, ToolStatus::Skipped);
        assert_eq!(tool.note, Some(format!("{name} not found on PATH")));
    }
    assert!(report.measurements.iter().all(|m| m.tool == "mobile-agent"));
    assert!(report.to_markdown().contains(
        "Skipped: maestro (maestro not found on PATH); maestro-runner (maestro-runner not \
         found on PATH); agent-device (agent-device not found on PATH)."
    ));
}

#[test]
fn a_flow_the_tool_fails_is_a_failed_iteration_with_its_output() {
    let setup = Setup::new("baselines-maestro-fails");
    fake_tool(
        &setup.bin,
        "maestro",
        "2.10.0",
        "echo 'Running tab-list'; echo 'Assertion is false: id: row-1 is visible' >&2; exit 1",
    );
    let report = setup.run();
    let m = report
        .measurements
        .iter()
        .find(|m| m.tool == "maestro")
        .unwrap();
    assert_eq!(m.stats, None);
    assert!(m.samples.iter().all(|s| s.outcome == Outcome::Fail));
    let error = &m.samples[1].errors[0];
    assert!(error.contains("exit status 1"), "{error}");
    assert!(
        error.contains("Assertion is false: id: row-1 is visible"),
        "{error}"
    );
    let comparison = common::table(&report.to_markdown(), "Comparison");
    let column = comparison
        .header
        .iter()
        .position(|h| h == "maestro")
        .unwrap();
    assert_eq!(comparison.rows[0][column], "no samples");
}

#[test]
fn agent_device_failures_are_read_from_its_json_reply() {
    let setup = Setup::new("baselines-agent-device-fails");
    fake_tool(
        &setup.bin,
        "agent-device",
        "0.21.14",
        r#"echo '{"success":false,"error":{"code":"ELEMENT_NOT_FOUND","message":"no element matches"},"data":{"results":[{"step":1,"command":"open","ok":true},{"step":2,"command":"press","ok":false}]}}'; exit 1"#,
    );
    let report = setup.run();
    let m = report
        .measurements
        .iter()
        .find(|m| m.tool == "agent-device")
        .unwrap();
    assert_eq!(m.stats, None);
    assert_eq!(
        m.samples[0].errors,
        ["step 2 (press) failed: ELEMENT_NOT_FOUND: no element matches"]
    );
}

/// `PATH` holding the fake tools and the system directories the harness's
/// own probes need, and none of the places a real baseline is installed.
fn hermetic_path(bin: &Path) -> String {
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
}

#[test]
fn a_run_through_the_binary_includes_the_installed_baselines() {
    let setup = Setup::new("baselines-binary-run");
    fake_tool(&setup.bin, "maestro", "2.10.0", MAESTRO_PASSES);
    let script_path = setup.dir.join("script.json");
    fs::write(&script_path, serde_json::to_string(&script()).unwrap()).unwrap();
    let out = setup.dir.join("report");
    let output = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
        .arg("run")
        .arg("--fake")
        .arg(&script_path)
        .arg("--out")
        .arg(&out)
        .args(["--iterations", "3", "--warmup", "0"])
        .env("PATH", hermetic_path(&setup.bin))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    let report = Report::from_json(&fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    let skipped: Vec<&str> = report
        .provenance
        .tools
        .iter()
        .filter(|t| t.status == ToolStatus::Skipped)
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(skipped, ["maestro-runner", "agent-device"]);
    let measured: Vec<&str> = report
        .measurements
        .iter()
        .map(|m| m.tool.as_str())
        .collect();
    assert_eq!(measured, ["mobile-agent", "maestro"]);
    assert!(
        out.join("flows/maestro-tab-list-emulator-5554.yaml")
            .is_file()
    );
}

#[test]
fn the_baselines_command_lists_each_tool_and_notes_absent_ones() {
    let setup = Setup::new("baselines-command");
    fake_tool(&setup.bin, "maestro", "2.10.0", MAESTRO_PASSES);
    let output = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
        .arg("baselines")
        .env("PATH", hermetic_path(&setup.bin))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let table = common::table(&format!("# Baselines\n\n{stdout}"), "Baselines");
    let row = |name: &str| table.rows.iter().find(|r| r[0] == name).unwrap().clone();
    assert_eq!(row("maestro"), ["maestro", "available", "2.10.0", "—"]);
    assert_eq!(
        row("maestro-runner"),
        [
            "maestro-runner",
            "skipped",
            "—",
            "maestro-runner not found on PATH"
        ]
    );
    assert_eq!(
        row("agent-device"),
        [
            "agent-device",
            "skipped",
            "—",
            "agent-device not found on PATH"
        ]
    );
}

/// A run of `series` on the script's emulator, with the tab-list flow.
fn check(series: Vec<bench::harness::Series>) -> Result<Report, String> {
    let fake = FakeAdapter::new(script());
    Harness::new(RunConfig::default(), fake.targets(), series)
        .flows(fake.flows())
        .tool(Box::new(fake))
        .probe(Box::new(common::probe()), "macos")
        .run()
}

#[test]
fn a_series_the_run_cannot_carry_out_is_refused_before_anything_runs() {
    use bench::fixture::Screen;
    use bench::harness::Series;
    let series = |fixture, screen, subject| Series {
        target: "emulator-5554".into(),
        fixture,
        screen,
        subject,
    };
    let unknown_flow = check(vec![series(
        Some(Fixture::Expo),
        None,
        Subject::Flow("checkout".into()),
    )])
    .unwrap_err();
    assert!(unknown_flow.contains("flow checkout"), "{unknown_flow}");
    let wrong_fixture = check(vec![series(
        Some(Fixture::Compose),
        None,
        Subject::Flow("tab-list".into()),
    )])
    .unwrap_err();
    assert!(wrong_fixture.contains("expo"), "{wrong_fixture}");
    let with_screen = check(vec![series(
        Some(Fixture::Expo),
        Some(Screen::List),
        Subject::Flow("tab-list".into()),
    )]);
    assert!(with_screen.is_err());
    let swiftui_on_android = check(vec![series(
        Some(Fixture::SwiftUi),
        Some(Screen::Big),
        Subject::Primitive(bench::primitive::Primitive::WarmSnapshot),
    )])
    .unwrap_err();
    assert!(
        swiftui_on_android.contains("swiftui fixture on emulator-5554"),
        "{swiftui_on_android}"
    );
}
