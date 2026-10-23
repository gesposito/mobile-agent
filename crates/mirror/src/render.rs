//! Listings: which elements a snapshot shows, at what depth, and the lines
//! agent-browser prints for them.
//!
//! Application windows come first, then system bars, then the keyboard,
//! which is one `- keyboard` line, or in an interactive listing a line that
//! counts what lies wholly under it; a keyboard window that draws nothing,
//! as while a hardware keyboard serves the field in focus, is no keyboard
//! and gets no line (rn-app-fixes, design D52). Within a window, a node is
//! listed when it is visible and not absorbed or covered, and, in an
//! interactive listing, when it has a ref, as agent-browser's `-i`, does not
//! lie wholly under the keyboard (rn-app-fixes, design D38) and has a part a
//! tap reaches (design D45), the full listing keeping one a tap reaches
//! none of and both marking the share a tap reaches (design D50); in a full
//! listing, unnamed generic containers, containers whose label names the
//! field inside them, unnamed text and images and window roots are skipped
//! and their children listed in their place. A node a view drawn over it
//! covers whole is left out of the interactive listing and kept in the
//! full one at visible=0% when it has a ref, a modal's leaving it out of
//! both (rn-app-fixes, design D57). Scroll hints sit next to their
//! container's children: content above or to the left before them, below
//! or to the right after them.

use protocol::snapshot::{WindowFlags, WindowType};

use crate::geometry::Rect;
use crate::json;
use crate::replica::{RNode, Replica};
use crate::snapshot::{Direction, Element, Role, ScrollHint, SnapshotOptions};
use crate::view::{Facts, Hidden, Hints, View};
use crate::visibility::{Reaches, beside, keyboards_above};

/// One line of a listing.
pub(crate) enum Line {
    /// Node `i` of window `w`, and the share of it in view, in whole percent,
    /// when only part of it is: beside the keyboard, when the keyboard lies
    /// over part of it, and the share a tap reaches, when that is less, 0
    /// only for one a tap reaches none of, which only a full listing lists.
    Element {
        w: usize,
        i: usize,
        depth: usize,
        visible: Option<u8>,
    },
    /// A keyboard window.
    Keyboard { w: usize },
    /// Hint `index` of the snapshot's hints.
    Hint { index: usize, depth: usize },
    /// How many elements with refs an interactive listing leaves out under
    /// the keyboard.
    UnderKeyboard { count: usize },
    /// Entry `index` of the hints' counts of elements outside any scroll
    /// container that a view drawn over them leaves out.
    Covered { index: usize, depth: usize },
}

/// Window indices in listing order.
fn window_order(replica: &Replica) -> Vec<usize> {
    let mut order: Vec<usize> = (0..replica.windows.len()).collect();
    order.sort_by_key(|&w| {
        let window = &replica.windows[w];
        if window.kind == WindowType::INPUT_METHOD {
            2
        } else if window.kind == WindowType::APPLICATION
            || window.modal
            || window.flags.contains(WindowFlags::ACTIVE)
        {
            0
        } else {
            1
        }
    });
    order
}

/// Whether a full listing shows a node that has no ref.
pub(crate) fn listed_in_full(facts: &Facts, node: &RNode) -> bool {
    if facts.bears_ref {
        return true;
    }
    if node.parent.is_none() {
        return false;
    }
    match facts.role {
        Role::Generic | Role::Group => {
            (!facts.name.is_empty() && !facts.lent) || node.identifier.is_some()
        }
        Role::Text | Role::Image => !facts.name.is_empty() || node.identifier.is_some(),
        _ => true,
    }
}

/// The lines of a listing; `reach` is the share of each element with a ref
/// that a tap reaches ([`crate::visibility::listed`]).
pub(crate) fn listing(
    replica: &Replica,
    view: &View,
    hints: &Hints,
    options: &SnapshotOptions,
    reach: &Reaches<'_>,
) -> Vec<Line> {
    let mut walker = Walker {
        replica,
        view,
        hints,
        options,
        reach,
        lines: Vec::new(),
        under_keyboard: 0,
    };
    for w in window_order(replica) {
        let window = &replica.windows[w];
        if window.kind == WindowType::INPUT_METHOD {
            let draws = !Rect::from_pixels(window.bounds, replica.scale).is_empty();
            if !options.interactive && draws && !window.nodes.is_empty() {
                walker.lines.push(Line::Keyboard { w });
            }
            if options.interactive && walker.under_keyboard > 0 {
                let count = std::mem::take(&mut walker.under_keyboard);
                walker.lines.push(Line::UnderKeyboard { count });
            }
            continue;
        }
        if !window.nodes.is_empty() {
            let keyboards = keyboards_above(replica, w);
            walker.walk(w, 0, 0, &keyboards);
        }
    }
    walker.lines
}

struct Walker<'a> {
    replica: &'a Replica,
    view: &'a View,
    hints: &'a Hints,
    options: &'a SnapshotOptions,
    reach: &'a Reaches<'a>,
    lines: Vec<Line>,
    /// The elements with refs an interactive listing has left out under the
    /// keyboard so far.
    under_keyboard: usize,
}

impl Walker<'_> {
    /// Lists node `i` of window `w` at `depth` and what it holds, the
    /// keyboard windows above the window lying at `keyboards`.
    fn walk(&mut self, w: usize, i: usize, depth: usize, keyboards: &[Rect]) {
        let facts = &self.view.windows[w][i];
        let node = &self.replica.windows[w].nodes[i];
        // A view drawn over the node covers it whole: the full listing keeps
        // it at visible=0%, with what it holds, as the interactive listing's
        // hints count it, but a scroll container, whose hints none count
        // (rn-app-fixes, design D57); the interactive listing leaves it out.
        let drawn_over = facts.hidden == Hidden::Covered
            && !self.options.interactive
            && self.view.drawn_over_whole(w, i)
            && !crate::view::scrolls(node, facts);
        if facts.hidden != Hidden::No && !drawn_over {
            return;
        }
        let beside = beside(&facts.rect, keyboards);
        let under = self.options.interactive && beside.is_some_and(|share| share <= 0.0);
        if under && facts.visible && facts.bears_ref && !facts.absorbed {
            self.under_keyboard += 1;
        }
        let reached = self.reach.of(w, i);
        let shown = facts.visible
            && !facts.absorbed
            && !under
            && !(self.options.interactive && reached == Some(0.0))
            && if self.options.interactive || drawn_over {
                facts.bears_ref
            } else {
                listed_in_full(facts, node)
            };
        if shown {
            if self.options.max_depth.is_some_and(|max| depth > max) {
                return;
            }
            self.lines.push(Line::Element {
                w,
                i,
                depth,
                visible: share_in_view(beside, reached),
            });
        }
        let inner = if shown { depth + 1 } else { depth };
        self.hints_of(w, i, true, inner);
        for &child in &node.children {
            self.walk(w, child, inner, keyboards);
        }
        self.hints_of(w, i, false, inner);
        for (index, (cw, ci, _, _)) in self.hints.covered.iter().enumerate() {
            if (*cw, *ci) == (w, i) {
                self.lines.push(Line::Covered { index, depth });
            }
        }
    }

    /// Lists the hints of container `i`: those before its children or after.
    fn hints_of(&mut self, w: usize, i: usize, before: bool, depth: usize) {
        for (index, (hw, hi, hint)) in self.hints.scroll.iter().enumerate() {
            let leading = matches!(hint.direction, Direction::Above | Direction::Left);
            if *hw == w && *hi == i && leading == before {
                self.lines.push(Line::Hint { index, depth });
            }
        }
    }
}

/// The share of an element in view a listing gives as `visible=`, in whole
/// percent, at least 1 unless a tap reaches none of it: the share `beside`
/// the keyboard, when the keyboard lies over part of it, or the share a tap
/// reaches, `reached`, when that is less (rn-app-fixes, design D45); without
/// the keyboard over it, the share a tap reaches when it is less than all to
/// the whole percent. `None` otherwise.
fn share_in_view(beside: Option<f64>, reached: Option<f64>) -> Option<u8> {
    let percent = |share: f64| {
        if share > 0.0 {
            ((share * 100.0).round() as u8).max(1)
        } else {
            0
        }
    };
    match (beside.filter(|&share| share > 0.0), reached) {
        (Some(share), reached) => Some(percent(reached.map_or(share, |r| share.min(r)))),
        (None, Some(reached)) if (reached * 100.0).round() < 100.0 => Some(percent(reached)),
        _ => None,
    }
}

/// An identifier as an attribute value: bare when it cannot be misread.
fn attribute(value: &str) -> String {
    let plain = !value.is_empty()
        && !value.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, ',' | '[' | ']' | '"' | '\\')
        });
    if plain {
        value.to_owned()
    } else {
        json::string(value)
    }
}

/// An element as agent-browser prints it:
/// `- role "name" [checked=…, selected, disabled, focused, visible=…, id=…, ref=eN]: value`,
/// `visible=` giving the share in view of an element only partly in view.
pub(crate) fn element_line(element: &Element) -> String {
    let mut line = "  ".repeat(element.depth);
    line.push_str("- ");
    line.push_str(element.role.as_str());
    if !element.name.is_empty() {
        line.push(' ');
        json::push_str(&mut line, &element.name);
    }
    let mut attrs: Vec<String> = Vec::new();
    if let Some(checked) = element.checked {
        attrs.push(format!("checked={checked}"));
    }
    if element.selected {
        attrs.push("selected".to_owned());
    }
    if !element.enabled {
        attrs.push("disabled".to_owned());
    }
    if element.focused {
        attrs.push("focused".to_owned());
    }
    if let Some(share) = element.visible {
        attrs.push(format!("visible={share}%"));
    }
    if let Some(identifier) = &element.identifier {
        attrs.push(format!("id={}", attribute(identifier)));
    }
    if let Some(reference) = element.reference {
        attrs.push(format!("ref={reference}"));
    }
    if !attrs.is_empty() {
        line.push_str(" [");
        line.push_str(&attrs.join(", "));
        line.push(']');
    }
    if let Some(value) = &element.value
        && !value.is_empty()
        && *value != element.name
    {
        line.push_str(": ");
        line.extend(
            value
                .chars()
                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c }),
        );
    }
    line
}

/// A hint line at `depth`.
pub(crate) fn hint_line(hint: &ScrollHint, depth: usize) -> String {
    format!("{}{}", "  ".repeat(depth), hint.text())
}

/// The line that counts the `count` elements with refs an interactive
/// listing left out under the keyboard, as a scroll hint counts what lies
/// beyond a container.
pub(crate) fn under_keyboard_line(count: usize) -> String {
    let items = if count == 1 { "item" } else { "items" };
    format!("({count} more actionable {items} under the keyboard, close it to reveal)")
}

/// The line that counts the `count` elements with refs outside any scroll
/// container that a listing left out under a view drawn over them, which
/// `by` names, as a scroll hint names what covers what it counts
/// (rn-app-fixes, designs D50 and D54).
pub(crate) fn covered_line(count: u32, by: &str, depth: usize) -> String {
    let items = if count == 1 { "item" } else { "items" };
    format!(
        "{}({count} more actionable {items} {by})",
        "  ".repeat(depth)
    )
}

/// What an empty listing prints, as agent-browser does.
pub(crate) fn empty(options: &SnapshotOptions) -> &'static str {
    if options.interactive {
        "(no interactive elements)"
    } else {
        "(empty screen)"
    }
}
