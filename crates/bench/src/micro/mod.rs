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
//!
//! [`ab`], `micro --ab <ref>`, compares the checkout with another ref built
//! and run on the same host instead of with the baseline (design D2 of
//! `ci-benchmarks`), through the same guard, reader and reports, its two
//! trees read through the seam [`ab::Trees`].

pub mod ab;
pub mod compare;
pub mod criterion;
pub mod guard;
pub mod markdown;
pub mod report;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::provenance::{Probe, capture_commit, capture_host, compact_timestamp, utc_timestamp};
use crate::report::ReportFiles;

use self::compare::Baseline;
use self::guard::Host;
use self::report::{GuardSettings, HarnessSettings, Provenance, Report};

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

/// The `cargo` arguments that run every suite in the bench profile, which
/// takes the release profile's settings: the bench targets of `protocol`,
/// `mirror`, `daemon`, `simulator`, `android` and `mcp`, by name, so no
/// library's or binary's libtest harness runs.
pub const CARGO_BENCH: [&str; 12] = [
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
    "--bench",
    "mcp",
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
    /// percentage, and outside the interval the baseline recorded around its
    /// own median, is a regression or an improvement.
    pub threshold_pct: f64,
    /// criterion's filter, a regular expression over the benchmark ids.
    pub filter: Option<String>,
    /// Rewrite the baseline with this run's values and provenance.
    pub save_baseline: bool,
    /// The host's `std::env::consts::OS`, which says how provenance reads
    /// the host.
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
    /// The target directory cargo is given as `CARGO_TARGET_DIR`; `None`
    /// for cargo's own choice.
    pub target_dir: Option<PathBuf>,
    /// The toolchain cargo is given as `RUSTUP_TOOLCHAIN`; `None` for the
    /// one rustup resolves in `root`.
    pub toolchain: Option<String>,
}

impl CargoBench {
    /// The cargo `$CARGO` names, else `cargo`, run in `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        CargoBench {
            program: std::env::var_os("CARGO")
                .map_or_else(|| PathBuf::from("cargo"), PathBuf::from),
            root: root.into(),
            target_dir: None,
            toolchain: None,
        }
    }

    /// The command for `args`, in `root` with the target directory and the
    /// toolchain this cargo is given.
    fn command(&self, args: &[String]) -> Command {
        let mut command = Command::new(&self.program);
        command.args(args).current_dir(&self.root);
        if let Some(dir) = &self.target_dir {
            command.env("CARGO_TARGET_DIR", dir);
        }
        if let Some(toolchain) = &self.toolchain {
            command.env("RUSTUP_TOOLCHAIN", toolchain);
        }
        command
    }

    /// Builds every suite without running it: [`CARGO_BENCH`] and
    /// `--no-run`, cargo's output going straight through.
    pub fn build(&self) -> Result<(), String> {
        let args: Vec<String> = CARGO_BENCH
            .into_iter()
            .chain(["--no-run"])
            .map(str::to_owned)
            .collect();
        let program = self.program.display().to_string();
        let status = self
            .command(&args)
            .stdin(Stdio::null())
            .status()
            .map_err(|e| format!("cannot start {program}: {e}"))?;
        if !status.success() {
            return Err(format!(
                "cargo bench --no-run failed in {} with {status}; nothing was measured",
                self.root.display()
            ));
        }
        Ok(())
    }

    /// The arguments for `filter`: [`CARGO_BENCH`], `--`, [`CRITERION_FLAGS`]
    /// and the filter, if any.
    pub fn args(filter: Option<&str>) -> Vec<String> {
        CARGO_BENCH
            .into_iter()
            .chain(["--"])
            .chain(CRITERION_FLAGS)
            .chain(filter)
            .map(str::to_owned)
            .collect()
    }
}

impl Suites for CargoBench {
    fn run(&mut self, criterion_home: &Path, filter: Option<&str>) -> Result<String, String> {
        let program = self.program.display().to_string();
        let mut child = self
            .command(&CargoBench::args(filter))
            .env("CRITERION_HOME", criterion_home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("cannot start {program}: {e}"))?;
        let mut captured = String::new();
        if let Some(stdout) = child.stdout.take() {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let text = String::from_utf8_lossy(&line);
                        print!("{text}");
                        captured.push_str(&text);
                    }
                    Err(e) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("cannot read what cargo bench printed: {e}"));
                    }
                }
            }
        }
        let status = child
            .wait()
            .map_err(|e| format!("cannot wait for {program}: {e}"))?;
        if !status.success() {
            return Err(format!(
                "cargo bench failed with {status}; nothing was compared"
            ));
        }
        Ok(captured)
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
    if settings.save_baseline {
        // A baseline never comes from a host the default guard would refuse,
        // nor from part of the suites.
        if settings.max_load > MAX_LOAD {
            return Err(format!(
                "--save-baseline takes a 1-minute load under {MAX_LOAD} at most, the default \
                 bound; --max-load {} would let a busier host write the baseline",
                settings.max_load
            ));
        }
        if let Some(filter) = &settings.filter {
            return Err(format!(
                "--save-baseline writes every suite's values, and --filter {filter} measures \
                 only some of them"
            ));
        }
    }
    let baseline_path = settings.root.join(BASELINE);
    let baseline = Baseline::load(&baseline_path)?;
    let quiet = guard::wait_quiet(host, settings.max_load, settings.wait)?;

    let started = SystemTime::now();
    let out = settings.out.clone().unwrap_or_else(|| {
        settings
            .root
            .join("target/bench/micro")
            .join(compact_timestamp(started))
    });
    // The bench binaries run in their crates' directories, so criterion is
    // given an absolute path, and a directory of its own, so that every
    // estimates.json in it is this run's.
    let criterion_home = std::path::absolute(out.join("criterion"))
        .map_err(|e| format!("no absolute path for {}: {e}", out.display()))?;
    fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    fs::create_dir(&criterion_home).map_err(|e| {
        format!(
            "cannot create a fresh {} for criterion's files: {e}",
            criterion_home.display()
        )
    })?;
    let mut machine = capture_host(probe, &settings.os);
    machine.load_average = Some(quiet.load_average);
    let commit = capture_commit(probe);
    let toolchain = probe.output("rustc", &["-V"]);
    let library = fs::read_to_string(settings.root.join("Cargo.lock"))
        .ok()
        .and_then(|lock| criterion::version(&lock));

    let stdout = suites.run(&criterion_home, settings.filter.as_deref())?;
    let measured = criterion::read(&criterion_home, &stdout)?;
    if measured.is_empty() {
        return Err(match &settings.filter {
            Some(filter) => {
                format!("the suites measured no benchmark whose id matches --filter {filter}")
            }
            None => "the suites measured no benchmark".to_owned(),
        });
    }
    let provenance = Provenance {
        started_at: utc_timestamp(started),
        finished_at: utc_timestamp(SystemTime::now()),
        host: machine,
        load_average_end: host.check().load_average,
        commit,
        toolchain,
        harness: HarnessSettings::criterion(library),
        guard: GuardSettings {
            max_load: settings.max_load,
            waited_s: quiet.waited.as_secs(),
        },
    };
    let mut report = Report::new(
        provenance,
        settings.threshold_pct,
        BASELINE,
        baseline.as_ref(),
        &measured,
    );

    // The baseline is written after the comparison, which read the previous
    // one, and before the reports, which say whether it was.
    let mut written = None;
    let mut unwritten = None;
    if settings.save_baseline {
        let fresh = Baseline {
            schema_version: SCHEMA_VERSION,
            provenance: report.provenance.clone(),
            benchmarks: measured,
        };
        match fresh.save(&baseline_path) {
            Ok(()) => {
                report.baseline.saved = true;
                written = Some(baseline_path);
            }
            Err(e) => {
                unwritten = Some(format!(
                    "cannot write the baseline {}: {e}",
                    baseline_path.display()
                ))
            }
        }
    }
    let files = report
        .write(&out)
        .map_err(|e| format!("cannot write the report into {}: {e}", out.display()))?;
    if let Some(error) = unwritten {
        return Err(format!("{error}; the reports are in {}", out.display()));
    }
    Ok(Finished {
        report,
        files,
        baseline: written,
    })
}
