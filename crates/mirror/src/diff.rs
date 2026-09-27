//! The structural diff between a baseline snapshot and the current one
//! (spec: Structural diff): elements matched by identity, listed as added,
//! removed or changed with the fields that changed.

use crate::snapshot::{Element, Snapshot};

/// What changed between two snapshots.
#[derive(Clone, Debug, PartialEq)]
pub struct Diff {
    pub baseline_revision: u64,
    pub revision: u64,
    pub added: Vec<Element>,
    pub removed: Vec<Element>,
    pub changed: Vec<Change>,
}

/// An element in both snapshots whose fields differ.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub before: Element,
    pub after: Element,
    pub fields: Vec<Field>,
}

/// A field a diff compares. Bounds are left out: scrolling moves every
/// element without changing any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Role,
    Name,
    Value,
    Identifier,
    Enabled,
    Focused,
    Selected,
    Checked,
}

impl Field {
    pub fn name(self) -> &'static str {
        match self {
            Field::Role => "role",
            Field::Name => "name",
            Field::Value => "value",
            Field::Identifier => "id",
            Field::Enabled => "enabled",
            Field::Focused => "focused",
            Field::Selected => "selected",
            Field::Checked => "checked",
        }
    }
}

impl Diff {
    /// The diff between `baseline` and `current`, elements matched by
    /// identity.
    pub fn between(_baseline: &Snapshot, _current: &Snapshot) -> Diff {
        todo!("task 7.6")
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// The diff as text: `added:`, `removed:` and `changed:` sections of
    /// snapshot lines, each change followed by its fields.
    pub fn text(&self) -> String {
        todo!("task 7.6")
    }
}
