//! `mobile-agent-bench trend <previous> <report>`, the comparison of design
//! D5 of `ci-benchmarks`: a scheduled run's report beside the previous
//! scheduled run's, both of the same job. For two microbenchmark A/B reports
//! it lists each benchmark's head median in both and the change between them,
//! with each run's difference against its base; for two harness reports, each
//! flow's ratios against each baseline in both, recomputed from the
//! measurements so that a report from before the ratios compares too, matched
//! by target class, fixture, flow and baseline, since the targets' ids differ
//! from run to run. The runs had different machines, so the changes are shown
//! and never judged.

use std::collections::BTreeMap;

use crate::markdown::{Table, or_dash};
use crate::micro::ab;
use crate::ratio::{self, Row};
use crate::report::Report;
use crate::stats::ms;

/// The comparison of two reports' texts, `previous` and `current`, as
/// markdown; an error when either is neither kind of report or the two are
/// of different kinds.
pub fn compare(previous: &str, current: &str) -> Result<String, String> {
    match (
        ab::Report::from_json(previous),
        ab::Report::from_json(current),
    ) {
        (Ok(previous), Ok(current)) => return Ok(micro(&previous, &current)),
        (Ok(_), Err(_)) | (Err(_), Ok(_)) => {
            return Err(
                "the two reports are of different kinds: one microbenchmark A/B report and one other"
                    .to_owned(),
            );
        }
        (Err(_), Err(_)) => {}
    }
    let previous = Report::from_json(previous)
        .map_err(|e| format!("the previous report is neither an A/B report nor a run's: {e}"))?;
    let current = Report::from_json(current)
        .map_err(|e| format!("the report is neither an A/B report nor a run's: {e}"))?;
    Ok(harness(&previous, &current))
}

/// A change in percent of `before`, with its sign, such as `+3.2%`; `None`
/// for a zero `before`.
fn change_pct(before: u64, now: u64) -> Option<String> {
    crate::micro::compare::difference_pct(before, now).map(|pct| format!("{pct:+.1}%"))
}

/// Microseconds with three decimals.
fn us(ns: u64) -> String {
    crate::micro::markdown::us(ns)
}

/// Two A/B reports' head medians and differences, benchmark by benchmark.
pub fn micro(previous: &ab::Report, current: &ab::Report) -> String {
    let mut out = String::new();
    out.push_str("## Against the previous scheduled run\n\n");
    out.push_str(&format!(
        "Head's median of each benchmark in the previous scheduled run, at {}, and in this \
         one, at {}, with the change between them, and each run's difference against its base. \
         The two runs had different machines, so the changes are shown and never judged.\n\n",
        commit(previous.provenance.commit.as_ref()),
        commit(current.provenance.commit.as_ref()),
    ));
    let before: BTreeMap<&str, &ab::Benchmark> = previous
        .benchmarks
        .iter()
        .map(|b| (b.id.as_str(), b))
        .collect();
    let mut table = Table::new(&[
        "Benchmark",
        "Previous head µs",
        "Head µs",
        "Change",
        "Previous difference",
        "Difference",
    ]);
    let pct = |p: Option<f64>| or_dash(p.map(|p| format!("{p:+.1}%")));
    for b in &current.benchmarks {
        let was = before.get(b.id.as_str());
        table.row(vec![
            b.id.clone(),
            or_dash(was.map(|w| us(w.head.median_ns))),
            us(b.head.median_ns),
            match was {
                Some(w) => or_dash(change_pct(w.head.median_ns, b.head.median_ns)),
                None => "new".to_owned(),
            },
            pct(was.and_then(|w| w.difference_pct)),
            pct(b.difference_pct),
        ]);
    }
    for b in previous
        .benchmarks
        .iter()
        .filter(|b| !current.benchmarks.iter().any(|c| c.id == b.id))
    {
        table.row(vec![
            b.id.clone(),
            us(b.head.median_ns),
            "—".to_owned(),
            "gone".to_owned(),
            pct(b.difference_pct),
            "—".to_owned(),
        ]);
    }
    table.render(&mut out);
    out
}

fn commit(commit: Option<&crate::provenance::Commit>) -> String {
    or_dash(commit.map(|c| c.sha.clone()))
}

/// The key a ratio row is matched by across runs.
type Key = (String, Option<String>, String, String);

fn key(row: &Row) -> Key {
    (
        row.target_class.to_string(),
        row.fixture.map(|f| f.to_string()),
        row.flow.clone(),
        row.baseline.clone(),
    )
}

/// Two runs' ratios, flow by flow and baseline by baseline.
pub fn harness(previous: &Report, current: &Report) -> String {
    let before = ratio::of(previous);
    let now = ratio::of(current);
    let tool = now
        .tool
        .clone()
        .unwrap_or_else(|| "the tool under test".to_owned());
    let mut out = String::new();
    out.push_str("## Against the previous scheduled run\n\n");
    out.push_str(&format!(
        "Each flow's ratio against each baseline in the previous scheduled run, at {}, and in \
         this one, at {}, with {tool}'s p50 in both; rows are matched by target class, fixture, \
         flow and baseline. The two runs had different machines, so the changes are shown and \
         never judged.\n\n",
        commit(previous.provenance.commit.as_ref()),
        commit(current.provenance.commit.as_ref()),
    ));
    let was: BTreeMap<Key, &Row> = before.rows.iter().map(|r| (key(r), r)).collect();
    let previous_p50 = format!("Previous {tool} p50 ms");
    let p50 = format!("{tool} p50 ms");
    let mut table = Table::new(&[
        "Class",
        "Fixture",
        "Flow",
        "Baseline",
        "Previous ratio",
        "Ratio",
        "Change",
        &previous_p50,
        &p50,
    ]);
    let ratio_cell = |r: Option<f64>| or_dash(r.map(|r| format!("{r:.2}x")));
    for row in &now.rows {
        let then = was.get(&key(row));
        let change = match (then.and_then(|t| t.ratio), row.ratio) {
            (Some(a), Some(b)) => format!("{:+.2}", b - a),
            _ if then.is_none() => "new".to_owned(),
            _ => "—".to_owned(),
        };
        table.row(vec![
            row.target_class.to_string(),
            or_dash(row.fixture),
            row.flow.clone(),
            row.baseline.clone(),
            ratio_cell(then.and_then(|t| t.ratio)),
            ratio_cell(row.ratio),
            change,
            or_dash(then.and_then(|t| t.tool_p50_us).map(ms)),
            or_dash(row.tool_p50_us.map(ms)),
        ]);
    }
    if now.rows.is_empty() {
        out.push_str("No baseline ran a flow in this run.\n\n");
    } else {
        table.render(&mut out);
    }
    out
}
