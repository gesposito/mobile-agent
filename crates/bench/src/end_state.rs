//! End states: what the screen shows once a flow has run, in the few terms
//! that tell whether the flow did its work. A flow's golden names the texts
//! elements must show, the elements that must be present and the numbered
//! elements to count; a tool's reading of its final screen is summarized in
//! the same terms, so every tool compares with the same golden.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
}
