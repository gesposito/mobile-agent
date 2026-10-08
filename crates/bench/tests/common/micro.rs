//! Helpers of the microbenchmark tests: criterion's files written as a run
//! writes them, from the ones spike S8 recorded; suites that write them and
//! print the allocation lines instead of running `cargo bench`; a host whose
//! checks are scripted; and fake tools for runs of the binary.
//!
//! `tests/fixtures/micro/s8-home/` holds unchanged copies of spike S8's
//! `spikes/s8-bench-harness/results/quiet/criterion/round-01/decode/` and
//! `decode_slow/`, which `bench.sh` copied out of `$CRITERION_HOME/<id>/new/`
//! after a run, put back where criterion wrote them; `s8-round-01.txt` is
//! that run's output, `round-01.txt` beside them. `list.stdout` is what the
//! suites printed on stdout for `cargo bench --workspace --bench codec
//! --bench mirror --bench daemon --bench frames -- --noplot --list` at this
//! change's base, before the mirror suite had benchmarks.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use bench::micro::Suites;
use bench::micro::criterion::{Measured, Spread};
use bench::micro::guard::{Host, Process, Reading};
use bench::micro::report::{GuardSettings, HarnessSettings, Provenance};
use bench::provenance::{Commit, ScriptedProbe};
use serde_json::Value;

/// What `rustc -V` prints in the checkout.
pub const TOOLCHAIN: &str = "rustc 1.98.1 (48a229cea 2026-09-01)";

/// The commit the scripted probe and the fake git report.
pub const SHA: &str = "d7d0f41c0ffee0000000000000000000000000ab";

/// The directory of the micro fixtures.
pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/micro")
}

/// The text of the fixture `name`.
pub fn fixture(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).unwrap()
}

/// Copies the directory `from` into `to`, creating it.
pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A benchmark as recorded suites write it: criterion's files for its id,
/// and its allocation line when its bench counts allocations.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub id: String,
    pub median_ns: u64,
    pub low_ns: u64,
    pub high_ns: u64,
    pub samples: usize,
    pub allocations: Option<(u64, u64)>,
}

impl Recorded {
    /// A benchmark with its median at `median_ns`, its interval 1% to either
    /// side, 100 samples and no allocations counted.
    pub fn new(id: &str, median_ns: u64) -> Self {
        Recorded {
            id: id.to_owned(),
            median_ns,
            low_ns: median_ns - median_ns / 100,
            high_ns: median_ns + median_ns / 100,
            samples: 100,
            allocations: None,
        }
    }

    /// The same, with `allocations` and `bytes` per iteration.
    pub fn allocating(mut self, allocations: u64, bytes: u64) -> Self {
        self.allocations = Some((allocations, bytes));
        self
    }

    /// The same, with `samples` samples.
    pub fn sampled(mut self, samples: usize) -> Self {
        self.samples = samples;
        self
    }

    /// The same, with its median's interval from `low_ns` to `high_ns`.
    pub fn interval(mut self, low_ns: u64, high_ns: u64) -> Self {
        self.low_ns = low_ns;
        self.high_ns = high_ns;
        self
    }

    /// What the run reads for it.
    pub fn measured(&self) -> Measured {
        Measured {
            id: self.id.clone(),
            median_ns: self.median_ns,
            spread_ns: Some(Spread {
                low: self.low_ns,
                high: self.high_ns,
            }),
            samples: self.samples as u32,
            allocations: self.allocations.map(|(n, _)| n),
            bytes_allocated: self.allocations.map(|(_, bytes)| bytes),
        }
    }

    /// Writes criterion's files for it under `home` as a run does: S8's
    /// `decode` files with its median, interval and sample count in
    /// `new/`, and the copy criterion keeps in `base/`.
    pub fn write(&self, home: &Path) {
        let s8 = fixtures().join("s8-home/decode/new");
        let mut estimates: Value =
            serde_json::from_str(&fs::read_to_string(s8.join("estimates.json")).unwrap()).unwrap();
        let median = &mut estimates["median"];
        median["point_estimate"] = (self.median_ns as f64).into();
        median["confidence_interval"]["lower_bound"] = (self.low_ns as f64).into();
        median["confidence_interval"]["upper_bound"] = (self.high_ns as f64).into();
        let mut sample: Value =
            serde_json::from_str(&fs::read_to_string(s8.join("sample.json")).unwrap()).unwrap();
        for key in ["iters", "times"] {
            let values = sample[key].as_array().unwrap();
            let kept: Vec<Value> = values.iter().cycle().take(self.samples).cloned().collect();
            sample[key] = Value::Array(kept);
        }
        for dir in ["new", "base"] {
            let dir = home.join(&self.id).join(dir);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("estimates.json"), estimates.to_string()).unwrap();
            fs::write(dir.join("sample.json"), sample.to_string()).unwrap();
        }
    }

    /// The line its bench prints before it runs, if it counts allocations.
    pub fn allocation_line(&self) -> Option<String> {
        self.allocations
            .map(|(n, bytes)| format!("allocations {} {n} {bytes}", self.id))
    }
}

/// What recorded suites print on stdout for `benchmarks`: before each, its
/// allocation line if it has one, then criterion's lines for it.
pub fn stdout(benchmarks: &[Recorded]) -> String {
    let mut out = String::new();
    for b in benchmarks {
        if let Some(line) = b.allocation_line() {
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str(&format!(
            "Benchmarking {id}\nBenchmarking {id}: Warming up for 3.0000 s\n\
             Benchmarking {id}: Collecting 100 samples in estimated 5.0000 s\n\
             Benchmarking {id}: Analyzing\n",
            id = b.id
        ));
    }
    out
}

/// Suites that write the recorded benchmarks' files and print what
/// [`stdout`] gives for them, instead of running `cargo bench`.
#[derive(Debug, Default)]
pub struct RecordedSuites {
    pub benchmarks: Vec<Recorded>,
    /// Each run's `CRITERION_HOME` and filter.
    pub runs: Vec<(PathBuf, Option<String>)>,
    /// Fail every run with this message instead.
    pub fail: Option<String>,
}

impl RecordedSuites {
    pub fn new(benchmarks: Vec<Recorded>) -> Self {
        RecordedSuites {
            benchmarks,
            ..RecordedSuites::default()
        }
    }
}

impl Suites for RecordedSuites {
    fn run(&mut self, criterion_home: &Path, filter: Option<&str>) -> Result<String, String> {
        self.runs
            .push((criterion_home.to_owned(), filter.map(str::to_owned)));
        if let Some(message) = &self.fail {
            return Err(message.clone());
        }
        for b in &self.benchmarks {
            b.write(criterion_home);
        }
        Ok(stdout(&self.benchmarks))
    }
}

/// A host whose checks give `readings` in turn, and the last one again once
/// they run out; it records each wait instead of sleeping.
#[derive(Debug)]
pub struct ScriptedHost {
    readings: Vec<Reading>,
    pub checks: usize,
    pub waits: Vec<(Duration, String)>,
}

impl ScriptedHost {
    pub fn new(readings: Vec<Reading>) -> Self {
        ScriptedHost {
            readings,
            checks: 0,
            waits: Vec::new(),
        }
    }
}

impl Host for ScriptedHost {
    fn check(&mut self) -> Reading {
        let reading = self.readings[self.checks.min(self.readings.len() - 1)].clone();
        self.checks += 1;
        reading
    }

    fn wait(&mut self, duration: Duration, reason: &str) {
        self.waits.push((duration, reason.to_owned()));
    }
}

/// A process the guard watches or not.
pub fn process(pid: u32, name: &str, cpu_pct: f64) -> Process {
    Process {
        pid,
        ppid: 1,
        cpu_pct,
        name: name.to_owned(),
    }
}

/// A check at a 1-minute load of `load` that found `busy`.
pub fn reading(load: f64, busy: Vec<Process>) -> Reading {
    Reading {
        load_average: Some([load, 3.5, 4.0]),
        busy: Some(busy),
    }
}

/// A check of a quiet host at a 1-minute load of `load`.
pub fn quiet(load: f64) -> Reading {
    reading(load, Vec::new())
}

/// A checkout for micro to run in: a scratch directory holding a
/// `Cargo.lock` that pins criterion 0.8.2.
pub fn checkout(name: &str) -> PathBuf {
    let root = super::scratch(name);
    fs::write(
        root.join("Cargo.lock"),
        "# This file is automatically @generated by Cargo.\nversion = 4\n\n\
         [[package]]\nname = \"criterion\"\nversion = \"0.8.2\"\n\
         source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
         [[package]]\nname = \"criterion-plot\"\nversion = \"0.8.2\"\n",
    )
    .unwrap();
    root
}

/// The host of [`super::probe`], with the toolchain the checkout pins.
pub fn probe() -> ScriptedProbe {
    super::probe().with("rustc -V", TOOLCHAIN)
}

/// The provenance of a run on the host [`probe`] describes, as a baseline
/// saved earlier records it.
pub fn provenance() -> Provenance {
    Provenance {
        started_at: "2026-10-08T06:00:00Z".to_owned(),
        finished_at: "2026-10-08T06:11:40Z".to_owned(),
        host: bench::provenance::Host {
            hardware: Some("Apple M1 Max".to_owned()),
            cpu_cores: Some(10),
            memory_bytes: Some(68_719_476_736),
            os: Some("macOS 26.6.2 (25G83)".to_owned()),
            load_average: Some([2.89, 3.41, 4.12]),
        },
        load_average_end: Some([3.27, 3.93, 5.49]),
        commit: Some(Commit {
            sha: "c0ffee0000000000000000000000000000000042".to_owned(),
            dirty: false,
        }),
        toolchain: Some(TOOLCHAIN.to_owned()),
        harness: HarnessSettings::criterion(Some("0.8.2".to_owned())),
        guard: GuardSettings {
            max_load: 4.0,
            waited_s: 0,
        },
    }
}

/// Microseconds with three decimals, as the summary prints nanoseconds.
pub fn us(ns: u64) -> String {
    format!("{}.{:03}", ns / 1000, ns % 1000)
}

/// Fake cargo, ps, git, rustc, sysctl and sw_vers for runs of the binary in
/// a checkout of their own. The cargo, which the binary runs as `$CARGO`,
/// records its arguments, its `CRITERION_HOME` and its directory, then
/// copies the recorded benchmarks' files into `CRITERION_HOME` and prints
/// their stdout, or fails when `cargo.status` holds an exit status.
pub struct FakeTools {
    pub dir: PathBuf,
    pub bin: PathBuf,
    pub root: PathBuf,
}

impl FakeTools {
    pub fn new(name: &str, benchmarks: &[Recorded]) -> Self {
        let dir = super::scratch(name);
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let root = checkout(&format!("{name}-checkout"));
        let home = dir.join("home");
        fs::create_dir_all(&home).unwrap();
        for b in benchmarks {
            b.write(&home);
        }
        fs::write(dir.join("stdout.txt"), stdout(benchmarks)).unwrap();
        let tools = FakeTools { dir, bin, root };
        tools.processes("    1     0   0.1 /sbin/launchd\n  500     1  13.3 /Applications/Warp.app/Contents/MacOS/stable\n");
        tools.script(
            "cargo",
            "echo \"$@\" > \"$dir/cargo.args\"\n\
             echo \"$CRITERION_HOME\" > \"$dir/cargo.home\"\n\
             pwd > \"$dir/cargo.cwd\"\n\
             if [ -f \"$dir/cargo.status\" ]; then echo 'error: bench failed' >&2; exit \"$(cat \"$dir/cargo.status\")\"; fi\n\
             cp -R \"$dir/home/.\" \"$CRITERION_HOME/\"\n\
             cat \"$dir/stdout.txt\"\n",
        );
        tools.script("ps", "cat \"$dir/ps.txt\"\n");
        tools.script(
            "git",
            &format!(
                "case \"$*\" in\n\
                 \"rev-parse --show-toplevel\") echo '{}' ;;\n\
                 \"rev-parse HEAD\") echo {SHA} ;;\n\
                 \"status --porcelain --untracked-files=no\") ;;\n\
                 *) exit 1 ;;\n\
                 esac\n",
                tools.root.display()
            ),
        );
        tools.script("rustc", &format!("echo '{TOOLCHAIN}'\n"));
        tools.script(
            "sysctl",
            "case \"$2\" in\n\
             vm.loadavg) echo '{ 1.20 1.50 1.80 }' ;;\n\
             hw.ncpu) echo 10 ;;\n\
             hw.memsize) echo 68719476736 ;;\n\
             machdep.cpu.brand_string) echo 'Apple M1 Max' ;;\n\
             hw.model) echo 'placeholder-model' ;;\n\
             *) exit 1 ;;\n\
             esac\n",
        );
        tools.script(
            "sw_vers",
            "case \"$1\" in\n\
             -productName) echo macOS ;;\n\
             -productVersion) echo 26.6.2 ;;\n\
             -buildVersion) echo 25G83 ;;\n\
             esac\n",
        );
        tools
    }

    /// Writes an executable `name` into the bin directory running `body`,
    /// with `$dir` the tools' directory.
    fn script(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        fs::write(&path, format!("#!/bin/sh\ndir=\"${{0%/*}}/..\"\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// What the fake ps lists from now on.
    pub fn processes(&self, listing: &str) {
        fs::write(self.dir.join("ps.txt"), listing).unwrap();
    }

    /// Makes the fake cargo fail with `status`.
    pub fn fail_cargo(&self, status: i32) {
        fs::write(self.dir.join("cargo.status"), status.to_string()).unwrap();
    }

    /// What the fake cargo recorded about its run, `None` when it never ran:
    /// its arguments, its `CRITERION_HOME` and its directory.
    pub fn cargo_run(&self) -> Option<(String, String, String)> {
        let read = |name: &str| {
            fs::read_to_string(self.dir.join(name))
                .ok()
                .map(|text| text.trim().to_owned())
        };
        Some((read("cargo.args")?, read("cargo.home")?, read("cargo.cwd")?))
    }

    /// `mobile-agent-bench micro <args>` in the checkout, with the fake tools
    /// first on `PATH` and the fake cargo as `$CARGO`.
    pub fn micro(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"));
        command
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", self.bin.display()),
            )
            .env("CARGO", self.bin.join("cargo"))
            .env_remove("CRITERION_HOME")
            .current_dir(&self.root)
            .arg("micro")
            .args(args);
        command
    }
}
