//! The tree both read paths produce, and its conversion into the protocol's
//! snapshot payload (`crates/protocol`, `snapshot.rs`).
//!
//! The in-guest reader fills [`Element`]s from the runtime's snapshot
//! dictionaries; the host translator fills them from `AXPTranslator`'s
//! elements. [`snapshot`] then turns one read into windows and nodes the way
//! the mirror reads them (see `mirror::backend`):
//!
//! - `class` is the `XCUIElementType` name, such as `XCUIElementTypeButton`,
//!   or the platform class for elements whose type says nothing (`Other`,
//!   `Group`), so that an alert controller (`_UIAlertControllerPhoneTVMacView`)
//!   can be recognized. The application element and windows use their type
//!   names; a static text with the header trait is named `Heading`.
//! - `desc` is the label, or a text field's placeholder when it has no label;
//!   `text` is the value, except an empty field's value, which repeats its
//!   placeholder, and a switch's, which becomes the `checked` flag.
//! - `res_id` is the accessibility identifier.
//! - Bounds are physical pixels: frames in points times the screen scale.
//! - Flags come from the type and the UIAccessibilityTraits bits; iOS reports
//!   no visibility, so `visible` means the frame overlaps the screen.
//! - `scroll` comes from a scroll view's [`ScrollArea`]: whether content lies
//!   beyond the visible frame toward the start and the end of each axis,
//!   and which part of the content the visible frame shows, in percent. The
//!   runtime reports no items, so no range counts them.
//!
//! One read is one process, the one that owns the foreground: its
//! application element becomes a window whose id is the process id, and the
//! keyboard window inside it (`UIRemoteKeyboardWindow`) a separate input
//! method window, which the mirror collapses to one line. The keys come in
//! the application's own read: agent-device lists that window as an
//! Application because iOS 26.5 gives every window automation type 2. A
//! child moved to a window of its own still counts as delivered, so moving
//! it cuts nothing. The keyboard's window spans the screen and lets touches
//! through outside what it draws, so its bounds are what it draws: the host
//! view of its input views, which on iOS 26.5 runs from the prediction bar
//! above the `Keyboard` element down through the dock to the bottom of the
//! screen (539 to 874 points on an iPhone 17), or none when it draws
//! nothing. The mirror cuts an element's visible bounds by that window
//! (task 10.21). The window metadata of protocol task 3.6:
//!
//! - `emits_events` is `Some(false)` on every window: the Simulator sends no
//!   change events (spikes S3 and S4), so the mirror reads its windows again
//!   after each action and on a poll.
//! - `owner` names the process on every window of system UI, such as
//!   `SpringBoard` under a permission alert, and is absent on an
//!   application's.
//! - An application's alert on screen moves, with the view that presents it
//!   (the child of the alert's window that holds it), to a window of its
//!   own above the application's, marked `modal`: the mirror then covers the
//!   application's window, which keeps what the alert covers. System UI
//!   keeps its alerts in its window, which is not modal: the read holds no
//!   application window for them to cover, and the mirror cuts the rest of
//!   the window by the alert's class.
//! - So does a sheet that blocks the screen: a transition view that is the
//!   last child of an application's window, holding a dimming view that
//!   spans it and takes touches (UIKit's rule, and agent-device's), with the
//!   sheet beside it. A dimming view that lets touches through, at an
//!   undimmed detent, or whose answer the runtime does not give, blocks
//!   nothing, and the sheet stays in the application's window.
//! - Windows are listed and their nodes numbered top-most first: the
//!   keyboard, the alert, the sheet, then the process's own window, so a
//!   node limit cuts the windows behind first.

use protocol::message::SnapshotParams;
use protocol::snapshot::{
    Bounds, MAX_NODE_DEPTH, Node, NodeFlags, Scroll, ScrollAxis, ScrollFlags, ScrollPosition,
    ScrollRange, ScrollUnit, Snapshot, Truncation, Window, WindowFlags, WindowType,
};

/// UIAccessibilityTraits bits the conversion reads.
pub mod traits {
    pub const BUTTON: u64 = 1 << 0;
    pub const LINK: u64 = 1 << 1;
    pub const SELECTED: u64 = 1 << 3;
    pub const NOT_ENABLED: u64 = 1 << 8;
    pub const HEADER: u64 = 1 << 16;
    /// Private: the element is being edited, so it holds keyboard focus. Set
    /// on the focused text field and on no other element on iOS 26.5.
    pub const IS_EDITING: u64 = 1 << 21;
}

/// `XCUIElementType` values the conversion treats specially.
pub mod types {
    pub const ANY: u64 = 0;
    pub const OTHER: u64 = 1;
    /// What iOS 26.5 reports for a `UIWindow`, where XCTest's enum says
    /// Application (spike S4).
    pub const RUNTIME_WINDOW: u64 = 2;
    pub const GROUP: u64 = 3;
    pub const TABLE: u64 = 26;
    pub const COLLECTION_VIEW: u64 = 32;
    pub const SWITCH: u64 = 40;
    pub const TOGGLE: u64 = 41;
    pub const SEARCH_FIELD: u64 = 45;
    pub const SCROLL_VIEW: u64 = 46;
    pub const STATIC_TEXT: u64 = 48;
    pub const TEXT_FIELD: u64 = 49;
    pub const SECURE_TEXT_FIELD: u64 = 50;
    pub const TEXT_VIEW: u64 = 52;
}

/// The prefix of every type-derived class name.
pub const XCUI_PREFIX: &str = "XCUIElementType";

/// The platform class of the window that holds the keyboard.
pub const KEYBOARD_WINDOW_CLASS: &str = "UIRemoteKeyboardWindow";

/// Set in a keyboard window's id, which is otherwise its process id.
pub const KEYBOARD_WINDOW: u32 = 1 << 31;

/// Set in the id of the window an in-app alert moves to, which is otherwise
/// its process id.
pub const ALERT_WINDOW: u32 = 1 << 30;

/// Set in the id of the window a sheet that blocks the screen moves to,
/// which is otherwise its process id.
pub const SHEET_WINDOW: u32 = 1 << 29;

/// The class of the view UIKit presents a view controller in, a later
/// sibling of the one it presents from.
pub const PRESENTATION_CLASS: &str = "UITransitionView";

/// The class of the view UIKit dims what a sheet covers with. It spans three
/// screens and, while it takes touches, keeps them from what it covers.
pub const DIMMING_CLASS: &str = "UIDimmingView";

/// The classes of the containers UIKit draws an alert controller in, which
/// the runtime types as groups (spike S4), and XCTest's name for an alert.
pub const ALERT_CLASSES: [&str; 4] = [
    "_UIAlertControllerPhoneTVMacView",
    "_UIAlertControllerView",
    "UIAlertControllerView",
    "XCUIElementTypeAlert",
];

/// The bundle identifier of SpringBoard, which owns the home screen and
/// system alerts.
pub const SPRINGBOARD: &str = "com.apple.springboard";

/// One accessibility element as a read path found it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    /// The runtime's automation type: an `XCUIElementType` value for
    /// controls, 0 for views without one, 1 for the application element and
    /// 2 for a window on iOS 26.5.
    pub automation_type: Option<u64>,
    /// The platform class (`XC_kAXXCAttributeElementType`), such as
    /// `UITextField` or `SwiftUI.AccessibilityNode`.
    pub class: Option<String>,
    pub label: Option<String>,
    pub identifier: Option<String>,
    pub value: Option<String>,
    pub placeholder: Option<String>,
    /// UIAccessibilityTraits bits.
    pub traits: u64,
    /// `[x, y, width, height]` in points.
    pub frame: Option<[f64; 4]>,
    /// How many children the accessibility server said the element has,
    /// when it said; more than `children` holds means some were withheld.
    pub children_count: Option<u64>,
    pub children: Vec<Element>,
    /// Where a scroll view's viewport is in its content, as the runtime
    /// reports it; absent on other elements.
    pub scroll_area: Option<ScrollArea>,
    /// Whether the element takes touches
    /// (`XC_kAXXCAttributeIsUserInteractionEnabled`), read for dimming views
    /// alone: it tells a sheet that blocks the screen from one resting at an
    /// undimmed detent. Absent when not read.
    pub user_interaction: Option<bool>,
}

/// What the runtime reports about a scroll view's viewport: where the edges
/// of the element's frame fall in its content along each axis, as fractions
/// of the content's length (`XC_kAXXCAttributeVisibleVerticalScrollArea`
/// and `XC_kAXXCAttributeVisibleHorizontalScrollArea`, points whose x is the
/// start and y the end), and the part of the frame that shows content
/// (`XC_kAXXCAttributeVisibleFrame`), which leaves out the insets UIKit
/// keeps under bars.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollArea {
    /// The frame's top and bottom, as fractions of the content's height.
    pub vertical: Option<[f64; 2]>,
    /// The frame's left and right, as fractions of the content's width.
    pub horizontal: Option<[f64; 2]>,
    /// `[x, y, width, height]` in points.
    pub visible_frame: Option<[f64; 4]>,
}

/// The process whose tree was read: the one that owns the foreground.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Owner {
    pub pid: i32,
    pub bundle_id: Option<String>,
    /// The process's name: its bundle's display name or name, else its
    /// executable's. It titles the window when the application element has
    /// no label, and system UI names itself by it as its windows' owner.
    pub name: String,
    /// System UI, such as SpringBoard, rather than an application.
    pub system: bool,
}

/// How much of the tree to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Deepest level to read below the application element.
    pub max_depth: u32,
    /// Most nodes to read.
    pub max_nodes: u32,
}

impl Default for Limits {
    /// Agent-device's bounds, which S4 used: 64 levels and 10,000 nodes.
    fn default() -> Limits {
        Limits {
            max_depth: 64,
            max_nodes: 10_000,
        }
    }
}

impl Limits {
    /// The request's bounds, the defaults where it sets none, capped at the
    /// depth the protocol decodes.
    pub fn from_params(params: &SnapshotParams) -> Limits {
        let defaults = Limits::default();
        Limits {
            max_depth: params
                .max_depth
                .unwrap_or(defaults.max_depth)
                .min(MAX_NODE_DEPTH as u32),
            max_nodes: params.max_nodes.unwrap_or(defaults.max_nodes).max(1),
        }
    }
}

/// One read, ready to convert.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub owner: Owner,
    /// The application element.
    pub root: Element,
    /// Physical pixels per point.
    pub scale: f64,
    pub limits: Limits,
    /// The read path had to ask for fewer levels than `limits` allows.
    pub depth_cut: bool,
}

/// Turns a read into the protocol's snapshot payload.
pub fn snapshot(read: &Read) -> Snapshot {
    let mut builder = Builder {
        strings: Vec::new(),
        next_id: 1,
        count: 0,
        truncated: Truncation::EMPTY,
        limits: read.limits,
        scale: read.scale,
        screen: read.root.frame.filter(|f| f[2] > 0.0 && f[3] > 0.0),
    };
    if read.depth_cut {
        builder.truncated |= Truncation::DEPTH;
    }
    let package = read
        .owner
        .bundle_id
        .as_deref()
        .map(|bundle| builder.string(bundle));
    let owner = system_name(&read.owner).map(|name| builder.string(name));
    let pid = read.owner.pid as u32;

    // The keyboard window leaves the application's tree for its own window.
    let mut app = read.root.clone();
    let mut keyboards = Vec::new();
    while let Some(index) = app.children.iter().position(is_keyboard_window) {
        keyboards.push(take_child(&mut app, index));
    }
    // So does an application's alert, and then a sheet that blocks the
    // screen, each for a modal window.
    let alert = if read.owner.system {
        None
    } else {
        take_alert(&mut app, builder.screen)
    };
    let sheet = if read.owner.system {
        None
    } else {
        take_sheet(&mut app, builder.screen)
    };

    let title = present(&app.label)
        .map(str::to_owned)
        .or_else(|| (!read.owner.name.is_empty()).then(|| read.owner.name.clone()));
    let mut windows = Vec::new();
    let sheet_layer = 1;
    let alert_layer = sheet_layer + i32::from(sheet.is_some());
    let keyboard_layer = alert_layer + i32::from(alert.is_some());
    for keyboard in &keyboards {
        let root = builder.node(keyboard, 1);
        windows.push(Window {
            id: pid | KEYBOARD_WINDOW,
            kind: WindowType::INPUT_METHOD,
            layer: keyboard_layer,
            package,
            title: None,
            bounds: builder.bounds(keyboard_area(keyboard, builder.screen)),
            flags: WindowFlags::EMPTY,
            root,
            emits_events: Some(false),
            owner,
            modal: false,
        });
    }
    if let Some(alert) = &alert {
        let root = builder.node(&alert.view, alert.level);
        windows.push(Window {
            id: pid | ALERT_WINDOW,
            kind: WindowType::APPLICATION,
            layer: alert_layer,
            package,
            title: alert.title.clone(),
            bounds: builder.bounds(alert.view.frame),
            flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
            root,
            emits_events: Some(false),
            owner,
            modal: true,
        });
    }
    if let Some(sheet) = &sheet {
        let root = builder.node(&sheet.view, sheet.level);
        windows.push(Window {
            id: pid | SHEET_WINDOW,
            kind: WindowType::APPLICATION,
            layer: sheet_layer,
            package,
            title: sheet.title.clone(),
            bounds: builder.bounds(sheet.view.frame),
            flags: if alert.is_some() {
                WindowFlags::EMPTY
            } else {
                WindowFlags::ACTIVE | WindowFlags::FOCUSED
            },
            root,
            emits_events: Some(false),
            owner,
            modal: true,
        });
    }
    let app_root = builder.node(&app, 0);
    windows.push(Window {
        id: pid,
        kind: if read.owner.system {
            WindowType::SYSTEM
        } else {
            WindowType::APPLICATION
        },
        layer: 0,
        package,
        title,
        bounds: builder.bounds(app.frame),
        flags: if alert.is_some() || sheet.is_some() {
            WindowFlags::EMPTY
        } else {
            WindowFlags::ACTIVE | WindowFlags::FOCUSED
        },
        root: app_root,
        emits_events: Some(false),
        owner,
        modal: false,
    });
    Snapshot {
        strings: builder.strings,
        windows,
        node_count: builder.count,
        truncated: builder.truncated,
        seq: 0,
    }
}

fn is_keyboard_window(element: &Element) -> bool {
    element.class.as_deref() == Some(KEYBOARD_WINDOW_CLASS)
}

/// Less than this, in points, between two edges counts as none.
const EDGE_SLACK: f64 = 0.5;

/// The part of the screen a keyboard window covers, in points: the union of
/// its outermost descendants on screen that do not span it, clipped to it.
/// The window and the input set container inside it span the screen and let
/// touches through; below them the host view holds what the keyboard draws
/// (on iOS 26.5 the prediction bar, the keys and the dock, from 539 points
/// down on an iPhone 17), and a descent stops there, before the views of
/// the prediction bar that reach beyond the screen. `None` when the window
/// shows nothing.
fn keyboard_area(window: &Element, screen: Option<[f64; 4]>) -> Option<[f64; 4]> {
    fn collect(element: &Element, within: [f64; 4], area: &mut Option<[f64; 4]>) {
        for child in &element.children {
            match child.frame.and_then(|frame| intersection(frame, within)) {
                Some(shown) if !spans(shown, within) => {
                    *area = Some(area.map_or(shown, |area| union(area, shown)));
                }
                _ => collect(child, within, area),
            }
        }
    }
    let within = window
        .frame
        .filter(|[_, _, width, height]| *width > 0.0 && *height > 0.0)
        .or(screen)?;
    let mut area = None;
    collect(window, within, &mut area);
    area
}

/// The part of `frame` inside `within`, when it has an area.
fn intersection(frame: [f64; 4], within: [f64; 4]) -> Option<[f64; 4]> {
    let left = frame[0].max(within[0]);
    let top = frame[1].max(within[1]);
    let right = (frame[0] + frame[2]).min(within[0] + within[2]);
    let bottom = (frame[1] + frame[3]).min(within[1] + within[3]);
    (right > left && bottom > top).then_some([left, top, right - left, bottom - top])
}

/// Whether `frame` covers all of `within`, to within [`EDGE_SLACK`].
fn spans(frame: [f64; 4], within: [f64; 4]) -> bool {
    frame[0] <= within[0] + EDGE_SLACK
        && frame[1] <= within[1] + EDGE_SLACK
        && frame[0] + frame[2] >= within[0] + within[2] - EDGE_SLACK
        && frame[1] + frame[3] >= within[1] + within[3] - EDGE_SLACK
}

/// The smallest frame that holds both.
fn union(one: [f64; 4], other: [f64; 4]) -> [f64; 4] {
    let left = one[0].min(other[0]);
    let top = one[1].min(other[1]);
    let right = (one[0] + one[2]).max(other[0] + other[2]);
    let bottom = (one[1] + one[3]).max(other[1] + other[3]);
    [left, top, right - left, bottom - top]
}

/// The name system UI goes by as the owner of its windows: the process's
/// name, else its bundle identifier. `None` for an application.
fn system_name(owner: &Owner) -> Option<&str> {
    if !owner.system {
        return None;
    }
    Some(owner.name.as_str())
        .filter(|name| !name.is_empty())
        .or(owner.bundle_id.as_deref())
}

/// Removes child `index` of `parent` for a window of its own. The child
/// still counts as delivered, so the child count the runtime reported for
/// `parent` drops with it and the move reads as no cut.
fn take_child(parent: &mut Element, index: usize) -> Element {
    let child = parent.children.remove(index);
    if let Some(count) = &mut parent.children_count {
        *count = count.saturating_sub(1);
    }
    child
}

/// An application's alert or sheet, taken out of its tree for a window of
/// its own.
struct Presentation {
    /// The view that presents it: for an alert, the child of the alert's
    /// window that holds it, or the alert itself when it sits right in a
    /// window; for a sheet, its transition view.
    view: Element,
    /// How many levels below the application element `view` sat.
    level: u32,
    /// An alert's title, its container's label; a sheet's, its navigation
    /// bar's identifier or its first heading.
    title: Option<String>,
}

/// Takes the last alert on screen, in pre-order (the one in front), out of
/// the application's tree `app`, with the view that presents it.
fn take_alert(app: &mut Element, screen: Option<[f64; 4]>) -> Option<Presentation> {
    fn find(
        element: &Element,
        level: u32,
        screen: Option<[f64; 4]>,
        path: &mut Vec<usize>,
        found: &mut Option<(Vec<usize>, Option<String>)>,
    ) {
        if level > 0
            && on_screen(element.frame, screen)
            && ALERT_CLASSES.contains(&class_name(level, element).as_str())
        {
            *found = Some((path.clone(), present(&element.label).map(str::to_owned)));
        }
        for (index, child) in element.children.iter().enumerate() {
            path.push(index);
            find(child, level + 1, screen, path, found);
            path.pop();
        }
    }
    let mut found = None;
    find(app, 0, screen, &mut Vec::new(), &mut found);
    let (path, title) = found?;
    // The path leads from the application through the alert's window to
    // the alert; the view two levels down presents it.
    let depth = path.len().min(2);
    let mut parent = app;
    for &index in &path[..depth - 1] {
        parent = &mut parent.children[index];
    }
    let view = take_child(parent, path[depth - 1]);
    Some(Presentation {
        view,
        level: depth as u32,
        title,
    })
}

/// Takes the top-most sheet that blocks the screen out of the application's
/// tree `app`: the last child of one of its windows that [`blocks`], in the
/// top-most window that has one.
fn take_sheet(app: &mut Element, screen: Option<[f64; 4]>) -> Option<Presentation> {
    let window = (0..app.children.len()).rev().find(|&index| {
        app.children[index]
            .children
            .last()
            .is_some_and(|view| blocks(view, screen))
    })?;
    let window = &mut app.children[window];
    let view = take_child(window, window.children.len() - 1);
    let title = sheet_title(&view);
    Some(Presentation {
        view,
        level: 2,
        title,
    })
}

/// Whether `view` presents a sheet that keeps touches from what lies below
/// it: a transition view on screen holding, beside the sheet, a dimming
/// view that spans it and takes touches.
fn blocks(view: &Element, screen: Option<[f64; 4]>) -> bool {
    let spans = |dimming: &Element| match (dimming.frame, view.frame.or(screen)) {
        (Some([x, y, width, height]), Some([vx, vy, vw, vh])) => {
            x <= vx && y <= vy && x + width >= vx + vw && y + height >= vy + vh
        }
        _ => false,
    };
    view.class.as_deref() == Some(PRESENTATION_CLASS)
        && on_screen(view.frame, screen)
        && view.children.len() >= 2
        && view.children.iter().any(|child| {
            child.class.as_deref() == Some(DIMMING_CLASS)
                && child.user_interaction == Some(true)
                && spans(child)
        })
}

/// A sheet's title: the identifier of its first navigation bar, which UIKit
/// names after the title, else its first heading.
fn sheet_title(view: &Element) -> Option<String> {
    fn find(element: &Element, wanted: &impl Fn(&Element) -> Option<String>) -> Option<String> {
        wanted(element).or_else(|| {
            element
                .children
                .iter()
                .find_map(|child| find(child, wanted))
        })
    }
    let navigation_bar = xcui_element_type("NavigationBar");
    find(view, &|element| {
        (element.automation_type == navigation_bar)
            .then(|| present(&element.identifier).map(str::to_owned))
            .flatten()
    })
    .or_else(|| {
        find(view, &|element| {
            (element.automation_type == Some(types::STATIC_TEXT)
                && element.traits & traits::HEADER != 0)
                .then(|| present(&element.label).map(str::to_owned))
                .flatten()
        })
    })
}

/// Whether an element of this automation type is a scroll view: a scroll
/// view, table or collection view, which the reader marks `scrollable` and
/// asks for its scroll area.
pub fn is_scroll_view(automation_type: Option<u64>) -> bool {
    matches!(
        automation_type,
        Some(types::SCROLL_VIEW | types::TABLE | types::COLLECTION_VIEW)
    )
}

/// Less content than this beyond the viewport, in points, does not count:
/// UIKit gives an alert's message 72.33 points of text in a 72-point scroll
/// view.
const SCROLL_SLACK: f64 = 1.0;

/// The scroll semantics of a scroll view at `frame`, in points, whose
/// runtime scroll area is `area`. One that scrolls along one axis names it,
/// the ways it can scroll, and the part of its content the visible frame
/// shows as a percent range; one that scrolls along both names no axis and
/// no range; one whose content fits its visible frame has none.
pub fn scroll(frame: [f64; 4], area: &ScrollArea) -> Option<Scroll> {
    let [x, y, width, height] = frame;
    let [shown_x, shown_y, shown_width, shown_height] = area.visible_frame.unwrap_or(frame);
    let vertical = area
        .vertical
        .and_then(|ends| Reach::along(y, height, shown_y, shown_height, ends));
    let horizontal = area
        .horizontal
        .and_then(|ends| Reach::along(x, width, shown_x, shown_width, ends));
    match (vertical, horizontal) {
        (None, None) => None,
        (Some(reach), None) => Some(reach.semantics(ScrollAxis::VERTICAL)),
        (None, Some(reach)) => Some(reach.semantics(ScrollAxis::HORIZONTAL)),
        (Some(vertical), Some(horizontal)) => Some(Scroll {
            axis: ScrollAxis::UNKNOWN,
            flags: vertical.flags() | horizontal.flags(),
            range: None,
            position: None,
        }),
    }
}

/// How a scroll view's viewport sits in its content along one axis.
struct Reach {
    /// Content lies before the viewport.
    back: bool,
    /// Content lies after it.
    forward: bool,
    /// The first and last percent of the content the viewport shows.
    first: f64,
    last: f64,
}

impl Reach {
    /// Along an axis where the frame starts at `start` and is `length`
    /// long, the visible frame starts at `shown_start` and is `shown_length`
    /// long, and the frame's ends fall at `from` and `to` of the content, as
    /// fractions of its length; `None` when nothing lies beyond the
    /// viewport. The frame's ends include the insets UIKit keeps under bars
    /// (62 and 83 points on the SwiftUI list), so the viewport is the visible
    /// frame.
    fn along(
        start: f64,
        length: f64,
        shown_start: f64,
        shown_length: f64,
        [from, to]: [f64; 2],
    ) -> Option<Reach> {
        let span = to - from;
        if !(length > 0.0 && span > 0.0 && from.is_finite() && span.is_finite()) {
            return None;
        }
        let content = length / span;
        let before = (shown_start - start).clamp(0.0, length);
        let after = (start + length - (shown_start + shown_length)).clamp(0.0, length);
        let first = from * content + before;
        let last = to * content - after;
        let back = first > SCROLL_SLACK;
        let forward = content - last > SCROLL_SLACK;
        (back || forward).then(|| Reach {
            back,
            forward,
            first: first * 100.0 / content,
            last: last * 100.0 / content,
        })
    }

    fn flags(&self) -> ScrollFlags {
        let mut flags = ScrollFlags::EMPTY;
        if self.back {
            flags |= ScrollFlags::BACK;
        }
        if self.forward {
            flags |= ScrollFlags::FORWARD;
        }
        flags
    }

    /// The semantics along `axis`. The position is rounded to whole
    /// percent, but it is 0 only while the viewport shows the start and 100
    /// only while it shows the end, as the flags say.
    fn semantics(&self, axis: ScrollAxis) -> Scroll {
        let first = if self.back {
            let most = if self.forward { 99.0 } else { 100.0 };
            self.first.round().clamp(1.0, most)
        } else {
            0.0
        };
        let last = if self.forward {
            self.last.round().clamp(first, 99.0)
        } else {
            100.0
        };
        Scroll {
            axis,
            flags: self.flags(),
            range: Some(ScrollRange {
                unit: ScrollUnit::PERCENT,
                min: 0,
                max: 100,
            }),
            position: Some(ScrollPosition {
                first: first as i32,
                last: last as i32,
            }),
        }
    }
}

/// Whether a frame, in points, has an area and overlaps the screen, the
/// application element's frame when it has one.
pub(crate) fn on_screen(frame: Option<[f64; 4]>, screen: Option<[f64; 4]>) -> bool {
    let Some([x, y, width, height]) = frame else {
        return false;
    };
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    match screen {
        None => true,
        Some([sx, sy, sw, sh]) => x < sx + sw && x + width > sx && y < sy + sh && y + height > sy,
    }
}

/// The string, unless absent or only whitespace.
fn present(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|v| !v.trim().is_empty())
}

struct Builder {
    strings: Vec<String>,
    next_id: u32,
    count: u32,
    truncated: Truncation,
    limits: Limits,
    scale: f64,
    /// The application element's frame, for visibility.
    screen: Option<[f64; 4]>,
}

impl Builder {
    fn string(&mut self, value: &str) -> u32 {
        match self.strings.iter().position(|s| s == value) {
            Some(index) => index as u32,
            None => {
                self.strings.push(value.to_owned());
                (self.strings.len() - 1) as u32
            }
        }
    }

    fn bounds(&self, frame: Option<[f64; 4]>) -> Bounds {
        let Some([x, y, width, height]) = frame else {
            return Bounds::default();
        };
        let pixels = |points: f64| (points * self.scale).round() as i32;
        Bounds::new(pixels(x), pixels(y), pixels(x + width), pixels(y + height))
    }

    fn visible(&self, frame: Option<[f64; 4]>) -> bool {
        on_screen(frame, self.screen)
    }

    /// Converts `element`, `level` levels below the application element,
    /// with as many descendants as the limits allow.
    fn node(&mut self, element: &Element, level: u32) -> Option<Node> {
        if self.count >= self.limits.max_nodes {
            self.truncated |= Truncation::NODES;
            return None;
        }
        self.count += 1;
        let id = self.next_id;
        self.next_id += 1;

        let automation_type = element.automation_type;
        let text_input = matches!(
            automation_type,
            Some(
                types::TEXT_FIELD
                    | types::SECURE_TEXT_FIELD
                    | types::SEARCH_FIELD
                    | types::TEXT_VIEW
            )
        );
        let switch = matches!(automation_type, Some(types::SWITCH | types::TOGGLE));
        let placeholder = present(&element.placeholder);
        let value = present(&element.value);
        let class = class_name(level, element);
        let class = self.string(&class);

        let mut flags = NodeFlags::EMPTY;
        if element.traits & traits::NOT_ENABLED == 0 {
            flags |= NodeFlags::ENABLED;
        }
        if self.visible(element.frame) {
            flags |= NodeFlags::VISIBLE;
        }
        if element.traits & traits::SELECTED != 0 {
            flags |= NodeFlags::SELECTED;
        }
        if element.traits & (traits::BUTTON | traits::LINK) != 0 {
            flags |= NodeFlags::CLICKABLE;
        }
        if text_input {
            flags |= NodeFlags::EDITABLE | NodeFlags::FOCUSABLE;
        }
        if automation_type == Some(types::SECURE_TEXT_FIELD) {
            flags |= NodeFlags::PASSWORD;
        }
        if element.traits & traits::IS_EDITING != 0 {
            flags |= NodeFlags::FOCUSED;
        }
        if switch {
            flags |= NodeFlags::CHECKABLE;
            if value == Some("1") {
                flags |= NodeFlags::CHECKED;
            }
        }
        if is_scroll_view(automation_type) {
            flags |= NodeFlags::SCROLLABLE;
        }

        let desc = present(&element.label).or(if text_input { placeholder } else { None });
        let repeats_placeholder = |value: &&str| text_input && Some(*value) == placeholder;
        let text = value.filter(|value| !switch && !repeats_placeholder(value));

        let delivered = element.children.len() as u64;
        if element
            .children_count
            .is_some_and(|count| count > delivered)
        {
            self.truncated |= if level >= self.limits.max_depth {
                Truncation::DEPTH
            } else {
                Truncation::NODES
            };
        }
        let mut children = Vec::new();
        if !element.children.is_empty() {
            if level >= self.limits.max_depth {
                self.truncated |= Truncation::DEPTH;
            } else {
                for child in &element.children {
                    match self.node(child, level + 1) {
                        Some(child) => children.push(child),
                        None => break,
                    }
                }
            }
        }
        Some(Node {
            id,
            class,
            res_pkg: None,
            res_id: present(&element.identifier).map(str::to_owned),
            text: text.map(str::to_owned),
            desc: desc.map(str::to_owned),
            bounds: self.bounds(element.frame),
            flags,
            children,
            scroll: element
                .scroll_area
                .as_ref()
                .zip(element.frame)
                .and_then(|(area, frame)| scroll(frame, area)),
        })
    }
}

/// The class name a node gets: see the module documentation.
pub fn class_name(level: u32, element: &Element) -> String {
    let platform = || {
        element
            .class
            .as_deref()
            .filter(|class| !class.trim().is_empty())
            .map(str::to_owned)
    };
    let name = match (level, element.automation_type) {
        (0, _) => "Application",
        (_, Some(types::RUNTIME_WINDOW)) => "Window",
        (_, None | Some(types::ANY | types::OTHER)) => {
            return platform().unwrap_or_else(|| format!("{XCUI_PREFIX}Other"));
        }
        (_, Some(types::GROUP)) => {
            return platform().unwrap_or_else(|| format!("{XCUI_PREFIX}Group"));
        }
        (_, Some(types::STATIC_TEXT)) if element.traits & traits::HEADER != 0 => "Heading",
        (_, Some(value)) => xcui_element_type_name(value).unwrap_or("Other"),
    };
    format!("{XCUI_PREFIX}{name}")
}

/// The `XCUIElementType` case name for a value, as XCTest names them
/// (`XCUIElementTypeButton` is 9).
pub fn xcui_element_type_name(value: u64) -> Option<&'static str> {
    XCUI_ELEMENT_TYPES
        .get(usize::try_from(value).ok()?)
        .copied()
}

/// The value of an `XCUIElementType` case name.
pub fn xcui_element_type(name: &str) -> Option<u64> {
    XCUI_ELEMENT_TYPES
        .iter()
        .position(|candidate| *candidate == name)
        .map(|index| index as u64)
}

/// `XCUIElementType` from XCTest's `XCUIElementTypes.h`, indexed by value.
const XCUI_ELEMENT_TYPES: [&str; 83] = [
    "Any",
    "Other",
    "Application",
    "Group",
    "Window",
    "Sheet",
    "Drawer",
    "Alert",
    "Dialog",
    "Button",
    "RadioButton",
    "RadioGroup",
    "CheckBox",
    "DisclosureTriangle",
    "PopUpButton",
    "ComboBox",
    "MenuButton",
    "ToolbarButton",
    "Popover",
    "Keyboard",
    "Key",
    "NavigationBar",
    "TabBar",
    "TabGroup",
    "Toolbar",
    "StatusBar",
    "Table",
    "TableRow",
    "TableColumn",
    "Outline",
    "OutlineRow",
    "Browser",
    "CollectionView",
    "Slider",
    "PageIndicator",
    "ProgressIndicator",
    "ActivityIndicator",
    "SegmentedControl",
    "Picker",
    "PickerWheel",
    "Switch",
    "Toggle",
    "Link",
    "Image",
    "Icon",
    "SearchField",
    "ScrollView",
    "ScrollBar",
    "StaticText",
    "TextField",
    "SecureTextField",
    "DatePicker",
    "TextView",
    "Menu",
    "MenuItem",
    "MenuBar",
    "MenuBarItem",
    "Map",
    "WebView",
    "IncrementArrow",
    "DecrementArrow",
    "Timeline",
    "RatingIndicator",
    "ValueIndicator",
    "SplitGroup",
    "Splitter",
    "RelevanceIndicator",
    "ColorWell",
    "HelpTag",
    "Matte",
    "DockItem",
    "Ruler",
    "RulerMarker",
    "Grid",
    "LevelIndicator",
    "Cell",
    "LayoutArea",
    "LayoutItem",
    "Handle",
    "Stepper",
    "Tab",
    "TouchBar",
    "StatusItem",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn element(automation_type: u64, class: &str) -> Element {
        Element {
            automation_type: Some(automation_type),
            class: Some(class.to_owned()),
            frame: Some([0.0, 0.0, 402.0, 874.0]),
            ..Element::default()
        }
    }

    fn with(mut element: Element, edit: impl FnOnce(&mut Element)) -> Element {
        edit(&mut element);
        element
    }

    fn at(element: Element, frame: [f64; 4]) -> Element {
        with(element, |e| e.frame = Some(frame))
    }

    fn owner() -> Owner {
        Owner {
            pid: 4242,
            bundle_id: Some("dev.mobileagent.fixture.swiftui".into()),
            name: "FixtureSwiftUI".into(),
            system: false,
        }
    }

    fn read(root: Element) -> Read {
        Read {
            owner: owner(),
            root,
            scale: 3.0,
            limits: Limits::default(),
            depth_cut: false,
        }
    }

    /// The SwiftUI fixture's Login screen as the reader returned it on iOS
    /// 26.5, containers trimmed: the application, its window, the fields,
    /// the button, the status text and the selected Login tab.
    fn login() -> Element {
        let application = with(
            element(
                1,
                "_TtC7SwiftUIP33_ACC2C5639A7D76F611E170E831FCA49118SwiftUIApplication",
            ),
            |e| e.label = Some("FixtureSwiftUI".into()),
        );
        let username = at(
            with(element(49, "UITextField"), |e| {
                e.identifier = Some("login-username".into());
                e.value = Some("Username".into());
                e.placeholder = Some("Username".into());
                e.traits = 0x22_0004_0000;
            }),
            [16.0, 78.0, 370.0, 34.0],
        );
        let password = at(
            with(element(50, "UITextField"), |e| {
                e.identifier = Some("login-password".into());
                e.value = Some("Password".into());
                e.placeholder = Some("Password".into());
                e.traits = 0x22_0104_0000;
            }),
            [16.0, 128.0, 370.0, 34.0],
        );
        let submit = at(
            with(element(9, "SwiftUI.AccessibilityNode"), |e| {
                e.identifier = Some("login-submit".into());
                e.label = Some("Sign in".into());
                e.traits = 0x2_0000_0001;
            }),
            [16.0, 178.0, 75.33, 34.33],
        );
        let status = at(
            with(element(48, "SwiftUI.AccessibilityNode"), |e| {
                e.identifier = Some("login-status".into());
                e.label = Some("Signed out".into());
                e.traits = 0x2_0000_0040;
            }),
            [16.0, 228.33, 82.33, 20.33],
        );
        let tab = at(
            with(element(9, "_UITabButton"), |e| {
                e.identifier = Some("tab-login".into());
                e.label = Some("Login".into());
                e.traits = 0x2_1000_0009;
            }),
            [25.0, 795.0, 77.0, 54.0],
        );
        let content = with(element(0, "SwiftUI.AccessibilityNode"), |e| {
            e.children = vec![username, password, submit, status]
        });
        let tab_bar = at(
            with(element(22, "UITabBar"), |e| {
                e.label = Some("Tab Bar".into());
                e.children = vec![tab];
            }),
            [0.0, 791.0, 402.0, 83.0],
        );
        let window = with(element(2, "UIWindow"), |e| {
            e.children = vec![content, tab_bar]
        });
        with(application, |e| e.children = vec![window])
    }

    fn nodes(snapshot: &Snapshot) -> Vec<&Node> {
        fn walk<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
            out.push(node);
            for child in &node.children {
                walk(child, out);
            }
        }
        let mut out = Vec::new();
        for window in &snapshot.windows {
            if let Some(root) = &window.root {
                walk(root, &mut out);
            }
        }
        out
    }

    fn by_id<'a>(snapshot: &'a Snapshot, identifier: &str) -> &'a Node {
        nodes(snapshot)
            .into_iter()
            .find(|node| node.res_id.as_deref() == Some(identifier))
            .unwrap_or_else(|| panic!("no node {identifier}"))
    }

    fn class<'a>(snapshot: &'a Snapshot, node: &Node) -> &'a str {
        &snapshot.strings[node.class as usize]
    }

    /// Whether a node of `window` has this identifier, or this label when
    /// the identifier starts with `label:`.
    fn holds(window: &Window, wanted: &str) -> bool {
        fn walk(node: &Node, wanted: &str) -> bool {
            let found = match wanted.strip_prefix("label:") {
                Some(label) => node.desc.as_deref() == Some(label),
                None => node.res_id.as_deref() == Some(wanted),
            };
            found || node.children.iter().any(|child| walk(child, wanted))
        }
        window.root.as_ref().is_some_and(|root| walk(root, wanted))
    }

    fn owner_name<'a>(snapshot: &'a Snapshot, window: &Window) -> Option<&'a str> {
        window
            .owner
            .map(|index| snapshot.strings[index as usize].as_str())
    }

    fn button(class: &str, identifier: Option<&str>, label: &str, frame: [f64; 4]) -> Element {
        at(
            with(element(9, class), |e| {
                e.identifier = identifier.map(str::to_owned);
                e.label = Some(label.into());
                e.traits = traits::BUTTON;
            }),
            frame,
        )
    }

    /// An alert controller's container titled `title` with one button,
    /// which carries `identifier` where the platform allows one.
    fn alert_container(title: &str, action: &str, identifier: Option<&str>) -> Element {
        let action = button(
            "_UIAlertControllerActionView",
            identifier,
            action,
            [57.0, 463.0, 288.0, 48.0],
        );
        at(
            with(element(3, "_UIAlertControllerPhoneTVMacView"), |e| {
                e.label = Some(title.into());
                e.children = vec![action];
            }),
            [41.0, 375.0, 320.0, 152.0],
        )
    }

    /// A presentation's transition view: two dimming views and the alert.
    fn presentation(alert: Element) -> Element {
        with(element(0, "UITransitionView"), |e| {
            e.children = vec![element(0, "UIView"), element(0, "UIView"), alert]
        })
    }

    /// The System screen in a transition view, with its request button.
    fn system_screen() -> Element {
        with(element(0, "UITransitionView"), |e| {
            e.children = vec![button(
                "SwiftUI.AccessibilityNode",
                Some("system-permission"),
                "Request location permission",
                [16.0, 78.0, 238.67, 34.33],
            )]
        })
    }

    /// The SwiftUI fixture's System screen under its in-app alert, as the
    /// reader returned it on iOS 26.5, containers trimmed: the one window
    /// holds the screen in a transition view and the alert, with two
    /// dimming views, in another.
    fn swiftui_alert() -> Element {
        let alert = alert_container("In-app alert", "OK", Some("inapp-alert-ok"));
        let window = with(element(2, "UIWindow"), |e| {
            e.children = vec![system_screen(), presentation(alert)]
        });
        with(element(1, "SwiftUIApplication"), |e| {
            e.label = Some("FixtureSwiftUI".into());
            e.children = vec![window];
        })
    }

    /// The Expo fixture's System screen under its in-app alert: React
    /// Native presents the alert in a window of its own, next to the empty
    /// view of the controller that presents it, and its button carries no
    /// identifier.
    fn expo_alert() -> Element {
        let screen = with(element(2, "UIWindow"), |e| {
            e.children = vec![system_screen()]
        });
        let presenter = with(element(0, "UITransitionView"), |e| {
            e.children = vec![with(element(0, "UIDropShadowView"), |e| {
                e.children = vec![element(0, "UIView")]
            })]
        });
        let alert = alert_container("In-app alert", "OK", None);
        let alert_window = with(element(2, "UIWindow"), |e| {
            e.children = vec![presenter, presentation(alert)]
        });
        with(element(1, "UIApplication"), |e| {
            e.label = Some("FixtureExpo".into());
            e.children = vec![screen, alert_window];
        })
    }

    /// The keyboard's window with one key.
    fn keyboard() -> Element {
        let key = at(
            with(element(20, "UIAccessibilityElementKBKey"), |e| {
                e.label = Some("q".into())
            }),
            [4.67, 590.0, 39.33, 54.0],
        );
        with(element(2, KEYBOARD_WINDOW_CLASS), |e| {
            e.children = vec![key]
        })
    }

    /// The keyboard's window over Bluesky's search screen, as the reader read
    /// it on iOS 26.5 for task 10.21, keys trimmed to one: the window and its
    /// input set container span the screen, and the host view below them
    /// holds the prediction bar (with a scroll view's wrapper reaching far
    /// beyond the screen), the backdrop, the key plane (the `Keyboard`
    /// element, 583 to 816 points down) and the dock with the globe and
    /// dictation keys, which ends at the bottom of the screen.
    fn bluesky_keyboard() -> Element {
        let predictions = at(
            with(element(0, "TUISystemInputAssistantView"), |e| {
                e.identifier = Some("SystemInputAssistantView".into());
                e.children = vec![at(element(0, "UIView"), [0.0, -61.0, 402.0, 1244.0])];
            }),
            [0.0, 539.0, 402.0, 44.0],
        );
        let backdrop = at(
            element(0, "UIKBInputBackdropView"),
            [0.0, 539.0, 402.0, 335.0],
        );
        let key = at(
            with(element(20, "UIAccessibilityElementKBKey"), |e| {
                e.label = Some("q".into())
            }),
            [4.67, 590.0, 39.33, 54.0],
        );
        let keys = at(
            with(element(19, "UIKeyboardAutomatic"), |e| {
                e.children = vec![key]
            }),
            [0.0, 583.0, 402.0, 233.0],
        );
        let input = at(
            with(element(0, "_UIKBCompatInputView"), |e| {
                e.children = vec![keys]
            }),
            [0.0, 583.0, 402.0, 233.0],
        );
        let globe = button(
            "UIKeyboardDockItemButton",
            None,
            "Next keyboard",
            [8.48, 805.7, 68.09, 68.74],
        );
        let dictation = button(
            "UIKeyboardDockItemButton",
            Some("dictation"),
            "Dictate",
            [325.44, 805.37, 68.09, 68.74],
        );
        let dock = at(
            with(element(0, "UIKeyboardDockView"), |e| {
                e.children = vec![globe, dictation]
            }),
            [0.0, 583.0, 402.0, 291.0],
        );
        let host = at(
            with(element(0, "UIInputSetHostView"), |e| {
                e.children = vec![predictions, backdrop, input, dock]
            }),
            [0.0, 539.0, 402.0, 335.0],
        );
        with(element(2, KEYBOARD_WINDOW_CLASS), |e| {
            e.children = vec![with(element(0, "UIInputSetContainerView"), |e| {
                e.children = vec![host]
            })]
        })
    }

    /// SpringBoard under the location permission alert, containers trimmed:
    /// a window of the status bar's and the one holding the alert.
    fn springboard_alert() -> Read {
        let alert = alert_container(
            "Allow “FixtureSwiftUI” to use your location?",
            "Don’t Allow",
            None,
        );
        let status_bar = with(element(2, "UIWindow"), |e| {
            e.children = vec![at(
                element(0, "_SBSystemApertureMagiciansCurtainView"),
                [138.33, 14.0, 125.33, 36.67],
            )]
        });
        let window = with(element(2, "UIWindow"), |e| {
            e.children = vec![presentation(alert)]
        });
        let mut read = read(with(element(1, "SpringBoard"), |e| {
            e.label = Some(" ".into());
            e.children = vec![status_bar, window];
        }));
        read.owner = Owner {
            pid: 22789,
            bundle_id: Some(SPRINGBOARD.into()),
            name: "SpringBoard".into(),
            system: true,
        };
        read
    }

    /// What the runtime reports as each node's child count: everything the
    /// read delivered.
    fn counted(mut element: Element) -> Element {
        element.children = element.children.into_iter().map(counted).collect();
        element.children_count = Some(element.children.len() as u64);
        element
    }

    /// UIKit's dimming view behind a presented sheet: three screens wide and
    /// tall around the screen, taking touches or not.
    fn dimming(takes_touches: Option<bool>) -> Element {
        at(
            with(element(0, DIMMING_CLASS), |e| {
                e.user_interaction = takes_touches
            }),
            [-402.0, -874.0, 1206.0, 2622.0],
        )
    }

    /// A sheet UIKit presents: a transition view holding `dimming` and the
    /// sheet's drop shadow view at `frame` with `content`.
    fn sheet(dimming: Element, frame: [f64; 4], content: Vec<Element>) -> Element {
        let shadow = at(
            with(element(0, "UIDropShadowView"), |e| e.children = content),
            frame,
        );
        with(element(0, PRESENTATION_CLASS), |e| {
            e.children = vec![dimming, shadow]
        })
    }

    /// The controller a sheet is presented from, in its transition view:
    /// its drop shadow view holds a dimming view of its own, which takes
    /// touches, under the content.
    fn presenting(content: Vec<Element>) -> Element {
        let mut children = vec![dimming(Some(true))];
        children.extend(content);
        with(element(0, PRESENTATION_CLASS), |e| {
            e.children = vec![with(element(0, "UIDropShadowView"), |e| {
                e.children = children
            })]
        })
    }

    /// The text effects window UIKit keeps next to an application's own,
    /// whose gesture view spans the screen.
    fn text_effects() -> Element {
        with(element(2, "UITextEffectsWindow"), |e| {
            e.children = vec![with(element(0, "UITrackingWindowView"), |e| {
                e.children = vec![element(0, "UIEditingOverlayGestureView")]
            })]
        })
    }

    /// Contacts' New Contact sheet over its list, as the reader read it on
    /// iOS 26.5, containers trimmed: the list in the presenting transition
    /// view, the sheet with its grabber and navigation bar in another, the
    /// text effects window, and the keyboard for the focused first field.
    /// Every node reports the children it delivered.
    fn contacts_sheet(takes_touches: Option<bool>) -> Element {
        let contact = at(
            with(element(75, "UICollectionViewListCell"), |e| {
                e.label = Some("John Appleseed".into())
            }),
            [2.0, 154.0, 384.0, 60.0],
        );
        let grabber = button(
            "_UIGrabber",
            None,
            "Sheet Grabber",
            [163.0, 57.0, 76.0, 25.0],
        );
        let title = at(
            with(element(48, "UILabel"), |e| {
                e.label = Some("New Contact".into());
                e.traits = traits::HEADER;
            }),
            [149.0, 90.0, 103.0, 21.0],
        );
        let bar = at(
            with(element(21, "UINavigationBar"), |e| {
                e.identifier = Some("New Contact".into());
                e.children = vec![title];
            }),
            [0.0, 78.0, 402.0, 54.0],
        );
        let window = with(element(2, "UIWindow"), |e| {
            e.children = vec![
                presenting(vec![contact]),
                sheet(
                    dimming(takes_touches),
                    [0.0, 62.0, 402.0, 812.0],
                    vec![grabber, bar],
                ),
            ]
        });
        counted(with(element(1, "UIApplication"), |e| {
            e.label = Some("Contacts".into());
            e.children = vec![window, text_effects(), keyboard()];
        }))
    }

    /// Bluesky's hosting-provider sheet over its sign-in form, from the raw
    /// tree of the Bluesky smoke test (reports/10.4-rn-smoke.md): a sheet at
    /// the bottom of the screen, whose dimming view, like the presenting
    /// controller's, reads "dismiss popup" and PopoverDismissRegion.
    fn bluesky_sheet() -> Element {
        let dismiss = |e: &mut Element| {
            e.label = Some("dismiss popup".into());
            e.identifier = Some("PopoverDismissRegion".into());
        };
        let username = at(
            with(element(49, "RCTUITextField"), |e| {
                e.identifier = Some("loginUsernameInput".into());
                e.label = Some("Username or email address".into());
            }),
            [56.0, 195.67, 314.0, 44.33],
        );
        let form = with(presenting(vec![username]), |e| {
            let shadow = &mut e.children[0];
            dismiss(&mut shadow.children[0]);
        });
        let automatic = at(
            with(element(0, "RCTViewComponentView"), |e| {
                e.identifier = Some("automaticSelectBtn".into());
                e.label = Some("Automatic".into());
            }),
            [34.89, 609.95, 166.11, 38.41],
        );
        let done = button(
            "RCTViewComponentView",
            Some("doneBtn"),
            "Done",
            [31.04, 800.39, 339.91, 42.57],
        );
        let window = with(element(2, "UIWindow"), |e| {
            e.children = vec![
                form,
                sheet(
                    with(dimming(Some(true)), dismiss),
                    [8.0, 545.29, 386.0, 320.71],
                    vec![automatic, done],
                ),
            ]
        });
        counted(with(element(1, "UIApplication"), |e| {
            e.label = Some("Blue Sky".into());
            e.children = vec![window, text_effects()];
        }))
    }

    fn window_ids(snapshot: &Snapshot) -> Vec<u32> {
        snapshot.windows.iter().map(|window| window.id).collect()
    }

    #[test]
    fn a_read_with_the_keyboard_up_is_not_truncated() {
        let mut root = login();
        root.children.push(keyboard());
        let snapshot = snapshot(&read(counted(root)));
        assert!(
            snapshot.truncated.is_empty(),
            "the keyboard's window moved out; nothing was cut: {:?}",
            snapshot.truncated
        );
        assert_eq!(window_ids(&snapshot), [4242 | KEYBOARD_WINDOW, 4242]);
        assert!(holds(&snapshot.windows[0], "label:q"));
    }

    #[test]
    fn an_alert_moved_to_its_own_window_leaves_the_read_whole() {
        for root in [swiftui_alert(), expo_alert()] {
            let snapshot = snapshot(&read(counted(root)));
            assert!(snapshot.truncated.is_empty(), "{:?}", snapshot.truncated);
            assert!(snapshot.windows.iter().any(|window| window.modal));
        }
    }

    #[test]
    fn a_sheet_that_takes_touches_moves_to_a_modal_window_above_the_screen() {
        let snapshot = snapshot(&read(contacts_sheet(Some(true))));
        assert!(snapshot.truncated.is_empty(), "{:?}", snapshot.truncated);
        assert_eq!(
            window_ids(&snapshot),
            [4242 | KEYBOARD_WINDOW, 4242 | SHEET_WINDOW, 4242]
        );
        let [keyboard, sheet, application] = &snapshot.windows[..] else {
            unreachable!()
        };
        assert!(sheet.modal);
        assert_eq!(sheet.kind, WindowType::APPLICATION);
        assert_eq!(sheet.title.as_deref(), Some("New Contact"));
        assert!(
            sheet
                .flags
                .contains(WindowFlags::ACTIVE | WindowFlags::FOCUSED)
        );
        assert!(holds(sheet, "label:Sheet Grabber"));
        assert!(!holds(sheet, "label:John Appleseed"));
        assert_eq!(
            class(&snapshot, sheet.root.as_ref().unwrap()),
            PRESENTATION_CLASS
        );
        assert!(!application.modal);
        assert!(application.flags.is_empty(), "the sheet has the focus");
        assert!(
            holds(application, "label:John Appleseed"),
            "the application keeps what the sheet covers"
        );
        assert!(!holds(application, "label:Sheet Grabber"));
        assert!(keyboard.layer > sheet.layer && sheet.layer > application.layer);
        assert_eq!(sheet.package, application.package);
        assert_eq!(sheet.emits_events, Some(false));
    }

    #[test]
    fn blueskys_bottom_sheet_is_a_modal_window_and_its_presenter_is_not() {
        let snapshot = snapshot(&read(bluesky_sheet()));
        assert!(snapshot.truncated.is_empty(), "{:?}", snapshot.truncated);
        assert_eq!(window_ids(&snapshot), [4242 | SHEET_WINDOW, 4242]);
        let [sheet, application] = &snapshot.windows[..] else {
            unreachable!()
        };
        assert!(sheet.modal);
        assert!(holds(sheet, "automaticSelectBtn") && holds(sheet, "doneBtn"));
        assert_eq!(
            sheet.bounds,
            Bounds::new(0, 0, 1206, 2622),
            "the window spans the transition view, the screen"
        );
        assert!(!application.modal);
        assert!(holds(application, "loginUsernameInput"));
    }

    #[test]
    fn a_sheet_that_lets_touches_through_stays_in_the_application_window() {
        for takes_touches in [Some(false), None] {
            let snapshot = snapshot(&read(contacts_sheet(takes_touches)));
            assert_eq!(
                window_ids(&snapshot),
                [4242 | KEYBOARD_WINDOW, 4242],
                "{takes_touches:?}"
            );
            let application = &snapshot.windows[1];
            assert!(!application.modal);
            assert!(application.flags.contains(WindowFlags::ACTIVE));
            assert!(holds(application, "label:Sheet Grabber"));
        }
    }

    #[test]
    fn a_dimming_view_that_does_not_span_the_screen_dims_nothing() {
        let mut root = contacts_sheet(Some(true));
        root.children[0].children[1].children[0].frame = Some([0.0, 700.0, 402.0, 174.0]);
        let snapshot = snapshot(&read(root));
        assert_eq!(window_ids(&snapshot), [4242 | KEYBOARD_WINDOW, 4242]);
    }

    #[test]
    fn an_alert_over_a_sheet_is_the_top_most_modal() {
        let mut root = contacts_sheet(Some(true));
        let alert = presentation(alert_container("Discard changes?", "Discard", None));
        root.children[0].children.push(counted(alert));
        root.children[0].children_count = Some(3);
        let snapshot = snapshot(&read(root));
        assert!(snapshot.truncated.is_empty(), "{:?}", snapshot.truncated);
        assert_eq!(
            window_ids(&snapshot),
            [
                4242 | KEYBOARD_WINDOW,
                4242 | ALERT_WINDOW,
                4242 | SHEET_WINDOW,
                4242
            ]
        );
        let layers: Vec<i32> = snapshot.windows.iter().map(|window| window.layer).collect();
        assert!(
            layers.windows(2).all(|pair| pair[0] > pair[1]),
            "{layers:?}"
        );
        let [_, alert, sheet, application] = &snapshot.windows[..] else {
            unreachable!()
        };
        assert!(alert.modal && sheet.modal && !application.modal);
        assert!(alert.flags.contains(WindowFlags::FOCUSED));
        assert!(sheet.flags.is_empty() && application.flags.is_empty());
    }

    #[test]
    fn system_ui_keeps_a_sheet_in_its_window() {
        let mut springboard = springboard_alert();
        springboard.root.children = vec![with(element(2, "UIWindow"), |e| {
            e.children = vec![
                presenting(vec![]),
                sheet(dimming(Some(true)), [0.0, 62.0, 402.0, 812.0], vec![]),
            ]
        })];
        let snapshot = snapshot(&springboard);
        assert_eq!(window_ids(&snapshot), [22789]);
        assert!(!snapshot.windows[0].modal);
    }

    #[test]
    fn one_window_per_process_named_by_its_pid_bundle_and_label() {
        let snapshot = snapshot(&read(login()));
        assert_eq!(snapshot.windows.len(), 1);
        let window = &snapshot.windows[0];
        assert_eq!(window.id, 4242);
        assert_eq!(window.kind, WindowType::APPLICATION);
        assert_eq!(
            window
                .package
                .map(|i| snapshot.strings[i as usize].as_str()),
            Some("dev.mobileagent.fixture.swiftui")
        );
        assert_eq!(window.title.as_deref(), Some("FixtureSwiftUI"));
        assert_eq!(window.bounds, Bounds::new(0, 0, 1206, 2622));
        assert!(window.flags.contains(WindowFlags::ACTIVE));
        let root = window.root.as_ref().expect("a root");
        assert_eq!(class(&snapshot, root), "XCUIElementTypeApplication");
        assert_eq!(root.desc.as_deref(), Some("FixtureSwiftUI"));
        assert_eq!(snapshot.node_count, 9);
        assert!(snapshot.truncated.is_empty());
    }

    #[test]
    fn classes_are_type_names_or_the_platform_class_of_untyped_elements() {
        let snapshot = snapshot(&read(login()));
        let classes: Vec<&str> = nodes(&snapshot)
            .into_iter()
            .map(|node| class(&snapshot, node))
            .collect();
        assert_eq!(
            classes,
            [
                "XCUIElementTypeApplication",
                "XCUIElementTypeWindow",
                "SwiftUI.AccessibilityNode",
                "XCUIElementTypeTextField",
                "XCUIElementTypeSecureTextField",
                "XCUIElementTypeButton",
                "XCUIElementTypeStaticText",
                "XCUIElementTypeTabBar",
                "XCUIElementTypeButton",
            ]
        );
        let alert = with(element(3, "_UIAlertControllerPhoneTVMacView"), |e| {
            e.label = Some("In-app alert".into())
        });
        assert_eq!(class_name(4, &alert), "_UIAlertControllerPhoneTVMacView");
        let header = with(element(48, "UILabel"), |e| e.traits = traits::HEADER);
        assert_eq!(class_name(3, &header), "XCUIElementTypeHeading");
        let untyped = Element::default();
        assert_eq!(class_name(3, &untyped), "XCUIElementTypeOther");
        assert_eq!(
            class_name(
                3,
                &with(Element::default(), |e| e.automation_type = Some(3))
            ),
            "XCUIElementTypeGroup"
        );
    }

    #[test]
    fn labels_values_and_placeholders_become_descriptions_and_text() {
        let snapshot = snapshot(&read(login()));
        let username = by_id(&snapshot, "login-username");
        assert_eq!(username.desc.as_deref(), Some("Username"));
        assert_eq!(
            username.text, None,
            "an empty field's value is its placeholder"
        );
        let submit = by_id(&snapshot, "login-submit");
        assert_eq!(submit.desc.as_deref(), Some("Sign in"));
        assert_eq!(submit.text, None);

        let typed = snapshot_of(with(element(49, "UITextField"), |e| {
            e.identifier = Some("field".into());
            e.label = Some("Username".into());
            e.value = Some("alice".into());
            e.placeholder = Some("Username".into());
        }));
        let field = by_id(&typed, "field");
        assert_eq!(field.desc.as_deref(), Some("Username"));
        assert_eq!(field.text.as_deref(), Some("alice"));
    }

    fn snapshot_of(child: Element) -> Snapshot {
        let root = with(element(1, "UIApplication"), |e| e.children = vec![child]);
        snapshot(&read(root))
    }

    #[test]
    fn flags_come_from_the_type_and_traits() {
        let snapshot = snapshot(&read(login()));
        let username = by_id(&snapshot, "login-username");
        for flag in [NodeFlags::EDITABLE, NodeFlags::ENABLED, NodeFlags::VISIBLE] {
            assert!(username.flags.contains(flag), "{flag:?}");
        }
        assert!(!username.flags.contains(NodeFlags::PASSWORD));
        assert!(!username.flags.contains(NodeFlags::FOCUSED));
        let password = by_id(&snapshot, "login-password");
        assert!(password.flags.contains(NodeFlags::PASSWORD));
        let submit = by_id(&snapshot, "login-submit");
        assert!(submit.flags.contains(NodeFlags::CLICKABLE));
        assert!(!submit.flags.contains(NodeFlags::SELECTED));
        let tab = by_id(&snapshot, "tab-login");
        assert!(tab.flags.contains(NodeFlags::SELECTED));

        let focused = snapshot_of(with(element(49, "UITextField"), |e| {
            e.identifier = Some("field".into());
            e.traits = 0x22_0024_0000;
        }));
        assert!(by_id(&focused, "field").flags.contains(NodeFlags::FOCUSED));

        let disabled = snapshot_of(with(element(9, "UIButton"), |e| {
            e.identifier = Some("off".into());
            e.traits = traits::BUTTON | traits::NOT_ENABLED;
        }));
        assert!(!by_id(&disabled, "off").flags.contains(NodeFlags::ENABLED));

        let switch = snapshot_of(with(element(40, "UISwitch"), |e| {
            e.identifier = Some("toggle".into());
            e.label = Some("Animate".into());
            e.value = Some("1".into());
        }));
        let toggle = by_id(&switch, "toggle");
        assert!(toggle.flags.contains(NodeFlags::CHECKABLE));
        assert!(toggle.flags.contains(NodeFlags::CHECKED));
        assert_eq!(toggle.text, None, "a switch's value is its checked flag");
    }

    #[test]
    fn only_frames_on_screen_are_visible() {
        let below = at(
            with(element(75, "UICollectionViewCell"), |e| {
                e.identifier = Some("cell-500".into())
            }),
            [8.0, 2400.0, 19.33, 27.67],
        );
        let empty = at(
            with(element(20, "UIAccessibilityElementKBKey"), |e| {
                e.identifier = Some("padding".into())
            }),
            [0.0, 583.0, 0.0, 0.0],
        );
        let root = with(element(1, "UIApplication"), |e| {
            e.children = vec![below, empty]
        });
        let snapshot = snapshot(&read(root));
        assert!(
            !by_id(&snapshot, "cell-500")
                .flags
                .contains(NodeFlags::VISIBLE)
        );
        assert!(
            !by_id(&snapshot, "padding")
                .flags
                .contains(NodeFlags::VISIBLE)
        );
    }

    #[test]
    fn bounds_are_pixels_rounded_from_points() {
        let snapshot = snapshot(&read(login()));
        let submit = by_id(&snapshot, "login-submit");
        assert_eq!(submit.bounds, Bounds::new(48, 534, 274, 637));
    }

    #[test]
    fn node_ids_are_unique_in_pre_order_and_strings_are_shared() {
        let snapshot = snapshot(&read(login()));
        let ids: Vec<u32> = nodes(&snapshot).iter().map(|node| node.id).collect();
        assert_eq!(ids, (1..=9).collect::<Vec<u32>>());
        let buttons = snapshot
            .strings
            .iter()
            .filter(|s| *s == "XCUIElementTypeButton")
            .count();
        assert_eq!(buttons, 1);
    }

    #[test]
    fn the_keyboard_window_becomes_an_input_method_window_on_top() {
        let key = at(
            with(element(20, "UIAccessibilityElementKBKey"), |e| {
                e.label = Some("q".into())
            }),
            [4.67, 590.0, 39.33, 54.0],
        );
        let keyboard = with(element(2, KEYBOARD_WINDOW_CLASS), |e| {
            e.children = vec![key]
        });
        let mut root = login();
        root.children.push(keyboard);
        let snapshot = snapshot(&read(root));
        assert_eq!(snapshot.windows.len(), 2);
        let top = &snapshot.windows[0];
        assert_eq!(top.kind, WindowType::INPUT_METHOD);
        assert_eq!(top.id, 4242 | KEYBOARD_WINDOW);
        assert_eq!(top.layer, 1);
        let keys = top
            .root
            .as_ref()
            .expect("the keyboard window")
            .children
            .len();
        assert_eq!(keys, 1);
        let app = &snapshot.windows[1];
        assert_eq!(app.id, 4242);
        assert_eq!(snapshot.node_count, 11);
        let app_nodes = {
            let one = Snapshot {
                windows: vec![app.clone()],
                ..snapshot.clone()
            };
            nodes(&one).len()
        };
        assert_eq!(app_nodes, 9, "the keys left the application's window");
    }

    /// Task 10.21: the window spans the screen, so with its frame as its
    /// bounds the mirror counted every element under it, the sign-in button
    /// and the search history above the keyboard included, as 0% in view.
    #[test]
    fn the_keyboard_window_covers_the_input_views_it_holds_not_the_screen() {
        let mut root = login();
        root.children.push(bluesky_keyboard());
        let snapshot = snapshot(&read(counted(root)));
        let keyboard = &snapshot.windows[0];
        assert_eq!(keyboard.kind, WindowType::INPUT_METHOD);
        assert_eq!(
            keyboard.bounds,
            Bounds::new(0, 1617, 1206, 2622),
            "the host view's frame: the prediction bar from 539 points down, the keys and the dock"
        );
        assert_eq!(
            keyboard.root.as_ref().expect("the window's node").bounds,
            Bounds::new(0, 0, 1206, 2622),
            "the window's own node keeps the window's frame"
        );
        assert!(holds(keyboard, "label:q") && holds(keyboard, "dictation"));
    }

    #[test]
    fn a_keyboard_sliding_in_covers_only_its_part_on_screen() {
        let mut root = login();
        let mut sliding = bluesky_keyboard();
        sliding.children[0].children[0].frame = Some([0.0, 800.0, 402.0, 335.0]);
        root.children.push(sliding);
        let snapshot = snapshot(&read(counted(root)));
        assert_eq!(snapshot.windows[0].bounds, Bounds::new(0, 2400, 1206, 2622));
    }

    #[test]
    fn a_keyboard_window_that_shows_nothing_covers_nothing() {
        let mut root = login();
        root.children
            .push(with(element(2, KEYBOARD_WINDOW_CLASS), |e| {
                e.children = vec![element(0, "UIInputSetContainerView")]
            }));
        let snapshot = snapshot(&read(counted(root)));
        let keyboard = &snapshot.windows[0];
        assert_eq!(keyboard.kind, WindowType::INPUT_METHOD);
        assert_eq!(keyboard.bounds, Bounds::default());
    }

    #[test]
    fn system_ui_owns_a_system_window_titled_by_its_name() {
        let mut springboard = read(with(element(1, "SpringBoard"), |e| {
            e.label = Some(" ".into())
        }));
        springboard.owner = Owner {
            pid: 22789,
            bundle_id: Some(SPRINGBOARD.into()),
            name: "SpringBoard".into(),
            system: true,
        };
        let snapshot = snapshot(&springboard);
        let window = &snapshot.windows[0];
        assert_eq!(window.kind, WindowType::SYSTEM);
        assert_eq!(window.title.as_deref(), Some("SpringBoard"));
        assert_eq!(window.root.as_ref().unwrap().desc, None);
    }

    #[test]
    fn an_in_app_alert_moves_to_a_modal_window_above_the_application() {
        let snapshot = snapshot(&read(swiftui_alert()));
        assert_eq!(snapshot.windows.len(), 2, "{:?}", snapshot.windows);
        let (alert, app) = (&snapshot.windows[0], &snapshot.windows[1]);
        assert!(alert.modal, "the alert's window is a modal");
        assert_eq!(alert.id, 4242 | ALERT_WINDOW);
        assert_eq!(alert.kind, WindowType::APPLICATION);
        assert_eq!(alert.layer, 1);
        assert_eq!(alert.package, app.package);
        assert_eq!(alert.title.as_deref(), Some("In-app alert"));
        assert_eq!(alert.flags, WindowFlags::ACTIVE | WindowFlags::FOCUSED);
        assert_eq!(alert.bounds, Bounds::new(0, 0, 1206, 2622));
        assert_eq!(alert.owner, None);
        let presentation = alert.root.as_ref().expect("the alert's presentation");
        assert_eq!(class(&snapshot, presentation), "UITransitionView");
        assert!(holds(alert, "inapp-alert-ok"));
        assert!(!holds(alert, "system-permission"));

        assert!(!app.modal, "the application's window is not a modal");
        assert_eq!(app.id, 4242);
        assert_eq!(app.layer, 0);
        assert_eq!(
            app.flags,
            WindowFlags::EMPTY,
            "the alert's window has the focus"
        );
        assert!(
            holds(app, "system-permission"),
            "what the alert covers stays in the payload, for the mirror"
        );
        assert!(!holds(app, "inapp-alert-ok"));
        assert_eq!(snapshot.node_count, 9);
        assert_eq!(nodes(&snapshot).len(), 9, "no node is lost");
    }

    #[test]
    fn an_alert_in_a_window_of_its_own_moves_with_its_presentation() {
        let snapshot = snapshot(&read(expo_alert()));
        assert_eq!(snapshot.windows.len(), 2, "{:?}", snapshot.windows);
        let (alert, app) = (&snapshot.windows[0], &snapshot.windows[1]);
        assert!(alert.modal && !app.modal);
        assert_eq!(alert.id, 4242 | ALERT_WINDOW);
        assert_eq!(alert.title.as_deref(), Some("In-app alert"));
        let presentation = alert.root.as_ref().expect("the alert's presentation");
        assert_eq!(class(&snapshot, presentation), "UITransitionView");
        assert_eq!(
            presentation.children.len(),
            3,
            "the dimming views and the alert"
        );
        assert!(holds(alert, "label:OK"));
        assert!(holds(app, "system-permission"));
        // The alert's window stays in the application's, holding only the
        // empty view of the controller that presented the alert.
        let application = app.root.as_ref().expect("the application");
        assert_eq!(application.children.len(), 2);
        assert!(!holds(app, "label:OK"));
    }

    #[test]
    fn an_alert_that_is_not_on_screen_stays_where_it_is() {
        let mut root = swiftui_alert();
        root.children[0].children[1].children[2].frame = Some([41.0, 375.0, 0.0, 0.0]);
        let snapshot = snapshot(&read(root));
        assert_eq!(snapshot.windows.len(), 1);
        assert!(!snapshot.windows[0].modal);
        assert_eq!(
            snapshot.windows[0].flags,
            WindowFlags::ACTIVE | WindowFlags::FOCUSED
        );
    }

    #[test]
    fn system_ui_owns_its_windows_by_process_name_and_keeps_its_alert() {
        let mut springboard = springboard_alert();
        springboard.root.children.push(keyboard());
        let snapshot = snapshot(&springboard);
        assert_eq!(snapshot.windows.len(), 2, "{:?}", snapshot.windows);
        for window in &snapshot.windows {
            assert_eq!(
                owner_name(&snapshot, window),
                Some("SpringBoard"),
                "window {}",
                window.id
            );
            assert!(!window.modal, "window {}", window.id);
        }
        let system = &snapshot.windows[1];
        assert_eq!(system.kind, WindowType::SYSTEM);
        assert!(
            holds(system, "label:Don’t Allow"),
            "the alert stays in SpringBoard's window, whose other content it covers"
        );
        let names = snapshot
            .strings
            .iter()
            .filter(|s| *s == "SpringBoard")
            .count();
        assert_eq!(names, 1, "interned once: {:?}", snapshot.strings);
    }

    #[test]
    fn an_applications_windows_have_no_owner() {
        let mut root = swiftui_alert();
        root.children.push(keyboard());
        let snapshot = snapshot(&read(root));
        assert_eq!(snapshot.windows.len(), 3);
        for window in &snapshot.windows {
            assert_eq!(window.owner, None, "window {}", window.id);
        }
    }

    #[test]
    fn every_window_says_it_sends_no_change_events() {
        let mut root = swiftui_alert();
        root.children.push(keyboard());
        let mut springboard = springboard_alert();
        springboard.root.children.push(keyboard());
        for snapshot in [snapshot(&read(root)), snapshot(&springboard)] {
            for window in &snapshot.windows {
                assert_eq!(window.emits_events, Some(false), "window {}", window.id);
            }
        }
    }

    #[test]
    fn the_keyboard_stays_above_an_alert_window() {
        let mut root = swiftui_alert();
        root.children.push(keyboard());
        let snapshot = snapshot(&read(root));
        let stack: Vec<(u32, i32, bool)> = snapshot
            .windows
            .iter()
            .map(|window| (window.id, window.layer, window.modal))
            .collect();
        assert_eq!(
            stack,
            [
                (4242 | KEYBOARD_WINDOW, 2, false),
                (4242 | ALERT_WINDOW, 1, true),
                (4242, 0, false),
            ]
        );
    }

    /// A scroll view with the identifier `list` at `frame`, whose frame's
    /// edges the runtime puts at `vertical` and `horizontal` of its content,
    /// and whose content shows in `visible`.
    fn scroller(
        automation_type: u64,
        class: &str,
        frame: [f64; 4],
        visible: [f64; 4],
        vertical: [f64; 2],
        horizontal: [f64; 2],
    ) -> Element {
        at(
            with(element(automation_type, class), |e| {
                e.identifier = Some("list".into());
                e.scroll_area = Some(ScrollArea {
                    vertical: Some(vertical),
                    horizontal: Some(horizontal),
                    visible_frame: Some(visible),
                });
            }),
            frame,
        )
    }

    /// The SwiftUI fixture's List as the runtime reported it on iOS 26.5: a
    /// collection view over the whole screen whose 200 rows of 52 points
    /// (10,400 in all) scroll under the status bar's 62 points and the tab
    /// bar's 83, which its visible frame leaves out.
    fn swiftui_list(vertical: [f64; 2]) -> Element {
        scroller(
            types::COLLECTION_VIEW,
            "SwiftUI.UpdateCoalescingCollectionView",
            [0.0, 0.0, 402.0, 874.0],
            [0.0, 62.0, 402.0, 729.0],
            vertical,
            [0.0, 1.0],
        )
    }

    /// The Expo fixture's List: a scroll view between the status bar and
    /// the tab bar over 200 rows of 48 points (9,600), without insets.
    fn expo_list(vertical: [f64; 2]) -> Element {
        scroller(
            types::SCROLL_VIEW,
            "RCTEnhancedScrollView",
            [0.0, 62.0, 402.0, 722.0],
            [0.0, 62.0, 402.0, 722.0],
            vertical,
            [0.0, 1.0],
        )
    }

    fn scroll_of(element: Element) -> Option<Scroll> {
        by_id(&snapshot_of(element), "list").scroll
    }

    fn percent(first: i32, last: i32) -> (Option<ScrollRange>, Option<ScrollPosition>) {
        (
            Some(ScrollRange {
                unit: ScrollUnit::PERCENT,
                min: 0,
                max: 100,
            }),
            Some(ScrollPosition { first, last }),
        )
    }

    #[test]
    fn a_list_at_its_top_scrolls_forward_only() {
        // At rest, the collection view's frame starts 62 points above its
        // content, under the status bar.
        let swiftui = scroll_of(swiftui_list([-0.005962, 0.078077])).expect("scroll semantics");
        assert_eq!(swiftui.axis, ScrollAxis::VERTICAL);
        assert_eq!(swiftui.flags, ScrollFlags::FORWARD);
        assert_eq!((swiftui.range, swiftui.position), percent(0, 7));
        let expo = scroll_of(expo_list([0.0, 0.075208])).expect("scroll semantics");
        assert_eq!(expo.axis, ScrollAxis::VERTICAL);
        assert_eq!(expo.flags, ScrollFlags::FORWARD);
        assert_eq!((expo.range, expo.position), percent(0, 8));
    }

    #[test]
    fn a_list_scrolled_down_scrolls_both_ways() {
        // 20 points down: the frame's top is still above the content, but
        // the visible frame's is not.
        let nudged = scroll_of(swiftui_list([-0.004038, 0.08])).expect("scroll semantics");
        assert_eq!(nudged.flags, ScrollFlags::BACK | ScrollFlags::FORWARD);
        assert_eq!(
            (nudged.range, nudged.position),
            percent(1, 7),
            "the position leaves 0 as soon as the start is out of view"
        );
        let expo = scroll_of(expo_list([0.002083, 0.077292])).expect("scroll semantics");
        assert_eq!(expo.flags, ScrollFlags::BACK | ScrollFlags::FORWARD);
        assert_eq!((expo.range, expo.position), percent(1, 8));
        let further = scroll_of(swiftui_list([0.03766, 0.121699])).expect("scroll semantics");
        assert_eq!(further.flags, ScrollFlags::BACK | ScrollFlags::FORWARD);
        assert_eq!((further.range, further.position), percent(4, 11));
    }

    #[test]
    fn a_list_at_its_end_scrolls_back_only() {
        // The frame's bottom is 83 points past the content, under the tab bar.
        let swiftui = scroll_of(swiftui_list([0.923942, 1.007981])).expect("scroll semantics");
        assert_eq!(swiftui.flags, ScrollFlags::BACK);
        assert_eq!((swiftui.range, swiftui.position), percent(93, 100));
        let expo = scroll_of(expo_list([0.924792, 1.0])).expect("scroll semantics");
        assert_eq!(expo.flags, ScrollFlags::BACK);
        assert_eq!((expo.range, expo.position), percent(92, 100));
    }

    #[test]
    fn content_that_fits_its_visible_frame_does_not_scroll() {
        // An alert's message: 72.33 points of text in 72 points of scroll view.
        let header = scroller(
            types::SCROLL_VIEW,
            "_UIInterfaceActionGroupHeaderScrollView",
            [41.0, 375.0, 320.0, 72.0],
            [41.0, 375.0, 320.0, 72.0],
            [0.0, 0.995392],
            [0.0, 1.0],
        );
        assert_eq!(scroll_of(header), None);
        let exact = scroller(
            types::SCROLL_VIEW,
            "UIScrollView",
            [41.0, 447.0, 320.0, 80.0],
            [41.0, 447.0, 320.0, 80.0],
            [0.0, 1.0],
            [0.0, 1.0],
        );
        assert_eq!(scroll_of(exact), None);
        // Three rows of 52 points under the SwiftUI list's insets.
        let short = swiftui_list([-62.0 / 156.0, 812.0 / 156.0]);
        assert_eq!(scroll_of(short), None);
    }

    #[test]
    fn content_taller_than_the_visible_frame_scrolls_though_the_frame_holds_it() {
        // 800 points of rows: less than the 874-point frame, more than the
        // 729 points between the bars.
        let list = swiftui_list([-62.0 / 800.0, 812.0 / 800.0]);
        let scroll = scroll_of(list).expect("scroll semantics");
        assert_eq!(scroll.flags, ScrollFlags::FORWARD);
        assert_eq!((scroll.range, scroll.position), percent(0, 91));
    }

    #[test]
    fn a_carousel_scrolls_horizontally() {
        let carousel = scroller(
            types::COLLECTION_VIEW,
            "UICollectionView",
            [0.0, 100.0, 402.0, 200.0],
            [0.0, 100.0, 402.0, 200.0],
            [0.0, 1.0],
            [0.5, 0.75],
        );
        let scroll = scroll_of(carousel).expect("scroll semantics");
        assert_eq!(scroll.axis, ScrollAxis::HORIZONTAL);
        assert_eq!(scroll.flags, ScrollFlags::BACK | ScrollFlags::FORWARD);
        assert_eq!((scroll.range, scroll.position), percent(50, 75));
    }

    #[test]
    fn content_that_scrolls_both_ways_names_no_axis() {
        let map = scroller(
            types::SCROLL_VIEW,
            "UIScrollView",
            [0.0, 0.0, 402.0, 874.0],
            [0.0, 0.0, 402.0, 874.0],
            [0.0, 0.5],
            [0.5, 1.0],
        );
        let scroll = scroll_of(map).expect("scroll semantics");
        assert_eq!(scroll.axis, ScrollAxis::UNKNOWN);
        assert_eq!(scroll.flags, ScrollFlags::BACK | ScrollFlags::FORWARD);
        assert_eq!((scroll.range, scroll.position), (None, None));
    }

    #[test]
    fn elements_without_a_scroll_area_have_no_scroll_semantics() {
        // What the runtime reports for an element that is no scroll area.
        let not_an_area = scroller(
            types::SCROLL_VIEW,
            "UIScrollView",
            [0.0, 0.0, 402.0, 874.0],
            [0.0, 0.0, 402.0, 874.0],
            [2147483647.0, 0.0],
            [2147483647.0, 0.0],
        );
        assert_eq!(scroll_of(not_an_area), None);
        let snapshot = snapshot(&read(login()));
        assert!(nodes(&snapshot).iter().all(|node| node.scroll.is_none()));
        // Without a visible frame, the whole frame shows content.
        let mut bare = expo_list([0.0, 0.075208]);
        bare.scroll_area.as_mut().unwrap().visible_frame = None;
        let scroll = scroll_of(bare).expect("scroll semantics");
        assert_eq!((scroll.range, scroll.position), percent(0, 8));
    }

    #[test]
    fn the_node_limit_truncates_and_says_so() {
        let mut limited = read(login());
        limited.limits.max_nodes = 4;
        let snapshot = snapshot(&limited);
        assert_eq!(snapshot.node_count, 4);
        assert_eq!(nodes(&snapshot).len(), 4);
        assert!(snapshot.truncated.contains(Truncation::NODES));
    }

    #[test]
    fn the_depth_limit_truncates_and_says_so() {
        let mut shallow = read(login());
        shallow.limits.max_depth = 2;
        let cut_short = snapshot(&shallow);
        assert!(cut_short.truncated.contains(Truncation::DEPTH));
        assert!(nodes(&cut_short).iter().all(|node| node.res_id.is_none()));

        let mut rejected = read(login());
        rejected.depth_cut = true;
        assert!(snapshot(&rejected).truncated.contains(Truncation::DEPTH));
    }

    #[test]
    fn withheld_children_mark_the_read_truncated() {
        let mut root = login();
        root.children[0].children_count = Some(5);
        let snapshot = snapshot(&read(root));
        assert!(snapshot.truncated.contains(Truncation::NODES));
    }

    #[test]
    fn limits_come_from_the_request_capped_at_the_protocol_depth() {
        let limits = Limits::from_params(&SnapshotParams {
            max_depth: Some(1000),
            ..SnapshotParams::default()
        });
        assert_eq!(limits.max_depth, MAX_NODE_DEPTH as u32);
        assert_eq!(limits.max_nodes, 10_000);
        assert_eq!(
            Limits::from_params(&SnapshotParams::default()),
            Limits::default()
        );
    }

    #[test]
    fn xcui_element_types_are_named_both_ways() {
        assert_eq!(xcui_element_type_name(9), Some("Button"));
        assert_eq!(xcui_element_type_name(48), Some("StaticText"));
        assert_eq!(xcui_element_type_name(83), None);
        assert_eq!(xcui_element_type("SecureTextField"), Some(50));
        assert_eq!(xcui_element_type("Nothing"), None);
    }
}
