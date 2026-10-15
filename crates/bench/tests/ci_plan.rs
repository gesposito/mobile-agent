//! Task 3.3 of `ci-benchmarks`: `run --plan ci`, the harness's reduced plan
//! for a CI runner (design D4), one fixture per platform, SwiftUI on iOS and
//! Compose on Android, with 10 measured iterations after 2 warm-up; `--gates
//! none`, which judges no gate; and the ratios that stand in for the gates
//! there: per flow, target and fixture, how many times faster the tool under
//! test's p50 was than each baseline's in the same run, with the baselines
//! the run skipped and why, in both reports and in what the run prints.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bench::adapter::{Adapter, Detection, Role, Scenario};
use bench::end_state::EndState;
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::{Fixture, Screen};
use bench::flow;
use bench::harness::{Harness, RunConfig, Subject};
use bench::plan::{self, Plan};
use bench::primitive::Primitive;
use bench::ratio::{self, Ratios};
use bench::report::Report;
use bench::target::{Platform, Target, TargetClass};
use common::FakeDevices;
use common::tools::{Tools, end_screen};
use serde_json::Value;

/// `PATH` holding `bin` and the system directories the harness's probes need.
fn hermetic_path(bin: &Path) -> String {
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
}

/// `mobile-agent-bench run <args>` in `devices`' environment with `path` as
/// `PATH`.
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

/// The report `run` wrote into `out`, its markdown agreeing with the JSON on
/// every number, and the markdown.
fn written(out: &Path) -> (Report, String) {
    let json = fs::read_to_string(out.join("report.json")).unwrap();
    let md = fs::read_to_string(out.join("report.md")).unwrap();
    let value: Value = serde_json::from_str(&json).unwrap();
    common::assert_every_markdown_number_is_in_the_json(&md, &value);
    (Report::from_json(&json).unwrap(), md)
}

/// An empty directory for `PATH`, so that no baseline is found.
fn empty_bin(name: &str) -> PathBuf {
    let bin = common::scratch(name).join("bin");
    fs::create_dir_all(&bin).unwrap();
    bin
}

#[test]
fn the_ci_plan_measures_swiftui_on_a_simulator_and_compose_on_an_emulator_and_judges_no_gate() {
    let devices = FakeDevices::new("ios:FAKE-IOS-9,android:emulator-5590");
    let bin = empty_bin("ci-plan-two-targets");
    let out = bin.parent().unwrap().join("report");
    let cli = common::mobile_agent();
    let output = bench_run(
        &devices,
        &hermetic_path(&bin),
        &[
            "--plan",
            "ci",
            "--gates",
            "none",
            "--target",
            "FAKE-IOS-9",
            "--target",
            "emulator-5590",
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
    let (report, md) = written(&out);

    // Each target runs its platform's one fixture: the plan's series on it.
    for (id, class, fixture) in [
        ("FAKE-IOS-9", TargetClass::Simulator, Fixture::SwiftUi),
        (
            "emulator-5590",
            TargetClass::AndroidEmulator,
            Fixture::Compose,
        ),
    ] {
        let target = Target::new(id, class);
        let measured: Vec<(Option<Fixture>, String)> = report
            .measurements
            .iter()
            .filter(|m| m.target == id)
            .map(|m| (m.fixture, m.subject.to_string()))
            .collect();
        let planned: Vec<(Option<Fixture>, String)> = plan::series(&target, &[fixture])
            .iter()
            .map(|s| (s.fixture, s.subject.to_string()))
            .collect();
        assert_eq!(measured, planned, "{id}");
        assert_eq!(measured.len(), 13, "{id}");
    }
    assert!(report.measurements.iter().all(|m| m.error.is_none()));

    // No gate is judged: the report has no verdict and the run says so.
    assert_eq!(report.gates, None);
    assert!(!md.contains("## Gates"), "{md}");
    assert!(
        stdout.contains("No gate judged (--gates none)."),
        "{stdout}"
    );

    // No baseline is on PATH, and none drives a physical iPhone: each is
    // skipped, with why, in both reports and in what the run prints.
    let ratios = report.ratios.as_ref().expect("the run's ratios");
    assert_eq!(ratios.tool.as_deref(), Some("mobile-agent"));
    assert!(ratios.rows.is_empty(), "{:?}", ratios.rows);
    let skipped: Vec<&str> = ratios.skipped.iter().map(|s| s.tool.as_str()).collect();
    assert_eq!(
        skipped,
        [
            "maestro",
            "maestro-runner",
            "agent-device",
            "webdriveragent"
        ]
    );
    for printed in [&stdout, &md] {
        assert!(
            printed.contains("Skipped baselines: maestro (maestro not found on PATH); "),
            "{printed}"
        );
        assert!(
            printed.contains("webdriveragent (WebDriverAgent runs here on a physical iPhone only, and the run has none)."),
            "{printed}"
        );
    }
}

/// A fake tool under test on a Simulator: CLI start and the SwiftUI login
/// flow, each at a constant 4 ms and 4.1 s.
fn fake_script() -> FakeScript {
    FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![Target::new("SIM-1", TargetClass::Simulator)],
        series: vec![
            FakeSeries::new(
                "SIM-1",
                None,
                None,
                Subject::Primitive(Primitive::CliStart),
                vec![4.0],
            ),
            FakeSeries::new(
                "SIM-1",
                Some(Fixture::SwiftUi),
                Some(Screen::Big),
                Subject::Primitive(Primitive::WarmSnapshot),
                vec![200.0],
            ),
        ],
        flows: Vec::new(),
    }
}

/// `run --fake <script> <args> --no-baselines`, into a directory of its own.
fn run_fake(name: &str, script: &FakeScript, args: &[&str]) -> (Output, PathBuf) {
    let dir = common::scratch(name);
    let path = dir.join("script.json");
    fs::write(&path, serde_json::to_string(script).unwrap()).unwrap();
    let out = dir.join("report");
    let output = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
        .arg("run")
        .arg("--fake")
        .arg(&path)
        .arg("--out")
        .arg(&out)
        .arg("--no-baselines")
        .args(args)
        .output()
        .unwrap();
    (output, out)
}

#[test]
fn the_ci_plan_takes_10_measured_iterations_after_2_warm_up_unless_the_run_names_its_own() {
    let script = fake_script();
    for (args, iterations, warmup) in [
        (&["--plan", "ci", "--gates", "none"][..], 10, 2),
        (
            &["--plan", "ci", "--gates", "none", "--iterations", "4"][..],
            4,
            2,
        ),
        (
            &["--plan", "ci", "--gates", "none", "--warmup", "0"][..],
            10,
            0,
        ),
        (&["--gates", "none"][..], 50, 3),
        (&["--plan", "full", "--gates", "none"][..], 50, 3),
    ] {
        let (output, out) = run_fake("ci-plan-iterations", &script, args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&output.stderr)
        );
        let (report, _) = written(&out);
        let harness = &report.provenance.harness;
        assert_eq!(
            (harness.iterations, harness.warmup),
            (iterations, warmup),
            "{args:?}"
        );
        for m in &report.measurements {
            assert_eq!((m.iterations, m.warmup), (iterations, warmup), "{args:?}");
            assert_eq!(m.samples.len() as u32, iterations + warmup, "{args:?}");
        }
    }
}

#[test]
fn gates_none_judges_no_gate_where_the_defaults_fail_the_run() {
    // The SwiftUI warm snapshot at 200 ms misses the Simulator's 150 ms gate.
    let script = fake_script();
    let (output, out) = run_fake(
        "ci-plan-gates-defaults",
        &script,
        &["--iterations", "2", "--warmup", "0"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let (report, md) = written(&out);
    assert!(report.gates.is_some_and(|g| !g.passed));
    assert!(md.contains("## Gates"), "{md}");

    let (output, out) = run_fake(
        "ci-plan-gates-none",
        &script,
        &["--iterations", "2", "--warmup", "0", "--gates", "none"],
    );
    let stdout = text(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{stdout}{}",
        text(&output.stderr)
    );
    let (report, md) = written(&out);
    assert_eq!(report.gates, None);
    assert!(!md.contains("## Gates"), "{md}");
    assert!(
        stdout.contains("No gate judged (--gates none)."),
        "{stdout}"
    );
    assert!(!stdout.contains("gate checks"), "{stdout}");
    // The report can still be judged afterwards.
    let judged = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
        .arg("gates")
        .arg(out.join("report.json"))
        .output()
        .unwrap();
    assert_eq!(judged.status.code(), Some(1));
}

#[test]
fn the_ci_plan_picks_its_fixtures_and_refuses_others() {
    let devices = FakeDevices::new("ios:FAKE-IOS-9");
    let bin = empty_bin("ci-plan-fixture");
    let cli = common::mobile_agent();
    let output = bench_run(
        &devices,
        &hermetic_path(&bin),
        &[
            "--plan",
            "ci",
            "--target",
            "FAKE-IOS-9",
            "--fixture",
            "expo",
            "--cli",
            cli.to_str().unwrap(),
        ],
    );
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("--plan ci runs one fixture per platform, swiftui on iOS and compose on Android; --fixture names expo"),
        "{stderr}"
    );
    let output = bench_run(
        &devices,
        &hermetic_path(&bin),
        &["--plan", "fast", "--target", "FAKE-IOS-9"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("invalid value 'fast'"));
    assert_eq!(Plan::from_id("ci"), Some(Plan::Ci));
    assert_eq!(Plan::from_id("full"), Some(Plan::Full));
    assert_eq!(Plan::from_id("fast"), None);
}

// The ratios.

/// A ratio row's target, fixture, flow and baseline, and its ratio.
type RatioKey<'a> = (&'a str, Option<Fixture>, &'a str, &'a str, Option<f64>);

/// A tool that is never found, as a baseline the run skips.
struct Absent;

impl Adapter for Absent {
    fn name(&self) -> &str {
        "webdriveragent"
    }

    fn role(&self) -> Role {
        Role::Baseline
    }

    fn detect(&mut self) -> Detection {
        Detection::Absent {
            note: "WebDriverAgent runs here on a physical iPhone only, and the run has none"
                .to_owned(),
        }
    }

    fn supports(&self, _: &Target, _: &Scenario, _: &Subject) -> bool {
        false
    }
}

/// A fake tool `name` with `role` that runs the flows `ms` gives on both
/// targets, each at a constant duration in ms, `fail` the attempts that fail
/// and `end_state` what its checks read, the golden when `None`.
fn flows_tool(
    name: &str,
    role: Role,
    ms: &[(&str, Fixture, &str, f64)],
    fail: &[u32],
    end_state: Option<(&str, EndState)>,
) -> FakeAdapter {
    let series = ms
        .iter()
        .map(|&(target, fixture, flow, ms)| {
            let mut series = FakeSeries::new(
                target,
                Some(fixture),
                None,
                Subject::Flow(flow.to_owned()),
                vec![ms],
            );
            series.fail = fail.to_vec();
            if let Some((short_flow, state)) = &end_state
                && *short_flow == flow
            {
                series.end_state = Some(state.clone());
            }
            series
        })
        .collect();
    FakeAdapter::new(FakeScript {
        tool: name.into(),
        version: Some("1.0.0".into()),
        role,
        targets: targets(),
        series,
        flows: Vec::new(),
    })
}

fn targets() -> Vec<Target> {
    vec![
        Target::new("SIM-1", TargetClass::Simulator),
        Target::new("emulator-5554", TargetClass::AndroidEmulator),
    ]
}

/// A run of the login and list-scroll flows on a Simulator's SwiftUI fixture
/// and an emulator's Compose fixture: mobile-agent; Maestro, whose second
/// measured iteration of each flow passes only on its retry; agent-device,
/// whose list-scroll ends short of the golden; and WebDriverAgent, skipped.
fn ratio_run() -> Report {
    use Fixture::{Compose, SwiftUi};
    let flows = [
        ("SIM-1", SwiftUi, flow::LOGIN),
        ("SIM-1", SwiftUi, flow::LIST_SCROLL),
        ("emulator-5554", Compose, flow::LOGIN),
        ("emulator-5554", Compose, flow::LIST_SCROLL),
    ];
    let at = |times: [f64; 4]| -> Vec<(&str, Fixture, &str, f64)> {
        flows
            .iter()
            .zip(times)
            .map(|(&(target, fixture, flow), ms)| (target, fixture, flow, ms))
            .collect()
    };
    let mobile_agent = flows_tool(
        "mobile-agent",
        Role::UnderTest,
        &at([4100.0, 16300.0, 3370.0, 15334.0]),
        &[],
        None,
    );
    let series = mobile_agent.series();
    let maestro = flows_tool(
        "maestro",
        Role::Baseline,
        &at([25010.0, 98800.0, 20177.0, 60000.0]),
        &[2],
        None,
    );
    let short = EndState::default().with_text("detail-title", "Detail 97");
    let agent_device = flows_tool(
        "agent-device",
        Role::Baseline,
        &at([8670.0, 30000.0, 5870.0, 30000.0]),
        &[],
        Some((flow::LIST_SCROLL, short)),
    );
    let fixtures = [SwiftUi, Compose];
    Harness::new(
        RunConfig {
            iterations: 4,
            warmup: 1,
            retries: 1,
        },
        targets(),
        series,
    )
    .flows(plan::flows(&fixtures))
    .tool(Box::new(mobile_agent))
    .tool(Box::new(maestro))
    .tool(Box::new(agent_device))
    .tool(Box::new(Absent))
    .probe(Box::new(common::probe()), "macos")
    .run()
    .unwrap()
}

#[test]
fn the_ratios_give_how_many_times_faster_the_tool_under_test_was_than_each_baseline() {
    let report = ratio_run();
    let ratios: Ratios = ratio::of(&report);
    assert_eq!(ratios.tool.as_deref(), Some("mobile-agent"));
    let rows: Vec<RatioKey> = ratios
        .rows
        .iter()
        .map(|r| {
            (
                r.target.as_str(),
                r.fixture,
                r.flow.as_str(),
                r.baseline.as_str(),
                r.ratio,
            )
        })
        .collect();
    use Fixture::{Compose, SwiftUi};
    assert_eq!(
        rows,
        [
            ("SIM-1", Some(SwiftUi), "login", "maestro", Some(6.1)),
            ("SIM-1", Some(SwiftUi), "login", "agent-device", Some(2.11)),
            ("SIM-1", Some(SwiftUi), "list-scroll", "maestro", Some(6.06)),
            ("SIM-1", Some(SwiftUi), "list-scroll", "agent-device", None),
            (
                "emulator-5554",
                Some(Compose),
                "login",
                "maestro",
                Some(5.99)
            ),
            (
                "emulator-5554",
                Some(Compose),
                "login",
                "agent-device",
                Some(1.74)
            ),
            (
                "emulator-5554",
                Some(Compose),
                "list-scroll",
                "maestro",
                Some(3.91)
            ),
            (
                "emulator-5554",
                Some(Compose),
                "list-scroll",
                "agent-device",
                None
            ),
        ]
    );
    let row = &ratios.rows[0];
    assert_eq!(
        (row.tool_p50_us, row.baseline_p50_us, row.target_class),
        (Some(4_100_000), Some(25_010_000), TargetClass::Simulator)
    );
    // Maestro's second measured iteration passed only on its retry: a flake
    // in one of the four.
    assert_eq!(
        (row.baseline_failure_rate_pct, row.baseline_flake_rate_pct),
        (0.0, 25.0)
    );
    // agent-device's list-scroll ended short of the golden, so it was not
    // timed, and the row says why.
    let short = &ratios.rows[3];
    assert_eq!((short.baseline_p50_us, short.ratio), (None, None));
    let note = short.note.as_deref().unwrap();
    assert!(note.starts_with("not timed: "), "{note}");
    assert!(
        note.contains("the end state differs from the golden"),
        "{note}"
    );
    assert_eq!(
        ratios.skipped,
        [ratio::Skipped {
            tool: "webdriveragent".to_owned(),
            note: Some(
                "WebDriverAgent runs here on a physical iPhone only, and the run has none"
                    .to_owned()
            ),
        }]
    );
    // A ratio is rounded half up to a hundredth, under 1 when the baseline
    // was faster.
    assert_eq!(ratio::ratio(25_010_000, 4_100_000), Some(6.1));
    assert_eq!(ratio::ratio(1_005, 1_000), Some(1.01));
    assert_eq!(ratio::ratio(500, 1_000), Some(0.5));
    assert_eq!(ratio::ratio(5, 0), None);
}

#[test]
fn both_reports_list_the_ratios_per_flow_and_platform_with_the_skipped_baselines() {
    let mut report = ratio_run();
    report.ratios = Some(ratio::of(&report));
    let json = report.to_json();
    let md = report.to_markdown();
    let value: Value = serde_json::from_str(&json).unwrap();
    common::assert_every_markdown_number_is_in_the_json(&md, &value);
    assert_eq!(Report::from_json(&json).unwrap(), report);
    assert_eq!(value["ratios"]["rows"][0]["ratio"], 6.1);
    assert_eq!(value["ratios"]["rows"][3]["ratio"], Value::Null);

    let table = common::table(&md, "Ratios");
    assert_eq!(table.rows.len(), 8, "{md}");
    let first = &table.rows[0];
    assert_eq!(table.cell(first, "Target"), "SIM-1");
    assert_eq!(table.cell(first, "Class"), "simulator");
    assert_eq!(table.cell(first, "Fixture"), "swiftui");
    assert_eq!(table.cell(first, "Flow"), "login");
    assert_eq!(table.cell(first, "mobile-agent p50 ms"), "4100.000");
    assert_eq!(table.cell(first, "Baseline"), "maestro");
    assert_eq!(table.cell(first, "Baseline p50 ms"), "25010.000");
    assert_eq!(table.cell(first, "Ratio"), "6.10x");
    let short = &table.rows[3];
    assert_eq!(table.cell(short, "Ratio"), "—");
    assert!(table.cell(short, "Note").starts_with("not timed: "));
    assert!(
        md.contains(
            "Skipped baselines: webdriveragent (WebDriverAgent runs here on a physical iPhone only, and the run has none)."
        ),
        "{md}"
    );
    // A report from before the ratios, which has no such key, still reads.
    let mut old: Value = serde_json::from_str(&json).unwrap();
    old.as_object_mut().unwrap().remove("ratios");
    let read = Report::from_json(&old.to_string()).unwrap();
    assert_eq!(read.ratios, None);
    assert!(!read.to_markdown().contains("## Ratios"));
}

#[test]
fn a_ci_run_on_an_emulator_prints_its_ratios_against_the_installed_baselines() {
    let devices = FakeDevices::new("android:emulator-5590");
    let tools = Tools::new("ci-plan-ratios", Platform::Android, end_screen);
    let out = tools.dir.join("report");
    let cli = common::mobile_agent();
    let output = bench_run(
        &devices,
        &hermetic_path(&tools.bin),
        &[
            "--plan",
            "ci",
            "--gates",
            "none",
            "--target",
            "emulator-5590",
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
    let (report, md) = written(&out);
    let ratios = report.ratios.as_ref().unwrap();
    let rows: Vec<(&str, &str)> = ratios
        .rows
        .iter()
        .map(|r| (r.flow.as_str(), r.baseline.as_str()))
        .collect();
    let mut expected = Vec::new();
    for flow in [flow::LOGIN, flow::LIST_SCROLL, flow::BIG_SCREEN] {
        for baseline in ["maestro", "maestro-runner", "agent-device"] {
            expected.push((flow, baseline));
        }
    }
    assert_eq!(rows, expected);
    for row in &ratios.rows {
        assert_eq!(row.fixture, Some(Fixture::Compose));
        let (tool, baseline) = (row.tool_p50_us.unwrap(), row.baseline_p50_us.unwrap());
        assert_eq!(row.ratio, ratio::ratio(baseline, tool), "{row:?}");
    }
    assert_eq!(common::table(&md, "Ratios").rows.len(), 9);
    // The run prints the ratio table where it would print the gate table.
    let printed = common::table(&format!("# Ratios\n\n{stdout}"), "Ratios");
    assert_eq!(printed.rows.len(), 9, "{stdout}");
    assert!(
        stdout.contains("Skipped baselines: webdriveragent"),
        "{stdout}"
    );
}
