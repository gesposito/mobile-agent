//! The fixtures the microbenchmarks read (`benches/mirror.rs`, design D14),
//! at the sizes the device reports measured: the 500-cell screens, the Expo
//! list and Bluesky's search screen on Android, and the 1,049-node React
//! Native screen spike S1 read on the API 36 emulator. Their node counts are
//! constants here, so that a fixture that drifts fails a test instead of
//! changing a benchmark's input unnoticed.

mod support;

use mirror::SnapshotOptions;
use mirror::backend::TreeRead;
use protocol::snapshot::{Node, NodeFlags, WindowType};
use support::{fixture, mirror_on};

/// The SwiftUI Big tab: a header, 500 cells and the tab bar.
const GRID_NODES: usize = 510;
/// The Expo list: the status bar, 19 mounted rows with their texts, the tab bar.
const LIST_NODES: usize = 51;
/// Bluesky's search screen on Android with its typeahead and the keyboard.
const BLUESKY_TYPEAHEAD_ANDROID_NODES: usize = 46;
/// The Expo Big tab as spike S1 counted it on the API 36 emulator.
const GRID_RN_NODES: usize = 1_049;
/// Of those, the status bar's; the app's are the other 1,021.
const GRID_RN_STATUS_BAR_NODES: usize = 28;

fn count(node: &Node) -> usize {
    1 + node.children.iter().map(count).sum::<usize>()
}

/// Every node of every window of `read`, walked from the roots.
fn nodes(read: &TreeRead) -> usize {
    read.snapshot
        .windows
        .iter()
        .filter_map(|window| window.root.as_ref())
        .map(count)
        .sum()
}

#[test]
fn the_existing_bench_fixtures_keep_their_node_counts() {
    for (name, expected) in [
        ("grid", GRID_NODES),
        ("list", LIST_NODES),
        ("bluesky-typeahead-android", BLUESKY_TYPEAHEAD_ANDROID_NODES),
    ] {
        let read = fixture(name);
        assert_eq!(nodes(&read), expected, "{name}");
        assert_eq!(read.snapshot.node_count as usize, expected, "{name}");
    }
}

#[test]
fn grid_rn_holds_the_1049_nodes_of_the_expo_500_cell_screen() {
    let read = fixture("grid-rn");
    assert_eq!(nodes(&read), GRID_RN_NODES);
    assert_eq!(read.snapshot.node_count as usize, GRID_RN_NODES);
    assert_eq!(read.scale, 2.625);

    let windows = &read.snapshot.windows;
    assert_eq!(windows.len(), 2, "the status bar and the app");
    let status_bar = windows
        .iter()
        .find(|w| w.kind == WindowType::SYSTEM)
        .and_then(|w| w.root.as_ref())
        .expect("a status bar window");
    assert_eq!(count(status_bar), GRID_RN_STATUS_BAR_NODES);
    let app = windows
        .iter()
        .find(|w| w.kind == WindowType::APPLICATION)
        .expect("an app window");
    assert_eq!(
        app.emits_events,
        Some(true),
        "React Native views send events"
    );
    let root = app.root.as_ref().expect("the app's root");
    assert_eq!(count(root), GRID_RN_NODES - GRID_RN_STATUS_BAR_NODES);

    // Each cell is a clickable view group holding one text view with the
    // cell's number: 1,000 of the app's nodes.
    let strings = &read.snapshot.strings;
    let class = |node: &Node| strings[node.class as usize].as_str();
    let mut cells = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node
            .res_id
            .as_deref()
            .is_some_and(|id| id.starts_with("cell-"))
        {
            cells.push(node);
        }
        stack.extend(node.children.iter());
    }
    assert_eq!(cells.len(), 500);
    for cell in cells {
        let n = &cell.res_id.as_deref().unwrap()["cell-".len()..];
        assert_eq!(class(cell), "android.view.ViewGroup", "cell-{n}");
        assert!(cell.flags.contains(NodeFlags::CLICKABLE), "cell-{n}");
        assert!(cell.flags.contains(NodeFlags::VISIBLE), "cell-{n}");
        assert_eq!(cell.desc.as_deref(), Some(format!("Cell {n}").as_str()));
        let [text] = cell.children.as_slice() else {
            panic!("cell-{n} holds one node: {:?}", cell.children);
        };
        assert_eq!(class(text), "android.widget.TextView", "cell-{n}");
        assert_eq!(text.text.as_deref(), Some(n));
        assert!(text.children.is_empty(), "cell-{n}");
    }
}

#[test]
fn an_interactive_snapshot_of_grid_rn_lists_the_500_cells_refs_or_hints_the_rest() {
    let (_, mut mirror) = mirror_on("grid-rn");
    let snapshot = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    let cells: Vec<&str> = snapshot
        .elements
        .iter()
        .filter_map(|e| e.identifier.as_deref())
        .filter(|id| id.starts_with("cell-"))
        .collect();
    let hinted: usize = snapshot
        .hints
        .iter()
        .filter_map(|h| h.count)
        .map(|count| count as usize)
        .sum();
    assert_eq!(cells.len() + hinted, 500, "{:?}", snapshot.hints);
    assert!(snapshot.elements.iter().all(|e| e.reference.is_some()));

    // The emulator lays the grid out inside the scroll view, so every cell
    // is on screen: the listing holds all of them in order, then the tabs.
    let expected: Vec<String> = (1..=500).map(|n| format!("cell-{n}")).collect();
    assert_eq!(cells, expected);
    assert!(snapshot.hints.is_empty(), "{:?}", snapshot.hints);
    assert_eq!(snapshot.refs().len(), 505, "500 cells and 5 tabs");
    assert!(
        snapshot
            .text
            .starts_with("- button \"Cell 1\" [id=cell-1, ref=e1]\n"),
        "{}",
        &snapshot.text[..80]
    );
    assert!(
        snapshot
            .text
            .ends_with("- button \"System\" [id=tab-system, ref=e505]"),
        "{}",
        snapshot.text
    );
    assert!(!snapshot.verdict.degraded, "{:?}", snapshot.verdict);
}
