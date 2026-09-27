//! Task 9.1: primitive measurement with warm-up exclusion, provenance, and the
//! JSON report and markdown summary agreeing on every number.

mod common;

use std::fs;

use bench::adapter::Role;
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::{Fixture, Screen};
use bench::harness::{Harness, RunConfig, Series, Subject};
use bench::primitive::Primitive;
use bench::provenance::ScriptedProbe;
use bench::report::{Measurement, NodeRange, Report};
use bench::stats::Summary;
use bench::target::{Target, TargetClass};
use serde_json::Value;

const ITERATIONS: u32 = 20;
const WARMUP: u32 = 3;

fn config() -> RunConfig {
    RunConfig {
        iterations: ITERATIONS,
        warmup: WARMUP,
    }
}

/// Three 900 ms warm-up attempts, then `base`, `base + step`, and so on.
fn ramp(base: f64, step: f64) -> Vec<f64> {
    let mut ms = vec![900.0; WARMUP as usize];
    ms.extend((0..ITERATIONS).map(|i| base + step * f64::from(i)));
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

/// Every primitive on the Simulator, two on the emulator.
fn script() -> FakeScript {
    use Fixture::{Compose, SwiftUi};
    use Primitive::*;
    use Screen::{Big, List, Login};
    let sim = |fixture, screen, primitive, ms| series("SIM-1", fixture, screen, primitive, ms);
    let mut sim_warm = sim(Some(SwiftUi), Some(Big), WarmSnapshot, ramp(120.0, 0.5));
    sim_warm.nodes = Some(546);
    let mut emulator_warm = series(
        "emulator-5554",
        Some(Compose),
        Some(Big),
        WarmSnapshot,
        ramp(3.0, 0.1),
    );
    emulator_warm.nodes = Some(573);
    FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![
            Target::new("SIM-1", TargetClass::Simulator),
            Target::new("emulator-5554", TargetClass::AndroidEmulator),
        ],
        series: vec![
            sim(None, None, CliStart, ramp(4.0, 0.1)),
            sim(None, None, ColdAttach, ramp(700.0, 10.0)),
            sim(Some(SwiftUi), Some(Big), FirstSnapshot, ramp(400.0, 2.0)),
            sim_warm,
            sim(Some(SwiftUi), Some(List), TapDispatch, ramp(0.03, 0.001)),
            sim(Some(SwiftUi), Some(List), TapToFrameChange, ramp(36.0, 1.0)),
            sim(Some(SwiftUi), Some(List), TapToTreeDelta, ramp(40.0, 1.0)),
            sim(Some(SwiftUi), Some(Login), TextEntry, ramp(80.0, 1.5)),
            sim(Some(SwiftUi), Some(Big), Screenshot, ramp(25.0, 0.25)),
            sim(Some(SwiftUi), Some(List), Settle, ramp(250.0, 3.0)),
            emulator_warm,
            series("emulator-5554", None, None, CliStart, ramp(4.2, 0.1)),
        ],
        flows: Vec::new(),
    }
}

fn run_with(script: FakeScript, probe: ScriptedProbe) -> Report {
    let fake = FakeAdapter::new(script);
    Harness::new(config(), fake.targets(), fake.series())
        .tool(Box::new(fake))
        .probe(Box::new(probe), "macos")
        .run()
        .unwrap()
}

fn run(script: FakeScript) -> Report {
    run_with(script, common::probe())
}

fn find<'a>(report: &'a Report, target: &str, primitive: Primitive) -> &'a Measurement {
    report
        .measurements
        .iter()
        .find(|m| m.target == target && m.subject == Subject::Primitive(primitive))
        .unwrap_or_else(|| panic!("no {primitive} measurement on {target}"))
}

fn row<'a>(table: &'a common::Table, target: &str, subject: &str) -> &'a [String] {
    table
        .rows
        .iter()
        .find(|r| table.cell(r, "Target") == target && table.cell(r, "Subject") == subject)
        .unwrap_or_else(|| panic!("no row for {subject} on {target} in {table:?}"))
}

#[test]
fn every_listed_primitive_appears_with_its_percentiles_and_sample_count() {
    let report = run(script());
    let md = report.to_markdown();
    let table = common::table(&md, "Measurements");
    for primitive in Primitive::LISTED {
        let stats = find(&report, "SIM-1", primitive)
            .stats
            .unwrap_or_else(|| panic!("no statistics for {primitive}"));
        assert_eq!(stats.n, ITERATIONS, "{primitive}");
        assert!(stats.p50_us <= stats.p95_us && stats.p95_us <= stats.max_us);
        let row = row(&table, "SIM-1", primitive.id());
        assert_eq!(table.cell(row, "n"), ITERATIONS.to_string());
        assert_eq!(table.cell(row, "p50 ms"), common::ms(stats.p50_us));
        assert_eq!(table.cell(row, "p95 ms"), common::ms(stats.p95_us));
        assert_eq!(table.cell(row, "max ms"), common::ms(stats.max_us));
    }
    // Tap dispatch is measured too: the Simulator gate needs it.
    let dispatch = find(&report, "SIM-1", Primitive::TapDispatch);
    assert_eq!(
        dispatch.stats,
        Some(Summary {
            n: ITERATIONS,
            p50_us: 39,
            p95_us: 48,
            max_us: 49,
        })
    );
}

#[test]
fn warm_up_iterations_are_excluded_from_statistics() {
    let mut script = script();
    let mut ms = vec![900.0; WARMUP as usize];
    ms.extend((1..=ITERATIONS).map(f64::from));
    script.series = vec![series("SIM-1", None, None, Primitive::CliStart, ms)];
    let report = run(script);
    let m = &report.measurements[0];
    assert_eq!(
        m.stats,
        Some(Summary {
            n: 20,
            p50_us: 10_000,
            p95_us: 19_000,
            max_us: 20_000,
        })
    );
    assert_eq!((m.iterations, m.warmup), (ITERATIONS, WARMUP));
    assert_eq!(m.samples.len(), (WARMUP + ITERATIONS) as usize);
    assert!(
        m.samples[..3]
            .iter()
            .all(|s| s.warmup && s.us == Some(900_000))
    );
    assert!(m.samples[3..].iter().all(|s| !s.warmup));
    let json: Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(json["measurements"][0]["samples"][0]["warmup"], true);
    assert_eq!(json["provenance"]["harness"]["warmup"], WARMUP);
    assert_eq!(json["provenance"]["harness"]["iterations"], ITERATIONS);
}

#[test]
fn the_json_report_and_the_markdown_summary_agree_on_every_number() {
    let dir = common::scratch("measurement-two-outputs");
    let files = run(script()).write(&dir).unwrap();
    assert_eq!(files.json, dir.join("report.json"));
    assert_eq!(files.markdown, dir.join("report.md"));
    let json: Value = serde_json::from_str(&fs::read_to_string(&files.json).unwrap()).unwrap();
    let md = fs::read_to_string(&files.markdown).unwrap();

    let table = common::table(&md, "Measurements");
    let measurements = json["measurements"].as_array().unwrap();
    assert_eq!(table.rows.len(), measurements.len());
    for m in measurements {
        let row = table
            .rows
            .iter()
            .find(|r| {
                table.cell(r, "Tool") == m["tool"]
                    && table.cell(r, "Target") == m["target"]
                    && table.cell(r, "Fixture") == common::or_dash(&m["fixture"])
                    && table.cell(r, "Screen") == common::or_dash(&m["screen"])
                    && table.cell(r, "Subject") == common::subject_cell(m)
            })
            .unwrap_or_else(|| panic!("no markdown row for {m}"));
        let stats = &m["stats"];
        assert_eq!(table.cell(row, "n"), stats["n"].to_string());
        for (column, field) in [
            ("p50 ms", "p50_us"),
            ("p95 ms", "p95_us"),
            ("max ms", "max_us"),
        ] {
            assert_eq!(
                table.cell(row, column),
                common::ms(stats[field].as_u64().unwrap())
            );
        }
        assert_eq!(table.cell(row, "Clock"), m["clock"]);
        let nodes = match &m["nodes"] {
            Value::Null => "—".to_owned(),
            n => n["max"].to_string(),
        };
        assert_eq!(table.cell(row, "Nodes"), nodes);
    }

    let comparison = common::table(&md, "Comparison");
    for row in &comparison.rows {
        for (column, tool) in comparison.header.iter().enumerate().skip(4) {
            let m = measurements
                .iter()
                .find(|m| {
                    m["tool"] == tool.as_str()
                        && m["target"] == row[0].as_str()
                        && common::or_dash(&m["fixture"]) == row[1]
                        && common::or_dash(&m["screen"]) == row[2]
                        && common::subject_cell(m) == row[3]
                })
                .unwrap();
            let stats = &m["stats"];
            let expected = format!(
                "{} / {}",
                common::ms(stats["p50_us"].as_u64().unwrap()),
                common::ms(stats["p95_us"].as_u64().unwrap())
            );
            assert_eq!(row[column], expected);
        }
    }

    common::assert_every_markdown_number_is_in_the_json(&md, &json);
}

#[test]
fn provenance_is_recorded_in_both_outputs() {
    let report = run(script());
    let json: Value = serde_json::from_str(&report.to_json()).unwrap();
    let p = &json["provenance"];
    assert_eq!(p["host"]["hardware"], "Apple M1 Max");
    assert_eq!(p["host"]["cpu_cores"], 10);
    assert_eq!(p["host"]["memory_bytes"], 68_719_476_736_u64);
    assert_eq!(p["host"]["os"], "macOS 26.6.2 (25G83)");
    assert_eq!(p["xcode"], "Xcode 27.0 (27A266a)");
    assert_eq!(
        p["commit"]["sha"],
        "d7d0f41c0ffee0000000000000000000000000ab"
    );
    assert_eq!(p["commit"]["dirty"], false);
    assert!(p["started_at"].as_str().unwrap().ends_with('Z'));
    let targets = p["targets"].as_array().unwrap();
    let emulator = targets.iter().find(|t| t["id"] == "emulator-5554").unwrap();
    assert_eq!(emulator["api_level"], 36);
    assert_eq!(emulator["model"], "sdk_gphone64_arm64 (AVD pixel_api36)");
    assert_eq!(emulator["os"], "Android 16");
    let sim = targets.iter().find(|t| t["id"] == "SIM-1").unwrap();
    assert_eq!(sim["model"], "iPhone 17");
    assert_eq!(sim["os"], "iOS 26.5");
    assert_eq!(p["tools"][0]["name"], "mobile-agent");
    assert_eq!(p["tools"][0]["version"], "0.1.0");
    assert_eq!(p["tools"][0]["status"], "available");

    let md = report.to_markdown();
    let provenance = common::table(&md, "Provenance");
    let value = |field: &str| {
        provenance
            .rows
            .iter()
            .find(|r| r[0] == field)
            .map(|r| r[1].clone())
            .unwrap_or_else(|| panic!("no provenance row {field:?}"))
    };
    assert_eq!(value("Host hardware"), "Apple M1 Max");
    assert_eq!(value("Host OS"), "macOS 26.6.2 (25G83)");
    assert_eq!(value("Xcode"), "Xcode 27.0 (27A266a)");
    assert_eq!(
        value("Commit under test"),
        "d7d0f41c0ffee0000000000000000000000000ab (clean)"
    );
    let target_table = common::table(&md, "Targets");
    let emulator_row = target_table
        .rows
        .iter()
        .find(|r| r[0] == "emulator-5554")
        .unwrap();
    assert_eq!(target_table.cell(emulator_row, "Android API level"), "36");
    assert_eq!(
        target_table.cell(emulator_row, "Model"),
        "sdk_gphone64_arm64 (AVD pixel_api36)"
    );
    let tools = common::table(&md, "Tools");
    let tool_row = tools.rows.iter().find(|r| r[0] == "mobile-agent").unwrap();
    assert_eq!(tools.cell(tool_row, "Version"), "0.1.0");
    assert_eq!(tools.cell(tool_row, "Status"), "available");
}

#[test]
fn provenance_tells_whether_two_environments_matched() {
    let first = run(script());
    // Another run on the same host, at a different time and load.
    let again = run_with(
        script(),
        common::probe().with("sysctl -n vm.loadavg", "{ 9.00 9.00 9.00 }"),
    );
    assert_eq!(
        first.provenance.environment_differences(&again.provenance),
        Vec::<String>::new()
    );
    // Another Xcode and an older emulator image.
    let elsewhere = run_with(
        script(),
        common::probe()
            .with("xcodebuild -version", "Xcode 26.4\nBuild version 17E192")
            .with(
                "adb -s emulator-5554 shell getprop",
                "[ro.boot.qemu.avd_name]: [pixel_api36]\n\
                 [ro.build.version.release]: [15]\n\
                 [ro.build.version.sdk]: [35]\n\
                 [ro.product.model]: [sdk_gphone64_arm64]",
            ),
    );
    assert_eq!(
        first
            .provenance
            .environment_differences(&elsewhere.provenance),
        [
            "xcode",
            "targets[emulator-5554].os",
            "targets[emulator-5554].api_level",
        ]
    );
}

#[test]
fn the_large_screen_records_the_node_count_the_tool_returned() {
    let report = run(script());
    let sim = find(&report, "SIM-1", Primitive::WarmSnapshot);
    assert_eq!(sim.screen, Some(Screen::Big));
    assert_eq!(sim.nodes, Some(NodeRange { min: 546, max: 546 }));
    assert_eq!(
        find(&report, "emulator-5554", Primitive::WarmSnapshot).nodes,
        Some(NodeRange { min: 573, max: 573 })
    );
    let md = report.to_markdown();
    let table = common::table(&md, "Measurements");
    assert_eq!(
        table.cell(row(&table, "SIM-1", "warm-snapshot"), "Nodes"),
        "546"
    );
    assert_eq!(
        table.cell(row(&table, "emulator-5554", "warm-snapshot"), "Nodes"),
        "573"
    );
    assert_eq!(table.cell(row(&table, "SIM-1", "cli-start"), "Nodes"), "—");
}

#[test]
fn the_comparison_table_spans_tools_and_targets() {
    let mut other = script();
    other.tool = "other-tool".into();
    other.version = Some("2.0.0".into());
    other.role = Role::Baseline;
    other
        .series
        .retain(|s| s.subject == Subject::Primitive(Primitive::WarmSnapshot));
    for series in &mut other.series {
        series.ms = ramp(600.0, 5.0);
    }
    let mine = FakeAdapter::new(script());
    let theirs = FakeAdapter::new(other);
    let report = Harness::new(config(), mine.targets(), mine.series())
        .tool(Box::new(mine))
        .tool(Box::new(theirs))
        .probe(Box::new(common::probe()), "macos")
        .run()
        .unwrap();
    let md = report.to_markdown();
    let comparison = common::table(&md, "Comparison");
    assert_eq!(
        comparison.header,
        [
            "Target",
            "Fixture",
            "Screen",
            "Subject",
            "mobile-agent",
            "other-tool"
        ]
    );
    let cells = |target: &str, subject: &str| {
        comparison
            .rows
            .iter()
            .find(|r| r[0] == target && r[3] == subject)
            .map(|r| (r[4].clone(), r[5].clone()))
            .unwrap()
    };
    let (mine_sim, theirs_sim) = cells("SIM-1", "warm-snapshot");
    assert_eq!(mine_sim, "124.500 / 129.000");
    assert_eq!(theirs_sim, "645.000 / 690.000");
    let (_, theirs_emulator) = cells("emulator-5554", "warm-snapshot");
    assert_eq!(theirs_emulator, "645.000 / 690.000");
    let (mine_cli, theirs_cli) = cells("SIM-1", "cli-start");
    assert_eq!(mine_cli, "4.900 / 5.800");
    assert_eq!(theirs_cli, "—");
}

#[test]
fn a_series_on_an_unknown_target_is_an_error() {
    let fake = FakeAdapter::new(script());
    let series = vec![Series {
        target: "nowhere".into(),
        fixture: None,
        screen: None,
        subject: Subject::Primitive(Primitive::CliStart),
    }];
    let error = Harness::new(config(), fake.targets(), series)
        .tool(Box::new(fake))
        .probe(Box::new(common::probe()), "macos")
        .run()
        .unwrap_err();
    assert!(error.contains("nowhere"), "{error}");
}

#[test]
fn the_report_round_trips_through_json() {
    let report = run(script());
    assert_eq!(Report::from_json(&report.to_json()), Ok(report));
}
