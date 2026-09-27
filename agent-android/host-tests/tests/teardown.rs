//! Task 10.9: a tree read that meets a torn-down subtree, and a snapshot
//! asked for while the agent's Compose poll re-reads.
//!
//! On a debuggable build such as the API 36 emulator, `getChild` of a child
//! that went away throws a `NullPointerException` from the accessibility
//! client's integrity check: the app answers the walk's prefetching request
//! for a node it no longer has with a list holding only null. A release
//! build returns null there. A tap that switches the Compose fixture away
//! from its Big screen while a walk is among the 500 cells leaves the walk
//! hundreds of such children to fetch, and every tree read runs on one
//! worker, so a snapshot the host asks for meanwhile waits for the walk.
//!
//! The agent's hooks (`--test-hooks`) set the scene:
//! `MOBILE_AGENT_TEST_PAUSE_AT_CHILD` holds a walk at `cell-100` while the
//! test taps the Login tab, so every cell after it is gone when the walk
//! goes on; `MOBILE_AGENT_TEST_GET_CHILD_AS_RELEASE` turns what the platform
//! throws into the null a release build returns; and
//! `MOBILE_AGENT_TEST_GET_CHILD_NPE` with `cell-*` makes every cell throw
//! while the screen stays as it is.

use std::time::{Duration, Instant};

use agent_android_host_tests::agent::fields;
use agent_android_host_tests::client::Client;
use agent_android_host_tests::tree::{self, center, find, find_all};
use agent_android_host_tests::{Agent, Device, Launch, device, fixture, ms, poll};
use protocol::event::EventPayload;
use protocol::message::{Call, InputAck, Reply, SnapshotParams, TapParams};
use protocol::snapshot::{Snapshot, Truncation};

/// The environment variable that makes children throw.
const GET_CHILD_NPE: &str = "MOBILE_AGENT_TEST_GET_CHILD_NPE";

/// The environment variable that holds a walk at a child.
const PAUSE_AT_CHILD: &str = "MOBILE_AGENT_TEST_PAUSE_AT_CHILD";

/// The environment variable that makes `getChild` answer as on a release build.
const GET_CHILD_AS_RELEASE: &str = "MOBILE_AGENT_TEST_GET_CHILD_AS_RELEASE";

/// The driver's deadline for a snapshot (`ReadOptions::default` in
/// `crates/android`), which the 10.1 run's read missed.
const DEADLINE: Duration = Duration::from_secs(10);

/// The cell a walk waits at: the 100th of 500, so 400 are left to fetch.
const PAUSE_AT: &str = "cell-100";

/// How long a walk waits there: time for a tap and the new screen.
const PAUSE: Duration = Duration::from_millis(1000);

fn no_wait() -> SnapshotParams {
    SnapshotParams {
        idle_ms: Some(0),
        ..SnapshotParams::default()
    }
}

fn snapshot(client: &mut Client, params: SnapshotParams) -> Snapshot {
    match client.call(Call::Snapshot(params)) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("snapshot failed: {other:?}"),
    }
}

/// The snapshot `reply` carries, or a panic naming how long it took.
fn answered(reply: Result<Reply, protocol::Error>, took: Duration) -> Snapshot {
    match reply {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("the snapshot failed after {:.0} ms: {other:?}", ms(took)),
    }
}

/// An agent on the Compose fixture's Big screen, tracing, with the hooks
/// `env` arms; the Compose poll re-reads only when `rereads`. The fixture is
/// in front before the agent starts, since the hooks count the walks that
/// fetch the cells from the first read on.
fn agent_on_big(dev: &Device, env: &[(&str, String)], rereads: bool) -> Agent {
    fixture::launch(dev, fixture::COMPOSE, "big");
    let front = poll(Duration::from_secs(5), Duration::from_millis(100), || {
        dev.shell("dumpsys activity activities | grep -m1 topResumedActivity")
            .contains(fixture::COMPOSE)
            .then_some(())
    });
    assert!(front.is_some(), "the Compose fixture is not in front");
    let mut launch = Launch::default().arg("--test-hooks").arg("--trace");
    if !rereads {
        launch = launch.arg("--compose-poll-ms").arg("0");
    }
    for (name, value) in env {
        launch.env.push(((*name).to_owned(), value.clone()));
    }
    Agent::launch(dev, launch)
}

/// Panics, saying what `s` shows, unless it is the Big screen.
fn big_screen(s: &Snapshot, agent: &Agent) {
    let cells = find_all(s, "cell-").len();
    if cells != 500 {
        let windows: Vec<String> = s
            .windows
            .iter()
            .map(|w| format!("{} {:?}", w.id, tree::package(s, w)))
            .collect();
        panic!(
            "the first read is not the Big screen: {cells} cells, windows {windows:?}\n{}",
            agent.transcript()
        );
    }
}

/// Holds the second walk that fetches [`PAUSE_AT`] there for `wait`.
fn pause_at_second_fetch(wait: Duration) -> (&'static str, String) {
    (PAUSE_AT_CHILD, format!("2:{}:{PAUSE_AT}", wait.as_millis()))
}

/// Taps the center of the node with resource id `id` in `s` through the
/// agent, whose input runs beside the tree reads.
fn tap(client: &mut Client, s: &Snapshot, id: &str) {
    let (node, _) = find(s, id).unwrap_or_else(|| panic!("no {id} in the snapshot"));
    let (x, y) = center(node.node.bounds);
    match client.call(Call::Tap(TapParams {
        x,
        y,
        duration_ms: None,
    })) {
        Ok(Reply::Input(InputAck { accepted: true })) => {}
        other => panic!("the tap on {id} failed: {other:?}"),
    }
}

/// The agent's next stdout line that starts with `prefix`, skipping others.
fn line(agent: &mut Agent, prefix: &str, timeout: Duration) -> String {
    agent
        .wait_line(timeout, |l| l.starts_with(prefix))
        .unwrap_or_else(|| {
            panic!(
                "no line starting {prefix:?} within {timeout:?}:\n{}",
                agent.transcript()
            )
        })
}

/// Like [`line`], serving the connection meanwhile, so that the agent,
/// which closes a connection after 3 s without a word from the host, hears
/// the answers to its pings during a long wait.
fn line_serving(client: &mut Client, agent: &mut Agent, prefix: &str, timeout: Duration) -> String {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if let Some(found) = agent.wait_line(Duration::from_millis(50), |l| l.starts_with(prefix)) {
            return found;
        }
        client.idle(Duration::from_millis(50));
    }
    panic!(
        "no line starting {prefix:?} within {timeout:?}:\n{}",
        agent.transcript()
    )
}

/// Every cell of the Big screen throws, as a child the platform fails to
/// return does, while the screen stays as it is: the read spends its retry
/// budget on the first cells and skips the others at once, so it is
/// answered well within its deadline, where waiting 50 ms before fetching
/// each of the 500 cells again takes 25 s.
#[test]
fn a_read_whose_every_cell_throws_retries_within_its_budget_and_skips_the_rest() {
    let Some(dev) =
        device("a_read_whose_every_cell_throws_retries_within_its_budget_and_skips_the_rest")
    else {
        return;
    };
    let mut agent = agent_on_big(&dev, &[(GET_CHILD_NPE, "100000:cell-*".to_owned())], false);
    let mut client = agent.connect();
    let asked = Instant::now();
    let reply = client.call_within(
        Call::Snapshot(no_wait()),
        Some(DEADLINE),
        Duration::from_secs(30),
    );
    let took = asked.elapsed();
    let s = answered(reply, took);
    let trace = line(&mut agent, "trace snapshot ", Duration::from_secs(2));
    let log = agent.debug_lines("are spent");
    let retried = log
        .iter()
        .filter(|l| l.contains("fetching it again"))
        .count();
    let spent = log.iter().filter(|l| l.contains("are spent")).count();
    println!(
        "every cell throwing: answered after {:.0} ms, {retried} cells fetched again; {trace}",
        ms(took)
    );
    assert!(
        find_all(&s, "cell-").is_empty(),
        "a cell was serialized although every cell threw"
    );
    assert_eq!(s.truncated, Truncation::WINDOWS, "{}", s.truncated);
    assert_eq!(
        s.node_count as usize,
        tree::count(&s) + 500,
        "the 500 cells are counted as omitted"
    );
    assert_eq!(
        retried,
        2,
        "cells fetched again within the read's budget; first lines: {:#?}",
        &log[..log.len().min(6)]
    );
    assert_eq!(spent, 1, "the spent budget is logged once per read");
    assert!(
        took < Duration::from_secs(3),
        "the read took {:.0} ms",
        ms(took)
    );
}

/// A snapshot's walk waits at [`PAUSE_AT`] while a tap switches the fixture
/// to its Login tab, so every cell after it is gone when the walk goes on:
/// the read finds the cells' parent gone, starts again from the window
/// roots, and answers with the Login screen, whole, soon after the wait.
/// With `release`, `getChild` answers as a release build does, with null
/// instead of throwing.
fn torn_down_under_a_snapshot(test: &str, release: bool) {
    let Some(dev) = device(test) else {
        return;
    };
    let mut env = vec![pause_at_second_fetch(PAUSE)];
    if release {
        env.push((GET_CHILD_AS_RELEASE, "1".to_owned()));
    }
    let mut agent = agent_on_big(&dev, &env, false);
    let mut client = agent.connect();
    // The first read fetches cell-100 once; the second, which evicts the
    // Compose window's cache so that the cells come from the app, waits there.
    let big = snapshot(&mut client, no_wait());
    big_screen(&big, &agent);
    let asked = Instant::now();
    let id = client
        .send(
            Call::Snapshot(SnapshotParams {
                evict_event_less: Some(true),
                ..no_wait()
            }),
            Some(DEADLINE),
        )
        .expect("send the snapshot");
    let paused = line_serving(
        &mut client,
        &mut agent,
        "test-hook pause ",
        Duration::from_secs(10),
    );
    tap(&mut client, &big, "tab-login");
    let tapped = asked.elapsed();
    let reply = client.wait(id, Duration::from_secs(30));
    let took = asked.elapsed();
    let s = answered(reply, took);
    let trace = line(&mut agent, "trace snapshot ", Duration::from_secs(2));
    println!(
        "{paused}; Login tapped {:.0} ms after the snapshot was asked for, answered after {:.0} ms; \
         {trace}",
        ms(tapped),
        ms(took)
    );
    assert!(
        find(&s, "login-username").is_some(),
        "the read shows the screen from before the tap: {} cells, truncated {}",
        find_all(&s, "cell-").len(),
        s.truncated
    );
    assert!(find_all(&s, "cell-").is_empty(), "cells of the Big screen");
    assert_eq!(s.truncated, Truncation::EMPTY, "{}", s.truncated);
    assert_eq!(s.node_count as usize, tree::count(&s), "nodes omitted");
    assert_eq!(
        fields(&trace).get("restarts").map(String::as_str),
        Some("1"),
        "{trace}"
    );
    assert!(
        took < PAUSE + Duration::from_secs(2),
        "answered {:.0} ms after it was asked for",
        ms(took)
    );
}

#[test]
fn a_snapshot_whose_walk_meets_a_torn_down_subtree_reads_the_new_screen_from_the_root() {
    torn_down_under_a_snapshot(
        "a_snapshot_whose_walk_meets_a_torn_down_subtree_reads_the_new_screen_from_the_root",
        false,
    );
}

#[test]
fn a_snapshot_whose_walk_meets_a_torn_down_subtree_as_a_release_build_reads_the_new_screen() {
    torn_down_under_a_snapshot(
        "a_snapshot_whose_walk_meets_a_torn_down_subtree_as_a_release_build_reads_the_new_screen",
        true,
    );
}

/// While the agent's Compose poll re-read waits at [`PAUSE_AT`] for five
/// seconds, standing for a long walk, a tap switches to Login and the host
/// asks for a snapshot: the re-read yields to it, so the snapshot is
/// answered at once rather than after the re-read, and shows Login.
#[test]
fn a_snapshot_asked_for_during_a_compose_reread_is_answered_without_waiting_for_it() {
    let Some(dev) =
        device("a_snapshot_asked_for_during_a_compose_reread_is_answered_without_waiting_for_it")
    else {
        return;
    };
    let long = Duration::from_secs(5);
    let mut agent = agent_on_big(&dev, &[pause_at_second_fetch(long)], true);
    let mut client = agent.connect();
    // The first read fetches cell-100 once and shows the agent the Compose
    // window; the poll's re-read of that window waits at the cell.
    let big = snapshot(&mut client, no_wait());
    big_screen(&big, &agent);
    let paused = line_serving(
        &mut client,
        &mut agent,
        "test-hook pause ",
        Duration::from_secs(10),
    );
    tap(&mut client, &big, "tab-login");
    let asked = Instant::now();
    // The default idle bound, as the driver's read after an action has.
    let reply = client.call_within(
        Call::Snapshot(SnapshotParams::default()),
        Some(DEADLINE),
        Duration::from_secs(30),
    );
    let took = asked.elapsed();
    let s = answered(reply, took);
    assert!(
        took < Duration::from_secs(2),
        "the snapshot waited for the re-read: answered after {:.0} ms",
        ms(took)
    );
    assert!(
        find(&s, "login-username").is_some(),
        "the snapshot shows the screen from before the tap: {} cells",
        find_all(&s, "cell-").len()
    );
    let resumed = line(&mut agent, "test-hook resume ", Duration::from_secs(2));
    let yielded = line(&mut agent, "trace compose yielded ", Duration::from_secs(2));
    let trace = line(&mut agent, "trace snapshot ", Duration::from_secs(2));
    println!(
        "{paused}; snapshot answered after {:.0} ms\n{resumed}\n{yielded}\n{trace}",
        ms(took)
    );
    assert_eq!(
        fields(&resumed).get("abandoned").map(String::as_str),
        Some("true"),
        "{resumed}"
    );
    let queued: u64 = fields(&trace)["queued_ms"]
        .parse()
        .expect("queued_ms is a number");
    assert!(
        queued < 500,
        "the snapshot waited {queued} ms for the tree worker"
    );
}

/// The agent's Compose poll re-read waits at [`PAUSE_AT`] while a tap
/// switches to Login, and no request comes: when it goes on, the re-read
/// finds the cells' parent gone, reads the window again from its root, and
/// reports the change as a tree event soon after the wait, not after
/// fetching 400 cells that are gone. The re-read's trace line tells its
/// event from those the tap's own accessibility events can bring.
#[test]
fn a_compose_reread_that_meets_a_torn_down_subtree_reads_the_new_screen_and_reports_the_change() {
    let Some(dev) = device(
        "a_compose_reread_that_meets_a_torn_down_subtree_reads_the_new_screen_and_reports_the_change",
    ) else {
        return;
    };
    let mut agent = agent_on_big(&dev, &[pause_at_second_fetch(PAUSE)], true);
    let mut client = agent.connect();
    let big = snapshot(&mut client, no_wait());
    big_screen(&big, &agent);
    let (cell, _) = find(&big, "cell-1").expect("the Big screen");
    let app = cell.window.id;
    let paused = line_serving(
        &mut client,
        &mut agent,
        "test-hook pause ",
        Duration::from_secs(10),
    );
    tap(&mut client, &big, "tab-login");
    let tapped = Instant::now();
    let reread = line_serving(
        &mut client,
        &mut agent,
        "trace compose windows=",
        PAUSE + Duration::from_secs(3),
    );
    let took = tapped.elapsed();
    println!(
        "{paused}; the re-read ended {:.0} ms after the tap: {reread}",
        ms(took)
    );
    let reported = fields(&reread);
    assert_eq!(
        reported.get("restarts").map(String::as_str),
        Some("1"),
        "{reread}"
    );
    let changed = reported.get("changed").map_or("-", String::as_str);
    let (window, path) = changed
        .split_once(':')
        .unwrap_or_else(|| panic!("the re-read reports no change: {reread}"));
    assert_eq!(window, app.to_string(), "{reread}");
    let path: Vec<u32> = path
        .split('.')
        .filter(|i| !i.is_empty())
        .map(|i| i.parse().expect("a child index"))
        .collect();
    let event = client.wait_event(
        Duration::from_secs(2),
        |e| matches!(&e.payload, EventPayload::Tree(t) if t.window == app && t.path == path),
    );
    assert!(
        event.is_some(),
        "no tree event at {path:?} for the Compose window"
    );
    let after = snapshot(&mut client, no_wait());
    assert!(
        find(&after, "login-username").is_some(),
        "Login is not in the next read"
    );
    assert!(
        find_all(&after, "cell-").is_empty(),
        "cells in the next read"
    );
    assert!(
        tree::at_path(&after, app, &path).is_some(),
        "the event's path {path:?} is not in the next read"
    );
}
