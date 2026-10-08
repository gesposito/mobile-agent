//! The inputs of the codec benchmarks (`benches/codec.rs`, design D14) keep
//! their shapes and their bytes: the node counts, the event split and the
//! request mix, every message's round trip, and each dataset's encoded length
//! and digest, so a change to the generator that changes what the benchmarks
//! time fails here instead of moving their numbers silently.

#[path = "../benches/support/generate.rs"]
mod generate;

use std::collections::BTreeSet;

use protocol::Message;
use protocol::event::EventPayload;
use protocol::message::{Call, Method, Reply, Response};
use protocol::snapshot::{Node, NodeFlags, ScrollAxis, ScrollUnit, Snapshot, WindowType};

/// Each dataset's messages encoded back to back: their length in bytes and
/// their FNV-1a digest, recorded from the generator's first green run. A
/// deliberate change to the generator records them again.
const PINNED: [(&str, usize, u64); 4] = [
    ("snapshot-500", 0, 0),
    ("snapshot-1049", 0, 0),
    ("events-10k", 0, 0),
    ("requests-1k", 0, 0),
];

fn snapshot(message: &Message) -> &Snapshot {
    match message {
        Message::Response(Response {
            method: Method::SNAPSHOT,
            result: Ok(Reply::Snapshot(snapshot)),
            ..
        }) => snapshot,
        other => panic!("not a snapshot response: kind {}", other.kind()),
    }
}

/// Calls `f` with every node under `node` and its depth, the window root's
/// being 0, in pre-order.
fn visit<'a>(node: &'a Node, depth: usize, f: &mut impl FnMut(&'a Node, usize)) {
    f(node, depth);
    for child in &node.children {
        visit(child, depth + 1, f);
    }
}

/// Every node of `snapshot` with its depth, window by window.
fn nodes(snapshot: &Snapshot) -> Vec<(&Node, usize)> {
    let mut out = Vec::new();
    for root in snapshot.windows.iter().filter_map(|w| w.root.as_ref()) {
        visit(root, 0, &mut |node, depth| out.push((node, depth)));
    }
    out
}

fn class<'a>(snapshot: &'a Snapshot, node: &Node) -> &'a str {
    &snapshot.strings[node.class as usize]
}

/// The messages encoded back to back, as the benchmarks encode them.
fn encoded(messages: &[Message]) -> Vec<u8> {
    let mut out = Vec::new();
    for message in messages {
        message.encode(&mut out);
    }
    out
}

/// 64-bit FNV-1a.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn the_500_node_snapshot_has_s5s_shape() {
    let message = generate::snapshot_500();
    let snap = snapshot(&message);
    assert_eq!(snap.node_count, 500);
    let kinds: Vec<WindowType> = snap.windows.iter().map(|w| w.kind).collect();
    assert_eq!(kinds, [WindowType::APPLICATION], "one application window");

    let nodes = nodes(snap);
    assert_eq!(nodes.len(), 500, "nodes in the tree");
    assert_eq!(nodes.iter().map(|&(_, depth)| depth).max(), Some(7));
    let ids: BTreeSet<u32> = nodes.iter().map(|(n, _)| n.id).collect();
    assert_eq!(ids, (0..500).collect(), "ids unique and numbered from 0");

    // S5's string table: 14 view classes and 2 resource-id prefixes, each used.
    let classes: BTreeSet<u32> = nodes.iter().map(|(n, _)| n.class).collect();
    let prefixes: BTreeSet<u32> = nodes.iter().filter_map(|(n, _)| n.res_pkg).collect();
    assert_eq!(snap.strings.len(), 16);
    assert_eq!((classes.len(), prefixes.len()), (14, 2));
    assert!(classes.is_disjoint(&prefixes));
    for &prefix in &prefixes {
        assert!(snap.strings[prefix as usize].ends_with(":id/"));
    }

    // S5's proportions: resource ids on about 55% of the nodes, text on half,
    // descriptions on about 12%.
    let count = |field: fn(&Node) -> bool| nodes.iter().filter(|(n, _)| field(n)).count();
    let res_ids = count(|n| n.res_id.is_some());
    assert!((265..=285).contains(&res_ids), "{res_ids} resource ids");
    assert_eq!(count(|n| n.text.is_some()), 250);
    let descs = count(|n| n.desc.is_some());
    assert!((55..=65).contains(&descs), "{descs} descriptions");
    assert!(count(|n| n.flags.contains(NodeFlags::VISIBLE)) > 0);

    // One node scrolls, the list: an item range over its rows, with the rows
    // inside its bounds in view and more past both ends.
    let scrolling: Vec<&Node> = nodes
        .iter()
        .filter(|(n, _)| n.scroll.is_some())
        .map(|&(n, _)| n)
        .collect();
    assert_eq!(scrolling.len(), 1, "nodes with scroll semantics");
    let list = scrolling[0];
    let scroll = list.scroll.expect("filtered on it");
    assert_eq!(scroll.axis, ScrollAxis::VERTICAL);
    let range = scroll.range.expect("the list reports its range");
    let last_row = list.children.len() as i32 - 1;
    assert_eq!(
        (range.unit, range.min, range.max),
        (ScrollUnit::ITEMS, 0, last_row)
    );
    let position = scroll.position.expect("the list reports the rows in view");
    assert!(
        0 < position.first && position.first <= position.last && position.last < range.max,
        "rows {}..={} in view of 0..={}",
        position.first,
        position.last,
        range.max
    );

    // Text in 1-, 2-, 3- and 4-byte UTF-8.
    let widths: BTreeSet<usize> = nodes
        .iter()
        .flat_map(|(n, _)| [&n.res_id, &n.text, &n.desc])
        .flatten()
        .flat_map(|s| s.chars().map(char::len_utf8))
        .collect();
    assert_eq!(widths, BTreeSet::from([1, 2, 3, 4]));
}

#[test]
fn the_1049_node_snapshot_has_the_react_native_shape_of_the_expo_grid() {
    let message = generate::snapshot_1049();
    let snap = snapshot(&message);
    assert_eq!(snap.node_count, 1049);
    let nodes = nodes(snap);
    assert_eq!(nodes.len(), 1049, "nodes in the tree");

    // As the agent read the Expo fixture's Big screen on the emulator (spike
    // S1): the status bar's 28 nodes, then the app's 1,021, 11 levels deep
    // below a window root and all but 2 visible.
    let per_window: Vec<(WindowType, usize)> = snap
        .windows
        .iter()
        .map(|w| {
            let mut count = 0;
            if let Some(root) = &w.root {
                visit(root, 0, &mut |_, _| count += 1);
            }
            (w.kind, count)
        })
        .collect();
    assert_eq!(
        per_window,
        [(WindowType::SYSTEM, 28), (WindowType::APPLICATION, 1021)]
    );
    assert_eq!(nodes.iter().map(|&(_, depth)| depth).max(), Some(11));
    let visible = nodes
        .iter()
        .filter(|(n, _)| n.flags.contains(NodeFlags::VISIBLE))
        .count();
    assert_eq!(visible, 1047);
    let ids: BTreeSet<u32> = nodes.iter().map(|(n, _)| n.id).collect();
    assert_eq!(ids, (0..1049).collect(), "ids unique and numbered from 0");

    // One grid container holds the 500 cells, each a button named by its
    // label and test id that holds one text; the other 49 nodes are chrome.
    let grids: Vec<&Node> = nodes
        .iter()
        .filter(|(n, _)| n.children.len() == 500)
        .map(|&(n, _)| n)
        .collect();
    assert_eq!(grids.len(), 1, "grid containers");
    for (i, cell) in grids[0].children.iter().enumerate() {
        let n = i + 1;
        assert_eq!(class(snap, cell), "android.widget.Button", "cell {n}");
        assert_eq!(cell.res_pkg, None, "cell {n}");
        assert_eq!(cell.res_id.as_deref(), Some(format!("cell-{n}").as_str()));
        assert_eq!(cell.desc.as_deref(), Some(format!("Cell {n}").as_str()));
        assert_eq!(cell.text, None, "cell {n}");
        assert_eq!(cell.children.len(), 1, "cell {n}");
        let label = &cell.children[0];
        assert_eq!(class(snap, label), "android.widget.TextView", "cell {n}");
        assert_eq!(label.text.as_deref(), Some(n.to_string().as_str()));
        assert!(label.children.is_empty(), "cell {n}");
    }
    let cell_nodes = grids[0].children.len() * 2;
    assert_eq!(nodes.len() - cell_nodes, 49, "chrome");
}

#[test]
fn the_event_run_splits_like_s5s_and_numbers_each_stream_from_one() {
    let events = generate::events_10k();
    assert_eq!(events.len(), 10_000);
    // Per stream, the last sequence number: tree, window.
    let mut seq = [0u64; 2];
    let mut time_us = 1u64 << 32;
    for message in &events {
        let Message::Event(event) = message else {
            panic!("not an event: kind {}", message.kind());
        };
        assert!(
            event.time_us > time_us,
            "time {} after {time_us}",
            event.time_us
        );
        time_us = event.time_us;
        let stream = match &event.payload {
            EventPayload::Tree(change) => {
                assert!(
                    (2..=10).contains(&change.path.len()),
                    "path {:?}",
                    change.path
                );
                0
            }
            EventPayload::Window(_) => 1,
            other => panic!("an event on another stream: {other:?}"),
        };
        seq[stream] += 1;
        assert_eq!(event.seq, seq[stream], "stream {stream}");
    }
    assert_eq!(seq, [8_975, 1_025], "tree and window events");
}

#[test]
fn the_request_run_has_s5s_mix() {
    let requests = generate::requests_1k();
    assert_eq!(requests.len(), 1_000);
    let (mut taps, mut texts, mut snapshots, mut deadlines) = (0, 0, 0, 0);
    let mut ids = BTreeSet::new();
    for message in &requests {
        let Message::Request(request) = message else {
            panic!("not a request: kind {}", message.kind());
        };
        ids.insert(request.id);
        deadlines += usize::from(request.deadline_ms.is_some());
        match &request.call {
            Call::Tap(_) => taps += 1,
            Call::TextSet(params) => {
                assert!(!params.text.is_empty(), "request {}", request.id);
                texts += 1;
            }
            Call::Snapshot(_) => snapshots += 1,
            other => panic!("a {} request", other.method()),
        }
    }
    assert_eq!((taps, texts, snapshots), (595, 198, 207));
    assert_eq!(deadlines, 691, "requests with a deadline");
    assert_eq!(ids, (1..=1_000).collect(), "request ids");
}

#[test]
fn every_generated_message_round_trips() {
    let mut checked = 0;
    for (name, messages) in generate::datasets() {
        for (i, message) in messages.iter().enumerate() {
            let bytes = message.to_vec();
            let decoded = Message::decode(&bytes)
                .unwrap_or_else(|e| panic!("{name} message {i} does not decode: {e}"));
            assert!(decoded == *message, "{name} message {i} decodes to another");
            checked += 1;
        }
    }
    assert_eq!(checked, 1 + 1 + 10_000 + 1_000, "messages checked");
}

#[test]
fn the_encoded_bytes_are_pinned() {
    let datasets = generate::datasets();
    assert!(
        datasets == generate::datasets(),
        "a second call generates other inputs"
    );
    let names: Vec<&str> = datasets.iter().map(|&(name, _)| name).collect();
    assert_eq!(names, PINNED.map(|(name, ..)| name));
    for ((name, messages), (_, len, digest)) in datasets.iter().zip(PINNED) {
        let bytes = encoded(messages);
        assert_eq!(bytes.len(), len, "{name}: encoded length");
        assert_eq!(fnv1a(&bytes), digest, "{name}: digest of its bytes");
    }
}
