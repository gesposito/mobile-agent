//! What criterion wrote for a run (design D13): for each benchmark, the
//! median's point estimate and 95% interval from
//! `$CRITERION_HOME/<id>/new/estimates.json` and the sample count from
//! `new/sample.json`, with the allocations its bench printed on stdout as
//! `allocations <id> <allocations> <bytes>`. criterion's book calls these
//! files private, so the version stays pinned in `Cargo.lock` and the reader
//! is tested on files spike S8 recorded.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// One benchmark as a run measured it, as the report and the baseline record
/// it. Times are whole nanoseconds, criterion's estimates rounded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measured {
    /// criterion's id, `<crate>/<function>/<input>`: the benchmark's
    /// directory under `CRITERION_HOME`.
    pub id: String,
    /// The median's point estimate.
    pub median_ns: u64,
    /// The median's 95% interval.
    pub spread_ns: Spread,
    /// The samples criterion took.
    pub samples: u32,
    /// Allocations per iteration, where the bench counts them.
    pub allocations: Option<u64>,
    /// Bytes those allocations asked for.
    pub bytes_allocated: Option<u64>,
}

/// An interval, low and high bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spread {
    pub low: u64,
    pub high: u64,
}

/// Every benchmark criterion wrote under `home`, sorted by id, with the
/// allocations `stdout` reports for it. Only `new/` directories count; the
/// `base/` copies criterion keeps beside them do not. The suites print an
/// allocation line before each benchmark even when a filter skips it, and
/// under `--list`, so the lines of benchmarks that wrote no estimates in
/// `home` are dropped.
pub fn read(home: &Path, stdout: &str) -> Result<Vec<Measured>, String> {
    let mut written = Vec::new();
    walk(home, &mut written)
        .map_err(|e| format!("cannot read criterion's files in {}: {e}", home.display()))?;
    let counted = allocations(stdout);
    let mut measured = written
        .iter()
        .map(|new| benchmark(home, new, &counted))
        .collect::<Result<Vec<_>, _>>()?;
    measured.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(measured)
}

/// The part of `estimates.json` the run reads.
#[derive(Deserialize)]
struct Estimates {
    median: Estimate,
}

#[derive(Deserialize)]
struct Estimate {
    point_estimate: f64,
    confidence_interval: Interval,
}

#[derive(Deserialize)]
struct Interval {
    lower_bound: f64,
    upper_bound: f64,
}

/// The part of `sample.json` the run reads: each sample's iteration count.
#[derive(Deserialize)]
struct Sample {
    iters: Vec<f64>,
}

/// Collects every directory under `dir` named `new` that holds an
/// `estimates.json`.
fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        if entry.file_name() == "new" && path.join("estimates.json").is_file() {
            found.push(path.clone());
        }
        walk(&path, found)?;
    }
    Ok(())
}

/// The benchmark whose files are in `new`, under `home`, with its counts
/// from `counted`.
fn benchmark(
    home: &Path,
    new: &Path,
    counted: &BTreeMap<String, (u64, u64)>,
) -> Result<Measured, String> {
    let dir = new.parent().unwrap_or(new);
    let id = dir
        .strip_prefix(home)
        .unwrap_or(dir)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    let estimates: Estimates = parse(&new.join("estimates.json"), &id)?;
    let sample: Sample = parse(&new.join("sample.json"), &id)?;
    let median = &estimates.median;
    let ns = |value: f64, what: &str| {
        if value.is_finite() && value >= 0.0 {
            Ok(value.round() as u64)
        } else {
            Err(format!(
                "the estimates.json of {id} gives its median's {what} as {value}"
            ))
        }
    };
    let median_ns = ns(median.point_estimate, "point estimate")?;
    let spread_ns = Spread {
        low: ns(median.confidence_interval.lower_bound, "lower bound")?,
        high: ns(median.confidence_interval.upper_bound, "upper bound")?,
    };
    let counts = counted.get(&id).copied();
    Ok(Measured {
        id,
        median_ns,
        spread_ns,
        samples: u32::try_from(sample.iters.len()).unwrap_or(u32::MAX),
        allocations: counts.map(|(n, _)| n),
        bytes_allocated: counts.map(|(_, bytes)| bytes),
    })
}

/// The criterion file at `path`, of the benchmark `id`.
fn parse<T: DeserializeOwned>(path: &Path, id: &str) -> Result<T, String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read the {name} of {id}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("the {name} of {id} is not criterion's: {e}"))
}

/// The `allocations <id> <allocations> <bytes>` lines of `stdout`, by id: the
/// allocations and the bytes. A later line for an id replaces an earlier one.
pub fn allocations(stdout: &str) -> BTreeMap<String, (u64, u64)> {
    let mut found = BTreeMap::new();
    for line in stdout.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let ["allocations", id, n, bytes] = words[..]
            && let (Ok(n), Ok(bytes)) = (n.parse(), bytes.parse())
        {
            found.insert(id.to_owned(), (n, bytes));
        }
    }
    found
}

/// criterion's version in the text of a `Cargo.lock`, such as `0.8.2`;
/// several versions are joined by `, `.
pub fn version(lock: &str) -> Option<String> {
    let mut versions: Vec<&str> = Vec::new();
    let mut lines = lock.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line != "name = \"criterion\"" {
            continue;
        }
        let version = lines
            .next()
            .and_then(|next| next.strip_prefix("version = \""))
            .and_then(|rest| rest.strip_suffix('"'));
        if let Some(version) = version
            && !versions.contains(&version)
        {
            versions.push(version);
        }
    }
    (!versions.is_empty()).then(|| versions.join(", "))
}
