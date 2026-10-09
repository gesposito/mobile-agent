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
//!    hold within its bounds (task 10.22);
//! 9. a view drawn over the siblings it overlaps (the `on_top` node flag,
//!    which the Android agent sets from the order a parent draws its
//!    children in) cuts what they hold within its bounds when it shows
//!    something (task 10.28);
//! 10. in XCTest's tree, whose order is the order UIKit draws views in, a
//!     node listed after a sibling it overlaps cuts what that sibling holds
//!     within its bounds when it shows something, as `on_top` does
//!     (rn-app-fixes, design D31).

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
    /// A modal, a view marked modal or a view drawn over it covers it.
    Covered,
    /// It belongs to a keyboard window, listed as one line.
    Keyboard,
}

/// What covers a node, by window and node index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cover {
    /// A modal window, named by its title.
    Window(usize),
    /// Node `i` of window `w`, named by its role and name, or as `noun`
    /// when it is a plain container without a name: an alert or a dialog,
    /// or a view marked modal.
    Node {
        w: usize,
        i: usize,
        noun: &'static str,
    },
    /// Node `i` of the covered node's window, drawn over it.
    DrawnOver(usize),
}

/// How errors name what covers a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CoverName {
    /// As `role "name"`, as `dialog "title"`, or as a plain container is
    /// called: `modal view`, `overlay`.
    pub by: String,
    /// Whether a view drawn over the node covers it, rather than a modal
    /// that an action on it would dismiss.
    pub drawn_over: bool,
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
/// what covers the nodes that are covered.
pub(crate) struct View {
    pub windows: Vec<Vec<Facts>>,
    /// The top-most modal, when one cuts the screen: it covers every node
    /// it hid.
    modal: Option<Cover>,
    /// What covers each node that a view marked modal or a view drawn over
    /// it hid: window, node, cover.
    covers: Vec<(usize, usize, Cover)>,
}

impl View {
    pub fn build(replica: &Replica) -> View {
        let mut windows: Vec<Vec<Facts>> = replica
            .windows
            .iter()
            .map(|w| window_facts(w, replica.scale))
            .collect();
        let mut covers = Vec::new();
        let modal = cut(replica, &mut windows);
        cover_by_modal_views(replica, &mut windows, &mut covers);
        cover_by_views_on_top(replica, &mut windows, &mut covers);
        if drawn_in_order(replica) {
            cover_by_later_siblings(replica, &mut windows, &mut covers);
        }
        View {
            windows,
            modal,
            covers,
        }
    }

    /// How errors name what covers node `i` of window `w`, if something
    /// does. A view drawn over it is named after the innermost node of it
    /// that holds the covered node's bounds and shows something itself,
    /// where a tap on the covered node would land, else after the view.
    pub fn cover(&self, replica: &Replica, w: usize, i: usize) -> Option<CoverName> {
        if self.windows[w][i].hidden != Hidden::Covered {
            return None;
        }
        let cover = self
            .covers
            .iter()
            .find(|&&(cw, ci, _)| (cw, ci) == (w, i))
            .map(|&(_, _, cover)| cover)
            .or(self.modal);
        Some(match cover {
            Some(Cover::Window(m)) => CoverName {
                by: format!(
                    "dialog {:?}",
                    replica.windows[m].title.as_deref().unwrap_or_default()
                ),
                drawn_over: false,
            },
            Some(Cover::Node { w, i, noun }) => CoverName {
                by: cover_label(&self.windows[w][i], noun),
                drawn_over: false,
            },
            Some(Cover::DrawnOver(s)) => {
                let lies_under = holder(
                    &replica.windows[w],
                    &self.windows[w],
                    s,
                    &self.windows[w][i].rect,
                    pixel(replica),
                );
                CoverName {
                    by: cover_label(&self.windows[w][lies_under], "overlay"),
                    drawn_over: true,
                }
            }
            None => CoverName {
                by: "modal".to_owned(),
                drawn_over: false,
            },
        })
    }
}

/// One physical pixel in points: bounds within it of each other meet.
fn pixel(replica: &Replica) -> f64 {
    if replica.scale > 0.0 {
        1.0 / replica.scale
    } else {
        1.0
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

/// Finds the top-most modal and hides what it covers; returns it.
fn cut(replica: &Replica, facts: &mut [Vec<Facts>]) -> Option<Cover> {
    for (w, window) in replica.windows.iter().enumerate() {
        if window.kind == WindowType::INPUT_METHOD {
            continue;
        }
        if window.modal {
            cover_below(replica, facts, w);
            return Some(Cover::Window(w));
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
            // An alert or a dialog is no plain container: it is named by
            // its role.
            return Some(Cover::Node {
                w,
                i: m,
                noun: "modal",
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
/// holds inside is hidden. Records the view as the cover of what it hid.
fn cover_by_modal_views(
    replica: &Replica,
    facts: &mut [Vec<Facts>],
    covers: &mut Vec<(usize, usize, Cover)>,
) {
    let slack = pixel(replica);
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
            let cover = Cover::Node {
                w,
                i: m,
                noun: "modal view",
            };
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
                    covers.push((w, d, cover));
                }
                stack.extend(window.nodes[d].children.iter().copied());
            }
        }
    }
}

/// Hides what each view drawn over its siblings covers in its window (a node
/// with the `on_top` flag, which the Android agent sets on a node drawn after
/// every sibling its bounds overlap, from the order the platform reports a
/// parent draws its children in): the nodes of its siblings that lie within
/// its bounds, as Bluesky's typeahead and recent searches lie over the
/// Explore page, both with the same bounds, the platform listing the
/// typeahead first (task 10.28). The platform reports no occlusion inside a
/// window, so the page's rows read as on screen, and a tap on one lands on
/// the typeahead. Only a view that shows something hides what it
/// covers, one holding a node on screen, itself or below it, with a ref or
/// a name: a plain view the agent flags, such as the decoration Bluesky
/// draws over a video, can be transparent, and nothing in the tree tells a
/// transparent view from a drawn one. A node is judged by its own bounds and
/// only when it has an area, since Android clips a node's bounds to its
/// ancestors' and leaves one scrolled out of its list with none, so that the
/// rows below a list keep counting in its hint under a tab bar drawn over
/// its bottom. Records the view as the cover of what it hid.
fn cover_by_views_on_top(
    replica: &Replica,
    facts: &mut [Vec<Facts>],
    covers: &mut Vec<(usize, usize, Cover)>,
) {
    let slack = pixel(replica);
    let mut stack = Vec::new();
    for (w, window) in replica.windows.iter().enumerate() {
        let on_top = |i: usize| window.nodes[i].flags.contains(NodeFlags::ON_TOP);
        if window.kind == WindowType::INPUT_METHOD || !(0..window.nodes.len()).any(on_top) {
            continue;
        }
        let shows = showing(window, &facts[w]);
        for s in 0..window.nodes.len() {
            let f = &facts[w][s];
            if !on_top(s) || !shows[s] || !f.visible || f.hidden != Hidden::No {
                continue;
            }
            let Some(parent) = window.nodes[s].parent else {
                continue;
            };
            stack.clear();
            stack.extend(
                window.nodes[parent]
                    .children
                    .iter()
                    .copied()
                    .filter(|&sibling| sibling != s),
            );
            hide_under(window, &mut facts[w], s, &mut stack, slack, false, |d| {
                covers.push((w, d, Cover::DrawnOver(s)));
            });
        }
    }
}

/// Hides the nodes on `stack` and below them that lie within the bounds of
/// node `s` of `window`, a view drawn over them, and calls `hid` with each.
/// What a node holds lies within its bounds as Android clips them, so the
/// walk does not enter a node without an area in the view's bounds. iOS
/// reports whole frames, and a node there can hold one outside its own
/// bounds; such a node lies outside the clip of a scroll container it
/// overflows, out of view rather than covered, and keeps counting in the
/// container's hint. With `spare_bars` the walk leaves tab bars and
/// toolbars alone, with what they hold.
fn hide_under(
    window: &Win,
    facts: &mut [Facts],
    s: usize,
    stack: &mut Vec<usize>,
    slack: f64,
    spare_bars: bool,
    mut hid: impl FnMut(usize),
) {
    let area = facts[s].rect;
    while let Some(d) = stack.pop() {
        let covered = &mut facts[d];
        if !covered.rect.overlaps(&area)
            || (spare_bars && matches!(covered.role, Role::Tablist | Role::Toolbar))
        {
            continue;
        }
        if covered.hidden == Hidden::No && within(&covered.rect, &area, slack) {
            covered.hidden = Hidden::Covered;
            hid(d);
        }
        stack.extend(window.nodes[d].children.iter().copied());
    }
}

/// Whether the order of the replica's nodes is the order they are drawn
/// in, a later sibling over an earlier one: the tree is XCTest's, as the
/// Simulator's reader reads it (a window's root carries an
/// `XCUIElementType` class), which lists a view's subviews in the order
/// UIKit draws them, and does not come from a fallback, as the
/// translator's tree does, which flattens it into a list in another order.
/// Android's agent lists a view group's children by where they lie and
/// flags the one drawn over the others (`on_top`) instead.
fn drawn_in_order(replica: &Replica) -> bool {
    !replica.backend.as_ref().is_some_and(|b| b.fallback)
        && replica.windows.iter().any(|window| {
            window
                .nodes
                .first()
                .is_some_and(|root| root.class.starts_with(roles::XCUI_PREFIX))
        })
}

/// Hides, in a tree whose order is its drawing order ([`drawn_in_order`]),
/// what each node drawn after a sibling it overlaps covers of that
/// sibling: the sibling's nodes that lie within its bounds, when it shows
/// something, a node on screen with a ref or a name in it or below it, as
/// a view flagged `on_top` does on Android (task 10.28). XCTest reports no
/// occlusion, so only the order and the bounds say that a React Native
/// screen's fixed footer lies over the heading scrolled under it, a
/// floating button over a list's last row, a sticky header over the post
/// scrolled under it (rn-app-fixes, design D31). Only siblings are
/// compared, only nodes with an area are judged, and the walk below a
/// covered sibling follows [`hide_under`]. Bars are spared: XCTest lists a
/// navigation bar before the content its controller draws it over, which
/// runs under it (the SwiftUI fixture's List tab), so the order says
/// nothing about a bar listed first; `visibility` already cuts every tab
/// bar and toolbar out of the visible bounds of what lies beside it. A
/// bar listed after content, as UIKit's tab bar is, covers what lies
/// wholly under it as any later sibling does.
///
/// The covers are taken top-most first, parents before their children and
/// later siblings before earlier ones, so a node two views cover is
/// recorded under the one drawn last. Overlapping siblings are found by
/// [`overlapping`].
fn cover_by_later_siblings(
    replica: &Replica,
    facts: &mut [Vec<Facts>],
    covers: &mut Vec<(usize, usize, Cover)>,
) {
    let slack = pixel(replica);
    let mut sweep = Sweep::default();
    let mut stack = Vec::new();
    for (w, window) in replica.windows.iter().enumerate() {
        if window.kind == WindowType::INPUT_METHOD {
            continue;
        }
        let widest = window.nodes.iter().map(|n| n.children.len()).max();
        sweep.active.reserve(widest.unwrap_or(0));
        let mut shows: Option<Vec<bool>> = None;
        for parent in 0..window.nodes.len() {
            let children = &window.nodes[parent].children;
            if children.len() < 2 {
                continue;
            }
            let pairs = overlapping(&facts[w], children, &mut sweep);
            if pairs.is_empty() {
                continue;
            }
            pairs.sort_unstable_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
            let shows = shows.get_or_insert_with(|| showing(window, &facts[w]));
            for &(earlier, later) in pairs.iter() {
                let (e, s) = (children[earlier], children[later]);
                let f = &facts[w][s];
                if !shows[s] || !f.visible || f.hidden != Hidden::No {
                    continue;
                }
                stack.clear();
                stack.push(e);
                hide_under(window, &mut facts[w], s, &mut stack, slack, true, |d| {
                    covers.push((w, d, Cover::DrawnOver(s)));
                });
            }
        }
    }
}

/// Scratch space for [`overlapping`], kept across the parents of a view.
#[derive(Default)]
struct Sweep {
    /// Positions of a parent's children in the order of their top edges,
    /// when they do not come in it.
    order: Vec<usize>,
    /// The children the sweep has passed whose bottom edges it may not
    /// have passed.
    active: Vec<usize>,
    /// The overlapping pairs found.
    pairs: Vec<(usize, usize)>,
}

/// The pairs of `children`, by their positions among them, earlier first,
/// whose bounds in `facts` overlap: a sweep over the children with an area
/// in the order of their top edges, comparing each with those above it
/// whose bottom edges reach below its top. Most children come in that
/// order already, a list's rows and a grid's cells among them, and those
/// cost a comparison or two each: a row starts below every row before it,
/// and a cell to the right of every cell beside it.
fn overlapping<'s>(
    facts: &[Facts],
    children: &[usize],
    sweep: &'s mut Sweep,
) -> &'s mut Vec<(usize, usize)> {
    let Sweep {
        order,
        active,
        pairs,
    } = sweep;
    pairs.clear();
    active.clear();
    let rect = |k: usize| &facts[children[k]].rect;
    let sorted = children
        .windows(2)
        .all(|pair| facts[pair[0]].rect.top <= facts[pair[1]].rect.top);
    order.clear();
    if !sorted {
        order.extend(0..children.len());
        order.sort_unstable_by(|&a, &b| rect(a).top.total_cmp(&rect(b).top).then(a.cmp(&b)));
    }
    let mut in_place = 0..children.len();
    let mut by_top = order.iter().copied();
    let positions: &mut dyn Iterator<Item = usize> =
        if sorted { &mut in_place } else { &mut by_top };
    // The highest bottom edge and the rightmost right edge among the
    // active children: below the one, or right of the other, nothing
    // active reaches.
    let (mut low, mut right) = (f64::INFINITY, f64::NEG_INFINITY);
    for k in positions {
        let r = rect(k);
        if r.is_empty() {
            continue;
        }
        if r.top >= low {
            active.retain(|&a| rect(a).bottom > r.top);
            low = active
                .iter()
                .map(|&a| rect(a).bottom)
                .fold(f64::INFINITY, f64::min);
            right = active
                .iter()
                .map(|&a| rect(a).right)
                .fold(f64::NEG_INFINITY, f64::max);
        }
        if r.left < right {
            pairs.extend(
                active
                    .iter()
                    .filter(|&&a| rect(a).overlaps(r))
                    .map(|&a| (a.min(k), a.max(k))),
            );
        }
        active.push(k);
        low = low.min(r.bottom);
        right = right.max(r.right);
    }
    pairs
}

/// The node of `window` that a tap on a node at `covered` would land on
/// below node `cover`, a view drawn over it: the innermost node of the
/// view, or the one drawn last, whose bounds hold `covered`, to within
/// `slack`, and that shows something itself, on screen with a ref or a
/// name; else the view itself.
fn holder(window: &Win, facts: &[Facts], cover: usize, covered: &Rect, slack: f64) -> usize {
    let mut found = cover;
    let mut stack = vec![cover];
    while let Some(n) = stack.pop() {
        let f = &facts[n];
        if !within(covered, &f.rect, slack) {
            continue;
        }
        if f.visible && (f.bears_ref || !f.name.is_empty()) && n > found {
            found = n;
        }
        stack.extend(window.nodes[n].children.iter().copied());
    }
    found
}

/// Whether each node of `window`, or a node below it, is on screen with a
/// ref or a name.
fn showing(window: &Win, facts: &[Facts]) -> Vec<bool> {
    let mut shows: Vec<bool> = facts
        .iter()
        .map(|f| f.visible && (f.bears_ref || !f.name.is_empty()))
        .collect();
    // Children come after their parent in the pre-order.
    for i in (0..window.nodes.len()).rev() {
        if shows[i]
            && let Some(parent) = window.nodes[i].parent
        {
            shows[parent] = true;
        }
    }
    shows
}

/// Whether `inner` lies within `outer`, to within `slack` on each side.
fn within(inner: &Rect, outer: &Rect, slack: f64) -> bool {
    inner.left >= outer.left - slack
        && inner.top >= outer.top - slack
        && inner.right <= outer.right + slack
        && inner.bottom <= outer.bottom + slack
}

/// How errors name a view that covers what lies under it: by its role and
/// name, or as `noun` (a modal view, an overlay) when it says neither, as a
/// plain container.
fn cover_label(facts: &Facts, noun: &str) -> String {
    let plain = matches!(facts.role, Role::Generic | Role::Group);
    match (plain, facts.name.is_empty()) {
        (true, true) => noun.to_owned(),
        (true, false) => format!("{noun} {:?}", facts.name),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Plain containers at `rects`, in points.
    fn at(rects: &[[f64; 4]]) -> Vec<Facts> {
        rects
            .iter()
            .map(|&[left, top, right, bottom]| Facts {
                role: Role::Generic,
                name: String::new(),
                value: None,
                rect: Rect::new(left, top, right, bottom),
                visible: true,
                actionable: false,
                checked: None,
                bears_ref: false,
                signature: 0,
                absorbed: false,
                lent: false,
                hidden: Hidden::No,
            })
            .collect()
    }

    fn pairs(facts: &[Facts]) -> Vec<(usize, usize)> {
        let children: Vec<usize> = (0..facts.len()).collect();
        let mut found = overlapping(facts, &children, &mut Sweep::default()).clone();
        found.sort_unstable();
        found
    }

    #[test]
    fn siblings_overlap_in_whatever_order_they_come_and_touching_ones_do_not() {
        let facts = at(&[
            [0.0, 62.0, 402.0, 874.0],
            // A header listed after the content it overlaps, above it.
            [0.0, 0.0, 402.0, 118.0],
            [16.0, 126.0, 196.0, 190.0],
            [196.0, 126.0, 386.0, 190.0],
            [16.0, 190.0, 386.0, 254.0],
            // Nothing without an area overlaps.
            [100.0, 100.0, 100.0, 300.0],
        ]);
        assert_eq!(pairs(&facts), [(0, 1), (0, 2), (0, 3), (0, 4)]);
    }

    #[test]
    fn the_cells_of_a_grid_and_the_rows_of_a_list_overlap_nothing() {
        let grid: Vec<[f64; 4]> = (0..60)
            .map(|n| {
                let (column, row) = (f64::from(n % 20), f64::from(n / 20));
                [
                    1.0 + 20.0 * column,
                    90.0 + 28.0 * row,
                    21.0 + 20.0 * column,
                    118.0 + 28.0 * row,
                ]
            })
            .collect();
        assert_eq!(pairs(&at(&grid)), []);
        let mut list: Vec<[f64; 4]> = (0..10)
            .map(|n| [0.0, 48.0 * f64::from(n), 402.0, 48.0 * f64::from(n + 1)])
            .collect();
        assert_eq!(pairs(&at(&list)), []);
        // A row drawn over the two it straddles.
        list.push([0.0, 72.0, 402.0, 120.0]);
        assert_eq!(pairs(&at(&list)), [(1, 10), (2, 10)]);
    }
}
