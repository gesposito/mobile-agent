//! The flow description: steps written once and executed through every
//! adapter, so tools are compared on the same work. Selectors address the
//! identifiers and labels `fixtures/CONTRACT.md` fixes for every fixture.
//!
//! As JSON a flow reads `{"name": "login", "fixture": "expo", "steps":
//! ["launch", {"tap": {"id": "login-submit"}}], "golden": {"texts":
//! {"login-status": "Welcome, agent"}}}`.

use serde::{Deserialize, Serialize};

use crate::end_state::EndState;
use crate::fixture::Fixture;

/// The name of [`login`].
pub const LOGIN: &str = "login";
/// The name of [`list_scroll`].
pub const LIST_SCROLL: &str = "list-scroll";
/// The name of [`big_screen`].
pub const BIG_SCREEN: &str = "big-screen";

/// The benchmark-harness spec's three flows on `fixture`, in report order:
/// [`login`], [`list_scroll`] and [`big_screen`]. Every fixture implements the
/// same contract, so the flows differ only in the app they drive.
pub fn reference(fixture: Fixture) -> Vec<Flow> {
    vec![login(fixture), list_scroll(fixture), big_screen(fixture)]
}

/// Signs in: fills `login-username` with `agent` and `login-password` with
/// `hunter2`, taps `login-submit` and waits until `login-status` shows
/// `Welcome, agent`, the text its golden requires.
pub fn login(fixture: Fixture) -> Flow {
    Flow {
        name: LOGIN.to_owned(),
        fixture,
        steps: vec![
            Step::Launch,
            Step::Fill {
                target: Selector::id("login-username"),
                text: "agent".into(),
            },
            Step::Fill {
                target: Selector::id("login-password"),
                text: "hunter2".into(),
            },
            Step::Tap(Selector::id("login-submit")),
            Step::AssertText {
                id: "login-status".into(),
                text: "Welcome, agent".into(),
            },
        ],
        count_nodes: false,
        golden: EndState::default().with_text("login-status", "Welcome, agent"),
    }
}

/// Scrolls the 200-row list to its last row and opens it: taps `tab-list`,
/// scrolls down until `row-200` is visible, taps it and waits until
/// `detail-200` shows `Detail 200`. Its golden requires `detail-200`.
pub fn list_scroll(fixture: Fixture) -> Flow {
    Flow {
        name: LIST_SCROLL.to_owned(),
        fixture,
        steps: vec![
            Step::Launch,
            Step::Tap(Selector::id("tab-list")),
            Step::ScrollTo {
                target: Selector::id("row-200"),
                direction: Direction::Down,
            },
            Step::Tap(Selector::id("row-200")),
            Step::AssertText {
                id: "detail-200".into(),
                text: "Detail 200".into(),
            },
        ],
        count_nodes: false,
        golden: EndState::default().with_present("detail-200"),
    }
}

/// Opens the 500-cell screen: taps `tab-big` and waits until `cell-500` is
/// visible. The screen's node count is recorded outside the flow's timing,
/// and its golden requires all 500 cells.
pub fn big_screen(fixture: Fixture) -> Flow {
    Flow {
        name: BIG_SCREEN.to_owned(),
        fixture,
        steps: vec![
            Step::Launch,
            Step::Tap(Selector::id("tab-big")),
            Step::AssertVisible(Selector::id("cell-500")),
        ],
        count_nodes: true,
        golden: EndState::default().with_count("cell", 500),
    }
}

/// A flow: one app, a fixed sequence of steps, whether the node count of the
/// screen it ends on is recorded, and what that screen must show.
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
    /// What the final screen shows when the flow did its work. A tool's run of
    /// the flow is timed only once its reading of that screen matches. Empty
    /// for a flow without a golden, which is timed unchecked.
    #[serde(default, skip_serializing_if = "EndState::is_empty")]
    pub golden: EndState,
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
                golden: EndState::default(),
            }
        );
        let json = serde_json::to_string(&flow).unwrap();
        assert_eq!(serde_json::from_str::<Flow>(&json).unwrap(), flow);
    }
}
