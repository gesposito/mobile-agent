//! Reading snapshots: nodes with their window and child-index path, resource
//! ids joined with their string-table prefix, class names.

use protocol::snapshot::{Bounds, Node, NodeFlags, Snapshot, Window};

/// A node of a snapshot with where it sits.
#[derive(Clone, Copy)]
pub struct Found<'a> {
    pub window: &'a Window,
    pub node: &'a Node,
    /// Levels below the window root, which is at depth 0.
    pub depth: usize,
}

/// Every node, window by window, depth first, with its child-index path.
pub fn nodes(s: &Snapshot) -> Vec<(Found<'_>, Vec<u32>)> {
    let mut out = Vec::new();
    for window in &s.windows {
        if let Some(root) = &window.root {
            let mut stack = vec![(root, Vec::new())];
            while let Some((node, path)) = stack.pop() {
                for (i, child) in node.children.iter().enumerate().rev() {
                    let mut p = path.clone();
                    p.push(i as u32);
                    stack.push((child, p));
                }
                let depth = path.len();
                out.push((
                    Found {
                        window,
                        node,
                        depth,
                    },
                    path,
                ));
            }
        }
    }
    out
}

/// Nodes serialized in the snapshot.
pub fn count(s: &Snapshot) -> usize {
    nodes(s).len()
}

/// Serialized nodes the platform reports visible to the user.
pub fn visible(s: &Snapshot) -> usize {
    nodes(s)
        .iter()
        .filter(|(f, _)| f.node.flags.contains(NodeFlags::VISIBLE))
        .count()
}

/// The full resource id of `n`: its string-table prefix and the rest.
pub fn resource_id(s: &Snapshot, n: &Node) -> Option<String> {
    let rest = n.res_id.as_deref()?;
    let prefix = n
        .res_pkg
        .and_then(|i| s.strings.get(i as usize))
        .map_or("", String::as_str);
    Some(format!("{prefix}{rest}"))
}

/// The class name of `n`.
pub fn class<'a>(s: &'a Snapshot, n: &Node) -> &'a str {
    s.strings.get(n.class as usize).map_or("", String::as_str)
}

/// The package of `w`, if the window names one.
pub fn package<'a>(s: &'a Snapshot, w: &Window) -> Option<&'a str> {
    w.package
        .and_then(|i| s.strings.get(i as usize))
        .map(String::as_str)
}

/// The first node whose resource id is `id`, with its path.
pub fn find<'a>(s: &'a Snapshot, id: &str) -> Option<(Found<'a>, Vec<u32>)> {
    nodes(s)
        .into_iter()
        .find(|(f, _)| resource_id(s, f.node).as_deref() == Some(id))
}

/// Every node whose resource id starts with `prefix`.
pub fn find_all<'a>(s: &'a Snapshot, prefix: &str) -> Vec<Found<'a>> {
    nodes(s)
        .into_iter()
        .filter(|(f, _)| resource_id(s, f.node).is_some_and(|r| r.starts_with(prefix)))
        .map(|(f, _)| f)
        .collect()
}

/// The node at `path` below the root of window `window`.
pub fn at_path<'a>(s: &'a Snapshot, window: u32, path: &[u32]) -> Option<&'a Node> {
    let mut node = s.windows.iter().find(|w| w.id == window)?.root.as_ref()?;
    for &i in path {
        node = node.children.get(i as usize)?;
    }
    Some(node)
}

/// The text of `n` and of its descendants, depth first.
pub fn texts(n: &Node) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![n];
    while let Some(node) = stack.pop() {
        if let Some(t) = &node.text {
            out.push(t.clone());
        }
        stack.extend(node.children.iter().rev());
    }
    out
}

pub fn center(b: Bounds) -> (i32, i32) {
    ((b.left + b.right) / 2, (b.top + b.bottom) / 2)
}

/// The window the user is interacting with.
pub fn active_window(s: &Snapshot) -> Option<&Window> {
    s.windows
        .iter()
        .find(|w| w.flags.contains(protocol::snapshot::WindowFlags::ACTIVE))
}
