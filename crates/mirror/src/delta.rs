//! agent-browser's delta snapshots (`snapshot --delta`): a revision number
//! per listing, and against the previous one either the whole listing, an
//! `unchanged` marker, or the ref changes plus a line splice that rebuilds
//! the new listing from the old one, falling back to the whole listing when
//! the delta is 70% of its size or more.

use crate::refs::Ref;
use crate::snapshot::Role;

/// One delta snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeltaSnapshot {
    Full {
        revision: u64,
        tree: String,
        refs: Vec<RefEntry>,
    },
    Unchanged {
        revision: u64,
        base_revision: u64,
    },
    Delta {
        revision: u64,
        base_revision: u64,
        changes: Vec<RefChange>,
        tree_change: TreeSplice,
    },
}

/// A ref and what it names, as agent-browser's `refs` map holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefEntry {
    pub reference: Ref,
    pub role: Role,
    pub name: String,
}

/// A change to the refs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefChange {
    Add(RefEntry),
    Remove(Ref),
    /// The ref's role or name changed.
    Replace {
        reference: Ref,
        field: RefField,
        value: String,
    },
}

/// The fields agent-browser's ref metadata holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefField {
    Role,
    Name,
}

/// Replace `delete_count` lines from `start_line` of the old listing with
/// `lines`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeSplice {
    pub start_line: usize,
    pub delete_count: usize,
    pub lines: Vec<String>,
}

impl DeltaSnapshot {
    /// `full`, `unchanged` or `delta`.
    pub fn kind(&self) -> &'static str {
        match self {
            DeltaSnapshot::Full { .. } => "full",
            DeltaSnapshot::Unchanged { .. } => "unchanged",
            DeltaSnapshot::Delta { .. } => "delta",
        }
    }

    pub fn revision(&self) -> u64 {
        match self {
            DeltaSnapshot::Full { revision, .. }
            | DeltaSnapshot::Unchanged { revision, .. }
            | DeltaSnapshot::Delta { revision, .. } => *revision,
        }
    }

    /// The JSON agent-browser prints under `snapshot`, byte for byte:
    /// compact, keys in `serde_json`'s sorted order.
    pub fn to_json(&self) -> String {
        todo!("task 7.6")
    }
}
