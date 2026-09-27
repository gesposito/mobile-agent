//! What the replica's nodes look like to a user: role, accessible name,
//! value, bounds in points, visibility.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use protocol::snapshot::NodeFlags;

use crate::geometry::Rect;
use crate::replica::{RNode, Replica, Win};
use crate::roles;
use crate::snapshot::Role;

/// A node as snapshots present it.
#[derive(Clone, Debug)]
pub(crate) struct Facts {
    pub role: Role,
    pub name: String,
    pub value: Option<String>,
    pub rect: Rect,
    pub visible: bool,
    pub actionable: bool,
    pub checked: Option<bool>,
    /// Whether the element gets a ref: actionable, or named content.
    pub bears_ref: bool,
    /// The node's identity without sibling positions: its window, then the
    /// class, identifier and content of every node down to it. Two elements
    /// that share it are told apart only by where they sit.
    pub signature: u64,
}

/// The facts of every node of every window, parallel to the replica.
pub(crate) struct View {
    pub windows: Vec<Vec<Facts>>,
}

impl View {
    pub fn build(replica: &Replica) -> View {
        View {
            windows: replica
                .windows
                .iter()
                .map(|w| window_facts(w, replica.scale))
                .collect(),
        }
    }
}

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

fn window_facts(window: &Win, scale: f64) -> Vec<Facts> {
    let area = Rect::from_pixels(window.bounds, scale);
    let mut out: Vec<Facts> = Vec::with_capacity(window.nodes.len());
    for node in &window.nodes {
        let mut facts = node_facts(node, &area, scale);
        let mut h = DefaultHasher::new();
        match node.parent {
            Some(parent) => out[parent].signature.hash(&mut h),
            None => window.id.hash(&mut h),
        }
        node.class.hash(&mut h);
        node.identifier.hash(&mut h);
        node.content.hash(&mut h);
        facts.signature = h.finish();
        out.push(facts);
    }
    out
}

/// The facts of node `i` of window `w` alone.
pub(crate) fn facts_of(replica: &Replica, w: usize, i: usize) -> Facts {
    let window = &replica.windows[w];
    let area = Rect::from_pixels(window.bounds, replica.scale);
    node_facts(&window.nodes[i], &area, replica.scale)
}

fn node_facts(node: &RNode, area: &Rect, scale: f64) -> Facts {
    let text = non_empty(&node.text);
    let desc = non_empty(&node.desc);
    let role = roles::role(&node.class, node.flags, text.is_some() || desc.is_some());
    let editable = node.editable() || matches!(role, Role::Textbox | Role::Searchbox);
    let name = match (desc, text) {
        (Some(desc), _) => desc.to_owned(),
        (None, Some(text)) if !editable => text.to_owned(),
        _ => String::new(),
    };
    let value = match (desc, text) {
        (_, Some(text)) if editable => Some(text.to_owned()),
        (Some(desc), Some(text)) if text != desc => Some(text.to_owned()),
        _ => None,
    };
    let rect = Rect::from_pixels(node.bounds, scale);
    let visible = node.flags.contains(NodeFlags::VISIBLE) && rect.overlaps(area);
    let checkable = node.flags.contains(NodeFlags::CHECKABLE)
        || matches!(role, Role::Checkbox | Role::Switch | Role::Radio);
    let actionable = roles::acts(node.flags)
        || node.flags.contains(NodeFlags::CHECKABLE)
        || role.is_interactive();
    let bears_ref = actionable || (role.is_content() && !name.is_empty());
    Facts {
        role,
        name,
        value,
        rect,
        visible,
        actionable,
        checked: checkable.then(|| node.flags.contains(NodeFlags::CHECKED)),
        bears_ref,
        signature: 0,
    }
}
