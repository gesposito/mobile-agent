//! The committed baseline and the comparison of design D16. A median slower
//! than the baseline's by more than the threshold is a regression, and one
//! faster by more than it an improvement; allocations and bytes are
//! deterministic, so any change in them is a regression or an improvement,
//! whatever the time. A benchmark the baseline lacks is new and one the run
//! lacks missing, neither a regression.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::SCHEMA_VERSION;
use super::criterion::Measured;
use super::report::Provenance;

/// The baseline file, `reports/micro/baseline.json`: the values and the
/// provenance of the run that saved it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub schema_version: u32,
    pub provenance: Provenance,
    /// Sorted by id.
    pub benchmarks: Vec<Measured>,
}

impl Baseline {
    /// Parses a baseline written by [`Baseline::to_json`].
    pub fn from_json(text: &str) -> Result<Baseline, String> {
        let _ = (text, SCHEMA_VERSION);
        todo!()
    }

    /// Pretty-printed JSON, one benchmark after another in id order, so a
    /// refreshed baseline's diff shows what moved.
    pub fn to_json(&self) -> String {
        todo!()
    }

    /// The baseline at `path`; `None` when there is no file there.
    pub fn load(path: &Path) -> Result<Option<Baseline>, String> {
        let _ = path;
        todo!()
    }

    /// Writes the baseline to `path`, creating its directory.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let _ = (path, fs::create_dir_all::<&Path>);
        todo!()
    }
}

/// How a benchmark, or one of its measures, compares with the baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Regression,
    Improvement,
    /// Within the threshold, or the same allocations and bytes.
    Same,
    /// Not in the baseline.
    New,
}

/// One benchmark of the run with its comparison.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Benchmark {
    #[serde(flatten)]
    pub measured: Measured,
    /// The baseline's values for the benchmark; `None` when it is new.
    pub baseline: Option<Measured>,
    /// How much slower the median is than the baseline's, in percent to a
    /// tenth, negative when faster; `None` when new.
    pub difference_pct: Option<f64>,
    /// The median against the threshold; `None` when new.
    pub time_verdict: Option<Verdict>,
    /// Allocations and bytes against the baseline's, compared exactly: a
    /// regression when either rose, an improvement when one fell and neither
    /// rose; `None` when the run or the baseline has none for it.
    pub allocations_verdict: Option<Verdict>,
    /// A regression when the time or the allocations regressed, else an
    /// improvement when either improved, else the same; new when the
    /// baseline lacks the benchmark.
    pub verdict: Verdict,
}

/// A run's benchmarks against the baseline.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    /// Every benchmark of the run, in its order.
    pub benchmarks: Vec<Benchmark>,
    /// The baseline's benchmarks the run did not measure, in id order.
    pub missing: Vec<Measured>,
}

/// Compares `measured` with `baseline` under `threshold_pct`; without a
/// baseline every benchmark is new.
pub fn compare(
    measured: &[Measured],
    baseline: Option<&Baseline>,
    threshold_pct: f64,
) -> Comparison {
    let _ = (measured, baseline, threshold_pct);
    todo!()
}

/// How much slower `measured_ns` is than `baseline_ns`, in percent rounded
/// to a tenth, negative when faster; `None` for a zero baseline.
pub fn difference_pct(baseline_ns: u64, measured_ns: u64) -> Option<f64> {
    let _ = (baseline_ns, measured_ns);
    todo!()
}
