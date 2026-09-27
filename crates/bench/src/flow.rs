//! The flow description: steps written once and executed through every
//! adapter, so tools are compared on the same work. Selectors address the
//! identifiers and labels `fixtures/CONTRACT.md` fixes for every fixture.
//!
//! As JSON a flow reads
//! `{"name": "login", "fixture": "expo", "steps": ["launch", {"tap": {"id": "login-submit"}}]}`.

use serde::{Deserialize, Serialize};

use crate::fixture::Fixture;

/// A flow: one app, a fixed sequence of steps, and whether the node count of
/// the screen it ends on is recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    /// Stable name used in reports, such as `login`.
    pub name: String,
    /// The app the flow drives, identified per platform by
    /// [`Fixture::app_id`].
    pub fixture: Fixture,
    pub steps: Vec<Step>,
    /// Record the node count of the final screen, outside the flow's timing,
    /// as the 500-node flow does.
    #[serde(default)]
    pub count_nodes: bool,
}

/// One step. A flow's timing covers all of its steps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Step {
    /// Stop the app if it runs, then launch it. It opens on the Login tab, so
    /// a flow reaches other screens by tapping their tabs.
    Launch,
    /// Tap the element.
    Tap(Selector),
    /// Type `text` into the field, which is empty: every field is after a
    /// launch.
    Fill { target: Selector, text: String },
    /// Scroll in `direction` until the element is visible.
    ScrollTo {
        target: Selector,
        direction: Direction,
    },
    /// Wait, within the tool's usual timeout, until the element is visible.
    AssertVisible(Selector),
    /// Wait until the element with identifier `id` shows exactly `text`.
    AssertText { id: String, text: String },
}

/// How a step finds its element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Selector {
    /// The contract identifier: `accessibilityIdentifier`, `testID` or
    /// `testTag`.
    Id(String),
    /// The exact visible text or label.
    Text(String),
}

impl Selector {
    pub fn id(id: impl Into<String>) -> Self {
        Selector::Id(id.into())
    }

    pub fn text(text: impl Into<String>) -> Self {
        Selector::Text(text.into())
    }
}

/// A scroll direction: the direction the content moves into view from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Down,
    Up,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flow_reads_from_its_documented_json_shape() {
        let flow: Flow = serde_json::from_str(
            r#"{"name": "list-scroll", "fixture": "compose", "steps": [
                "launch",
                {"tap": {"id": "tab-list"}},
                {"scroll-to": {"target": {"id": "row-200"}, "direction": "down"}},
                {"tap": {"text": "Row 200"}},
                {"fill": {"target": {"id": "login-username"}, "text": "agent"}},
                {"assert-visible": {"id": "detail-200"}},
                {"assert-text": {"id": "detail-200", "text": "Detail 200"}}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            flow,
            Flow {
                name: "list-scroll".into(),
                fixture: Fixture::Compose,
                steps: vec![
                    Step::Launch,
                    Step::Tap(Selector::id("tab-list")),
                    Step::ScrollTo {
                        target: Selector::id("row-200"),
                        direction: Direction::Down,
                    },
                    Step::Tap(Selector::text("Row 200")),
                    Step::Fill {
                        target: Selector::id("login-username"),
                        text: "agent".into(),
                    },
                    Step::AssertVisible(Selector::id("detail-200")),
                    Step::AssertText {
                        id: "detail-200".into(),
                        text: "Detail 200".into(),
                    },
                ],
                count_nodes: false,
            }
        );
        let json = serde_json::to_string(&flow).unwrap();
        assert_eq!(serde_json::from_str::<Flow>(&json).unwrap(), flow);
    }
}
