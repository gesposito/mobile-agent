//! Maestro and maestro-runner, which runs Maestro flow files unchanged. A flow
//! becomes one Maestro flow file, run as
//! `maestro --device <id> test <file>` and as
//! `maestro-runner --platform <ios|android> --device <id> test <file>`, the
//! shapes of their CLI references. Both exit non-zero when a flow fails.
//!
//! Maestro matches `id` and `text` selectors as regular expressions, so
//! selector values are escaped; `launchApp` stops a running app before it
//! launches it, and `inputText` types into the focused field, so a fill taps
//! its field first.

use std::ffi::OsString;
use std::path::Path;

use crate::flow::Flow;
use crate::target::Target;

/// The Maestro flow file for `flow` against the app `app_id`.
pub fn flow_yaml(flow: &Flow, app_id: &str) -> String {
    todo!("render {} for {app_id}", flow.name)
}

/// `value` as a regular expression that matches exactly it.
pub fn regex_literal(value: &str) -> String {
    todo!("escape {value}")
}

/// `value` as a YAML double-quoted scalar.
pub fn quoted(value: &str) -> String {
    todo!("quote {value}")
}

/// Arguments for `maestro`.
pub fn maestro_args(target: &Target, file: &Path) -> Vec<OsString> {
    todo!("maestro arguments for {} and {}", target.id, file.display())
}

/// Arguments for `maestro-runner`.
pub fn runner_args(target: &Target, file: &Path) -> Vec<OsString> {
    todo!(
        "maestro-runner arguments for {} and {}",
        target.id,
        file.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;
    use crate::flow::{Direction, Selector, Step};
    use crate::target::TargetClass;

    fn every_step() -> Flow {
        Flow {
            name: "every-step".into(),
            fixture: Fixture::Expo,
            steps: vec![
                Step::Launch,
                Step::Tap(Selector::id("tab-list")),
                Step::ScrollTo {
                    target: Selector::id("row-200"),
                    direction: Direction::Down,
                },
                Step::Tap(Selector::text("Row 200")),
                Step::AssertVisible(Selector::id("detail-200")),
                Step::Fill {
                    target: Selector::id("login-username"),
                    text: "agent".into(),
                },
                Step::ScrollTo {
                    target: Selector::text("Row 1"),
                    direction: Direction::Up,
                },
                Step::AssertText {
                    id: "login-status".into(),
                    text: "Welcome, agent".into(),
                },
            ],
            count_nodes: false,
        }
    }

    #[test]
    fn every_step_becomes_its_maestro_command() {
        assert_eq!(
            flow_yaml(&every_step(), "dev.mobileagent.fixture.expo"),
            r#"appId: dev.mobileagent.fixture.expo
---
- launchApp
- tapOn:
    id: "tab-list"
- scrollUntilVisible:
    element:
      id: "row-200"
    direction: DOWN
- tapOn:
    text: "Row 200"
- assertVisible:
    id: "detail-200"
- tapOn:
    id: "login-username"
- inputText: "agent"
- scrollUntilVisible:
    element:
      text: "Row 1"
    direction: UP
- assertVisible:
    id: "login-status"
    text: "Welcome, agent"
"#
        );
    }

    #[test]
    fn selector_values_match_literally_and_quote_safely() {
        assert_eq!(regex_literal("Row 1.5 (x)"), r"Row 1\.5 \(x\)");
        assert_eq!(
            regex_literal(r"a+b*c?[d]{e}^$|\"),
            r"a\+b\*c\?\[d\]\{e\}\^\$\|\\"
        );
        assert_eq!(regex_literal("tab-list"), "tab-list");
        assert_eq!(quoted(r#"say "hi" \o/"#), r#""say \"hi\" \\o/""#);
        let flow = Flow {
            name: "escape".into(),
            fixture: Fixture::Expo,
            steps: vec![
                Step::Tap(Selector::text("Row 1.5")),
                Step::Fill {
                    target: Selector::id("login-username"),
                    text: "a.b \"c\"".into(),
                },
            ],
            count_nodes: false,
        };
        let yaml = flow_yaml(&flow, "dev.mobileagent.fixture.expo");
        assert!(yaml.contains(r#"    text: "Row 1\\.5""#), "{yaml}");
        assert!(yaml.contains(r#"- inputText: "a.b \"c\"""#), "{yaml}");
    }

    #[test]
    fn maestro_names_the_device_and_maestro_runner_the_platform_too() {
        let file = Path::new("/work/maestro-login-emulator-5554.yaml");
        let emulator = Target::new("emulator-5554", TargetClass::AndroidEmulator);
        let sim = Target::new(
            "B0647CDD-575F-45E9-9398-F6F5AFF23C1F",
            TargetClass::Simulator,
        );
        let strings = |args: Vec<OsString>| -> Vec<String> {
            args.into_iter().map(|a| a.into_string().unwrap()).collect()
        };
        assert_eq!(
            strings(maestro_args(&emulator, file)),
            [
                "--device",
                "emulator-5554",
                "test",
                "/work/maestro-login-emulator-5554.yaml"
            ]
        );
        assert_eq!(
            strings(runner_args(&sim, file)),
            [
                "--platform",
                "ios",
                "--device",
                "B0647CDD-575F-45E9-9398-F6F5AFF23C1F",
                "test",
                "/work/maestro-login-emulator-5554.yaml"
            ]
        );
        assert_eq!(strings(runner_args(&emulator, file))[1], "android");
    }
}
