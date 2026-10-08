//! The quiet-host guard of design D15: a run measures only while the 1-minute
//! load average is under a bound and no build, test, emulator or Simulator
//! process uses more than 5% of a core, as spike S8's `bench.sh` checked by
//! hand. Otherwise it refuses, or checks again every 30 s up to a time limit
//! and refuses at the limit; it never measures a busy host. The guard reads
//! the host through a [`Host`], so tests script the readings and the waits.

use std::time::Duration;

use crate::provenance::Probe;

/// How long a waiting run lets pass between two checks.
pub const INTERVAL: Duration = Duration::from_secs(30);

/// A watched process above this share of one core, in percent, is busy.
pub const BUSY_CPU_PCT: f64 = 5.0;

/// The builders and devices the reports waited for, matched as whole names,
/// as spike S8's `bench.sh` matched them.
pub const NAMES: [&str; 5] = ["cargo", "rustc", "xcodebuild", "java", "Simulator"];

/// The same, matched as name prefixes, for the families whose executables
/// carry a suffix, such as `qemu-system-aarch64-headless` or
/// `com.apple.CoreSimulator.CoreSimulatorService`.
pub const PREFIXES: [&str; 5] = [
    "gradle",
    "kotlinc",
    "qemu-system",
    "emulator",
    "com.apple.CoreSimulator",
];

/// One process of the host's process list.
#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    /// Its share of one core in percent, as `ps` reports it.
    pub cpu_pct: f64,
    /// Its executable's name, without the directory.
    pub name: String,
}

/// What one check of the host found.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// The 1, 5 and 15 minute load averages; `None` when they could not be
    /// read.
    pub load_average: Option<[f64; 3]>,
    /// The watched processes above [`BUSY_CPU_PCT`], outside the runner's
    /// own process tree; `None` when the process list could not be read.
    pub busy: Option<Vec<Process>>,
}

/// The host the guard checks.
pub trait Host {
    /// Checks the host once: its load averages and its busy processes.
    fn check(&mut self) -> Reading;

    /// Lets `duration` pass before the next check, the host not being quiet
    /// for `reason`.
    fn wait(&mut self, duration: Duration, reason: &str);
}

/// This host, read through a [`Probe`]: the load averages from
/// `sysctl -n vm.loadavg` on macOS and `/proc/loadavg` elsewhere, the
/// processes from `ps -A -o pid=,ppid=,pcpu=,comm=`. A wait prints its
/// reason on stderr and sleeps.
pub struct SystemHost {
    probe: Box<dyn Probe>,
    os: String,
    pid: u32,
}

impl SystemHost {
    /// The host `probe` reads, running `os` (a value of
    /// `std::env::consts::OS`), for the runner whose process id is `pid`.
    pub fn new(probe: Box<dyn Probe>, os: &str, pid: u32) -> Self {
        SystemHost {
            probe,
            os: os.to_owned(),
            pid,
        }
    }
}

impl Host for SystemHost {
    fn check(&mut self) -> Reading {
        read(self.probe.as_ref(), &self.os, self.pid)
    }

    fn wait(&mut self, duration: Duration, reason: &str) {
        let _ = (duration, reason);
        todo!()
    }
}

/// One check through `probe` on a host running `os`, for the runner whose
/// process id is `own`.
pub fn read(probe: &dyn Probe, os: &str, own: u32) -> Reading {
    let _ = (probe, os, own);
    todo!()
}

/// The processes of a `ps -A -o pid=,ppid=,pcpu=,comm=` listing. macOS
/// prints the executable's path, which may hold spaces, as the last column;
/// the name is what follows its last `/`. Lines that do not read are left
/// out.
pub fn processes(listing: &str) -> Vec<Process> {
    let _ = listing;
    todo!()
}

/// Whether a process named `name` is one the guard watches: one of [`NAMES`],
/// or a name starting with one of [`PREFIXES`].
pub fn watched(name: &str) -> bool {
    let _ = name;
    todo!()
}

/// The watched processes of `processes` above [`BUSY_CPU_PCT`], leaving out
/// the runner's own process tree: the process `own` and every process it
/// started, directly or not.
pub fn busy(processes: &[Process], own: u32) -> Vec<Process> {
    let _ = (processes, own);
    todo!()
}

/// Why `reading` is not quiet under `max_load`, naming the 1-minute load and
/// the busy processes; `None` when it is quiet. A load or a process list
/// that could not be read is not quiet.
pub fn noise(reading: &Reading, max_load: f64) -> Option<String> {
    let _ = (reading, max_load);
    todo!()
}

/// The check that found the host quiet.
#[derive(Clone, Debug, PartialEq)]
pub struct Quiet {
    /// The load averages at that check, the run's start.
    pub load_average: [f64; 3],
    /// How long the run waited for it.
    pub waited: Duration,
}

/// Checks `host` until it is quiet under `max_load`, letting [`INTERVAL`]
/// pass between checks for `limit` in all. A zero `limit` checks once. Once
/// `limit` has passed it refuses, with what the last check found.
pub fn wait_quiet(host: &mut dyn Host, max_load: f64, limit: Duration) -> Result<Quiet, String> {
    let _ = (host, max_load, limit);
    todo!()
}
