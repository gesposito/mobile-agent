//! The committed baseline and the comparison of design D16. A median slower
//! than the baseline's by more than the threshold and above the 95% interval
//! the baseline recorded around its own median is a regression, and one
//! faster by more than it and below that interval an improvement: a median
//! inside the interval is within the baseline's own uncertainty, whatever
//! its difference. A baseline entry without an interval is judged on the
//! threshold alone. Allocations and bytes are deterministic, so any change
//! in them is a regression or an improvement, whatever the time. A benchmark
//! the baseline lacks is new and one the run lacks missing, neither a
//! regression.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::SCHEMA_VERSION;
use super::criterion::{Measured, Spread};
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
        let baseline: Baseline = serde_json::from_str(text)
            .map_err(|e| format!("not a microbenchmark baseline: {e}"))?;
        if baseline.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "baseline schema version {} is not the supported {SCHEMA_VERSION}",
                baseline.schema_version
            ));
        }
        Ok(baseline)
    }

    /// Pretty-printed JSON, the benchmarks one after another in id order, so
    /// a refreshed baseline's diff shows what moved.
    pub fn to_json(&self) -> String {
        // Every key is a string and serde_json writes non-finite floats as
        // null, so serializing a baseline cannot fail.
        serde_json::to_string_pretty(self).expect("a baseline always serializes")
    }

    /// The baseline at `path`; `None` when there is no file there.
    pub fn load(path: &Path) -> Result<Option<Baseline>, String> {
        match fs::read_to_string(path) {
            Ok(text) => Baseline::from_json(&text)
                .map(Some)
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    /// Writes the baseline to `path`, creating its directory.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, self.to_json() + "\n")
    }
}

/// How a benchmark, or one of its measures, compares with the baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Regression,
    Improvement,
    /// Within the threshold or inside the baseline's interval, or the same
    /// allocations and bytes.
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
    /// The median against the threshold and the baseline's interval; `None`
    /// when new.
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
    let saved = baseline.map_or(&[][..], |b| &b.benchmarks[..]);
    let benchmarks = measured
        .iter()
        .map(|m| judge(m, saved.iter().find(|s| s.id == m.id), threshold_pct))
        .collect();
    let mut missing: Vec<Measured> = saved
        .iter()
        .filter(|s| !measured.iter().any(|m| m.id == s.id))
        .cloned()
        .collect();
    missing.sort_by(|a, b| a.id.cmp(&b.id));
    Comparison {
        benchmarks,
        missing,
    }
}

/// `measured` against `saved`, the baseline's values for it, if any.
fn judge(measured: &Measured, saved: Option<&Measured>, threshold_pct: f64) -> Benchmark {
    let Some(saved) = saved else {
        return Benchmark {
            measured: measured.clone(),
            baseline: None,
            difference_pct: None,
            time_verdict: None,
            allocations_verdict: None,
            verdict: Verdict::New,
        };
    };
    let difference = difference_pct(saved.median_ns, measured.median_ns);
    let time_verdict =
        difference.map(|pct| time(pct, threshold_pct, measured.median_ns, saved.spread_ns));
    let allocations_verdict = match (
        saved.allocations.zip(saved.bytes_allocated),
        measured.allocations.zip(measured.bytes_allocated),
    ) {
        (Some((n, bytes)), Some((now_n, now_bytes))) => {
            Some(exactly(&[(n, now_n), (bytes, now_bytes)]))
        }
        _ => None,
    };
    let verdicts = [time_verdict, allocations_verdict];
    let verdict = if verdicts.contains(&Some(Verdict::Regression)) {
        Verdict::Regression
    } else if verdicts.contains(&Some(Verdict::Improvement)) {
        Verdict::Improvement
    } else {
        Verdict::Same
    };
    Benchmark {
        measured: measured.clone(),
        baseline: Some(saved.clone()),
        difference_pct: difference,
        time_verdict,
        allocations_verdict,
        verdict,
    }
}

/// A median's verdict: `pct` is how much slower it is than the baseline's,
/// negative when faster, `median_ns` its value and `interval` the one the
/// baseline recorded around its own median. A regression when slower by
/// more than `threshold_pct` and above the interval's high end, an
/// improvement when faster by more than it and below the low end, else the
/// same; a median at either end is inside. Without an interval the
/// threshold alone decides.
fn time(pct: f64, threshold_pct: f64, median_ns: u64, interval: Option<Spread>) -> Verdict {
    let above = interval.is_none_or(|i| median_ns > i.high);
    let below = interval.is_none_or(|i| median_ns < i.low);
    if pct > threshold_pct && above {
        Verdict::Regression
    } else if pct < -threshold_pct && below {
        Verdict::Improvement
    } else {
        Verdict::Same
    }
}

/// Counts as `(baseline, measured)` pairs, compared exactly: a regression
/// when any rose, an improvement when one fell and none rose.
fn exactly(counts: &[(u64, u64)]) -> Verdict {
    if counts.iter().any(|(saved, now)| now > saved) {
        Verdict::Regression
    } else if counts.iter().any(|(saved, now)| now < saved) {
        Verdict::Improvement
    } else {
        Verdict::Same
    }
}

/// How much slower `measured_ns` is than `baseline_ns`, in percent rounded
/// to a tenth, negative when faster; `None` for a zero baseline. The
/// verdicts are taken on this rounded value, the one both reports show.
pub fn difference_pct(baseline_ns: u64, measured_ns: u64) -> Option<f64> {
    if baseline_ns == 0 {
        return None;
    }
    let pct = (measured_ns as f64 - baseline_ns as f64) / baseline_ns as f64 * 100.0;
    let tenths = (pct * 10.0).round() / 10.0;
    // A difference that rounds to nothing would print as -0.0.
    Some(if tenths == 0.0 { 0.0 } else { tenths })
}
