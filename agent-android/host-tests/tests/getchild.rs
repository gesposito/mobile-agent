//! Task 4.11: a tree read survives a child the platform fails to return.
//!
//! On a debuggable build such as the API 36 emulator,
//! `AccessibilityNodeInfo.getChild` can throw a `NullPointerException` from
//! the accessibility client's integrity check instead of returning the child.
//! With `--test-hooks`, the agent reads `MOBILE_AGENT_TEST_GET_CHILD_NPE` and
//! throws that exception in place of the child with a given resource id, a
//! given number of times. Thrown once, the child is fetched again and the
//! read is complete; thrown twice, the child is skipped, counted as omitted
//! and the snapshot marked truncated, and the read still succeeds.

use std::collections::HashSet;
use std::time::Duration;

use agent_android_host_tests::agent::fields;
use agent_android_host_tests::client::Client;
use agent_android_host_tests::tree::{self, find, find_all, nodes};
use agent_android_host_tests::{Agent, Device, Launch, device, fixture, poll};
use protocol::message::{Call, Reply, SnapshotParams};
use protocol::snapshot::{Node, Snapshot, Truncation};

/// The cell whose fetch the hook fails.
const TARGET: &str = "cell-7";

/// The environment variable that arms the hook.
const HOOK_VAR: &str = "MOBILE_AGENT_TEST_GET_CHILD_NPE";

/// The tag of the agent's lines in logcat.
const LOG_TAG: &str = "mobile-agent";

fn snapshot(client: &mut Client) -> Snapshot {
    match client.call(Call::Snapshot(SnapshotParams {
        idle_ms: Some(0),
        ..SnapshotParams::default()
    })) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("snapshot failed: {other:?}"),
    }
}

/// An agent launched with `HOOK_VAR` set to fail [`TARGET`] `times` times,
/// with `--test-hooks` when `armed`. The Compose re-read poll is off, so
/// only the test's snapshots meet the hook.
fn hooked(dev: &Device, times: u32, armed: bool) -> Agent {
    let mut launch = Launch::default().arg("--compose-poll-ms").arg("0");
    if armed {
        launch = launch.arg("--test-hooks");
    }
    launch
        .env
        .push((HOOK_VAR.to_owned(), format!("{times}:{TARGET}")));
    Agent::launch(dev, launch)
}

/// The hook's lines: `expected` of them, and no more.
fn hook_lines(agent: &mut Agent, expected: usize) -> Vec<String> {
    let hook = |l: &str| l.starts_with("test-hook ");
    let mut lines = Vec::new();
    while lines.len() < expected {
        let Some(line) = agent.wait_line(Duration::from_secs(2), hook) else {
            panic!(
                "the hook fired {} of {expected} times:\n{}",
                lines.len(),
                agent.transcript()
            );
        };
        lines.push(line);
    }
    let extra = agent.wait_line(Duration::from_millis(300), hook);
    assert_eq!(extra, None, "the hook fired more than {expected} times");
    lines
}

/// The agent's debug lines in logcat, once one of them contains `needle`.
fn debug_lines(dev: &Device, pid: u32, needle: &str) -> Vec<String> {
    let tagged = format!(" D {LOG_TAG}: ");
    poll(Duration::from_secs(3), Duration::from_millis(100), || {
        let lines: Vec<String> = dev
            .shell(&format!("logcat -d --pid={pid} -s {LOG_TAG}:D"))
            .lines()
            .filter(|l| l.contains(&tagged))
            .map(str::to_owned)
            .collect();
        lines.iter().any(|l| l.contains(needle)).then_some(lines)
    })
    .unwrap_or_else(|| panic!("no debug line of pid {pid} in logcat contains {needle:?}"))
}

/// The resource ids of the Big screen's cells in `s`.
fn cells(s: &Snapshot) -> HashSet<String> {
    find_all(s, "cell-")
        .iter()
        .filter_map(|f| tree::resource_id(s, f.node))
        .collect()
}

fn all_cells() -> HashSet<String> {
    (1..=500).map(|n| format!("cell-{n}")).collect()
}

/// Nodes in the subtree of `n`, itself included.
fn size(n: &Node) -> usize {
    1 + n.children.iter().map(size).sum::<usize>()
}

/// Nodes serialized in window `window`.
fn in_window(s: &Snapshot, window: u32) -> usize {
    nodes(s)
        .iter()
        .filter(|(f, _)| f.window.id == window)
        .count()
}

fn emits_events(s: &Snapshot, window: u32) -> Option<bool> {
    s.windows
        .iter()
        .find(|w| w.id == window)
        .expect("the window of the target")
        .emits_events
}

fn fetched_again(package: &str) {
    let Some(dev) = device(&format!("fetched_again {package}")) else {
        return;
    };
    fixture::launch(&dev, package, "big");
    let mut agent = hooked(&dev, 1, true);
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    let fired = hook_lines(&mut agent, 1);
    assert_eq!(fields(&fired[0])["id"], TARGET, "{fired:?}");

    assert_eq!(s.truncated, Truncation::EMPTY, "{}", s.truncated);
    assert_eq!(s.node_count as usize, tree::count(&s), "nodes omitted");
    assert_eq!(cells(&s), all_cells());
    let log = debug_lines(&dev, agent.pid, "on the second attempt");
    assert!(
        log.iter().any(|l| l.contains("fetching it again")),
        "{log:#?}"
    );
    println!("{package}: {}", log.join("\n"));
}

fn skipped(package: &str) {
    let Some(dev) = device(&format!("skipped {package}")) else {
        return;
    };
    fixture::launch(&dev, package, "big");
    let mut agent = hooked(&dev, 2, true);
    let mut client = agent.connect();
    let partial = snapshot(&mut client);
    // The hook is spent, so the next read is complete.
    let full = snapshot(&mut client);
    let fired = hook_lines(&mut agent, 2);
    // The second throw is the retry of the same child.
    let (first, second) = (fields(&fired[0]), fields(&fired[1]));
    assert_eq!(first["id"], TARGET, "{fired:?}");
    assert_eq!(second["id"], TARGET, "{fired:?}");
    assert_eq!(first["index"], second["index"], "{fired:?}");

    assert_eq!(full.truncated, Truncation::EMPTY, "{}", full.truncated);
    assert_eq!(cells(&full), all_cells());
    let (target, _) = find(&full, TARGET).expect("the target in the complete read");
    let window = target.window.id;

    assert_eq!(
        partial.truncated,
        Truncation::WINDOWS,
        "{}",
        partial.truncated
    );
    assert_eq!(
        partial.node_count as usize,
        tree::count(&partial) + 1,
        "the skipped child is the one node omitted"
    );
    let mut others = all_cells();
    others.remove(TARGET);
    assert_eq!(cells(&partial), others);
    assert_eq!(
        in_window(&full, window),
        in_window(&partial, window) + size(target.node),
        "the window lost other nodes than the target's subtree"
    );
    // A window whose walk skipped a child cannot tell whether it sends
    // change events, unless the walk met Compose, which never does.
    let expected = match emits_events(&full, window) {
        Some(false) => Some(false),
        _ => None,
    };
    assert_eq!(emits_events(&partial, window), expected);
    let log = debug_lines(&dev, agent.pid, "skipped");
    assert!(
        log.iter().any(|l| l.contains("fetching it again")),
        "{log:#?}"
    );
    println!(
        "{package}: {} of {} nodes serialized, {} lost with {TARGET}\n{}",
        tree::count(&partial),
        partial.node_count,
        size(target.node),
        log.join("\n")
    );
}

#[test]
fn a_compose_child_whose_fetch_throws_once_is_fetched_again_and_the_read_is_complete() {
    fetched_again(fixture::COMPOSE);
}

#[test]
fn an_expo_child_whose_fetch_throws_once_is_fetched_again_and_the_read_is_complete() {
    fetched_again(fixture::EXPO);
}

#[test]
fn a_compose_child_whose_fetch_throws_twice_is_skipped_and_counted_as_omitted_in_a_truncated_read()
{
    skipped(fixture::COMPOSE);
}

#[test]
fn an_expo_child_whose_fetch_throws_twice_is_skipped_and_counted_as_omitted_in_a_truncated_read() {
    skipped(fixture::EXPO);
}

#[test]
fn the_hook_variable_arms_nothing_without_the_test_hooks_flag() {
    let Some(dev) = device("the_hook_variable_arms_nothing_without_the_test_hooks_flag") else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "big");
    let mut agent = hooked(&dev, 2, false);
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    hook_lines(&mut agent, 0);
    assert_eq!(s.truncated, Truncation::EMPTY, "{}", s.truncated);
    assert_eq!(cells(&s), all_cells());
}
