//! End states: what the screen shows once a flow has run, in the few terms
//! that tell whether the flow did its work. A flow's golden names the texts
//! elements must show, the elements that must be present and the numbered
//! elements to count; a tool's reading of its final screen is summarized in
//! the same terms, so every tool compares with the same golden.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// One element of a tool's reading of a screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    /// The contract identifier, when the element has one.
    pub id: Option<String>,
    /// The text the element shows, as the tool reports it.
    pub text: Option<String>,
}

impl Element {
    pub fn new(id: Option<&str>, text: Option<&str>) -> Self {
        Element {
            id: id.map(str::to_owned),
            text: text.map(str::to_owned),
        }
    }
}

/// A tool's reading of the screen: its elements, and how many nodes its
/// output held, which is the node count the report records for the tool.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub elements: Vec<Element>,
    pub nodes: u32,
}

/// A summary of a screen. As a golden it states what the screen must show;
/// as a reading, what it showed, under the golden's keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndState {
    /// Identifier to the text of the element that has it; `None` when no
    /// element has it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub texts: BTreeMap<String, Option<String>>,
    /// Identifier to whether an element has it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub present: BTreeMap<String, bool>,
    /// Prefix to how many distinct identifiers of the form `<prefix>-<n>`,
    /// `n` a number, elements have: `cell` counts `cell-1` to `cell-500`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, u32>,
}

impl EndState {
    /// This state, also stating that the element `id` shows `text`.
    pub fn with_text(mut self, id: &str, text: &str) -> Self {
        self.texts.insert(id.to_owned(), Some(text.to_owned()));
        self
    }

    /// This state, also stating that an element has the identifier `id`.
    pub fn with_present(mut self, id: &str) -> Self {
        self.present.insert(id.to_owned(), true);
        self
    }

    /// This state, also stating that `count` distinct identifiers
    /// `<prefix>-<n>` are on the screen.
    pub fn with_count(mut self, prefix: &str, count: u32) -> Self {
        self.counts.insert(prefix.to_owned(), count);
        self
    }

    /// Whether the state states nothing, as the golden of a flow that is
    /// timed unchecked.
    pub fn is_empty(&self) -> bool {
        self.texts.is_empty() && self.present.is_empty() && self.counts.is_empty()
    }

    /// `elements`, a tool's reading of a screen, summarized in this state's
    /// terms.
    pub fn observe(&self, elements: &[Element]) -> EndState {
        let _ = elements;
        EndState::default()
    }

    /// How `observed` differs from this state, one phrase per difference;
    /// empty when they match.
    pub fn differences(&self, observed: &EndState) -> Vec<String> {
        let _ = observed;
        Vec::new()
    }
}

impl fmt::Display for EndState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Vec<Element> {
        vec![
            Element::new(None, Some("FixtureExpo")),
            Element::new(Some("login-status"), Some("Welcome, agent")),
            Element::new(Some("detail-200"), None),
            Element::new(Some("detail-200"), Some("Detail 200")),
            Element::new(Some("cell-1"), Some("Cell 1")),
            Element::new(Some("cell-2"), Some("Cell 2")),
            // One identifier on two elements counts once; identifiers that are
            // not the prefix, a hyphen and a number do not count.
            Element::new(Some("cell-2"), None),
            Element::new(Some("cell-x"), None),
            Element::new(Some("cells-3"), None),
            Element::new(Some("cell-"), None),
            Element::new(Some("big-cell-4"), None),
        ]
    }

    #[test]
    fn a_reading_is_summarized_in_the_golden_s_terms() {
        let golden = EndState::default()
            .with_text("login-status", "Welcome, agent")
            .with_text("detail-200", "Detail 200")
            .with_text("login-error", "Invalid credentials")
            .with_present("detail-200")
            .with_present("detail-199")
            .with_count("cell", 500);
        let observed = golden.observe(&screen());
        assert_eq!(
            observed,
            EndState {
                texts: [
                    ("detail-200".to_owned(), Some("Detail 200".to_owned())),
                    ("login-error".to_owned(), None),
                    ("login-status".to_owned(), Some("Welcome, agent".to_owned())),
                ]
                .into(),
                present: [
                    ("detail-199".to_owned(), false),
                    ("detail-200".to_owned(), true)
                ]
                .into(),
                counts: [("cell".to_owned(), 2)].into(),
            }
        );
        assert_eq!(EndState::default().observe(&screen()), EndState::default());
    }

    #[test]
    fn an_element_that_shows_no_text_shows_the_empty_text() {
        let golden = EndState::default().with_text("login-username", "agent");
        let observed = golden.observe(&[Element::new(Some("login-username"), None)]);
        assert_eq!(observed.texts["login-username"].as_deref(), Some(""));
    }

    #[test]
    fn differences_name_each_term_that_does_not_match() {
        let golden = EndState::default()
            .with_text("login-status", "Welcome, agent")
            .with_present("detail-200")
            .with_count("cell", 500);
        assert!(golden.differences(&golden).is_empty());
        let observed = EndState {
            texts: [(
                "login-status".to_owned(),
                Some("Invalid credentials".to_owned()),
            )]
            .into(),
            present: [("detail-200".to_owned(), false)].into(),
            counts: [("cell".to_owned(), 499)].into(),
        };
        assert_eq!(
            golden.differences(&observed),
            [
                "login-status shows \"Invalid credentials\", not \"Welcome, agent\"",
                "detail-200 is absent",
                "499 cell-N identifiers, not 500",
            ]
        );
        let unread = EndState {
            texts: [("login-status".to_owned(), None)].into(),
            ..EndState::default()
        };
        assert_eq!(
            golden.differences(&unread),
            [
                "no element is login-status, which should show \"Welcome, agent\"",
                "detail-200 was not read",
                "cell-N identifiers were not counted",
            ]
        );
        let mut extra = golden.clone();
        extra.present.insert("detail-199".to_owned(), false);
        assert_eq!(
            golden.differences(&extra),
            ["detail-199 was read though the golden does not name it"]
        );
    }

    #[test]
    fn a_state_reads_as_its_terms() {
        let golden = EndState::default()
            .with_text("login-status", "Welcome, agent")
            .with_present("detail-200")
            .with_count("cell", 500);
        assert_eq!(
            golden.to_string(),
            "login-status shows \"Welcome, agent\"; detail-200 present; 500 cell-N"
        );
        let absent = EndState {
            texts: [("login-status".to_owned(), None)].into(),
            present: [("detail-200".to_owned(), false)].into(),
            ..EndState::default()
        };
        assert_eq!(absent.to_string(), "no login-status; detail-200 absent");
        assert_eq!(EndState::default().to_string(), "nothing");
    }
}
