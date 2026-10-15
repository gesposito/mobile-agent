//! Task 3.2 of `ci-benchmarks`: `mobile-agent-bench trend`, a scheduled
//! run's report beside the previous scheduled run's (design D5): for two
//! microbenchmark A/B reports, each benchmark's head median in both with the
//! change; for two `run` reports, each flow's ratio against each baseline in
//! both, matched by target class, fixture, flow and baseline. Shown, never
//! judged.

mod common;

use std::fs;
use std::process::Command;

use bench::adapter::Role;
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::Fixture;
use bench::flow;
use bench::harness::{Harness, RunConfig, Subject};
use bench::micro::ab;
use bench::micro::criterion::Measured;
use bench::plan;
use bench::report::Report;
use bench::target::{Target, TargetClass};
use bench::trend;
use common::micro::{BASE_REF, BASE_SHA, Recorded, provenance};

/// An A/B report of one round per side, `head` against `base`, each a list of
/// benchmark ids and medians.
fn ab_report(base: &[(&str, u64)], head: &[(&str, u64)], sha: &str) -> ab::Report {
    let measured = |list: &[(&str, u64)]| -> Vec<Measured> {
        list.iter()
            .map(|&(id, ns)| Recorded::new(id, ns).measured())
            .collect()
    };
    let mut provenance = provenance();
    provenance.commit.as_mut().unwrap().sha = sha.to_owned();
    ab::Report::new(
        provenance,
        ab::Base {
            name: BASE_REF.to_owned(),
            sha: BASE_SHA.to_owned(),
        },
        1,
        None,
        10.0,
        &[measured(base)],
        &[measured(head)],
    )
}

#[test]
fn two_ab_reports_give_each_benchmark_s_head_median_in_both_and_the_change() {
    let previous = ab_report(
        &[
            ("protocol/decode/snapshot-500", 100_000),
            ("mirror/settle/no-frames-grid", 5_000),
        ],
        &[
            ("protocol/decode/snapshot-500", 101_000),
            ("mirror/settle/no-frames-grid", 5_000),
        ],
        "1111111111111111111111111111111111111111",
    );
    let current = ab_report(
        &[
            ("protocol/decode/snapshot-500", 101_000),
            ("mirror/pump/first-read-grid", 97_000),
        ],
        &[
            ("protocol/decode/snapshot-500", 111_100),
            ("mirror/pump/first-read-grid", 98_000),
        ],
        "2222222222222222222222222222222222222222",
    );
    let md = trend::micro(&previous, &current);
    assert!(
        md.starts_with("## Against the previous scheduled run\n\n"),
        "{md}"
    );
    assert!(
        md.contains("1111111111111111111111111111111111111111"),
        "{md}"
    );
    assert!(
        md.contains("2222222222222222222222222222222222222222"),
        "{md}"
    );
    let table = common::table(&md, "Against the previous scheduled run");
    let rows: Vec<Vec<&str>> = table
        .rows
        .iter()
        .map(|r| {
            [
                "Benchmark",
                "Previous head µs",
                "Head µs",
                "Change",
                "Previous difference",
                "Difference",
            ]
            .iter()
            .map(|c| table.cell(r, c))
            .collect()
        })
        .collect();
    assert_eq!(
        rows,
        [
            vec![
                "mirror/pump/first-read-grid",
                "—",
                "98.000",
                "new",
                "—",
                "+1.0%"
            ],
            vec![
                "protocol/decode/snapshot-500",
                "101.000",
                "111.100",
                "+10.0%",
                "+1.0%",
                "+10.0%"
            ],
            vec![
                "mirror/settle/no-frames-grid",
                "5.000",
                "—",
                "gone",
                "+0.0%",
                "—"
            ],
        ]
    );
}

/// A run of the login flow on a Simulator `udid`'s SwiftUI fixture, through
/// mobile-agent in `tool_ms` and Maestro in `maestro_ms`.
fn run_report(udid: &str, tool_ms: f64, maestro_ms: f64) -> Report {
    let tool = |name: &str, role, ms| {
        FakeAdapter::new(FakeScript {
            tool: name.into(),
            version: Some("1.0.0".into()),
            role,
            targets: vec![Target::new(udid, TargetClass::Simulator)],
            series: vec![FakeSeries::new(
                udid,
                Some(Fixture::SwiftUi),
                None,
                Subject::Flow(flow::LOGIN.to_owned()),
                vec![ms],
            )],
            flows: Vec::new(),
        })
    };
    let under_test = tool("mobile-agent", Role::UnderTest, tool_ms);
    let series = under_test.series();
    Harness::new(
        RunConfig {
            iterations: 2,
            warmup: 0,
            retries: 0,
        },
        under_test.targets(),
        series,
    )
    .flows(plan::flows(&[Fixture::SwiftUi]))
    .tool(Box::new(under_test))
    .tool(Box::new(tool("maestro", Role::Baseline, maestro_ms)))
    .probe(Box::new(common::probe()), "macos")
    .run()
    .unwrap()
}

#[test]
fn two_run_reports_give_each_flow_s_ratios_in_both_matched_by_class_not_by_target() {
    // A report from before the ratios, without them: they are recomputed.
    let previous = run_report("SIM-OLD", 4_000.0, 24_000.0);
    assert_eq!(previous.ratios, None);
    let mut current = run_report("SIM-NEW", 4_100.0, 25_010.0);
    current.ratios = Some(bench::ratio::of(&current));
    let md = trend::harness(&previous, &current);
    let table = common::table(&md, "Against the previous scheduled run");
    assert_eq!(table.rows.len(), 1, "{md}");
    let row = &table.rows[0];
    assert_eq!(table.cell(row, "Class"), "simulator");
    assert_eq!(table.cell(row, "Fixture"), "swiftui");
    assert_eq!(table.cell(row, "Flow"), "login");
    assert_eq!(table.cell(row, "Baseline"), "maestro");
    assert_eq!(table.cell(row, "Previous ratio"), "6.00x");
    assert_eq!(table.cell(row, "Ratio"), "6.10x");
    assert_eq!(table.cell(row, "Change"), "+0.10");
    assert_eq!(table.cell(row, "Previous mobile-agent p50 ms"), "4000.000");
    assert_eq!(table.cell(row, "mobile-agent p50 ms"), "4100.000");
}

#[test]
fn trend_prints_the_comparison_and_exits_0_or_2_when_the_reports_do_not_compare() {
    let dir = common::scratch("trend-binary");
    let write = |name: &str, text: String| {
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        path
    };
    let previous = write(
        "previous.json",
        ab_report(
            &[("protocol/decode/snapshot-500", 100_000)],
            &[("protocol/decode/snapshot-500", 100_000)],
            "1111111111111111111111111111111111111111",
        )
        .to_json(),
    );
    let current = write(
        "current.json",
        ab_report(
            &[("protocol/decode/snapshot-500", 100_000)],
            &[("protocol/decode/snapshot-500", 150_000)],
            "2222222222222222222222222222222222222222",
        )
        .to_json(),
    );
    let run = write("run.json", run_report("SIM-1", 4_000.0, 24_000.0).to_json());
    let trend = |a: &std::path::Path, b: &std::path::Path| {
        let output = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
            .arg("trend")
            .arg(a)
            .arg(b)
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    // A head 50% slower than the previous run's is shown, not judged.
    let (status, stdout, stderr) = trend(&previous, &current);
    assert_eq!(status, Some(0), "{stderr}");
    assert!(
        stdout.contains("| protocol/decode/snapshot-500 | 100.000 | 150.000 | +50.0% |"),
        "{stdout}"
    );
    let (status, stdout, _) = trend(&run, &run);
    assert_eq!(status, Some(0));
    assert!(
        stdout.contains("| simulator | swiftui | login | maestro | 6.00x | 6.00x | +0.00 |"),
        "{stdout}"
    );
    let (status, _, stderr) = trend(&previous, &run);
    assert_eq!(status, Some(2));
    assert!(stderr.contains("different kinds"), "{stderr}");
    let garbage = write("garbage.json", "{\"schema_version\": 1}".to_owned());
    let (status, _, stderr) = trend(&garbage, &garbage);
    assert_eq!(status, Some(2));
    assert!(stderr.contains("neither"), "{stderr}");
    let (status, _, stderr) = trend(&dir.join("absent.json"), &current);
    assert_eq!(status, Some(2));
    assert!(stderr.contains("cannot read"), "{stderr}");
}
