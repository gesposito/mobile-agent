//! The fake driver: targets that `MOBILE_AGENT_FAKE_TARGETS` (or the
//! daemon's `--fake` flag) names, each a fake device on the mirror's
//! scripted [`mirror::testing::FakeTarget`], run in real time. Every test of
//! the daemon and the CLI, and the CLI overhead measurement, runs on them.
//!
//! A fake device shows a launcher (a heading and the buttons Fixture and
//! Settings), the fixture app of `fixtures/CONTRACT.md` under the ids
//! `dev.mobileagent.fixture.expo` and the platform's native fixture, a
//! settings app, and the placeholder app `dev.mobileagent.placeholder`, one
//! screen with the shapes a React Native app showed in the smoke test of
//! 2026-10-09 under placeholder names. Taps, text, keys and swipes change it
//! as the real apps change; screenshots return placeholder bytes, not an
//! image. Emulators and Simulators have a frame source and Android devices
//! none, as design D7 has it, and emulators have none either when the
//! daemon's emulator controller setting is off; `status` names an Android
//! target's input and frame paths as the Android driver names them. A fake
//! iPhone (`ios-device`) stands in for a physical one: an iOS fake whose
//! frames are screenshots, in the `ios-device` class, whose `status` names
//! its input, frames and runner as the physical iOS driver names them, and
//! which [`FakeDriver::with_readiness`] can list in a readiness state that
//! refuses an attach, as that driver lists a phone that is not ready. A latency
//! (`MOBILE_AGENT_FAKE_LATENCY_MS`) delays every change,
//! so that the time from a tap to its effects can be measured, and a frame
//! delay (`MOBILE_AGENT_FAKE_FRAME_DELAY_MS`) starts the frame stream after
//! the attach, as an emulator's starts once its subscription is up. A
//! splash (`MOBILE_AGENT_FAKE_SPLASH_MS`) shows a fixture app's launch
//! splash for that long when its process starts, an activation
//! (`MOBILE_AGENT_FAKE_ACTIVATION_MS`) holds input until the input session
//! has been active that long, as a Simulator's dtuhidd session does, and a
//! stop (`MOBILE_AGENT_FAKE_STOP_MS`) makes detaching a target take that
//! long, as stopping the Android agent does. A fling
//! (`MOBILE_AGENT_FAKE_FLING`) makes the fixture's list glide on after a
//! stroke whose finger lifts moving, as far as the platform's fling carries
//! it; without it, and after a stroke whose finger comes to rest, the list
//! moves as far as the finger. A typing mode (`MOBILE_AGENT_FAKE_TYPING`)
//! says how the fixture's fields take text whose method the target may
//! choose, as `fill` sends it, which they otherwise take whole, as the
//! Android agent sets it (see [`Typing`]).
//!
//! A fake shows its keyboard while a field has focus, and with
//! [`FakeDriver::with_keyboard`] over every fixture screen, so a list can
//! lie partly under it: on Android a window over the screen from 600 points
//! down, on iOS the window the Simulator's reader gives the keyboard, from
//! its prediction bar at 539 points down. The keyboard comes up with the
//! focus, or as [`FakeDriver::with_slide`] says (see [`Slide`]): some time
//! after it, an iOS field losing the text that comes sooner, or never. A
//! stroke that comes down on the
//! keyboard goes to the keyboard and moves nothing: its glide typing enters
//! a word into the field in focus, as Gboard took the strokes of a `scroll`
//! that started on its keys on a Nexus 5X
//! (`reports/10.3-physical-android.md`, finding 4).

mod app;
mod realtime;
mod tree;

use std::sync::Arc;
use std::time::Duration;

use realtime::Frames;
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
    /// How long detaching the target takes, as the Android agent's
    /// `shutdown` and kill take.
    pub stop: Duration,
    /// How the keyboard comes up once a field takes the focus.
    pub slide: Slide,
}

/// How a fake's keyboard comes up once a field takes the focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Slide {
    /// With the focus.
    #[default]
    AtOnce,
    /// That long after the field took the focus. On iOS text that reaches
    /// the field before then is lost, typed or pasted, as the first keys
    /// typed into a just-focused field were lost while the Simulator's
    /// keyboard slid in, in 3 of 8 runs of the React Native smoke test of
    /// 2026-10-09; Android's text set types nothing and is never lost.
    After(Duration),
    /// Never: no keyboard window shows, and fields take text, as with a
    /// hardware keyboard connected to a Simulator.
    Never,
}

/// How fake targets' fields take text whose method the target may choose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Typing {
    /// Whole, set directly as the Android agent sets it.
    #[default]
    Whole,
    /// Key by key up to 32 characters, as a Simulator types it, and pasted
    /// whole beyond.
    Keys,
    /// As [`Typing::Keys`], each key from the fourth on landing before the
    /// field's last character, so "nobel" reads "noelb", as Bluesky's search
    /// field took it on the Simulator (`reports/10.4-rn-smoke.md`).
    Reorder,
    /// Whole, and the fields keep their first `n` characters, however text
    /// reaches them, as a field with a maximum length does.
    Max(usize),
}

impl Typing {
    /// Parses `whole`, `keys`, `reorder` or `max:<n>`.
    pub fn parse(text: &str) -> Result<Typing, String> {
        let text = text.trim();
        match text {
            "whole" => Ok(Typing::Whole),
            "keys" => Ok(Typing::Keys),
            "reorder" => Ok(Typing::Reorder),
            _ => text
                .strip_prefix("max:")
                .and_then(|n| n.parse().ok())
                .map(Typing::Max)
                .ok_or_else(|| {
                    format!("fake typing {text:?}: use whole, keys, reorder or max:<characters>")
                }),
        }
    }
}

/// Fake targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeDriver {
    targets: Vec<TargetInfo>,
    timing: Timing,
    /// Whether fake emulators take the controller's path, with frames.
    controller: EmulatorController,
    /// Whether a stroke whose finger lifts moving glides the list on.
    fling: bool,
    /// How the fields take text whose method the target may choose.
    typing: Typing,
    /// Whether the keyboard stays up over every fixture screen.
    keyboard: bool,
}

impl FakeDriver {
    /// Parses a comma-separated list of `platform[:id]` entries, such as
    /// `android:emulator-5554,ios:FAKE-IOS-1`, the platform `android`, `ios`
    /// for a Simulator or `ios-device` for a physical iPhone. Without an id,
    /// Android fakes are `emulator-5554`, `emulator-5556` and so on, iOS fakes
    /// `FAKE-IOS-1`, `FAKE-IOS-2`, and fake iPhones `FAKE-IOS-DEVICE-1`,
    /// `FAKE-IOS-DEVICE-2`. An Android id starting `emulator-` is an
    /// emulator, any other a device.
    pub fn parse(spec: &str) -> Result<FakeDriver, String> {
        let mut targets: Vec<TargetInfo> = Vec::new();
        let (mut android, mut ios, mut devices) = (0u32, 0u32, 0u32);
        for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let (platform, id) = match entry.split_once(':') {
                Some((platform, id)) => (platform.trim(), Some(id.trim())),
                None => (entry, None),
            };
            let device = platform.eq_ignore_ascii_case("ios-device");
            let platform = if device {
                Platform::Ios
            } else {
                Platform::parse(platform).ok_or_else(|| {
                    format!(
                        "fake target {entry:?}: the platform must be android, ios or ios-device"
                    )
                })?
            };
            let id = match (platform, id) {
                (_, Some(id)) if !id.is_empty() => id.to_owned(),
                (Platform::Android, _) => {
                    android += 1;
                    format!("emulator-{}", 5552 + 2 * android)
                }
                (Platform::Ios, _) if device => {
                    devices += 1;
                    format!("FAKE-IOS-DEVICE-{devices}")
                }
                (Platform::Ios, _) => {
                    ios += 1;
                    format!("FAKE-IOS-{ios}")
                }
            };
            if targets.iter().any(|t| t.id == id) {
                return Err(format!("fake target {id} is listed twice"));
            }
            let validated = format!("iOS {}", ios::VALIDATED_IOS);
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
                Platform::Ios if device => (
                    TargetClass::IosDevice,
                    "Fake physical iPhone",
                    validated.as_str(),
                    ios::Readiness::Ready.code(),
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
            fling: false,
            typing: Typing::Whole,
            keyboard: false,
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

    /// These targets, each taking `stop` to detach.
    pub fn with_stop(mut self, stop: Duration) -> FakeDriver {
        self.timing.stop = stop;
        self
    }

    /// These targets under the daemon's emulator controller setting: off,
    /// emulators take the agent's path, without a frame source.
    pub fn with_emulator_controller(mut self, controller: EmulatorController) -> FakeDriver {
        self.controller = controller;
        self
    }

    /// These targets, their fixture's list gliding on after a stroke whose
    /// finger lifts moving when `fling` is set, as far as the platform's
    /// fling carries it.
    pub fn with_fling(mut self, fling: bool) -> FakeDriver {
        self.fling = fling;
        self
    }

    /// These targets, their fixture's fields taking text whose method the
    /// target may choose as `typing` says.
    pub fn with_typing(mut self, typing: Typing) -> FakeDriver {
        self.typing = typing;
        self
    }

    /// These targets, keeping their keyboard up over every screen of the
    /// fixture apps when `up` is set, whether or not a field has focus.
    pub fn with_keyboard(mut self, up: bool) -> FakeDriver {
        self.keyboard = up;
        self
    }

    /// These targets, their keyboard coming up as `slide` says once a field
    /// takes the focus.
    pub fn with_slide(mut self, slide: Slide) -> FakeDriver {
        self.timing.slide = slide;
        self
    }

    /// These targets, the fake iPhone `id` listed in the readiness state
    /// `code` with `advice`, as the physical iOS driver lists a phone that
    /// is not ready (`crates/ios`'s readiness states and messages); an
    /// attach to it fails with `code`. Other targets are left as they are.
    pub fn with_readiness(mut self, id: &str, code: &str, advice: &str) -> FakeDriver {
        for target in &mut self.targets {
            if target.id == id && target.class == TargetClass::IosDevice {
                target.state = code.to_owned();
                target.advice = (code != ios::Readiness::Ready.code()).then(|| advice.to_owned());
            }
        }
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
        // A phone that is not ready refuses a session with its state's
        // code, as the physical iOS driver's attach does.
        if target.class == TargetClass::IosDevice && target.state != ios::Readiness::Ready.code() {
            let advice = target.advice.as_deref().unwrap_or("it is not ready");
            return Err(DriverError::new(
                &target.state,
                format!("iOS device {} is not ready: {advice}", target.id),
            ));
        }
        let frames = match target.class {
            TargetClass::Simulator => Frames::Stream,
            TargetClass::IosDevice => Frames::Screenshots,
            TargetClass::AndroidEmulator if self.controller == EmulatorController::Auto => {
                Frames::Stream
            }
            TargetClass::AndroidEmulator | TargetClass::AndroidDevice => Frames::None,
        };
        let mut attachment = realtime::attach(
            target.platform,
            frames,
            self.timing,
            self.fling,
            self.typing,
            self.keyboard,
        );
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
    fn typing_modes_are_named_whole_keys_reorder_or_max_with_a_length() {
        assert_eq!(Typing::parse("whole"), Ok(Typing::Whole));
        assert_eq!(Typing::parse(" keys "), Ok(Typing::Keys));
        assert_eq!(Typing::parse("reorder"), Ok(Typing::Reorder));
        assert_eq!(Typing::parse("max:8"), Ok(Typing::Max(8)));
        for bad in ["", "slow", "max:", "max:-1", "max:eight"] {
            assert!(Typing::parse(bad).is_err(), "{bad:?}");
        }
    }

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

    /// Task 5.1: `ios-device` stands in for a physical iPhone, its ids
    /// `FAKE-IOS-DEVICE-1`, `FAKE-IOS-DEVICE-2` unless given, listed in the
    /// `ios-device` class, ready, on the iOS the physical driver was
    /// validated on.
    #[test]
    fn ios_device_fakes_take_default_ids_and_the_ios_device_class() {
        let driver =
            FakeDriver::parse("ios-device, ios, ios-device:FAKE-IOS-DEVICE-9, ios-device").unwrap();
        let ids: Vec<(&str, TargetClass)> = driver
            .targets
            .iter()
            .map(|t| (t.id.as_str(), t.class))
            .collect();
        assert_eq!(
            ids,
            [
                ("FAKE-IOS-DEVICE-1", TargetClass::IosDevice),
                ("FAKE-IOS-1", TargetClass::Simulator),
                ("FAKE-IOS-DEVICE-9", TargetClass::IosDevice),
                ("FAKE-IOS-DEVICE-2", TargetClass::IosDevice),
            ]
        );
        let device = &driver.targets[0];
        assert_eq!(device.platform, Platform::Ios);
        assert_eq!(device.state, "ready");
        assert_eq!(device.os.as_deref(), Some("iOS 26.6.2"));
        assert_eq!(device.advice, None);
        let refused = FakeDriver::parse("ios-devices").unwrap_err();
        assert!(refused.contains("ios-device"), "{refused}");
    }

    /// Task 5.1, spec `ios-device-driver` "Device not ready": a fake iPhone
    /// can be listed in a readiness state with its advice, and an attach
    /// to it fails with the state's code, as the driver's commands do.
    #[test]
    fn an_unready_fake_iphone_is_listed_in_its_state_and_refuses_an_attach() {
        let advice = ios::Readiness::DeveloperModeOff.message();
        let driver = FakeDriver::parse("ios-device,ios-device")
            .unwrap()
            .with_readiness("FAKE-IOS-DEVICE-2", "developer_mode_off", &advice);
        let listed = driver.discover().unwrap();
        assert_eq!(listed[0].state, "ready");
        assert_eq!(listed[1].state, "developer_mode_off");
        assert_eq!(listed[1].advice.as_deref(), Some(advice.as_str()));
        match driver.attach(&listed[1], &AttachOptions::default()) {
            Ok(_) => panic!("an unready iPhone attaches"),
            Err(refused) => {
                assert_eq!(refused.code, "developer_mode_off");
                assert!(refused.message.contains("Developer Mode"), "{refused:?}");
            }
        }
        assert!(driver.attach(&listed[0], &AttachOptions::default()).is_ok());
    }

    /// Task 5.1: a fake iPhone's status names its input path, `dtuhidd`, its
    /// frames, screenshots polled at the device's rate, and the runner by its
    /// agent and restarts, as the physical driver's status does.
    #[test]
    fn a_fake_iphone_reports_dtuhidd_screenshots_and_its_runner() {
        let driver = FakeDriver::parse("ios-device").unwrap();
        let target = driver.discover().unwrap().remove(0);
        let attachment = driver.attach(&target, &AttachOptions::default()).unwrap();
        assert!(attachment.backend.frames.is_some());
        let report = attachment.report.expect("a report").report();
        let data = Value::Object(report.data.clone());
        assert_eq!(data["input"], "dtuhidd", "{data}");
        assert_eq!(data["frames"], "screenshots", "{data}");
        assert_eq!(data["runner"]["agent"], agent_ios::AGENT, "{data}");
        assert_eq!(data["runner"]["restarts"], 0, "{data}");
        let text = report.lines.join("\n");
        assert!(
            text.contains("input: dtuhidd")
                && text.contains("frames: screenshots")
                && text.contains(agent_ios::AGENT),
            "{text}"
        );
    }
}
