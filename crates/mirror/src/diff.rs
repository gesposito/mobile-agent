//! The structural diff between a baseline snapshot and the current one
//! (spec: Structural diff): elements matched by identity, listed as added,
//! removed or changed with the fields that changed.

use std::collections::{HashMap, HashSet};

use crate::json;
use crate::render::element_line;
use crate::snapshot::{Element, ElementId, Snapshot};

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
    pub fn between(baseline: &Snapshot, current: &Snapshot) -> Diff {
        let before: HashMap<ElementId, &Element> =
            baseline.elements.iter().map(|e| (e.id, e)).collect();
        let after: HashSet<ElementId> = current.elements.iter().map(|e| e.id).collect();
        let mut added = Vec::new();
        let mut changed = Vec::new();
        for element in &current.elements {
            match before.get(&element.id) {
                None => added.push(element.clone()),
                Some(old) => {
                    let fields = changed_fields(old, element);
                    if !fields.is_empty() {
                        changed.push(Change {
                            before: (*old).clone(),
                            after: element.clone(),
                            fields,
                        });
                    }
                }
            }
        }
        let removed = baseline
            .elements
            .iter()
            .filter(|e| !after.contains(&e.id))
            .cloned()
            .collect();
        Diff {
            baseline_revision: baseline.revision,
            revision: current.revision,
            added,
            removed,
            changed,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// The diff as text: `added:`, `removed:` and `changed:` sections of
    /// snapshot lines, each change followed by its fields.
    pub fn text(&self) -> String {
        if self.is_empty() {
            return "(no changes)".to_owned();
        }
        let mut lines = Vec::new();
        for (title, elements) in [("added:", &self.added), ("removed:", &self.removed)] {
            if !elements.is_empty() {
                lines.push(title.to_owned());
                lines.extend(elements.iter().map(section_line));
            }
        }
        if !self.changed.is_empty() {
            lines.push("changed:".to_owned());
            for change in &self.changed {
                lines.push(section_line(&change.after));
                for field in &change.fields {
                    lines.push(format!(
                        "    {}: {} -> {}",
                        field.name(),
                        show(&change.before, *field),
                        show(&change.after, *field)
                    ));
                }
            }
        }
        lines.join("\n")
    }
}

/// An element as a line inside a section.
fn section_line(element: &Element) -> String {
    element_line(&Element {
        depth: 1,
        ..element.clone()
    })
}

fn changed_fields(before: &Element, after: &Element) -> Vec<Field> {
    [
        (Field::Role, before.role != after.role),
        (Field::Name, before.name != after.name),
        (Field::Value, before.value != after.value),
        (Field::Identifier, before.identifier != after.identifier),
        (Field::Enabled, before.enabled != after.enabled),
        (Field::Focused, before.focused != after.focused),
        (Field::Selected, before.selected != after.selected),
        (Field::Checked, before.checked != after.checked),
    ]
    .into_iter()
    .filter_map(|(field, differs)| differs.then_some(field))
    .collect()
}

/// A field's value as the diff prints it.
fn show(element: &Element, field: Field) -> String {
    let optional = |value: Option<String>| value.unwrap_or_else(|| "none".to_owned());
    match field {
        Field::Role => element.role.to_string(),
        Field::Name => json::string(&element.name),
        Field::Value => optional(element.value.as_deref().map(json::string)),
        Field::Identifier => optional(element.identifier.clone()),
        Field::Enabled => element.enabled.to_string(),
        Field::Focused => element.focused.to_string(),
        Field::Selected => element.selected.to_string(),
        Field::Checked => optional(element.checked.map(|c| c.to_string())),
    }
}
