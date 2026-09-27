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

use serde_json::Value;

use super::{json_document, last_line};
use crate::end_state::{Element, Reading};
use crate::flow::{Direction, Flow, Selector, Step};
use crate::target::Target;

/// The Maestro flow file for `flow` against the app `app_id`.
pub fn flow_yaml(flow: &Flow, app_id: &str) -> String {
    let mut yaml = format!("appId: {app_id}\n---\n");
    for step in &flow.steps {
        match step {
            Step::Launch => yaml.push_str("- launchApp\n"),
            Step::Tap(target) => {
                yaml.push_str("- tapOn:\n");
                yaml.push_str(&selector(target, 4));
            }
            Step::Fill { target, text } => {
                yaml.push_str("- tapOn:\n");
                yaml.push_str(&selector(target, 4));
                yaml.push_str(&format!("- inputText: {}\n", quoted(text)));
            }
            Step::ScrollTo { target, direction } => {
                yaml.push_str("- scrollUntilVisible:\n    element:\n");
                yaml.push_str(&selector(target, 6));
                let direction = match direction {
                    Direction::Down => "DOWN",
                    Direction::Up => "UP",
                };
                yaml.push_str(&format!("    direction: {direction}\n"));
            }
            Step::AssertVisible(target) => {
                yaml.push_str("- assertVisible:\n");
                yaml.push_str(&selector(target, 4));
            }
            Step::AssertText { id, text } => {
                yaml.push_str("- assertVisible:\n");
                yaml.push_str(&selector(&Selector::Id(id.clone()), 4));
                yaml.push_str(&selector(&Selector::Text(text.clone()), 4));
            }
        }
    }
    yaml
}

/// One selector line at `indent` spaces.
fn selector(selector: &Selector, indent: usize) -> String {
    let (key, value) = match selector {
        Selector::Id(id) => ("id", id),
        Selector::Text(text) => ("text", text),
    };
    format!("{:indent$}{key}: {}\n", "", quoted(&regex_literal(value)))
}

/// `value` as a regular expression that matches exactly it.
pub fn regex_literal(value: &str) -> String {
    let mut literal = String::with_capacity(value.len());
    for c in value.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            literal.push('\\');
        }
        literal.push(c);
    }
    literal
}

/// `value` as a YAML double-quoted scalar.
pub fn quoted(value: &str) -> String {
    let mut scalar = String::with_capacity(value.len() + 2);
    scalar.push('"');
    for c in value.chars() {
        match c {
            '\\' => scalar.push_str("\\\\"),
            '"' => scalar.push_str("\\\""),
            '\n' => scalar.push_str("\\n"),
            _ => scalar.push(c),
        }
    }
    scalar.push('"');
    scalar
}

/// Arguments for `maestro`.
pub fn maestro_args(target: &Target, file: &Path) -> Vec<OsString> {
    vec![
        "--device".into(),
        target.id.clone().into(),
        "test".into(),
        file.as_os_str().to_owned(),
    ]
}

/// Arguments for `maestro-runner`.
pub fn runner_args(target: &Target, file: &Path) -> Vec<OsString> {
    vec![
        "--platform".into(),
        target.class.platform().id().into(),
        "--device".into(),
        target.id.clone().into(),
        "test".into(),
        file.as_os_str().to_owned(),
    ]
}

/// Arguments for `maestro hierarchy`, which prints the screen's view
/// hierarchy.
pub fn maestro_hierarchy_args(target: &Target) -> Vec<OsString> {
    vec![
        "--device".into(),
        target.id.clone().into(),
        "hierarchy".into(),
    ]
}

/// Arguments for `maestro-runner hierarchy`, which prints the hierarchy as a
/// tree normalized across its drivers.
pub fn runner_hierarchy_args(target: &Target) -> Vec<OsString> {
    vec![
        "--platform".into(),
        target.class.platform().id().into(),
        "--device".into(),
        target.id.clone().into(),
        "hierarchy".into(),
    ]
}

/// Reads what `maestro hierarchy` prints: a tree of nodes, each with its
/// `attributes` and `children`. An element's identifier is its `resource-id`;
/// its text is `text`, else `accessibilityText`, which is where Maestro puts
/// the label on iOS when an element has no title or value. Every node counts.
pub fn read_hierarchy(stdout: &str) -> Result<Reading, String> {
    let tree = json_document(stdout).ok_or_else(|| no_tree("maestro", stdout))?;
    Ok(read_tree(&tree, |node| {
        let attributes = &node["attributes"];
        let text = ["text", "accessibilityText"]
            .into_iter()
            .filter_map(|key| attributes[key].as_str())
            .find(|text| !text.is_empty());
        (attributes["resource-id"].as_str(), text)
    }))
}

/// Reads what `maestro-runner hierarchy` prints: a tree of nodes with `id`,
/// `text` and `children`. Every node counts.
pub fn read_runner_hierarchy(stdout: &str) -> Result<Reading, String> {
    let tree = json_document(stdout).ok_or_else(|| no_tree("maestro-runner", stdout))?;
    Ok(read_tree(&tree, |node| {
        (
            node["id"].as_str(),
            node["text"].as_str().filter(|text| !text.is_empty()),
        )
    }))
}

/// Every node of `tree` in document order, with the identifier and text
/// `fields` gives it. An identifier loses the `<package>:id/` prefix Android
/// gives a native view's resource id.
fn read_tree<'a>(
    tree: &'a Value,
    fields: impl Fn(&'a Value) -> (Option<&'a str>, Option<&'a str>),
) -> Reading {
    let mut reading = Reading::default();
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        let (id, text) = fields(node);
        let id = id
            .filter(|id| !id.is_empty())
            .map(|id| id.split_once(":id/").map_or(id, |(_, name)| name));
        reading.elements.push(Element::new(id, text));
        if let Some(children) = node["children"].as_array() {
            stack.extend(children.iter().rev());
        }
    }
    reading.nodes = u32::try_from(reading.elements.len()).unwrap_or(u32::MAX);
    reading
}

fn no_tree(tool: &str, stdout: &str) -> String {
    match last_line(stdout) {
        Some(line) => format!("{tool} printed no hierarchy: {line}"),
        None => format!("{tool} printed no hierarchy"),
    }
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
            golden: Default::default(),
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
            golden: Default::default(),
        };
        let yaml = flow_yaml(&flow, "dev.mobileagent.fixture.expo");
        assert!(yaml.contains(r#"    text: "Row 1\\.5""#), "{yaml}");
        assert!(yaml.contains(r#"- inputText: "a.b \"c\"""#), "{yaml}");
    }

    fn element(id: Option<&str>, text: Option<&str>) -> Element {
        Element::new(id, text)
    }

    #[test]
    fn hierarchy_reads_name_the_device_and_maestro_runner_the_platform_too() {
        let strings = |args: Vec<OsString>| -> Vec<String> {
            args.into_iter().map(|a| a.into_string().unwrap()).collect()
        };
        let emulator = Target::new("emulator-5554", TargetClass::AndroidEmulator);
        let sim = Target::new("SIM-1", TargetClass::Simulator);
        assert_eq!(
            strings(maestro_hierarchy_args(&emulator)),
            ["--device", "emulator-5554", "hierarchy"]
        );
        assert_eq!(
            strings(runner_hierarchy_args(&sim)),
            ["--platform", "ios", "--device", "SIM-1", "hierarchy"]
        );
    }

    #[test]
    fn maestro_s_android_hierarchy_gives_resource_ids_texts_and_its_node_count() {
        let hierarchy = r#"{
  "attributes" : { },
  "children" : [ {
    "attributes" : { "text" : "", "resource-id" : "", "bounds" : "[0,0][1080,2400]", "class" : "android.widget.FrameLayout" },
    "children" : [ {
      "attributes" : { "text" : "Welcome, agent", "resource-id" : "login-status", "bounds" : "[42,735][1038,798]", "class" : "android.widget.TextView" },
      "children" : [ ],
      "clickable" : false,
      "enabled" : true
    }, {
      "attributes" : { "accessibilityText" : "Sign in", "resource-id" : "dev.mobileagent.fixture.compose:id/login-submit", "class" : "android.view.View" },
      "clickable" : true
    } ]
  } ]
}"#;
        assert_eq!(
            read_hierarchy(hierarchy),
            Ok(Reading {
                elements: vec![
                    element(None, None),
                    element(None, None),
                    element(Some("login-status"), Some("Welcome, agent")),
                    element(Some("login-submit"), Some("Sign in")),
                ],
                nodes: 4,
            })
        );
    }

    #[test]
    fn maestro_s_ios_hierarchy_gives_labels_where_the_text_is_empty() {
        let hierarchy = r#"Running on SIM-1
{
  "attributes" : { "accessibilityText" : "FixtureExpo", "title" : "", "value" : "", "text" : "", "hintText" : "", "resource-id" : "" },
  "children" : [ {
    "attributes" : { "accessibilityText" : "Welcome, agent", "title" : "", "value" : "", "text" : "", "hintText" : "", "resource-id" : "login-status" }
  }, {
    "attributes" : { "accessibilityText" : "Username", "title" : "", "value" : "agent", "text" : "agent", "hintText" : "Username", "resource-id" : "login-username" }
  } ]
}"#;
        assert_eq!(
            read_hierarchy(hierarchy),
            Ok(Reading {
                elements: vec![
                    element(None, Some("FixtureExpo")),
                    element(Some("login-status"), Some("Welcome, agent")),
                    element(Some("login-username"), Some("agent")),
                ],
                nodes: 3,
            })
        );
        let error = read_hierarchy("Error: no device connected\n").unwrap_err();
        assert!(error.contains("no device connected"), "{error}");
    }

    #[test]
    fn maestro_runner_s_hierarchy_gives_its_normalized_ids_and_texts() {
        let hierarchy = r#"{
  "type": "Root",
  "children": [
    {
      "type": "FrameLayout",
      "bounds": {"x": 0, "y": 0, "width": 1080, "height": 2400},
      "children": [
        {"type": "TextView", "id": "login-status", "text": "Welcome, agent", "bounds": {"x": 42, "y": 735, "width": 996, "height": 63}},
        {"type": "View", "id": "dev.mobileagent.fixture.compose:id/login-submit", "text": "Sign in", "enabled": false}
      ]
    }
  ]
}"#;
        assert_eq!(
            read_runner_hierarchy(hierarchy),
            Ok(Reading {
                elements: vec![
                    element(None, None),
                    element(None, None),
                    element(Some("login-status"), Some("Welcome, agent")),
                    element(Some("login-submit"), Some("Sign in")),
                ],
                nodes: 4,
            })
        );
        assert!(read_runner_hierarchy("(no elements)").is_err());
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
