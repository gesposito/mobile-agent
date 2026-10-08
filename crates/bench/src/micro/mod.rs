//! `mobile-agent-bench micro`, the runner of the host microbenchmarks (design
//! D15): it refuses a busy host or waits for a quiet one ([`guard`]), runs
//! every suite through `cargo bench` with criterion's files written under a
//! fresh `CRITERION_HOME`, reads each benchmark's median, interval, sample
//! count and allocations ([`criterion`]), compares them with the committed
//! baseline ([`compare`], design D16), and writes `report.json` and
//! `report.md` from one value ([`report`], [`markdown`]).
//!
//! The host, the provenance commands and the suites are each read through a
//! seam, [`guard::Host`], [`Probe`] and [`Suites`], so tests script the
//! readings and replace `cargo bench` with recorded output.

pub mod compare;
pub mod criterion;
pub mod guard;
pub mod markdown;
pub mod report;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::provenance::Probe;
use crate::report::ReportFiles;

use self::guard::Host;
use self::report::Report;

/// Version of the JSON layouts of `report.json` and the baseline file, bumped
/// on incompatible changes.
pub const SCHEMA_VERSION: u32 = 1;

/// The committed baseline, relative to the repository root.
pub const BASELINE: &str = "reports/micro/baseline.json";

/// The default bound on the 1-minute load average, the device reports' bound,
/// which spike S8 kept (design D15).
pub const MAX_LOAD: f64 = 4.0;

/// The default threshold in percent: twice criterion's quiet-host CV of
/// 1.15% that spike S8 measured, raised to the floor of 5% (design D16).
pub const THRESHOLD_PCT: f64 = 5.0;

/// The `cargo` arguments that run every suite in the bench profile, release
/// settings: the bench targets of `protocol`, `mirror`, `daemon`, `simulator`
/// and `android`, by name, so no library's or binary's libtest harness runs.
pub const CARGO_BENCH: [&str; 10] = [
    "bench",
    "--workspace",
    "--bench",
    "codec",
    "--bench",
    "mirror",
    "--bench",
    "daemon",
    "--bench",
    "frames",
];

/// criterion's flags after `--`: no plots. Its default sample settings
/// apply (design D13), and it writes `estimates.json` and `sample.json` for
/// every benchmark it measures.
pub const CRITERION_FLAGS: [&str; 1] = ["--noplot"];

/// What one run is asked to do.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The repository root: where `cargo bench` runs, `Cargo.lock` is read
    /// and the baseline lives.
    pub root: PathBuf,
    /// Where `report.json` and `report.md` go; `None` for
    /// `<root>/target/bench/micro/<UTC start time>/`.
    pub out: Option<PathBuf>,
    /// The 1-minute load average must be under this.
    pub max_load: f64,
    /// How long to wait for a quiet host; zero refuses a busy one at once.
    pub wait: Duration,
    /// A median slower or faster than the baseline's by more than this
    /// percentage is a regression or an improvement.
    pub threshold_pct: f64,
    /// criterion's filter, a regular expression over the benchmark ids.
    pub filter: Option<String>,
    /// Rewrite the baseline with this run's values and provenance.
    pub save_baseline: bool,
    /// The host's `std::env::consts::OS`, for provenance and the load.
    pub os: String,
}

impl Settings {
    /// The defaults for a checkout at `root` on this host: no wait, the
    /// default bound and threshold, every benchmark, no baseline written.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Settings {
            root: root.into(),
            out: None,
            max_load: MAX_LOAD,
            wait: Duration::ZERO,
            threshold_pct: THRESHOLD_PCT,
            filter: None,
            save_baseline: false,
            os: std::env::consts::OS.to_owned(),
        }
    }
}

/// Runs the suites: `cargo bench` on this host, recorded output in tests.
pub trait Suites {
    /// Runs every suite, or the benchmarks `filter` matches, with criterion's
    /// files written under `criterion_home`, and returns what the suites
    /// printed on stdout. Fails when the suites could not run or one failed.
    fn run(&mut self, criterion_home: &Path, filter: Option<&str>) -> Result<String, String>;
}

/// `cargo bench` with [`CARGO_BENCH`] and [`CRITERION_FLAGS`], run in the
/// repository root. Its stdout is echoed line by line as it comes and kept
/// for the allocation lines; its stderr, cargo's progress among it, goes
/// straight through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CargoBench {
    /// The cargo to run: `$CARGO`, which `cargo run` sets, else `cargo` on
    /// `PATH`.
    pub program: PathBuf,
    pub root: PathBuf,
}

impl CargoBench {
    /// The cargo `$CARGO` names, else `cargo`, run in `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        CargoBench {
            program: std::env::var_os("CARGO")
                .map_or_else(|| PathBuf::from("cargo"), PathBuf::from),
            root: root.into(),
        }
    }

    /// The arguments for `filter`: [`CARGO_BENCH`], `--`, [`CRITERION_FLAGS`]
    /// and the filter, if any.
    pub fn args(filter: Option<&str>) -> Vec<String> {
        let _ = filter;
        todo!()
    }
}

impl Suites for CargoBench {
    fn run(&mut self, criterion_home: &Path, filter: Option<&str>) -> Result<String, String> {
        let _ = (criterion_home, filter);
        todo!()
    }
}

/// A run that measured: its report, where the report went, and the baseline
/// it wrote, if asked to.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub report: Report,
    pub files: ReportFiles,
    pub baseline: Option<PathBuf>,
}

/// One run of design D15: refuses, before anything is measured, a
/// `--save-baseline` with a raised load bound or a filter, a baseline file it
/// cannot read, and a host that is not quiet within `settings.wait`; then
/// runs the suites, reads what criterion wrote, compares it with the
/// baseline, writes both reports and, when asked, the baseline. Every error
/// is one the runner exits 2 on; the report's summary holds the exit status
/// of a run that measured, 1 on any regression and 0 otherwise.
pub fn run(
    settings: &Settings,
    host: &mut dyn Host,
    probe: &dyn Probe,
    suites: &mut dyn Suites,
) -> Result<Finished, String> {
    let _ = (settings, host, probe, suites);
    todo!()
}
