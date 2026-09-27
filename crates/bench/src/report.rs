//! The run report. One value renders as JSON and as the markdown summary, so
//! both outputs carry the same numbers.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::adapter::Clock;
use crate::fixture::{Fixture, Screen};
use crate::harness::Subject;
use crate::provenance::Provenance;
use crate::stats::Summary;
use crate::target::TargetClass;

/// Version of the JSON layout, bumped on incompatible changes.
pub const SCHEMA_VERSION: u32 = 1;

/// Everything one run measured and where it ran.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub provenance: Provenance,
    pub measurements: Vec<Measurement>,
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
    /// Node counts the tool returned over the measured iterations.
    pub nodes: Option<NodeRange>,
    /// Why the series could not run at all, such as a failed preparation.
    pub error: Option<String>,
    /// Every iteration in order, warm-up included and flagged.
    pub samples: Vec<SampleRecord>,
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
}

/// How an iteration ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The first attempt succeeded.
    Pass,
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
        todo!("serialize {} measurements", self.measurements.len())
    }

    /// Parses a report written by [`Report::to_json`].
    pub fn from_json(text: &str) -> Result<Report, String> {
        todo!("parse {} bytes", text.len())
    }

    /// The markdown summary.
    pub fn to_markdown(&self) -> String {
        crate::markdown::render(self)
    }

    /// Writes `report.json` and `report.md` into `dir`, creating it.
    pub fn write(&self, dir: &Path) -> io::Result<ReportFiles> {
        todo!("write into {}", dir.display())
    }
}
