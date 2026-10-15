//! The ratios of design D4 of `ci-benchmarks`: per flow, target and fixture,
//! how many times faster the tool under test's p50 was than each baseline's
//! in the same run, with the baselines the run skipped and why. On a CI
//! runner they are the judgement in place of the gates, whose limits belong
//! to the maintainer's measured hosts; a run with baselines reports them
//! wherever it ran.

use serde::{Deserialize, Serialize};

use crate::adapter::Role;
use crate::fixture::Fixture;
use crate::harness::Subject;
use crate::markdown::{Table, escape, or_dash};
use crate::provenance::ToolStatus;
use crate::report::{Measurement, Report};
use crate::stats::ms;
use crate::target::TargetClass;

/// The tool under test's flows against each baseline's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ratios {
    /// The tool under test; `None` when the run had none.
    pub tool: Option<String>,
    /// One row per flow series of the tool under test and baseline that ran
    /// the flow on the same target and fixture, in the order of the
    /// measurements and of the tools.
    pub rows: Vec<Row>,
    /// The baselines the run skipped, in the order of the tools.
    pub skipped: Vec<Skipped>,
}

/// One flow on one target and fixture, the tool under test against one
/// baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub target: String,
    pub target_class: TargetClass,
    pub fixture: Option<Fixture>,
    pub flow: String,
    pub baseline: String,
    /// The tool under test's p50; `None` without samples.
    pub tool_p50_us: Option<u64>,
    /// The baseline's p50; `None` without samples.
    pub baseline_p50_us: Option<u64>,
    /// The baseline's p50 over the tool under test's, rounded half up to a
    /// hundredth: how many times faster the tool under test was, under 1
    /// when the baseline was faster; `None` when either has no p50.
    pub ratio: Option<f64>,
    pub baseline_failure_rate_pct: f64,
    pub baseline_flake_rate_pct: f64,
    /// Why there is no ratio, such as the baseline's failed golden check,
    /// which kept it from timing the flow.
    pub note: Option<String>,
}

/// A baseline the run skipped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub tool: String,
    /// Why, as the tool's detection said.
    pub note: Option<String>,
}

/// `baseline_us` over `tool_us`, rounded half up to a hundredth; `None` for
/// a tool time of zero.
pub fn ratio(baseline_us: u64, tool_us: u64) -> Option<f64> {
    if tool_us == 0 {
        return None;
    }
    let hundredths =
        (u128::from(baseline_us) * 100 + u128::from(tool_us) / 2) / u128::from(tool_us);
    Some(hundredths as f64 / 100.0)
}

/// Why a series has no p50, when it has none.
fn missing(m: &Measurement) -> Option<String> {
    if m.stats.is_some() {
        return None;
    }
    Some(match (&m.not_measured, &m.error) {
        (Some(reason), _) => format!("not measured: {reason}"),
        (None, Some(error)) => format!("not timed: {error}"),
        (None, None) => "no samples: every measured iteration failed".to_owned(),
    })
}

/// The ratios of `report`: each flow series of its tool under test against
/// the same flow, target and fixture of every baseline that ran it.
pub fn of(report: &Report) -> Ratios {
    let _ = (report, missing, Role::UnderTest, ToolStatus::Available);
    let _: Option<Subject> = None;
    Ratios {
        tool: None,
        rows: Vec::new(),
        skipped: Vec::new(),
    }
}

impl Ratios {
    /// Whether there is anything to show: a row or a skipped baseline.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.skipped.is_empty()
    }

    /// The ratio table in markdown, then the skipped baselines; what the
    /// run prints in place of the gate table when it judges no gate.
    pub fn table(&self) -> String {
        let tool = self.tool.as_deref().unwrap_or("the tool under test");
        let mut out = String::new();
        if self.rows.is_empty() {
            out.push_str(&format!("No baseline ran a flow of {tool}.\n\n"));
        } else {
            let p50 = format!("{tool} p50 ms");
            let mut table = Table::new(&[
                "Target",
                "Class",
                "Fixture",
                "Flow",
                &p50,
                "Baseline",
                "Baseline p50 ms",
                "Ratio",
                "Baseline Fail %",
                "Baseline Flake %",
                "Note",
            ]);
            for row in &self.rows {
                table.row(vec![
                    row.target.clone(),
                    row.target_class.to_string(),
                    or_dash(row.fixture),
                    row.flow.clone(),
                    or_dash(row.tool_p50_us.map(ms)),
                    row.baseline.clone(),
                    or_dash(row.baseline_p50_us.map(ms)),
                    or_dash(row.ratio.map(|r| format!("{r:.2}x"))),
                    format!("{:.1}", row.baseline_failure_rate_pct),
                    format!("{:.1}", row.baseline_flake_rate_pct),
                    or_dash(row.note.as_deref()),
                ]);
            }
            table.render(&mut out);
        }
        if !self.skipped.is_empty() {
            let skipped: Vec<String> = self
                .skipped
                .iter()
                .map(|s| {
                    format!(
                        "{} ({})",
                        s.tool,
                        escape(s.note.as_deref().unwrap_or("absent"))
                    )
                })
                .collect();
            out.push_str(&format!("Skipped baselines: {}.\n", skipped.join("; ")));
        }
        out
    }
}
