//! What the replica's nodes look like to a user: role, accessible name,
//! value, bounds in points, visibility, and what covers them.
//!
//! Facts are computed per window in passes over the pre-order:
//!
//! 1. each node on its own: role from class and states, or a checkbox that
//!    React Native announces only in its value on iOS, its own label,
//!    value, bounds, visibility;
//! 2. a tapped container that keeps the role on a child, as Material
//!    components in Compose do, takes the child's role and absorbs it;
//! 3. an actionable element with no label of its own is named by its
//!    non-actionable descendants, which it absorbs, the way accessibility
//!    services compute a name (design D5);
//! 4. a node that repeats its parent's role, name and identifier, as XCTest
//!    reports UIKit's nested alert buttons, is folded into the parent, and
//!    so is the only child that repeats a text or a heading, as XCTest
//!    reports React Native's text and UIKit's scroll bars;
//! 5. a field with no label of its own takes the label of the container it
//!    sits in, when that container holds no other field, as React Native
//!    lays out a field marked accessible with a label (Ignite's sign-in
//!    fields in the React Native smoke test, reports/10.4-rn-smoke.md); the
//!    container is then not
//!    listed again;
//! 6. a labelled node without traits whose children, if any, are static
//!    text, or that is selected, gets a ref unless an actionable element
//!    holds it: it is a control the platform exposes without saying what it
//!    does, as React Native exposes a view marked accessible without a role,
//!    whose content it may hide (Ignite's sign-in fields and Bluesky's
//!    options and language rows in the React Native smoke test), and a tab;
//!    such nodes laid out in one row with a selected one are tabs
//!    (rn-app-fixes, design D40);
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
//!     (rn-app-fixes, design D31), but for an alert's text, over which
//!     UIKit draws nothing (physical-ios, design D26), and a host, a node
//!     that holds the whole sibling within its frame, shows nothing itself
//!     and is no bar, cuts it within the bounds of what shows in it
//!     (real-app-fixes, design D2); and a bar, or a header the app pins
//!     across the top of a scrolling sibling, cuts what its later siblings
//!     hold within its bounds, whatever the order (rn-app-fixes, design
//!     D38).
//!
//! A disabled control shows something wherever showing decides a cover:
//! UIKit hit-tests it as an enabled one, and the touch goes nowhere
//! (design D38). What a cover overlaps without holding it whole is kept
//! with the cover, for the part of it a tap reaches (design D37).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use protocol::snapshot::{NodeFlags, Scroll, ScrollAxis, ScrollFlags, ScrollUnit, WindowType};

use crate::geometry::{Point, Rect};
use crate::replica::{RNode, Replica, Win};
use crate::roles;
use crate::snapshot::{Direction, HintSource, Role, ScrollHint};
use crate::visibility::Reaches;

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
pub(crate) enum Cover {
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
    /// Node `s` of window `w`, the covered node's, drawn over it.
    DrawnOver { w: usize, s: usize },
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
    /// What lies over part of a node, a view marked modal or a view drawn
    /// over it that overlaps the node without holding it: window, node,
    /// cover, by window and node, each node's covers in the order recorded.
    partial: Vec<(usize, usize, Cover)>,
}

impl View {
    pub fn build(replica: &Replica) -> View {
        let mut windows: Vec<Vec<Facts>> = replica
            .windows
            .iter()
            .map(|w| window_facts(w, replica.scale))
            .collect();
        let mut covers = Covers::default();
        let modal = cut(replica, &mut windows);
        cover_by_modal_views(replica, &mut windows, &mut covers);
        cover_by_views_on_top(replica, &mut windows, &mut covers);
        if drawn_in_order(replica) {
            cover_by_later_siblings(replica, &mut windows, &mut covers);
            cover_by_bars_and_headers(replica, &mut windows, &mut covers);
        }
        covers.partial.sort_by_key(|&(w, i, _)| (w, i));
        View {
            windows,
            modal,
            covers: covers.whole,
            partial: covers.partial,
        }
    }

    /// How errors name what covers node `i` of window `w`, if something
    /// covers all of it: what a tap at its centre would land on (see
    /// [`View::name`]).
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
        let at = self.windows[w][i].rect.center();
        Some(match cover {
            Some(cover) => self.cover_name(replica, (w, i), cover, at),
            None => CoverName {
                by: "modal".to_owned(),
                drawn_over: false,
            },
        })
    }

    /// The top-most modal that cuts the screen, a modal window or an alert
    /// or dialog node, with the index of the window that holds it and how
    /// errors name it over a node it covers; `None` when none cuts it.
    pub fn front_modal(&self, replica: &Replica) -> Option<(usize, CoverName)> {
        let cover = self.modal?;
        let w = match cover {
            Cover::Window(w) | Cover::Node { w, .. } | Cover::DrawnOver { w, .. } => w,
        };
        Some((w, self.name(replica, cover, Point::new(0.0, 0.0))))
    }

    /// What covers all of node `i` of window `w`, when a view marked modal or
    /// a view drawn over it does; `None` under the top-most modal alone or
    /// nothing.
    pub fn whole_cover(&self, w: usize, i: usize) -> Option<Cover> {
        self.covers
            .iter()
            .find(|&&(cw, ci, _)| (cw, ci) == (w, i))
            .map(|&(_, _, cover)| cover)
    }

    /// Whether a view drawn over node `i` of window `w` covers all of it, as
    /// D31's later siblings, Android's views on top, bars and pinned
    /// headers do; such a node stays in the full listing at visible=0%
    /// (rn-app-fixes, design D57), where a modal's cuts it.
    pub(crate) fn drawn_over_whole(&self, w: usize, i: usize) -> bool {
        matches!(self.whole_cover(w, i), Some(Cover::DrawnOver { .. }))
    }

    /// How hints and refusals name `cover` over node `i` of window `w`, by
    /// one rule (rn-app-fixes, design D57): a view drawn over it is the
    /// header or the footer of the scroll container the node counts in,
    /// or of the window for a node outside any ([`edge`]), as the hints
    /// have named it since designs D50 and D54; any other cover is named as
    /// what a tap at `at` would land on ([`View::name`]). The sixth
    /// production-app run's click on a control under the creation tab's footer
    /// was refused naming the overlay while the hint said the footer.
    pub fn cover_name(
        &self,
        replica: &Replica,
        (w, i): (usize, usize),
        cover: Cover,
        at: Point,
    ) -> CoverName {
        if let Cover::DrawnOver { w: cw, s } = cover
            && cw == w
        {
            let (window, facts) = (&replica.windows[w], &self.windows[w]);
            let list = owner(window, facts, i);
            let area = list.map_or_else(
                || Rect::from_pixels(window.bounds, replica.scale),
                |c| facts[c].rect,
            );
            if let Some(edge) = edge(window, facts, s, (list, &area), pixel(replica)) {
                return CoverName {
                    by: edge.to_owned(),
                    drawn_over: true,
                };
            }
        }
        self.name(replica, cover, at)
    }

    /// The views that lie over part of node `i` of window `w` without
    /// holding all of it, with their bounds.
    pub fn over_part(&self, w: usize, i: usize) -> impl Iterator<Item = (Cover, Rect)> + '_ {
        let from = self
            .partial
            .partition_point(|&(cw, ci, _)| (cw, ci) < (w, i));
        self.partial[from..]
            .iter()
            .take_while(move |&&(cw, ci, _)| (cw, ci) == (w, i))
            .map(|&(_, _, cover)| (cover, self.bounds_of(cover)))
    }

    /// The bounds of what `cover` names, in points; a modal window's cover
    /// the screen, which `Rect::default` stands for nowhere.
    fn bounds_of(&self, cover: Cover) -> Rect {
        match cover {
            Cover::Node { w, i, .. } => self.windows[w][i].rect,
            Cover::DrawnOver { w, s } => self.windows[w][s].rect,
            Cover::Window(_) => Rect::default(),
        }
    }

    /// How errors name `cover` as what a tap at `at` would land on. A view
    /// drawn over a node is named after the innermost node of it that
    /// holds `at` and shows something itself, else after the view, as a
    /// plain container is called when it says neither its role nor a name:
    /// an overlay, or a disabled control.
    pub fn name(&self, replica: &Replica, cover: Cover, at: Point) -> CoverName {
        match cover {
            Cover::Window(m) => CoverName {
                by: format!(
                    "dialog {:?}",
                    replica.windows[m].title.as_deref().unwrap_or_default()
                ),
                drawn_over: false,
            },
            Cover::Node { w, i, noun } => CoverName {
                by: cover_label(&replica.windows[w].nodes[i], &self.windows[w][i], noun),
                drawn_over: false,
            },
            Cover::DrawnOver { w, s } => {
                let window = &replica.windows[w];
                let lies_under = holder(window, &self.windows[w], s, at);
                CoverName {
                    by: cover_label(
                        &window.nodes[lies_under],
                        &self.windows[w][lies_under],
                        "overlay",
                    ),
                    drawn_over: true,
                }
            }
        }
    }
}

impl View {
    /// How errors name node `b` of window `w`, a bar drawn over a node, as
    /// what a tap at the node's centre would land on: by its role and name.
    pub fn name_bar(&self, replica: &Replica, w: usize, b: usize) -> CoverName {
        CoverName {
            by: cover_label(&replica.windows[w].nodes[b], &self.windows[w][b], "bar"),
            drawn_over: true,
        }
    }
}

/// The covers a view's passes record: what covers each node it hid, and
/// what lies over part of a node.
#[derive(Default)]
struct Covers {
    whole: Vec<(usize, usize, Cover)>,
    partial: Vec<(usize, usize, Cover)>,
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
    tabs_by_row(window, &mut out);
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
    let mut role = roles::role(&node.class, node.flags, text.is_some() || desc.is_some());
    let editable = node.editable() || matches!(role, Role::Textbox | Role::Searchbox);
    // React Native says that a view is a checkbox, and whether it is
    // checked, only in its value on iOS: the value is neither the
    // checkbox's name nor a value of its own.
    let announced = match (role, text) {
        (Role::Generic | Role::Group | Role::Text | Role::Image, Some(text)) if !editable => {
            roles::announced_checkbox(text)
        }
        _ => None,
    };
    let (name, value) = if announced.is_some() {
        role = Role::Checkbox;
        (desc.map(normalize).unwrap_or_default(), None)
    } else {
        let value = match (desc, text) {
            (_, Some(text)) if editable => Some(text.to_owned()),
            (Some(desc), Some(text)) if text != desc => Some(text.to_owned()),
            _ => None,
        };
        (
            own_label(node, editable).map(normalize).unwrap_or_default(),
            value,
        )
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
        checked: announced
            .unwrap_or_else(|| checkable.then(|| node.flags.contains(NodeFlags::CHECKED))),
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

/// Folds into its parent a node that repeats it: one with the parent's
/// role, name and identifier under a parent with a ref, as XCTest reports
/// UIKit's nested alert buttons; and the only child of a text or a heading
/// that repeats its name, as text or in its role, without an identifier
/// of its own, as XCTest reports React Native's text, a heading's without
/// the heading's identifier, and UIKit's scroll bars (rn-app-fixes, design
/// D35).
fn fold_duplicates(window: &Win, facts: &mut [Facts]) {
    for (i, node) in window.nodes.iter().enumerate() {
        let Some(parent) = node.parent else {
            continue;
        };
        let (p, c) = (&facts[parent], &facts[i]);
        if p.absorbed || p.name != c.name {
            continue;
        }
        let same_identifier = window.nodes[parent].identifier == node.identifier;
        let repeated = p.bears_ref && p.role == c.role && same_identifier;
        let own_text = !c.name.is_empty()
            && window.nodes[parent].children.len() == 1
            && matches!(p.role, Role::Text | Role::Heading)
            && (c.role == p.role || c.role == Role::Text)
            && (node.identifier.is_none() || same_identifier);
        if repeated || own_text {
            let folded = &mut facts[i];
            folded.absorbed = true;
            folded.bears_ref = false;
        }
    }
}

/// Whether a node takes text.
pub(crate) fn field(node: &RNode, role: Role) -> bool {
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
/// agent-device lists these nodes as `Other` and `Cell`. A selected one
/// gets a ref whatever it holds: React Native exposes a tab so on iOS,
/// with no trait but `selected`, and the selected tab may show more than
/// the others, as its label under its icon (rn-app-fixes, design D35).
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
        let selected = node.flags.contains(NodeFlags::SELECTED);
        if (selected || node.children.iter().all(|&c| static_text(c))) && !held {
            facts[i].bears_ref = true;
        }
    }
}

/// Lists as tabs the labelled nodes without traits laid out in one row
/// with a selected one, the selected one with its state (rn-app-fixes,
/// design D40): React Native exposes a tab so on iOS, the `selected` trait
/// on the selected tab alone, and the production-app run found an unselected
/// tab holding its icon's text, a text whose only child is its own text,
/// with nothing else to tell it a control by. A row is the siblings whose
/// bounds overlap the selected one's over at least half the height of the
/// shorter; a node an actionable element holds is its content, and a row
/// without a selected node changes nothing.
fn tabs_by_row(window: &Win, facts: &mut [Facts]) {
    for parent in &window.nodes {
        let children = &parent.children;
        if children.len() < 2 {
            continue;
        }
        let labelled = |c: usize, facts: &[Facts]| {
            matches!(facts[c].role, Role::Generic | Role::Group)
                && !facts[c].name.is_empty()
                && !facts[c].absorbed
        };
        let Some(&selected) = children
            .iter()
            .find(|&&c| labelled(c, facts) && window.nodes[c].flags.contains(NodeFlags::SELECTED))
        else {
            continue;
        };
        let row = facts[selected].rect;
        let held =
            std::iter::successors(window.nodes[selected].parent, |&a| window.nodes[a].parent)
                .any(|a| facts[a].actionable);
        if held {
            continue;
        }
        for &c in children {
            let r = facts[c].rect;
            let shared = r.bottom.min(row.bottom) - r.top.max(row.top);
            if labelled(c, facts) && shared >= r.height().min(row.height()) / 2.0 {
                let tab = &mut facts[c];
                tab.role = Role::Tab;
                tab.actionable = true;
                tab.bears_ref = true;
            }
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
/// holds inside is hidden. Records the view as the cover of what it hid,
/// and of what it overlaps without holding.
fn cover_by_modal_views(replica: &Replica, facts: &mut [Vec<Facts>], covers: &mut Covers) {
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
                    covers.whole.push((w, d, cover));
                } else if covered.hidden == Hidden::No && covered.rect.overlaps(&area) {
                    covers.partial.push((w, d, cover));
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
/// its bottom. Records the view as the cover of what it hid, and of what it
/// overlaps without holding.
fn cover_by_views_on_top(replica: &Replica, facts: &mut [Vec<Facts>], covers: &mut Covers) {
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
            let spared = |_: usize, _: &Facts| false;
            hide_under(
                window,
                &mut facts[w],
                s,
                &mut stack,
                slack,
                spared,
                |d, whole| {
                    record(covers, whole, (w, d, Cover::DrawnOver { w, s }));
                },
            );
        }
    }
}

/// Records that `cover` covers its node, whole or in part.
fn record(covers: &mut Covers, whole: bool, cover: (usize, usize, Cover)) {
    if whole {
        covers.whole.push(cover);
    } else {
        covers.partial.push(cover);
    }
}

/// Hides the nodes on `stack` and below them that lie within the bounds of
/// node `s` of `window`, a view drawn over them, and calls `hid` with each
/// and `true`, and with each it overlaps without holding and `false`. What
/// a node holds lies within its bounds as Android clips them, so the walk
/// does not enter a node without an area in the view's bounds. iOS reports
/// whole frames, and a node there can hold one outside its own bounds; such
/// a node lies outside the clip of a scroll container it overflows, out of
/// view rather than covered, and keeps counting in the container's hint.
/// The walk leaves alone the nodes `spare` names, with what they hold.
fn hide_under(
    window: &Win,
    facts: &mut [Facts],
    s: usize,
    stack: &mut Vec<usize>,
    slack: f64,
    spare: impl Fn(usize, &Facts) -> bool,
    mut hid: impl FnMut(usize, bool),
) {
    let area = facts[s].rect;
    while let Some(d) = stack.pop() {
        let covered = &mut facts[d];
        if !covered.rect.overlaps(&area) || spare(d, covered) {
            continue;
        }
        if covered.hidden == Hidden::No {
            let whole = within(&covered.rect, &area, slack);
            if whole {
                covered.hidden = Hidden::Covered;
            }
            hid(d, whole);
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
/// An alert's static text is spared: UIKit's alert controller draws
/// nothing over its title and message, while XCTest lists its scroll
/// views' indicators after the text they scroll, their accessibility frames
/// 30-point hit areas along the view's edges, which UIKit draws as bars a
/// few points thick, when at all. The horizontal one's frame holds the
/// message on an iPhone 17 Simulator and on the SE, which hid the message
/// from the listing (physical-ios, design D26). For the same reason a
/// scroll indicator ([`SCROLL_INDICATOR`]) covers nothing but its own copy,
/// which XCTest lists after it, with one frame: over part of an alert's
/// button its frame left a tap 61% of it, which the interactive listing
/// marks (rn-app-fixes, design D45).
///
/// A view that holds nothing and names nothing covers what it lies over of
/// a scroll container its earlier sibling holds or is, when it lies over
/// part of it ([`over_part_of_a_list`]): React Native's accessibility lists
/// a view that draws something, such as a footer's background, as a node
/// holding nothing, the content it lays out listed after it as its
/// siblings, and leaves out a view that draws nothing, so the footer over a
/// list's lower part shows nothing in the tree while a tap on it lands on it
/// (rn-app-fixes, design D48).
///
/// A view listed after a sibling that its frame holds whole, that shows
/// nothing itself and is no bar, is a host: it covers what shows in it
/// ([`drawn_parts`]), each part recorded as the view over what it covers,
/// not its frame, which XCTest gives it whole while it draws only what it
/// holds (real-app-fixes, design D2). On iOS 26 the container a navigation
/// controller's floating bottom toolbar lives in spans the window, listed
/// after the content, and draws only the toolbar, 86 points at the
/// screen's bottom; a sheet's list, in a view of the sheet's content frame
/// listed after the search bar's view, has that frame while its first cell
/// starts 73 points lower, under the search bar (spike S14). A view listed
/// after a sibling it overlaps only in part is a bar or a float, a footer,
/// a header or a floating button laid out over one edge of what it is
/// drawn over, and covers its frame.
///
/// The covers are taken top-most first, parents before their children,
/// later siblings before earlier ones and a host's parts the last drawn
/// first, so a node two views cover is recorded under the one drawn last.
/// Overlapping siblings are found by [`overlapping`]. A header the app pins
/// across a scrolling sibling's top ([`pinned_over`]) is drawn over that
/// sibling whatever the order, and the sibling covers none of it.
fn cover_by_later_siblings(replica: &Replica, facts: &mut [Vec<Facts>], covers: &mut Covers) {
    let slack = pixel(replica);
    let mut sweep = Sweep::default();
    let mut stack = Vec::new();
    // What the host met last draws ([`drawn_parts`]).
    let mut parts = Vec::new();
    for (w, window) in replica.windows.iter().enumerate() {
        if window.kind == WindowType::INPUT_METHOD {
            continue;
        }
        let widest = window.nodes.iter().map(|n| n.children.len()).max();
        sweep.active.reserve(widest.unwrap_or(0));
        let mut shows: Option<Vec<bool>> = None;
        let mut alerted: Option<Vec<bool>> = None;
        let mut lists: Option<Vec<usize>> = None;
        let mut hosted: Option<usize> = None;
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
            let alerted = alerted.get_or_insert_with(|| in_alerts(window, &facts[w]));
            for &(earlier, later) in pairs.iter() {
                let (e, s) = (children[earlier], children[later]);
                let draws = shows[s] || {
                    let lists = lists.get_or_insert_with(|| scrolling_nodes(window, &facts[w]));
                    over_part_of_a_list(window, &facts[w], lists, (e, s), slack)
                };
                let f = &facts[w][s];
                if !draws
                    || !f.visible
                    || f.hidden != Hidden::No
                    || pinned_over(window, &facts[w], e, s, slack)
                {
                    continue;
                }
                let node = &window.nodes[s];
                let host = shows[s]
                    && !shows_itself(node, f)
                    && !BAR_CLASSES.contains(&&*node.class)
                    && within(&facts[w][e].rect, &f.rect, slack);
                if host && hosted != Some(s) {
                    parts.clear();
                    drawn_parts(window, &facts[w], shows, s, &mut parts);
                    hosted = Some(s);
                }
                let covering: &[usize] = if host {
                    &parts
                } else {
                    std::slice::from_ref(&s)
                };
                let indicator = |n: usize| &*window.nodes[n].class == SCROLL_INDICATOR;
                let spared = |d: usize, f: &Facts| {
                    matches!(f.role, Role::Tablist | Role::Toolbar)
                        || (alerted[d] && &*window.nodes[d].class == roles::XCUI_STATIC_TEXT)
                        || (indicator(s) && !indicator(d))
                };
                for &c in covering.iter().rev() {
                    let cover = &facts[w][c];
                    if !cover.visible || cover.hidden != Hidden::No {
                        continue;
                    }
                    stack.clear();
                    stack.push(e);
                    hide_under(
                        window,
                        &mut facts[w],
                        c,
                        &mut stack,
                        slack,
                        spared,
                        |d, whole| {
                            record(covers, whole, (w, d, Cover::DrawnOver { w, s: c }));
                        },
                    );
                }
            }
        }
    }
}

/// What node `s` of `window`, a host ([`cover_by_later_siblings`]), draws,
/// as far as the tree tells, `shows` saying what shows something
/// ([`showing`]): the nodes below it, in pre-order, that draw, each over
/// its bounds: a bar ([`BAR_CLASSES`]), the children of a scroll container
/// that show something, its cells, rather than the container, whose frame
/// takes in the insets it lays its content out under, and a node that
/// shows something itself ([`shows_itself`]); a scroll indicator, which
/// covers nothing but its own copy, and a node that shows nothing draw
/// nothing (real-app-fixes, design D2).
fn drawn_parts(window: &Win, facts: &[Facts], shows: &[bool], s: usize, parts: &mut Vec<usize>) {
    let indicator = |n: usize| &*window.nodes[n].class == SCROLL_INDICATOR;
    let mut stack = vec![s];
    while let Some(n) = stack.pop() {
        let node = &window.nodes[n];
        if !shows[n] || indicator(n) {
            continue;
        }
        if n != s && BAR_CLASSES.contains(&&*node.class) {
            parts.push(n);
        } else if scrolls(node, &facts[n]) {
            parts.extend(
                node.children
                    .iter()
                    .copied()
                    .filter(|&c| shows[c] && !indicator(c)),
            );
        } else if n != s && shows_itself(node, &facts[n]) {
            parts.push(n);
        } else {
            stack.extend(node.children.iter().rev().copied());
        }
    }
}

/// The nodes of `window` that scroll ([`scrolls`]), in pre-order.
fn scrolling_nodes(window: &Win, facts: &[Facts]) -> Vec<usize> {
    (0..window.nodes.len())
        .filter(|&c| scrolls(&window.nodes[c], &facts[c]))
        .collect()
}

/// Whether node `s` of `window`, listed after its sibling `e`, is a view
/// that holds nothing and names nothing, without a ref or an identifier,
/// lying over part of a scroll container among `lists` that `e` holds or
/// is: over it, without holding all of it, as a background listed after a
/// list would (rn-app-fixes, design D48). On the creation tab of the fourth
/// production-app run the footer's view, which React Native's accessibility
/// lists holding nothing, its tabs after it, lies over the lower part of a
/// scroll view that runs on to the bottom of the screen.
fn over_part_of_a_list(
    window: &Win,
    facts: &[Facts],
    lists: &[usize],
    (e, s): (usize, usize),
    slack: f64,
) -> bool {
    let (node, f) = (&window.nodes[s], &facts[s]);
    if !node.children.is_empty()
        || !f.name.is_empty()
        || f.bears_ref
        || node.identifier.is_some()
        || f.rect.is_empty()
    {
        return false;
    }
    lists.iter().any(|&c| {
        let list = &facts[c].rect;
        list.overlaps(&f.rect)
            && !within(list, &f.rect, slack)
            && std::iter::successors(Some(c), |&a| window.nodes[a].parent).any(|a| a == e)
    })
}

/// The class of a scroll view's indicator in XCTest's tree, its frame a
/// 30-point hit area along the view's edge that UIKit draws as a bar a few
/// points thick while the view scrolls (physical-ios, design D26).
const SCROLL_INDICATOR: &str = "_UIScrollViewScrollIndicator";

/// The classes of the bars UIKit draws over the content its controllers
/// lay out under them, which XCTest lists before or after that content.
const BAR_CLASSES: [&str; 3] = [
    "XCUIElementTypeNavigationBar",
    "XCUIElementTypeTabBar",
    "XCUIElementTypeToolbar",
];

/// Whether node `h` of `window`, listed before its sibling `c`, is a header
/// the app pins across `c`'s top: `c` scrolls and `h` does not, and `h`
/// spans `c`'s width from an edge at or above `c`'s top to one at or below
/// it, short of `c`'s bottom. A React Native header listed before its list
/// is drawn over what the list scrolls up past its top, which XCTest's
/// whole frames put in the header's frame.
fn pinned_over(window: &Win, facts: &[Facts], h: usize, c: usize, slack: f64) -> bool {
    scrolls(&window.nodes[c], &facts[c])
        && !scrolls(&window.nodes[h], &facts[h])
        && across_top(&facts[h].rect, &facts[c].rect, slack)
}

/// Hides, in a tree whose order is its drawing order ([`drawn_in_order`]),
/// what a bar or a pinned header covers of the siblings listed after it,
/// whatever the order (rn-app-fixes, design D38). XCTest lists a navigation
/// bar before the content its controller draws it over, and a React Native
/// header before the list it is drawn over, the rows the list scrolled up
/// past its top lying in the header's frame (the production-app run's 44-point
/// row at 2 points): a bar by its class ([`BAR_CLASSES`]) covers what its
/// later siblings hold within its bounds, and a header pinned across a
/// scrolling sibling's top ([`pinned_over`]) what that sibling holds within
/// its bounds, when the header shows something. A sibling is entered
/// whatever its own bounds, since what it holds may lie outside them; below
/// it the walk follows [`hide_under`]. A parent's children are compared
/// only where one of them is a bar or scrolls, so a grid's cells cost
/// nothing.
fn cover_by_bars_and_headers(replica: &Replica, facts: &mut [Vec<Facts>], covers: &mut Covers) {
    let slack = pixel(replica);
    let mut stack = Vec::new();
    for (w, window) in replica.windows.iter().enumerate() {
        if window.kind == WindowType::INPUT_METHOD {
            continue;
        }
        let bar = |n: usize| BAR_CLASSES.contains(&&*window.nodes[n].class);
        let mut shows: Option<Vec<bool>> = None;
        for parent in 0..window.nodes.len() {
            let children = &window.nodes[parent].children;
            if children.len() < 2 {
                continue;
            }
            for (k, &c) in children.iter().enumerate() {
                if scrolls(&window.nodes[c], &facts[w][c]) {
                    for &h in &children[..k] {
                        if pinned_over(window, &facts[w], h, c, slack) {
                            let shows = shows.get_or_insert_with(|| showing(window, &facts[w]));
                            let pair = (w, h, c);
                            cover_sibling(
                                window,
                                &mut facts[w],
                                shows,
                                pair,
                                slack,
                                &mut stack,
                                covers,
                            );
                        }
                    }
                }
                if bar(c) {
                    for &s in &children[k + 1..] {
                        let shows = shows.get_or_insert_with(|| showing(window, &facts[w]));
                        let pair = (w, c, s);
                        cover_sibling(
                            window,
                            &mut facts[w],
                            shows,
                            pair,
                            slack,
                            &mut stack,
                            covers,
                        );
                    }
                }
            }
        }
    }
}

/// Hides what node `cover` of window `w`, a bar or a pinned header, covers
/// of its sibling `under` when it is on screen, not covered and shows
/// something, as `shows` says: the sibling itself when it lies within the
/// cover's bounds, and below it what [`hide_under`] finds, the sibling
/// entered whatever its own bounds.
fn cover_sibling(
    window: &Win,
    facts: &mut [Facts],
    shows: &[bool],
    (w, cover, under): (usize, usize, usize),
    slack: f64,
    stack: &mut Vec<usize>,
    covers: &mut Covers,
) {
    let f = &facts[cover];
    if !shows[cover] || !f.visible || f.hidden != Hidden::No {
        return;
    }
    let area = f.rect;
    let sibling = &mut facts[under];
    if sibling.hidden == Hidden::No && sibling.rect.overlaps(&area) {
        let whole = within(&sibling.rect, &area, slack);
        if whole {
            sibling.hidden = Hidden::Covered;
        }
        record(covers, whole, (w, under, Cover::DrawnOver { w, s: cover }));
    }
    stack.clear();
    stack.extend(window.nodes[under].children.iter().copied());
    let spared = |_: usize, f: &Facts| matches!(f.role, Role::Tablist | Role::Toolbar);
    hide_under(window, facts, cover, stack, slack, spared, |d, whole| {
        record(covers, whole, (w, d, Cover::DrawnOver { w, s: cover }));
    });
}

/// Whether each node of `window` is an alert or lies inside one.
fn in_alerts(window: &Win, facts: &[Facts]) -> Vec<bool> {
    let mut alerted = Vec::with_capacity(window.nodes.len());
    // Parents come before their children in the pre-order.
    for (node, f) in window.nodes.iter().zip(facts) {
        let inside = node.parent.is_some_and(|parent| alerted[parent]);
        alerted.push(inside || f.role == Role::Alert);
    }
    alerted
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

/// The node of `window` that a tap at `at` would land on below node
/// `cover`, a view drawn over it: the innermost node of the view, or the
/// one drawn last, whose bounds hold `at` and that shows something itself
/// ([`shows_itself`]); else the view itself.
fn holder(window: &Win, facts: &[Facts], cover: usize, at: Point) -> usize {
    let mut found = cover;
    let mut stack = vec![cover];
    while let Some(n) = stack.pop() {
        let f = &facts[n];
        if !f.rect.contains(at) {
            continue;
        }
        if shows_itself(&window.nodes[n], f) && n > found {
            found = n;
        }
        stack.extend(window.nodes[n].children.iter().copied());
    }
    found
}

/// Whether a node shows something itself: it is on screen with a ref or a
/// name, or a disabled control, which UIKit hit-tests as an enabled one
/// (rn-app-fixes, design D38).
fn shows_itself(node: &RNode, facts: &Facts) -> bool {
    facts.visible
        && (facts.bears_ref || !facts.name.is_empty() || !node.flags.contains(NodeFlags::ENABLED))
}

/// Whether each node of `window`, or a node below it, shows something
/// itself ([`shows_itself`]).
fn showing(window: &Win, facts: &[Facts]) -> Vec<bool> {
    let mut shows: Vec<bool> = window
        .nodes
        .iter()
        .zip(facts)
        .map(|(node, f)| shows_itself(node, f))
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
/// plain container, or as a disabled control when it is one.
fn cover_label(node: &RNode, facts: &Facts, noun: &str) -> String {
    let plain = matches!(facts.role, Role::Generic | Role::Group);
    let noun = if node.flags.contains(NodeFlags::ENABLED) {
        noun
    } else {
        "disabled control"
    };
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
/// An element is counted when the listing leaves it out: one off the
/// screen, and, in an `interactive` listing, one outside its container's
/// bounds or one the listing leaves out for a cover, one a view drawn over
/// it covers whole or one a tap reaches none of, as `reach` has the share
/// of each element a tap reaches ([`crate::visibility::listed`]); such an
/// element inside the container's bounds counts on the side of the
/// container's middle it lies on along the axis the container scrolls
/// (rn-app-fixes, design D45). The full listing keeps every element with a
/// ref on the screen, at visible=0% when a tap reaches none of it, a view
/// covering it whole or not, and counts none of them (design D57). The
/// interactive listing's hint names, beside what is scrolled away, the cover
/// of each element on the screen it counts as left out ([`hint_cover`],
/// design D50), so that its hints count what the full listing shows at
/// visible=0% (designs D54 and D57).
pub(crate) fn hints(
    replica: &Replica,
    view: &View,
    reach: &Reaches<'_>,
    interactive: bool,
) -> Hints {
    let mut out = Hints::default();
    let slack = pixel(replica);
    for (w, window) in replica.windows.iter().enumerate() {
        let facts = &view.windows[w];
        let area = Rect::from_pixels(window.bounds, replica.scale);
        let container = |i: usize| scrolls(&window.nodes[i], &facts[i]);
        let mut counts: Vec<[u32; 4]> = vec![[0; 4]; window.nodes.len()];
        // What covers the counted elements a tap reaches none of: the
        // container, the side and how the hint names the cover.
        let mut covered: Vec<(usize, usize, String)> = Vec::new();
        for x in 0..window.nodes.len() {
            let f = &facts[x];
            let whole = match f.hidden {
                Hidden::No => None,
                Hidden::Covered => match view.whole_cover(w, x) {
                    Some(Cover::DrawnOver { s, .. }) => Some(s),
                    _ => continue,
                },
                Hidden::Keyboard => continue,
            };
            if !f.bears_ref || f.absorbed {
                continue;
            }
            // What the listing leaves out for a cover: the interactive
            // listing what a view covers whole and what a tap reaches none
            // of, which the full listing keeps at visible=0% (design D57).
            let left_out = interactive && (whole.is_some() || reach.of(w, x) == Some(0.0));
            let reached = !left_out;
            let Some(s) = owner(window, facts, x) else {
                if left_out
                    && f.visible
                    && let Some((Some(c), by)) =
                        hint_cover(replica, view, (w, x), whole, None, slack)
                {
                    match out
                        .covered
                        .iter_mut()
                        .find(|(cw, cc, named, _)| (*cw, *cc) == (w, c) && *named == by)
                    {
                        Some((_, _, _, n)) => *n += 1,
                        None => out.covered.push((w, c, by, 1)),
                    }
                }
                continue;
            };
            // A container off the screen or covered gets no hint.
            if !facts[s].visible || facts[s].hidden != Hidden::No {
                continue;
            }
            let viewport = facts[s].rect.intersect(&area);
            if f.visible && f.rect.overlaps(&viewport) && reached {
                continue;
            }
            // The full listing shows every element with a ref on the screen
            // and counts none of them (design D57).
            if !interactive && f.visible {
                continue;
            }
            let r = &f.rect;
            let horizontal = window.nodes[s]
                .scroll
                .is_some_and(|scroll| scroll.axis == ScrollAxis::HORIZONTAL);
            let (middle, centre) = (viewport.center(), r.center());
            let direction = if r.top >= viewport.bottom {
                Direction::Below
            } else if r.bottom <= viewport.top {
                Direction::Above
            } else if r.left >= viewport.right {
                Direction::Right
            } else if r.right <= viewport.left {
                Direction::Left
            } else if reached {
                continue;
            } else if horizontal {
                if centre.x < middle.x {
                    Direction::Left
                } else {
                    Direction::Right
                }
            } else if centre.y < middle.y {
                Direction::Above
            } else {
                Direction::Below
            };
            counts[s][slot(direction)] += 1;
            // A cover's count holds only what a listing could show, an
            // element on the screen (design D57).
            if left_out
                && f.visible
                && let Some((_, by)) =
                    hint_cover(replica, view, (w, x), whole, Some((s, direction)), slack)
            {
                covered.push((s, slot(direction), by));
            }
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
                    let mut by: Vec<(String, u32)> = Vec::new();
                    for (_, _, cover) in covered
                        .iter()
                        .filter(|(c, side, _)| (*c, *side) == (s, slot(direction)))
                    {
                        match by.iter_mut().find(|(named, _)| named == cover) {
                            Some((_, n)) => *n += 1,
                            None => by.push((cover.clone(), 1)),
                        }
                    }
                    out.scroll.push((
                        w,
                        s,
                        ScrollHint {
                            container: node.element,
                            window: window.id,
                            direction,
                            count,
                            source,
                            covered: by,
                        },
                    ));
                }
            }
        }
    }
    out
}

/// The scroll container node `i` of `window` counts in, `facts` being the
/// window's: the nearest above it, unless an element with a ref on the way
/// stands for it; `None` for one outside any.
fn owner(window: &Win, facts: &[Facts], i: usize) -> Option<usize> {
    let mut up = window.nodes[i].parent;
    while let Some(a) = up {
        if scrolls(&window.nodes[a], &facts[a]) {
            return Some(a);
        }
        if facts[a].bears_ref && !facts[a].absorbed {
            return None;
        }
        up = window.nodes[a].parent;
    }
    None
}

/// The edge node `c` of `window` keeps of what scroll container `list`
/// shows, as hints and refusals name it (designs D50, D54, D55 and D57):
/// `"header"` for a view listed before the container across its top,
/// whatever it holds, or a plain view across the top of `area`, the
/// container's bounds or, for an element outside any container, its
/// window's, short of its bottom; `"footer"` for a plain view across the
/// bottom of `area`, short of its top; `None` for any other view, a plain
/// one over all of `area` among them, as Bluesky's typeahead lies over its
/// page, and for a window over the application's own, as the text effects
/// window holding iOS's edit menu is.
fn edge(
    window: &Win,
    facts: &[Facts],
    c: usize,
    (list, area): (Option<usize>, &Rect),
    slack: f64,
) -> Option<&'static str> {
    let (node, f) = (&window.nodes[c], &facts[c]);
    if &*node.class == roles::XCUI_WINDOW {
        return None;
    }
    let plain = matches!(f.role, Role::Generic | Role::Group)
        && f.name.is_empty()
        && node.flags.contains(NodeFlags::ENABLED);
    let across = f.rect.left <= area.left + slack && f.rect.right >= area.right - slack;
    let (top, bottom) = (
        f.rect.top <= area.top + slack,
        f.rect.bottom >= area.bottom - slack,
    );
    if list.is_some_and(|s| heads(window, facts, c, s, slack))
        || (plain && across && top && !bottom)
    {
        Some("header")
    } else if plain && across && bottom && !top {
        Some("footer")
    } else {
        None
    }
}

/// What leaves a tap none of node `i` of window `w`, an element inside the
/// scroll container `container` names, with the side of it the element
/// counts on, or outside any, and how a hint names it: the view drawn over
/// all of it, `whole`, else the view drawn over part of it that holds its
/// centre, a tab bar or toolbar that does, or the keyboard, which is no
/// node; for an element nothing covers at its centre, out of what its
/// container shows above or below, the container's header or footer on
/// that side ([`edge_view`]); `None` when there is none, the scroll keeping
/// it out of view. The hint names a view as a refusal does, by one rule
/// ([`View::cover_name`], design D57): the header the element lies behind,
/// the footer it lies under, or what a tap at its centre would land on.
fn hint_cover(
    replica: &Replica,
    view: &View,
    (w, i): (usize, usize),
    whole: Option<usize>,
    container: Option<(usize, Direction)>,
    slack: f64,
) -> Option<(Option<usize>, String)> {
    /// What lies over the element's centre: a view, a tab bar or toolbar,
    /// or the container's edge beyond what it shows.
    enum Over {
        View(Cover),
        Bar(usize),
        Edge(usize),
    }
    let (window, facts) = (&replica.windows[w], &view.windows[w]);
    let centre = facts[i].rect.center();
    let over = whole
        .map(|s| Over::View(Cover::DrawnOver { w, s }))
        .or_else(|| {
            view.over_part(w, i)
                .filter(|(_, rect)| rect.contains(centre))
                .last()
                .and_then(|(cover, _)| match cover {
                    Cover::Window(_) => None,
                    cover => Some(Over::View(cover)),
                })
        })
        .or_else(|| {
            (0..window.nodes.len())
                .find(|&b| {
                    let f = &facts[b];
                    matches!(f.role, Role::Tablist | Role::Toolbar)
                        && f.visible
                        && f.hidden == Hidden::No
                        && f.rect.contains(centre)
                })
                .map(Over::Bar)
        });
    if over.is_none()
        && crate::visibility::keyboards_above(replica, w)
            .iter()
            .any(|keyboard| keyboard.contains(centre))
    {
        return Some((None, format!("under the {}", crate::visibility::KEYBOARD)));
    }
    let over = over.or_else(|| {
        container
            .and_then(|(s, side)| edge_view(window, facts, s, side, slack))
            .map(Over::Edge)
    })?;
    let (c, named) = match over {
        Over::View(cover) => {
            let c = match cover {
                Cover::DrawnOver { s, .. } => s,
                Cover::Node { i, .. } => i,
                Cover::Window(_) => unreachable!("a modal window is no view over part"),
            };
            (c, view.cover_name(replica, (w, i), cover, centre))
        }
        Over::Bar(b) => (b, view.name_bar(replica, w, b)),
        Over::Edge(c) => (
            c,
            view.cover_name(replica, (w, i), Cover::DrawnOver { w, s: c }, centre),
        ),
    };
    let by = if named.by == "header" {
        "behind the header".to_owned()
    } else {
        format!("under the {}", named.by)
    };
    Some((Some(c), by))
}

/// Whether node `h` of `window`, listed before node `s` and not holding it,
/// lies across `s`'s top: it spans `s`'s width from an edge at or above
/// `s`'s top to one at or below it, short of its bottom, `facts` being the
/// window's. A React Native header listed before its list is so.
fn heads(window: &Win, facts: &[Facts], h: usize, s: usize, slack: f64) -> bool {
    h < s && !holds(window, h, s) && across_top(&facts[h].rect, &facts[s].rect, slack)
}

/// Whether node `outer` of `window` is node `inner` or one of its
/// ancestors.
fn holds(window: &Win, outer: usize, inner: usize) -> bool {
    std::iter::successors(Some(inner), |&a| window.nodes[a].parent).any(|a| a == outer)
}

/// Whether `header` spans `list`'s width from an edge at or above `list`'s
/// top to one at or below it, short of `list`'s bottom.
fn across_top(header: &Rect, list: &Rect, slack: f64) -> bool {
    header.left <= list.left + slack
        && header.right >= list.right - slack
        && header.top <= list.top + slack
        && header.bottom >= list.top - slack
        && header.bottom < list.bottom - slack
}

/// The view on `side` of scroll container `s` of `window` that keeps what
/// the container shows from what lies beyond it there, `facts` being the
/// window's: above, its header, a node on screen across its top
/// ([`across_top`]); below, its footer, a node on screen that spans its
/// width from inside it to its bottom or below; of those neither holding
/// the container nor held by it nor a window (design D55), the one listed
/// last. On the fifth
/// production-app run rows a list scrolled above its frame lay on the screen
/// above the header, where the status bar is drawn, and nothing over their
/// centres named a cover (rn-app-fixes, design D54).
fn edge_view(
    window: &Win,
    facts: &[Facts],
    s: usize,
    side: Direction,
    slack: f64,
) -> Option<usize> {
    let list = &facts[s].rect;
    let edge = |rect: &Rect| match side {
        Direction::Above => across_top(rect, list, slack),
        Direction::Below => {
            rect.left <= list.left + slack
                && rect.right >= list.right - slack
                && rect.top > list.top + slack
                && rect.top <= list.bottom + slack
                && rect.bottom >= list.bottom - slack
        }
        Direction::Left | Direction::Right => false,
    };
    (0..window.nodes.len()).rev().find(|&n| {
        let f = &facts[n];
        f.visible
            && f.hidden == Hidden::No
            && &*window.nodes[n].class != roles::XCUI_WINDOW
            && !holds(window, n, s)
            && !holds(window, s, n)
            && edge(&f.rect)
    })
}

/// The hints of a listing: the scroll hints, and how many of the elements
/// with refs outside any scroll container the listing leaves out for a view
/// drawn over them, by window, that view's node and how the line names it,
/// which the listing prints after the view (rn-app-fixes, designs D50 and
/// D54).
#[derive(Default)]
pub(crate) struct Hints {
    pub scroll: Vec<(usize, usize, ScrollHint)>,
    pub covered: Vec<(usize, usize, String, u32)>,
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
