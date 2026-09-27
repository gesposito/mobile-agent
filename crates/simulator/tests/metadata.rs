//! Task 6.10: the window metadata and scroll semantics the reader fills in
//! for the mirror (protocol task 3.6; simulator-driver spec, "Foreground tree
//! read"; ui-mirror spec, "Windows without change events"): which system
//! process owns a window, which window is a modal, that no window sends
//! change events, and how a scroll view scrolls. Task 6.12 adds the
//! keyboard's window, read whole, and sheets that block the screen, as modal
//! windows.

#![cfg(target_os = "macos")]

mod common;

use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

use protocol::snapshot::{
    Node, NodeFlags, ScrollAxis, ScrollFlags, ScrollUnit, Window, WindowType,
};
use simulator::{
    ForegroundTree, Input, LaunchOptions, Lifecycle, Point, SimulatorDriver, Swipe, Tree,
    TreeOptions,
};

use common::{EXPO_BUNDLE, ExpoOnLogin, SWIFTUI_BUNDLE};

/// A reference fixture and how to show one of its tabs.
#[derive(Clone, Copy, Debug)]
enum Fixture {
    SwiftUi,
    Expo,
}

impl Fixture {
    const ALL: [Fixture; 2] = [Fixture::SwiftUi, Fixture::Expo];

    fn bundle(self) -> &'static str {
        match self {
            Fixture::SwiftUi => SWIFTUI_BUNDLE,
            Fixture::Expo => EXPO_BUNDLE,
        }
    }

    /// Relaunches the fixture on `tab` and reads it until the tree holds
    /// `identifier`.
    fn show(
        self,
        driver: &SimulatorDriver,
        udid: &str,
        tab: &str,
        identifier: &str,
    ) -> ForegroundTree {
        match self {
            Fixture::SwiftUi => {
                common::show_swiftui(driver, udid, tab);
            }
            Fixture::Expo => common::show_expo(driver, udid, tab),
        }
        // Until the process a relaunch replaced is gone, it can still own
        // the foreground, so the read waits for the fixture's own tree.
        common::snapshot_until(driver, udid, Duration::from_secs(15), |tree| {
            tree.foreground.bundle_id.as_deref() == Some(self.bundle()) && has(tree, identifier)
        })
    }
}

fn has(tree: &ForegroundTree, identifier: &str) -> bool {
    tree.find(identifier).is_some()
}

fn holds(node: &Node, identifier: &str) -> bool {
    node.res_id.as_deref() == Some(identifier)
        || node.children.iter().any(|child| holds(child, identifier))
}

/// The window whose tree holds the node with `identifier`.
fn window_of<'a>(tree: &'a ForegroundTree, identifier: &str) -> Option<&'a Window> {
    tree.snapshot.windows.iter().find(|window| {
        window
            .root
            .as_ref()
            .is_some_and(|root| holds(root, identifier))
    })
}

/// The scroll view around the List tab's rows: the nearest scrollable
/// ancestor of the first row in the tree.
fn list_of(tree: &ForegroundTree) -> Option<&Node> {
    fn trail<'a>(node: &'a Node, path: &mut Vec<&'a Node>) -> bool {
        path.push(node);
        let row = node
            .res_id
            .as_deref()
            .is_some_and(|id| id.starts_with("row-"));
        if row || node.children.iter().any(|child| trail(child, path)) {
            return true;
        }
        path.pop();
        false
    }
    tree.snapshot
        .windows
        .iter()
        .filter_map(|window| window.root.as_ref())
        .find_map(|root| {
            let mut path = Vec::new();
            trail(root, &mut path).then(|| {
                path.into_iter()
                    .rev()
                    .skip(1)
                    .find(|node| node.flags.contains(NodeFlags::SCROLLABLE))
            })?
        })
}

/// The name a window's owner indexes in the read's string table.
fn owner<'a>(tree: &'a ForegroundTree, window: &Window) -> Option<&'a str> {
    window.owner.map(|index| {
        tree.snapshot
            .strings
            .get(index as usize)
            .map_or("<outside the string table>", String::as_str)
    })
}

/// Every window of a Simulator read is re-read rather than followed by
/// events: the Simulator sends none (spikes S3 and S4).
fn assert_event_less(tree: &ForegroundTree) {
    for window in &tree.snapshot.windows {
        assert_eq!(
            window.emits_events,
            Some(false),
            "window {} ({:?}) must say it sends no change events",
            window.id,
            window.kind
        );
    }
}

/// Dismisses the permission alert in `alert` with its Don’t Allow button
/// and waits for the fixture to own the foreground again. SpringBoard keeps
/// the alert up after its application is gone, over whatever runs next, so
/// this runs whether the checks passed or not. A tap from a new input
/// session did not always land on the long-running shared Simulator, where
/// a press held for 100 ms did, so the press follows when the tap misses.
fn deny(driver: &SimulatorDriver, udid: &str, alert: &ForegroundTree) {
    let deny = alert
        .alert
        .as_ref()
        .and_then(|alert| alert.buttons.iter().find(|b| b.label == "Don’t Allow"))
        .expect("the alert's Don’t Allow button")
        .center;
    // UIKit ignores touches until the alert's presentation ends.
    common::settle(driver, udid);
    for hold in [Duration::ZERO, Duration::from_millis(100)] {
        driver
            .press(udid, deny, hold)
            .expect("a press on Don’t Allow");
        let gone = common::wait_for(Duration::from_secs(5), || {
            driver
                .snapshot(udid, &TreeOptions::default())
                .is_ok_and(|tree| !tree.foreground.system)
        });
        if gone {
            return;
        }
    }
    panic!("the permission alert is still up after Don’t Allow was pressed");
}

#[test]
fn a_permission_alert_window_is_owned_by_springboard() {
    let Some(udid) = common::gated("a_permission_alert_window_is_owned_by_springboard") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    for fixture in Fixture::ALL {
        common::reset_location(&udid, fixture.bundle());
        let screen = fixture.show(&driver, &udid, "system", "system-permission");
        for window in &screen.snapshot.windows {
            assert_eq!(
                owner(&screen, window),
                None,
                "{fixture:?}: the application owns window {}",
                window.id
            );
        }
        assert_event_less(&screen);
        let request = screen.center(screen.find("system-permission").unwrap());
        driver
            .tap(&udid, request)
            .expect("a tap on the request button");

        let alert = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
            tree.foreground.system && tree.alert.is_some()
        });
        let checked = panic::catch_unwind(AssertUnwindSafe(|| {
            let owners: Vec<(u32, Option<&str>)> = alert
                .snapshot
                .windows
                .iter()
                .map(|window| (window.id, owner(&alert, window)))
                .collect();
            eprintln!(
                "{fixture:?} permission alert: {} (pid {}), window owners {owners:?}",
                alert.foreground.name, alert.foreground.pid
            );
            for (id, name) in &owners {
                assert_eq!(
                    *name,
                    Some("SpringBoard"),
                    "{fixture:?}: window {id} belongs to SpringBoard, named in the string table"
                );
            }
            assert_event_less(&alert);
        }));
        deny(&driver, &udid, &alert);
        if let Err(failure) = checked {
            common::reset_location(&udid, fixture.bundle());
            panic::resume_unwind(failure);
        }
        let back = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
            !tree.foreground.system
                && tree
                    .find("system-permission-status")
                    .and_then(|node| node.desc.as_deref())
                    == Some("Permission: denied")
        });
        for window in &back.snapshot.windows {
            assert_eq!(
                owner(&back, window),
                None,
                "{fixture:?}: window {} is the application's again",
                window.id
            );
        }
        assert_event_less(&back);
        common::reset_location(&udid, fixture.bundle());
        driver
            .terminate(&udid, fixture.bundle())
            .expect("terminated");
    }
}

#[test]
fn an_in_app_alert_window_is_modal_and_the_application_window_is_not() {
    let Some(udid) =
        common::gated("an_in_app_alert_window_is_modal_and_the_application_window_is_not")
    else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    for fixture in Fixture::ALL {
        let screen = fixture.show(&driver, &udid, "system", "system-alert");
        assert!(
            screen.snapshot.windows.iter().all(|window| !window.modal),
            "{fixture:?}: no modal before the alert"
        );
        let open = screen.center(screen.find("system-alert").unwrap());
        driver.tap(&udid, open).expect("a tap on the alert button");

        let covered = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
            tree.alert.is_some()
        });
        let windows: Vec<(u32, i32, bool, Option<&str>)> = covered
            .snapshot
            .windows
            .iter()
            .map(|window| {
                (
                    window.id,
                    window.layer,
                    window.modal,
                    window.title.as_deref(),
                )
            })
            .collect();
        eprintln!("{fixture:?} in-app alert: windows (id, layer, modal, title) {windows:?}");
        assert!(!covered.foreground.system, "the application owns its alert");
        let alert = covered.alert.as_ref().unwrap();
        assert_eq!(alert.title.as_deref(), Some("In-app alert"));
        let alert_window = covered
            .snapshot
            .windows
            .iter()
            .find(|window| window.id == alert.window)
            .expect("the alert's window");
        assert!(
            alert_window.modal,
            "{fixture:?}: the alert's window {} is a modal",
            alert_window.id
        );
        let application = window_of(&covered, "system-permission")
            .expect("the window with the covered screen, which the payload keeps");
        assert!(
            !application.modal,
            "{fixture:?}: the application's window {} is not a modal",
            application.id
        );
        for window in &covered.snapshot.windows {
            assert_eq!(
                owner(&covered, window),
                None,
                "{fixture:?}: the application owns window {}",
                window.id
            );
        }
        assert_event_less(&covered);

        // UIKit ignores touches until the alert's presentation ends.
        common::settle(&driver, &udid);
        let ok = alert
            .buttons
            .iter()
            .find(|button| button.label == "OK")
            .expect("the alert's OK button")
            .center;
        driver.tap(&udid, ok).expect("a tap on OK");
        let back = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
            tree.alert.is_none() && has(tree, "system-alert")
        });
        assert!(
            back.snapshot.windows.iter().all(|window| !window.modal),
            "{fixture:?}: no modal once the alert is dismissed"
        );
        assert_event_less(&back);
        driver
            .terminate(&udid, fixture.bundle())
            .expect("terminated");
    }
}

#[test]
fn a_list_scroll_view_reports_its_axis_and_whether_it_is_at_its_top() {
    let Some(udid) =
        common::gated("a_list_scroll_view_reports_its_axis_and_whether_it_is_at_its_top")
    else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    for fixture in Fixture::ALL {
        let top = fixture.show(&driver, &udid, "list", "row-1");
        assert_event_less(&top);
        let list = list_of(&top).expect("the rows sit in a scroll view");
        let class = top.class_of(list).to_owned();
        eprintln!("{fixture:?} list at its top: {class} {:?}", list.scroll);
        let scroll = list.scroll.unwrap_or_else(|| {
            panic!("{fixture:?}: the list's scroll view, {class}, carries no scroll semantics")
        });
        assert_eq!(scroll.axis, ScrollAxis::VERTICAL, "{fixture:?}");
        assert_eq!(
            scroll.flags,
            ScrollFlags::FORWARD,
            "{fixture:?}: at its top the list scrolls forward only"
        );
        let range = scroll
            .range
            .expect("the runtime reports where the viewport is");
        assert_eq!(
            (range.unit, range.min, range.max),
            (ScrollUnit::PERCENT, 0, 100),
            "{fixture:?}"
        );
        let position = scroll.position.expect("the viewport's part of the range");
        assert_eq!(
            position.first, 0,
            "{fixture:?}: the viewport shows the start"
        );
        assert!(
            position.last > 0 && position.last < 100,
            "{fixture:?}: the viewport shows part of the rows: {position:?}"
        );

        // A slow drag up of 300 points scrolls the list down, away from its
        // top. The first rows can leave the tree, so the list is found
        // again by whichever rows it holds.
        common::settle(&driver, &udid);
        let drag = Swipe::new(
            Point::new(201.0, 650.0),
            Point::new(201.0, 350.0),
            Duration::from_millis(1000),
        );
        driver.swipe(&udid, &drag).expect("a drag up the list");
        common::settle(&driver, &udid);
        let scrolled = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
            list_of(tree)
                .and_then(|list| list.scroll)
                .is_some_and(|scroll| scroll.flags.contains(ScrollFlags::BACK))
        });
        let scroll = list_of(&scrolled).and_then(|list| list.scroll).unwrap();
        eprintln!("{fixture:?} list scrolled: {scroll:?}");
        assert_eq!(scroll.axis, ScrollAxis::VERTICAL, "{fixture:?}");
        assert_eq!(
            scroll.flags,
            ScrollFlags::BACK | ScrollFlags::FORWARD,
            "{fixture:?}: away from its top and its end, the list scrolls both ways"
        );
        let position = scroll.position.expect("the viewport's part of the range");
        assert!(
            0 < position.first && position.first < position.last && position.last < 100,
            "{fixture:?}: the viewport shows neither end: {position:?}"
        );
        driver
            .terminate(&udid, fixture.bundle())
            .expect("terminated");
    }
}

/// Contacts, which every Simulator has: its New Contact sheet blocks the
/// list it is presented over, and focuses its first field.
const CONTACTS: &str = "com.apple.MobileAddressBook";

/// Whether a node of `window` has this label.
fn labelled(window: &Window, label: &str) -> bool {
    fn walk(node: &Node, label: &str) -> bool {
        node.desc.as_deref() == Some(label) || node.children.iter().any(|child| walk(child, label))
    }
    window.root.as_ref().is_some_and(|root| walk(root, label))
}

/// The labels of the keyboard's keys in `window`.
fn keys(tree: &ForegroundTree, window: &Window) -> Vec<String> {
    fn walk(tree: &ForegroundTree, node: &Node, out: &mut Vec<String>) {
        if tree.class_of(node) == "XCUIElementTypeKey"
            && let Some(label) = &node.desc
        {
            out.push(label.clone());
        }
        for child in &node.children {
            walk(tree, child, out);
        }
    }
    let mut out = Vec::new();
    if let Some(root) = &window.root {
        walk(tree, root, &mut out);
    }
    out
}

fn keyboard_of(tree: &ForegroundTree) -> Option<&Window> {
    tree.snapshot
        .windows
        .iter()
        .find(|window| window.kind == WindowType::INPUT_METHOD)
}

#[test]
fn a_read_with_the_keyboard_up_is_whole_and_holds_the_keys() {
    let Some(udid) = common::gated("a_read_with_the_keyboard_up_is_whole_and_holds_the_keys")
    else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    for fixture in Fixture::ALL {
        let screen = fixture.show(&driver, &udid, "login", "login-username");
        let field = screen.center(screen.find("login-username").unwrap());
        // The first tap after a launch can come before the field takes
        // input, so it is repeated until the keyboard is up.
        let mut up = None;
        for _ in 0..3 {
            driver
                .tap(&udid, field)
                .expect("a tap on the username field");
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while up.is_none() && std::time::Instant::now() < deadline {
                if let Ok(tree) = driver.snapshot(&udid, &TreeOptions::default())
                    && keyboard_of(&tree).is_some_and(|window| keys(&tree, window).len() >= 26)
                {
                    up = Some(tree);
                }
            }
            if up.is_some() {
                break;
            }
            common::settle(&driver, &udid);
        }
        let tree = up.unwrap_or_else(|| panic!("{fixture:?}: the keyboard never came up"));
        let keyboard = keyboard_of(&tree).expect("the keyboard's window");
        let labels = keys(&tree, keyboard);
        eprintln!(
            "{fixture:?}: {} nodes, truncated {:?}, {} keys in the input method window",
            tree.snapshot.node_count,
            tree.snapshot.truncated,
            labels.len()
        );
        assert!(
            tree.snapshot.truncated.is_empty(),
            "{fixture:?}: a read with the keyboard up was cut: {:?}",
            tree.snapshot.truncated
        );
        assert!(!tree.is_degraded(), "{fixture:?}: {:?}", tree.degraded);
        assert!(
            labels.iter().any(|label| label.eq_ignore_ascii_case("q")),
            "{fixture:?}: {labels:?}"
        );
        assert!(
            tree.find("login-username")
                .unwrap()
                .flags
                .contains(NodeFlags::FOCUSED),
            "{fixture:?}: the field with the keyboard's focus says so"
        );
        driver
            .terminate(&udid, fixture.bundle())
            .expect("terminated");
    }
}

/// Terminates an application when dropped, so a failing test leaves no
/// sheet up in it.
struct Terminate<'a> {
    driver: &'a SimulatorDriver,
    udid: &'a str,
    bundle: &'a str,
}

impl Drop for Terminate<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.driver.terminate(self.udid, self.bundle) {
            eprintln!("could not terminate {}: {error}", self.bundle);
        }
    }
}

#[test]
fn a_sheet_over_the_screen_is_a_modal_window() {
    let Some(udid) = common::gated("a_sheet_over_the_screen_is_a_modal_window") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    driver
        .launch(&udid, CONTACTS, &LaunchOptions::new().terminating_running())
        .expect("Contacts launches");
    let _contacts = Terminate {
        driver: &driver,
        udid: &udid,
        bundle: CONTACTS,
    };
    let list = common::snapshot_until(&driver, &udid, Duration::from_secs(15), |tree| {
        tree.foreground.bundle_id.as_deref() == Some(CONTACTS)
            && tree.find_label("Add").is_some()
            && tree.find_label("John Appleseed").is_some()
    });
    let add = list.center(list.find_label("Add").unwrap());
    driver.tap(&udid, add).expect("a tap on Add");

    let covered = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        tree.find_label("Sheet Grabber").is_some()
    });
    let windows: Vec<(u32, bool, Option<&str>)> = covered
        .snapshot
        .windows
        .iter()
        .map(|window| (window.id, window.modal, window.title.as_deref()))
        .collect();
    eprintln!(
        "New Contact: windows {windows:?}, truncated {:?}",
        covered.snapshot.truncated
    );
    let sheet = covered
        .snapshot
        .windows
        .iter()
        .find(|window| window.modal)
        .expect("the sheet's modal window");
    assert_eq!(sheet.title.as_deref(), Some("New Contact"));
    assert!(labelled(sheet, "Sheet Grabber"));
    assert!(!labelled(sheet, "John Appleseed"));
    let application = covered
        .snapshot
        .windows
        .iter()
        .find(|window| window.id == covered.foreground.pid as u32)
        .expect("the application's window");
    assert!(!application.modal);
    assert!(
        labelled(application, "John Appleseed"),
        "the application's window keeps what the sheet covers, for the mirror"
    );
    assert_eq!(covered.foreground.bundle_id.as_deref(), Some(CONTACTS));
    assert!(
        covered.snapshot.truncated.is_empty(),
        "{:?}",
        covered.snapshot.truncated
    );
    assert_event_less(&covered);
    let uncovered = covered.uncovered();
    assert!(
        uncovered
            .windows
            .iter()
            .any(|window| labelled(window, "Sheet Grabber"))
    );
    assert!(
        !uncovered
            .windows
            .iter()
            .any(|window| labelled(window, "John Appleseed")),
        "what the sheet covers is cut"
    );

    // Closing the sheet hands the screen back. UIKit ignores touches until
    // the sheet's presentation ends.
    common::settle(&driver, &udid);
    let close = covered.center(
        covered
            .find_label("close")
            .expect("the sheet's close button"),
    );
    driver.tap(&udid, close).expect("a tap on close");
    common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        tree.find_label("Sheet Grabber").is_none()
            && !tree.snapshot.windows.iter().any(|window| window.modal)
    });
}
