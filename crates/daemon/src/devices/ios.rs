//! The physical iOS driver (`crates/ios`, design D18 to D25) behind the
//! daemon's driver contract and the mirror's backend traits, as
//! `crates/ios/README.md`'s Driver contract maps them.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use ios::{CallSite, DeviceInfo, Dispatch, InputChoice, TargetStatus};
use serde_json::Value;

use crate::driver::{
    AttachOptions, Attachment, CompatibilityRow, Driver, DriverError, TargetInfo, TargetReport,
};

/// The driver's name, as `status`, `doctor` and `--drivers` give it.
pub(crate) const NAME: &str = "ios";

/// The variable that names the runner bundle an attach installs.
pub(crate) const RUNNER_APP_VAR: &str = "MOBILE_AGENT_IOS_RUNNER_APP";

/// The variable that picks the way input goes.
pub(crate) const INPUT_VAR: &str = "MOBILE_AGENT_IOS_INPUT";

/// Where `agent-ios/build.sh` writes the runner bundle, from the workspace's
/// root.
pub(crate) const RUNNER_BUNDLE: &str = "agent-ios/dist/MobileAgentRunner-Runner.app";

/// The iPhones attached over USB.
pub(crate) struct IosDevices;

impl IosDevices {
    pub(crate) fn new() -> IosDevices {
        IosDevices
    }
}

impl Driver for IosDevices {
    fn name(&self) -> &str {
        NAME
    }

    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError> {
        unimplemented!()
    }

    fn attach(
        &self,
        _target: &TargetInfo,
        _options: &AttachOptions,
    ) -> Result<Attachment, DriverError> {
        unimplemented!()
    }
}

/// What `status` reports beside the driver's own status.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Facts {
    pub ios_version: Option<String>,
    pub input: InputChoice,
    pub last_input: Option<Dispatch>,
    pub frames: u64,
    pub capture_rate: Option<f64>,
    pub frame_error: Option<String>,
    pub bundle: Option<PathBuf>,
}

/// An iPhone as `devices` lists it.
pub(crate) fn target_info(_device: DeviceInfo) -> TargetInfo {
    unimplemented!()
}

/// The runner bundle an attach installs.
pub(crate) fn runner_bundle(_variable: Option<&OsStr>) -> PathBuf {
    unimplemented!()
}

/// What `doctor` says of the runner bundle.
pub(crate) fn runner_doctor(_bundle: &Path) -> Value {
    unimplemented!()
}

/// The way input goes.
pub(crate) fn input_choice(_variable: Option<&str>) -> Result<InputChoice, DriverError> {
    unimplemented!()
}

/// A driver error as the protocol carries it.
pub(crate) fn error(_error: &ios::Error) -> DriverError {
    unimplemented!()
}

/// A launch or stop's error.
pub(crate) fn app_error(_error: &ios::Error) -> DriverError {
    unimplemented!()
}

/// What an iOS key code names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IosKey {
    Key(ios::Key),
    Button(ios::Button),
}

/// The key or button `code` names.
pub(crate) fn ios_key(_code: u32, _meta: Option<u32>) -> Result<IosKey, DriverError> {
    unimplemented!()
}

/// The compatibility table as `doctor`'s rows.
pub(crate) fn doctor_rows(_sites: &[CallSite]) -> Vec<CompatibilityRow> {
    unimplemented!()
}

/// What `status` shows about an attached iPhone.
pub(crate) fn status_report(_status: &TargetStatus, _facts: &Facts) -> TargetReport {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use ios::{
        DiskImage, InputPath, Modifiers, MountReport, Probe, Readiness, RunnerState, RunnerStatus,
        Trust, TunnelStatus,
    };
    use serde_json::json;

    use crate::driver::{Platform, TargetClass};

    const PHONE: &str = "placeholder-udid-phone";

    fn device(readiness: Readiness) -> DeviceInfo {
        DeviceInfo {
            udid: PHONE.into(),
            name: Some("placeholder-iphone".into()),
            model: Some("iPhone14,6".into()),
            ios_version: Some("26.6.2".into()),
            build: Some("23G90".into()),
            probe: Probe {
                trust: Trust::Trusted,
                developer_mode: Some(true),
                disk_image: Some(DiskImage::Mounted {
                    image_type: "Personalized".into(),
                    mount_path: Some("/System/Developer".into()),
                }),
                password_protected: Some(false),
            },
            readiness,
        }
    }

    /// Task 5.1, spec `ios-device-driver` "Ready device listed": an iPhone
    /// is listed by its identifier in the `ios-device` class, with its name,
    /// model and iOS version, ready.
    #[test]
    fn a_ready_iphone_is_listed_with_its_name_model_and_ios_version() {
        let target = target_info(device(Readiness::Ready));
        assert_eq!(target.id, PHONE);
        assert_eq!(target.platform, Platform::Ios);
        assert_eq!(target.class, TargetClass::IosDevice);
        assert_eq!(target.class.id(), "ios-device");
        assert_eq!(target.name, "placeholder-iphone");
        assert_eq!(target.model.as_deref(), Some("iPhone14,6"));
        assert_eq!(target.os.as_deref(), Some("iOS 26.6.2"));
        assert_eq!(target.state, "ready");
        assert_eq!(target.advice, None);
        assert_eq!(target.driver, NAME);
    }

    /// Task 5.1, spec `ios-device-driver` "Device not ready": an unready
    /// iPhone is listed in its readiness state, the state's message from
    /// `crates/ios` its advice; one lockdown described nothing of goes by
    /// its identifier.
    #[test]
    fn an_unready_iphone_is_listed_in_its_state_with_the_step_as_advice() {
        for readiness in [
            Readiness::NotTrusted {
                reason: "usbmuxd holds no pairing record for it".into(),
            },
            Readiness::DeveloperModeOff,
            Readiness::DiskImageUnmounted,
            Readiness::Locked,
            Readiness::NoSigningIdentity {
                reason: "MOBILE_AGENT_IOS_TEAM is not set".into(),
            },
        ] {
            let target = target_info(device(readiness.clone()));
            assert_eq!(target.state, readiness.code());
            assert_eq!(target.advice.as_deref(), Some(readiness.message().as_str()));
        }
        let bare = DeviceInfo {
            name: None,
            model: None,
            ios_version: None,
            ..device(Readiness::NotTrusted {
                reason: "lockdown did not answer".into(),
            })
        };
        let target = target_info(bare);
        assert_eq!(target.name, PHONE);
        assert_eq!(target.os, None);
    }

    fn status() -> TargetStatus {
        TargetStatus {
            tunnel: Some(TunnelStatus {
                opens: 1,
                opened_in: Duration::from_micros(77_000),
                services: (0..82)
                    .map(|n| format!("placeholder.service.{n}"))
                    .collect(),
                missing: Vec::new(),
            }),
            disk_image: Some(MountReport {
                image: DiskImage::Mounted {
                    image_type: "Personalized".into(),
                    mount_path: Some("/System/Developer".into()),
                },
                attempts: Vec::new(),
            }),
            services_open: vec!["com.apple.instruments.server.services.processcontrol"],
            runner: Some(RunnerStatus {
                state: RunnerState::Ready,
                agent: Some("mobile-agent-ios-runner 0.2.0".into()),
                capabilities: protocol::Capabilities::TREE
                    | protocol::Capabilities::INPUT
                    | protocol::Capabilities::SCREENSHOT
                    | protocol::Capabilities::TEXT_SET,
                restarts: 1,
                handshake_retries: 0,
                replaced: false,
                leftovers: vec![412],
                started_in: Some(Duration::from_millis(812)),
                last_death: Some("the runner's connection closed".into()),
            }),
        }
    }

    fn facts() -> Facts {
        Facts {
            ios_version: Some("26.6.2".into()),
            input: InputChoice::Auto,
            last_input: None,
            frames: 34,
            capture_rate: Some(19.1),
            frame_error: None,
            bundle: Some(PathBuf::from(
                "/placeholder/agent-ios/dist/MobileAgentRunner-Runner.app",
            )),
        }
    }

    /// Task 5.1, spec `ios-device-driver` "Runner stays up": the status
    /// names the input path, the frame path with its capture rate, and the
    /// runner's agent, restarts and the leftovers its start ended.
    #[test]
    fn the_status_names_the_input_path_the_frames_with_their_rate_and_the_runner() {
        let report = status_report(&status(), &facts());
        let data = Value::Object(report.data.clone());
        assert_eq!(data["input"], "dtuhidd", "{data}");
        assert_eq!(data["frames"], "screenshots", "{data}");
        assert_eq!(data["captureRate"], 19.1, "{data}");
        assert_eq!(data["framesReceived"], 34, "{data}");
        assert_eq!(
            data["runner"]["agent"], "mobile-agent-ios-runner 0.2.0",
            "{data}"
        );
        assert_eq!(data["runner"]["state"], "ready", "{data}");
        assert_eq!(data["runner"]["restarts"], 1, "{data}");
        assert_eq!(data["runner"]["leftovers"], json!([412]), "{data}");
        assert_eq!(data["runner"]["startedMs"], 812.0, "{data}");
        assert_eq!(data["tunnel"]["services"], 82, "{data}");
        let text = report.lines.join("\n");
        for part in [
            "input: dtuhidd",
            "frames: screenshots",
            "19.1 captures a second",
            "34 received",
            "runner: mobile-agent-ios-runner 0.2.0",
            "1 restart",
            "412",
        ] {
            assert!(text.contains(part), "{part:?} is missing from:\n{text}");
        }

        // The runner's synthesis, asked for, and the input it took.
        let runner = Facts {
            input: InputChoice::Only(InputPath::Runner),
            last_input: Some(Dispatch {
                path: InputPath::Runner,
                reports: 1,
                first: Duration::from_micros(290_000),
                acknowledged: Duration::from_micros(292_200),
            }),
            ..facts()
        };
        let report = status_report(&status(), &runner);
        let data = Value::Object(report.data.clone());
        assert_eq!(data["input"], "runner", "{data}");
        assert_eq!(data["lastInputMs"], 292.2, "{data}");
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.starts_with("input: runner")),
            "{:?}",
            report.lines
        );

        // No frames: why.
        let blind = Facts {
            frames: 0,
            capture_rate: None,
            frame_error: Some("DVT's takeScreenshot failed".into()),
            ..facts()
        };
        let report = status_report(&status(), &blind);
        let data = Value::Object(report.data.clone());
        assert_eq!(data["frames"], Value::Null, "{data}");
        assert_eq!(data["frameError"], "DVT's takeScreenshot failed", "{data}");
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.starts_with("frames: none") && line.contains("takeScreenshot")),
            "{:?}",
            report.lines
        );
    }

    /// Task 5.1, spec `ios-device-driver` "Table lists the device paths":
    /// the supported range, then each call site with the iOS and Xcode it
    /// was validated on, in the shape the Simulator driver's rows take.
    #[test]
    fn the_compatibility_rows_are_the_supported_range_then_every_call_site() {
        let sites = ios::call_sites();
        let rows = doctor_rows(&sites);
        assert_eq!(rows.len(), sites.len() + 1);
        let range = &rows[0];
        assert!(
            range.subject.contains("iOS 26.6.2") && range.subject.contains("Xcode 27.0"),
            "{range:?}"
        );
        assert!(
            range.feature.contains("17.4") && range.feature.contains("27"),
            "{range:?}"
        );
        assert_eq!(range.status, "supported");
        for (row, site) in rows[1..].iter().zip(&sites) {
            assert_eq!(row.subject, site.validated_on);
            assert!(
                row.feature.contains(site.call) && row.feature.contains(site.service),
                "{row:?}"
            );
            let status = if site.validated_on.contains("not exercised") {
                "supported"
            } else {
                "validated"
            };
            assert_eq!(row.status, status, "{row:?}");
        }
    }

    /// Task 5.1: the runner bundle is the one `agent-ios/build.sh` writes in
    /// this workspace, unless `MOBILE_AGENT_IOS_RUNNER_APP` names another, a
    /// relative path taken from the workspace's root, as the device tests of
    /// `crates/ios` take it.
    #[test]
    fn the_runner_bundle_is_the_built_one_or_the_one_the_variable_names() {
        let built = runner_bundle(None);
        assert!(built.is_absolute(), "{}", built.display());
        assert!(
            built.ends_with("agent-ios/dist/MobileAgentRunner-Runner.app"),
            "{}",
            built.display()
        );
        assert_eq!(runner_bundle(Some(OsStr::new(""))), built);
        let dist = built.parent().expect("the bundle sits in dist");
        assert_eq!(
            runner_bundle(Some(OsStr::new("agent-ios/dist/Other.app"))),
            dist.join("Other.app")
        );
        assert_eq!(
            runner_bundle(Some(OsStr::new("/placeholder/Runner.app"))),
            PathBuf::from("/placeholder/Runner.app")
        );
        let doctor = runner_doctor(&built);
        assert_eq!(doctor["bundle"], built.to_string_lossy().as_ref());
        assert_eq!(doctor["version"], agent_ios::VERSION);
    }

    /// Task 5.1: `MOBILE_AGENT_IOS_INPUT` picks the way input goes: `auto`,
    /// the default, by the device's iOS; `dtuhidd` or `runner` alone.
    #[test]
    fn the_input_variable_takes_auto_dtuhidd_or_runner() {
        for (value, choice) in [
            (None, InputChoice::Auto),
            (Some(""), InputChoice::Auto),
            (Some("auto"), InputChoice::Auto),
            (Some(" Runner "), InputChoice::Only(InputPath::Runner)),
            (Some("dtuhidd"), InputChoice::Only(InputPath::Hid)),
        ] {
            assert_eq!(input_choice(value), Ok(choice), "{value:?}");
        }
        let refused = input_choice(Some("bluetooth")).unwrap_err();
        assert_eq!(refused.code, "invalid_argument");
        for part in [INPUT_VAR, "bluetooth", "auto", "dtuhidd", "runner"] {
            assert!(refused.message.contains(part), "{refused:?}");
        }
    }

    /// Task 5.1: the driver's errors keep their codes, a readiness state's
    /// among them, and their retryable flag; an app that is not installed is
    /// `app_not_found`, as on the other drivers.
    #[test]
    fn driver_errors_keep_their_code_and_retryable_flag() {
        let locked = error(&ios::Error::NotReady {
            target: PHONE.into(),
            readiness: Readiness::Locked,
        });
        assert_eq!(locked.code, "device_locked");
        assert!(!locked.retryable);
        assert!(locked.message.contains("unlock it"), "{locked:?}");
        let late = error(&ios::Error::Timeout {
            what: "the runner's reply".into(),
            after: Duration::from_secs(10),
        });
        assert_eq!(late.code, "timeout");
        assert!(late.retryable);
        let missing = app_error(&ios::Error::AppNotInstalled {
            target: PHONE.into(),
            bundle_id: "dev.mobileagent.placeholder".into(),
        });
        assert_eq!(missing.code, "app_not_found");
        assert!(
            missing.message.contains("dev.mobileagent.placeholder"),
            "{missing:?}"
        );
        let refused = app_error(&ios::Error::LaunchFailed {
            bundle_id: "dev.mobileagent.placeholder".into(),
            reason: "denied".into(),
        });
        assert_eq!(refused.code, "launch_failed");
    }

    /// Task 5.1: iOS key codes are keyboard usages with the HID modifier
    /// byte, or consumer-page buttons, as on the Simulator.
    #[test]
    fn keyboard_codes_become_keys_and_consumer_codes_buttons() {
        let key = |usage, modifiers| IosKey::Key(ios::Key { usage, modifiers });
        assert_eq!(
            ios_key(0x07 << 16 | 0x28, None).unwrap(),
            key(0x28, Modifiers::default())
        );
        assert_eq!(
            ios_key(0x04, Some(0x02 | 0x80)).unwrap(),
            key(
                0x04,
                Modifiers {
                    shift: true,
                    command: true,
                    ..Modifiers::default()
                }
            ),
            "a code without a page is a keyboard usage; right-hand keys count as left"
        );
        let button = |code: u32| ios_key(0x0C << 16 | code, None).unwrap();
        assert_eq!(button(0x40), IosKey::Button(ios::Button::Home));
        assert_eq!(button(0x30), IosKey::Button(ios::Button::Lock));
        assert_eq!(button(0xE9), IosKey::Button(ios::Button::VolumeUp));
        assert_eq!(button(0xEA), IosKey::Button(ios::Button::VolumeDown));
        for unknown in [0x0C << 16 | 0xCF, 0x09 << 16 | 0x01] {
            let refused = ios_key(unknown, None).unwrap_err();
            assert_eq!(refused.code, "unknown_key", "{refused:?}");
        }
    }
}
