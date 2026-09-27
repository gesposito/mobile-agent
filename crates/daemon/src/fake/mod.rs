//! The fake driver: targets that `MOBILE_AGENT_FAKE_TARGETS` (or the
//! daemon's `--fake` flag) names, each a fake device on the mirror's
//! scripted [`mirror::testing::FakeTarget`], run in real time. Every test of
//! the daemon and the CLI, and the CLI overhead measurement, runs on them.
//!
//! A fake device shows a launcher (a heading and the buttons Fixture and
//! Settings), the fixture app of `fixtures/CONTRACT.md` under the ids
//! `dev.mobileagent.fixture.expo` and the platform's native fixture, and a
//! settings app. Taps, text, keys and swipes change it as the real apps
//! change; screenshots return placeholder bytes, not an image. Emulators and
//! Simulators have a frame source and Android devices none, as design D7
//! has it, and emulators have none either when the daemon's emulator
//! controller setting is off; `status` names an Android target's input and
//! frame paths as the Android driver names them. A latency
//! (`MOBILE_AGENT_FAKE_LATENCY_MS`) delays every change,
//! so that the time from a tap to its effects can be measured, and a frame
//! delay (`MOBILE_AGENT_FAKE_FRAME_DELAY_MS`) starts the frame stream after
//! the attach, as an emulator's starts once its subscription is up. A
//! splash (`MOBILE_AGENT_FAKE_SPLASH_MS`) shows a fixture app's launch
//! splash for that long when its process starts, and an activation
//! (`MOBILE_AGENT_FAKE_ACTIVATION_MS`) holds input until the input session
//! has been active that long, as a Simulator's dtuhidd session does.

mod app;
mod realtime;
mod tree;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::devices::android::report_controller;
use crate::driver::{
    AttachOptions, Attachment, Driver, DriverError, EmulatorController, Platform, Report,
    TargetClass, TargetInfo, TargetReport,
};

/// How fake targets take their time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Timing {
    /// How long a change takes to show, as a slow app or device takes.
    pub latency: Duration,
    /// How long after the attach the frame stream starts.
    pub frame_delay: Duration,
    /// How long a fixture app shows its launch splash when its process
    /// starts.
    pub splash: Duration,
    /// How long after it starts the input session is active; input before
    /// then waits for it.
    pub activation: Duration,
}

/// Fake targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeDriver {
    targets: Vec<TargetInfo>,
    timing: Timing,
    /// Whether fake emulators take the controller's path, with frames.
    controller: EmulatorController,
}

impl FakeDriver {
    /// Parses a comma-separated list of `platform[:id]` entries, such as
    /// `android:emulator-5554,ios:FAKE-IOS-1`. Without an id, Android fakes
    /// are `emulator-5554`, `emulator-5556` and so on, iOS fakes
    /// `FAKE-IOS-1`, `FAKE-IOS-2`. An Android id starting `emulator-` is an
    /// emulator, any other a device.
    pub fn parse(spec: &str) -> Result<FakeDriver, String> {
        let mut targets: Vec<TargetInfo> = Vec::new();
        let (mut android, mut ios) = (0u32, 0u32);
        for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let (platform, id) = match entry.split_once(':') {
                Some((platform, id)) => (platform, Some(id.trim())),
                None => (entry, None),
            };
            let platform = Platform::parse(platform.trim()).ok_or_else(|| {
                format!("fake target {entry:?}: the platform must be android or ios")
            })?;
            let id = match (platform, id) {
                (_, Some(id)) if !id.is_empty() => id.to_owned(),
                (Platform::Android, _) => {
                    android += 1;
                    format!("emulator-{}", 5552 + 2 * android)
                }
                (Platform::Ios, _) => {
                    ios += 1;
                    format!("FAKE-IOS-{ios}")
                }
            };
            if targets.iter().any(|t| t.id == id) {
                return Err(format!("fake target {id} is listed twice"));
            }
            let (class, name, os, state) = match platform {
                Platform::Android if id.starts_with("emulator-") => (
                    TargetClass::AndroidEmulator,
                    "Fake Android emulator",
                    "Android 16",
                    "device",
                ),
                Platform::Android => (
                    TargetClass::AndroidDevice,
                    "Fake Android device",
                    "Android 16",
                    "device",
                ),
                Platform::Ios => (TargetClass::Simulator, "Fake iPhone", "iOS 26.5", "booted"),
            };
            targets.push(TargetInfo {
                id,
                platform,
                class,
                name: name.to_owned(),
                model: Some("fake".to_owned()),
                os: Some(os.to_owned()),
                state: state.to_owned(),
                advice: None,
                driver: "fake".to_owned(),
            });
        }
        Ok(FakeDriver {
            targets,
            timing: Timing::default(),
            controller: EmulatorController::Auto,
        })
    }

    /// These targets, showing each change `latency` after its cause.
    pub fn with_latency(mut self, latency: Duration) -> FakeDriver {
        self.timing.latency = latency;
        self
    }

    /// These targets, their frame streams starting `frame_delay` after the
    /// attach.
    pub fn with_frame_delay(mut self, frame_delay: Duration) -> FakeDriver {
        self.timing.frame_delay = frame_delay;
        self
    }

    /// These targets, the fixture apps showing their launch splash for
    /// `splash` when their process starts.
    pub fn with_splash(mut self, splash: Duration) -> FakeDriver {
        self.timing.splash = splash;
        self
    }

    /// These targets, their input sessions active `activation` after they
    /// start.
    pub fn with_activation(mut self, activation: Duration) -> FakeDriver {
        self.timing.activation = activation;
        self
    }

    /// These targets under the daemon's emulator controller setting: off,
    /// emulators take the agent's path, without a frame source.
    pub fn with_emulator_controller(mut self, controller: EmulatorController) -> FakeDriver {
        self.controller = controller;
        self
    }
}

/// What `status` shows about a fake Android emulator the switch keeps off
/// its controller: its input and frames take the agent's path, named as the
/// Android driver names them. A fake target on its default paths reports
/// nothing about them.
struct AgentPath;

impl Report for AgentPath {
    fn report(&self) -> TargetReport {
        let mut data = Map::new();
        data.insert("input".into(), "agent".into());
        data.insert("frames".into(), Value::Null);
        let mut report = TargetReport {
            lines: vec![
                "input: agent (taps, long presses and swipes)".to_owned(),
                "frames: none; screenshots come from the agent on demand".to_owned(),
            ],
            data,
        };
        report_controller(&mut report, EmulatorController::Off, true);
        report
    }
}

impl Driver for FakeDriver {
    fn name(&self) -> &str {
        "fake"
    }

    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError> {
        Ok(self.targets.clone())
    }

    fn attach(
        &self,
        target: &TargetInfo,
        _options: &AttachOptions,
    ) -> Result<Attachment, DriverError> {
        let frames = match target.class {
            TargetClass::Simulator => true,
            TargetClass::AndroidEmulator => self.controller == EmulatorController::Auto,
            TargetClass::AndroidDevice => false,
        };
        let mut attachment = realtime::attach(target.platform, frames, self.timing);
        if target.class == TargetClass::AndroidEmulator
            && self.controller == EmulatorController::Off
        {
            attachment.report = Some(Arc::new(AgentPath) as Arc<dyn Report>);
        }
        Ok(attachment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_targets_take_default_ids_and_classes() {
        let driver = FakeDriver::parse("android, ios, android:R58M123 ,android").unwrap();
        let ids: Vec<(&str, TargetClass)> = driver
            .targets
            .iter()
            .map(|t| (t.id.as_str(), t.class))
            .collect();
        assert_eq!(
            ids,
            [
                ("emulator-5554", TargetClass::AndroidEmulator),
                ("FAKE-IOS-1", TargetClass::Simulator),
                ("R58M123", TargetClass::AndroidDevice),
                ("emulator-5556", TargetClass::AndroidEmulator),
            ]
        );
        assert!(FakeDriver::parse("windows").is_err());
        assert!(FakeDriver::parse("ios:X,android:X").is_err());
    }

    /// Task 10.12: with the emulator controller setting off, a fake
    /// emulator attaches without a frame source and reports the agent's
    /// paths, as a real emulator does; Simulators keep their frames.
    #[test]
    fn the_controller_setting_off_puts_fake_emulators_on_the_agent_path() {
        let targets = |driver: &FakeDriver| driver.targets.clone();
        for controller in [EmulatorController::Auto, EmulatorController::Off] {
            let driver = FakeDriver::parse("android, android:R58M123, ios")
                .unwrap()
                .with_emulator_controller(controller);
            let listed = targets(&driver);
            let attach = |id: &str| {
                let target = listed.iter().find(|t| t.id == id).unwrap();
                driver.attach(target, &AttachOptions::default()).unwrap()
            };
            let emulator = attach("emulator-5554");
            let off = controller == EmulatorController::Off;
            assert_eq!(emulator.backend.frames.is_some(), !off, "{controller:?}");
            match emulator.report.as_ref().map(|r| r.report()) {
                Some(report) => {
                    assert!(off, "{controller:?}: {:?}", report.data);
                    assert_eq!(report.data["input"], "agent");
                    assert!(report.data["frames"].is_null(), "{:?}", report.data);
                    assert_eq!(report.data["emulatorController"], "off");
                }
                None => assert!(!off, "an emulator on the agent's path says so"),
            }
            let device = attach("R58M123");
            assert!(device.backend.frames.is_none());
            assert!(device.report.is_none(), "{controller:?}");
            let simulator = attach("FAKE-IOS-1");
            assert!(simulator.backend.frames.is_some(), "{controller:?}");
            assert!(simulator.report.is_none());
        }
    }
}
