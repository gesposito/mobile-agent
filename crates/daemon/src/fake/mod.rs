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
//! has it; a latency (`MOBILE_AGENT_FAKE_LATENCY_MS`) delays every change,
//! so that the time from a tap to its effects can be measured, and a frame
//! delay (`MOBILE_AGENT_FAKE_FRAME_DELAY_MS`) starts the frame stream after
//! the attach, as an emulator's starts once its subscription is up.

mod app;
mod realtime;
mod tree;

use std::time::Duration;

use crate::driver::{
    AttachOptions, Attachment, Driver, DriverError, Platform, TargetClass, TargetInfo,
};

/// Fake targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeDriver {
    targets: Vec<TargetInfo>,
    latency: Duration,
    frame_delay: Duration,
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
            latency: Duration::ZERO,
            frame_delay: Duration::ZERO,
        })
    }

    /// These targets, showing each change `latency` after its cause.
    pub fn with_latency(self, latency: Duration) -> FakeDriver {
        FakeDriver { latency, ..self }
    }

    /// These targets, their frame streams starting `frame_delay` after the
    /// attach.
    pub fn with_frame_delay(self, frame_delay: Duration) -> FakeDriver {
        FakeDriver {
            frame_delay,
            ..self
        }
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
        let frames = target.class != TargetClass::AndroidDevice;
        Ok(realtime::attach(
            target.platform,
            frames,
            self.latency,
            self.frame_delay,
        ))
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
}
