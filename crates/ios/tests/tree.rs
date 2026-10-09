//! Task 4.4 on the device: the runner's reads of the fixtures' screens list
//! through the mirror as the Simulator reader's do, node for node: the
//! SwiftUI and Expo Big tabs with their 500 cells, the SwiftUI Login tab
//! with the keyboard up, and the SwiftUI System tab with its in-app alert.
//! The mirror's full listings are compared line for line, so refs, roles,
//! names, identifiers, values, the keyboard's line and what the alert's modal
//! window covers must agree; bounds are not in a listing, since the SE's
//! screen is not the Simulator's iPhone 17.
//!
//! The Simulator's listings are recorded under `tests/fixtures/listings/` by
//! `record_the_simulators_listings`, ignored, on a booted Simulator with both
//! fixtures installed (the Simulator driver's tests' setup):
//!
//! ```sh
//! MOBILE_AGENT_SIM_UDID=<udid> cargo test -p ios --test tree -- --ignored --nocapture record
//! ```

#![cfg(target_os = "macos")]

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ios::{AttachOptions, Input, IosDriver, LaunchOptions, Lifecycle, Point, Session};
use mirror::backend::{BackendInfo, TreeRead};
use mirror::testing::FakeTarget;
use mirror::{Mirror, MirrorConfig, SnapshotOptions};
use protocol::message::{Call, Reply, SnapshotParams};
use protocol::snapshot::{Node, NodeFlags, Snapshot};

const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";
const EXPO: &str = "dev.mobileagent.fixture.expo";

/// When a screen is ready to read.
#[derive(Clone, Copy)]
enum Ready {
    /// The elements with these identifiers are in the tree.
    Shows([&'static str; 2]),
    /// The element with this identifier has keyboard focus, and the keyboard
    /// had a second to come up.
    Focused(&'static str),
}

/// A fixture's screen: the tab it launches on, what shows it is ready (the
/// tab's own identifier among them, which SwiftUI sets after the first
/// cells show), the element tapped then, and what shows the tap took.
struct Screen {
    name: &'static str,
    bundle: &'static str,
    tab: &'static str,
    ready: [&'static str; 2],
    tap: Option<(&'static str, Ready)>,
    /// What the Simulator's listing leaves out for its screen's size alone,
    /// by identifier: the iPhone 17's vertical scroll indicator covers the
    /// last column of the Expo fixture's grid, below its first row, where
    /// the SE's grid fits its scroll view. The device lists them, and the
    /// refs after them follow, so this screen's refs are compared in order
    /// but not by number.
    covered: fn(&str) -> bool,
}

fn nothing(_: &str) -> bool {
    false
}

/// The Expo grid's last column below its first row: cells 40, 60, ..., 500.
fn last_column(id: &str) -> bool {
    id.strip_prefix("cell-")
        .and_then(|n| n.parse::<u32>().ok())
        .is_some_and(|n| n > 20 && n % 20 == 0)
}

const SCREENS: [Screen; 4] = [
    Screen {
        name: "swiftui-big",
        bundle: SWIFTUI,
        tab: "big",
        ready: ["cell-500", "tab-big"],
        tap: None,
        covered: nothing,
    },
    Screen {
        name: "expo-big",
        bundle: EXPO,
        tab: "big",
        ready: ["cell-500", "tab-big"],
        tap: None,
        covered: last_column,
    },
    Screen {
        name: "swiftui-login-keyboard",
        bundle: SWIFTUI,
        tab: "login",
        ready: ["login-username", "tab-login"],
        tap: Some(("login-username", Ready::Focused("login-username"))),
        covered: nothing,
    },
    Screen {
        name: "swiftui-alert",
        bundle: SWIFTUI,
        tab: "system",
        ready: ["system-alert", "tab-system"],
        tap: Some((
            "system-alert",
            Ready::Shows(["inapp-alert-ok", "inapp-alert-ok"]),
        )),
        covered: nothing,
    },
];

fn listings_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/listings")
}

fn find<'a>(node: &'a Node, id: &str) -> Option<&'a Node> {
    if node.res_id.as_deref() == Some(id) {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, id))
}

/// The node `id` of `snapshot`, in any window.
fn node<'a>(snapshot: &'a Snapshot, id: &str) -> Option<&'a Node> {
    snapshot
        .windows
        .iter()
        .filter_map(|window| window.root.as_ref())
        .find_map(|root| find(root, id))
}

fn ready(snapshot: &Snapshot, ready: Ready) -> bool {
    match ready {
        Ready::Shows(ids) => ids.iter().all(|id| node(snapshot, id).is_some()),
        Ready::Focused(id) => {
            node(snapshot, id).is_some_and(|n| n.flags.contains(NodeFlags::FOCUSED))
        }
    }
}

/// The node's center in points, at `scale` pixels a point.
fn center(node: &Node, scale: f64) -> (f64, f64) {
    let b = node.bounds;
    (
        f64::from(b.left + b.right) / 2.0 / scale,
        f64::from(b.top + b.bottom) / 2.0 / scale,
    )
}

/// The mirror's full listing of a read.
fn listing(snapshot: Snapshot, scale: f64) -> String {
    let target = FakeTarget::new(TreeRead {
        snapshot,
        backend: BackendInfo::new("reference", false),
        scale,
    });
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror
        .snapshot(&SnapshotOptions::full())
        .expect("the mirror lists the read")
        .text
}

/// Reads the screen through `read` until `wanted` holds, for up to 20 s.
fn read_until(mut read: impl FnMut() -> Option<(Snapshot, f64)>, wanted: Ready) -> (Snapshot, f64) {
    let started = Instant::now();
    loop {
        if let Some((snapshot, scale)) = read()
            && ready(&snapshot, wanted)
        {
            if matches!(wanted, Ready::Focused(_)) {
                std::thread::sleep(Duration::from_secs(1));
                if let Some(again) = read() {
                    return again;
                }
            }
            return (snapshot, scale);
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the screen did not get ready within 20 s"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The device's tree through the runner, at the SE's 2 pixels a point (the
/// scale moves bounds in points, which a listing does not print); `None`
/// when the read can be repeated, as when the foreground changed during it.
fn device_read(driver: &IosDriver, udid: &str) -> Option<(Snapshot, f64)> {
    match driver.call(
        udid,
        Call::Snapshot(SnapshotParams::default()),
        Duration::from_secs(10),
    ) {
        Ok(Reply::Snapshot(snapshot)) => Some((snapshot, 2.0)),
        Ok(other) => panic!("a snapshot was answered with {other:?}"),
        Err(error) if error.retryable() => None,
        Err(error) => panic!("the runner reads the tree: {error}"),
    }
}

/// A listing without what the screen's size decides: the scroll
/// indicators' page counts, which UIKit words from the content's length and
/// the scroll view's (the alert's message is 72.33 points in a 72-point view
/// on the iPhone 17, 2 pages, and fits on the SE, 1), and the scroll hints,
/// which follow from where the content sits on the screen; without the lines
/// of the elements `covered` names, and without ref numbers when there were
/// any.
fn normalized(listing: &str, covered: fn(&str) -> bool, refs: bool) -> String {
    let lines = listing.lines().filter(|line| {
        !line.trim_start().starts_with("(more items")
            && !listed_identifier(line).is_some_and(covered)
    });
    let lines = lines.map(|line| {
        let line = match line.find(" scroll bar, ") {
            Some(at) => {
                let end = line[at..].find('"').map_or(line.len(), |quote| at + quote);
                format!("{} scroll bar{}", &line[..at], &line[end..])
            }
            None => line.to_owned(),
        };
        match line
            .find(", ref=e")
            .or_else(|| line.find("[ref=e"))
            .filter(|_| !refs)
        {
            Some(at) => {
                let start = at + line[at..].find("ref=e").expect("found above");
                let end = start + line[start..].find(']').unwrap_or(line.len() - start);
                format!("{}ref=e…{}", &line[..start], &line[end..])
            }
            None => line,
        }
    });
    lines.collect::<Vec<_>>().join("\n")
}

/// The identifier a listing's line names.
fn listed_identifier(line: &str) -> Option<&str> {
    line.split("id=")
        .nth(1)
        .and_then(|rest| rest.split([',', ']']).next())
}

/// The lines of `ours` that differ from `theirs`, with their numbers.
fn differences(ours: &str, theirs: &str) -> Vec<String> {
    let (a, b): (Vec<&str>, Vec<&str>) = (ours.lines().collect(), theirs.lines().collect());
    (0..a.len().max(b.len()))
        .filter(|&i| a.get(i) != b.get(i))
        .map(|i| {
            format!(
                "line {}: device {:?}, Simulator {:?}",
                i + 1,
                a.get(i).unwrap_or(&"(none)"),
                b.get(i).unwrap_or(&"(none)")
            )
        })
        .collect()
}

#[test]
fn the_fixtures_screens_list_through_the_mirror_as_on_the_simulator() {
    let Some(udid) = common::gated("the_fixtures_screens_list_as_on_the_simulator") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    let mut mismatched = Vec::new();
    for screen in &SCREENS {
        driver
            .launch(
                &udid,
                screen.bundle,
                &LaunchOptions::new()
                    .env("FIXTURE_INITIAL_TAB", screen.tab)
                    .terminating_running(),
            )
            .unwrap_or_else(|error| panic!("{} launches: {error}", screen.bundle));
        driver
            .attach(&udid, &AttachOptions::new(common::runner()))
            .expect("the runner starts");
        let (mut snapshot, mut scale) =
            read_until(|| device_read(&driver, &udid), Ready::Shows(screen.ready));
        if let Some((id, after)) = screen.tap {
            let (x, y) = center(node(&snapshot, id).expect("the element to tap"), scale);
            driver
                .tap(&udid, Point::new(x, y))
                .expect("the tap is sent");
            (snapshot, scale) = read_until(|| device_read(&driver, &udid), after);
        }
        let read_at = Instant::now();
        let ours = listing(snapshot.clone(), scale);
        let path = listings_dir().join(format!("{}.txt", screen.name));
        let theirs = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}; record it first", path.display()));
        // Refs are compared by number unless the device lists what the
        // Simulator's screen covered, which numbers the rest differently.
        let refs = !ours
            .lines()
            .any(|line| listed_identifier(line).is_some_and(screen.covered));
        let differ = differences(
            &normalized(&ours, screen.covered, refs),
            &normalized(&theirs, screen.covered, refs),
        );
        if let Some(cells) = screen.ready[0].strip_prefix("cell-") {
            let cells: u32 = cells.parse().expect("a cell count");
            let read = (1..=cells)
                .filter(|n| node(&snapshot, &format!("cell-{n}")).is_some())
                .count();
            assert_eq!(
                read, cells as usize,
                "{}: every cell in one read",
                screen.name
            );
        }
        eprintln!(
            "{}: {} nodes, {} listed lines, {} differing from the Simulator's (listed in {:?})",
            screen.name,
            snapshot.node_count,
            ours.lines().count(),
            differ.len(),
            read_at.elapsed()
        );
        if !differ.is_empty() {
            eprintln!("{} on the device:\n{ours}", screen.name);
            for line in &differ {
                eprintln!("  {line}");
            }
            mismatched.push(screen.name);
        }
    }
    driver.release(&udid).expect("releases");
    assert!(
        mismatched.is_empty(),
        "these screens list differently on the device: {mismatched:?}"
    );
}

/// Records the Simulator's listings of the screens, through the Simulator
/// driver and its reader.
#[test]
#[ignore = "records the Simulator's listings; needs MOBILE_AGENT_SIM_UDID"]
fn record_the_simulators_listings() {
    use simulator::{Input as _, Lifecycle as _, Tree as _};

    let Ok(udid) = std::env::var("MOBILE_AGENT_SIM_UDID") else {
        eprintln!("record_the_simulators_listings: skipped; set MOBILE_AGENT_SIM_UDID");
        return;
    };
    let sim = simulator::SimulatorDriver::open().expect("the Simulator driver opens");
    let read = || match sim.snapshot(&udid, &simulator::TreeOptions::default()) {
        Ok(tree) => {
            assert_eq!(
                tree.backend,
                simulator::TreeBackend::Reader,
                "the reader reads"
            );
            Some((tree.snapshot, tree.scale))
        }
        Err(error) if error.retryable() => None,
        Err(error) => panic!("the Simulator's tree: {error}"),
    };
    std::fs::create_dir_all(listings_dir()).expect("the listings' directory");
    for screen in &SCREENS {
        sim.launch(
            &udid,
            screen.bundle,
            &simulator::LaunchOptions::new()
                .env("FIXTURE_INITIAL_TAB", screen.tab)
                .terminating_running(),
        )
        .unwrap_or_else(|error| panic!("{} launches: {error}", screen.bundle));
        let (mut snapshot, mut scale) = read_until(read, Ready::Shows(screen.ready));
        if let Some((id, after)) = screen.tap {
            let (x, y) = center(node(&snapshot, id).expect("the element to tap"), scale);
            sim.tap(&udid, simulator::Point::new(x, y))
                .expect("the tap is sent");
            (snapshot, scale) = read_until(read, after);
        }
        let text = listing(snapshot, scale);
        let path = listings_dir().join(format!("{}.txt", screen.name));
        std::fs::write(&path, &text).expect("the listing is written");
        eprintln!(
            "{}: {} lines to {}",
            screen.name,
            text.lines().count(),
            path.display()
        );
    }
    // The Simulator driver's tests expect the Expo fixture on its Login tab.
    sim.launch(
        &udid,
        EXPO,
        &simulator::LaunchOptions::new()
            .env("FIXTURE_INITIAL_TAB", "login")
            .terminating_running(),
    )
    .expect("the Expo fixture goes back to Login");
}

/// Nearest-rank percentile of sorted samples.
fn rank(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

/// The measured series of design D20's tree gate (under 1 s at p95): the
/// SwiftUI and Expo Big tabs read through the runner, 20 times each after 2
/// warm-up reads, from the request to the reply, on a quiet host.
#[test]
#[ignore = "a measurement, run on a quiet host"]
fn tree_series() {
    let Some(udid) = common::gated("tree_series") else {
        return;
    };
    let _device = common::exclusive();
    let driver = IosDriver::open().expect("the driver opens");
    for screen in SCREENS.iter().filter(|screen| screen.tab == "big") {
        driver
            .launch(
                &udid,
                screen.bundle,
                &LaunchOptions::new()
                    .env("FIXTURE_INITIAL_TAB", screen.tab)
                    .terminating_running(),
            )
            .unwrap_or_else(|error| panic!("{} launches: {error}", screen.bundle));
        driver
            .attach(&udid, &AttachOptions::new(common::runner()))
            .expect("the runner starts");
        read_until(|| device_read(&driver, &udid), Ready::Shows(screen.ready));
        let quiet = common::quiet_host(screen.name, Duration::from_secs(90 * 60));
        let before = common::load();
        let mut times = Vec::new();
        let mut nodes = 0;
        for index in 0..22 {
            let sent = Instant::now();
            let reply = driver
                .call(
                    &udid,
                    Call::Snapshot(SnapshotParams::default()),
                    Duration::from_secs(10),
                )
                .expect("a read");
            let took = sent.elapsed().as_secs_f64() * 1e3;
            let Reply::Snapshot(snapshot) = reply else {
                panic!("a snapshot was answered with {reply:?}");
            };
            nodes = snapshot.node_count;
            if index >= 2 {
                times.push(took);
            }
        }
        let after = common::load();
        times.sort_by(f64::total_cmp);
        eprintln!(
            "{}: n={} {nodes} nodes, request to reply p50 {:.1} / p95 {:.1} ms (min {:.1}, max \
             {:.1}); guard {} checks over {:.0} s, relaxed {}; load before {before:?}, after \
             {after:?}",
            screen.name,
            times.len(),
            rank(&times, 50),
            rank(&times, 95),
            times[0],
            times[times.len() - 1],
            quiet.checks,
            quiet.waited.as_secs_f64(),
            quiet.relaxed
        );
        assert!(rank(&times, 95) < 1000.0, "p95 under 1 s");
    }
    driver.release(&udid).expect("releases");
}
