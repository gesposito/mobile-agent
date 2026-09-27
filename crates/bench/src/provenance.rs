//! What a report records about the environment it ran in: host hardware and
//! OS, Xcode, each target's model, OS and API level, tool versions and the
//! commit under test. Everything is read through a [`Probe`], so tests script
//! the commands' output.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::adapter::Role;
use crate::target::Target;

/// Runs host commands for provenance capture.
pub trait Probe {
    /// Trimmed standard output of `program args`, or `None` when it cannot be
    /// run, fails or times out.
    fn output(&self, program: &str, args: &[&str]) -> Option<String>;
}

/// Runs the commands on this host, each with a 10 s deadline.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProbe;

impl Probe for SystemProbe {
    fn output(&self, program: &str, args: &[&str]) -> Option<String> {
        todo!("run {program} {args:?}")
    }
}

/// Answers from a fixed table keyed by the command line, words joined by single
/// spaces; any other command gives `None`.
#[derive(Clone, Debug, Default)]
pub struct ScriptedProbe {
    outputs: BTreeMap<String, String>,
}

impl ScriptedProbe {
    pub fn new() -> Self {
        ScriptedProbe::default()
    }

    /// Answers `command_line` with `output`.
    pub fn with(mut self, command_line: &str, output: &str) -> Self {
        self.outputs
            .insert(command_line.to_owned(), output.trim().to_owned());
        self
    }
}

impl Probe for ScriptedProbe {
    fn output(&self, program: &str, args: &[&str]) -> Option<String> {
        let line = std::iter::once(program)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        self.outputs.get(&line).cloned()
    }
}

/// Everything a report records about where and how it ran.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// RFC 3339 UTC, second precision.
    pub started_at: String,
    pub finished_at: String,
    pub host: Host,
    /// `xcodebuild -version` as `Xcode 27.0 (27A266a)`; `None` without Xcode.
    pub xcode: Option<String>,
    /// The checkout the harness ran from, which built the CLI under test.
    pub commit: Option<Commit>,
    pub harness: HarnessInfo,
    pub targets: Vec<Target>,
    pub tools: Vec<ToolReport>,
}

/// The host machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Host {
    /// CPU and machine model, such as `Apple M1 Max`.
    pub hardware: Option<String>,
    pub cpu_cores: Option<u32>,
    pub memory_bytes: Option<u64>,
    /// Such as `macOS 26.6.2 (25G83)`.
    pub os: Option<String>,
    /// 1, 5 and 15 minute load averages when the run started; run-specific,
    /// so not part of the environment comparison.
    pub load_average: Option<[f64; 3]>,
}

/// The commit under test.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub sha: String,
    /// Tracked files differ from the commit, or their state is unknown.
    pub dirty: bool,
}

/// The harness build and the run settings statistics depend on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessInfo {
    pub version: String,
    /// Measured iterations per series.
    pub iterations: u32,
    /// Warm-up iterations per series, run first and excluded from statistics.
    pub warmup: u32,
}

/// A tool as the run found it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolReport {
    pub name: String,
    pub role: Role,
    pub status: ToolStatus,
    pub version: Option<String>,
    /// Why the tool was skipped.
    pub note: Option<String>,
}

/// Whether a tool took part in the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    Available,
    Skipped,
}

/// Reads the host's hardware, OS and load through `probe`; `os` is the value
/// of `std::env::consts::OS` for the host.
pub fn capture_host(probe: &dyn Probe, os: &str) -> Host {
    todo!("capture the {os} host through {:p}", probe)
}

/// `xcodebuild -version` condensed to one line.
pub fn capture_xcode(probe: &dyn Probe) -> Option<String> {
    todo!("capture Xcode through {:p}", probe)
}

/// The checkout's HEAD and whether tracked files differ from it.
pub fn capture_commit(probe: &dyn Probe) -> Option<Commit> {
    todo!("capture the commit through {:p}", probe)
}

/// Fills the target fields an adapter left empty: from `adb -s <id> shell
/// getprop` on Android, from `xcrun simctl list devices --json` on the
/// Simulator.
pub fn complete_target(probe: &dyn Probe, target: &mut Target) {
    todo!("complete {} through {:p}", target.id, probe)
}

impl Provenance {
    /// The environment fields in which `self` and `other` differ, as dotted
    /// paths such as `host.os` or `targets[emulator-5554].api_level`. Times and
    /// load averages are run-specific and never listed. An empty list means
    /// the environments matched.
    pub fn environment_differences(&self, other: &Provenance) -> Vec<String> {
        todo!("compare {} with {}", self.started_at, other.started_at)
    }
}

/// RFC 3339 UTC timestamp with second precision.
pub fn utc_timestamp(time: SystemTime) -> String {
    todo!("format {time:?}")
}

/// The same instant as a compact `YYYYMMDDTHHMMSSZ` stamp for directory names.
pub fn compact_timestamp(time: SystemTime) -> String {
    todo!("format {time:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::TargetClass;
    use std::time::{Duration, UNIX_EPOCH};

    fn mac() -> ScriptedProbe {
        ScriptedProbe::new()
            .with("sysctl -n machdep.cpu.brand_string", "Apple M1 Max")
            .with("sysctl -n hw.model", "placeholder-model")
            .with("sysctl -n hw.ncpu", "10")
            .with("sysctl -n hw.memsize", "68719476736")
            .with("sysctl -n vm.loadavg", "{ 3.85 4.10 5.00 }")
            .with("sw_vers -productName", "macOS")
            .with("sw_vers -productVersion", "26.6.2")
            .with("sw_vers -buildVersion", "25G83")
    }

    #[test]
    fn a_mac_host_is_read_from_sysctl_and_sw_vers() {
        let host = capture_host(&mac(), "macos");
        assert_eq!(
            host,
            Host {
                hardware: Some("Apple M1 Max".into()),
                cpu_cores: Some(10),
                memory_bytes: Some(68_719_476_736),
                os: Some("macOS 26.6.2 (25G83)".into()),
                load_average: Some([3.85, 4.1, 5.0]),
            }
        );
    }

    #[test]
    fn a_linux_host_is_read_from_uname_and_proc() {
        let probe = ScriptedProbe::new()
            .with(
                "grep -m1 model name /proc/cpuinfo",
                "model name\t: AMD EPYC 7763 64-Core Processor",
            )
            .with("nproc", "4")
            .with(
                "grep -m1 MemTotal /proc/meminfo",
                "MemTotal:       16318504 kB",
            )
            .with("uname -sr", "Linux 6.8.0-1017-azure")
            .with("cat /proc/loadavg", "0.52 0.58 0.59 1/1234 5678");
        let host = capture_host(&probe, "linux");
        assert_eq!(
            host,
            Host {
                hardware: Some("AMD EPYC 7763 64-Core Processor".into()),
                cpu_cores: Some(4),
                memory_bytes: Some(16_318_504 * 1024),
                os: Some("Linux 6.8.0-1017-azure".into()),
                load_average: Some([0.52, 0.58, 0.59]),
            }
        );
    }

    #[test]
    fn missing_commands_leave_fields_empty() {
        let host = capture_host(&ScriptedProbe::new(), "macos");
        assert_eq!(host.hardware, None);
        assert_eq!(host.os, None);
        assert_eq!(host.load_average, None);
        assert_eq!(capture_xcode(&ScriptedProbe::new()), None);
        assert_eq!(capture_commit(&ScriptedProbe::new()), None);
    }

    #[test]
    fn xcode_and_the_commit_are_condensed() {
        let probe = ScriptedProbe::new()
            .with("xcodebuild -version", "Xcode 27.0\nBuild version 27A266a")
            .with(
                "git rev-parse HEAD",
                "d7d0f41c0ffee0000000000000000000000000ab",
            )
            .with("git status --porcelain --untracked-files=no", "");
        assert_eq!(
            capture_xcode(&probe).as_deref(),
            Some("Xcode 27.0 (27A266a)")
        );
        assert_eq!(
            capture_commit(&probe),
            Some(Commit {
                sha: "d7d0f41c0ffee0000000000000000000000000ab".into(),
                dirty: false,
            })
        );
        let dirty = probe.with(
            "git status --porcelain --untracked-files=no",
            " M crates/bench/src/lib.rs",
        );
        assert_eq!(capture_commit(&dirty).map(|c| c.dirty), Some(true));
    }

    #[test]
    fn an_emulator_is_completed_from_getprop() {
        let probe = ScriptedProbe::new().with(
            "adb -s emulator-5554 shell getprop",
            "[ro.boot.qemu.avd_name]: [pixel_api36]\n\
             [ro.build.version.release]: [16]\n\
             [ro.build.version.sdk]: [36]\n\
             [ro.product.model]: [sdk_gphone64_arm64]",
        );
        let mut target = Target::new("emulator-5554", TargetClass::AndroidEmulator);
        complete_target(&probe, &mut target);
        assert_eq!(
            target.model.as_deref(),
            Some("sdk_gphone64_arm64 (AVD pixel_api36)")
        );
        assert_eq!(target.os.as_deref(), Some("Android 16"));
        assert_eq!(target.api_level, Some(36));
    }

    #[test]
    fn a_simulator_is_completed_from_simctl() {
        let probe = ScriptedProbe::new().with(
            "xcrun simctl list devices --json",
            r#"{"devices":{
                "com.apple.CoreSimulator.SimRuntime.iOS-26-5":[
                    {"udid":"OTHER","name":"x","deviceTypeIdentifier":"com.apple.CoreSimulator.SimDeviceType.iPhone-16e"},
                    {"udid":"SIM-1","name":"bench","deviceTypeIdentifier":"com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro-Max"}
                ]}}"#,
        );
        let mut target = Target::new("SIM-1", TargetClass::Simulator);
        complete_target(&probe, &mut target);
        assert_eq!(target.model.as_deref(), Some("iPhone 17 Pro Max"));
        assert_eq!(target.os.as_deref(), Some("iOS 26.5"));
        assert_eq!(target.api_level, None);
    }

    #[test]
    fn fields_an_adapter_filled_are_kept() {
        let probe = ScriptedProbe::new().with(
            "adb -s R5CT shell getprop",
            "[ro.build.version.sdk]: [35]\n[ro.product.model]: [SM-S921B]",
        );
        let mut target = Target::new("R5CT", TargetClass::AndroidDevice);
        target.model = Some("Galaxy S24".into());
        complete_target(&probe, &mut target);
        assert_eq!(target.model.as_deref(), Some("Galaxy S24"));
        assert_eq!(target.api_level, Some(35));
    }

    #[test]
    fn timestamps_are_utc() {
        let at = |secs| UNIX_EPOCH + Duration::from_secs(secs);
        assert_eq!(utc_timestamp(at(0)), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(at(951_782_400)), "2000-02-29T00:00:00Z");
        assert_eq!(utc_timestamp(at(1_700_000_000)), "2023-11-14T22:13:20Z");
        assert_eq!(utc_timestamp(at(1_790_467_200)), "2026-09-27T00:00:00Z");
        assert_eq!(compact_timestamp(at(1_700_000_000)), "20231114T221320Z");
    }
}
