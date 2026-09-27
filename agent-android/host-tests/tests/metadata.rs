//! Task 3.6: window metadata and scroll semantics in snapshots. The agent
//! reads each window's metadata from `AccessibilityWindowInfo` and its
//! content (whether it hosts Compose, which sends no change events; who owns
//! it; whether it is a modal dialog) and each scrolling node's semantics from
//! its scroll actions, collection info and range info.

use std::time::Duration;

use agent_android_host_tests::client::Client;
use agent_android_host_tests::tree::{center, find, nodes};
use agent_android_host_tests::{Agent, Device, Launch, device, fixture, poll};
use protocol::message::{Call, Reply, SnapshotParams, SwipeParams, TapParams};
use protocol::snapshot::{
    Node, NodeFlags, Scroll, ScrollAxis, ScrollFlags, ScrollPosition, ScrollRange, ScrollUnit,
    Snapshot, Window, WindowType,
};

fn snapshot(client: &mut Client) -> Snapshot {
    match client.call(Call::Snapshot(SnapshotParams::default())) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("snapshot failed: {other:?}"),
    }
}

fn tap(client: &mut Client, (x, y): (i32, i32)) {
    let reply = client.call(Call::Tap(TapParams {
        x,
        y,
        duration_ms: None,
    }));
    assert!(matches!(reply, Ok(Reply::Input(_))), "tap: {reply:?}");
}

/// The name of the system process that owns `w`, if any.
fn owner<'a>(s: &'a Snapshot, w: &Window) -> Option<&'a str> {
    w.owner
        .and_then(|i| s.strings.get(i as usize))
        .map(String::as_str)
}

/// The window holding the node whose resource id is `id`.
fn window_of<'a>(s: &'a Snapshot, id: &str) -> Option<&'a Window> {
    find(s, id).map(|(f, _)| f.window)
}

/// The nearest ancestor of the node `id` that carries scroll semantics.
fn scroller_above<'a>(s: &'a Snapshot, id: &str) -> Option<&'a Node> {
    let (found, path) = find(s, id)?;
    let mut ancestors = vec![found.window.root.as_ref()?];
    for &i in &path[..path.len().saturating_sub(1)] {
        let next = ancestors.last()?.children.get(i as usize)?;
        ancestors.push(next);
    }
    ancestors.into_iter().rev().find(|n| n.scroll.is_some())
}

/// Rows of the list tab in view.
fn visible_rows(s: &Snapshot) -> i32 {
    nodes(s)
        .iter()
        .filter(|(f, _)| {
            f.node
                .res_id
                .as_deref()
                .is_some_and(|r| r.starts_with("row-"))
                && f.node.flags.contains(NodeFlags::VISIBLE)
        })
        .count() as i32
}

/// The scroll semantics of the list of 200 rows, found by its range, since
/// Compose leaves the rows scrolled out of view out of the tree.
fn list_scroll(s: &Snapshot) -> Option<Scroll> {
    nodes(s)
        .into_iter()
        .find_map(|(f, _)| f.node.scroll.filter(|sc| sc.range == rows()))
}

fn rows() -> Option<ScrollRange> {
    Some(ScrollRange {
        unit: ScrollUnit::ITEMS,
        min: 0,
        max: 199,
    })
}

#[test]
fn a_compose_list_reports_its_item_range_and_rows_in_view_in_a_window_without_change_events() {
    let Some(dev) = device(
        "a_compose_list_reports_its_item_range_and_rows_in_view_in_a_window_without_change_events",
    ) else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "list");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);

    let app = window_of(&s, "row-1").expect("the list's window");
    assert_eq!(
        app.emits_events,
        Some(false),
        "Compose sends no change events"
    );
    assert_eq!(owner(&s, app), None, "the fixture owns its window");
    assert!(!app.modal);
    let bar = s
        .windows
        .iter()
        .find(|w| w.kind == WindowType::SYSTEM)
        .expect("the status bar");
    assert_eq!(owner(&s, bar), Some("com.android.systemui"));
    assert_eq!(bar.emits_events, Some(true));
    assert!(!bar.modal);

    // At the top of the list: 200 rows, the first ones in view, nothing above.
    let list = scroller_above(&s, "row-1").expect("no ancestor of row-1 scrolls");
    let in_view = visible_rows(&s);
    assert!(in_view >= 10, "{in_view} rows in view");
    assert_eq!(
        list.scroll,
        Some(Scroll {
            axis: ScrollAxis::VERTICAL,
            flags: ScrollFlags::FORWARD,
            range: rows(),
            position: Some(ScrollPosition {
                first: 0,
                last: in_view - 1,
            }),
        })
    );

    // Mid-list: it scrolls both ways, and Compose names no row index.
    let b = list.bounds;
    let x = (b.left + b.right) / 2;
    let reply = client.call(Call::Swipe(SwipeParams {
        x1: x,
        y1: b.bottom - 300,
        x2: x,
        y2: b.top + 300,
        duration_ms: 300,
    }));
    assert!(matches!(reply, Ok(Reply::Input(_))), "swipe: {reply:?}");
    let mid = poll(Duration::from_secs(5), Duration::from_millis(200), || {
        let scroll = list_scroll(&snapshot(&mut client))?;
        scroll.flags.contains(ScrollFlags::BACK).then_some(scroll)
    })
    .expect("the list never reported that it can scroll back");
    fixture::force_stop(&dev, fixture::COMPOSE);
    assert_eq!(
        mid,
        Scroll {
            axis: ScrollAxis::VERTICAL,
            flags: ScrollFlags::BACK | ScrollFlags::FORWARD,
            range: rows(),
            position: None,
        }
    );
}

#[test]
fn an_expo_list_is_a_vertical_scroll_view_in_a_window_that_sends_change_events() {
    let Some(dev) =
        device("an_expo_list_is_a_vertical_scroll_view_in_a_window_that_sends_change_events")
    else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "list");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    fixture::force_stop(&dev, fixture::EXPO);

    let app = window_of(&s, "row-1").expect("the list's window");
    assert_eq!(
        app.emits_events,
        Some(true),
        "React Native views send events"
    );
    assert_eq!(owner(&s, app), None);
    assert!(!app.modal);
    // React Native's list reports scroll actions and no collection.
    let list = scroller_above(&s, "row-1").expect("no ancestor of row-1 scrolls");
    assert_eq!(
        list.scroll,
        Some(Scroll {
            axis: ScrollAxis::VERTICAL,
            flags: ScrollFlags::FORWARD,
            range: None,
            position: None,
        })
    );
}

#[test]
fn an_in_app_alert_is_a_modal_window_of_the_app_that_sends_no_change_events() {
    let Some(dev) =
        device("an_in_app_alert_is_a_modal_window_of_the_app_that_sends_no_change_events")
    else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "system");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    let (button, _) = find(&s, "system-alert").expect("system-alert");
    let screen = window_of(&s, "system-alert").expect("the app's window");
    assert!(
        !screen.modal,
        "the full-screen activity window is not modal"
    );
    tap(&mut client, center(button.node.bounds));
    let s = poll(Duration::from_secs(5), Duration::from_millis(200), || {
        let s = snapshot(&mut client);
        find(&s, "inapp-alert-ok").is_some().then_some(s)
    })
    .expect("the alert did not appear");
    let (ok, _) = find(&s, "inapp-alert-ok").unwrap();
    let dialog = ok.window;
    let bounds = ok.node.bounds;
    let fields = (
        dialog.modal,
        owner(&s, dialog).map(str::to_owned),
        dialog.emits_events,
    );
    tap(&mut client, center(bounds));
    fixture::force_stop(&dev, fixture::COMPOSE);
    // A Compose dialog shows no Compose view class, only Compose's semantics.
    assert_eq!(fields, (true, None, Some(false)));
}

/// The location permission of the Compose fixture, put back to never asked
/// when dropped.
struct PermissionGuard<'a>(&'a Device);

impl PermissionGuard<'_> {
    fn reset(device: &Device) {
        for permission in ["ACCESS_FINE_LOCATION", "ACCESS_COARSE_LOCATION"] {
            let name = format!("{} android.permission.{permission}", fixture::COMPOSE);
            device.shell(&format!(
                "pm revoke {name}; pm clear-permission-flags {name} user-set user-fixed"
            ));
        }
    }
}

impl Drop for PermissionGuard<'_> {
    fn drop(&mut self) {
        PermissionGuard::reset(self.0);
    }
}

#[test]
fn a_permission_prompt_is_a_modal_window_owned_by_the_permission_controller() {
    let Some(dev) =
        device("a_permission_prompt_is_a_modal_window_owned_by_the_permission_controller")
    else {
        return;
    };
    PermissionGuard::reset(&dev);
    let _guard = PermissionGuard(&dev);
    fixture::launch(&dev, fixture::COMPOSE, "system");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    let (button, _) = find(&s, "system-permission").expect("system-permission");
    tap(&mut client, center(button.node.bounds));
    // The prompt slides in from below the display, and until it stops the
    // platform still lists the app's window under it, so a read then finds
    // it neither inset nor alone; each read is judged, the last one named
    // if none finds it modal.
    let mut seen = String::from("no read");
    let prompt = poll(Duration::from_secs(5), Duration::from_millis(200), || {
        let s = snapshot(&mut client);
        seen = s
            .windows
            .iter()
            .map(|w| {
                let package = w.package.and_then(|i| s.strings.get(i as usize));
                format!("{package:?} {:?} modal={}", w.bounds, w.modal)
            })
            .collect::<Vec<_>>()
            .join("; ");
        let w = s.windows.iter().find(|w| {
            w.package
                .and_then(|i| s.strings.get(i as usize))
                .is_some_and(|p| p.ends_with("permissioncontroller"))
        })?;
        let package = s.strings[w.package? as usize].clone();
        w.modal
            .then(|| (package, owner(&s, w).map(str::to_owned), w.kind))
    });
    dev.shell("input keyevent KEYCODE_BACK");
    fixture::force_stop(&dev, fixture::COMPOSE);
    // The prompt keeps the user from the app once it has come to rest.
    let (package, owned_by, kind) = prompt
        .unwrap_or_else(|| panic!("no modal permission prompt within 5 s; the last read: {seen}"));
    assert_eq!(kind, WindowType::APPLICATION);
    assert_eq!(owned_by.as_deref(), Some(package.as_str()));
}
