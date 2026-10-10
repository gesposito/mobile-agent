//! The run report. One value renders as JSON and as the markdown summary, so
//! both outputs carry the same numbers.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::adapter::Clock;
use crate::end_state::EndState;
use crate::fixture::{Fixture, Screen};
use crate::gates::GateReport;
use crate::handoff::HandOff;
use crate::harness::Subject;
use crate::provenance::Provenance;
use crate::stats::{Summary, percent, summarize};
use crate::target::TargetClass;

/// Version of the JSON layout, bumped on incompatible changes.
pub const SCHEMA_VERSION: u32 = 1;

/// Everything one run measured and where it ran.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub provenance: Provenance,
    pub measurements: Vec<Measurement>,
    /// Every hand-off of a target from one tool to the next, and once the run
    /// was over, in order; none happens inside a series.
    #[serde(default)]
    pub handoffs: Vec<HandOff>,
    /// The gate verdicts, once [`crate::gates::GateSet::evaluate`] has judged
    /// the run.
    #[serde(default)]
    pub gates: Option<GateReport>,
}

/// One series as measured by one tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub tool: String,
    /// The [`crate::target::Target::id`].
    pub target: String,
    pub target_class: TargetClass,
    pub fixture: Option<Fixture>,
    pub screen: Option<Screen>,
    #[serde(flatten)]
    pub subject: Subject,
    /// The clock of the measured samples; `None` without any.
    pub clock: Option<Clock>,
    /// Measured iterations, warm-up excluded.
    pub iterations: u32,
    pub warmup: u32,
    /// Over the successful measured iterations; `None` when none succeeded.
    pub stats: Option<Summary>,
    /// Failures and flakes over the measured iterations.
    pub reliability: Reliability,
    /// Node counts the tool returned over the measured iterations.
    pub nodes: Option<NodeRange>,
    /// Why the series could not run at all, such as a failed preparation.
    pub error: Option<String>,
    /// Why the tool cannot measure the subject, when it cannot: the series
    /// then has no samples and is neither failed nor gated.
    #[serde(default)]
    pub not_measured: Option<String>,
    /// For a flow with a golden, the end state the tool's check run read: the
    /// golden when it matched, which it must before the flow is timed.
    #[serde(default)]
    pub end_state: Option<EndState>,
    /// For snapshots whose replies say what they cost the tool, the measured
    /// ones' time to answer, those served from the replica apart from those
    /// that read the device.
    #[serde(default)]
    pub reads: Option<Reads>,
    /// Every iteration in order, warm-up included and flagged.
    pub samples: Vec<SampleRecord>,
}

/// What a snapshot series cost the tool on its own side: the time to answer
/// of the measured snapshots it served from its replica, and of those that
/// read the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reads {
    /// Over the snapshots served from the replica; `None` when none was.
    pub replica: Option<Summary>,
    /// Over the snapshots that read the device; `None` when none did.
    pub device: Option<Summary>,
}

impl Reads {
    /// Over the measured samples among `samples` that say what they cost;
    /// `None` when none says.
    pub fn of(samples: &[SampleRecord]) -> Option<Reads> {
        let said: Vec<(u64, bool)> = samples
            .iter()
            .filter(|s| !s.warmup)
            .filter_map(|s| s.read_us.zip(s.served_from_replica))
            .collect();
        if said.is_empty() {
            return None;
        }
        let times = |replica: bool| {
            let us: Vec<u64> = said
                .iter()
                .filter(|(_, served)| *served == replica)
                .map(|(us, _)| *us)
                .collect();
            summarize(&us)
        };
        Some(Reads {
            replica: times(true),
            device: times(false),
        })
    }
}

/// Failures and flakes over a series' measured iterations, warm-up excluded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reliability {
    /// Measured iterations.
    pub iterations: u32,
    /// Iterations in which no attempt succeeded.
    pub failures: u32,
    /// Iterations that failed and then succeeded on a retry.
    pub flakes: u32,
    /// `failures` as a percentage of `iterations`, to a tenth.
    pub failure_rate_pct: f64,
    /// `flakes` as a percentage of `iterations`, to a tenth.
    pub flake_rate_pct: f64,
}

impl Reliability {
    /// Counts over the measured iterations in `samples`. A series with no
    /// measured sample at all, because its preparation failed, failed each of
    /// its `iterations`.
    pub fn of(samples: &[SampleRecord], iterations: u32) -> Self {
        let measured: Vec<&SampleRecord> = samples.iter().filter(|s| !s.warmup).collect();
        let count = |outcome| {
            let n = measured.iter().filter(|s| s.outcome == outcome).count();
            u32::try_from(n).unwrap_or(u32::MAX)
        };
        let (failures, flakes) = if measured.is_empty() {
            (iterations, 0)
        } else {
            (count(Outcome::Fail), count(Outcome::Flake))
        };
        Reliability {
            iterations,
            failures,
            flakes,
            failure_rate_pct: percent(failures, iterations),
            flake_rate_pct: percent(flakes, iterations),
        }
    }
}

/// Smallest and largest node count among a series' samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRange {
    pub min: u32,
    pub max: u32,
}

/// One iteration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleRecord {
    /// Zero-based position in the series, warm-up iterations first.
    pub iteration: u32,
    pub warmup: bool,
    pub outcome: Outcome,
    /// Attempts made, including the one that succeeded.
    pub attempts: u32,
    /// Latency of the successful attempt.
    pub us: Option<u64>,
    pub nodes: Option<u32>,
    pub detail: Option<String>,
    /// Messages of the failed attempts, in order.
    pub errors: Vec<String>,
    /// For a snapshot, the tool's own time to answer, when its reply says:
    /// `mobile-agent`'s `readMs`, in whole microseconds.
    #[serde(default)]
    pub read_us: Option<u64>,
    /// For a snapshot, whether the tool answered from its replica without
    /// reading the device, when its reply says: `servedFromReplica`.
    #[serde(default)]
    pub served_from_replica: Option<bool>,
}

/// How an iteration ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The first attempt succeeded.
    Pass,
    /// An attempt failed and a retry succeeded.
    Flake,
    /// No attempt succeeded.
    Fail,
}

/// Where [`Report::write`] put the two outputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportFiles {
    pub json: PathBuf,
    pub markdown: PathBuf,
}

impl Report {
    /// Pretty-printed JSON.
    pub fn to_json(&self) -> String {
        // Every key is a string and serde_json writes non-finite floats as
        // null, so serializing a report cannot fail.
        serde_json::to_string_pretty(self).expect("a report always serializes")
    }

    /// Parses a report written by [`Report::to_json`].
    pub fn from_json(text: &str) -> Result<Report, String> {
        let report: Report =
            serde_json::from_str(text).map_err(|e| format!("not a benchmark report: {e}"))?;
        if report.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "report schema version {} is not the supported {SCHEMA_VERSION}",
                report.schema_version
            ));
        }
        Ok(report)
    }

    /// The markdown summary.
    pub fn to_markdown(&self) -> String {
        crate::markdown::render(self)
    }

    /// Writes `report.json` and `report.md` into `dir`, creating it.
    pub fn write(&self, dir: &Path) -> io::Result<ReportFiles> {
        fs::create_dir_all(dir)?;
        let files = ReportFiles {
            json: dir.join("report.json"),
            markdown: dir.join("report.md"),
        };
        fs::write(&files.json, self.to_json() + "\n")?;
        fs::write(&files.markdown, self.to_markdown())?;
        Ok(files)
    }
}
