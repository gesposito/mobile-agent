//! Task 10.11: bounds probes and eviction by window (design D8). A
//! translation moves nodes without a change event, so the platform's node
//! cache keeps the bounds it read before the move: `bounds` reads nodes of
//! the last snapshot again past that cache, and a snapshot's `evict` empties
//! the cache of the windows it names. Jetpack Compose sends no events to UI
//! automation at all, so with the agent's Compose poll off, a drag the agent
//! did not inject moves rows that only a probe or a fresh read can see.
//! A window the platform animates in reads the same to both until the
//! animation ends; snapshots flag it transformed meanwhile.

use std::time::{Duration, Instant};

use agent_android_host_tests::agent::fields;
use agent_android_host_tests::client::Client;

use agent_android_host_tests::tree::{find, nodes, package, resource_id};
use agent_android_host_tests::{Agent, Device, Launch, device, fixture};
use protocol::message::{BoundsParams, Call, NodeBounds, Reply, SnapshotParams};
use protocol::snapshot::{Bounds, Snapshot, WindowFlags, WindowType};

/// A snapshot without an idle wait, evicting the windows `evict` names.
fn read(client: &mut Client, evict: Option<Vec<u32>>) -> Snapshot {
    match client.call(Call::Snapshot(SnapshotParams {
        idle_ms: Some(0),
        evict,
        ..SnapshotParams::default()
    })) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("snapshot failed: {other:?}"),
    }
}

/// A probe of `nodes`, with the agent's trace line for it.
fn probe(client: &mut Client, agent: &mut Agent, nodes: Vec<u32>) -> (Vec<NodeBounds>, String) {
    agent.drain_lines();
    let found = match client.call(Call::Bounds(BoundsParams { nodes })) {
        Ok(Reply::Bounds(probed)) => probed.nodes,
        other => panic!("bounds failed: {other:?}"),
    };
    let line = agent
        .wait_line(Duration::from_secs(2), |l| l.starts_with("trace bounds "))
        .expect("trace");
    (found, line)
}

/// The id and bounds of the node with resource id `id`.
#[track_caller]
fn node(s: &Snapshot, id: &str) -> (u32, Bounds) {
    let (found, _) = find(s, id).unwrap_or_else(|| panic!("no {id}"));
    (found.node.id, found.node.bounds)
}

fn app_window(s: &Snapshot) -> u32 {
    s.windows
        .iter()
        .find(|w| w.kind == WindowType::APPLICATION)
        .expect("an app window")
        .id
}

#[test]
fn a_probe_answers_the_nodes_of_the_last_snapshot_with_their_bounds() {
    let Some(dev) = device("a_probe_answers_the_nodes_of_the_last_snapshot_with_their_bounds")
    else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "login");
    let mut agent = Agent::launch(&dev, Launch::default().arg("--trace"));
    let mut client = agent.connect();
    // Once the launch has settled, a read with the app's window evicted.
    client.idle(Duration::from_millis(1500));
    let window = app_window(&read(&mut client, None));
    let screen = read(&mut client, Some(vec![window]));
    let (tab, tab_bounds) = node(&screen, "tab-big");
    let (field, field_bounds) = node(&screen, "login-username");
    let (found, line) = probe(&mut client, &mut agent, vec![tab, 1 << 20, field, u32::MAX]);
    println!("{line}");
    assert_eq!(
        found,
        [
            NodeBounds {
                node: tab,
                bounds: tab_bounds,
            },
            NodeBounds {
                node: field,
                bounds: field_bounds,
            },
        ],
        "the nodes the snapshot holds, in the request's order, where the snapshot has them; \
         ids it does not hold are left out"
    );
    let f = fields(&line);
    assert_eq!(f["nodes"], "4", "{line}");
    assert_eq!(f["found"], "2", "{line}");
    assert_eq!(
        f["round_trips"], "2",
        "one round trip per node the snapshot holds: {line}"
    );
}

#[test]
fn a_probe_sees_rows_a_drag_moved_that_the_last_snapshot_does_not_have() {
    let Some(dev) = device("a_probe_sees_rows_a_drag_moved_that_the_last_snapshot_does_not_have")
    else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "list");
    let mut agent = Agent::launch(
        &dev,
        Launch::default()
            .arg("--trace")
            .arg("--compose-poll-ms")
            .arg("0"),
    );
    let mut client = agent.connect();
    let before = read(&mut client, None);
    let window = app_window(&before);
    // A row in the lower half of the screen, which a drag of 600 px keeps
    // on it.
    let row = (5..=40)
        .map(|n| format!("row-{n}"))
        .find(|id| {
            find(&before, id)
                .is_some_and(|(f, _)| f.node.bounds.top > 1300 && f.node.bounds.bottom < 1900)
        })
        .expect("a row in the lower half of the screen");
    let (id, was) = node(&before, &row);
    // Input the agent did not inject drags the list up about 580 px and
    // rests before it lifts, so no fling carries the rows further; Compose
    // reports none of it, and the agent's poll is off.
    dev.shell(
        "input motionevent DOWN 540 1700; input motionevent MOVE 540 1500; \
         input motionevent MOVE 540 1300; input motionevent MOVE 540 1100; \
         sleep 0.3; input motionevent UP 540 1100",
    );
    client.idle(Duration::from_millis(1000));
    // The probe names the row by its id in the snapshot before the drag.
    let (found, line) = probe(&mut client, &mut agent, vec![id]);
    println!("{line}");
    let [moved] = found.as_slice() else {
        panic!("{row} is still on the screen: {found:?}");
    };
    assert!(
        moved.bounds.top < was.top - 300,
        "the probe sees {row} where the drag took it: {:?}, was {was:?}",
        moved.bounds
    );
    assert_eq!(fields(&line)["round_trips"], "1", "{line}");

    let evicted = read(&mut client, Some(vec![window]));
    let (_, fresh) = node(&evicted, &row);
    assert_eq!(
        fresh, moved.bounds,
        "a read evicting the window shows {row} where the probe found it"
    );
}

/// A snapshot's `evict` empties the cache of the windows it names: on the
/// Compose Big tab, 500 cells under one parent, a read evicting the app's
/// window fetches every node again (spike S1: about 460 round trips cold),
/// while the reads before and after it are served from the cache.
#[test]
fn a_snapshot_evicting_a_window_reads_it_again_in_full() {
    let Some(dev) = device("a_snapshot_evicting_a_window_reads_it_again_in_full") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "big");
    let mut agent = Agent::launch(
        &dev,
        Launch::default()
            .arg("--trace")
            .arg("--compose-poll-ms")
            .arg("0"),
    );
    let mut client = agent.connect();
    let window = app_window(&read(&mut client, None));
    // Once the launch's events are over, a read fills the cache again.
    client.idle(Duration::from_millis(1000));
    let _ = read(&mut client, None);
    let (_, warm) = traced_read(&mut client, &mut agent, None);
    let (after, evicting) = traced_read(&mut client, &mut agent, Some(vec![window]));
    let (_, later) = traced_read(&mut client, &mut agent, None);
    println!("warm: {warm}\nevicting: {evicting}\nlater: {later}");
    assert!(find(&after, "cell-500").is_some(), "the Big tab is read");
    let trips = |line: &str| -> i64 { fields(line)["round_trips"].parse().expect("round trips") };
    assert!(
        trips(&warm) < 100,
        "a read that evicts nothing is served from the cache: {warm}"
    );
    assert!(
        trips(&evicting) >= 100,
        "the evicting read was served from the cache: {evicting}"
    );
    assert!(
        trips(&later) < 100,
        "the read after it is served from the cache again: {later}"
    );
}

/// A snapshot without an idle wait, evicting the windows `evict` names,
/// with the agent's trace line.
fn traced_read(
    client: &mut Client,
    agent: &mut Agent,
    evict: Option<Vec<u32>>,
) -> (Snapshot, String) {
    agent.drain_lines();
    let snapshot = read(client, evict);
    let line = agent
        .wait_line(Duration::from_secs(2), |l| l.starts_with("trace snapshot "))
        .expect("trace");
    (snapshot, line)
}

/// The Expo fixture launched over `front`, or over the launcher: whether
/// each read of the next 2.5 s flagged the app's window transformed, and
/// where it had the Big tab. Every other read evicts the app's window
/// first, as the host does once probes find it moved, so that its nodes,
/// its root among them, are fetched again rather than served from the
/// platform's cache.
fn launch_reads(dev: &Device, front: Option<&str>) -> Vec<(bool, Bounds)> {
    fixture::force_stop(dev, fixture::EXPO);
    match front {
        Some(package) => {
            dev.shell(&format!("am start -W -n {package}/.MainActivity"));
        }
        None => {
            dev.shell("input keyevent KEYCODE_HOME");
        }
    }
    std::thread::sleep(Duration::from_millis(1500));
    let agent = Agent::launch(dev, Launch::default());
    let mut client = agent.connect();
    dev.shell(&format!(
        "am start -S -n {}/.MainActivity --es initial_tab login",
        fixture::EXPO
    ));
    let mut reads = Vec::new();
    let mut app = None;
    let end = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < end {
        let evict = app.filter(|_| reads.len() % 2 == 1).map(|id| vec![id]);
        let snapshot = read(&mut client, evict);
        let tab = nodes(&snapshot).into_iter().find(|(f, _)| {
            f.window.kind == WindowType::APPLICATION
                && package(&snapshot, f.window) == Some(fixture::EXPO)
                && resource_id(&snapshot, f.node).as_deref() == Some("tab-big")
        });
        if let Some((f, _)) = tab {
            app = Some(f.window.id);
            reads.push((
                f.window.flags.contains(WindowFlags::TRANSFORMED),
                f.node.bounds,
            ));
        }
        client.idle(Duration::from_millis(40));
    }
    reads
}

/// Whether the device reports bounds in the window, from which the agent
/// tells a window drawn at another size than its layout's.
fn reports_bounds_in_window(dev: &Device) -> bool {
    let api: u32 = dev
        .shell("getprop ro.build.version.sdk")
        .trim()
        .parse()
        .unwrap_or(0);
    if api < 34 {
        eprintln!("skipped: API {api} reports no bounds in the window");
    }
    api >= 34
}

/// The reads flag the window transformed for a while after the launch,
/// and every read that does not flag it has the Big tab where it comes to
/// rest. A read may still flag it with the tab at rest as the animation
/// ends, the window's frame reading a moment later than its nodes; the
/// settle then waits one read longer.
fn assert_flagged_while_away(reads: &[(bool, Bounds)]) {
    let &(transformed_last, rest) = reads.last().expect("the Expo login screen");
    assert!(
        !transformed_last,
        "at rest 2.5 s after the launch: {reads:?}"
    );
    assert!(
        reads.iter().any(|&(transformed, _)| transformed),
        "the launch animation reads as transformed: {reads:?}"
    );
    for &(transformed, tab) in reads {
        assert!(
            transformed || tab == rest,
            "a read that does not flag the window has the tab at rest: {reads:?}"
        );
    }
}

/// Android 14 and later report a window the platform animates in at one
/// frame of that animation, the same to every read and probe, until the
/// animation ends. Over the launcher, the API 36 emulator scales the Expo
/// app in, reporting it at about 95% of its size for about 300 ms: reads
/// flag the window transformed meanwhile, and one that does not flag it has
/// the app's nodes where they come to rest.
#[test]
fn a_window_the_platform_scales_in_reads_as_transformed_until_it_comes_to_rest() {
    let Some(dev) =
        device("a_window_the_platform_scales_in_reads_as_transformed_until_it_comes_to_rest")
    else {
        return;
    };
    if !reports_bounds_in_window(&dev) {
        return;
    }
    assert_flagged_while_away(&launch_reads(&dev, None));
}

/// Over another app, the API 36 emulator slides the Expo app in from the
/// right, reporting its window partly off the display for about 500 ms.
#[test]
fn a_window_the_platform_slides_in_reads_as_transformed_until_it_comes_to_rest() {
    let Some(dev) =
        device("a_window_the_platform_slides_in_reads_as_transformed_until_it_comes_to_rest")
    else {
        return;
    };
    if !reports_bounds_in_window(&dev) {
        return;
    }
    fixture::require(&dev, fixture::COMPOSE);
    assert_flagged_while_away(&launch_reads(&dev, Some(fixture::COMPOSE)));
}
