//! The reference fixture apps and their screens, as `fixtures/CONTRACT.md`
//! defines them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::target::Platform;

/// A reference fixture app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fixture {
    /// SwiftUI, iOS only.
    SwiftUi,
    /// Expo React Native, iOS and Android.
    Expo,
    /// Jetpack Compose, Android only.
    Compose,
}

impl Fixture {
    /// Stable identifier used in reports and gate files.
    pub fn id(self) -> &'static str {
        match self {
            Fixture::SwiftUi => "swiftui",
            Fixture::Expo => "expo",
            Fixture::Compose => "compose",
        }
    }

    /// Bundle identifier or package name on `platform`, or `None` where the
    /// fixture does not exist.
    pub fn app_id(self, platform: Platform) -> Option<&'static str> {
        match (self, platform) {
            (Fixture::SwiftUi, Platform::Ios) => Some("dev.mobileagent.fixture.swiftui"),
            (Fixture::Expo, _) => Some("dev.mobileagent.fixture.expo"),
            (Fixture::Compose, Platform::Android) => Some("dev.mobileagent.fixture.compose"),
            _ => None,
        }
    }
}

impl fmt::Display for Fixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// A fixture screen, named like its tab and the value of the launch option
/// that opens it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Screen {
    Login,
    List,
    /// The deterministic 500-cell screen, the spec's reference screen.
    Big,
    Anim,
    System,
}

impl Screen {
    /// Stable identifier, also the value of the launch option that opens the
    /// screen.
    pub fn id(self) -> &'static str {
        match self {
            Screen::Login => "login",
            Screen::List => "list",
            Screen::Big => "big",
            Screen::Anim => "anim",
            Screen::System => "system",
        }
    }

    /// The `KEY=VALUE` launch option that opens a fixture on this screen on
    /// `platform`, as `mobile-agent open --env` passes it: the environment
    /// variable `FIXTURE_INITIAL_TAB` on iOS, and on Android the launch
    /// intent's string extra `initial_tab`, which is what `--env` becomes there
    /// (`fixtures/CONTRACT.md`, Launch options).
    pub fn launch_option(self, platform: Platform) -> String {
        let key = match platform {
            Platform::Ios => "FIXTURE_INITIAL_TAB",
            Platform::Android => "initial_tab",
        };
        format!("{key}={}", self.id())
    }
}

impl fmt::Display for Screen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_follow_the_fixture_readmes() {
        assert_eq!(
            Fixture::SwiftUi.app_id(Platform::Ios),
            Some("dev.mobileagent.fixture.swiftui")
        );
        assert_eq!(Fixture::SwiftUi.app_id(Platform::Android), None);
        assert_eq!(
            Fixture::Expo.app_id(Platform::Ios),
            Fixture::Expo.app_id(Platform::Android)
        );
        assert_eq!(
            Fixture::Compose.app_id(Platform::Android),
            Some("dev.mobileagent.fixture.compose")
        );
        assert_eq!(Fixture::Compose.app_id(Platform::Ios), None);
    }

    #[test]
    fn each_platform_opens_a_screen_through_the_launch_option_its_fixtures_read() {
        assert_eq!(
            Screen::Big.launch_option(Platform::Android),
            "initial_tab=big"
        );
        assert_eq!(
            Screen::List.launch_option(Platform::Android),
            "initial_tab=list"
        );
        assert_eq!(
            Screen::Big.launch_option(Platform::Ios),
            "FIXTURE_INITIAL_TAB=big"
        );
        assert_eq!(
            Screen::Login.launch_option(Platform::Ios),
            "FIXTURE_INITIAL_TAB=login"
        );
    }

    #[test]
    fn serialized_names_are_the_ids() {
        for fixture in [Fixture::SwiftUi, Fixture::Expo, Fixture::Compose] {
            assert_eq!(
                serde_json::to_string(&fixture).unwrap(),
                format!("\"{}\"", fixture.id())
            );
        }
        for screen in [
            Screen::Login,
            Screen::List,
            Screen::Big,
            Screen::Anim,
            Screen::System,
        ] {
            assert_eq!(
                serde_json::to_string(&screen).unwrap(),
                format!("\"{}\"", screen.id())
            );
        }
    }
}
