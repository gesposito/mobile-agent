//! The SwiftUI fixture's screens as the device tests reach them, through
//! the runner's reads and the driver's taps: the List tab after a launch,
//! and the Login tab's `login-username` focused and emptied. The crate's own
//! tests run these helpers on the fake device's SwiftUI fixture, whose reads
//! come in the shapes the phone gave them (`src/macos/setups.rs`), so a setup
//! is checked there before it runs on the phone.
//!
//! A helper acts on what the runner's reads give: SwiftUI sets the tab bar's
//! identifiers after the first rows show, so the read after the one that
//! listed `row-1` can lack `tab-login` (task 9.7's device check), and a tap
//! waits until its element is in the tree.

use std::time::{Duration, Instant};

use ios::{AttachOptions, Dispatch, Input, LaunchOptions, Lifecycle, Point, Session};
use protocol::message::{Call, Reply, SnapshotParams, TextMethod, TextParams};
use protocol::snapshot::{Bounds, Node, NodeFlags, Snapshot};

pub const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";
/// The iPhone SE (3rd generation)'s points to pixels.
pub const SCALE: f64 = 2.0;
/// How long a tap waits for its element to show in the tree: SwiftUI sets
/// identifiers up to a second after a launch, and a read of a 500-cell
/// screen takes about 0.5 s on the phone.
pub const ELEMENT_WAIT: Duration = Duration::from_secs(5);

pub fn find<'a>(node: &'a Node, id: &str) -> Option<&'a Node> {
    if node.res_id.as_deref() == Some(id) {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, id))
}

/// The bounds of the node `id` in `snapshot`, in pixels.
pub fn bounds(snapshot: &Snapshot, id: &str) -> Option<Bounds> {
    snapshot
        .windows
        .iter()
        .filter_map(|window| window.root.as_ref())
        .find_map(|root| find(root, id))
        .map(|node| node.bounds)
}

pub fn center(bounds: Bounds) -> Point {
    Point::new(
        f64::from(bounds.left + bounds.right) / 2.0 / SCALE,
        f64::from(bounds.top + bounds.bottom) / 2.0 / SCALE,
    )
}

/// The tree, read again while the foreground changes during a read, as it
/// does when a tap or the home button switches apps (`window_changed`).
pub fn snapshot<D: Session>(driver: &D, udid: &str) -> Snapshot {
    loop {
        let read = driver.call(
            udid,
            Call::Snapshot(SnapshotParams::default()),
            Duration::from_secs(10),
        );
        match read {
            Ok(Reply::Snapshot(snapshot)) => return snapshot,
            Ok(other) => panic!("a snapshot was answered with {other:?}"),
            Err(error) if error.retryable() => continue,
            Err(error) => panic!("the runner reads the tree: {error}"),
        }
    }
}

/// The first snapshot within `wait` where `done` holds, and when.
pub fn wait_for<D: Session>(
    driver: &D,
    udid: &str,
    wait: Duration,
    done: impl Fn(&Snapshot) -> bool,
) -> Option<(Snapshot, Duration)> {
    let started = Instant::now();
    loop {
        let read = snapshot(driver, udid);
        if done(&read) {
            return Some((read, started.elapsed()));
        }
        if started.elapsed() >= wait {
            return None;
        }
    }
}

/// The fixture on its List tab, the runner attached with `options`.
pub fn on_the_list_with<D: Lifecycle + Session>(driver: &D, udid: &str, options: AttachOptions) {
    driver
        .launch(
            udid,
            SWIFTUI,
            &LaunchOptions::new()
                .env("FIXTURE_INITIAL_TAB", "list")
                .terminating_running(),
        )
        .expect("the SwiftUI fixture launches; install it first");
    driver.attach(udid, &options).expect("the runner starts");
    wait_for(driver, udid, Duration::from_secs(10), |read| {
        bounds(read, "row-1").is_some()
    })
    .expect("the List tab shows row-1");
}

/// A tap on `id` once the tree shows it, within [`ELEMENT_WAIT`], and how
/// long until `expected` showed in the tree.
pub fn tap_on<D: Session + Input>(
    driver: &D,
    udid: &str,
    id: &str,
    expected: &str,
) -> (Dispatch, Option<Duration>) {
    let (read, _) = wait_for(driver, udid, ELEMENT_WAIT, |read| {
        bounds(read, id).is_some()
    })
    .unwrap_or_else(|| panic!("{id} is on screen within {ELEMENT_WAIT:?}"));
    let target = bounds(&read, id).expect("the read shows it");
    let dispatch = driver.tap(udid, center(target)).expect("the tap is sent");
    let reached = wait_for(driver, udid, Duration::from_secs(3), |read| {
        bounds(read, expected).is_some()
    })
    .map(|(_, after)| after);
    (dispatch, reached)
}

/// Enters `text` into the focused field through the runner, as `mode` asks,
/// and returns the method it reports and how long it took.
pub fn enter<D: Session>(
    driver: &D,
    udid: &str,
    text: &str,
    mode: Option<TextMethod>,
) -> (TextMethod, Duration) {
    let sent = Instant::now();
    let reply = driver
        .call(
            udid,
            Call::TextSet(TextParams {
                text: text.to_owned(),
                node: None,
                mode,
            }),
            Duration::from_secs(30),
        )
        .unwrap_or_else(|error| panic!("the runner enters {text:?}: {error}"));
    match reply {
        Reply::Text(ack) => (ack.method, sent.elapsed()),
        other => panic!("text was answered with {other:?}"),
    }
}

/// The SwiftUI fixture's Login tab with `login-username` focused through
/// the runner's taps, its text set empty.
pub fn on_the_focused_username<D: Session + Input>(driver: &D, udid: &str) {
    tap_on(driver, udid, "tab-login", "login-username");
    let read = snapshot(driver, udid);
    let field = bounds(&read, "login-username").expect("the username field");
    driver
        .tap(udid, center(field))
        .expect("the field is tapped");
    wait_for(driver, udid, Duration::from_secs(5), |read| {
        read.windows
            .iter()
            .filter_map(|window| window.root.as_ref())
            .find_map(|root| find(root, "login-username"))
            .is_some_and(|node| node.flags.contains(NodeFlags::FOCUSED))
    })
    .expect("the username field has keyboard focus");
    enter(driver, udid, "", Some(TextMethod::DIRECT));
}
