//! Task 6.10: the window metadata and scroll semantics the reader fills in
//! for the mirror (protocol task 3.6; simulator-driver spec, "Foreground tree
//! read"; ui-mirror spec, "Windows without change events"): which system
//! process owns a window, which window is a modal, that no window sends
//! change events, and how a scroll view scrolls.

#![cfg(target_os = "macos")]

mod common;

use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

use protocol::snapshot::{Node, Window};
use simulator::{ForegroundTree, Input, Lifecycle, SimulatorDriver, Tree, TreeOptions};

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
