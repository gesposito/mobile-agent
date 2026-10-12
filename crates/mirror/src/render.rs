//! Listings: which elements a snapshot shows, at what depth, and the lines
//! agent-browser prints for them.
//!
//! Application windows come first, then system bars, then the keyboard,
//! which is one `- keyboard` line, or in an interactive listing a line that
//! counts what lies wholly under it. Within a window, a node is listed when
//! it is visible and not absorbed or covered, and, in an interactive
//! listing, when it has a ref, as agent-browser's `-i`, and does not lie
//! wholly under the keyboard (rn-app-fixes, design D38); in a full listing, unnamed
//! generic containers, containers whose label names the field inside them,
//! unnamed text and images and window roots are skipped and their children
//! listed in their place. Scroll hints sit next to their
//! container's children: content above or to the left before them, below or
//! to the right after them.

use protocol::snapshot::{WindowFlags, WindowType};

use crate::geometry::Rect;
use crate::json;
use crate::replica::{RNode, Replica};
use crate::snapshot::{Direction, Element, Role, ScrollHint, SnapshotOptions};
use crate::view::{Facts, Hidden, View};

/// One line of a listing.
pub(crate) enum Line {
    /// Node `i` of window `w`, and the share of it beside the keyboard, in
    /// whole percent and at least 1, when the keyboard lies over part of
    /// it.
    Element {
        w: usize,
        i: usize,
        depth: usize,
        beside: Option<u8>,
    },
    /// A keyboard window.
    Keyboard { w: usize },
    /// Hint `index` of the snapshot's hints.
    Hint { index: usize, depth: usize },
    /// How many elements with refs an interactive listing leaves out under
    /// the keyboard.
    UnderKeyboard { count: usize },
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
fn listed_in_full(facts: &Facts, node: &RNode) -> bool {
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

/// The lines of a listing.
pub(crate) fn listing(
    replica: &Replica,
    view: &View,
    hints: &[(usize, usize, ScrollHint)],
    options: &SnapshotOptions,
) -> Vec<Line> {
    let mut walker = Walker {
        replica,
        view,
        hints,
        options,
        lines: Vec::new(),
        under_keyboard: 0,
    };
    for w in window_order(replica) {
        let window = &replica.windows[w];
        if window.kind == WindowType::INPUT_METHOD {
            if !options.interactive && !window.nodes.is_empty() {
                walker.lines.push(Line::Keyboard { w });
            }
            if options.interactive && walker.under_keyboard > 0 {
                let count = std::mem::take(&mut walker.under_keyboard);
                walker.lines.push(Line::UnderKeyboard { count });
            }
            continue;
        }
        if !window.nodes.is_empty() {
            let keyboards: Vec<Rect> = replica.windows[..w]
                .iter()
                .filter(|above| above.kind == WindowType::INPUT_METHOD)
                .map(|above| Rect::from_pixels(above.bounds, replica.scale))
                .collect();
            walker.walk(w, 0, 0, &keyboards);
        }
    }
    walker.lines
}

struct Walker<'a> {
    replica: &'a Replica,
    view: &'a View,
    hints: &'a [(usize, usize, ScrollHint)],
    options: &'a SnapshotOptions,
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
        if facts.hidden != Hidden::No {
            return;
        }
        let node = &self.replica.windows[w].nodes[i];
        let beside = beside(&facts.rect, keyboards);
        let under = self.options.interactive && beside.is_some_and(|share| share <= 0.0);
        if under && facts.visible && facts.bears_ref && !facts.absorbed {
            self.under_keyboard += 1;
        }
        let shown = facts.visible
            && !facts.absorbed
            && !under
            && if self.options.interactive {
                facts.bears_ref
            } else {
                listed_in_full(facts, node)
            };
        if shown {
            if self.options.max_depth.is_some_and(|max| depth > max) {
                return;
            }
            let beside = beside
                .filter(|&share| share > 0.0)
                .map(|share| ((share * 100.0).round() as u8).max(1));
            self.lines.push(Line::Element {
                w,
                i,
                depth,
                beside,
            });
        }
        let inner = if shown { depth + 1 } else { depth };
        self.hints_of(w, i, true, inner);
        for &child in &node.children {
            self.walk(w, child, inner, keyboards);
        }
        self.hints_of(w, i, false, inner);
    }

    /// Lists the hints of container `i`: those before its children or after.
    fn hints_of(&mut self, w: usize, i: usize, before: bool, depth: usize) {
        for (index, (hw, hi, hint)) in self.hints.iter().enumerate() {
            let leading = matches!(hint.direction, Direction::Above | Direction::Left);
            if *hw == w && *hi == i && leading == before {
                self.lines.push(Line::Hint { index, depth });
            }
        }
    }
}

/// The share of `rect` beside the keyboards at `keyboards`, 0 for one wholly
/// under them; `None` when none of them lies over it. The keyboards' bounds
/// are what the drivers say a keyboard draws (tasks 10.21, 10.25).
fn beside(rect: &Rect, keyboards: &[Rect]) -> Option<f64> {
    let area = rect.width() * rect.height();
    if keyboards.is_empty() || area <= 0.0 {
        return None;
    }
    let under: f64 = keyboards
        .iter()
        .map(|keyboard| {
            let shared = rect.intersect(keyboard);
            shared.width() * shared.height()
        })
        .sum();
    (under > 0.0).then(|| (1.0 - under / area).clamp(0.0, 1.0))
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
/// `visible=` giving the share in view of an element the keyboard lies over
/// part of.
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
    if let Some(share) = element.beside_keyboard {
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

/// What an empty listing prints, as agent-browser does.
pub(crate) fn empty(options: &SnapshotOptions) -> &'static str {
    if options.interactive {
        "(no interactive elements)"
    } else {
        "(empty screen)"
    }
}
