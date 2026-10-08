//! The quiet-host guard of design D15: a run measures only while the 1-minute
//! load average is under a bound and no build, test, emulator or Simulator
//! process uses more than 5% of a core, as spike S8's `bench.sh` checked by
//! hand. Otherwise it refuses, or checks again every 30 s up to a time limit
//! and refuses at the limit; it never measures a busy host. The guard reads
//! the host through a [`Host`], so tests script the readings and the waits.

use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::Duration;

use crate::provenance::{Probe, capture_load};

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
        eprintln!(
            "mobile-agent-bench: the host is not quiet: {reason}; checking again in {} s",
            duration.as_secs()
        );
        thread::sleep(duration);
    }
}

/// One check through `probe` on a host running `os`, for the runner whose
/// process id is `own`.
pub fn read(probe: &dyn Probe, os: &str, own: u32) -> Reading {
    Reading {
        load_average: capture_load(probe, os),
        busy: probe
            .output("ps", &["-A", "-o", "pid=,ppid=,pcpu=,comm="])
            .map(|listing| busy(&processes(&listing), own)),
    }
}

/// The processes of a `ps -A -o pid=,ppid=,pcpu=,comm=` listing. macOS
/// prints the executable's path, which may hold spaces, as the last column;
/// the name is what follows its last `/`. Lines that do not read are left
/// out.
pub fn processes(listing: &str) -> Vec<Process> {
    listing.lines().filter_map(process).collect()
}

/// One line of the listing: the pid, the parent's pid and the CPU share,
/// then the command.
fn process(line: &str) -> Option<Process> {
    let mut rest = line.trim_start();
    let mut field = || {
        let (word, after) = rest.split_once(char::is_whitespace)?;
        rest = after.trim_start();
        Some(word)
    };
    let pid = field()?.parse().ok()?;
    let ppid = field()?.parse().ok()?;
    let cpu_pct = field()?.parse().ok()?;
    let name = rest.trim_end().rsplit('/').next()?;
    (!name.is_empty()).then(|| Process {
        pid,
        ppid,
        cpu_pct,
        name: name.to_owned(),
    })
}

/// Whether a process named `name` is one the guard watches: one of [`NAMES`],
/// or a name starting with one of [`PREFIXES`].
pub fn watched(name: &str) -> bool {
    NAMES.contains(&name) || PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// The watched processes of `processes` above [`BUSY_CPU_PCT`], leaving out
/// the runner's own process tree: the process `own` and every process it
/// started, directly or not.
pub fn busy(processes: &[Process], own: u32) -> Vec<Process> {
    let parents: HashMap<u32, u32> = processes.iter().map(|p| (p.pid, p.ppid)).collect();
    // Up the parents from `pid`; a pid met twice, as launchd's parent 0 or a
    // listing taken while processes came and went, ends the walk.
    let ours = |pid: u32| {
        let mut seen = HashSet::new();
        let mut at = pid;
        while seen.insert(at) {
            if at == own {
                return true;
            }
            match parents.get(&at) {
                Some(&parent) => at = parent,
                None => return false,
            }
        }
        false
    };
    processes
        .iter()
        .filter(|p| p.cpu_pct > BUSY_CPU_PCT && watched(&p.name) && !ours(p.pid))
        .cloned()
        .collect()
}

/// Why `reading` is not quiet under `max_load`, naming the 1-minute load and
/// the busy processes; `None` when it is quiet. A load or a process list
/// that could not be read is not quiet.
pub fn noise(reading: &Reading, max_load: f64) -> Option<String> {
    judge(reading, max_load).err()
}

/// The load averages of `reading` when it is quiet under `max_load`, else
/// why it is not.
fn judge(reading: &Reading, max_load: f64) -> Result<[f64; 3], String> {
    let (load, quiet_load) = match reading.load_average {
        None => ("the load average cannot be read".to_owned(), None),
        Some(load @ [one, ..]) if one < max_load => (
            format!("the 1-minute load average is {one:.2}, under {max_load}"),
            Some(load),
        ),
        Some([one, ..]) => (
            format!("the 1-minute load average is {one:.2}, not under {max_load}"),
            None,
        ),
    };
    let (processes, quiet_processes) = match &reading.busy {
        None => ("the process list cannot be read".to_owned(), false),
        Some(busy) if busy.is_empty() => (
            "no build, test, emulator or Simulator process is busy".to_owned(),
            true,
        ),
        Some(busy) => {
            let listed: Vec<String> = busy
                .iter()
                .map(|p| format!("{} (pid {}, {:.1}% of a core)", p.name, p.pid, p.cpu_pct))
                .collect();
            (format!("busy: {}", listed.join(", ")), false)
        }
    };
    match quiet_load {
        Some(load_average) if quiet_processes => Ok(load_average),
        _ => Err(format!("{load}; {processes}")),
    }
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
    let mut waited = Duration::ZERO;
    loop {
        let reason = match judge(&host.check(), max_load) {
            Ok(load_average) => {
                return Ok(Quiet {
                    load_average,
                    waited,
                });
            }
            Err(reason) => reason,
        };
        if waited >= limit {
            return Err(if limit.is_zero() {
                format!("the host is not quiet: {reason}; nothing was measured")
            } else {
                format!(
                    "the host is still not quiet after waiting {}: {reason}; nothing was measured",
                    minutes(limit)
                )
            });
        }
        host.wait(INTERVAL, &reason);
        waited += INTERVAL;
    }
}

/// Whole minutes of `limit`, such as `1 minute` or `60 minutes`.
fn minutes(limit: Duration) -> String {
    match limit.as_secs() / 60 {
        1 => "1 minute".to_owned(),
        n => format!("{n} minutes"),
    }
}
