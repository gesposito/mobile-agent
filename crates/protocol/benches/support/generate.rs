//! The inputs of the codec benchmarks (design D14), generated from one seed
//! with integer arithmetic only, so their bytes are the same on every run and
//! every platform:
//!
//! - [`snapshot_500`]: a snapshot response of 500 nodes in the shape of spike
//!   S5's `T500`. One application window holds a mail screen (toolbar, tabs, a
//!   list laid out past both ends of its viewport, a bottom bar and a
//!   floating button), 7 levels deep below the window root, with a 16-entry
//!   string table of 14 view classes and 2 resource-id prefixes, resource ids
//!   on about 55% of the nodes, text on half of them and descriptions on about
//!   12%, in 1- to 4-byte UTF-8, and the list's scroll semantics.
//! - [`snapshot_1049`]: the 1,049 nodes the agent read of the Expo fixture's
//!   Big screen on the emulator (spike S1). The status bar's window holds 28
//!   nodes, 11 levels deep, and the app's 1,021: 500 cells, each a button
//!   holding its number as text, under one grid container, and 21 nodes of
//!   chrome around them.
//! - [`events_10k`]: 10,000 events split as S5's `E10K`, 8,975 on the tree
//!   stream and 1,025 on the window stream, each stream numbered from 1, with
//!   tree paths of 2 to 10 child indices and microsecond times past 2^32.
//! - [`requests_1k`]: 1,000 requests in S5's `R1K` mix, 595 taps, 198 text
//!   sets and 207 snapshot requests with their parameters, 691 of them with a
//!   deadline.
//!
//! Snapshots number their nodes and order their string table as the Android
//! agent does. Each dataset draws from its own stream of the generator, so a
//! change to one leaves the others' bytes alone. `tests/bench_inputs.rs`
//! checks the shapes and pins each dataset's encoded length and digest.

// Not every target that includes this file uses all of it.
#![allow(dead_code)]

use std::collections::VecDeque;

use protocol::Message;
use protocol::event::{
    Event, EventPayload, TreeChange, TreeChangeKind, WindowChange, WindowChangeKind,
};
use protocol::message::{
    Call, Method, Reply, Request, Response, SnapshotParams, TapParams, TextMethod, TextParams,
};
use protocol::snapshot::{
    Bounds, Node, NodeFlags, Scroll, ScrollAxis, ScrollFlags, ScrollPosition, ScrollRange,
    ScrollUnit, Snapshot, Truncation, Window, WindowFlags, WindowType,
};

/// The seed every dataset is drawn from.
pub const SEED: u64 = 20_261_007;

/// Every dataset, by the name its benchmarks carry.
pub fn datasets() -> Vec<(&'static str, Vec<Message>)> {
    vec![
        ("snapshot-500", vec![snapshot_500()]),
        ("snapshot-1049", vec![snapshot_1049()]),
        ("events-10k", events_10k()),
        ("requests-1k", requests_1k()),
    ]
}

/// The window the mail screen of [`snapshot_500`] is read from; the events
/// and the snapshot requests name it too.
const MAIL_WINDOW: u32 = 37;
/// A keyboard's window, which some tree events name.
const KEYBOARD_WINDOW: u32 = 41;

/// The emulator's screen in pixels, and where its status bar ends and its
/// gesture bar begins.
const WIDTH: i32 = 1080;
const HEIGHT: i32 = 2400;
const STATUS_BAR: i32 = 132;
const GESTURE_BAR: i32 = 2274;

const FRAME_LAYOUT: &str = "android.widget.FrameLayout";
const LINEAR_LAYOUT: &str = "android.widget.LinearLayout";
const CONSTRAINT_LAYOUT: &str = "androidx.constraintlayout.widget.ConstraintLayout";
const VIEW_GROUP: &str = "android.view.ViewGroup";
const RECYCLER_VIEW: &str = "androidx.recyclerview.widget.RecyclerView";
const HORIZONTAL_SCROLL_VIEW: &str = "android.widget.HorizontalScrollView";
const SCROLL_VIEW: &str = "android.widget.ScrollView";
const CARD_VIEW: &str = "androidx.cardview.widget.CardView";
const TEXT_VIEW: &str = "android.widget.TextView";
const IMAGE_VIEW: &str = "android.widget.ImageView";
const IMAGE_BUTTON: &str = "android.widget.ImageButton";
const BUTTON: &str = "android.widget.Button";
const CHECK_BOX: &str = "android.widget.CheckBox";
const SWITCH: &str = "android.widget.Switch";
const VIEW: &str = "android.view.View";

const ANDROID_IDS: &str = "android:id/";
const APP_IDS: &str = "dev.placeholder.fixture:id/";
const SYSTEMUI_IDS: &str = "com.android.systemui:id/";
const SYSTEMUI: &str = "com.android.systemui";
const EXPO: &str = "dev.mobileagent.fixture.expo";

/// On screen; and on screen and tappable.
const SHOWN: NodeFlags = NodeFlags(NodeFlags::ENABLED.0 | NodeFlags::VISIBLE.0);
const TAPPABLE: NodeFlags = NodeFlags(
    NodeFlags::CLICKABLE.0 | NodeFlags::FOCUSABLE.0 | NodeFlags::ENABLED.0 | NodeFlags::VISIBLE.0,
);
/// Tappable, before the list says whether a row is in view.
const ACTIONABLE: NodeFlags =
    NodeFlags(NodeFlags::CLICKABLE.0 | NodeFlags::FOCUSABLE.0 | NodeFlags::ENABLED.0);

const WORDS: &[&str] = &[
    "about",
    "agenda",
    "album",
    "and",
    "approved",
    "at",
    "backup",
    "board",
    "budget",
    "build",
    "call",
    "cancelled",
    "change",
    "check",
    "code",
    "comment",
    "confirmed",
    "contract",
    "dinner",
    "draft",
    "due",
    "event",
    "feedback",
    "flight",
    "for",
    "from",
    "invoice",
    "is",
    "launch",
    "list",
    "meeting",
    "minutes",
    "new",
    "notes",
    "offer",
    "on",
    "order",
    "paid",
    "photos",
    "plan",
    "please",
    "question",
    "receipt",
    "reminder",
    "renewal",
    "reply",
    "report",
    "review",
    "schedule",
    "shared",
    "shipped",
    "slides",
    "status",
    "team",
    "the",
    "ticket",
    "today",
    "trip",
    "update",
    "week",
    "with",
    "you",
    "your",
];

/// Words in 2-, 3- and 4-byte UTF-8.
const RARE_WORDS: &[&str] = &[
    "café",
    "Zürich",
    "smörgåsbord",
    "naïve",
    "Ελλάδα",
    "東京",
    "€20",
    "–",
    "👍",
    "🚀",
];

const NAMES: &[&str] = &[
    "Alice", "Bob", "Carol", "Dave", "Erin", "Frank", "Grace", "Heidi", "Ivan", "Judy", "Mallory",
    "Niaj", "Olivia", "Peggy", "Rupert", "Sybil", "Trent", "Victor", "Walter", "Zoë",
];

const MAIL_TABS: &[&str] = &["Primary", "Social", "Updates"];
const DESTINATIONS: &[&str] = &["Mail", "Chat", "Calendar", "Settings"];
const HEADERS: &[&str] = &["Pinned", "Today", "Yesterday", "This week", "Earlier"];
const ACTIONS: &[&str] = &["Reply", "Forward", "Archive", "Snooze", "Accept", "Decline"];

/// The Expo fixture's tabs: test id and label.
const EXPO_TABS: &[(&str, &str)] = &[
    ("login", "Login"),
    ("list", "List"),
    ("big", "Big"),
    ("anim", "Anim"),
    ("system", "System"),
];

/// SplitMix64, one stream per dataset.
struct Rng(u64);

impl Rng {
    fn new(stream: u64) -> Rng {
        Rng(SEED ^ (stream << 32))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        ((u128::from(self.next()) * u128::from(n)) >> 64) as u64
    }

    /// Uniform in `lo..=hi`.
    fn between(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }

    /// An index of `weights`, each drawn in proportion to its weight.
    fn weighted(&mut self, weights: &[u64]) -> usize {
        let mut draw = self.below(weights.iter().sum());
        for (i, &weight) in weights.iter().enumerate() {
            if draw < weight {
                return i;
            }
            draw -= weight;
        }
        unreachable!("the draw is below the sum of the weights")
    }

    /// Each index of `counts` as many times as its count, in a random order:
    /// every draw takes an index in proportion to what is left of its count,
    /// so the totals are exact.
    fn deal(&mut self, counts: &[u64]) -> Vec<usize> {
        let mut left = counts.to_vec();
        (0..counts.iter().sum::<u64>())
            .map(|_| {
                let i = self.weighted(&left);
                left[i] -= 1;
                i
            })
            .collect()
    }
}

/// A capitalized phrase of `min` to `max` words, a few in other scripts.
fn phrase(rng: &mut Rng, min: u64, max: u64) -> String {
    let mut out = String::new();
    for i in 0..rng.between(min, max) {
        let word = if rng.chance(4) {
            *rng.pick(RARE_WORDS)
        } else {
            *rng.pick(WORDS)
        };
        if i == 0 {
            let mut chars = word.chars();
            out.extend(chars.next().map(|c| c.to_ascii_uppercase()));
            out.push_str(chars.as_str());
        } else {
            out.push(' ');
            out.push_str(word);
        }
    }
    out
}

/// The strings a tree's nodes index, in the order they were first used.
#[derive(Default)]
struct Strings(Vec<String>);

impl Strings {
    fn intern(&mut self, s: &str) -> u32 {
        let i = match self.0.iter().position(|t| t == s) {
            Some(i) => i,
            None => {
                self.0.push(s.to_owned());
                self.0.len() - 1
            }
        };
        i as u32
    }

    /// A node with no strings but its class and no children; [`response`]
    /// gives it its id.
    fn node(&mut self, class: &str, bounds: Bounds, flags: NodeFlags) -> Node {
        Node {
            id: 0,
            class: self.intern(class),
            res_pkg: None,
            res_id: None,
            text: None,
            desc: None,
            bounds,
            flags,
            children: Vec::new(),
            scroll: None,
        }
    }

    /// Gives `node` the resource id `prefix` `name`.
    fn res_id(&mut self, node: &mut Node, prefix: &str, name: &str) {
        node.res_pkg = Some(self.intern(prefix));
        node.res_id = Some(name.to_owned());
    }
}

/// A snapshot response for request `id` holding `windows`, top-most first,
/// whose nodes index `strings`. Ids and the string table follow the Android
/// agent's walk: the windows' packages come first in the table; then a
/// breadth-first walk across the windows, starting from every window's root
/// in turn, numbers the nodes from 0 and adds each class and resource-id
/// prefix as it first meets it.
fn response(id: u32, mut windows: Vec<Window>, strings: &Strings, seq: u64) -> Message {
    let mut table = Vec::new();
    let mut moved: Vec<Option<u32>> = vec![None; strings.0.len()];
    let mut place = |i: u32| {
        *moved[i as usize].get_or_insert_with(|| {
            table.push(strings.0[i as usize].clone());
            table.len() as u32 - 1
        })
    };
    for window in &mut windows {
        window.package = window.package.map(&mut place);
        window.owner = window.owner.map(&mut place);
    }
    let mut queue: VecDeque<&mut Node> =
        windows.iter_mut().filter_map(|w| w.root.as_mut()).collect();
    let mut node_count = 0;
    while let Some(node) = queue.pop_front() {
        node.id = node_count;
        node_count += 1;
        node.class = place(node.class);
        node.res_pkg = node.res_pkg.map(&mut place);
        queue.extend(node.children.iter_mut());
    }
    Message::Response(Response {
        id,
        method: Method::SNAPSHOT,
        result: Ok(Reply::Snapshot(Snapshot {
            strings: table,
            windows,
            node_count,
            truncated: Truncation::EMPTY,
            seq,
        })),
    })
}

/// A 500-node snapshot response in spike S5's shape (see the module's
/// documentation).
pub fn snapshot_500() -> Message {
    let mut screen = Mail {
        rng: Rng::new(1),
        strings: Strings::default(),
    };
    let root = screen.decor();
    let window = Window {
        id: MAIL_WINDOW,
        kind: WindowType::APPLICATION,
        layer: 0,
        package: None,
        title: Some("Mail".to_owned()),
        bounds: Bounds::new(0, 0, WIDTH, HEIGHT),
        flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
        root: Some(root),
        emits_events: Some(true),
        owner: None,
        modal: false,
    };
    response(812, vec![window], &screen.strings, 4_117)
}

/// The list's viewport, between the tabs and the bottom bar.
const LIST_TOP: i32 = 410;
const LIST_BOTTOM: i32 = 2100;

/// The list's rows by kind: messages of 6 nodes, settings of 4, action rows
/// of 3 and headers of 1, 472 nodes that the 28 around them make 500.
const ROWS: [u64; 4] = [52, 26, 15, 11];
/// The settings' controls: switches, check boxes and labelled check boxes.
const CONTROLS: [u64; 3] = [13, 3, 10];
/// The headers: titled ones and plain dividers.
const DIVIDERS: [u64; 2] = [9, 2];

/// The mail screen of [`snapshot_500`]. The node counts and the text count
/// (250) are exact; resource ids and descriptions come with the chances of
/// each kind of view.
struct Mail {
    rng: Rng,
    strings: Strings,
}

impl Mail {
    fn node(&mut self, class: &str, bounds: Bounds, flags: NodeFlags) -> Node {
        self.strings.node(class, bounds, flags)
    }

    /// A node of a row, visible when its bounds reach into the list's viewport.
    fn row_node(&mut self, class: &str, bounds: Bounds, flags: NodeFlags) -> Node {
        let in_view = bounds.bottom > LIST_TOP && bounds.top < LIST_BOTTOM;
        let visible = if in_view {
            NodeFlags::VISIBLE
        } else {
            NodeFlags::EMPTY
        };
        self.strings.node(class, bounds, flags | visible)
    }

    fn res_id(&mut self, node: &mut Node, prefix: &str, name: &str) {
        self.strings.res_id(node, prefix, name);
    }

    /// Gives `node` a resource id `percent` times in a hundred.
    fn maybe_res_id(&mut self, node: &mut Node, prefix: &str, name: &str, percent: u64) {
        if self.rng.chance(percent) {
            self.strings.res_id(node, prefix, name);
        }
    }

    /// Gives `node` the description `desc` `percent` times in a hundred.
    fn maybe_desc(&mut self, node: &mut Node, desc: String, percent: u64) {
        if self.rng.chance(percent) {
            node.desc = Some(desc);
        }
    }

    fn phrase(&mut self, min: u64, max: u64) -> String {
        phrase(&mut self.rng, min, max)
    }

    fn decor(&mut self) -> Node {
        let screen = Bounds::new(0, 0, WIDTH, HEIGHT);
        let inset = Bounds::new(0, STATUS_BAR, WIDTH, GESTURE_BAR);
        let mut decor = self.node(FRAME_LAYOUT, screen, SHOWN);
        let mut body = self.node(LINEAR_LAYOUT, screen, SHOWN);
        let mut content = self.node(FRAME_LAYOUT, inset, SHOWN);
        self.res_id(&mut content, ANDROID_IDS, "content");
        let mut root = self.node(CONSTRAINT_LAYOUT, inset, SHOWN);
        self.res_id(&mut root, APP_IDS, "root");
        root.children = vec![
            self.toolbar(),
            self.tabs(),
            self.list(),
            self.bottom_bar(),
            self.compose(),
        ];
        content.children = vec![root];
        body.children = vec![content];
        decor.children = vec![body];
        decor
    }

    fn toolbar(&mut self) -> Node {
        let (top, bottom) = (STATUS_BAR, STATUS_BAR + 168);
        let mut bar = self.node(VIEW_GROUP, Bounds::new(0, top, WIDTH, bottom), SHOWN);
        self.res_id(&mut bar, APP_IDS, "toolbar");
        let mut drawer = self.node(IMAGE_BUTTON, Bounds::new(0, top, 168, bottom), TAPPABLE);
        drawer.desc = Some("Open navigation drawer".to_owned());
        let mut title = self.node(
            TEXT_VIEW,
            Bounds::new(200, top + 32, 760, bottom - 32),
            SHOWN,
        );
        title.text = Some("Inbox".to_owned());
        self.res_id(&mut title, APP_IDS, "toolbar_title");
        let mut search = self.node(IMAGE_BUTTON, Bounds::new(800, top, 940, bottom), TAPPABLE);
        search.desc = Some("Search mail".to_owned());
        self.res_id(&mut search, APP_IDS, "action_search");
        let mut more = self.node(IMAGE_BUTTON, Bounds::new(940, top, WIDTH, bottom), TAPPABLE);
        more.desc = Some("More options".to_owned());
        self.res_id(&mut more, APP_IDS, "action_more");
        bar.children = vec![drawer, title, search, more];
        bar
    }

    fn tabs(&mut self) -> Node {
        let (top, bottom) = (STATUS_BAR + 168, LIST_TOP);
        let mut strip = self.node(
            HORIZONTAL_SCROLL_VIEW,
            Bounds::new(0, top, WIDTH, bottom),
            SHOWN,
        );
        self.res_id(&mut strip, APP_IDS, "tabs");
        for (i, label) in MAIL_TABS.iter().enumerate() {
            let left = 32 + 340 * i as i32;
            let flags = if i == 0 {
                TAPPABLE | NodeFlags::SELECTED
            } else {
                TAPPABLE
            };
            let mut tab = self.node(TEXT_VIEW, Bounds::new(left, top, left + 320, bottom), flags);
            tab.text = Some((*label).to_owned());
            self.res_id(&mut tab, APP_IDS, "tab");
            strip.children.push(tab);
        }
        strip
    }

    /// The list, scrolled some way down: its first rows lie above its
    /// viewport, the first ones above the screen, and most of them below it.
    fn list(&mut self) -> Node {
        let flags = SHOWN | NodeFlags::FOCUSABLE | NodeFlags::SCROLLABLE;
        let mut list = self.node(
            RECYCLER_VIEW,
            Bounds::new(0, LIST_TOP, WIDTH, LIST_BOTTOM),
            flags,
        );
        self.res_id(&mut list, APP_IDS, "list");
        let mut y = LIST_TOP - 600 - self.rng.below(400) as i32;
        let rows = self.rng.deal(&ROWS);
        let mut controls = self.rng.deal(&CONTROLS).into_iter();
        let mut dividers = self.rng.deal(&DIVIDERS).into_iter();
        for kind in rows {
            let row = match kind {
                0 => self.message(&mut y),
                1 => {
                    let control = controls.next().expect("a control per setting");
                    self.setting(&mut y, control)
                }
                2 => self.actions(&mut y),
                _ => {
                    let titled = dividers.next().expect("a kind per header") == 0;
                    self.header(&mut y, titled)
                }
            };
            list.children.push(row);
        }
        let rows = &list.children;
        let first = rows.iter().position(|r| r.bounds.bottom > LIST_TOP);
        let last = rows.iter().rposition(|r| r.bounds.top < LIST_BOTTOM);
        let (first, last) = first.zip(last).expect("rows in the viewport");
        list.scroll = Some(Scroll {
            axis: ScrollAxis::VERTICAL,
            flags: ScrollFlags::BACK | ScrollFlags::FORWARD,
            range: Some(ScrollRange {
                unit: ScrollUnit::ITEMS,
                min: 0,
                max: rows.len() as i32 - 1,
            }),
            position: Some(ScrollPosition {
                first: first as i32,
                last: last as i32,
            }),
        });
        list
    }

    fn message(&mut self, y: &mut i32) -> Node {
        let top = *y;
        *y += 210;
        let class = if self.rng.chance(50) {
            CARD_VIEW
        } else {
            CONSTRAINT_LAYOUT
        };
        let mut row = self.row_node(
            class,
            Bounds::new(16, top, 1064, top + 210),
            ACTIONABLE | NodeFlags::LONG_CLICKABLE,
        );
        self.maybe_res_id(&mut row, APP_IDS, "conversation", 50);
        let name = *self.rng.pick(NAMES);
        self.maybe_desc(&mut row, format!("Message from {name}"), 10);
        let mut avatar = self.row_node(
            IMAGE_VIEW,
            Bounds::new(48, top + 45, 168, top + 165),
            NodeFlags::ENABLED,
        );
        self.maybe_res_id(&mut avatar, APP_IDS, "avatar", 30);
        self.maybe_desc(&mut avatar, format!("Photo of {name}"), 60);
        let mut column = self.row_node(
            LINEAR_LAYOUT,
            Bounds::new(200, top + 30, 880, top + 180),
            NodeFlags::ENABLED,
        );
        self.maybe_res_id(&mut column, APP_IDS, "text_column", 10);
        let mut subject = self.row_node(
            TEXT_VIEW,
            Bounds::new(200, top + 30, 880, top + 100),
            NodeFlags::ENABLED,
        );
        subject.text = Some(self.phrase(2, 5));
        self.maybe_res_id(&mut subject, APP_IDS, "subject", 80);
        let mut preview = self.row_node(
            TEXT_VIEW,
            Bounds::new(200, top + 110, 880, top + 180),
            NodeFlags::ENABLED,
        );
        preview.text = Some(self.phrase(4, 9));
        self.maybe_res_id(&mut preview, APP_IDS, "preview", 70);
        let mut time = self.row_node(
            TEXT_VIEW,
            Bounds::new(900, top + 36, 1040, top + 84),
            NodeFlags::ENABLED,
        );
        time.text = Some(format!(
            "{}:{:02}",
            7 + self.rng.below(13),
            self.rng.below(60)
        ));
        self.maybe_res_id(&mut time, APP_IDS, "time", 40);
        column.children = vec![subject, preview];
        row.children = vec![avatar, column, time];
        row
    }

    /// A setting with its control: a switch (0), a check box (1) or a
    /// labelled check box (2).
    fn setting(&mut self, y: &mut i32, control: usize) -> Node {
        let top = *y;
        *y += 168;
        let mut row = self.row_node(
            LINEAR_LAYOUT,
            Bounds::new(0, top, WIDTH, top + 168),
            ACTIONABLE,
        );
        self.maybe_res_id(&mut row, APP_IDS, "setting", 40);
        let mut title = self.row_node(
            TEXT_VIEW,
            Bounds::new(48, top + 26, 860, top + 88),
            NodeFlags::ENABLED,
        );
        title.text = Some(self.phrase(1, 3));
        self.res_id(&mut title, ANDROID_IDS, "title");
        let mut summary = self.row_node(
            TEXT_VIEW,
            Bounds::new(48, top + 92, 860, top + 142),
            NodeFlags::ENABLED,
        );
        summary.text = Some(self.phrase(3, 8));
        self.res_id(&mut summary, ANDROID_IDS, "summary");
        let class = if control == 0 { SWITCH } else { CHECK_BOX };
        let checked = if self.rng.chance(50) {
            NodeFlags::CHECKED
        } else {
            NodeFlags::EMPTY
        };
        let mut toggle = self.row_node(
            class,
            Bounds::new(900, top + 42, 1040, top + 126),
            ACTIONABLE | NodeFlags::CHECKABLE | checked,
        );
        if control == 2 {
            toggle.text = Some(self.phrase(1, 2));
        }
        self.maybe_res_id(&mut toggle, APP_IDS, "toggle", 60);
        row.children = vec![title, summary, toggle];
        row
    }

    fn actions(&mut self, y: &mut i32) -> Node {
        let top = *y;
        *y += 147;
        let mut row = self.row_node(
            LINEAR_LAYOUT,
            Bounds::new(0, top, WIDTH, top + 147),
            NodeFlags::ENABLED,
        );
        self.maybe_res_id(&mut row, APP_IDS, "actions", 20);
        let mut action = self.row_node(
            BUTTON,
            Bounds::new(32, top + 21, 540, top + 126),
            ACTIONABLE,
        );
        action.text = Some((*self.rng.pick(ACTIONS)).to_owned());
        self.maybe_res_id(&mut action, APP_IDS, "primary_action", 60);
        let mut share = self.row_node(
            IMAGE_BUTTON,
            Bounds::new(560, top + 21, 700, top + 126),
            ACTIONABLE,
        );
        share.desc = Some("Share".to_owned());
        self.maybe_res_id(&mut share, APP_IDS, "share", 50);
        row.children = vec![action, share];
        row
    }

    fn header(&mut self, y: &mut i32, titled: bool) -> Node {
        let top = *y;
        *y += 95;
        let bounds = Bounds::new(32, top, 1048, top + 95);
        let mut header = if titled {
            let mut header = self.row_node(TEXT_VIEW, bounds, NodeFlags::ENABLED);
            header.text = Some((*self.rng.pick(HEADERS)).to_owned());
            header
        } else {
            self.row_node(VIEW, bounds, NodeFlags::ENABLED)
        };
        self.maybe_res_id(&mut header, APP_IDS, "section_header", 30);
        header
    }

    fn bottom_bar(&mut self) -> Node {
        let (top, bottom) = (LIST_BOTTOM, GESTURE_BAR);
        let mut bar = self.node(LINEAR_LAYOUT, Bounds::new(0, top, WIDTH, bottom), SHOWN);
        self.res_id(&mut bar, APP_IDS, "bottom_bar");
        for (i, label) in DESTINATIONS.iter().enumerate() {
            let left = 270 * i as i32;
            let flags = if i == 0 {
                TAPPABLE | NodeFlags::SELECTED
            } else {
                TAPPABLE
            };
            let mut item = self.node(
                FRAME_LAYOUT,
                Bounds::new(left, top, left + 270, bottom),
                flags,
            );
            self.res_id(&mut item, APP_IDS, &format!("nav_{}", label.to_lowercase()));
            self.maybe_desc(&mut item, (*label).to_owned(), 50);
            let mut icon = self.node(
                IMAGE_VIEW,
                Bounds::new(left + 99, top + 16, left + 171, top + 88),
                SHOWN,
            );
            self.maybe_res_id(&mut icon, APP_IDS, "icon", 50);
            let mut text = self.node(
                TEXT_VIEW,
                Bounds::new(left + 40, top + 96, left + 230, top + 156),
                SHOWN,
            );
            text.text = Some((*label).to_owned());
            self.maybe_res_id(&mut text, APP_IDS, "label", 50);
            item.children = vec![icon, text];
            bar.children.push(item);
        }
        bar
    }

    fn compose(&mut self) -> Node {
        let mut button = self.node(IMAGE_BUTTON, Bounds::new(880, 1900, 1040, 2060), TAPPABLE);
        button.desc = Some("Compose".to_owned());
        self.res_id(&mut button, APP_IDS, "compose");
        button
    }
}

/// The 1,049-node snapshot response of the Expo fixture's Big screen (see
/// the module's documentation). It is laid out without the generator, as the
/// fixture's content is fixed.
pub fn snapshot_1049() -> Message {
    let mut strings = Strings::default();
    let status_bar = status_bar(&mut strings);
    let app = big_screen(&mut strings);
    let systemui = strings.intern(SYSTEMUI);
    let expo = strings.intern(EXPO);
    let windows = vec![
        Window {
            id: 3,
            kind: WindowType::SYSTEM,
            layer: 1,
            package: Some(systemui),
            title: None,
            bounds: Bounds::new(0, 0, WIDTH, STATUS_BAR),
            flags: WindowFlags::EMPTY,
            root: Some(status_bar),
            emits_events: Some(true),
            owner: Some(systemui),
            modal: false,
        },
        Window {
            id: 1,
            kind: WindowType::APPLICATION,
            layer: 0,
            package: Some(expo),
            title: Some("FixtureExpo".to_owned()),
            bounds: Bounds::new(0, 0, WIDTH, HEIGHT),
            flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
            root: Some(app),
            emits_events: Some(true),
            owner: None,
            modal: false,
        },
    ];
    response(1_293, windows, &strings, 268)
}

/// A SystemUI view with a resource id, or without one for `""`.
fn systemui(strings: &mut Strings, class: &str, bounds: Bounds, name: &str) -> Node {
    let mut node = strings.node(class, bounds, SHOWN);
    if !name.is_empty() {
        strings.res_id(&mut node, SYSTEMUI_IDS, name);
    }
    node
}

/// The status bar as SystemUI lays it out: the clock and two notification
/// icons at the start, the wifi and mobile signals and the battery at the
/// end, 28 nodes, the signals' icons 11 levels below the window's root. The
/// lights-out dot is the one node not visible.
fn status_bar(s: &mut Strings) -> Node {
    let bar = |left, right| Bounds::new(left, 0, right, STATUS_BAR);
    let icon = |left| Bounds::new(left, 40, left + 52, 92);
    let signal = |s: &mut Strings, left: i32, desc: &str, kind: &str| {
        let mut view = systemui(s, FRAME_LAYOUT, bar(left, left + 70), "");
        view.desc = Some(desc.to_owned());
        let mut group = systemui(s, LINEAR_LAYOUT, icon(left + 9), &format!("{kind}_group"));
        let mut combo = systemui(s, FRAME_LAYOUT, icon(left + 9), &format!("{kind}_combo"));
        let glyph = systemui(s, IMAGE_VIEW, icon(left + 9), &format!("{kind}_signal"));
        combo.children = vec![glyph];
        group.children = vec![combo];
        view.children = vec![group];
        view
    };

    let mut lights_out = systemui(s, IMAGE_VIEW, icon(42), "notification_lights_out");
    lights_out.flags = NodeFlags::ENABLED;

    let mut clock = systemui(s, TEXT_VIEW, Bounds::new(42, 33, 160, 99), "clock");
    clock.text = Some("12:30".to_owned());
    let mut notifications = Vec::new();
    for (left, desc) in [
        (192, "Android System notification: USB debugging connected"),
        (260, "FixtureExpo notification: Location in use"),
    ] {
        let mut notification = systemui(s, IMAGE_VIEW, icon(left), "");
        notification.desc = Some(desc.to_owned());
        notifications.push(notification);
    }
    let mut icons = systemui(s, VIEW_GROUP, bar(176, 486), "notificationIcons");
    icons.children = notifications;
    let mut icon_area = systemui(s, FRAME_LAYOUT, bar(176, 486), "notification_icon_area");
    icon_area.children = vec![icons];
    let mut start_line = systemui(
        s,
        LINEAR_LAYOUT,
        bar(42, 486),
        "status_bar_start_side_except_heads_up",
    );
    start_line.children = vec![clock, icon_area];
    let mut start_content = systemui(
        s,
        FRAME_LAYOUT,
        bar(42, 486),
        "status_bar_start_side_content",
    );
    start_content.children = vec![start_line];
    let mut start = systemui(
        s,
        FRAME_LAYOUT,
        bar(42, 486),
        "status_bar_start_side_container",
    );
    start.children = vec![start_content];

    let cutout = systemui(s, VIEW, bar(486, 594), "cutout_space_view");

    let mut status_icons = systemui(s, LINEAR_LAYOUT, bar(790, 930), "statusIcons");
    status_icons.children = vec![
        signal(s, 790, "Wifi signal full.", "wifi"),
        signal(s, 860, "Phone two bars.", "mobile"),
    ];
    let mut battery = systemui(s, LINEAR_LAYOUT, bar(944, 1038), "battery");
    battery.desc = Some("Battery 100 percent.".to_owned());
    battery.children = vec![systemui(s, IMAGE_VIEW, Bounds::new(944, 38, 990, 94), "")];
    let mut system_icons = systemui(s, LINEAR_LAYOUT, bar(790, 1038), "system_icons");
    system_icons.children = vec![status_icons, battery];
    let mut end_content = systemui(
        s,
        LINEAR_LAYOUT,
        bar(594, 1038),
        "status_bar_end_side_content",
    );
    end_content.children = vec![system_icons];
    let mut end = systemui(
        s,
        FRAME_LAYOUT,
        bar(594, 1038),
        "status_bar_end_side_container",
    );
    end.children = vec![end_content];

    let mut contents = systemui(s, LINEAR_LAYOUT, bar(42, 1038), "status_bar_contents");
    contents.children = vec![start, cutout, end];
    let mut status_bar = systemui(s, FRAME_LAYOUT, bar(0, WIDTH), "status_bar");
    status_bar.children = vec![lights_out, contents];
    let mut container = systemui(s, FRAME_LAYOUT, bar(0, WIDTH), "status_bar_container");
    container.children = vec![status_bar];
    let mut window_view = systemui(s, FRAME_LAYOUT, bar(0, WIDTH), "");
    window_view.children = vec![container];
    window_view
}

/// Where the Big screen's grid starts, its cells' side and its columns.
const GRID_LEFT: i32 = 40;
const GRID_TOP: i32 = 274;
const CELL: i32 = 50;
const COLUMNS: i32 = 20;

/// The Expo fixture's Big screen as React Native lays it out on Android,
/// with the layout-only views flattened: the window's decor, the React root,
/// the safe-area views, the header, the scroll view whose content view holds
/// the 500 cells, and the tab bar. Test ids are resource ids without a
/// prefix, and labels descriptions. The navigation bar's background, empty
/// when the app draws edge to edge, is the one node not visible.
fn big_screen(s: &mut Strings) -> Node {
    let screen = Bounds::new(0, 0, WIDTH, HEIGHT);
    let mut decor = s.node(FRAME_LAYOUT, screen, SHOWN);
    let mut body = s.node(LINEAR_LAYOUT, screen, SHOWN);
    let mut navigation_bar = s.node(
        VIEW,
        Bounds::new(0, HEIGHT, WIDTH, HEIGHT),
        NodeFlags::ENABLED,
    );
    s.res_id(&mut navigation_bar, ANDROID_IDS, "navigationBarBackground");
    let mut content = s.node(FRAME_LAYOUT, screen, SHOWN);
    s.res_id(&mut content, ANDROID_IDS, "content");
    let mut react_root = s.node(FRAME_LAYOUT, screen, SHOWN);
    let mut safe_area_provider = s.node(VIEW_GROUP, screen, SHOWN);
    let mut safe_area = s.node(
        VIEW_GROUP,
        Bounds::new(0, STATUS_BAR, WIDTH, GESTURE_BAR),
        SHOWN,
    );

    let mut header = s.node(TEXT_VIEW, Bounds::new(0, STATUS_BAR, WIDTH, 253), SHOWN);
    header.res_id = Some("big-count".to_owned());
    header.text = Some("500 elements".to_owned());

    let rows = 500 / COLUMNS;
    let mut grid = s.node(
        VIEW_GROUP,
        Bounds::new(0, 253, WIDTH, GRID_TOP + rows * CELL + 21),
        SHOWN,
    );
    grid.children = (0..500).map(|i| cell(s, i)).collect();
    let mut scroll = s.node(
        SCROLL_VIEW,
        Bounds::new(0, 253, WIDTH, 2127),
        SHOWN | NodeFlags::FOCUSABLE,
    );
    scroll.children = vec![grid];

    let mut tab_bar = s.node(VIEW_GROUP, Bounds::new(0, 2127, WIDTH, GESTURE_BAR), SHOWN);
    for (i, &(key, label)) in EXPO_TABS.iter().enumerate() {
        let left = 216 * i as i32;
        let flags = if key == "big" {
            TAPPABLE | NodeFlags::SELECTED
        } else {
            TAPPABLE
        };
        let mut tab = s.node(
            BUTTON,
            Bounds::new(left, 2127, left + 216, GESTURE_BAR),
            flags,
        );
        tab.res_id = Some(format!("tab-{key}"));
        tab.desc = Some(label.to_owned());
        let mut text = s.node(
            TEXT_VIEW,
            Bounds::new(left + 60, 2170, left + 156, 2231),
            SHOWN,
        );
        text.text = Some(label.to_owned());
        tab.children = vec![text];
        tab_bar.children.push(tab);
    }

    safe_area.children = vec![header, scroll, tab_bar];
    safe_area_provider.children = vec![safe_area];
    react_root.children = vec![safe_area_provider];
    content.children = vec![react_root];
    body.children = vec![content];
    decor.children = vec![body, navigation_bar];
    decor
}

/// Cell `i + 1` of the grid: a button labelled with its number, holding the
/// number as text, centred.
fn cell(s: &mut Strings, i: i32) -> Node {
    let n = i + 1;
    let left = GRID_LEFT + CELL * (i % COLUMNS);
    let top = GRID_TOP + CELL * (i / COLUMNS);
    let mut cell = s.node(
        BUTTON,
        Bounds::new(left, top, left + CELL, top + CELL),
        TAPPABLE,
    );
    cell.res_id = Some(format!("cell-{n}"));
    cell.desc = Some(format!("Cell {n}"));
    let number = n.to_string();
    let half = 5 * number.len() as i32;
    let middle = left + CELL / 2;
    let mut text = s.node(
        TEXT_VIEW,
        Bounds::new(middle - half, top + 13, middle + half, top + 37),
        SHOWN,
    );
    text.text = Some(number);
    cell.children = vec![text];
    cell
}

/// 10,000 events as S5's `E10K` split them (see the module's documentation).
pub fn events_10k() -> Vec<Message> {
    let mut rng = Rng::new(2);
    // The last number sent on the tree stream and on the window stream.
    let mut seq = [0u64; 2];
    let mut time_us = (1 << 33) + rng.below(1 << 36);
    rng.deal(&[8_975, 1_025])
        .into_iter()
        .map(|stream| {
            seq[stream] += 1;
            time_us += 20
                + if rng.chance(1) {
                    rng.below(200_000)
                } else {
                    rng.below(4_000)
                };
            let payload = if stream == 0 {
                let changes = [
                    TreeChangeKind::CONTENT,
                    TreeChangeKind::TEXT,
                    TreeChangeKind::SCROLL,
                    TreeChangeKind::FOCUS,
                    TreeChangeKind::SELECTION,
                ];
                EventPayload::Tree(TreeChange {
                    change: changes[rng.weighted(&[55, 10, 15, 10, 10])],
                    window: if rng.chance(95) {
                        MAIL_WINDOW
                    } else {
                        KEYBOARD_WINDOW
                    },
                    path: path(&mut rng),
                })
            } else {
                let changes = [
                    WindowChangeKind::STATE,
                    WindowChangeKind::ADDED,
                    WindowChangeKind::REMOVED,
                    WindowChangeKind::FOCUS,
                    WindowChangeKind::BOUNDS,
                ];
                EventPayload::Window(WindowChange {
                    change: changes[rng.weighted(&[50, 15, 15, 10, 10])],
                    window: if rng.chance(70) {
                        MAIL_WINDOW
                    } else {
                        rng.between(1, 400) as u32
                    },
                })
            };
            Message::Event(Event {
                seq: seq[stream],
                time_us,
                payload,
            })
        })
        .collect()
}

/// A path of 2 to 10 child indices from a window's root: the first levels
/// mostly a first child, the deeper ones mostly one of a few, sometimes one of
/// dozens or hundreds, as rows of a list are.
fn path(rng: &mut Rng) -> Vec<u32> {
    (0..rng.between(2, 10))
        .map(|level| {
            let index = if level < 3 {
                if rng.chance(90) { 0 } else { rng.between(1, 3) }
            } else {
                match rng.weighted(&[75, 20, 5]) {
                    0 => rng.below(8),
                    1 => rng.between(8, 63),
                    _ => rng.between(64, 400),
                }
            };
            index as u32
        })
        .collect()
}

const DEADLINES: &[u32] = &[200, 500, 1_000, 2_000, 5_000, 10_000, 30_000];

/// 1,000 requests in S5's `R1K` mix (see the module's documentation): text
/// goes to nodes of [`snapshot_500`], and snapshot requests carry each
/// parameter at random.
pub fn requests_1k() -> Vec<Message> {
    let mut rng = Rng::new(3);
    let methods = rng.deal(&[595, 198, 207]);
    let deadlines = rng.deal(&[309, 691]);
    methods
        .into_iter()
        .zip(deadlines)
        .enumerate()
        .map(|(i, (method, deadline))| {
            let call = match method {
                0 => Call::Tap(TapParams {
                    x: rng.below(WIDTH as u64) as i32,
                    y: rng.below(HEIGHT as u64) as i32,
                    duration_ms: None,
                }),
                1 => Call::TextSet(TextParams {
                    text: phrase(&mut rng, 1, 6),
                    node: Some(rng.below(500) as u32),
                    mode: match rng.below(10) {
                        0 => Some(TextMethod::DIRECT),
                        1 => Some(TextMethod::KEYS),
                        _ => None,
                    },
                }),
                _ => Call::Snapshot(SnapshotParams {
                    max_depth: rng.chance(50).then(|| *rng.pick(&[8, 16, 32, 64])),
                    max_nodes: rng
                        .chance(50)
                        .then(|| *rng.pick(&[500, 1_000, 2_000, 5_000, 10_000])),
                    idle_ms: rng.chance(50).then(|| *rng.pick(&[0, 100, 250, 500])),
                    evict_event_less: rng.chance(10).then_some(true),
                    evict: rng.chance(10).then(|| vec![MAIL_WINDOW]),
                }),
            };
            let deadline_ms = (deadline == 1).then(|| *rng.pick(DEADLINES));
            Message::Request(Request {
                id: i as u32 + 1,
                deadline_ms,
                call,
            })
        })
        .collect()
}
