//! The SwiftUI fixture as the fake runner reads it, for the device tests'
//! helpers run on the fake device (`setups.rs`): its List and Login tabs on
//! the iPhone SE (3rd generation)'s 375 by 667 points, in pixels as the
//! runner's tree has them, above the floating tab bar's five buttons. SwiftUI
//! sets the tab bar's identifiers after the first rows show (`tests/tree.rs`
//! waits for them), and on the phone the runner's read that listed `row-1`
//! was followed by one with no `tab-login` (task 9.7's device check), so the
//! first reads after a launch list the buttons by their labels alone.
//!
//! A launch through the fake DVT starts it on the tab `FIXTURE_INITIAL_TAB`
//! names, Login otherwise (`fixtures/CONTRACT.md`, Launch options). A touch
//! lifted through `dtuhidd` is a tap there: it selects the tab under it, or
//! gives keyboard focus to the field under it. Text goes into the field with
//! keyboard focus, typed after what it holds or set in its place, and is
//! refused without one, as the runner refuses it.

use plist::{Dictionary, Value};
use protocol::code;
use protocol::message::TextMethod;
use protocol::snapshot::{
    Bounds, Node, NodeFlags, Snapshot, Truncation, Window, WindowFlags, WindowType,
};

/// The fixture's bundle identifier and executable.
pub(crate) const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";
pub(crate) const EXECUTABLE: &str = "FixtureSwiftUI";

/// The SE's screen in points, and its pixels a point.
const WIDTH: f64 = 375.0;
const HEIGHT: f64 = 667.0;
const SCALE: f64 = 2.0;

/// The read after a launch from which the tab bar's buttons carry their
/// identifiers: the read that first lists the rows and the one after it
/// list the buttons by their labels alone.
pub(crate) const TAB_IDENTIFIERS_FROM: u32 = 3;

/// A `mainTouchscreen` report's state byte for a contact lifted.
const LIFTED: u8 = 0x02;

/// The class names the reader's conversion gives these elements.
const STRINGS: [&str; 8] = [
    SWIFTUI,
    "XCUIElementTypeApplication",
    "XCUIElementTypeTabBar",
    "XCUIElementTypeButton",
    "XCUIElementTypeCell",
    "XCUIElementTypeTextField",
    "XCUIElementTypeSecureTextField",
    "XCUIElementTypeStaticText",
];
const APPLICATION: u32 = 1;
const TAB_BAR: u32 = 2;
const BUTTON: u32 = 3;
const CELL: u32 = 4;
const TEXT_FIELD: u32 = 5;
const SECURE_TEXT_FIELD: u32 = 6;
const STATIC_TEXT: u32 = 7;

/// A rectangle in points: left, top, right, bottom.
#[derive(Clone, Copy, Debug)]
struct Rect(f64, f64, f64, f64);

impl Rect {
    fn contains(self, x: f64, y: f64) -> bool {
        (self.0..=self.2).contains(&x) && (self.1..=self.3).contains(&y)
    }

    /// In pixels, as the runner's tree has it.
    fn pixels(self) -> Bounds {
        let pixel = |value: f64| (value * SCALE).round() as i32;
        Bounds::new(pixel(self.0), pixel(self.1), pixel(self.2), pixel(self.3))
    }
}

/// The tab bar floats over the list from 588 to 642 points, and its Login
/// and List buttons are centred where the input tests tap them
/// (`tests/input.rs`); rows are 52 points tall from 20 points, so `row-12`
/// lies under the bar, and the fake leaves out the row whose middle the bar
/// covers, as XCTest left out `row-12`
/// (`reports/11.1-ios-device/probes/rowbox.txt`).
const BAR: Rect = Rect(0.0, 588.0, WIDTH, 642.0);
const ROW_HEIGHT: f64 = 52.0;
const FIRST_ROW_TOP: f64 = 20.0;
const USERNAME: Rect = Rect(16.0, 120.0, 359.0, 164.0);
const PASSWORD: Rect = Rect(16.0, 176.0, 359.0, 220.0);
const SUBMIT: Rect = Rect(16.0, 232.0, 359.0, 276.0);
const STATUS: Rect = Rect(16.0, 288.0, 359.0, 312.0);

/// The fixture's tabs, in the tab bar's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Login,
    List,
    Big,
    Anim,
    System,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::Login, Tab::List, Tab::Big, Tab::Anim, Tab::System];

    /// Its name in `FIXTURE_INITIAL_TAB` and in its identifier, `tab-<name>`.
    fn name(self) -> &'static str {
        match self {
            Tab::Login => "login",
            Tab::List => "list",
            Tab::Big => "big",
            Tab::Anim => "anim",
            Tab::System => "system",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Login => "Login",
            Tab::List => "List",
            Tab::Big => "Big",
            Tab::Anim => "Anim",
            Tab::System => "System",
        }
    }

    /// Its button in the tab bar, 62 points wide every 62.5 points from 31.
    fn button(self) -> Rect {
        let index = Tab::ALL.iter().position(|tab| *tab == self).unwrap_or(0) as f64;
        let left = 31.0 + 62.5 * index;
        Rect(left, BAR.1, left + 62.0, BAR.3)
    }
}

/// The fake SwiftUI fixture: the tab in front, the username field's focus
/// and text, and the reads since the last launch.
#[derive(Debug)]
pub(crate) struct FakeFixture {
    tab: Tab,
    focused: bool,
    username: String,
    reads: u32,
}

impl FakeFixture {
    /// The fixture as installed, before its first launch.
    pub(crate) fn swiftui() -> FakeFixture {
        FakeFixture {
            tab: Tab::Login,
            focused: false,
            username: String::new(),
            reads: 0,
        }
    }

    pub(crate) fn tab(&self) -> Tab {
        self.tab
    }

    /// Whether `login-username` has keyboard focus.
    pub(crate) fn focused(&self) -> bool {
        self.focused
    }

    /// What `login-username` holds.
    pub(crate) fn username(&self) -> &str {
        &self.username
    }

    /// The reads since the last launch.
    pub(crate) fn reads(&self) -> u32 {
        self.reads
    }

    /// A new process launched with `environment`, as DVT's process control
    /// passes it: on the tab `FIXTURE_INITIAL_TAB` names, nothing focused or
    /// typed, no read made.
    pub(crate) fn launch(&mut self, environment: Option<&Dictionary>) {
        let initial = environment
            .and_then(|environment| environment.get("FIXTURE_INITIAL_TAB"))
            .and_then(Value::as_string);
        *self = FakeFixture {
            tab: Tab::ALL
                .into_iter()
                .find(|tab| Some(tab.name()) == initial)
                .unwrap_or(Tab::Login),
            ..FakeFixture::swiftui()
        };
    }

    /// A `mainTouchscreen` report's first contact at the touchscreen's
    /// coordinates, 0 to 65535 across the screen on each axis: a contact
    /// lifted ends a tap there.
    pub(crate) fn touch(&mut self, state: u8, x: u16, y: u16) {
        if state != LIFTED {
            return;
        }
        let x = f64::from(x) / 65535.0 * WIDTH;
        let y = f64::from(y) / 65535.0 * HEIGHT;
        if let Some(tab) = Tab::ALL.into_iter().find(|tab| tab.button().contains(x, y)) {
            self.tab = tab;
            self.focused = false;
        } else if self.tab == Tab::Login && USERNAME.contains(x, y) {
            self.focused = true;
        }
    }

    /// Text entered into the field with keyboard focus: typed after what it
    /// holds, or set in its place; typed when the method is left to the
    /// runner.
    pub(crate) fn enter(
        &mut self,
        text: &str,
        mode: Option<TextMethod>,
    ) -> Result<TextMethod, protocol::Error> {
        if !self.focused {
            return Err(protocol::Error::new(
                code::INTERNAL,
                "no element of the foreground has keyboard focus",
            ));
        }
        if mode == Some(TextMethod::DIRECT) {
            self.username = text.to_owned();
            return Ok(TextMethod::DIRECT);
        }
        self.username.push_str(text);
        Ok(TextMethod::KEYS)
    }

    /// The runner's read of the screen.
    pub(crate) fn read(&mut self) -> Snapshot {
        self.reads += 1;
        let mut nodes = Nodes::default();
        let mut children = match self.tab {
            Tab::List => (1..=13)
                .filter_map(|row| {
                    let top = FIRST_ROW_TOP + ROW_HEIGHT * f64::from(row - 1);
                    let rect = Rect(0.0, top, WIDTH, top + ROW_HEIGHT);
                    let covered = (BAR.1..=BAR.3).contains(&(top + ROW_HEIGHT / 2.0));
                    (!covered).then(|| {
                        nodes.leaf(
                            CELL,
                            Some(format!("row-{row}")),
                            Some(format!("Row {row}")),
                            rect,
                            NodeFlags::CLICKABLE,
                        )
                    })
                })
                .collect(),
            Tab::Login => vec![
                nodes.leaf(
                    TEXT_FIELD,
                    Some("login-username".into()),
                    (!self.username.is_empty()).then(|| self.username.clone()),
                    USERNAME,
                    NodeFlags::EDITABLE
                        | NodeFlags::FOCUSABLE
                        | if self.focused {
                            NodeFlags::FOCUSED
                        } else {
                            NodeFlags::EMPTY
                        },
                ),
                nodes.leaf(
                    SECURE_TEXT_FIELD,
                    Some("login-password".into()),
                    None,
                    PASSWORD,
                    NodeFlags::EDITABLE | NodeFlags::FOCUSABLE | NodeFlags::PASSWORD,
                ),
                nodes.leaf(
                    BUTTON,
                    Some("login-submit".into()),
                    Some("Sign in".into()),
                    SUBMIT,
                    NodeFlags::CLICKABLE,
                ),
                nodes.leaf(
                    STATIC_TEXT,
                    Some("login-status".into()),
                    Some("Signed out".into()),
                    STATUS,
                    NodeFlags::EMPTY,
                ),
            ],
            Tab::Big | Tab::Anim | Tab::System => Vec::new(),
        };
        let identified = self.reads >= TAB_IDENTIFIERS_FROM;
        let buttons = Tab::ALL
            .into_iter()
            .map(|tab| {
                nodes.leaf(
                    BUTTON,
                    identified.then(|| format!("tab-{}", tab.name())),
                    Some(tab.label().into()),
                    tab.button(),
                    NodeFlags::CLICKABLE
                        | if tab == self.tab {
                            NodeFlags::SELECTED
                        } else {
                            NodeFlags::EMPTY
                        },
                )
            })
            .collect();
        children.push(nodes.node(TAB_BAR, None, None, BAR, NodeFlags::EMPTY, buttons));
        let screen = Rect(0.0, 0.0, WIDTH, HEIGHT);
        let root = nodes.node(APPLICATION, None, None, screen, NodeFlags::EMPTY, children);
        Snapshot {
            strings: STRINGS.iter().map(|name| (*name).to_owned()).collect(),
            windows: vec![Window {
                id: 1,
                kind: WindowType::APPLICATION,
                layer: 0,
                package: Some(0),
                title: None,
                bounds: screen.pixels(),
                flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
                root: Some(root),
                emits_events: Some(false),
                owner: None,
                modal: false,
            }],
            node_count: nodes.count,
            truncated: Truncation::EMPTY,
            seq: 0,
        }
    }
}

/// Numbers the nodes of one read.
#[derive(Default)]
struct Nodes {
    count: u32,
}

impl Nodes {
    fn node(
        &mut self,
        class: u32,
        res_id: Option<String>,
        text: Option<String>,
        rect: Rect,
        flags: NodeFlags,
        children: Vec<Node>,
    ) -> Node {
        self.count += 1;
        Node {
            id: self.count,
            class,
            res_pkg: None,
            res_id,
            text,
            desc: None,
            bounds: rect.pixels(),
            flags: flags | NodeFlags::ENABLED | NodeFlags::VISIBLE,
            children,
            scroll: None,
        }
    }

    fn leaf(
        &mut self,
        class: u32,
        res_id: Option<String>,
        text: Option<String>,
        rect: Rect,
        flags: NodeFlags,
    ) -> Node {
        self.node(class, res_id, text, rect, flags, Vec::new())
    }
}
