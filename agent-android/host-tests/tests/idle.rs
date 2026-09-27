//! Task 4.4: the bounded idle wait before a tree read (spec android-driver,
//! Bounded idle waiting).

use std::time::{Duration, Instant};

use agent_android_host_tests::agent::fields;
use agent_android_host_tests::client::Client;
use agent_android_host_tests::{Agent, Launch, device, fixture, ms};
use protocol::message::{Call, Reply, SnapshotParams};

/// The agent's default bound, which the spec requires to be under a second.
const DEFAULT_BOUND_MS: u64 = 500;

/// Slack for scheduling on a loaded host and emulator.
const SLACK_MS: u64 = 60;

/// One snapshot: host round trip, and the agent's idle wait and read time
/// from its trace line.
fn traced(agent: &mut Agent, client: &mut Client, params: SnapshotParams) -> (u64, u64, u64) {
    let t0 = Instant::now();
    let reply = client.call(Call::Snapshot(params));
    let elapsed = t0.elapsed().as_millis() as u64;
    assert!(matches!(reply, Ok(Reply::Snapshot(_))), "{reply:?}");
    let line = agent
        .wait_line(Duration::from_secs(2), |l| l.starts_with("trace snapshot "))
        .expect("a trace line");
    let f = fields(&line);
    (
        elapsed,
        f["idle_ms"].parse().expect("idle_ms"),
        f["read_ms"].parse().expect("read_ms"),
    )
}

#[test]
fn a_snapshot_during_an_endless_animation_returns_within_the_default_bound_plus_read_time() {
    let Some(dev) = device(
        "a_snapshot_during_an_endless_animation_returns_within_the_default_bound_plus_read_time",
    ) else {
        return;
    };
    // The React Native spinner sends a content change about every 100 ms.
    fixture::launch(&dev, fixture::EXPO, "anim");
    let mut agent = Agent::launch(&dev, Launch::default().arg("--trace"));
    let mut client = agent.connect();
    let _ = traced(
        &mut agent,
        &mut client,
        SnapshotParams {
            idle_ms: Some(0),
            ..SnapshotParams::default()
        },
    );
    for _ in 0..5 {
        let (elapsed, idle, read) = traced(&mut agent, &mut client, SnapshotParams::default());
        println!(
            "endless animation, default bound: waited {idle} ms, read {read} ms, answered after {elapsed} ms"
        );
        assert!(
            idle <= DEFAULT_BOUND_MS + SLACK_MS,
            "waited {idle} ms, over the {DEFAULT_BOUND_MS} ms bound"
        );
        // The animation never pauses for the quiet window, so the wait runs to its bound.
        assert!(idle + SLACK_MS >= DEFAULT_BOUND_MS, "waited only {idle} ms");
        assert!(
            elapsed <= DEFAULT_BOUND_MS + read + 2 * SLACK_MS,
            "answered after {elapsed} ms, read {read} ms"
        );
    }
}

#[test]
fn idle_zero_reads_at_once_during_an_animation() {
    let Some(dev) = device("idle_zero_reads_at_once_during_an_animation") else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "anim");
    let mut agent = Agent::launch(&dev, Launch::default().arg("--trace"));
    let mut client = agent.connect();
    for _ in 0..3 {
        let (elapsed, idle, read) = traced(
            &mut agent,
            &mut client,
            SnapshotParams {
                idle_ms: Some(0),
                ..SnapshotParams::default()
            },
        );
        assert_eq!(idle, 0);
        assert!(
            elapsed <= read + 2 * SLACK_MS,
            "answered after {elapsed} ms"
        );
    }
}

#[test]
fn a_requested_bound_caps_the_wait() {
    let Some(dev) = device("a_requested_bound_caps_the_wait") else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "anim");
    let mut agent = Agent::launch(&dev, Launch::default().arg("--trace"));
    let mut client = agent.connect();
    let (_, idle, _) = traced(
        &mut agent,
        &mut client,
        SnapshotParams {
            idle_ms: Some(150),
            ..SnapshotParams::default()
        },
    );
    assert!(idle <= 150 + SLACK_MS, "waited {idle} ms");
    assert!(idle + SLACK_MS >= 150, "waited only {idle} ms");
}

#[test]
fn a_quiet_screen_is_read_without_waiting_for_the_bound() {
    let Some(dev) = device("a_quiet_screen_is_read_without_waiting_for_the_bound") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let mut agent = Agent::launch(&dev, Launch::default().arg("--trace"));
    let mut client = agent.connect();
    // Let the events of the attach pass, then read with the default bound.
    client.idle(Duration::from_millis(600));
    let (elapsed, idle, _) = traced(&mut agent, &mut client, SnapshotParams::default());
    println!(
        "quiet screen: waited {idle} ms, answered after {:.1} ms",
        ms(Duration::from_millis(elapsed))
    );
    assert!(
        idle < DEFAULT_BOUND_MS / 2,
        "waited {idle} ms on a quiet screen"
    );
}
