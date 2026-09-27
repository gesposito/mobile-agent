//! agent-browser's delta snapshots (`snapshot --delta`): a revision number
//! per listing, and against the previous one either the whole listing, an
//! `unchanged` marker, or the ref changes plus a line splice that rebuilds
//! the new listing from the old one, falling back to the whole listing when
//! the delta is 70% of its size or more.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::backend::Owner;
use crate::json;
use crate::refs::Ref;
use crate::snapshot::{Role, SnapshotOptions};

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
        let mut out = String::new();
        match self {
            DeltaSnapshot::Full {
                revision,
                tree,
                refs,
            } => {
                out.push_str(r#"{"kind":"full","refs":"#);
                refs_json(&mut out, refs);
                let _ = write!(out, r#","revision":{revision},"tree":"#);
                json::push_str(&mut out, tree);
                out.push('}');
            }
            DeltaSnapshot::Unchanged {
                revision,
                base_revision,
            } => {
                let _ = write!(
                    out,
                    r#"{{"baseRevision":{base_revision},"kind":"unchanged","revision":{revision}}}"#
                );
            }
            DeltaSnapshot::Delta {
                revision,
                base_revision,
                changes,
                tree_change,
            } => {
                let _ = write!(out, r#"{{"baseRevision":{base_revision},"changes":["#);
                for (i, change) in changes.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    change_json(&mut out, change);
                }
                let _ = write!(
                    out,
                    r#"],"kind":"delta","revision":{revision},"treeChange":{{"deleteCount":{},"lines":["#,
                    tree_change.delete_count
                );
                for (i, line) in tree_change.lines.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    json::push_str(&mut out, line);
                }
                let _ = write!(out, r#"],"startLine":{}}}}}"#, tree_change.start_line);
            }
        }
        out
    }
}

/// The ref's key as agent-browser's `refs` map sorts it: by its text.
fn key(reference: Ref) -> String {
    reference.to_string()
}

fn refs_json(out: &mut String, refs: &[RefEntry]) {
    let mut sorted: Vec<&RefEntry> = refs.iter().collect();
    sorted.sort_by_key(|entry| key(entry.reference));
    out.push('{');
    for (i, entry) in sorted.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json::push_str(out, &key(entry.reference));
        out.push(':');
        node_json(out, entry);
    }
    out.push('}');
}

fn node_json(out: &mut String, entry: &RefEntry) {
    out.push_str(r#"{"name":"#);
    json::push_str(out, &entry.name);
    out.push_str(r#","role":"#);
    json::push_str(out, entry.role.as_str());
    out.push('}');
}

fn change_json(out: &mut String, change: &RefChange) {
    match change {
        RefChange::Add(entry) => {
            out.push_str(r#"{"node":"#);
            node_json(out, entry);
            out.push_str(r#","op":"add","ref":"#);
            json::push_str(out, &format!("@{}", entry.reference));
            out.push('}');
        }
        RefChange::Remove(reference) => {
            out.push_str(r#"{"op":"remove","ref":"#);
            json::push_str(out, &format!("@{reference}"));
            out.push('}');
        }
        RefChange::Replace {
            reference,
            field,
            value,
        } => {
            let field = match field {
                RefField::Role => "role",
                RefField::Name => "name",
            };
            let _ = write!(out, r#"{{"field":"{field}","op":"replace","ref":"#);
            json::push_str(out, &format!("@{reference}"));
            out.push_str(r#","value":"#);
            json::push_str(out, value);
            out.push('}');
        }
    }
}

/// What a delta snapshot compares against: the previous listing with the
/// same options and context.
pub(crate) struct DeltaState {
    pub revision: u64,
    pub context: DeltaContext,
    pub tree: String,
    pub refs: Vec<RefEntry>,
}

/// What must stay the same for a delta: the listing options, the replica's
/// generation (a resync starts over) and who owns the foreground, as
/// agent-browser starts over when the options or the page's URL change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeltaContext {
    pub options: SnapshotOptions,
    pub generation: u64,
    pub foreground: Owner,
}

/// The delta snapshot `revision` against `previous`.
pub(crate) fn against(
    previous: &DeltaState,
    revision: u64,
    tree: &str,
    refs: &[RefEntry],
) -> DeltaSnapshot {
    if previous.tree == tree {
        return DeltaSnapshot::Unchanged {
            revision,
            base_revision: previous.revision,
        };
    }
    let current: HashMap<Ref, &RefEntry> = refs.iter().map(|e| (e.reference, e)).collect();
    let mut before: Vec<&RefEntry> = previous.refs.iter().collect();
    before.sort_by_key(|entry| key(entry.reference));
    let mut changes = Vec::new();
    for old in before {
        match current.get(&old.reference) {
            None => changes.push(RefChange::Remove(old.reference)),
            Some(new) => {
                if old.role != new.role {
                    changes.push(RefChange::Replace {
                        reference: old.reference,
                        field: RefField::Role,
                        value: new.role.as_str().to_owned(),
                    });
                }
                if old.name != new.name {
                    changes.push(RefChange::Replace {
                        reference: old.reference,
                        field: RefField::Name,
                        value: new.name.clone(),
                    });
                }
            }
        }
    }
    let known: HashMap<Ref, ()> = previous.refs.iter().map(|e| (e.reference, ())).collect();
    let mut after: Vec<&RefEntry> = refs.iter().collect();
    after.sort_by_key(|entry| key(entry.reference));
    for new in after {
        if !known.contains_key(&new.reference) {
            changes.push(RefChange::Add(new.clone()));
        }
    }
    let old_lines: Vec<&str> = previous.tree.split('\n').collect();
    let new_lines: Vec<&str> = tree.split('\n').collect();
    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let delta = DeltaSnapshot::Delta {
        revision,
        base_revision: previous.revision,
        changes,
        tree_change: TreeSplice {
            start_line: prefix,
            delete_count: old_lines.len() - prefix - suffix,
            lines: new_lines[prefix..new_lines.len() - suffix]
                .iter()
                .map(|l| (*l).to_owned())
                .collect(),
        },
    };
    if delta.to_json().len() >= tree.len().saturating_mul(7) / 10 {
        DeltaSnapshot::Full {
            revision,
            tree: tree.to_owned(),
            refs: refs.to_vec(),
        }
    } else {
        delta
    }
}
