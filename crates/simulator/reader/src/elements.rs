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
//!
//! One read is one process, the one that owns the foreground: its
//! application element becomes a window whose id is the process id, and the
//! keyboard window inside it (`UIRemoteKeyboardWindow`) a separate input
//! method window, which the mirror collapses to one line.

use protocol::message::SnapshotParams;
use protocol::snapshot::{
    Bounds, MAX_NODE_DEPTH, Node, NodeFlags, Snapshot, Truncation, Window, WindowFlags, WindowType,
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
}

/// The process whose tree was read: the one that owns the foreground.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Owner {
    pub pid: i32,
    pub bundle_id: Option<String>,
    /// The process's name, for the window title when the application element
    /// has no label.
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

    // The keyboard window leaves the application's tree for its own window.
    let mut app = read.root.clone();
    let keyboards: Vec<Element> = {
        let (keyboards, rest) = std::mem::take(&mut app.children)
            .into_iter()
            .partition(is_keyboard_window);
        app.children = rest;
        keyboards
    };

    let title = present(&app.label)
        .map(str::to_owned)
        .or_else(|| (!read.owner.name.is_empty()).then(|| read.owner.name.clone()));
    let app_root = builder.node(&app, 0);
    let mut windows = Vec::new();
    for keyboard in &keyboards {
        let root = builder.node(keyboard, 1);
        windows.push(Window {
            id: read.owner.pid as u32 | KEYBOARD_WINDOW,
            kind: WindowType::INPUT_METHOD,
            layer: 1,
            package,
            title: None,
            bounds: builder.bounds(keyboard.frame),
            flags: WindowFlags::EMPTY,
            root,
            // The Simulator sends no change events for any window (spikes S3 and S4); owner
            // and modal are filled properly by task 6.10.
            emits_events: Some(false),
            owner: None,
            modal: false,
        });
    }
    windows.push(Window {
        id: read.owner.pid as u32,
        kind: if read.owner.system {
            WindowType::SYSTEM
        } else {
            WindowType::APPLICATION
        },
        layer: 0,
        package,
        title,
        bounds: builder.bounds(app.frame),
        flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
        root: app_root,
        emits_events: Some(false),
        owner: None,
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
        let Some([x, y, width, height]) = frame else {
            return false;
        };
        if width <= 0.0 || height <= 0.0 {
            return false;
        }
        match self.screen {
            None => true,
            Some([sx, sy, sw, sh]) => {
                x < sx + sw && x + width > sx && y < sy + sh && y + height > sy
            }
        }
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
        if matches!(
            automation_type,
            Some(types::SCROLL_VIEW | types::TABLE | types::COLLECTION_VIEW)
        ) {
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
            // Scroll semantics for Simulator elements arrive with task 6.10.
            scroll: None,
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
            max_nodes: None,
            idle_ms: None,
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
