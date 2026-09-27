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
    /// Text the element's name or value contains.
    pub text: Option<String>,
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!("task 7.5")
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
}

impl Condition {
    pub fn name(self) -> &'static str {
        match self {
            Condition::Appear => "appear",
            Condition::Disappear => "disappear",
            Condition::Enabled => "become enabled",
        }
    }
}

/// How a wait ended.
#[derive(Clone, Debug, PartialEq)]
pub struct WaitOutcome {
    /// From the start of the wait until the condition held.
    pub elapsed: Duration,
    /// The element that matched, for `Appear` and `Enabled`.
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
        let _ = f;
        todo!("task 7.5")
    }
}
