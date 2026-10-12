//! Builds the fake target's trees: elements in device points with the
//! classes each platform's driver reports, turned into the protocol payload
//! a driver returns, bounds in physical pixels.

use mirror::backend::{BackendInfo, Owner, TreeRead};
use protocol::snapshot::{
    Bounds, Node, NodeFlags, Scroll, Snapshot, Truncation, Window, WindowFlags, WindowType,
};

use crate::driver::Platform;

/// One element, bounds in device points as `[left, top, right, bottom]`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct El {
    pub class: &'static str,
    /// The identifier taps and text find the element by.
    pub id: Option<String>,
    /// Whether the tree reports `id` as the element's test identifier; the
    /// fake still finds an element whose app gives it none by it.
    pub reported: bool,
    pub text: Option<String>,
    pub desc: Option<String>,
    pub rect: [f64; 4],
    pub flags: NodeFlags,
    /// How the element scrolls, for one that does.
    pub scroll: Option<Scroll>,
    pub children: Vec<El>,
}

impl El {
    /// An enabled, visible element.
    pub fn new(class: &'static str, rect: [f64; 4]) -> El {
        El {
            class,
            id: None,
            reported: true,
            text: None,
            desc: None,
            rect,
            flags: NodeFlags::ENABLED | NodeFlags::VISIBLE,
            scroll: None,
            children: Vec::new(),
        }
    }

    pub fn scroll(mut self, scroll: Scroll) -> El {
        self.scroll = Some(scroll);
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> El {
        self.id = Some(id.into());
        self.reported = true;
        self
    }

    /// The element found by `key`, which the tree does not report: an
    /// element the app gives no test identifier.
    pub fn key(mut self, key: impl Into<String>) -> El {
        self.id = Some(key.into());
        self.reported = false;
        self
    }

    pub fn text(mut self, text: impl Into<String>) -> El {
        self.text = Some(text.into());
        self
    }

    pub fn desc(mut self, desc: impl Into<String>) -> El {
        self.desc = Some(desc.into());
        self
    }

    pub fn with(mut self, flags: NodeFlags) -> El {
        self.flags |= flags;
        self
    }

    /// Sets `flags` when `on`, clears them otherwise.
    pub fn set(mut self, flags: NodeFlags, on: bool) -> El {
        if on {
            self.flags |= flags;
        } else {
            self.flags = NodeFlags(self.flags.0 & !flags.0);
        }
        self
    }

    pub fn child(mut self, child: El) -> El {
        self.children.push(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = El>) -> El {
        self.children.extend(children);
        self
    }
}

/// SpringBoard's window id on a fake Simulator.
const SPRINGBOARD_WINDOW: u32 = 900;

/// The bit the Simulator's reader sets in an application's window id for
/// the window of its alert.
const ALERT_WINDOW: u32 = 1 << 30;

/// The bit the Simulator's reader sets in an application's window id for
/// the window of its keyboard.
const KEYBOARD_WINDOW: u32 = 1 << 31;

/// An application's window id on a fake device.
const APP_WINDOW: u32 = 1;

/// One window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Win {
    pub id: u32,
    pub kind: WindowType,
    pub layer: i32,
    pub package: Option<String>,
    pub title: Option<String>,
    pub rect: [f64; 4],
    pub flags: WindowFlags,
    pub events: Option<bool>,
    pub owner: Owner,
    pub modal: bool,
    pub root: El,
}

/// Whether `rect` contains the point.
pub(crate) fn contains(rect: [f64; 4], x: f64, y: f64) -> bool {
    x >= rect[0] && x < rect[2] && y >= rect[1] && y < rect[3]
}

/// `el` and its subtree with their bounds cut to `rect`.
fn clip(mut el: El, rect: [f64; 4]) -> El {
    el.rect = [
        el.rect[0].max(rect[0]),
        el.rect[1].max(rect[1]),
        el.rect[2].min(rect[2]),
        el.rect[3].min(rect[3]),
    ];
    el.children = el
        .children
        .into_iter()
        .map(|child| clip(child, rect))
        .collect();
    el
}

/// The identifier a tap at `(x, y)` in points lands on: the innermost
/// identified element on the path to the innermost visible element under
/// the point, in the top-most window that takes the tap. A modal window
/// takes every tap; a tap it does not contain lands nowhere.
pub(crate) fn hit(windows: &[Win], x: f64, y: f64) -> Option<String> {
    for window in windows {
        if !contains(window.rect, x, y) {
            if window.modal {
                return None;
            }
            continue;
        }
        return hit_el(&window.root, x, y, None).flatten();
    }
    None
}

fn hit_el(el: &El, x: f64, y: f64, identified: Option<&str>) -> Option<Option<String>> {
    if !contains(el.rect, x, y) || !el.flags.contains(NodeFlags::VISIBLE) {
        return None;
    }
    let identified = el.id.as_deref().or(identified);
    for child in el.children.iter().rev() {
        if let Some(found) = hit_el(child, x, y, identified) {
            return Some(found);
        }
    }
    Some(identified.map(str::to_owned))
}

/// The identifier of payload node `node` in window `window`, numbering
/// nodes in pre-order from 1 across the windows as [`tree_read`] does.
pub(crate) fn identifier_of(windows: &[Win], window: u32, node: u32) -> Option<String> {
    let mut next = 1u32;
    for w in windows {
        let mut stack = vec![&w.root];
        while let Some(el) = stack.pop() {
            let id = next;
            next += 1;
            if w.id == window && id == node {
                return el.id.clone();
            }
            stack.extend(el.children.iter().rev());
        }
    }
    None
}

/// The read a driver returns for `windows`.
pub(crate) fn tree_read(windows: &[Win], scale: f64, backend: BackendInfo) -> TreeRead {
    let mut strings: Vec<String> = Vec::new();
    let mut next = 1u32;
    let mut payload = Vec::with_capacity(windows.len());
    let mut count = 0u32;
    for w in windows {
        let before = next;
        let root = node(&w.root, scale, &mut strings, &mut next);
        count += next - before;
        payload.push(Window {
            id: w.id,
            kind: w.kind,
            layer: w.layer,
            package: w.package.as_deref().map(|p| intern(&mut strings, p)),
            title: w.title.clone(),
            bounds: pixels(w.rect, scale),
            flags: w.flags,
            root: Some(root),
            emits_events: w.events,
            owner: match &w.owner {
                Owner::System(name) => Some(intern(&mut strings, name)),
                Owner::Target => None,
            },
            modal: w.modal,
        });
    }
    TreeRead {
        snapshot: Snapshot {
            strings,
            windows: payload,
            node_count: count,
            truncated: Truncation::EMPTY,
            seq: 0,
        },
        backend,
        scale,
    }
}

fn intern(strings: &mut Vec<String>, s: &str) -> u32 {
    let index = match strings.iter().position(|x| x == s) {
        Some(index) => index,
        None => {
            strings.push(s.to_owned());
            strings.len() - 1
        }
    };
    u32::try_from(index).unwrap_or(u32::MAX)
}

fn pixels(rect: [f64; 4], scale: f64) -> Bounds {
    let px = |v: f64| (v * scale).round() as i32;
    Bounds::new(px(rect[0]), px(rect[1]), px(rect[2]), px(rect[3]))
}

fn node(el: &El, scale: f64, strings: &mut Vec<String>, next: &mut u32) -> Node {
    let id = *next;
    *next += 1;
    let class = intern(strings, el.class);
    let children = el
        .children
        .iter()
        .map(|child| node(child, scale, strings, next))
        .collect();
    Node {
        id,
        class,
        res_pkg: None,
        res_id: el.id.clone().filter(|_| el.reported),
        text: el.text.clone(),
        desc: el.desc.clone(),
        bounds: pixels(el.rect, scale),
        flags: el.flags,
        children,
        scroll: el.scroll,
    }
}

/// The classes and geometry of one platform's fake screens: Android widget
/// classes with text in `text`, or `XCUIElementType` names with labels in
/// `desc` and values in `text`, as the Simulator's reader reports them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kit {
    pub platform: Platform,
}

const TAPPABLE: NodeFlags = NodeFlags(NodeFlags::CLICKABLE.0 | NodeFlags::FOCUSABLE.0);

impl Kit {
    /// Physical pixels per point: a 420 dpi Android screen, or an iPhone.
    pub fn scale(self) -> f64 {
        match self.platform {
            Platform::Android => 2.625,
            Platform::Ios => 3.0,
        }
    }

    pub fn width(self) -> f64 {
        match self.platform {
            Platform::Android => 408.0,
            Platform::Ios => 402.0,
        }
    }

    pub fn height(self) -> f64 {
        match self.platform {
            Platform::Android => 912.0,
            Platform::Ios => 874.0,
        }
    }

    /// The top and bottom of an app's content, between the status bar and
    /// the tab bar.
    pub fn content(self) -> (f64, f64) {
        match self.platform {
            Platform::Android => (24.0, 840.0),
            Platform::Ios => (62.0, 791.0),
        }
    }

    pub fn tab_bar(self) -> [f64; 4] {
        match self.platform {
            Platform::Android => [0.0, 848.0, 408.0, 912.0],
            Platform::Ios => [0.0, 791.0, 402.0, 874.0],
        }
    }

    /// A full-width row at `top` of the content, `height` tall, inset 16.
    pub fn line(self, top: f64, height: f64) -> [f64; 4] {
        let (content, _) = self.content();
        [
            16.0,
            content + top,
            self.width() - 16.0,
            content + top + height,
        ]
    }

    pub fn backend(self) -> BackendInfo {
        BackendInfo::new("fake", false)
    }

    pub fn heading(self, id: &str, label: &str, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("dev.mobileagent.fake.Heading", rect).text(label),
            Platform::Ios => El::new("XCUIElementTypeHeading", rect).desc(label),
        }
        .id(id)
    }

    /// An image with an identifier and no label.
    pub fn image(self, id: &str, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("android.widget.ImageView", rect),
            Platform::Ios => El::new("XCUIElementTypeImage", rect),
        }
        .id(id)
    }

    pub fn label(self, id: Option<&str>, label: &str, rect: [f64; 4]) -> El {
        let el = match self.platform {
            Platform::Android => El::new("android.widget.TextView", rect).text(label),
            Platform::Ios => El::new("XCUIElementTypeStaticText", rect).desc(label),
        };
        match id {
            Some(id) => el.id(id),
            None => el,
        }
    }

    pub fn button(self, id: &str, label: &str, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("android.widget.Button", rect)
                .text(label)
                .with(TAPPABLE),
            Platform::Ios => El::new("XCUIElementTypeButton", rect).desc(label),
        }
        .id(id)
    }

    /// A tappable container named by a text child, as React Native renders
    /// a pressable on Android; a button on iOS.
    pub fn pressable(self, id: &str, label: &str, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => {
                let inner = [rect[0] + 16.0, rect[1] + 8.0, rect[2] - 16.0, rect[3] - 8.0];
                El::new("android.view.ViewGroup", rect)
                    .with(TAPPABLE)
                    .id(id)
                    .child(El::new("android.widget.TextView", inner).text(label))
            }
            Platform::Ios => self.button(id, label, rect),
        }
    }

    pub fn field(
        self,
        id: &str,
        placeholder: &str,
        value: &str,
        secure: bool,
        focused: bool,
        rect: [f64; 4],
    ) -> El {
        let shown = if secure {
            "•".repeat(value.chars().count())
        } else {
            value.to_owned()
        };
        let el = match self.platform {
            Platform::Android => El::new("android.widget.EditText", rect)
                .with(TAPPABLE)
                .with(NodeFlags::EDITABLE),
            Platform::Ios if secure => El::new("XCUIElementTypeSecureTextField", rect),
            Platform::Ios => El::new("XCUIElementTypeTextField", rect),
        }
        .id(id)
        .desc(placeholder)
        .set(NodeFlags::PASSWORD, secure)
        .set(NodeFlags::FOCUSED, focused);
        if shown.is_empty() { el } else { el.text(shown) }
    }

    pub fn switch(self, id: &str, label: &str, on: bool, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("android.widget.Switch", rect)
                .text(label)
                .with(TAPPABLE),
            Platform::Ios => El::new("XCUIElementTypeSwitch", rect).desc(label),
        }
        .id(id)
        .with(NodeFlags::CHECKABLE)
        .set(NodeFlags::CHECKED, on)
    }

    pub fn tab(self, id: &str, label: &str, selected: bool, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("android.view.ViewGroup", rect)
                .desc(label)
                .with(TAPPABLE),
            Platform::Ios => El::new("XCUIElementTypeButton", rect).desc(label),
        }
        .id(id)
        .set(NodeFlags::SELECTED, selected)
    }

    /// A grid cell with the button role the fixture contract gives cells.
    pub fn cell(self, id: &str, label: &str, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("android.view.ViewGroup", rect)
                .desc(label)
                .with(TAPPABLE),
            Platform::Ios => El::new("XCUIElementTypeButton", rect).desc(label),
        }
        .id(id)
    }

    pub fn group(self, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("android.view.ViewGroup", rect),
            Platform::Ios => El::new("XCUIElementTypeOther", rect),
        }
    }

    /// `el` as the platform reports it inside a scroll container at
    /// `viewport`: Android clips an element's bounds and its subtree's to
    /// its ancestors (`getBoundsInScreen`), so a row the list cuts off reads
    /// as the part that shows; iOS reports whole frames.
    pub fn inside(self, el: El, viewport: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => clip(el, viewport),
            Platform::Ios => el,
        }
    }

    /// A vertically scrolling list container.
    pub fn list(self, id: &str, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("androidx.recyclerview.widget.RecyclerView", rect)
                .with(NodeFlags::SCROLLABLE),
            Platform::Ios => El::new("XCUIElementTypeCollectionView", rect),
        }
        .id(id)
    }

    /// A vertically scrolling view without an identifier, as React Native's
    /// `ScrollView` reads.
    pub fn scroll_view(self, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("android.widget.ScrollView", rect),
            Platform::Ios => El::new("XCUIElementTypeScrollView", rect),
        }
        .with(NodeFlags::SCROLLABLE)
    }

    pub fn grid(self, id: &str, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("android.widget.GridView", rect),
            Platform::Ios => El::new("XCUIElementTypeOther", rect),
        }
        .id(id)
    }

    pub fn spinner(self, id: &str, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("android.widget.ProgressBar", rect),
            Platform::Ios => El::new("XCUIElementTypeActivityIndicator", rect),
        }
        .id(id)
    }

    /// An alert inside an app window, as iOS reports an alert controller;
    /// Android shows dialogs in windows of their own, see [`Kit::dialog`].
    pub fn alert(self, title: &str, rect: [f64; 4], children: Vec<El>) -> El {
        El::new("_UIAlertControllerPhoneTVMacView", rect)
            .desc(title)
            .children(children)
    }

    /// An app's window: its package, the title the platform reports and
    /// the elements below the root.
    pub fn app_window(self, package: &str, title: &str, content: Vec<El>) -> Win {
        let screen = [0.0, 0.0, self.width(), self.height()];
        let (layer, events, root) = match self.platform {
            Platform::Android => (
                1,
                Some(true),
                El::new("android.widget.FrameLayout", screen).children(content),
            ),
            Platform::Ios => (
                0,
                Some(false),
                El::new("XCUIElementTypeApplication", screen)
                    .desc(title)
                    .child(El::new("XCUIElementTypeWindow", screen).children(content)),
            ),
        };
        Win {
            id: APP_WINDOW,
            kind: WindowType::APPLICATION,
            layer,
            package: Some(package.to_owned()),
            title: Some(title.to_owned()),
            rect: screen,
            flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
            events,
            owner: Owner::Target,
            modal: false,
            root,
        }
    }

    /// A labelled node without traits: React Native's view marked
    /// accessible without a role, which a field it holds can stand behind.
    pub fn generic(self, id: &str, label: &str, rect: [f64; 4]) -> El {
        self.group(rect).id(id).desc(label)
    }

    /// A toolbar, which the mirror takes for a bar drawn over what lies
    /// under it.
    pub fn toolbar(self, id: &str, rect: [f64; 4]) -> El {
        match self.platform {
            Platform::Android => El::new("androidx.appcompat.widget.Toolbar", rect),
            Platform::Ios => El::new("XCUIElementTypeToolbar", rect),
        }
        .id(id)
    }

    /// A text that shows `text`, or nothing.
    pub fn text(self, text: Option<&str>, rect: [f64; 4]) -> El {
        let el = match self.platform {
            Platform::Android => El::new("android.widget.TextView", rect),
            Platform::Ios => El::new("XCUIElementTypeStaticText", rect),
        };
        match (text, self.platform) {
            (Some(text), Platform::Android) => el.text(text),
            (Some(text), Platform::Ios) => el.desc(text),
            (None, _) => el,
        }
    }

    /// SpringBoard's window on a Simulator, holding `content`: system UI,
    /// which the reader names by its process as the owner of its windows.
    pub fn springboard(self, content: Vec<El>) -> Win {
        let screen = [0.0, 0.0, self.width(), self.height()];
        Win {
            id: SPRINGBOARD_WINDOW,
            kind: WindowType::SYSTEM,
            layer: 0,
            package: Some("com.apple.springboard".to_owned()),
            title: Some("SpringBoard".to_owned()),
            rect: screen,
            flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
            events: Some(false),
            owner: Owner::System("SpringBoard".to_owned()),
            modal: false,
            root: El::new("XCUIElementTypeApplication", screen)
                .desc("SpringBoard")
                .children(content),
        }
    }

    /// The modal window the reader gives an application's alert on a
    /// Simulator: above `app_window`, its id the app window's with bit 30
    /// set, titled by the alert, active and focused in the app window's
    /// stead, holding the view that presents the alert (a transition view
    /// with two dimming views and the alert).
    pub fn alert_window(self, package: &str, app_window: &Win, alert: El) -> Win {
        let screen = [0.0, 0.0, self.width(), self.height()];
        let presenter = El::new("UITransitionView", screen).children([
            El::new("UIView", screen),
            El::new("UIView", screen),
            alert.clone(),
        ]);
        Win {
            id: app_window.id | ALERT_WINDOW,
            kind: WindowType::APPLICATION,
            layer: app_window.layer + 1,
            package: Some(package.to_owned()),
            title: alert.desc.clone(),
            rect: screen,
            flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
            events: Some(false),
            owner: Owner::Target,
            modal: true,
            root: presenter,
        }
    }

    /// Android's status bar window; iOS draws its status bar in
    /// SpringBoard, which the reader does not read with the app.
    pub fn status_bar(self) -> Option<Win> {
        if self.platform != Platform::Android {
            return None;
        }
        let rect = [0.0, 0.0, self.width(), 24.0];
        Some(Win {
            id: 2,
            kind: WindowType::SYSTEM,
            layer: 3,
            package: Some("com.android.systemui".to_owned()),
            title: Some("Status bar".to_owned()),
            rect,
            flags: WindowFlags::EMPTY,
            events: Some(true),
            owner: Owner::System("com.android.systemui".to_owned()),
            modal: false,
            root: El::new("android.widget.FrameLayout", rect)
                .child(El::new("android.widget.TextView", [16.0, 0.0, 56.0, 24.0]).text("12:30")),
        })
    }

    /// Where the keyboard's window starts, in points from the top: the fake
    /// Gboard's; on an iPhone, the keyboard's host view from its prediction
    /// bar down, as the Simulator's reader bounds the keyboard's window
    /// (task 10.21: from 539 points down on an iPhone 17).
    pub fn keyboard_top(self) -> f64 {
        match self.platform {
            Platform::Android => 600.0,
            Platform::Ios => 539.0,
        }
    }

    /// The keyboard's window over `app`: on Android the input method's; on
    /// iOS the one the Simulator's reader gives the keyboard
    /// (`UIRemoteKeyboardWindow`), its id the app window's with bit 31 set,
    /// with the app's package, bounded by what the keyboard draws, its root
    /// the window's own node across the screen, holding the keyboard.
    pub fn keyboard(self, app: &str) -> Win {
        let rect = [0.0, self.keyboard_top(), self.width(), self.height()];
        match self.platform {
            Platform::Android => Win {
                id: 5,
                kind: WindowType::INPUT_METHOD,
                layer: 4,
                package: Some("com.google.android.inputmethod.latin".to_owned()),
                title: Some("Keyboard".to_owned()),
                rect,
                flags: WindowFlags::EMPTY,
                events: Some(true),
                owner: Owner::System("com.google.android.inputmethod.latin".to_owned()),
                modal: false,
                root: El::new("android.widget.FrameLayout", rect),
            },
            Platform::Ios => {
                let screen = [0.0, 0.0, self.width(), self.height()];
                let keys = [0.0, rect[1] + 44.0, self.width(), self.height() - 58.0];
                Win {
                    id: APP_WINDOW | KEYBOARD_WINDOW,
                    kind: WindowType::INPUT_METHOD,
                    layer: 1,
                    package: Some(app.to_owned()),
                    title: None,
                    rect,
                    flags: WindowFlags::EMPTY,
                    events: Some(false),
                    owner: Owner::Target,
                    modal: false,
                    root: El::new("XCUIElementTypeWindow", screen)
                        .child(El::new("XCUIElementTypeKeyboard", keys)),
                }
            }
        }
    }

    /// An Android dialog window that covers what lies below it, owned by
    /// the app or by system UI such as the permission controller.
    pub fn dialog(
        self,
        id: u32,
        package: &str,
        owner: Owner,
        title: &str,
        content: Vec<El>,
    ) -> Win {
        let rect = [40.0, 320.0, self.width() - 40.0, 600.0];
        Win {
            id,
            kind: WindowType::APPLICATION,
            layer: 2,
            package: Some(package.to_owned()),
            title: Some(title.to_owned()),
            rect,
            flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
            events: Some(true),
            owner,
            modal: true,
            root: El::new("android.widget.FrameLayout", rect).children(content),
        }
    }
}
