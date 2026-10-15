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

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use super::compare::{self, Verdict};
use super::criterion::{self, Measured};
use super::guard::{self, Host};
use super::report::{GuardSettings, HarnessSettings, Provenance, Summary};
use super::{CargoBench, MAX_LOAD, SCHEMA_VERSION};
use crate::provenance::{Probe, capture_commit, capture_host, compact_timestamp, utc_timestamp};
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

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Side::Base => "base",
            Side::Head => "head",
        })
    }
}

/// The order the sides run in, round by round: base then head in the first
/// round, head then base in the second, and so on, so that a drift through
/// the run weighs on both sides alike.
pub fn order(rounds: u32) -> Vec<[Side; 2]> {
    (0..rounds)
        .map(|round| {
            if round % 2 == 0 {
                [Side::Base, Side::Head]
            } else {
                [Side::Head, Side::Base]
            }
        })
        .collect()
}

/// The bound on the 1-minute load average an A/B run takes by default: the
/// host's core count, on a runner whose few cores the job's own setup keeps
/// busy, rather than the quiet host's 4, which is the bound when the count
/// cannot be read.
pub fn default_max_load(cpu_cores: Option<u32>) -> f64 {
    cpu_cores.map_or(MAX_LOAD, f64::from)
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
    /// The median of the rounds' medians, the nearest rank: one of them.
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

impl SideResult {
    /// The side's values as a measurement without an interval, which the
    /// comparison judges on the threshold alone.
    fn measured(&self, id: &str) -> Measured {
        Measured {
            id: id.to_owned(),
            median_ns: self.median_ns,
            spread_ns: None,
            samples: self.samples,
            allocations: self.allocations,
            bytes_allocated: self.bytes_allocated,
        }
    }
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
    /// Head's median against base's and the threshold; `None` when new.
    pub time_verdict: Option<Verdict>,
    /// Allocations and bytes against base's, compared exactly; `None` when
    /// either side counts none.
    pub allocations_verdict: Option<Verdict>,
    /// A regression when the time or the allocations regressed, else an
    /// improvement when either improved, else the same; new when base
    /// lacks the benchmark.
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

/// Each benchmark's values over `rounds`, one list of measurements per
/// round, by id.
fn sides(rounds: &[Vec<Measured>]) -> BTreeMap<String, SideResult> {
    let mut by_id: BTreeMap<String, Vec<&Measured>> = BTreeMap::new();
    for round in rounds {
        for m in round {
            by_id.entry(m.id.clone()).or_default().push(m);
        }
    }
    by_id
        .into_iter()
        .map(|(id, measured)| {
            let mut ranked = measured.clone();
            ranked.sort_by_key(|m| m.median_ns);
            // The nearest rank: the ceil(n/2)-th smallest.
            let median = ranked[measured.len().div_ceil(2) - 1];
            let side = SideResult {
                median_ns: median.median_ns,
                rounds_ns: measured.iter().map(|m| m.median_ns).collect(),
                samples: median.samples,
                allocations: median.allocations,
                bytes_allocated: median.bytes_allocated,
            };
            (id, side)
        })
        .collect()
}

impl Report {
    /// The report of a run that measured `base_rounds` and `head_rounds`,
    /// each one list of measurements per round, of `rounds` rounds in
    /// [`order`].
    pub fn new(
        provenance: Provenance,
        base: Base,
        rounds: u32,
        rustup_toolchain: Option<String>,
        threshold_pct: f64,
        base_rounds: &[Vec<Measured>],
        head_rounds: &[Vec<Measured>],
    ) -> Report {
        let base_sides = sides(base_rounds);
        let head_sides = sides(head_rounds);
        let benchmarks: Vec<Benchmark> = head_sides
            .iter()
            .map(|(id, head)| {
                let saved = base_sides.get(id);
                let judged = compare::judge(
                    &head.measured(id),
                    saved.map(|b| b.measured(id)).as_ref(),
                    threshold_pct,
                );
                Benchmark {
                    id: id.clone(),
                    base: saved.cloned(),
                    head: head.clone(),
                    difference_pct: judged.difference_pct,
                    time_verdict: judged.time_verdict,
                    allocations_verdict: judged.allocations_verdict,
                    verdict: judged.verdict,
                }
            })
            .collect();
        let missing: Vec<Missing> = base_sides
            .iter()
            .filter(|(id, _)| !head_sides.contains_key(*id))
            .map(|(id, side)| Missing {
                id: id.clone(),
                base: side.clone(),
            })
            .collect();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        let with = |verdict| count(benchmarks.iter().filter(|b| b.verdict == verdict).count());
        let regressed: Vec<&str> = benchmarks
            .iter()
            .filter(|b| b.verdict == Verdict::Regression)
            .map(|b| b.id.as_str())
            .collect();
        let reason = match regressed.len() {
            0 => format!("no regression against {}", base.name),
            n => format!(
                "{n} {} against {}: {}",
                if n == 1 { "regression" } else { "regressions" },
                base.name,
                regressed.join(", ")
            ),
        };
        let summary = Summary {
            benchmarks: count(benchmarks.len()),
            regressions: with(Verdict::Regression),
            improvements: with(Verdict::Improvement),
            same: with(Verdict::Same),
            new: with(Verdict::New),
            missing: count(missing.len()),
            exit_status: u8::from(!regressed.is_empty()),
            reason,
        };
        Report {
            schema_version: SCHEMA_VERSION,
            provenance,
            base,
            rounds,
            order: order(rounds),
            rustup_toolchain,
            threshold_pct,
            benchmarks,
            missing,
            summary,
        }
    }

    /// Pretty-printed JSON.
    pub fn to_json(&self) -> String {
        // Every key is a string and serde_json writes non-finite floats as
        // null, so serializing a report cannot fail.
        serde_json::to_string_pretty(self).expect("a report always serializes")
    }

    /// Parses a report written by [`Report::to_json`].
    pub fn from_json(text: &str) -> Result<Report, String> {
        let report: Report = serde_json::from_str(text)
            .map_err(|e| format!("not a microbenchmark A/B report: {e}"))?;
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
        super::markdown::render_ab(self)
    }

    /// The regressions and the improvements as tables, then the summary
    /// line: what the run prints once it is over.
    pub fn verdict(&self) -> String {
        super::markdown::verdict_ab(self)
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

/// An A/B run that measured: its report, where the report went, and why the
/// base's checkout could not be removed, when it could not.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub report: Report,
    pub files: ReportFiles,
    pub leftover: Option<String>,
}

/// One A/B run of design D2: refuses, before anything is checked out, a
/// base the checkout has no commit for and a host that is not quiet within
/// `settings.wait`; then prepares the trees, runs the rounds, removes the
/// base's checkout whether or not they completed, compares the sides and
/// writes both reports. Every error is one the runner exits 2 on; the
/// report's summary holds the exit status of a run that measured, 1 on any
/// regression and 0 otherwise.
pub fn run(
    settings: &Settings,
    host: &mut dyn Host,
    probe: &dyn Probe,
    trees: &mut dyn Trees,
) -> Result<Finished, String> {
    let spec = format!("{}^{{commit}}", settings.base);
    let sha = probe
        .output("git", &["rev-parse", "--verify", &spec])
        .filter(|sha| !sha.is_empty())
        .ok_or_else(|| {
            format!(
                "--ab {}: the checkout has no commit by that name; nothing was measured",
                settings.base
            )
        })?;
    let quiet = guard::wait_quiet(host, settings.max_load, settings.wait)?;

    let started = SystemTime::now();
    let out = settings.out.clone().unwrap_or_else(|| {
        settings
            .root
            .join("target/bench/micro-ab")
            .join(compact_timestamp(started))
    });
    // The bench binaries run in their crates' directories, so criterion is
    // given absolute paths, under a directory of the run's own, so that
    // every estimates.json under it is this run's.
    let homes = std::path::absolute(out.join("criterion"))
        .map_err(|e| format!("no absolute path for {}: {e}", out.display()))?;
    fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    fs::create_dir(&homes).map_err(|e| {
        format!(
            "cannot create a fresh {} for criterion's files: {e}",
            homes.display()
        )
    })?;
    let mut machine = capture_host(probe, &settings.os);
    machine.load_average = Some(quiet.load_average);
    let commit = capture_commit(probe);
    let toolchain = probe.output("rustc", &["-V"]);
    let library = fs::read_to_string(settings.root.join("Cargo.lock"))
        .ok()
        .and_then(|lock| criterion::version(&lock));

    let measured = rounds(settings, &sha, &homes, trees);
    let removed = trees.finish();
    let (base_rounds, head_rounds) = match measured {
        Ok(measured) => measured,
        Err(error) => {
            return Err(match removed {
                Ok(()) => error,
                Err(leftover) => format!("{error}; {leftover}"),
            });
        }
    };
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
    let report = Report::new(
        provenance,
        Base {
            name: settings.base.clone(),
            sha,
        },
        settings.rounds,
        settings.toolchain.clone(),
        settings.threshold_pct,
        &base_rounds,
        &head_rounds,
    );
    let files = report
        .write(&out)
        .map_err(|e| format!("cannot write the report into {}: {e}", out.display()))?;
    Ok(Finished {
        report,
        files,
        leftover: removed.err(),
    })
}

/// One side's measurements, one list per round, in round order.
type Rounds = Vec<Vec<Measured>>;

/// Prepares the trees and runs the rounds: base's, then head's.
fn rounds(
    settings: &Settings,
    sha: &str,
    homes: &Path,
    trees: &mut dyn Trees,
) -> Result<(Rounds, Rounds), String> {
    trees.prepare(sha)?;
    let mut base = Vec::new();
    let mut head = Vec::new();
    for (n, sides) in order(settings.rounds).into_iter().enumerate() {
        for side in sides {
            let home = homes.join(format!("round-{}-{side}", n + 1));
            fs::create_dir(&home).map_err(|e| format!("cannot create {}: {e}", home.display()))?;
            let stdout = trees.run(side, &home, settings.filter.as_deref())?;
            let measured = criterion::read(&home, &stdout)?;
            if measured.is_empty() {
                return Err(match &settings.filter {
                    Some(filter) => format!(
                        "{side}'s suites measured no benchmark whose id matches --filter {filter}"
                    ),
                    None => format!("{side}'s suites measured no benchmark"),
                });
            }
            match side {
                Side::Base => base.push(measured),
                Side::Head => head.push(measured),
            }
        }
    }
    Ok((base, head))
}

/// The trees on this host: head is the checkout at `root`; base is checked
/// out with `git worktree add --detach` into `dir`, outside the checkout,
/// since cargo reads `.cargo/config.toml` from every directory above the one
/// it runs in, and built in a target directory of its own. Both build and
/// run with the same toolchain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktrees {
    /// The checkout under test.
    pub root: PathBuf,
    /// Where the base is checked out.
    pub dir: PathBuf,
    /// The suites of each side.
    pub head: CargoBench,
    pub base: CargoBench,
    /// Whether `dir` holds a worktree this run added.
    added: bool,
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
        let mut head = CargoBench::new(&root);
        head.toolchain = toolchain.clone();
        let mut base = CargoBench::new(&dir);
        base.target_dir = Some(target.into());
        base.toolchain = toolchain;
        Worktrees {
            root,
            dir,
            head,
            base,
            added: false,
        }
    }

    /// Runs `git <args>` in the checkout, failing with what git said.
    fn git(&self, args: &[&str]) -> Result<(), String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot start git: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        Err(format!(
            "git {} failed with {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

impl Trees for Worktrees {
    fn prepare(&mut self, sha: &str) -> Result<(), String> {
        let dir = self.dir.display().to_string();
        // A worktree whose directory is gone, as a killed run can leave, is
        // forgotten, and a directory still there from one is removed.
        self.git(&["worktree", "prune"])?;
        if self.dir.exists() {
            let _ = self.git(&["worktree", "remove", "--force", &dir]);
            if self.dir.exists() {
                fs::remove_dir_all(&self.dir)
                    .map_err(|e| format!("cannot remove {dir}, left by an earlier run: {e}"))?;
            }
        }
        self.git(&["worktree", "add", "--detach", &dir, sha])?;
        self.added = true;
        self.head.build()?;
        self.base.build()
    }

    fn run(
        &mut self,
        side: Side,
        criterion_home: &Path,
        filter: Option<&str>,
    ) -> Result<String, String> {
        use super::Suites;
        match side {
            Side::Base => self.base.run(criterion_home, filter),
            Side::Head => self.head.run(criterion_home, filter),
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        if !self.added {
            return Ok(());
        }
        let dir = self.dir.display().to_string();
        self.git(&["worktree", "remove", "--force", &dir])
            .map_err(|e| format!("the base's checkout {dir} was not removed: {e}"))?;
        self.added = false;
        Ok(())
    }
}
