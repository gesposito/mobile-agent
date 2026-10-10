//! Targets a run measures, and the classes gates are configured for.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::fixture::Fixture;

/// A class of target; gates are configured per class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetClass {
    Simulator,
    AndroidEmulator,
    AndroidDevice,
    /// A physical iPhone on USB, which `mobile-agent`'s `ios` driver drives.
    IosDevice,
}

impl TargetClass {
    /// Stable identifier used in reports and gate files.
    pub fn id(self) -> &'static str {
        match self {
            TargetClass::Simulator => "simulator",
            TargetClass::AndroidEmulator => "android-emulator",
            TargetClass::AndroidDevice => "android-device",
            TargetClass::IosDevice => "ios-device",
        }
    }

    /// The platform every target of this class runs.
    pub fn platform(self) -> Platform {
        match self {
            TargetClass::Simulator | TargetClass::IosDevice => Platform::Ios,
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

/// A physical iPhone's battery, as the tool under test read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    /// The charge, in percent.
    pub level_pct: Option<u8>,
    /// Whether the battery was charging.
    pub charging: Option<bool>,
    /// Whether the phone was on external power, a charger or the host's USB.
    pub external_power: Option<bool>,
}

/// A fixture's build the harness installed on a target before the run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Install {
    pub fixture: Fixture,
    /// The bundle as the run was given it.
    pub bundle: String,
    /// How long the install took, from the installing process's start to
    /// its exit.
    pub elapsed_us: u64,
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
    /// On a physical iPhone, the runner the tool under test started there,
    /// as its hello named it, such as `mobile-agent-ios-runner 0.2.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// On a physical iPhone, the path the tool under test sent input
    /// through, such as `dtuhidd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_path: Option<String>,
    /// On a physical iPhone, where the tool under test took frames from,
    /// such as `screenshots`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_path: Option<String>,
    /// On a physical iPhone, its battery as the tool under test read it when
    /// it first attached the phone in the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<Battery>,
    /// On a physical iPhone, `true` when the person running stated that its
    /// Auto-Lock was off (`run --auto-lock-off`): no tool reads the setting,
    /// so the report records it as stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_lock_off: Option<bool>,
    /// The fixtures' builds the harness installed on the target before the
    /// run, in order: on a physical iPhone, those `run --ios-app` names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub installs: Vec<Install>,
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
            runner: None,
            input_path: None,
            frame_path: None,
            battery: None,
            auto_lock_off: None,
            installs: Vec::new(),
        }
    }
}
