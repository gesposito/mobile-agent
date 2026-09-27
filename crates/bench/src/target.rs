//! Targets a run measures, and the classes gates are configured for.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A class of target; gates are configured per class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetClass {
    Simulator,
    AndroidEmulator,
    AndroidDevice,
}

impl TargetClass {
    /// Stable identifier used in reports and gate files.
    pub fn id(self) -> &'static str {
        match self {
            TargetClass::Simulator => "simulator",
            TargetClass::AndroidEmulator => "android-emulator",
            TargetClass::AndroidDevice => "android-device",
        }
    }

    /// The platform every target of this class runs.
    pub fn platform(self) -> Platform {
        match self {
            TargetClass::Simulator => Platform::Ios,
            TargetClass::AndroidEmulator | TargetClass::AndroidDevice => Platform::Android,
        }
    }
}

impl fmt::Display for TargetClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// The OS family of a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Android,
}

impl Platform {
    /// `ios` or `android`, as the baseline tools' `--platform` flags take it.
    pub fn id(self) -> &'static str {
        match self {
            Platform::Ios => "ios",
            Platform::Android => "android",
        }
    }
}

/// Whether an Android emulator's gRPC controller answered the tool under
/// test, which then sends taps through it and streams frames from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Controller {
    On,
    Off,
}

impl Controller {
    /// `on` or `off`, as reports print it.
    pub fn id(self) -> &'static str {
        match self {
            Controller::On => "on",
            Controller::Off => "off",
        }
    }
}

impl fmt::Display for Controller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// One device, emulator or Simulator in a run. The fields other than `id` and
/// `class` are provenance; the harness fills the ones an adapter leaves empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// Simulator UDID or adb serial.
    pub id: String,
    pub class: TargetClass,
    /// Device or emulator model, such as `iPhone 17` or `sdk_gphone64_arm64`.
    pub model: Option<String>,
    /// OS name and version, such as `iOS 26.5` or `Android 16`.
    pub os: Option<String>,
    /// Android API level; `None` on the Simulator.
    pub api_level: Option<u32>,
    /// On an Android emulator, whether its gRPC controller answered the tool
    /// under test; `None` on other targets and when the tool did not say.
    #[serde(default)]
    pub controller: Option<Controller>,
    /// Whether the target gave the tool under test frames, which tap to frame
    /// change needs; `None` when the tool did not say.
    #[serde(default)]
    pub frame_source: Option<bool>,
}

impl Target {
    /// A target with no provenance filled in yet.
    pub fn new(id: impl Into<String>, class: TargetClass) -> Self {
        Target {
            id: id.into(),
            class,
            model: None,
            os: None,
            api_level: None,
            controller: None,
            frame_source: None,
        }
    }
}
