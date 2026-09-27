//! What the mirror serves: snapshots of the replica, the elements in them and
//! the deltas it derives between reads.

use std::time::{Duration, Instant};

use protocol::snapshot::WindowType;

use crate::backend::{Owner, ReadReason};
use crate::geometry::Rect;
use crate::refs::Ref;
use crate::verdict::Verdict;

/// What a snapshot lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SnapshotOptions {
    /// Only elements with refs, as agent-browser's `-i`.
    pub interactive: bool,
    /// Deepest rendered level to list, as agent-browser's `-d`.
    pub max_depth: Option<usize>,
}

impl SnapshotOptions {
    /// Every element.
    pub fn full() -> SnapshotOptions {
        SnapshotOptions::default()
    }

    /// Only the elements with refs.
    pub fn interactive() -> SnapshotOptions {
        SnapshotOptions {
            interactive: true,
            max_depth: None,
        }
    }
}

/// An element's identity inside the mirror: stable while the element
/// survives from read to read, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId(pub u64);

/// A snapshot of the replica.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// The replica revision the snapshot shows; it grows with every delta.
    pub revision: u64,
    pub taken_at: Instant,
    pub options: SnapshotOptions,
    /// Physical pixels per device point; every bound is in points.
    pub scale: f64,
    /// Who owns the foreground: the target, or system UI such as a
    /// permission alert.
    pub foreground: Owner,
    /// The backend that produced the data and whether it is degraded.
    pub verdict: Verdict,
    /// The windows, top-most first, with how each is kept current.
    pub windows: Vec<WindowSummary>,
    /// The listed elements in document order.
    pub elements: Vec<Element>,
    /// Actionable content scrolled out of view, per scroll container.
    pub hints: Vec<ScrollHint>,
    /// Refs the previous snapshot listed that this one does not, as
    /// agent-browser's `removedRefs`.
    pub removed_refs: Vec<Ref>,
    /// The listing as agent-browser prints it, one element per line:
    /// `- role "name" [attrs, ref=eN]: value`, nested by indentation, with a
    /// parenthesized line for each scroll hint.
    pub text: String,
}

impl Snapshot {
    /// The first listed element with `identifier`.
    pub fn by_identifier(&self, identifier: &str) -> Option<&Element> {
        self.elements
            .iter()
            .find(|e| e.identifier.as_deref() == Some(identifier))
    }

    /// The first listed element named `name`.
    pub fn by_name(&self, name: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.name == name)
    }

    /// The summary of window `id`.
    pub fn window(&self, id: u32) -> Option<&WindowSummary> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// The listed element with `reference`.
    pub fn by_ref(&self, reference: Ref) -> Option<&Element> {
        self.elements
            .iter()
            .find(|e| e.reference == Some(reference))
    }

    /// The refs listed, in document order.
    pub fn refs(&self) -> Vec<Ref> {
        self.elements.iter().filter_map(|e| e.reference).collect()
    }
}

/// One listed element.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub id: ElementId,
    /// The element's ref, for actionable elements and named content.
    pub reference: Option<Ref>,
    pub role: Role,
    /// The accessible name: the element's own description or text, else
    /// the text of its non-actionable descendants.
    pub name: String,
    /// The test identifier: Android resource id without its package prefix,
    /// iOS accessibility identifier.
    pub identifier: Option<String>,
    /// The value: an editable field's text, or text that differs from the
    /// name.
    pub value: Option<String>,
    /// In device points.
    pub bounds: Rect,
    pub enabled: bool,
    pub focused: bool,
    pub selected: bool,
    /// For checkable elements.
    pub checked: Option<bool>,
    /// Whether the element can be acted on.
    pub actionable: bool,
    /// The window holding the element.
    pub window: u32,
    /// Nesting level in the listing.
    pub depth: usize,
    /// The platform class.
    pub class: String,
}

/// What an element is, with agent-browser's role names where they apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Button,
    Link,
    Textbox,
    Searchbox,
    Checkbox,
    Radio,
    Switch,
    Slider,
    Combobox,
    Tab,
    Menuitem,
    Option,
    Heading,
    Cell,
    Listitem,
    Text,
    Image,
    List,
    Grid,
    Tablist,
    Toolbar,
    Alert,
    Dialog,
    Progressbar,
    Keyboard,
    Webview,
    Group,
    Generic,
}

impl Role {
    /// The role as snapshots print it.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Button => "button",
            Role::Link => "link",
            Role::Textbox => "textbox",
            Role::Searchbox => "searchbox",
            Role::Checkbox => "checkbox",
            Role::Radio => "radio",
            Role::Switch => "switch",
            Role::Slider => "slider",
            Role::Combobox => "combobox",
            Role::Tab => "tab",
            Role::Menuitem => "menuitem",
            Role::Option => "option",
            Role::Heading => "heading",
            Role::Cell => "cell",
            Role::Listitem => "listitem",
            Role::Text => "text",
            Role::Image => "image",
            Role::List => "list",
            Role::Grid => "grid",
            Role::Tablist => "tablist",
            Role::Toolbar => "toolbar",
            Role::Alert => "alert",
            Role::Dialog => "dialog",
            Role::Progressbar => "progressbar",
            Role::Keyboard => "keyboard",
            Role::Webview => "webview",
            Role::Group => "group",
            Role::Generic => "generic",
        }
    }

    /// Roles that always get a ref: agent-browser's interactive roles.
    pub fn is_interactive(self) -> bool {
        matches!(
            self,
            Role::Button
                | Role::Link
                | Role::Textbox
                | Role::Searchbox
                | Role::Checkbox
                | Role::Radio
                | Role::Switch
                | Role::Slider
                | Role::Combobox
                | Role::Tab
                | Role::Menuitem
                | Role::Option
        )
    }

    /// Roles that get a ref when named: agent-browser's content roles.
    pub fn is_content(self) -> bool {
        matches!(self, Role::Heading | Role::Cell | Role::Listitem)
    }

    /// Roles for modals that cover what lies below them.
    pub fn is_modal(self) -> bool {
        matches!(self, Role::Alert | Role::Dialog)
    }
}

impl core::fmt::Display for Role {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One window in a snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSummary {
    pub id: u32,
    pub kind: WindowType,
    /// Z-order; higher is closer to the user.
    pub layer: i32,
    pub package: Option<String>,
    pub title: Option<String>,
    pub owner: Owner,
    /// In device points.
    pub bounds: Rect,
    pub modal: bool,
    /// How the window's tree is kept current and how old it is.
    pub source: WindowSource,
}

/// How a window's tree is kept current (spec: Windows without change
/// events).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowSource {
    pub mode: RefreshMode,
    /// Why the window was last read.
    pub reason: ReadReason,
    /// How long that read took.
    pub read_cost: Duration,
    /// How long ago that read finished.
    pub age: Duration,
}

/// How a window is kept current.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefreshMode {
    /// From the change events its toolkit sends: the window is re-read when
    /// an event names it.
    Events,
    /// By full re-reads after every injected action, when frames change and
    /// on a poll, because its toolkit sends no change events.
    Reread,
}

/// A change the mirror found between two reads of a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeDelta {
    /// When the read that found it finished.
    pub at: Instant,
    /// The replica revision it produced.
    pub revision: u64,
    pub window: u32,
    /// Why the window was read.
    pub reason: ReadReason,
    pub added: Vec<ElementId>,
    pub removed: Vec<ElementId>,
    /// Elements whose text, description, bounds or states changed.
    pub changed: Vec<ElementId>,
}

/// Content scrolled out of a container's view (spec: Snapshot content).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrollHint {
    /// The scroll container.
    pub container: ElementId,
    pub window: u32,
    pub direction: Direction,
    /// How many items lie that way; `None` when the platform only says that
    /// the container can scroll further.
    pub count: Option<u32>,
    pub source: HintSource,
}

impl ScrollHint {
    /// The hint as snapshots print it, such as
    /// `(3 more actionable items below, scroll down to reveal)`.
    pub fn text(&self) -> String {
        let (place, scroll) = match self.direction {
            Direction::Above => ("above", "up"),
            Direction::Below => ("below", "down"),
            Direction::Left => ("to the left", "left"),
            Direction::Right => ("to the right", "right"),
        };
        let items = match (self.count, self.source) {
            (None, _) => "more items".to_owned(),
            (Some(n), HintSource::OffscreenNodes) => {
                format!("{n} more actionable item{}", if n == 1 { "" } else { "s" })
            }
            (Some(n), HintSource::ScrollSemantics) => {
                format!("{n} more item{}", if n == 1 { "" } else { "s" })
            }
        };
        format!("({items} {place}, scroll {scroll} to reveal)")
    }
}

/// Where hidden content lies relative to a scroll container's viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Above,
    Below,
    Left,
    Right,
}

/// What a scroll hint was computed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HintSource {
    /// Actionable nodes the read returned outside the viewport, as View
    /// lists and React Native keep them.
    OffscreenNodes,
    /// The container's scroll semantics, for toolkits such as Compose that
    /// report only the nodes inside the viewport.
    ScrollSemantics,
}
