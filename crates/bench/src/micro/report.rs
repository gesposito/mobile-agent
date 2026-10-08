//! The microbenchmark report: the run's provenance, every benchmark with its
//! comparison, the baseline's benchmarks the run did not measure, and a
//! summary with the exit status and its reason. One value renders as
//! `report.json` and as `report.md`, so both carry the same numbers.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::compare::{Baseline, Benchmark};
use super::criterion::Measured;
use crate::provenance::{Commit, Host};
use crate::report::ReportFiles;

/// Where and how a run measured, as the device reports record it, with the
/// harness library, its sample settings and the guard's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// RFC 3339 UTC, second precision: when the guard found the host quiet.
    pub started_at: String,
    /// When the suites had run.
    pub finished_at: String,
    /// The host; its `load_average` is the 1, 5 and 15 minute load averages
    /// at the check that found it quiet, the run's start.
    pub host: Host,
    /// The 1, 5 and 15 minute load averages once the suites had run.
    pub load_average_end: Option<[f64; 3]>,
    /// The checkout the suites were built from.
    pub commit: Option<Commit>,
    /// The Rust toolchain, as `rustc -V` prints it in the checkout.
    pub toolchain: Option<String>,
    pub harness: HarnessSettings,
    pub guard: GuardSettings,
}

/// The harness library and the sample settings it ran with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessSettings {
    /// `criterion`.
    pub library: String,
    /// Its version in `Cargo.lock`.
    pub version: Option<String>,
    /// Warm-up per benchmark, in seconds.
    pub warm_up_s: u32,
    /// Measurement per benchmark, in seconds, which criterion stretches when
    /// its samples need longer.
    pub measurement_s: u32,
    /// Samples per benchmark, unless a bench sets its own; each benchmark's
    /// `samples` says what it took.
    pub sample_size: u32,
    /// Bootstrap resamples for the intervals.
    pub resamples: u32,
    /// The intervals' confidence level, in percent.
    pub confidence_pct: u32,
}

impl HarnessSettings {
    /// criterion at `version` with the default sample settings `micro` runs
    /// it with (design D13): 3 s of warm-up, 5 s of measurement, 100
    /// samples, 100,000 bootstrap resamples, 95% intervals.
    pub fn criterion(version: Option<String>) -> Self {
        HarnessSettings {
            library: "criterion".to_owned(),
            version,
            warm_up_s: 3,
            measurement_s: 5,
            sample_size: 100,
            resamples: 100_000,
            confidence_pct: 95,
        }
    }
}

/// The guard the run passed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuardSettings {
    /// The bound the 1-minute load average was under.
    pub max_load: f64,
    /// How long the run waited for a quiet host, in seconds.
    pub waited_s: u64,
}

/// The baseline the run compared against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaselineInfo {
    /// The baseline file, relative to the repository root.
    pub path: String,
    /// The provenance of the run that saved it; `None` when there was no
    /// baseline file.
    pub provenance: Option<Provenance>,
    /// The environment fields in which the baseline's run differs from this
    /// one, such as `host.os` or `toolchain` (see
    /// [`Provenance::environment_differences`]).
    pub environment_differences: Vec<String>,
    /// This run rewrote the baseline with its own values, after comparing.
    pub saved: bool,
}

/// What the run found, with its exit status and the reason for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub benchmarks: u32,
    pub regressions: u32,
    pub improvements: u32,
    /// Within the threshold, with the same allocations.
    pub same: u32,
    pub new: u32,
    pub missing: u32,
    /// 1 on any regression, else 0.
    pub exit_status: u8,
    pub reason: String,
}

/// Everything one microbenchmark run measured, compared and where it ran.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub provenance: Provenance,
    pub threshold_pct: f64,
    pub baseline: BaselineInfo,
    /// Sorted by id.
    pub benchmarks: Vec<Benchmark>,
    /// The baseline's benchmarks the run did not measure, sorted by id.
    pub missing: Vec<Measured>,
    pub summary: Summary,
}

impl Provenance {
    /// The environment fields in which `self` and `other` differ: the host's
    /// hardware, cores, memory and OS, the toolchain and the harness. The
    /// commit, the times, the loads and the guard are the run's own, and
    /// never listed.
    pub fn environment_differences(&self, other: &Provenance) -> Vec<String> {
        let _ = other;
        todo!()
    }
}

impl Report {
    /// The report of a run that measured `measured`, compared with
    /// `baseline`, the file at `baseline_path`, if there was one.
    pub fn new(
        provenance: Provenance,
        threshold_pct: f64,
        baseline_path: &str,
        baseline: Option<&Baseline>,
        measured: &[Measured],
    ) -> Report {
        let _ = (provenance, threshold_pct, baseline_path, baseline, measured);
        todo!()
    }

    /// Pretty-printed JSON.
    pub fn to_json(&self) -> String {
        todo!()
    }

    /// Parses a report written by [`Report::to_json`].
    pub fn from_json(text: &str) -> Result<Report, String> {
        let _ = text;
        todo!()
    }

    /// The markdown summary.
    pub fn to_markdown(&self) -> String {
        super::markdown::render(self)
    }

    /// The regressions and the improvements as tables, then the summary
    /// line: what the run prints once it is over.
    pub fn verdict(&self) -> String {
        super::markdown::verdict(self)
    }

    /// Writes `report.json` and `report.md` into `dir`, creating it.
    pub fn write(&self, dir: &Path) -> io::Result<ReportFiles> {
        let _ = (dir, fs::create_dir_all::<&Path>);
        todo!()
    }
}
