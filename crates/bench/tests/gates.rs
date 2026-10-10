//! Task 9.2: latency gates with the spec's defaults per target class, a table
//! naming each gate, its limit and the measured value, and a non-zero exit on
//! any miss.

mod common;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use bench::adapter::Role;
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::{Fixture, Screen};
use bench::gates::{Gate, GateSet, GateStatus, Statistic};
use bench::harness::{Harness, RunConfig, Subject};
use bench::primitive::Primitive;
use bench::report::Report;
use bench::target::{Target, TargetClass};
use serde_json::Value;

const ITERATIONS: u32 = 20;
const WARMUP: u32 = 3;

/// Three warm-up attempts and 18 measured ones at `rest` ms, then `p95` and
/// `p95 + 5`: of 20 measured samples the 19th smallest, the p95, is `p95`.
fn with_p95(rest: f64, p95: f64) -> Vec<f64> {
    let mut ms = vec![rest; WARMUP as usize + 18];
    ms.push(p95);
    ms.push(p95 + 5.0);
    ms
}

fn series(
    target: &str,
    fixture: Option<Fixture>,
    screen: Option<Screen>,
    primitive: Primitive,
    ms: Vec<f64>,
) -> FakeSeries {
    FakeSeries::new(target, fixture, screen, Subject::Primitive(primitive), ms)
}

/// A Simulator and an emulator measured on every primitive their default gates
/// judge, each within its gate except that the SwiftUI warm snapshot's p95 is
/// `swiftui_warm_p95` ms.
fn script(swiftui_warm_p95: f64) -> FakeScript {
    use Fixture::{Compose, Expo, SwiftUi};
    use Primitive::*;
    use Screen::{Big, List};
    FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![
            Target::new("SIM-1", TargetClass::Simulator),
            Target::new("emulator-5554", TargetClass::AndroidEmulator),
        ],
        series: vec![
            series("SIM-1", None, None, CliStart, with_p95(4.0, 6.0)),
            series("SIM-1", None, None, ColdAttach, with_p95(900.0, 1400.0)),
            series(
                "SIM-1",
                Some(SwiftUi),
                Some(Big),
                WarmSnapshot,
                with_p95(126.0, swiftui_warm_p95),
            ),
            series(
                "SIM-1",
                Some(Expo),
                Some(Big),
                WarmSnapshot,
                with_p95(242.0, 248.0),
            ),
            series(
                "SIM-1",
                Some(SwiftUi),
                Some(List),
                TapDispatch,
                with_p95(0.03, 0.16),
            ),
            series("emulator-5554", None, None, CliStart, with_p95(4.0, 6.0)),
            series(
                "emulator-5554",
                None,
                None,
                ColdAttach,
                with_p95(700.0, 1100.0),
            ),
            series(
                "emulator-5554",
                Some(Compose),
                Some(Big),
                WarmSnapshot,
                with_p95(3.0, 4.0),
            ),
            series(
                "emulator-5554",
                Some(Compose),
                Some(List),
                TapToFrameChange,
                with_p95(30.0, 45.0),
            ),
        ],
        flows: Vec::new(),
    }
}

fn config() -> RunConfig {
    RunConfig {
        iterations: ITERATIONS,
        warmup: WARMUP,
        retries: 0,
    }
}

fn run(script: FakeScript) -> Report {
    let fake = FakeAdapter::new(script);
    Harness::new(config(), fake.targets(), fake.series())
        .tool(Box::new(fake))
        .probe(Box::new(common::probe()), "macos")
        .run()
        .unwrap()
}

/// The gate table in `text`, which holds it without a heading.
fn gate_table(text: &str) -> common::Table {
    common::table(&format!("# Gates\n\n{text}"), "Gates")
}

fn bench(args: &[&str], paths: &[&Path]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"));
    command.args(args);
    command.args(paths);
    command.output().unwrap()
}

fn run_binary(name: &str, script: &FakeScript) -> (Output, std::path::PathBuf) {
    let dir = common::scratch(name);
    let script_path = dir.join("script.json");
    fs::write(&script_path, serde_json::to_string(script).unwrap()).unwrap();
    let out = dir.join("report");
    let output = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
        .arg("run")
        .arg("--fake")
        .arg(&script_path)
        .arg("--out")
        .arg(&out)
        .args(["--iterations", "20", "--warmup", "3", "--no-baselines"])
        .output()
        .unwrap();
    (output, out)
}

#[test]
fn the_defaults_are_the_spec_gates() {
    use Primitive::*;
    use TargetClass::*;
    let gates = GateSet::defaults().gates;
    assert_eq!(
        gates,
        [
            Gate::new(Simulator, CliStart, 20.0),
            Gate::new(Simulator, ColdAttach, 2000.0),
            Gate::new(Simulator, WarmSnapshot, 150.0).on(Some(Fixture::SwiftUi), Screen::Big),
            Gate::new(Simulator, WarmSnapshot, 250.0).on(Some(Fixture::Expo), Screen::Big),
            Gate::new(Simulator, TapDispatch, 20.0),
            Gate::new(AndroidEmulator, CliStart, 20.0),
            Gate::new(AndroidEmulator, ColdAttach, 1500.0),
            Gate::new(AndroidEmulator, WarmSnapshot, 100.0).on(None, Screen::Big),
            Gate::new(AndroidEmulator, TapToFrameChange, 67.0),
            Gate::new(AndroidDevice, CliStart, 20.0),
            Gate::new(AndroidDevice, ColdAttach, 1500.0),
            Gate::new(IosDevice, CliStart, 20.0),
            Gate::new(IosDevice, ColdAttach, 5000.0)
                .noted("the runner already installed on the phone"),
            Gate::new(IosDevice, WarmSnapshot, 1000.0).on(None, Screen::Big),
            Gate::new(IosDevice, TapDispatch, 150.0)
                .noted("through the host's input path, dtuhidd"),
            Gate::reported(IosDevice, TapToFrameChange)
                .noted("not gated while the phone's frames are polled screenshots"),
        ]
    );
    assert!(gates.iter().all(|g| g.statistic == Statistic::P95));
}

/// The spec's scenario "iOS device gates": a run on a physical iPhone whose
/// warm snapshot measures 640 ms at p95, tap dispatch 90 ms and cold attach
/// 3.1 s, and whose tap to frame change, from polled screenshots, measures
/// 439 ms.
fn iphone_script() -> FakeScript {
    use Fixture::SwiftUi;
    use Primitive::*;
    use Screen::{Big, List};
    FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.2.0".into()),
        role: Role::UnderTest,
        targets: vec![Target::new("IPHONE-1", TargetClass::IosDevice)],
        series: vec![
            series(
                "IPHONE-1",
                Some(SwiftUi),
                None,
                ColdAttach,
                with_p95(2400.0, 3100.0),
            ),
            series(
                "IPHONE-1",
                Some(SwiftUi),
                Some(Big),
                WarmSnapshot,
                with_p95(550.0, 640.0),
            ),
            series(
                "IPHONE-1",
                Some(SwiftUi),
                Some(List),
                TapDispatch,
                with_p95(55.0, 90.0),
            ),
            series(
                "IPHONE-1",
                Some(SwiftUi),
                Some(List),
                TapToFrameChange,
                with_p95(240.0, 439.0),
            ),
        ],
        flows: Vec::new(),
    }
}

/// Task 6.2: the three gates pass, the table shows their limits, and tap to
/// frame change is listed with its value and no verdict.
#[test]
fn an_iphone_passes_its_three_gates_and_lists_tap_to_frame_change_with_no_verdict() {
    let verdict = GateSet::defaults().evaluate(&run(iphone_script()));
    assert!(verdict.passed, "{}", verdict.table());
    assert_eq!(verdict.exit_code(), 0);
    let row = |name: &str| {
        verdict
            .rows
            .iter()
            .find(|r| r.gate == name)
            .unwrap_or_else(|| panic!("no row {name} in {}", verdict.table()))
    };
    for (name, limit, measured) in [
        ("ios-device cold-attach", 5_000_000, 3_100_000),
        ("ios-device warm-snapshot big", 1_000_000, 640_000),
        ("ios-device tap-dispatch", 150_000, 90_000),
    ] {
        let gate = row(name);
        assert_eq!(gate.status, GateStatus::Pass, "{name}");
        assert_eq!(
            (gate.limit_us, gate.measured_us),
            (Some(limit), Some(measured)),
            "{name}"
        );
        assert_eq!(gate.target.as_deref(), Some("IPHONE-1"), "{name}");
    }
    let frames = row("ios-device tap-to-frame-change");
    assert_eq!(frames.status, GateStatus::Reported);
    assert_eq!((frames.limit_us, frames.measured_us), (None, Some(439_000)));
    // CLI start is gated on every class; this run did not measure it.
    assert_eq!(row("ios-device cli-start").status, GateStatus::NotMeasured);
    // Only the iPhone's gates apply.
    assert!(
        verdict
            .rows
            .iter()
            .all(|r| r.gate.starts_with("ios-device"))
    );

    let text = verdict.table();
    let table = gate_table(&text);
    let cells = |gate: &str| {
        table
            .rows
            .iter()
            .find(|r| table.cell(r, "Gate") == gate)
            .unwrap_or_else(|| panic!("no row {gate} in {text}"))
    };
    let frames = cells("ios-device tap-to-frame-change");
    assert_eq!(table.cell(frames, "Limit ms"), "—");
    assert_eq!(table.cell(frames, "Measured ms"), "439.000");
    assert_eq!(table.cell(frames, "Result"), "reported");
    assert_eq!(
        table.cell(frames, "Note"),
        "not gated while the phone's frames are polled screenshots"
    );
    let attach = cells("ios-device cold-attach");
    assert_eq!(table.cell(attach, "Limit ms"), "5000.000");
    assert_eq!(table.cell(attach, "Measured ms"), "3100.000");
    assert_eq!(table.cell(attach, "Result"), "pass");
    assert_eq!(
        table.cell(attach, "Note"),
        "the runner already installed on the phone"
    );
    let snapshot = cells("ios-device warm-snapshot big");
    assert_eq!(table.cell(snapshot, "Limit ms"), "1000.000");
    assert_eq!(table.cell(snapshot, "Note"), "—");
    assert_eq!(
        table.cell(cells("ios-device tap-dispatch"), "Limit ms"),
        "150.000"
    );
    assert!(
        text.contains("All 3 gate checks passed. 1 not measured. 1 reported with no verdict."),
        "{text}"
    );
}

/// A reported statistic never fails the run, even when no iteration of its
/// series succeeded.
#[test]
fn a_reported_statistic_never_fails_the_run() {
    let mut script = iphone_script();
    let frames = script
        .series
        .iter_mut()
        .find(|s| s.subject == Subject::Primitive(Primitive::TapToFrameChange))
        .unwrap();
    frames.fail = (0..WARMUP + ITERATIONS).collect();
    let verdict = GateSet::defaults().evaluate(&run(script));
    assert!(verdict.passed, "{}", verdict.table());
    let row = verdict
        .rows
        .iter()
        .find(|r| r.gate == "ios-device tap-to-frame-change")
        .unwrap();
    assert_eq!((row.status, row.measured_us), (GateStatus::Reported, None));
    let text = verdict.table();
    let table = gate_table(&text);
    let row = table
        .rows
        .iter()
        .find(|r| table.cell(r, "Gate") == "ios-device tap-to-frame-change")
        .unwrap();
    assert_eq!(table.cell(row, "Measured ms"), "no samples");
    assert_eq!(table.cell(row, "Result"), "reported");
}

/// The iPhone's run through the binary: it exits zero, and both outputs carry
/// the reported row and agree on every number.
#[test]
fn an_iphone_run_through_the_binary_exits_zero_and_both_outputs_list_the_reported_row() {
    let (output, out) = run_binary("gates-binary-iphone", &iphone_script());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let table = gate_table(&stdout);
    let row = table
        .rows
        .iter()
        .find(|r| table.cell(r, "Gate") == "ios-device tap-to-frame-change")
        .unwrap_or_else(|| panic!("no reported row in {stdout}"));
    assert_eq!(table.cell(row, "Result"), "reported");
    let json: Value =
        serde_json::from_str(&fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    let rows = json["gates"]["rows"].as_array().unwrap();
    let reported = rows
        .iter()
        .find(|r| r["gate"] == "ios-device tap-to-frame-change")
        .unwrap();
    assert_eq!(reported["status"], "reported");
    assert_eq!(reported["limit_us"], Value::Null);
    assert_eq!(reported["measured_us"], 439_000);
    let md = fs::read_to_string(out.join("report.md")).unwrap();
    assert_eq!(common::table(&md, "Gates").rows, table.rows);
    common::assert_every_markdown_number_is_in_the_json(&md, &json);
}

#[test]
fn a_missed_gate_fails_the_run_and_the_table_names_the_gate_limit_and_value() {
    let verdict = GateSet::defaults().evaluate(&run(script(210.0)));
    assert!(!verdict.passed);
    assert_eq!(verdict.exit_code(), 1);
    // Five Simulator gates and four emulator gates; no Android device ran.
    assert_eq!(verdict.rows.len(), 9);
    assert!(
        verdict
            .rows
            .iter()
            .all(|r| !r.gate.starts_with("android-device"))
    );
    let failed: Vec<_> = verdict
        .rows
        .iter()
        .filter(|r| r.status == GateStatus::Fail)
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].gate, "simulator warm-snapshot swiftui big");
    assert_eq!(failed[0].limit_us, Some(150_000));
    assert_eq!(failed[0].measured_us, Some(210_000));
    assert_eq!(failed[0].target.as_deref(), Some("SIM-1"));

    let text = verdict.table();
    let table = gate_table(&text);
    let row = table
        .rows
        .iter()
        .find(|r| table.cell(r, "Result") == "FAIL")
        .unwrap();
    assert_eq!(
        table.cell(row, "Gate"),
        "simulator warm-snapshot swiftui big"
    );
    assert_eq!(table.cell(row, "Statistic"), "p95");
    assert_eq!(table.cell(row, "Limit ms"), "150.000");
    assert_eq!(table.cell(row, "Measured ms"), "210.000");
    assert_eq!(table.cell(row, "Target"), "SIM-1");
    assert!(text.contains("1 of 9 gate checks failed"), "{text}");
}

#[test]
fn when_every_gate_is_met_the_run_exits_zero_and_the_table_shows_each_passed() {
    let verdict = GateSet::defaults().evaluate(&run(script(148.0)));
    assert!(verdict.passed);
    assert_eq!(verdict.exit_code(), 0);
    assert_eq!(verdict.rows.len(), 9);
    assert!(verdict.rows.iter().all(|r| r.status == GateStatus::Pass));
    let text = verdict.table();
    let table = gate_table(&text);
    assert_eq!(table.rows.len(), 9);
    assert!(table.rows.iter().all(|r| table.cell(r, "Result") == "pass"));
    assert!(text.contains("All 9 gate checks passed"), "{text}");
}

#[test]
fn a_run_that_misses_a_gate_exits_non_zero_and_prints_the_table() {
    let (output, out) = run_binary("gates-binary-miss", &script(210.0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let table = gate_table(&stdout);
    let failed: Vec<_> = table
        .rows
        .iter()
        .filter(|r| table.cell(r, "Result") == "FAIL")
        .collect();
    assert_eq!(failed.len(), 1, "{stdout}");
    assert_eq!(
        table.cell(failed[0], "Gate"),
        "simulator warm-snapshot swiftui big"
    );
    assert_eq!(table.cell(failed[0], "Limit ms"), "150.000");
    assert_eq!(table.cell(failed[0], "Measured ms"), "210.000");

    // Both outputs carry the verdict, and agree on it.
    let json: Value =
        serde_json::from_str(&fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(json["gates"]["passed"], false);
    let md = fs::read_to_string(out.join("report.md")).unwrap();
    let in_md = common::table(&md, "Gates");
    assert_eq!(in_md.rows, table.rows);
    common::assert_every_markdown_number_is_in_the_json(&md, &json);
}

#[test]
fn a_run_that_meets_every_gate_exits_zero_through_the_binary() {
    let (output, _) = run_binary("gates-binary-pass", &script(148.0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    let table = gate_table(&String::from_utf8(output.stdout).unwrap());
    assert_eq!(table.rows.len(), 9);
    assert!(table.rows.iter().all(|r| table.cell(r, "Result") == "pass"));
}

/// The emulator's tap-to-frame p95 as S7 measured it under software rendering.
fn software_rendered_emulator() -> FakeScript {
    let mut script = script(148.0);
    let tap = script
        .series
        .iter_mut()
        .find(|s| s.subject == Subject::Primitive(Primitive::TapToFrameChange))
        .unwrap();
    tap.ms = with_p95(107.0, 147.0);
    script
}

/// The script with the emulator's tap-to-frame p95 at `p95` ms.
fn emulator_tap_p95(p95: f64) -> FakeScript {
    let mut script = script(140.0);
    let tap = script
        .series
        .iter_mut()
        .find(|s| s.subject == Subject::Primitive(Primitive::TapToFrameChange))
        .unwrap();
    tap.ms = with_p95(45.0, p95);
    script
}

/// Task 10.13: with the guest rendering on the host GPU, the guest's own
/// pipeline puts tap-to-frame-change's p95 near three frames of the AVD's
/// 60 Hz display (docs/research/emulator-frames.md), so the default gate
/// allows four: 59.1 ms, the Compose fixture's p95 through the daemon,
/// passes, and 67 ms does not.
#[test]
fn the_emulator_tap_to_frame_gate_allows_four_display_frames() {
    let row = |p95| {
        GateSet::defaults()
            .evaluate(&run(emulator_tap_p95(p95)))
            .rows
            .into_iter()
            .find(|r| r.gate == "android-emulator tap-to-frame-change")
            .unwrap()
    };
    let within = row(59.1);
    assert_eq!(within.status, GateStatus::Pass);
    assert_eq!(within.limit_us, Some(67_000));
    assert_eq!(row(67.0).status, GateStatus::Fail, "under is strict");
}

/// The defaults with the emulator's tap-to-frame gate restated at 160 ms, as
/// design.md allows per GPU mode.
fn restated_gates() -> GateSet {
    let mut gates = GateSet::defaults();
    for gate in &mut gates.gates {
        if gate.class == TargetClass::AndroidEmulator
            && gate.primitive == Primitive::TapToFrameChange
        {
            gate.limit_ms = Some(160.0);
        }
    }
    gates
}

#[test]
fn gates_are_configurable_per_target_class() {
    let report = run(software_rendered_emulator());
    let defaults = GateSet::defaults().evaluate(&report);
    assert!(!defaults.passed);
    let restated = GateSet::from_json(&restated_gates().to_json())
        .unwrap()
        .evaluate(&report);
    assert!(restated.passed);
    let row = restated
        .rows
        .iter()
        .find(|r| r.gate == "android-emulator tap-to-frame-change")
        .unwrap();
    assert_eq!(
        (row.limit_us, row.measured_us),
        (Some(160_000), Some(147_000))
    );
}

#[test]
fn the_gates_command_judges_an_existing_report() {
    let dir = common::scratch("gates-command");
    run(software_rendered_emulator()).write(&dir).unwrap();
    let report = dir.join("report.json");
    let gate_file = dir.join("gates.json");
    fs::write(&gate_file, restated_gates().to_json()).unwrap();

    let defaults = bench(&["gates"], &[&report]);
    assert_eq!(defaults.status.code(), Some(1));
    let table = gate_table(&String::from_utf8(defaults.stdout).unwrap());
    let row = table
        .rows
        .iter()
        .find(|r| table.cell(r, "Result") == "FAIL")
        .unwrap();
    assert_eq!(
        table.cell(row, "Gate"),
        "android-emulator tap-to-frame-change"
    );
    assert_eq!(table.cell(row, "Limit ms"), "67.000");
    assert_eq!(table.cell(row, "Measured ms"), "147.000");

    let restated = bench(&["gates", "--gates"], &[&gate_file, &report]);
    assert_eq!(restated.status.code(), Some(0));

    let printed = bench(&["default-gates"], &[]);
    assert_eq!(printed.status.code(), Some(0));
    assert_eq!(
        GateSet::from_json(&String::from_utf8(printed.stdout).unwrap()),
        Ok(GateSet::defaults())
    );
}

#[test]
fn only_the_tool_under_test_is_gated() {
    let mut slow = script(148.0);
    slow.tool = "baseline-tool".into();
    slow.role = Role::Baseline;
    for series in &mut slow.series {
        series.ms = series.ms.iter().map(|ms| ms * 10.0).collect();
    }
    let mine = FakeAdapter::new(script(148.0));
    let report = Harness::new(config(), mine.targets(), mine.series())
        .tool(Box::new(mine))
        .tool(Box::new(FakeAdapter::new(slow)))
        .probe(Box::new(common::probe()), "macos")
        .run()
        .unwrap();
    let verdict = GateSet::defaults().evaluate(&report);
    assert!(verdict.passed);
    assert!(
        verdict
            .rows
            .iter()
            .all(|r| r.tool.as_deref() == Some("mobile-agent"))
    );
}

#[test]
fn a_gate_without_a_measurement_is_reported_but_does_not_fail() {
    let mut script = script(148.0);
    script
        .series
        .retain(|s| s.subject != Subject::Primitive(Primitive::TapDispatch));
    let verdict = GateSet::defaults().evaluate(&run(script));
    assert!(verdict.passed);
    assert_eq!(verdict.exit_code(), 0);
    let row = verdict
        .rows
        .iter()
        .find(|r| r.gate == "simulator tap-dispatch")
        .unwrap();
    assert_eq!(row.status, GateStatus::NotMeasured);
    assert_eq!((row.measured_us, row.target.as_deref()), (None, None));
    let text = verdict.table();
    let table = gate_table(&text);
    let row = table
        .rows
        .iter()
        .find(|r| table.cell(r, "Gate") == "simulator tap-dispatch")
        .unwrap();
    assert_eq!(table.cell(row, "Result"), "not measured");
    assert_eq!(table.cell(row, "Measured ms"), "—");
    assert!(text.contains("1 not measured"), "{text}");
}

#[test]
fn a_primitive_that_never_succeeded_fails_its_gate() {
    let mut script = script(148.0);
    let dispatch = script
        .series
        .iter_mut()
        .find(|s| s.subject == Subject::Primitive(Primitive::TapDispatch))
        .unwrap();
    dispatch.fail = (0..WARMUP + ITERATIONS).collect();
    let verdict = GateSet::defaults().evaluate(&run(script));
    assert!(!verdict.passed);
    let row = verdict
        .rows
        .iter()
        .find(|r| r.gate == "simulator tap-dispatch")
        .unwrap();
    assert_eq!((row.status, row.measured_us), (GateStatus::Fail, None));
    let text = verdict.table();
    let table = gate_table(&text);
    let row = table
        .rows
        .iter()
        .find(|r| table.cell(r, "Gate") == "simulator tap-dispatch")
        .unwrap();
    assert_eq!(table.cell(row, "Measured ms"), "no samples");
    assert_eq!(table.cell(row, "Result"), "FAIL");
}

#[test]
fn a_value_equal_to_the_limit_misses_the_gate() {
    let verdict = GateSet::defaults().evaluate(&run(script(150.0)));
    let row = verdict
        .rows
        .iter()
        .find(|r| r.gate == "simulator warm-snapshot swiftui big")
        .unwrap();
    assert_eq!(row.measured_us, Some(150_000));
    assert_eq!(row.status, GateStatus::Fail);
}
