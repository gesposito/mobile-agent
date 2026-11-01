//! Waits for an element to appear, disappear or become enabled (spec: Wait
//! predicates from the event stream). A wait re-evaluates when the target
//! reports something or a re-read falls due, never on a fixed sleep, and
//! fails at its timeout with a summary of the screen it last saw.

use core::fmt;
use std::time::Duration;

use crate::refs::Ref;
use crate::snapshot::{Element, Role};
use crate::verdict::Verdict;

/// The elements a wait looks for: every field present must match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selector {
    /// Text the element's name or value contains, or is when `exact`.
    pub text: Option<String>,
    /// Whether `text` is the element's whole name or value, as the CLI's
    /// `text==` asks.
    pub exact: bool,
    /// The element's exact test identifier.
    pub identifier: Option<String>,
    pub role: Option<Role>,
    /// The element a ref names.
    pub reference: Option<Ref>,
}

impl Selector {
    pub fn text(text: impl Into<String>) -> Selector {
        Selector {
            text: Some(text.into()),
            ..Selector::default()
        }
    }

    pub fn identifier(identifier: impl Into<String>) -> Selector {
        Selector {
            identifier: Some(identifier.into()),
            ..Selector::default()
        }
    }

    pub fn reference(reference: Ref) -> Selector {
        Selector {
            reference: Some(reference),
            ..Selector::default()
        }
    }

    /// This selector, also requiring `role`.
    pub fn with_role(self, role: Role) -> Selector {
        Selector {
            role: Some(role),
            ..self
        }
    }
}

impl fmt::Display for Selector {
    /// Such as `text "Welcome"`, `button with id login-submit` or `@e4`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(identifier) = &self.identifier {
            parts.push(format!("id {identifier}"));
        }
        if let Some(text) = &self.text {
            let kind = if self.exact { "exact text" } else { "text" };
            parts.push(format!("{kind} {text:?}"));
        }
        if let Some(reference) = self.reference {
            parts.push(format!("@{reference}"));
        }
        match (self.role, parts.is_empty()) {
            (Some(role), true) => write!(f, "{role}"),
            (Some(role), false) => write!(f, "{role} with {}", parts.join(" and ")),
            (None, true) => f.write_str("any element"),
            (None, false) => f.write_str(&parts.join(" and ")),
        }
    }
}

impl Selector {
    /// Whether an element with these facts matches; `element_ref` is the
    /// element's ref, if it has one.
    pub(crate) fn matches(
        &self,
        name: &str,
        value: Option<&str>,
        identifier: Option<&str>,
        role: Role,
        element_ref: Option<Ref>,
    ) -> bool {
        self.text.as_deref().is_none_or(|text| {
            if self.exact {
                name == text || value == Some(text)
            } else {
                name.contains(text) || value.is_some_and(|v| v.contains(text))
            }
        }) && self
            .identifier
            .as_deref()
            .is_none_or(|id| identifier == Some(id))
            && self.role.is_none_or(|r| r == role)
            && self.reference.is_none_or(|r| element_ref == Some(r))
    }

    /// Whether a matching element with this name and value holds a text
    /// that this selector's text, when the element's text need only contain
    /// it, is the whole of, case and the whitespace around either aside: a
    /// wait for the text prefers such an element to one whose text merely
    /// contains it, as `wait --text Lisbon` resolved on "Lisbon Airport
    /// (LIS)" listed above "Lisbon" on the fourth pass (pass-4-fixes,
    /// design D8). True for any other selector.
    pub(crate) fn whole(&self, name: &str, value: Option<&str>) -> bool {
        let Some(text) = self.text.as_deref().filter(|_| !self.exact) else {
            return true;
        };
        let text = text.trim().to_lowercase();
        let holds = |shown: &str| shown.trim().to_lowercase() == text;
        holds(name) || value.is_some_and(holds)
    }
}

/// What a wait waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Condition {
    /// A matching element is on screen.
    Appear,
    /// No matching element is on screen.
    Disappear,
    /// A matching element is on screen and enabled.
    Enabled,
    /// The replica holds a matching element, on screen or not, covered or
    /// not (rn-app-fixes, design D51).
    Attached,
    /// The replica holds no matching element.
    Detached,
}

impl Condition {
    pub fn name(self) -> &'static str {
        match self {
            Condition::Appear => "appear",
            Condition::Disappear => "disappear",
            Condition::Enabled => "become enabled",
            Condition::Attached => "be held in the tree",
            Condition::Detached => "leave the tree",
        }
    }
}

/// How a wait ended.
#[derive(Clone, Debug, PartialEq)]
pub struct WaitOutcome {
    /// From the start of the wait until the condition held.
    pub elapsed: Duration,
    /// The element that matched, for `Appear`, `Enabled` and `Attached`.
    pub element: Option<Element>,
}

/// The screen a wait saw last, for its timeout error.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotSummary {
    pub revision: u64,
    /// How long ago the replica was last read.
    pub age: Duration,
    pub windows: usize,
    /// Elements with refs on screen.
    pub elements: usize,
    /// The first lines of the interactive listing.
    pub lines: Vec<String>,
    /// Lines left out.
    pub omitted: usize,
    pub verdict: Verdict,
}

impl fmt::Display for SnapshotSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "revision {}, {} windows, {} elements with refs, read {} ms ago",
            self.revision,
            self.windows,
            self.elements,
            self.age.as_millis()
        )?;
        if self.verdict.degraded {
            let reasons: Vec<&str> = self.verdict.reasons.iter().map(|r| r.code()).collect();
            write!(f, ", degraded ({})", reasons.join(", "))?;
        }
        f.write_str(":")?;
        for line in &self.lines {
            write!(f, "\n{line}")?;
        }
        if self.omitted > 0 {
            write!(f, "\n({} more lines)", self.omitted)?;
        }
        Ok(())
    }
}
