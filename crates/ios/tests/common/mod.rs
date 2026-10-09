//! Helpers shared by the device tests.
//!
//! Every test here drives a physical iPhone, so each one first checks
//! `MOBILE_AGENT_IOS_UDID`. When it is unset, as in CI, the test prints why
//! it skipped and passes. Set it to the identifier of an iPhone attached
//! over USB, trusted, unlocked, with Developer Mode on and the developer
//! disk image mounted, `MOBILE_AGENT_IOS_TEAM` to the team its runner is
//! signed for, and `MOBILE_AGENT_IOS_RUNNER_APP` to the runner bundle
//! `agent-ios/build.sh` built, which an attach installs when the phone's
//! runner is missing or of another version:
//!
//! ```sh
//! MOBILE_AGENT_IOS_UDID=<udid> MOBILE_AGENT_IOS_TEAM=<team> \
//!   MOBILE_AGENT_IOS_RUNNER_APP=agent-ios/dist/MobileAgentRunner-Runner.app cargo test -p ios
//! ```
//!
//! Neither value enters a tracked file, and no test prints them.

#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard};

/// The variable that enables the device tests.
pub const UDID_VARIABLE: &str = "MOBILE_AGENT_IOS_UDID";

/// The variable naming the runner bundle `agent-ios/build.sh` built,
/// `agent-ios/dist/MobileAgentRunner-Runner.app`, which an attach installs
/// in place of a runner that is missing or says hello with another version.
pub const RUNNER_APP_VARIABLE: &str = "MOBILE_AGENT_IOS_RUNNER_APP";

/// The runner built beside the driver: `agent-ios`'s version, with the
/// bundle `MOBILE_AGENT_IOS_RUNNER_APP` names when it is set, a relative path
/// taken from the workspace's root.
pub fn runner() -> ios::RunnerSpec {
    let spec = ios::RunnerSpec::new(agent_ios::VERSION);
    match std::env::var_os(RUNNER_APP_VARIABLE).filter(|path| !path.is_empty()) {
        Some(path) => {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("crates/ios sits two levels below the workspace's root");
            spec.with_bundle(root.join(path))
        }
        None => spec,
    }
}

/// The device to drive, or `None` after printing why the test is skipped.
pub fn gated(test: &str) -> Option<String> {
    match std::env::var(UDID_VARIABLE) {
        Ok(udid) if !udid.trim().is_empty() => Some(udid.trim().to_owned()),
        _ => {
            eprintln!(
                "{test}: skipped; set {UDID_VARIABLE} to the identifier of an iPhone attached \
                 over USB, trusted, unlocked, with Developer Mode on"
            );
            None
        }
    }
}

/// Serializes the tests of one binary that change what the device runs.
/// Cargo runs test binaries one after another, so this covers the rest.
pub fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The host's load averages over 1, 5 and 15 minutes.
pub fn load() -> [f64; 3] {
    let output = std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .expect("sysctl runs");
    let text = String::from_utf8_lossy(&output.stdout);
    let mut values = text
        .trim()
        .trim_matches(|c| c == '{' || c == '}')
        .split_whitespace()
        .filter_map(|value| value.parse::<f64>().ok());
    [(); 3].map(|()| values.next().unwrap_or(f64::NAN))
}

/// The watched processes above 5% of a core, as `name cpu%`: builds and
/// emulators of other sessions that would skew a measurement.
pub fn busy() -> Vec<String> {
    const WATCHED: [&str; 7] = [
        "cargo",
        "rustc",
        "xcodebuild",
        "gradle",
        "kotlinc",
        "java",
        "qemu-system-aarch64",
    ];
    let output = std::process::Command::new("ps")
        .args(["-A", "-o", "%cpu=,comm="])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (cpu, command) = line.split_once(' ')?;
            let cpu: f64 = cpu.trim().parse().ok()?;
            let name = command.trim().rsplit('/').next()?;
            (cpu > 5.0 && WATCHED.iter().any(|watched| name.starts_with(watched)))
                .then(|| format!("{name} {cpu:.0}%"))
        })
        .collect()
}

/// What let a measured series start.
#[derive(Debug)]
pub struct Quiet {
    pub load: [f64; 3],
    pub checks: u32,
    pub waited: std::time::Duration,
    /// The series started under a 1-minute load of 6 after the long wait,
    /// as device-bound series may; its numbers are marked.
    pub relaxed: bool,
}

/// Waits for a quiet host, as the spikes' guard did: a 1-minute load under
/// 4 and no watched process above 5% of a core, checked every 30 s, each
/// check written to standard error. After `relaxed_after`, a device-bound
/// series may start under a load of 6 instead.
pub fn quiet_host(label: &str, relaxed_after: std::time::Duration) -> Quiet {
    let started = std::time::Instant::now();
    let mut checks = 0;
    loop {
        checks += 1;
        let load = load();
        let busy = busy();
        let waited = started.elapsed();
        let relaxed = waited >= relaxed_after;
        let limit = if relaxed { 6.0 } else { 4.0 };
        let quiet = load[0] < limit && busy.is_empty();
        eprintln!(
            "{label}: guard check {checks} after {:.0} s: load {:.2} {:.2} {:.2}, busy {busy:?} -> {}",
            waited.as_secs_f64(),
            load[0],
            load[1],
            load[2],
            if quiet { "start" } else { "wait" }
        );
        if quiet {
            return Quiet {
                load,
                checks,
                waited,
                relaxed,
            };
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}
