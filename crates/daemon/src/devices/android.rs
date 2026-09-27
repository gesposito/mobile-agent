//! The Android driver (`crates/android`, design D2, D3, D5 and D7) behind
//! the daemon's driver contract.

use android::adb;
use android::agent::DeployError;
use android::session::Status;

use crate::driver::{DriverError, Launch, Size, TargetInfo, TargetReport};

/// The driver's name, as `status`, `doctor` and `--drivers` give it.
pub(crate) const NAME: &str = "android";

/// An adb target as `devices` lists it, in whatever state the adb server
/// reports it, with the advice for a state that keeps it from automation.
pub(crate) fn target_info(target: adb::Target) -> TargetInfo {
    let _ = target;
    todo!("task 8.8")
}

/// The adb server's name for a state: `device`, `offline`, `unauthorized`,
/// `authorizing`, `connecting`, `no permissions`, or the mode it names.
pub(crate) fn state_name(state: &adb::State) -> String {
    let _ = state;
    todo!("task 8.8")
}

/// Refuses a target whose state keeps it from automation, with its advice.
pub(crate) fn ready(target: &TargetInfo) -> Result<(), DriverError> {
    let _ = target;
    todo!("task 8.8")
}

/// The screen in points from `wm size`, the override size first, at
/// `scale` pixels per point.
pub(crate) fn display_size(wm_size: &str, scale: f64) -> Option<Size> {
    let _ = (wm_size, scale);
    todo!("task 8.8")
}

/// The launcher activity `cmd package resolve-activity --brief` printed,
/// as `package/activity`; `None` when it found none.
pub(crate) fn launcher_activity(resolved: &str) -> Option<String> {
    let _ = resolved;
    todo!("task 8.8")
}

/// The `am start` command that launches `component` as the launcher does,
/// waiting for its first frame: stopped first when `launch.relaunch` is
/// set, with each environment entry as a string extra of the same name.
pub(crate) fn start_command(component: &str, launch: &Launch) -> String {
    let _ = (component, launch);
    todo!("task 8.8")
}

/// The error `am start` printed, when it failed.
pub(crate) fn start_failure(output: &str) -> Option<String> {
    let _ = output;
    todo!("task 8.8")
}

/// What `status` shows about an attached Android target: the agent, the
/// active path of each kind of input and output, and what failed since.
pub(crate) fn status_report(status: &Status) -> TargetReport {
    let _ = status;
    todo!("task 8.8")
}

/// A deployment failure with a stable code: another holder of UI
/// automation, the adb server, the agent, or its record.
pub(crate) fn deploy_error(error: DeployError) -> DriverError {
    let _ = error;
    todo!("task 8.8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use android::agent::Conflict;
    use android::session::{Path, Paths};
    use serde_json::Value;

    use crate::driver::{Platform, TargetClass};

    fn adb_target(serial: &str, state: adb::State, emulator: bool) -> adb::Target {
        let online = state == adb::State::Device;
        adb::Target {
            serial: serial.into(),
            state,
            model: online.then(|| "sdk_gphone64_arm64".into()),
            api_level: online.then_some(36),
            emulator,
            transport_id: Some(6),
        }
    }

    /// Android-driver spec, Target discovery: every state is listed, an
    /// unusable one with what to do about it.
    #[test]
    fn adb_targets_are_listed_in_every_state_with_advice_for_the_unusable_ones() {
        let emulator = target_info(adb_target("emulator-5554", adb::State::Device, true));
        assert_eq!(emulator.id, "emulator-5554");
        assert_eq!(emulator.platform, Platform::Android);
        assert_eq!(emulator.class, TargetClass::AndroidEmulator);
        assert_eq!(emulator.name, "sdk_gphone64_arm64");
        assert_eq!(emulator.model.as_deref(), Some("sdk_gphone64_arm64"));
        assert_eq!(emulator.os.as_deref(), Some("Android API 36"));
        assert_eq!(emulator.state, "device");
        assert_eq!(emulator.advice, None);
        assert_eq!(emulator.driver, NAME);
        assert!(ready(&emulator).is_ok());

        let phone = target_info(adb_target("R58M123", adb::State::Unauthorized, false));
        assert_eq!(phone.class, TargetClass::AndroidDevice);
        assert_eq!(
            phone.name, "R58M123",
            "a target without a model shows its serial"
        );
        assert_eq!(phone.state, "unauthorized");
        let advice = phone.advice.clone().expect("advice");
        assert!(advice.contains("Allow USB debugging"), "{advice}");
        let refused = ready(&phone).unwrap_err();
        assert_eq!(refused.code, "target_not_ready");
        assert!(
            refused.message.contains("unauthorized") && refused.message.contains(&advice),
            "{refused:?}"
        );

        assert_eq!(state_name(&adb::State::Offline), "offline");
        assert_eq!(
            state_name(&adb::State::NoPermissions("udev".into())),
            "no permissions"
        );
        assert_eq!(
            state_name(&adb::State::Other("recovery".into())),
            "recovery"
        );
    }

    #[test]
    fn the_display_is_the_override_size_or_the_physical_one_in_points() {
        let size = display_size("Physical size: 1080x2400\n", 2.625).unwrap();
        assert!((size.width - 411.428_571).abs() < 1e-3, "{size:?}");
        assert!((size.height - 914.285_714).abs() < 1e-3, "{size:?}");
        let over =
            display_size("Physical size: 1080x2400\nOverride size: 720x1600\n", 2.0).unwrap();
        assert_eq!((over.width, over.height), (360.0, 800.0));
        assert_eq!(display_size("wm: not found", 2.625), None);
    }

    #[test]
    fn the_launcher_activity_is_what_resolve_activity_names() {
        let resolved = "priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false\n\
                        dev.mobileagent.fixture.compose/.MainActivity\n";
        assert_eq!(
            launcher_activity(resolved).as_deref(),
            Some("dev.mobileagent.fixture.compose/.MainActivity")
        );
        assert_eq!(launcher_activity("No activity found\n"), None);
        assert_eq!(launcher_activity(""), None);
    }

    /// Fixture contract, Launch options: on Android the initial tab is the
    /// string extra `initial_tab`, which `--env initial_tab=big` passes.
    #[test]
    fn a_launch_passes_its_environment_as_quoted_string_extras() {
        let component = "dev.mobileagent.fixture.compose/.MainActivity";
        let launch = Launch {
            relaunch: false,
            env: vec![("initial_tab".into(), "big".into())],
        };
        assert_eq!(
            start_command(component, &launch),
            "am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER \
             -n 'dev.mobileagent.fixture.compose/.MainActivity' --es 'initial_tab' 'big'"
        );
        let relaunch = Launch {
            relaunch: true,
            env: vec![("name".into(), "it's; rm -rf /".into())],
        };
        let command = start_command(component, &relaunch);
        assert!(command.starts_with("am start -W -S "), "{command}");
        assert!(
            command.ends_with("--es 'name' 'it'\\''s; rm -rf /'"),
            "{command}"
        );
    }

    #[test]
    fn a_failed_start_is_told_from_one_that_brought_the_app_to_the_front() {
        let failed = "Starting: Intent { cmp=x/.Main }\n\
                      Error: Activity class {x/x.Main} does not exist.\n";
        assert_eq!(
            start_failure(failed).as_deref(),
            Some("Error: Activity class {x/x.Main} does not exist.")
        );
        let front = "Starting: Intent { cmp=x/.Main }\nWarning: Activity not started, intent has \
                     been delivered to currently running top-most instance.\nStatus: ok\n\
                     LaunchState: UNKNOWN (0)\nComplete\n";
        assert_eq!(start_failure(front), None);
    }

    fn status(paths: Paths) -> Status {
        Status {
            serial: "emulator-5554".into(),
            agent: "mobile-agent-android 0.1.0".into(),
            protocol: 1,
            pid: 812,
            socket: "mobile-agent-0.1.0-p1-00ff".into(),
            reused: false,
            paths,
            inputs: (3, 1),
            frames_received: 40,
            controller_error: None,
            frame_error: None,
            disconnected: None,
        }
    }

    /// Android-driver spec, Emulator fast path: the status names the active
    /// path, in JSON and in the lines.
    #[test]
    fn the_status_names_the_input_path_and_the_agent() {
        let report = status_report(&status(Paths {
            emulator: true,
            input: Path::Controller,
            frames: Some(Path::Controller),
            screenshots: Path::Agent,
            keys: Path::Agent,
            keyboard: Some(false),
            endpoint: Some("http://127.0.0.1:8554".into()),
            warning: None,
        }));
        let data = Value::Object(report.data.clone());
        assert_eq!(data["input"], "emulator controller", "{data}");
        assert_eq!(data["frames"], "emulator controller", "{data}");
        assert_eq!(data["screenshots"], "agent", "{data}");
        assert_eq!(data["keys"], "agent", "{data}");
        assert_eq!(data["endpoint"], "http://127.0.0.1:8554", "{data}");
        assert_eq!(data["agent"], "mobile-agent-android 0.1.0", "{data}");
        assert_eq!(data["pid"], 812, "{data}");
        assert_eq!(data["socket"], "mobile-agent-0.1.0-p1-00ff", "{data}");
        assert_eq!(data["reused"], false, "{data}");
        assert_eq!(data["inputs"]["controller"], 3, "{data}");
        assert_eq!(data["framesReceived"], 40, "{data}");
        assert!(
            report
                .lines
                .iter()
                .any(|line| line == "input: emulator controller at http://127.0.0.1:8554 (taps, long presses and swipes)"),
            "{:?}",
            report.lines
        );

        let fallback = status_report(&status(Paths::agent(true, Some("start with -grpc".into()))));
        let data = Value::Object(fallback.data.clone());
        assert_eq!(data["input"], "agent", "{data}");
        assert_eq!(data["frames"], Value::Null, "{data}");
        assert_eq!(data["warning"], "start with -grpc", "{data}");
    }

    #[test]
    fn deployment_failures_keep_a_code_that_says_whose_they_are() {
        let conflict = deploy_error(DeployError::Conflict(Conflict {
            holder: None,
            detail: "uiautomator events runs".into(),
        }));
        assert_eq!(conflict.code, "automation_conflict");
        assert!(
            conflict.message.contains("uiautomator events"),
            "{conflict:?}"
        );
        let silent = deploy_error(DeployError::NoReadyLine {
            waited: std::time::Duration::from_secs(20),
            output: "nothing".into(),
        });
        assert_eq!(silent.code, "agent_failed");
        let records = deploy_error(DeployError::Records(std::io::Error::other("disk full")));
        assert_eq!(records.code, "io_error");
    }
}
