//! Task 4.3: tree serialization across interactive windows (spec
//! android-driver, Tree acquisition across windows; spec device-protocol,
//! deadlines and oversized frames on the agent side).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use agent_android_host_tests::client::Client;
use agent_android_host_tests::tree::{self, center, find, find_all, nodes, visible};
use agent_android_host_tests::{Agent, Launch, device, fixture, ms};
use protocol::code;
use protocol::message::{Call, Reply, SnapshotParams};
use protocol::session::HostConfig;
use protocol::snapshot::{NodeFlags, Snapshot, Truncation, WindowType};

fn snapshot(client: &mut Client, params: SnapshotParams) -> (Snapshot, Duration) {
    let t0 = Instant::now();
    match client.call(Call::Snapshot(params)) {
        Ok(Reply::Snapshot(s)) => (s, t0.elapsed()),
        other => panic!("snapshot failed: {other:?}"),
    }
}

fn no_wait() -> SnapshotParams {
    SnapshotParams {
        idle_ms: Some(0),
        ..SnapshotParams::default()
    }
}

/// Every cell of the Big screen, once each, and all visible.
fn assert_all_cells(s: &Snapshot) {
    let cells = find_all(s, "cell-");
    let ids: HashSet<String> = cells
        .iter()
        .filter_map(|f| tree::resource_id(s, f.node))
        .collect();
    assert_eq!(ids.len(), 500, "distinct cells");
    for n in 1..=500 {
        assert!(ids.contains(&format!("cell-{n}")), "cell-{n} is missing");
    }
    assert!(
        cells
            .iter()
            .all(|f| f.node.flags.contains(NodeFlags::VISIBLE))
    );
}

/// The string table holds each string once and every index is in range.
fn assert_string_table(s: &Snapshot) {
    let unique: HashSet<&String> = s.strings.iter().collect();
    assert_eq!(unique.len(), s.strings.len(), "repeated strings");
    for (f, _) in nodes(s) {
        assert!((f.node.class as usize) < s.strings.len());
        if let Some(p) = f.node.res_pkg {
            assert!((p as usize) < s.strings.len());
        }
    }
    for w in &s.windows {
        if let Some(p) = w.package {
            assert!((p as usize) < s.strings.len());
        }
    }
}

fn big_screen(package: &str) {
    let Some(dev) = device(&format!("big_screen {package}")) else {
        return;
    };
    fixture::launch(&dev, package, "big");
    // No Compose re-read poll between the reads being timed.
    let mut agent = Agent::launch(
        &dev,
        Launch::default()
            .arg("--trace")
            .arg("--compose-poll-ms")
            .arg("0"),
    );
    let mut client = agent.connect();
    let (cold, cold_time) = snapshot(&mut client, no_wait());
    let (warm, warm_time) = snapshot(&mut client, no_wait());
    let _ = snapshot(&mut client, no_wait());
    let _ = snapshot(&mut client, no_wait());
    for read in ["cold", "warm", "warm", "warm"] {
        let line = agent
            .wait_line(Duration::from_secs(2), |l| l.starts_with("trace snapshot "))
            .unwrap_or_default();
        println!("{package} {read}: {line}");
    }
    for s in [&cold, &warm] {
        assert_eq!(s.truncated, Truncation::EMPTY);
        assert_eq!(s.node_count as usize, tree::count(s));
        assert_all_cells(s);
        assert_string_table(s);
        assert!(visible(s) <= tree::count(s));
    }
    let kinds: Vec<WindowType> = cold.windows.iter().map(|w| w.kind).collect();
    assert!(kinds.contains(&WindowType::APPLICATION), "{kinds:?}");
    assert!(kinds.contains(&WindowType::SYSTEM), "{kinds:?}");
    let app = cold
        .windows
        .iter()
        .find(|w| w.kind == WindowType::APPLICATION)
        .expect("an app window");
    assert_eq!(tree::package(&cold, app), Some(package));
    // Top-most first.
    let layers: Vec<i32> = cold.windows.iter().map(|w| w.layer).collect();
    assert!(layers.windows(2).all(|p| p[0] >= p[1]), "{layers:?}");
    println!(
        "{package} Big: {} nodes, {} visible, {} windows, {} strings; cold {:.1} ms, warm {:.1} ms (host round trip)",
        cold.node_count,
        visible(&cold),
        cold.windows.len(),
        cold.strings.len(),
        ms(cold_time),
        ms(warm_time)
    );
}

#[test]
fn the_compose_500_cell_screen_returns_every_cell_with_total_and_visible_counts() {
    big_screen(fixture::COMPOSE);
}

#[test]
fn the_expo_500_cell_screen_returns_every_cell_with_total_and_visible_counts() {
    big_screen(fixture::EXPO);
}

#[test]
fn a_lower_node_cap_truncates_and_still_reports_the_full_count() {
    let Some(dev) = device("a_lower_node_cap_truncates_and_still_reports_the_full_count") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "big");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let (full, _) = snapshot(&mut client, no_wait());
    let (capped, _) = snapshot(
        &mut client,
        SnapshotParams {
            max_nodes: Some(100),
            ..no_wait()
        },
    );
    assert!(
        capped.truncated.contains(Truncation::NODES),
        "{:?}",
        capped.truncated
    );
    assert!(!capped.truncated.contains(Truncation::DEPTH));
    assert_eq!(tree::count(&capped), 100);
    assert_eq!(capped.node_count, full.node_count, "the full count");
    println!(
        "node cap 100: {} serialized, {} omitted of {}",
        tree::count(&capped),
        capped.node_count as usize - tree::count(&capped),
        capped.node_count
    );
}

#[test]
fn a_lower_depth_cap_truncates_below_it_and_still_reports_the_full_count() {
    let Some(dev) = device("a_lower_depth_cap_truncates_below_it_and_still_reports_the_full_count")
    else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "big");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let (full, _) = snapshot(&mut client, no_wait());
    let (capped, _) = snapshot(
        &mut client,
        SnapshotParams {
            max_depth: Some(3),
            ..no_wait()
        },
    );
    assert!(
        capped.truncated.contains(Truncation::DEPTH),
        "{:?}",
        capped.truncated
    );
    assert!(nodes(&capped).iter().all(|(f, _)| f.depth <= 3));
    assert!(nodes(&capped).iter().any(|(f, _)| f.depth == 3));
    assert!(tree::count(&capped) < tree::count(&full));
    assert_eq!(capped.node_count, full.node_count, "the full count");
}

#[test]
fn the_keyboard_window_appears_with_its_keys_when_a_field_is_focused() {
    let Some(dev) = device("the_keyboard_window_appears_with_its_keys_when_a_field_is_focused")
    else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let (before, _) = snapshot(&mut client, no_wait());
    assert!(
        !before
            .windows
            .iter()
            .any(|w| w.kind == WindowType::INPUT_METHOD),
        "a keyboard before any field was focused"
    );
    let (field, _) = find(&before, "login-username").expect("login-username");
    let (x, y) = center(field.node.bounds);
    dev.shell(&format!("input tap {x} {y}"));
    let with_keyboard =
        agent_android_host_tests::poll(Duration::from_secs(5), Duration::from_millis(200), || {
            let (s, _) = snapshot(&mut client, no_wait());
            s.windows
                .iter()
                .any(|w| w.kind == WindowType::INPUT_METHOD && w.root.is_some())
                .then_some(s)
        });
    // Stopping the app closes the keyboard; Back could reach the app instead.
    fixture::force_stop(&dev, fixture::COMPOSE);
    let s = with_keyboard.expect("no keyboard window within 5 s of focusing the field");
    let ime = s
        .windows
        .iter()
        .find(|w| w.kind == WindowType::INPUT_METHOD)
        .expect("keyboard window");
    // The keys: actionable nodes with a label. Gboard on an AVD whose
    // touchscreen also reports a stylus source shows its stylus toolbar
    // (voice typing, Delete, Next, emoji, stylus options) rather than letters.
    let keys: Vec<String> = nodes(&s)
        .iter()
        .filter(|(f, _)| f.window.id == ime.id && f.node.flags.contains(NodeFlags::CLICKABLE))
        .filter_map(|(f, _)| f.node.desc.clone().or(f.node.text.clone()))
        .collect();
    assert!(keys.len() >= 3, "keyboard keys with a label: {keys:?}");
    assert!(
        s.windows.iter().any(|w| w.kind == WindowType::APPLICATION),
        "the app window is missing alongside the keyboard"
    );
    let (focused, _) = find(&s, "login-username").expect("login-username");
    assert!(focused.node.flags.contains(NodeFlags::FOCUSED));
    assert!(focused.node.flags.contains(NodeFlags::EDITABLE));
    println!(
        "keyboard: {} labelled nodes, e.g. {:?}",
        keys.len(),
        &keys[..keys.len().min(8)]
    );
}

#[test]
fn a_snapshot_over_the_negotiated_frame_size_is_refused_naming_the_limit_and_request() {
    let Some(dev) =
        device("a_snapshot_over_the_negotiated_frame_size_is_refused_naming_the_limit_and_request")
    else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "big");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = Client::with_config(
        agent.device(),
        &agent.socket,
        HostConfig {
            token: agent.token.clone(),
            max_frame: 4096,
            ..HostConfig::default()
        },
    )
    .expect("open");
    client
        .handshake(Duration::from_secs(10))
        .expect("handshake");
    let id = client.send(Call::Snapshot(no_wait()), None).expect("send");
    let error = client
        .wait(id, Duration::from_secs(30))
        .expect_err("a snapshot larger than 4096 bytes arrived");
    assert_eq!(error.code, code::FRAME_TOO_LARGE);
    assert!(error.message.contains("4096"), "{}", error.message);
    assert!(
        error.message.contains(&format!("request {id}")),
        "{}",
        error.message
    );
    // The connection stays usable: a capped snapshot fits.
    let small = client.call(Call::Snapshot(SnapshotParams {
        max_nodes: Some(5),
        ..no_wait()
    }));
    assert!(matches!(small, Ok(Reply::Snapshot(_))), "{small:?}");
}

#[test]
fn a_snapshot_past_its_deadline_gets_a_deadline_error_and_the_next_request_is_served() {
    let Some(dev) =
        device("a_snapshot_past_its_deadline_gets_a_deadline_error_and_the_next_request_is_served")
    else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "big");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    // The first read after an attach is cold: several hundred round trips.
    let t0 = Instant::now();
    let error = client
        .call_within(
            Call::Snapshot(no_wait()),
            Some(Duration::from_millis(200)),
            Duration::from_secs(30),
        )
        .expect_err("a cold read of the Big screen finished within 200 ms");
    let answered = t0.elapsed();
    assert_eq!(error.code, code::DEADLINE_EXCEEDED);
    assert!(error.retryable);
    assert!(
        answered < Duration::from_millis(450),
        "the deadline error came after {answered:?}"
    );
    let (s, took) = snapshot(&mut client, no_wait());
    assert_all_cells(&s);
    println!(
        "deadline error after {:.1} ms; next snapshot {:.1} ms",
        ms(answered),
        ms(took)
    );
}
