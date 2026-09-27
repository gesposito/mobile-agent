//! Task 4.5: change events with subtree paths and per-stream sequences, and
//! the full re-read of windows hosting AndroidComposeView, which send no
//! events to UI automation (spec android-driver, Change events; design D5).

use std::time::Duration;

use agent_android_host_tests::agent::fields;
use agent_android_host_tests::client::Client;
use agent_android_host_tests::tree::{self, center, find};
use agent_android_host_tests::{Agent, Launch, device, fixture};
use protocol::event::{Event, EventPayload, Stream, TreeChangeKind, WindowChangeKind};
use protocol::message::{Call, Reply, ResyncParams, SnapshotParams};
use protocol::session::HostEvent;
use protocol::snapshot::{Snapshot, WindowType};

fn snapshot(client: &mut Client) -> Snapshot {
    match client.call(Call::Snapshot(SnapshotParams {
        idle_ms: Some(0),
        ..SnapshotParams::default()
    })) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("snapshot failed: {other:?}"),
    }
}

fn app_window(s: &Snapshot) -> u32 {
    s.windows
        .iter()
        .find(|w| w.kind == WindowType::APPLICATION)
        .expect("an app window")
        .id
}

fn tree_events(events: &[Event], window: u32) -> Vec<(TreeChangeKind, Vec<u32>)> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Tree(t) if t.window == window => Some((t.change, t.path.clone())),
            _ => None,
        })
        .collect()
}

fn assert_in_sync(client: &Client) {
    let gaps: Vec<&HostEvent> = client
        .notices
        .iter()
        .filter(|n| matches!(n, HostEvent::OutOfSync { .. }))
        .collect();
    assert!(gaps.is_empty(), "sequence gaps: {gaps:?}");
    for stream in [Stream::TREE, Stream::WINDOW, Stream::LIFECYCLE] {
        let seqs: Vec<u64> = client.events_on(stream).iter().map(|e| e.seq).collect();
        let expected: Vec<u64> = (1..=seqs.len() as u64).collect();
        assert_eq!(seqs, expected, "{stream} stream numbering");
    }
}

#[test]
fn editing_one_react_native_field_emits_events_naming_that_field_only() {
    let Some(dev) = device("editing_one_react_native_field_emits_events_naming_that_field_only")
    else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "login");
    let mut agent = Agent::launch(&dev, Launch::default().arg("--trace"));
    let mut client = agent.connect();
    let before = snapshot(&mut client);
    let app = app_window(&before);
    let (field, field_path) = find(&before, "login-username").expect("login-username");
    let (x, y) = center(field.node.bounds);
    dev.shell(&format!("input tap {x} {y}"));
    // Let focusing and the keyboard settle, then edit the field alone.
    client.idle(Duration::from_millis(1500));
    let _ = snapshot(&mut client);
    let seen = client.events.len();
    dev.shell("input text hello");
    client.idle(Duration::from_millis(1200));
    let during: Vec<Event> = client.events[seen..]
        .iter()
        .map(|(_, e)| e.clone())
        .collect();
    let changes = tree_events(&during, app);
    fixture::hide_keyboard(&dev);
    println!("events while typing into login-username at {field_path:?}: {changes:?}");
    assert!(
        changes.iter().any(|(_, p)| p.starts_with(&field_path)),
        "no event names the field's subtree {field_path:?}: {changes:?}"
    );
    for (kind, path) in &changes {
        assert!(
            path.starts_with(&field_path),
            "a {kind} event names {path:?}, outside the field's subtree {field_path:?}"
        );
    }
    let window_states: Vec<&Event> = during
        .iter()
        .filter(|e| {
            matches!(&e.payload, EventPayload::Window(w)
            if w.window == app && w.change == WindowChangeKind::STATE)
        })
        .collect();
    assert!(window_states.is_empty(), "{window_states:?}");
    // No full-tree refresh: the next read comes from the platform cache.
    let _ = agent.wait_line(Duration::from_millis(100), |_| false);
    let after = snapshot(&mut client);
    let line = agent
        .wait_line(Duration::from_secs(2), |l| {
            l.starts_with("trace snapshot ")
                && l.contains(&format!("windows={}", after.windows.len()))
        })
        .expect("trace line");
    let trips: i64 = fields(&line)["round_trips"].parse().expect("round trips");
    assert!(
        trips <= after.windows.len() as i64 + 6,
        "the read after the edit took {trips} round trips: {line}"
    );
    let (edited, _) = find(&after, "login-username").expect("login-username");
    assert_eq!(edited.node.text.as_deref(), Some("hello"));
    assert_in_sync(&client);
}

#[test]
fn a_keyboard_appearing_is_reported_on_the_window_stream() {
    let Some(dev) = device("a_keyboard_appearing_is_reported_on_the_window_stream") else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let before = snapshot(&mut client);
    let (field, _) = find(&before, "login-username").expect("login-username");
    let (x, y) = center(field.node.bounds);
    dev.shell(&format!("input tap {x} {y}"));
    let added = client.wait_event(
        Duration::from_secs(5),
        |e| matches!(&e.payload, EventPayload::Window(w) if w.change == WindowChangeKind::ADDED),
    );
    let s = snapshot(&mut client);
    fixture::hide_keyboard(&dev);
    let added = added.expect("no window added event after focusing a field");
    let EventPayload::Window(w) = added.payload else {
        unreachable!()
    };
    let ime = s
        .windows
        .iter()
        .find(|x| x.kind == WindowType::INPUT_METHOD)
        .expect("keyboard window");
    let ids: Vec<u32> = s.windows.iter().map(|x| x.id).collect();
    assert!(
        client.events_on(Stream::WINDOW).iter().any(|e| matches!(
            &e.payload, EventPayload::Window(x) if x.window == ime.id
        )),
        "no window event names the keyboard window {} (added {}, windows {ids:?})",
        ime.id,
        w.window
    );
    assert_in_sync(&client);
}

#[test]
fn a_compose_window_is_reread_on_a_poll_and_its_change_reported() {
    let Some(dev) = device("a_compose_window_is_reread_on_a_poll_and_its_change_reported") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let mut agent = Agent::launch(&dev, Launch::default().arg("--trace"));
    let mut client = agent.connect();
    let before = snapshot(&mut client);
    let app = app_window(&before);
    let (tab, _) = find(&before, "tab-list").expect("tab-list");
    let (x, y) = center(tab.node.bounds);
    let seen = client.events.len();
    // A change the agent did not cause: only the poll can find it.
    dev.shell(&format!("input tap {x} {y}"));
    let changed = client.wait_event(
        Duration::from_secs(5),
        |e| matches!(&e.payload, EventPayload::Tree(t) if t.window == app),
    );
    let reread = agent.wait_line(Duration::from_secs(1), |l| l.starts_with("trace compose "));
    let changed = changed.unwrap_or_else(|| {
        panic!(
            "no tree event for the Compose window within 5 s; events {:?}\n{}",
            &client.events[seen..],
            agent.transcript()
        )
    });
    println!("poll event: {changed:?}; {}", reread.unwrap_or_default());
    let after = snapshot(&mut client);
    assert!(
        find(&after, "row-1").is_some(),
        "the List tab is not in the tree"
    );
    assert!(
        find(&after, "login-username").is_none(),
        "the Login tab is still in the tree"
    );
    let EventPayload::Tree(t) = changed.payload else {
        unreachable!()
    };
    let node = tree::at_path(&after, app, &t.path).expect("the event's path resolves");
    println!(
        "changed subtree at {:?}: {}",
        t.path,
        tree::class(&after, node)
    );
    assert_in_sync(&client);
}

#[test]
fn a_tree_resync_answers_the_last_sequence_and_the_next_read_is_full() {
    let Some(dev) = device("a_tree_resync_answers_the_last_sequence_and_the_next_read_is_full")
    else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let mut agent = Agent::launch(
        &dev,
        Launch::default()
            .arg("--trace")
            .arg("--compose-poll-ms")
            .arg("0"),
    );
    let mut client = agent.connect();
    let before = snapshot(&mut client);
    let (tab, _) = find(&before, "tab-big").expect("tab-big");
    let (x, y) = center(tab.node.bounds);
    dev.shell(&format!("input tap {x} {y}"));
    client.idle(Duration::from_millis(800));
    let reply = client
        .call(Call::Resync(ResyncParams {
            stream: Stream::TREE,
        }))
        .expect("resync");
    let Reply::Resync(ack) = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(ack.stream, Stream::TREE);
    assert_eq!(ack.seq, client.events_on(Stream::TREE).len() as u64);
    agent.drain_lines();
    let after = snapshot(&mut client);
    assert_eq!(
        tree::find_all(&after, "cell-").len(),
        500,
        "the read after the resync is not the Big tab"
    );
    let line = agent
        .wait_line(Duration::from_secs(2), |l| l.starts_with("trace snapshot "))
        .expect("trace");
    let trips: i64 = fields(&line)["round_trips"].parse().expect("round trips");
    assert!(trips >= 100, "the read after the resync was warm: {line}");
    println!("{line}");
}
