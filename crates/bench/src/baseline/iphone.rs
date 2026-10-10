//! A physical iPhone's processes as CoreDevice lists them, and the XCTest
//! runners that hold its one automation session. `mobile-agent`'s driver,
//! agent-device and WebDriverAgent each keep a runner app on the phone for
//! as long as they drive it, and a runner that starts while another holds
//! the session waits rather than fails, so the tools release theirs before
//! another tool drives the phone, the hand-off ends one a release left
//! (`crate::handoff`), and a baseline that finds another tool's runner on
//! the phone fails at once, naming it.
//!
//! `xcrun devicectl device info processes --device <udid> --json-output -`
//! lists the processes, about 0.3 s a call on an iPhone SE (3rd generation)
//! with iOS 26.6.2, and `xcrun devicectl device process terminate --device
//! <udid> --pid <pid>`, with `--kill` for SIGKILL, ends one in about 0.26 s.

use serde_json::Value;

/// The tools whose XCTest runner holds a physical iPhone's automation
/// session while they drive it, each with the runner's executable.
pub const RUNNERS: [(&str, &str); 3] = [
    ("mobile-agent", "MobileAgentRunner-Runner"),
    ("agent-device", "AgentDeviceRunnerUITests-Runner"),
    (super::wda::NAME, "WebDriverAgentRunner-Runner"),
];

/// The executable of the runner `tool` keeps on a physical iPhone, if it
/// keeps one.
pub fn runner_of(tool: &str) -> Option<&'static str> {
    let _ = tool;
    None
}

/// A process on the iPhone: its pid and its executable's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub executable: String,
}

/// The arguments of `xcrun` that print the processes of the iPhone `udid`
/// as CoreDevice's JSON on standard output.
pub fn processes_args(udid: &str) -> [&str; 8] {
    let _ = udid;
    [""; 8]
}

/// The processes in what [`processes_args`] printed:
/// `result.runningProcesses`, each with its `processIdentifier` and its
/// `executable`, a file URL, of which the last component is the name.
pub fn processes(json: &str) -> Result<Vec<Process>, String> {
    let _ = serde_json::from_str::<Value>(json);
    Ok(Vec::new())
}

/// The arguments of `xcrun` that end the process `pid` on the iPhone `udid`:
/// SIGTERM, or SIGKILL when `kill`.
pub fn terminate_args<'a>(udid: &'a str, pid: &'a str, kill: bool) -> Vec<&'a str> {
    let _ = (udid, pid, kill);
    Vec::new()
}

/// The runners among `processes`, as their tool, executable and pid.
pub fn runners(processes: &[Process]) -> Vec<(&'static str, &'static str, u32)> {
    let _ = processes;
    Vec::new()
}

/// Why `tool` must not start on the iPhone whose processes are
/// `processes`: another tool's runner holds its automation session. `None`
/// when none does.
pub fn held_by_another(processes: &[Process], tool: &str) -> Option<String> {
    let _ = (processes, tool);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `devicectl device info processes --json-output -` printed on the
    /// iPhone SE with WebDriverAgent running, shortened.
    const LISTING: &str = r#"{
  "info" : {"arguments" : ["devicectl", "device", "info", "processes"], "outcome" : "success"},
  "result" : {
    "deviceIdentifier" : "39539691-B024-500A-A5C0-9C00C9E3A93F",
    "runningProcesses" : [
      {"executable" : "file:///sbin/launchd", "processIdentifier" : 1},
      {"executable" : "file:///private/var/containers/Bundle/Application/BAC2/FixtureExpo.app/FixtureExpo", "processIdentifier" : 7419},
      {"executable" : "file:///private/var/containers/Bundle/Application/BC72/WebDriverAgentRunner-Runner.app/WebDriverAgentRunner-Runner", "processIdentifier" : 7920},
      {"processIdentifier" : 7921}
    ]
  }
}"#;

    #[test]
    fn the_phone_s_processes_are_read_from_coredevice_s_listing() {
        assert_eq!(
            processes_args("PHONE-1"),
            [
                "devicectl",
                "device",
                "info",
                "processes",
                "--device",
                "PHONE-1",
                "--json-output",
                "-"
            ]
        );
        assert_eq!(
            processes(LISTING),
            Ok(vec![
                Process {
                    pid: 1,
                    executable: "launchd".into()
                },
                Process {
                    pid: 7419,
                    executable: "FixtureExpo".into()
                },
                Process {
                    pid: 7920,
                    executable: "WebDriverAgentRunner-Runner".into()
                },
            ])
        );
        let error = processes(r#"{"info": {"outcome": "failed"}}"#).unwrap_err();
        assert!(error.contains("runningProcesses"), "{error}");
        assert!(processes("Error: the device is locked").is_err());
    }

    #[test]
    fn a_process_is_ended_with_sigterm_or_with_sigkill() {
        assert_eq!(
            terminate_args("PHONE-1", "7920", false),
            [
                "devicectl",
                "device",
                "process",
                "terminate",
                "--device",
                "PHONE-1",
                "--pid",
                "7920",
                "--json-output",
                "-"
            ]
        );
        assert_eq!(
            terminate_args("PHONE-1", "7920", true),
            [
                "devicectl",
                "device",
                "process",
                "terminate",
                "--device",
                "PHONE-1",
                "--pid",
                "7920",
                "--kill",
                "--json-output",
                "-"
            ]
        );
    }

    #[test]
    fn each_tool_s_runner_is_told_by_its_executable() {
        assert_eq!(runner_of("mobile-agent"), Some("MobileAgentRunner-Runner"));
        assert_eq!(
            runner_of("agent-device"),
            Some("AgentDeviceRunnerUITests-Runner")
        );
        assert_eq!(
            runner_of(super::super::wda::NAME),
            Some("WebDriverAgentRunner-Runner")
        );
        // Maestro's driver ends with the process that runs a flow.
        assert_eq!(runner_of("maestro"), None);
        let found = runners(&processes(LISTING).unwrap());
        assert_eq!(
            found,
            [(super::super::wda::NAME, "WebDriverAgentRunner-Runner", 7920)]
        );
    }

    #[test]
    fn a_tool_finding_another_s_runner_on_the_phone_is_told_whose_it_is() {
        let listing = processes(LISTING).unwrap();
        let held = held_by_another(&listing, "agent-device").unwrap();
        assert_eq!(
            held,
            format!(
                "the iPhone's automation session is held by {}'s runner \
                 WebDriverAgentRunner-Runner (pid 7920); one tool drives a phone at a time",
                super::super::wda::NAME
            )
        );
        // Its own runner holds nothing against it, and a phone with no
        // runner is free.
        assert_eq!(held_by_another(&listing, super::super::wda::NAME), None);
        let free = vec![Process {
            pid: 1,
            executable: "launchd".into(),
        }];
        assert_eq!(held_by_another(&free, "agent-device"), None);
    }
}
