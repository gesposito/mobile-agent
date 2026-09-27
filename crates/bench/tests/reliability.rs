//! Task 9.5: failure rate and flake rate per primitive over a run's measured
//! iterations. A failed attempt is retried within its iteration; an iteration
//! that then succeeds is a flake, one that never does is a failure.

mod common;

use std::fs;
use std::process::Command;

use bench::adapter::{Adapter, AdapterError, Detection, Role, Sample, Scenario};
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::{Fixture, Screen};
use bench::harness::{Harness, RunConfig, Series, Subject};
use bench::primitive::Primitive;
use bench::report::{Measurement, Outcome, Reliability, Report};
use bench::target::{Target, TargetClass};
use serde_json::Value;

const ITERATIONS: u32 = 50;
const WARMUP: u32 = 3;

/// Tap-to-frame-change on the Simulator's List screen, 36 ms plus 1 ms per
/// attempt, failing at the zero-based attempt numbers in `fail`.
fn taps(fail: Vec<u32>) -> FakeScript {
    let mut series = FakeSeries::new(
        "SIM-1",
        Some(Fixture::SwiftUi),
        Some(Screen::List),
        Subject::Primitive(Primitive::TapToFrameChange),
        (0..60).map(|i| 36.0 + f64::from(i)).collect(),
    );
    series.fail = fail;
    FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![Target::new("SIM-1", TargetClass::Simulator)],
        series: vec![series],
        flows: Vec::new(),
    }
}

fn run(script: FakeScript, retries: u32) -> Report {
    let fake = FakeAdapter::new(script);
    Harness::new(
        RunConfig {
            iterations: ITERATIONS,
            warmup: WARMUP,
            retries,
        },
        fake.targets(),
        fake.series(),
    )
    .tool(Box::new(fake))
    .probe(Box::new(common::probe()), "macos")
    .run()
    .unwrap()
}

/// The markdown row of the only measurement, as (column, cell) lookups.
fn markdown_cell(report: &Report, column: &str) -> String {
    let md = report.to_markdown();
    let table = common::table(&md, "Measurements");
    table.cell(&table.rows[0], column).to_owned()
}

fn only(report: &Report) -> &Measurement {
    assert_eq!(report.measurements.len(), 1);
    &report.measurements[0]
}

#[test]
fn two_failed_iterations_of_fifty_taps_are_a_four_percent_failure_rate() {
    // Attempts 0 to 2 are the warm-up; 10 and 30 are measured iterations 7 and 27.
    let report = run(taps(vec![10, 30]), 0);
    let m = only(&report);
    assert_eq!(
        m.reliability,
        Reliability {
            iterations: 50,
            failures: 2,
            flakes: 0,
            failure_rate_pct: 4.0,
            flake_rate_pct: 0.0,
        }
    );
    // The latency percentiles sit alongside, over the 48 taps that succeeded.
    assert_eq!(m.stats.map(|s| s.n), Some(48));
    assert_eq!(markdown_cell(&report, "Fail %"), "4.0");
    assert_eq!(markdown_cell(&report, "Flake %"), "0.0");
    assert_eq!(markdown_cell(&report, "n"), "48");
    let json: Value = serde_json::from_str(&report.to_json()).unwrap();
    let reliability = &json["measurements"][0]["reliability"];
    assert_eq!(reliability["failure_rate_pct"], 4.0);
    assert_eq!(reliability["failures"], 2);
    common::assert_every_markdown_number_is_in_the_json(&report.to_markdown(), &json);
}

#[test]
fn a_failure_that_passes_on_retry_is_a_flake_not_a_failure() {
    // Measured iteration 10 starts at attempt 13 and fails once; its retry,
    // 14, passes. Iteration 30 then starts at attempt 34 and does the same.
    let report = run(taps(vec![13, 34]), 1);
    let m = only(&report);
    assert_eq!(
        m.reliability,
        Reliability {
            iterations: 50,
            failures: 0,
            flakes: 2,
            failure_rate_pct: 0.0,
            flake_rate_pct: 4.0,
        }
    );
    assert_eq!(m.stats.map(|s| s.n), Some(50));
    let flaked: Vec<_> = m
        .samples
        .iter()
        .filter(|s| s.outcome == Outcome::Flake)
        .collect();
    assert_eq!(flaked.len(), 2);
    assert_eq!((flaked[0].iteration, flaked[0].attempts), (WARMUP + 10, 2));
    assert_eq!(flaked[0].errors, ["injected failure at attempt 13"]);
    // The flaked iteration's latency is its successful retry's.
    assert_eq!(flaked[0].us, Some(50_000));
    assert_eq!(markdown_cell(&report, "Fail %"), "0.0");
    assert_eq!(markdown_cell(&report, "Flake %"), "4.0");
}

#[test]
fn an_iteration_that_fails_every_retry_is_a_failure() {
    let report = run(taps(vec![13, 14, 34, 35]), 1);
    let m = only(&report);
    assert_eq!((m.reliability.failures, m.reliability.flakes), (2, 0));
    assert_eq!(m.reliability.failure_rate_pct, 4.0);
    let failed: Vec<_> = m
        .samples
        .iter()
        .filter(|s| s.outcome == Outcome::Fail)
        .collect();
    assert_eq!(failed.len(), 2);
    assert_eq!(failed[0].attempts, 2);
    assert_eq!(
        failed[0].errors,
        [
            "injected failure at attempt 13",
            "injected failure at attempt 14"
        ]
    );
    assert_eq!(report.provenance.harness.retries, 1);
}

#[test]
fn warm_up_failures_are_not_counted() {
    let report = run(taps(vec![0, 1]), 0);
    let m = only(&report);
    assert_eq!(m.reliability.failures, 0);
    assert_eq!(m.reliability.failure_rate_pct, 0.0);
    assert_eq!(m.samples[0].outcome, Outcome::Fail);
    assert!(m.samples[0].warmup);
}

/// A tool whose preparation always fails.
struct Unpreparable;

impl Adapter for Unpreparable {
    fn name(&self) -> &str {
        "unpreparable"
    }

    fn role(&self) -> Role {
        Role::UnderTest
    }

    fn detect(&mut self) -> Detection {
        Detection::Available { version: None }
    }

    fn supports(&self, _: &Target, _: &Scenario, _: &Subject) -> bool {
        true
    }

    fn prepare(&mut self, _: &Target, _: &Scenario, _: &Subject) -> Result<(), AdapterError> {
        Err(AdapterError::new("the fixture is not installed"))
    }

    fn measure(&mut self, _: &Target, _: &Scenario, _: Primitive) -> Result<Sample, AdapterError> {
        unreachable!("a series that could not be prepared is not measured")
    }
}

#[test]
fn a_series_that_could_not_be_prepared_failed_every_iteration() {
    let series = vec![Series {
        target: "SIM-1".into(),
        fixture: Some(Fixture::SwiftUi),
        screen: Some(Screen::Big),
        subject: Subject::Primitive(Primitive::WarmSnapshot),
    }];
    let report = Harness::new(
        RunConfig {
            iterations: 20,
            warmup: 3,
            retries: 1,
        },
        vec![Target::new("SIM-1", TargetClass::Simulator)],
        series,
    )
    .tool(Box::new(Unpreparable))
    .probe(Box::new(common::probe()), "macos")
    .run()
    .unwrap();
    let m = only(&report);
    assert_eq!(
        m.error.as_deref(),
        Some("preparation failed: the fixture is not installed")
    );
    assert_eq!(
        m.reliability,
        Reliability {
            iterations: 20,
            failures: 20,
            flakes: 0,
            failure_rate_pct: 100.0,
            flake_rate_pct: 0.0,
        }
    );
    assert_eq!(markdown_cell(&report, "Fail %"), "100.0");
}

#[test]
fn the_rates_reach_both_outputs_through_the_binary() {
    let dir = common::scratch("reliability-binary");
    let script = dir.join("script.json");
    fs::write(&script, serde_json::to_string(&taps(vec![10, 30])).unwrap()).unwrap();
    let out = dir.join("report");
    let output = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
        .arg("run")
        .arg("--fake")
        .arg(&script)
        .arg("--out")
        .arg(&out)
        .args(["--iterations", "50", "--warmup", "3", "--retries", "0"])
        .arg("--no-baselines")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    let report = Report::from_json(&fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report.provenance.harness.retries, 0);
    assert_eq!(only(&report).reliability.failure_rate_pct, 4.0);
    let md = fs::read_to_string(out.join("report.md")).unwrap();
    let table = common::table(&md, "Measurements");
    assert_eq!(table.cell(&table.rows[0], "Fail %"), "4.0");
    let provenance = common::table(&md, "Provenance");
    let harness = provenance.rows.iter().find(|r| r[0] == "Harness").unwrap();
    assert_eq!(
        harness[1],
        "bench 0.1.0, 50 iterations after 3 warm-up, 0 retries"
    );
}
