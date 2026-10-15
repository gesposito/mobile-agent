//! `micro --ab <base>`, the A/B mode of design D2 of `ci-benchmarks`: the
//! checkout under test, head, against another ref of the repository, base,
//! both built and measured on this host. The base is checked out into a
//! worktree of its own and built in a target directory of its own ([`Trees`]);
//! the suites then run in rounds, base and head in turn, the side that goes
//! first alternating from round to round, each run with criterion's files
//! under a fresh `CRITERION_HOME` read as `micro` reads them. A side's value
//! for a benchmark is the median of its rounds' medians, and head is judged
//! against base by the relative difference of the two against the threshold
//! alone, with allocations and bytes compared exactly ([`compare::judge`]);
//! the committed baseline is not consulted.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::compare::{self, Verdict};
use super::guard::Host;
use super::report::{Provenance, Summary};
use crate::provenance::Probe;
use crate::report::ReportFiles;

/// The default number of rounds each side runs.
pub const ROUNDS: u32 = 3;

/// The default threshold in percent of an A/B run, until spike S12 measures
/// the runners' noise (design D3).
pub const THRESHOLD_PCT: f64 = 10.0;

/// One of the two trees an A/B run measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The ref the checkout is compared against.
    Base,
    /// The checkout itself.
    Head,
}

/// The order the sides run in, round by round.
pub fn order(rounds: u32) -> Vec<[Side; 2]> {
    let _ = rounds;
    Vec::new()
}

/// What one A/B run is asked to do.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The checkout under test, head: where its suites build and run, where
    /// `Cargo.lock` is read and the base's target directory lives.
    pub root: PathBuf,
    /// The ref to compare against, as given.
    pub base: String,
    /// Where `report.json` and `report.md` go; `None` for
    /// `<root>/target/bench/micro-ab/<UTC start time>/`.
    pub out: Option<PathBuf>,
    /// How many times each side runs the suites.
    pub rounds: u32,
    /// The 1-minute load average must be under this.
    pub max_load: f64,
    /// How long to wait for a quiet host; zero refuses a busy one at once.
    pub wait: Duration,
    /// A head median slower or faster than base's by more than this
    /// percentage is a regression or an improvement.
    pub threshold_pct: f64,
    /// criterion's filter, a regular expression over the benchmark ids.
    pub filter: Option<String>,
    /// The toolchain both sides build and run with, which cargo is given as
    /// `RUSTUP_TOOLCHAIN`; `None` when rustup names none.
    pub toolchain: Option<String>,
    /// The host's `std::env::consts::OS`.
    pub os: String,
}

/// The two trees: head, the checkout, and base, checked out for the run.
pub trait Trees {
    /// Checks the base commit `sha` out and builds both sides' suites.
    fn prepare(&mut self, sha: &str) -> Result<(), String>;

    /// Runs `side`'s suites, or the benchmarks `filter` matches, with
    /// criterion's files under `criterion_home`, and returns what they
    /// printed on stdout.
    fn run(
        &mut self,
        side: Side,
        criterion_home: &Path,
        filter: Option<&str>,
    ) -> Result<String, String>;

    /// Removes what [`Trees::prepare`] checked out. Called once the rounds
    /// are over, whether or not they completed.
    fn finish(&mut self) -> Result<(), String>;
}

/// A benchmark's values on one side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideResult {
    /// The median of the rounds' medians.
    pub median_ns: u64,
    /// Each round's median, in round order.
    pub rounds_ns: Vec<u64>,
    /// The samples criterion took in the round whose median is the side's.
    pub samples: u32,
    /// Allocations per iteration in that round, where the bench counts them.
    pub allocations: Option<u64>,
    /// Bytes those allocations asked for.
    pub bytes_allocated: Option<u64>,
}

/// One benchmark of head, with base's values and the verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Benchmark {
    pub id: String,
    /// Base's values; `None` when base lacks the benchmark.
    pub base: Option<SideResult>,
    pub head: SideResult,
    /// How much slower head's median is than base's, in percent to a tenth,
    /// negative when faster; `None` when new.
    pub difference_pct: Option<f64>,
    pub time_verdict: Option<Verdict>,
    pub allocations_verdict: Option<Verdict>,
    pub verdict: Verdict,
}

/// A benchmark base measured and head did not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Missing {
    pub id: String,
    pub base: SideResult,
}

/// The ref head was compared against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    /// As given to `--ab`.
    #[serde(rename = "ref")]
    pub name: String,
    /// The commit it named.
    pub sha: String,
}

/// Everything one A/B run measured and compared.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    /// Where and how the run measured; `commit` is head's.
    pub provenance: Provenance,
    pub base: Base,
    pub rounds: u32,
    /// The sides in the order they ran, round by round.
    pub order: Vec<[Side; 2]>,
    /// The toolchain both sides were built and run with, as cargo was given
    /// it in `RUSTUP_TOOLCHAIN`; `None` when rustup named none.
    pub rustup_toolchain: Option<String>,
    pub threshold_pct: f64,
    /// Head's benchmarks, sorted by id.
    pub benchmarks: Vec<Benchmark>,
    /// Base's benchmarks head did not measure, sorted by id.
    pub missing: Vec<Missing>,
    pub summary: Summary,
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
        serde_json::from_str(text).map_err(|e| format!("not a microbenchmark A/B report: {e}"))
    }

    /// The markdown summary.
    pub fn to_markdown(&self) -> String {
        String::new()
    }

    /// The regressions and the improvements as tables, then the summary
    /// line: what the run prints once it is over.
    pub fn verdict(&self) -> String {
        String::new()
    }

    /// Writes `report.json` and `report.md` into `dir`, creating it.
    pub fn write(&self, dir: &Path) -> std::io::Result<ReportFiles> {
        let _ = dir;
        Err(std::io::Error::other("not written"))
    }
}

/// An A/B run that measured: its report, where the report went, and why the
/// base's checkout could not be removed, when it could not.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub report: Report,
    pub files: ReportFiles,
    pub leftover: Option<String>,
}

/// One A/B run of design D2.
pub fn run(
    settings: &Settings,
    host: &mut dyn Host,
    probe: &dyn Probe,
    trees: &mut dyn Trees,
) -> Result<Finished, String> {
    let _ = (settings, host, probe, trees, compare::difference_pct);
    Err("the A/B mode is not built yet".to_owned())
}

/// The trees on this host: head is the checkout at `root`; base is checked
/// out with `git worktree add --detach` into `dir`, outside the checkout, and
/// built in a target directory of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktrees {
    /// The checkout under test.
    pub root: PathBuf,
    /// Where the base is checked out.
    pub dir: PathBuf,
    /// The suites of each side.
    pub head: super::CargoBench,
    pub base: super::CargoBench,
}

impl Worktrees {
    /// Head at `root`, base checked out into `dir` and built in `target`,
    /// both with `toolchain` when it is given.
    pub fn new(
        root: impl Into<PathBuf>,
        dir: impl Into<PathBuf>,
        target: impl Into<PathBuf>,
        toolchain: Option<String>,
    ) -> Self {
        let root = root.into();
        let dir = dir.into();
        let _ = (target.into(), toolchain);
        Worktrees {
            head: super::CargoBench::new(&root),
            base: super::CargoBench::new(&dir),
            root,
            dir,
        }
    }
}

impl Trees for Worktrees {
    fn prepare(&mut self, sha: &str) -> Result<(), String> {
        let _ = sha;
        Err("the A/B mode is not built yet".to_owned())
    }

    fn run(
        &mut self,
        side: Side,
        criterion_home: &Path,
        filter: Option<&str>,
    ) -> Result<String, String> {
        let _ = (side, criterion_home, filter);
        Err("the A/B mode is not built yet".to_owned())
    }

    fn finish(&mut self) -> Result<(), String> {
        Ok(())
    }
}
