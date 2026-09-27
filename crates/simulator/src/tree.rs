//! What a tree read returns (simulator-driver spec, "Foreground tree read"):
//! the protocol's snapshot payload of the process that owns the foreground,
//! who that is, the alert in front when there is one, and why the read is
//! degraded when it is.
//!
//! The payload follows the conventions the mirror reads (`mirror::backend`):
//! bounds in physical pixels, `XCUIElementType` class names or the platform
//! class of untyped elements, labels as descriptions and values as text; see
//! `simulator_reader::elements`. It keeps what an alert covers, because the
//! mirror cuts that itself and still reports refs to covered elements as
//! covered; [`ForegroundTree::uncovered`] cuts it for everyone else.

use std::fmt;
use std::time::Duration;

use protocol::snapshot::{Node, Snapshot, Window};

use crate::model::Point;

/// The classes of the containers UIKit draws an alert controller in. The
/// reader reports them as untyped groups (spike S4), so the class names them.
pub const ALERT_CLASSES: [&str; 4] = [
    "_UIAlertControllerPhoneTVMacView",
    "_UIAlertControllerView",
    "UIAlertControllerView",
    "XCUIElementTypeAlert",
];

/// Which path reads a target's tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TreeBackend {
    /// The reader spawned into the Simulator (design D6).
    Reader,
    /// The host translator, `AXPTranslator`: the fallback, whose reads are
    /// degraded.
    Translator,
}

impl TreeBackend {
    /// The backend's stable name, as the mirror's verdicts print it.
    pub fn name(self) -> &'static str {
        match self {
            TreeBackend::Reader => "simulator-reader",
            TreeBackend::Translator => "simulator-translator",
        }
    }

    /// Whether this is the fallback with known gaps.
    pub fn fallback(self) -> bool {
        self == TreeBackend::Translator
    }
}

impl fmt::Display for TreeBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How the driver picks a target's tree backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TreeChoice {
    /// The reader, and the translator when the reader cannot run.
    #[default]
    Auto,
    /// Only this backend, for tests and diagnosis.
    Only(TreeBackend),
}

/// Bounds of one read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeOptions {
    /// Deepest level to read below the application element.
    pub max_depth: u32,
    /// Most nodes to read.
    pub max_nodes: u32,
    /// How long the read may take.
    pub timeout: Duration,
}

impl Default for TreeOptions {
    fn default() -> TreeOptions {
        TreeOptions {
            max_depth: 64,
            max_nodes: 10_000,
            timeout: Duration::from_secs(10),
        }
    }
}

/// The process that owns the foreground, whose tree the read holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreground {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub name: String,
    /// System UI, such as SpringBoard showing a permission alert or the home
    /// screen, rather than an application.
    pub system: bool,
}

/// An alert in front, with what it says and the buttons it offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    /// The window and node of its container in the payload.
    pub window: u32,
    pub node: u32,
    pub title: Option<String>,
    pub message: Option<String>,
    pub buttons: Vec<AlertButton>,
}

/// One button of an alert.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertButton {
    pub label: String,
    pub identifier: Option<String>,
    pub node: u32,
    /// Where to tap it, in points.
    pub center: Point,
}

/// Why a read is degraded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Degradation {
    /// The host translator read it: it flattens containers, drops the
    /// children of UIKit's tab bar and the identifiers of React Native text
    /// (spike S4). `reason` says why the reader was not used.
    Fallback { reason: String },
    /// An application window came back as its root alone, as a read right
    /// after a launch or a failed read does.
    Sparse { window: u32 },
    /// A bound cut the read short.
    Truncated { depth: bool, nodes: bool },
}

impl Degradation {
    /// A snake_case name, the same the mirror's verdicts use.
    pub fn code(&self) -> &'static str {
        match self {
            Degradation::Fallback { .. } => "fallback_backend",
            Degradation::Sparse { .. } => "sparse_tree",
            Degradation::Truncated { .. } => "truncated",
        }
    }
}

impl fmt::Display for Degradation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Degradation::Fallback { reason } => write!(
                f,
                "read by the host translator, which flattens containers and misses elements, because {reason}"
            ),
            Degradation::Sparse { window } => write!(
                f,
                "window {window} holds only its application element although an application owns it; retry in a moment"
            ),
            Degradation::Truncated { depth, nodes } => {
                let limits: Vec<&str> = [(*depth, "depth"), (*nodes, "node count")]
                    .into_iter()
                    .filter_map(|(hit, name)| hit.then_some(name))
                    .collect();
                write!(f, "the read stopped at its {} limit", limits.join(" and "))
            }
        }
    }
}

/// One read of the foreground's tree.
#[derive(Clone, Debug, PartialEq)]
pub struct ForegroundTree {
    /// Windows and nodes, bounds in physical pixels, covered content kept.
    pub snapshot: Snapshot,
    /// Physical pixels per point.
    pub scale: f64,
    pub foreground: Foreground,
    pub backend: TreeBackend,
    /// Empty when the read can be trusted.
    pub degraded: Vec<Degradation>,
    /// The alert in front, in the foreground's own tree.
    pub alert: Option<Alert>,
    /// How long the read took, as the driver saw it.
    pub elapsed: Duration,
}

impl ForegroundTree {
    /// The result of a read. `fallback` says why the reader was not used
    /// when the translator read it. `None` when the payload names no window
    /// to own the foreground.
    pub fn new(
        snapshot: Snapshot,
        scale: f64,
        backend: TreeBackend,
        fallback: Option<String>,
        elapsed: Duration,
    ) -> Option<ForegroundTree> {
        let _ = (snapshot, scale, backend, fallback, elapsed);
        unimplemented!("task 6.6: foreground, alert and degradations")
    }

    pub fn is_degraded(&self) -> bool {
        !self.degraded.is_empty()
    }

    /// Every node with its window, in pre-order, top-most window first.
    pub fn nodes(&self) -> Vec<(&Window, &Node)> {
        let mut out = Vec::new();
        for window in &self.snapshot.windows {
            if let Some(root) = &window.root {
                walk(root, &mut |node| out.push((window, node)));
            }
        }
        out
    }

    /// The first node with this accessibility identifier.
    pub fn find(&self, identifier: &str) -> Option<&Node> {
        self.nodes()
            .into_iter()
            .map(|(_, node)| node)
            .find(|node| node.res_id.as_deref() == Some(identifier))
    }

    /// The first node with this label.
    pub fn find_label(&self, label: &str) -> Option<&Node> {
        self.nodes()
            .into_iter()
            .map(|(_, node)| node)
            .find(|node| node.desc.as_deref() == Some(label))
    }

    /// The class name of a node of this read.
    pub fn class_of(&self, node: &Node) -> &str {
        self.snapshot
            .strings
            .get(node.class as usize)
            .map_or("", String::as_str)
    }

    /// A node's frame in points: `[x, y, width, height]`.
    pub fn frame(&self, node: &Node) -> [f64; 4] {
        points(&node.bounds, self.scale)
    }

    /// The middle of a node, in points, where a tap on it lands.
    pub fn center(&self, node: &Node) -> Point {
        center(node, self.scale)
    }

    /// The payload without what the alert in front covers: in the alert's
    /// window only the alert and the containers it sits in, and no window
    /// below it. Without an alert, the payload as read.
    pub fn uncovered(&self) -> Snapshot {
        unimplemented!("task 6.6: covered-content cutting")
    }
}

fn walk<'a>(node: &'a Node, visit: &mut impl FnMut(&'a Node)) {
    visit(node);
    for child in &node.children {
        walk(child, visit);
    }
}

fn points(bounds: &protocol::snapshot::Bounds, scale: f64) -> [f64; 4] {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    [
        f64::from(bounds.left) / scale,
        f64::from(bounds.top) / scale,
        f64::from(bounds.right - bounds.left) / scale,
        f64::from(bounds.bottom - bounds.top) / scale,
    ]
}

fn center(node: &Node, scale: f64) -> Point {
    let [x, y, width, height] = points(&node.bounds, scale);
    Point::new(x + width / 2.0, y + height / 2.0)
}

#[cfg(test)]
mod tests {
    use protocol::snapshot::Truncation;
    use simulator_reader::elements::{self, Element, Limits, Owner, Read};

    use super::*;

    fn element(automation_type: u64, class: &str, label: Option<&str>, frame: [f64; 4]) -> Element {
        Element {
            automation_type: Some(automation_type),
            class: Some(class.into()),
            label: label.map(str::to_owned),
            frame: Some(frame),
            ..Element::default()
        }
    }

    fn parent(mut element: Element, children: Vec<Element>) -> Element {
        element.children = children;
        element
    }

    fn identified(mut element: Element, identifier: &str) -> Element {
        element.identifier = Some(identifier.into());
        element
    }

    const SCREEN: [f64; 4] = [0.0, 0.0, 402.0, 874.0];

    /// The SwiftUI fixture's System screen under its in-app alert, as the
    /// reader returned it on iOS 26.5, containers trimmed: the covered
    /// screen and tab bar, then the alert with its title, message and OK
    /// button, nested twice as UIKit draws it.
    fn in_app_alert() -> Element {
        let screen = parent(
            element(0, "SwiftUI.AccessibilityNode", None, SCREEN),
            vec![
                identified(
                    element(
                        9,
                        "SwiftUI.AccessibilityNode",
                        Some("Request location permission"),
                        [16.0, 78.0, 238.67, 34.33],
                    ),
                    "system-permission",
                ),
                identified(
                    element(
                        9,
                        "_UITabButton",
                        Some("System"),
                        [300.0, 795.0, 77.0, 54.0],
                    ),
                    "tab-system",
                ),
            ],
        );
        let ok = identified(
            element(
                9,
                "_UIInterfaceActionCustomViewRepresentationView",
                Some("OK"),
                [57.0, 463.0, 288.0, 48.0],
            ),
            "inapp-alert-ok",
        );
        let nested_ok = identified(
            element(
                9,
                "_UIAlertControllerActionView",
                Some("OK"),
                [57.0, 463.0, 288.0, 48.0],
            ),
            "inapp-alert-ok",
        );
        let alert = parent(
            element(
                3,
                "_UIAlertControllerPhoneTVMacView",
                Some("In-app alert"),
                [41.0, 375.0, 320.0, 152.0],
            ),
            vec![
                parent(
                    element(
                        46,
                        "_UIInterfaceActionGroupHeaderScrollView",
                        None,
                        [41.0, 375.0, 320.0, 72.0],
                    ),
                    vec![
                        element(
                            48,
                            "UILabel",
                            Some("In-app alert"),
                            [71.0, 397.0, 260.0, 20.33],
                        ),
                        element(
                            48,
                            "UILabel",
                            Some("This alert is presented by the app."),
                            [71.0, 424.67, 260.0, 18.0],
                        ),
                    ],
                ),
                parent(
                    element(
                        46,
                        "_UIInterfaceActionRepresentationsSequenceView",
                        None,
                        [41.0, 447.0, 320.0, 80.0],
                    ),
                    vec![parent(ok, vec![nested_ok])],
                ),
            ],
        );
        let window = parent(
            element(2, "UIWindow", None, SCREEN),
            vec![
                parent(element(0, "UITransitionView", None, SCREEN), vec![screen]),
                parent(element(0, "UITransitionView", None, SCREEN), vec![alert]),
            ],
        );
        parent(
            element(1, "SwiftUIApplication", Some("FixtureSwiftUI"), SCREEN),
            vec![window],
        )
    }

    fn payload(root: Element, owner: Owner) -> Snapshot {
        elements::snapshot(&Read {
            owner,
            root,
            scale: 3.0,
            limits: Limits::default(),
            depth_cut: false,
        })
    }

    fn fixture_owner() -> Owner {
        Owner {
            pid: 4242,
            bundle_id: Some("dev.mobileagent.fixture.swiftui".into()),
            name: "FixtureSwiftUI".into(),
            system: false,
        }
    }

    fn tree(snapshot: Snapshot, backend: TreeBackend) -> ForegroundTree {
        ForegroundTree::new(snapshot, 3.0, backend, None, Duration::from_millis(20))
            .expect("a window owns the foreground")
    }

    #[test]
    fn the_foreground_is_the_window_that_owns_the_read() {
        let read = tree(
            payload(in_app_alert(), fixture_owner()),
            TreeBackend::Reader,
        );
        assert_eq!(
            read.foreground,
            Foreground {
                pid: 4242,
                bundle_id: Some("dev.mobileagent.fixture.swiftui".into()),
                name: "FixtureSwiftUI".into(),
                system: false,
            }
        );
        assert!(!read.is_degraded(), "{:?}", read.degraded);
    }

    #[test]
    fn an_alert_is_found_with_its_title_message_and_buttons_once_each() {
        let read = tree(
            payload(in_app_alert(), fixture_owner()),
            TreeBackend::Reader,
        );
        let alert = read.alert.as_ref().expect("the alert");
        assert_eq!(alert.window, 4242);
        assert_eq!(alert.title.as_deref(), Some("In-app alert"));
        assert_eq!(
            alert.message.as_deref(),
            Some("This alert is presented by the app.")
        );
        assert_eq!(alert.buttons.len(), 1, "{:?}", alert.buttons);
        let ok = &alert.buttons[0];
        assert_eq!(ok.label, "OK");
        assert_eq!(ok.identifier.as_deref(), Some("inapp-alert-ok"));
        assert!((ok.center.x - 201.0).abs() < 0.5 && (ok.center.y - 487.0).abs() < 0.5);
    }

    #[test]
    fn what_the_alert_covers_is_cut_and_the_payload_keeps_it() {
        let read = tree(
            payload(in_app_alert(), fixture_owner()),
            TreeBackend::Reader,
        );
        assert!(
            read.find("system-permission").is_some(),
            "the payload keeps it"
        );
        let uncovered = read.uncovered();
        let cut = ForegroundTree {
            snapshot: uncovered.clone(),
            ..read.clone()
        };
        assert!(cut.find("system-permission").is_none());
        assert!(cut.find("tab-system").is_none());
        assert!(cut.find("inapp-alert-ok").is_some());
        assert!(
            cut.find_label("This alert is presented by the app.")
                .is_some()
        );
        assert_eq!(uncovered.node_count as usize, cut.nodes().len());
        assert!(uncovered.node_count < read.snapshot.node_count);
    }

    #[test]
    fn without_an_alert_nothing_is_cut() {
        let mut root = in_app_alert();
        root.children[0].children.pop();
        let read = tree(payload(root, fixture_owner()), TreeBackend::Reader);
        assert_eq!(read.alert, None);
        assert_eq!(read.uncovered(), read.snapshot);
    }

    #[test]
    fn a_system_alert_belongs_to_system_ui() {
        let alert = parent(
            element(
                3,
                "_UIAlertControllerPhoneTVMacView",
                Some("Allow “FixtureSwiftUI” to use your location?"),
                [41.0, 288.0, 320.0, 326.0],
            ),
            ["Allow Once", "Allow While Using App", "Don’t Allow"]
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    element(
                        9,
                        "_UIInterfaceActionCustomViewRepresentationView",
                        Some(label),
                        [57.0, 438.0 + 56.0 * i as f64, 288.0, 48.0],
                    )
                })
                .collect(),
        );
        let springboard = parent(element(1, "SpringBoard", Some(" "), SCREEN), vec![alert]);
        let owner = Owner {
            pid: 22789,
            bundle_id: Some("com.apple.springboard".into()),
            name: "SpringBoard".into(),
            system: true,
        };
        let read = tree(payload(springboard, owner), TreeBackend::Reader);
        assert!(read.foreground.system);
        assert_eq!(
            read.foreground.bundle_id.as_deref(),
            Some("com.apple.springboard")
        );
        let labels: Vec<&str> = read
            .alert
            .as_ref()
            .expect("the permission alert")
            .buttons
            .iter()
            .map(|button| button.label.as_str())
            .collect();
        assert_eq!(
            labels,
            ["Allow Once", "Allow While Using App", "Don’t Allow"]
        );
        assert!(
            read.degraded.is_empty(),
            "system UI is not a sparse application"
        );
    }

    #[test]
    fn an_application_root_alone_is_sparse() {
        let root = element(1, "SwiftUIApplication", Some("FixtureSwiftUI"), SCREEN);
        let read = tree(payload(root, fixture_owner()), TreeBackend::Reader);
        assert_eq!(read.degraded, [Degradation::Sparse { window: 4242 }]);
        assert!(
            read.degraded[0]
                .to_string()
                .contains("only its application element")
        );
        assert_eq!(read.degraded[0].code(), "sparse_tree");
    }

    #[test]
    fn the_translator_and_truncation_degrade_the_read_with_reasons() {
        let mut snapshot = payload(in_app_alert(), fixture_owner());
        snapshot.truncated = Truncation::NODES;
        let read = ForegroundTree::new(
            snapshot,
            3.0,
            TreeBackend::Translator,
            Some("the reader exited with status 3".into()),
            Duration::ZERO,
        )
        .unwrap();
        let codes: Vec<&str> = read.degraded.iter().map(Degradation::code).collect();
        assert_eq!(codes, ["fallback_backend", "truncated"]);
        assert!(read.degraded[0].to_string().contains("status 3"));
        assert_eq!(
            read.degraded[1].to_string(),
            "the read stopped at its node count limit"
        );
    }

    #[test]
    fn frames_and_centers_are_in_points() {
        let read = tree(
            payload(in_app_alert(), fixture_owner()),
            TreeBackend::Reader,
        );
        let permission = read.find("system-permission").unwrap();
        let [x, y, width, height] = read.frame(permission);
        assert!((x - 16.0).abs() < 0.34 && (y - 78.0).abs() < 0.34);
        assert!((width - 238.67).abs() < 0.34 && (height - 34.33).abs() < 0.34);
        let center = read.center(permission);
        assert!((center.x - 135.33).abs() < 0.5 && (center.y - 95.17).abs() < 0.5);
        assert_eq!(read.class_of(permission), "XCUIElementTypeButton");
    }

    #[test]
    fn backends_have_the_mirrors_names() {
        assert_eq!(TreeBackend::Reader.to_string(), "simulator-reader");
        assert_eq!(TreeBackend::Translator.name(), "simulator-translator");
        assert!(TreeBackend::Translator.fallback());
        assert!(!TreeBackend::Reader.fallback());
    }
}
