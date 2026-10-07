//! What the replica's nodes look like to a user: role, accessible name,
//! value, bounds in points, visibility, and what covers them.
//!
//! Facts are computed per window in passes over the pre-order:
//!
//! 1. each node on its own: role from class and states, its own label,
//!    value, bounds, visibility;
//! 2. a tapped container that keeps the role on a child, as Material
//!    components in Compose do, takes the child's role and absorbs it;
//! 3. an actionable element with no label of its own is named by its
//!    non-actionable descendants, which it absorbs, the way accessibility
//!    services compute a name (design D5);
//! 4. a node that repeats its parent's role, name and identifier, as XCTest
//!    reports UIKit's nested alert buttons, is folded into the parent;
//! 5. a field with no label of its own takes the label of the container it
//!    sits in, when that container holds no other field, as React Native
//!    lays out a field marked accessible with a label (Ignite's sign-in
//!    fields in the React Native smoke test); the container is then not
//!    listed again;
//! 6. a labelled node without traits whose children, if any, are static
//!    text gets a ref unless an actionable element holds it: it is a control
//!    the platform exposes without saying what it does, as React Native
//!    exposes a view marked accessible without a role, whose content it may
//!    hide (Ignite's sign-in fields and Bluesky's options and language rows
//!    in the React Native smoke test);
//! 7. the top-most modal (a modal window, or an alert or dialog node) cuts
//!    what lies outside it, since the Simulator's reader still returns the
//!    screen under an alert (spike S4); keyboard windows are collapsed;
//! 8. a view marked modal inside its window (the `modal` node flag, a view
//!    an iOS app marks `accessibilityViewIsModal`) cuts what its siblings
//!    hold within its bounds (task 10.22).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use protocol::snapshot::{NodeFlags, Scroll, ScrollAxis, ScrollFlags, ScrollUnit, WindowType};

use crate::geometry::Rect;
use crate::replica::{RNode, Replica, Win};
use crate::roles;
use crate::snapshot::{Direction, HintSource, Role, ScrollHint};

/// Why a node is not shown although it exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hidden {
    No,
    /// A modal covers it.
    Covered,
    /// It belongs to a keyboard window, listed as one line.
    Keyboard,
}

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
    /// Whether the element gets a ref: actionable, named content, or a
    /// labelled control without traits.
    pub bears_ref: bool,
    /// The node's identity without sibling positions: its window, then the
    /// class, identifier and content of every node down to it. Two elements
    /// that share it are told apart only by where they sit.
    pub signature: u64,
    /// Folded into an ancestor: its label, its role, or a duplicate of it.
    pub absorbed: bool,
    /// A container whose label names the one field inside it.
    pub lent: bool,
    pub hidden: Hidden,
}

/// The facts of every node of every window, parallel to the replica, and
/// what covers the screen.
pub(crate) struct View {
    pub windows: Vec<Vec<Facts>>,
    /// The covering modal as `role "name"`, when one cuts the screen.
    pub modal: Option<String>,
}

impl View {
    pub fn build(replica: &Replica) -> View {
        let mut windows: Vec<Vec<Facts>> = replica
            .windows
            .iter()
            .map(|w| window_facts(w, replica.scale))
            .collect();
        let modal = cut(replica, &mut windows);
        let view_modal = cover_by_modal_views(replica, &mut windows);
        View {
            windows,
            modal: modal.or(view_modal),
        }
    }
}

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

/// A node's own label: its description, else its text unless editable.
fn own_label(node: &RNode, editable: bool) -> Option<&str> {
    non_empty(&node.desc).or_else(|| {
        if editable {
            None
        } else {
            non_empty(&node.text)
        }
    })
}

/// Normalizes a name as agent-browser does: no-break spaces become spaces,
/// zero-width characters go.
pub(crate) fn normalize(name: &str) -> String {
    name.chars()
        .filter_map(|c| match c {
            '\u{00a0}' => Some(' '),
            '\u{feff}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' => None,
            c => Some(c),
        })
        .collect()
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
    promote_roles(window, &mut out);
    name_from_descendants(window, &mut out);
    fold_duplicates(window, &mut out);
    lend_container_labels(window, &mut out);
    untraited_controls(window, &mut out);
    if window.kind == WindowType::INPUT_METHOD {
        for facts in &mut out {
            facts.hidden = Hidden::Keyboard;
            facts.bears_ref = false;
        }
    }
    out
}

/// The facts of node `i` of window `w` as a full view computes them.
pub(crate) fn facts_of(replica: &Replica, w: usize, i: usize) -> Facts {
    window_facts(&replica.windows[w], replica.scale).swap_remove(i)
}

fn node_facts(node: &RNode, area: &Rect, scale: f64) -> Facts {
    let text = non_empty(&node.text);
    let desc = non_empty(&node.desc);
    let role = roles::role(&node.class, node.flags, text.is_some() || desc.is_some());
    let editable = node.editable() || matches!(role, Role::Textbox | Role::Searchbox);
    let name = own_label(node, editable).map(normalize).unwrap_or_default();
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
    Facts {
        role,
        bears_ref: actionable || (role.is_content() && !name.is_empty()),
        name,
        value,
        rect,
        visible,
        actionable,
        checked: checkable.then(|| node.flags.contains(NodeFlags::CHECKED)),
        signature: 0,
        absorbed: false,
        lent: false,
        hidden: Hidden::No,
    }
}

/// Whether a class leaves the role to the node's states.
fn weak_class(class: &str) -> bool {
    matches!(
        roles::class_role(class),
        None | Some(Role::Generic | Role::Group | Role::Text | Role::Image)
    )
}

/// Roles a child can lend its tapped container.
fn control(role: Role) -> bool {
    matches!(
        role,
        Role::Switch | Role::Checkbox | Role::Radio | Role::Slider | Role::Combobox
    )
}

/// The nodes below `root` in pre-order, not entering nodes that take input
/// themselves.
fn passive_descendants(window: &Win, root: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = window.nodes[root].children.iter().rev().copied().collect();
    while let Some(i) = stack.pop() {
        if roles::acts(window.nodes[i].flags) {
            continue;
        }
        out.push(i);
        stack.extend(window.nodes[i].children.iter().rev().copied());
    }
    out
}

fn promote_roles(window: &Win, facts: &mut [Facts]) {
    for (i, node) in window.nodes.iter().enumerate() {
        if !roles::acts(node.flags) || !weak_class(&node.class) {
            continue;
        }
        let lenders: Vec<usize> = passive_descendants(window, i)
            .into_iter()
            .filter(|&d| roles::class_role(&window.nodes[d].class).is_some_and(control))
            .collect();
        let [lender] = lenders[..] else {
            continue;
        };
        let role = roles::class_role(&window.nodes[lender].class).unwrap_or(Role::Generic);
        let checked = if node.flags.contains(NodeFlags::CHECKABLE) {
            node.flags.contains(NodeFlags::CHECKED)
        } else {
            window.nodes[lender].flags.contains(NodeFlags::CHECKED)
        };
        facts[i].role = role;
        facts[i].checked = Some(checked);
        let lent = &mut facts[lender];
        lent.absorbed = true;
        lent.actionable = false;
        lent.bears_ref = false;
    }
}

/// The nodes below `root` in pre-order whose label can name it: the walk
/// does not enter elements that are actionable in their own right.
fn label_sources(window: &Win, facts: &[Facts], root: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = window.nodes[root].children.iter().rev().copied().collect();
    while let Some(d) = stack.pop() {
        if facts[d].actionable && !facts[d].absorbed {
            continue;
        }
        out.push(d);
        stack.extend(window.nodes[d].children.iter().rev().copied());
    }
    out
}

fn name_from_descendants(window: &Win, facts: &mut [Facts]) {
    for i in 0..window.nodes.len() {
        let f = &facts[i];
        if !f.name.is_empty() || f.absorbed || !(f.actionable || f.role.is_content()) {
            continue;
        }
        let mut parts: Vec<String> = Vec::new();
        let mut used = Vec::new();
        for d in label_sources(window, facts, i) {
            let node = &window.nodes[d];
            if let Some(label) = own_label(node, node.editable()) {
                let label = normalize(label);
                if parts.last() != Some(&label) {
                    parts.push(label);
                }
                used.push(d);
            }
        }
        if parts.is_empty() {
            continue;
        }
        facts[i].name = parts.join(" ");
        facts[i].bears_ref = facts[i].actionable || facts[i].role.is_content();
        for d in used {
            facts[d].absorbed = true;
        }
    }
}

fn fold_duplicates(window: &Win, facts: &mut [Facts]) {
    for (i, node) in window.nodes.iter().enumerate() {
        let Some(parent) = node.parent else {
            continue;
        };
        let (p, c) = (&facts[parent], &facts[i]);
        if p.bears_ref
            && !p.absorbed
            && p.role == c.role
            && p.name == c.name
            && window.nodes[parent].identifier == node.identifier
        {
            let folded = &mut facts[i];
            folded.absorbed = true;
            folded.bears_ref = false;
        }
    }
}

/// Whether a node takes text.
fn field(node: &RNode, role: Role) -> bool {
    node.editable() || matches!(role, Role::Textbox | Role::Searchbox)
}

/// How many fields lie below `root`.
fn fields_below(window: &Win, facts: &[Facts], root: usize) -> usize {
    let mut count = 0;
    let mut stack: Vec<usize> = window.nodes[root].children.clone();
    while let Some(d) = stack.pop() {
        if field(&window.nodes[d], facts[d].role) && !facts[d].absorbed {
            count += 1;
        }
        stack.extend(window.nodes[d].children.iter().copied());
    }
    count
}

/// Names each field without a label of its own after its nearest labelled
/// ancestor, when that ancestor is a container rather than an element of
/// its own (not actionable, not the window's root) and holds no other
/// field. The container's label then names the field alone.
fn lend_container_labels(window: &Win, facts: &mut [Facts]) {
    for i in 0..window.nodes.len() {
        let f = &facts[i];
        if !f.name.is_empty() || f.absorbed || !field(&window.nodes[i], f.role) {
            continue;
        }
        let mut up = window.nodes[i].parent;
        while let Some(a) = up {
            let container = &facts[a];
            if container.actionable {
                break;
            }
            if !container.name.is_empty() {
                let lends = matches!(container.role, Role::Generic | Role::Group)
                    && !container.absorbed
                    && window.nodes[a].parent.is_some()
                    && fields_below(window, facts, a) == 1;
                if lends {
                    let name = container.name.clone();
                    facts[a].lent = true;
                    facts[i].name = name;
                }
                break;
            }
            up = window.nodes[a].parent;
        }
    }
}

/// Gives a ref to each labelled node without traits whose children, if any,
/// are static text, other than a window's root and the nodes an actionable
/// element holds, which are its content or, when it has no label of its
/// own, its name. The platform exposes such a node as an element without
/// saying what it does: React Native exposes a view marked accessible
/// without a role this way, on iOS hiding what the view holds, and
/// agent-device lists these nodes as `Other` and `Cell`.
fn untraited_controls(window: &Win, facts: &mut [Facts]) {
    for i in 0..window.nodes.len() {
        let node = &window.nodes[i];
        let f = &facts[i];
        if !matches!(f.role, Role::Generic | Role::Group)
            || f.name.is_empty()
            || f.bears_ref
            || f.absorbed
            || node.parent.is_none()
        {
            continue;
        }
        let static_text =
            |c: usize| facts[c].role == Role::Text && window.nodes[c].children.is_empty();
        let held = std::iter::successors(node.parent, |&a| window.nodes[a].parent)
            .any(|a| facts[a].actionable);
        if node.children.iter().all(|&c| static_text(c)) && !held {
            facts[i].bears_ref = true;
        }
    }
}

/// Finds the top-most modal and hides what it covers; returns its label.
fn cut(replica: &Replica, facts: &mut [Vec<Facts>]) -> Option<String> {
    for (w, window) in replica.windows.iter().enumerate() {
        if window.kind == WindowType::INPUT_METHOD {
            continue;
        }
        if window.modal {
            cover_below(replica, facts, w);
            let title = window.title.as_deref().unwrap_or_default();
            return Some(format!("dialog {title:?}"));
        }
        let modal = (0..window.nodes.len())
            .rev()
            .find(|&i| facts[w][i].visible && facts[w][i].role.is_modal());
        if let Some(m) = modal {
            let mut keep = vec![false; window.nodes.len()];
            let mut up = Some(m);
            while let Some(a) = up {
                keep[a] = true;
                up = window.nodes[a].parent;
            }
            let mut stack = vec![m];
            while let Some(d) = stack.pop() {
                keep[d] = true;
                stack.extend(window.nodes[d].children.iter().copied());
            }
            for (i, kept) in keep.into_iter().enumerate() {
                if !kept {
                    facts[w][i].hidden = Hidden::Covered;
                }
            }
            cover_below(replica, facts, w);
            let f = &facts[w][m];
            return Some(if f.name.is_empty() {
                f.role.to_string()
            } else {
                format!("{} {:?}", f.role, f.name)
            });
        }
    }
    None
}

/// Hides the application windows below window `w`.
fn cover_below(replica: &Replica, facts: &mut [Vec<Facts>], w: usize) {
    for (lower, window) in replica.windows.iter().enumerate().skip(w + 1) {
        if matches!(window.kind, WindowType::APPLICATION | WindowType::UNKNOWN) {
            for f in &mut facts[lower] {
                f.hidden = Hidden::Covered;
            }
        }
    }
}

/// Hides what each view marked modal covers in its window (a node with the
/// `modal` flag, which the Simulator's reader sets on a view the app marks
/// `accessibilityViewIsModal`): the nodes of its siblings, other than
/// siblings marked modal themselves, that lie within its bounds. Assistive
/// technology leaves out every sibling, as XCTest's tree does; a sibling
/// outside the view stays, since the user sees it and a tap reaches it, as
/// Bluesky's header does, a sibling of its typeahead only because React
/// Native flattens the views between them. A node is judged by its own
/// bounds, so a container that reaches outside the view stays while what it
/// holds inside is hidden. Returns the label of the first such view that
/// hid anything.
fn cover_by_modal_views(replica: &Replica, facts: &mut [Vec<Facts>]) -> Option<String> {
    let slack = if replica.scale > 0.0 {
        1.0 / replica.scale
    } else {
        1.0
    };
    let mut label = None;
    for (w, window) in replica.windows.iter().enumerate() {
        if window.kind == WindowType::INPUT_METHOD {
            continue;
        }
        let marked = |i: usize| window.nodes[i].flags.contains(NodeFlags::MODAL);
        for m in 0..window.nodes.len() {
            let f = &facts[w][m];
            if !marked(m) || !f.visible || f.hidden != Hidden::No {
                continue;
            }
            let Some(parent) = window.nodes[m].parent else {
                continue;
            };
            let area = f.rect;
            let mut hid = false;
            let mut stack: Vec<usize> = window.nodes[parent]
                .children
                .iter()
                .copied()
                .filter(|&s| s != m && !marked(s))
                .collect();
            while let Some(d) = stack.pop() {
                let covered = &mut facts[w][d];
                if covered.hidden == Hidden::No && within(&covered.rect, &area, slack) {
                    covered.hidden = Hidden::Covered;
                    hid = true;
                }
                stack.extend(window.nodes[d].children.iter().copied());
            }
            if hid && label.is_none() {
                label = Some(modal_view_label(&facts[w][m]));
            }
        }
    }
    label
}

/// Whether `inner` lies within `outer`, to within `slack` on each side.
fn within(inner: &Rect, outer: &Rect, slack: f64) -> bool {
    inner.left >= outer.left - slack
        && inner.top >= outer.top - slack
        && inner.right <= outer.right + slack
        && inner.bottom <= outer.bottom + slack
}

/// How errors name a view marked modal: by its role and name, or as a modal
/// view when it says neither, as a plain container.
fn modal_view_label(facts: &Facts) -> String {
    let plain = matches!(facts.role, Role::Generic | Role::Group);
    match (plain, facts.name.is_empty()) {
        (true, true) => "modal view".to_owned(),
        (true, false) => format!("modal view {:?}", facts.name),
        (false, true) => facts.role.to_string(),
        (false, false) => format!("{} {:?}", facts.role, facts.name),
    }
}

/// Whether a node scrolls its content: it says it scrolls, it is a list or
/// a grid, or it has scroll semantics.
pub(crate) fn scrolls(node: &RNode, facts: &Facts) -> bool {
    node.flags.contains(NodeFlags::SCROLLABLE)
        || matches!(facts.role, Role::List | Role::Grid)
        || node.scroll.is_some()
}

/// Scroll hints per container: `(window index, node index, hint)`, in
/// pre-order of containers and, per container, above, left, below, right.
pub(crate) fn hints(replica: &Replica, view: &View) -> Vec<(usize, usize, ScrollHint)> {
    let mut out = Vec::new();
    for (w, window) in replica.windows.iter().enumerate() {
        let facts = &view.windows[w];
        let area = Rect::from_pixels(window.bounds, replica.scale);
        let container = |i: usize| scrolls(&window.nodes[i], &facts[i]);
        let mut counts: Vec<[u32; 4]> = vec![[0; 4]; window.nodes.len()];
        for (x, node) in window.nodes.iter().enumerate() {
            let f = &facts[x];
            if !f.bears_ref || f.absorbed || f.hidden != Hidden::No {
                continue;
            }
            // The nearest scroll container above, unless another item on
            // the way already stands for this one.
            let mut up = node.parent;
            let mut owner = None;
            while let Some(a) = up {
                if container(a) {
                    owner = Some(a);
                    break;
                }
                if facts[a].bears_ref && !facts[a].absorbed {
                    break;
                }
                up = window.nodes[a].parent;
            }
            let Some(s) = owner else {
                continue;
            };
            let viewport = facts[s].rect.intersect(&area);
            if f.visible && f.rect.overlaps(&viewport) {
                continue;
            }
            let r = &f.rect;
            let direction = if r.top >= viewport.bottom {
                Direction::Below
            } else if r.bottom <= viewport.top {
                Direction::Above
            } else if r.left >= viewport.right {
                Direction::Right
            } else if r.right <= viewport.left {
                Direction::Left
            } else {
                continue;
            };
            counts[s][slot(direction)] += 1;
        }
        for (s, node) in window.nodes.iter().enumerate() {
            let f = &facts[s];
            if !container(s) || !f.visible || f.hidden != Hidden::No {
                continue;
            }
            for direction in [
                Direction::Above,
                Direction::Left,
                Direction::Below,
                Direction::Right,
            ] {
                let nodes = counts[s][slot(direction)];
                let hint = if nodes > 0 {
                    Some((Some(nodes), HintSource::OffscreenNodes))
                } else {
                    node.scroll
                        .and_then(|scroll| beyond(&scroll, direction))
                        .map(|count| (count, HintSource::ScrollSemantics))
                };
                if let Some((count, source)) = hint {
                    out.push((
                        w,
                        s,
                        ScrollHint {
                            container: node.element,
                            window: window.id,
                            direction,
                            count,
                            source,
                        },
                    ));
                }
            }
        }
    }
    out
}

/// What a node's scroll semantics say lies beyond its viewport in
/// `direction`: `Some(Some(n))` for `n` items, `Some(None)` for more content
/// of unknown size, `None` for nothing that way or semantics along the other
/// axis. A range with a position measures it, in items or only as a
/// direction in other units; without one, the ways the node can scroll say
/// where more lies.
fn beyond(scroll: &Scroll, direction: Direction) -> Option<Option<u32>> {
    let back = matches!(direction, Direction::Above | Direction::Left);
    let axis = if matches!(direction, Direction::Above | Direction::Below) {
        ScrollAxis::VERTICAL
    } else {
        ScrollAxis::HORIZONTAL
    };
    if scroll.axis != axis {
        return None;
    }
    match (scroll.range, scroll.position) {
        (Some(range), Some(position)) => {
            let beyond = if back {
                i64::from(position.first) - i64::from(range.min)
            } else {
                i64::from(range.max) - i64::from(position.last)
            };
            (beyond > 0).then(|| {
                (range.unit == ScrollUnit::ITEMS).then(|| u32::try_from(beyond).unwrap_or(u32::MAX))
            })
        }
        _ => {
            let way = if back {
                ScrollFlags::BACK
            } else {
                ScrollFlags::FORWARD
            };
            scroll.flags.contains(way).then_some(None)
        }
    }
}

fn slot(direction: Direction) -> usize {
    match direction {
        Direction::Above => 0,
        Direction::Left => 1,
        Direction::Below => 2,
        Direction::Right => 3,
    }
}
