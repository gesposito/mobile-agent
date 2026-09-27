//! Task 4.9: stopping, exiting cleanly, idle exit after the host leaves, and
//! surviving app restarts and rotation (spec android-driver, Agent lifecycle).

use std::time::{Duration, Instant};

use agent_android_host_tests::client::Client;
use agent_android_host_tests::fixture::RotationGuard;
use agent_android_host_tests::tree::find;
use agent_android_host_tests::{Agent, Launch, device, fixture, poll};
use protocol::event::Stream;
use protocol::message::{Call, Reply, SnapshotParams};
use protocol::snapshot::{Snapshot, WindowType};

fn snapshot(client: &mut Client) -> Snapshot {
    match client.call(Call::Snapshot(SnapshotParams::default())) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("snapshot failed: {other:?}"),
    }
}

/// Waits until no process names the agent's socket, returning how long it took.
fn gone(agent: &Agent, timeout: Duration) -> Option<Duration> {
    let t0 = Instant::now();
    poll(timeout, Duration::from_millis(100), || {
        agent
            .device()
            .processes_matching(&agent.socket)
            .is_empty()
            .then(|| t0.elapsed())
    })
}

#[test]
fn after_shutdown_no_agent_process_remains_and_the_role_is_free() {
    let Some(dev) = device("after_shutdown_no_agent_process_remains_and_the_role_is_free") else {
        return;
    };
    let mut agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let _ = snapshot(&mut client);
    assert_eq!(client.call(Call::Shutdown).expect("shutdown"), Reply::Empty);
    let lifecycle = client.lifecycle();
    assert_eq!(
        lifecycle.last().map(|(k, _)| k.as_str()),
        Some("shutting_down"),
        "{lifecycle:?}"
    );
    let status = agent
        .wait_exit(Duration::from_secs(5))
        .expect("the agent exited");
    assert!(status.success(), "{status}");
    let left = gone(&agent, Duration::from_secs(3));
    assert!(
        left.is_some(),
        "{:?}",
        dev.processes_matching(&agent.socket)
    );
    assert!(
        !dev.ui_automation_registered(),
        "the role outlived the agent"
    );
}

#[test]
fn after_the_host_disconnects_the_agent_exits_within_its_idle_bound() {
    let Some(dev) = device("after_the_host_disconnects_the_agent_exits_within_its_idle_bound")
    else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default().arg("--idle-exit-ms").arg("1500"));
    let mut client = agent.connect();
    // A connected host keeps the agent however quiet it is.
    client.idle(Duration::from_millis(2500));
    assert!(agent.alive(), "the agent exited with a host connected");
    let _ = snapshot(&mut client);
    client.disconnect();
    let t0 = Instant::now();
    let left = gone(&agent, Duration::from_secs(6)).map(|_| t0.elapsed());
    let left = left.unwrap_or_else(|| {
        panic!(
            "the agent was still running 6 s after the host left: {:?}",
            dev.processes_matching(&agent.socket)
        )
    });
    println!(
        "agent gone {:.1} s after the host disconnected",
        left.as_secs_f64()
    );
    assert!(left >= Duration::from_millis(1400), "exited after {left:?}");
    assert!(
        !dev.ui_automation_registered(),
        "the role outlived the agent"
    );
}

#[test]
fn an_agent_no_host_ever_reaches_exits_within_its_idle_bound() {
    let Some(dev) = device("an_agent_no_host_ever_reaches_exits_within_its_idle_bound") else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default().arg("--idle-exit-ms").arg("1000"));
    assert!(
        gone(&agent, Duration::from_secs(5)).is_some(),
        "the agent kept running without a host"
    );
}

#[test]
fn the_same_agent_serves_snapshots_after_the_app_is_force_stopped_and_relaunched() {
    let Some(dev) =
        device("the_same_agent_serves_snapshots_after_the_app_is_force_stopped_and_relaunched")
    else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "list");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let before = snapshot(&mut client);
    assert!(find(&before, "row-2").is_some());
    fixture::force_stop(&dev, fixture::EXPO);
    let home = snapshot(&mut client);
    assert!(
        find(&home, "row-2").is_none(),
        "the stopped app is still in the tree"
    );
    fixture::launch(&dev, fixture::EXPO, "login");
    let after = snapshot(&mut client);
    assert!(
        find(&after, "login-username").is_some(),
        "the relaunched app is missing"
    );
    assert!(agent.alive());
    assert!(client.closed().is_none());
    assert!(
        !client.events_on(Stream::WINDOW).is_empty(),
        "the restart sent no window events"
    );
}

#[test]
fn the_same_agent_serves_snapshots_across_a_rotation() {
    let Some(dev) = device("the_same_agent_serves_snapshots_across_a_rotation") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let rotation = RotationGuard::new(&dev);
    rotation.freeze(0);
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let app_width = |s: &Snapshot| {
        s.windows
            .iter()
            .find(|w| w.kind == WindowType::APPLICATION)
            .map(|w| w.bounds.right - w.bounds.left)
            .expect("an app window")
    };
    let portrait = snapshot(&mut client);
    rotation.freeze(1);
    let landscape = snapshot(&mut client);
    rotation.freeze(0);
    let back = snapshot(&mut client);
    assert!(app_width(&landscape) > app_width(&portrait));
    assert_eq!(app_width(&back), app_width(&portrait));
    assert!(find(&landscape, "login-username").is_some());
    assert!(agent.alive());
    drop(client);
    drop(agent);
    drop(rotation);
}

#[test]
fn a_first_attach_serves_a_snapshot_without_installing_a_package() {
    let Some(dev) = device("a_first_attach_serves_a_snapshot_without_installing_a_package") else {
        return;
    };
    let packages = || {
        let mut list: Vec<String> = dev
            .shell("pm list packages")
            .lines()
            .map(str::to_owned)
            .collect();
        list.sort();
        list
    };
    let instrumentations = || dev.shell("pm list instrumentation");
    let before = (packages(), instrumentations());
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    assert!(!s.windows.is_empty());
    assert_eq!(
        (packages(), instrumentations()),
        before,
        "the attach installed something"
    );
}
