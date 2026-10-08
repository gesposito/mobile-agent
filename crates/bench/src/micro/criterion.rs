//! What criterion wrote for a run (design D13): for each benchmark, the
//! median's point estimate and 95% interval from
//! `$CRITERION_HOME/<id>/new/estimates.json` and the sample count from
//! `new/sample.json`, with the allocations its bench printed on stdout as
//! `allocations <id> <allocations> <bytes>`. criterion's book calls these
//! files private, so the version stays pinned in `Cargo.lock` and the reader
//! is tested on files spike S8 recorded.

use std::collections::BTreeMap;
use std::path::Path;

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
    let _ = (home, stdout);
    todo!()
}

/// The `allocations <id> <allocations> <bytes>` lines of `stdout`, by id: the
/// allocations and the bytes. A later line for an id replaces an earlier one.
pub fn allocations(stdout: &str) -> BTreeMap<String, (u64, u64)> {
    let _ = stdout;
    todo!()
}

/// criterion's version in the text of a `Cargo.lock`, such as `0.8.2`;
/// several versions are joined by `, `.
pub fn version(lock: &str) -> Option<String> {
    let _ = lock;
    todo!()
}
