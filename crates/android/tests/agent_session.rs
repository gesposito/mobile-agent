//! Integration tests for the agent session behind the mirror (task 5.3).
//!
//! They run only when `MOBILE_AGENT_ANDROID_SERIAL` names a device with the
//! Compose fixture installed (`fixtures/compose`), one at a time, and need
//! the agent jar. Each starts the fixture, attaches, and stops its agent at
//! the end (or kills it by pid when it fails first); the controller test
//! also needs an emulator started with `-grpc <port> -grpc-use-token`. The
//! device may be shared: a test that another user disturbs (its agent takes
//! UI automation, or it moves the fixture) runs again once the device is
//! quiet.

mod common;

use std::time::{Duration, Instant};

use android::adb::{AdbServer, Device};
use android::agent::{Attached, DeployOptions, Deployer, Records};
use android::emulator::{self, Options};
use android::session::{ControllerLink, Session, SessionOptions};
use common::{
    Disturbed, Leftovers, agent_alive, agent_jar, device_lock, disturbed_by, retry_when_disturbed,
    serial_or_skip,
};
use mirror::backend::{ReadReason, ReadRequest, StreamItem};
use mirror::{
    Action, Condition, Degradation, Mirror, MirrorConfig, Selector, SnapshotOptions, Target,
};

const COMPOSE: &str = "dev.mobileagent.fixture.compose";

fn device(serial: &str) -> Device {
    AdbServer::from_env().device(serial)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// The activity in front, as the activity manager names it.
fn foreground(device: &Device) -> String {
    device
        .shell("dumpsys activity activities | grep -m1 topResumedActivity")
        .map(|out| out.stdout_text().trim().to_owned())
        .unwrap_or_default()
}

/// A disturbance unless the Compose fixture is in front.
fn compose_in_front(device: &Device, what: &str) -> Result<(), Disturbed> {
    let front = foreground(device);
    if front.contains(COMPOSE) {
        Ok(())
    } else {
        Err(Disturbed(format!(
            "{what}: another app is in front ({front})"
        )))
    }
}

/// Starts the Compose fixture afresh on `tab`, waits until it is in front
/// and for the tree to settle.
fn launch_compose(device: &Device, tab: &str) -> Result<(), Disturbed> {
    let out = device
        .shell(&format!(
            "am start -W -S -n {COMPOSE}/.MainActivity --es initial_tab {tab}"
        ))
        .unwrap();
    assert!(
        out.stdout_text().contains("Status: ok"),
        "the Compose fixture did not start: {}{}",
        out.stdout_text(),
        out.stderr_text()
    );
    let front = Instant::now();
    while compose_in_front(device, "launch").is_err() {
        if front.elapsed() > Duration::from_secs(5) {
            return compose_in_front(device, "launch");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(800));
    Ok(())
}

/// Stops the Compose fixture when dropped.
struct Fixture(Device);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.shell(&format!("am force-stop {COMPOSE}"));
    }
}

fn attach(device: &Device, leftovers: &mut Leftovers) -> Result<Attached, Disturbed> {
    let attached = Deployer::new(device.clone(), agent_jar(), Records::in_memory())
        .attach(&DeployOptions {
            idle_exit: Duration::from_secs(10),
            ..DeployOptions::default()
        })
        .map_err(|e| disturbed_by(device, e))?;
    leftovers.track(attached.agent.pid, &attached.agent.socket);
    Ok(attached)
}

/// Nodes in the mirror's replica: the indented lines of its dump.
fn replica_nodes(mirror: &Mirror) -> usize {
    mirror
        .dump()
        .lines()
        .filter(|line| line.starts_with("  "))
        .count()
}

/// The bounds of the node whose resource id is `id`.
fn find_bounds(
    snapshot: &protocol::snapshot::Snapshot,
    id: &str,
) -> Option<protocol::snapshot::Bounds> {
    fn walk(node: &protocol::snapshot::Node, id: &str) -> Option<protocol::snapshot::Bounds> {
        if node.res_id.as_deref() == Some(id) {
            return Some(node.bounds);
        }
        node.children.iter().find_map(|child| walk(child, id))
    }
    snapshot
        .windows
        .iter()
        .filter_map(|w| w.root.as_ref())
        .find_map(|root| walk(root, id))
}

/// The full-stack test of task 5.3: a mirror built on the session serves
/// the Compose Big screen with the node count the agent reported, a tap
/// through the input sink changes the tree, and a killed agent is reported
/// disconnected within 5 s (android-driver "Agent lifecycle", device-protocol
/// "Liveness detection", ui-mirror "Replica maintained from device events").
#[test]
fn the_mirror_serves_compose_big_follows_a_tap_and_reports_a_killed_agent() {
    let test = "the_mirror_serves_compose_big_follows_a_tap_and_reports_a_killed_agent";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || full_stack(&device));
}

fn full_stack(device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    launch_compose(device, "big")?;
    let _fixture = Fixture(device.clone());
    let attached = attach(device, &mut leftovers)?;
    let session =
        Session::start(attached, SessionOptions::default(), None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        session.scale(),
        2.625,
        "the API 36 emulator runs at 420 dpi"
    );
    let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());

    let started = Instant::now();
    let big = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    let cold = started.elapsed();
    compose_in_front(device, "the first snapshot")?;
    if !big.text.contains("cell-500") {
        return Err(Disturbed(format!(
            "the first snapshot shows another tab: {}",
            big.text.lines().take(3).collect::<Vec<_>>().join(" | ")
        )));
    }
    let read = session.last_read().expect("a tree read");
    assert_eq!(read.serialized, read.node_count, "the read was cut short");
    assert_eq!(replica_nodes(&mirror), read.node_count as usize);
    assert!(read.node_count >= 500, "{} nodes", read.node_count);
    for i in 1..=500 {
        let id = format!("cell-{i}");
        let cell = big
            .elements
            .iter()
            .find(|e| e.identifier.as_deref() == Some(id.as_str()))
            .unwrap_or_else(|| panic!("no {id} in the snapshot"));
        assert!(cell.reference.is_some(), "{id} has no ref");
    }
    assert!(!big.verdict.degraded, "{:?}", big.verdict);

    // Warm snapshots come from the replica without a device read. The
    // Compose window, which sends no events, is read again every second
    // (the mirror's poll); that read is let pass first, so that none falls
    // due while the snapshots are timed.
    if let Some(due) = mirror.next_wakeup() {
        mirror.run_until(due + Duration::from_millis(50)).unwrap();
    }
    let reads = session.reads();
    let quiet_until = mirror.next_wakeup();
    let mut warm = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
        warm.push(ms(t.elapsed()));
    }
    if quiet_until.is_none_or(|due| Instant::now() < due) {
        assert_eq!(session.reads(), reads, "a warm snapshot read the device");
    } else {
        eprintln!("the warm snapshots outlasted the poll interval; reads not compared");
    }

    // A tap on the Login tab, through the sink, changes the tree.
    let tab = big
        .elements
        .iter()
        .find(|e| e.identifier.as_deref() == Some("tab-login"))
        .and_then(|e| e.reference)
        .expect("a ref for tab-login");
    let tapped = Instant::now();
    let outcome = mirror
        .act(&Action::Tap {
            target: Target::Ref(tab),
            hold: None,
        })
        .unwrap();
    let acknowledged = tapped.elapsed();
    assert!(outcome.accepted);
    let login = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    let changed = tapped.elapsed();
    compose_in_front(device, "the tap")?;
    assert!(login.text.contains("login-username"), "{}", login.text);
    assert!(!login.text.contains("cell-1"), "{}", login.text);

    // Killing the agent is reported within 5 s. A read already due can fail
    // with `disconnected` before the mirror takes the stream's report;
    // either is the disconnection reported.
    let (pid, socket) = (session.agent().pid, session.agent().socket.clone());
    let killed = Instant::now();
    device
        .shell(&format!(
            "grep -qF {socket} /proc/{pid}/cmdline && kill -9 {pid}"
        ))
        .unwrap();
    let reported = |result: Result<(), mirror::MirrorError>| match result {
        Err(e) if e.code() == protocol::code::DISCONNECTED => true,
        Err(e) => panic!("{e}"),
        Ok(()) => false,
    };
    let detected = loop {
        let pumped = mirror.run_until(Instant::now() + Duration::from_millis(20));
        let snapshot = mirror.snapshot(&SnapshotOptions::interactive());
        let in_verdict = snapshot.as_ref().is_ok_and(|s| {
            s.verdict
                .reasons
                .iter()
                .any(|r| matches!(r, Degradation::Disconnected(_)))
        });
        if in_verdict || reported(pumped) || reported(snapshot.map(|_| ())) {
            break killed.elapsed();
        }
        assert!(
            killed.elapsed() < Duration::from_secs(5),
            "no disconnection reported 5 s after the kill"
        );
    };
    let last = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    assert!(
        last.verdict
            .reasons
            .iter()
            .any(|r| matches!(r, Degradation::Disconnected(_))),
        "{:?}",
        last.verdict
    );
    assert!(!agent_alive(device, pid, &socket));
    let refused = mirror.act(&Action::Tap {
        target: Target::Ref(tab),
        hold: None,
    });
    assert!(refused.is_err(), "an action on a disconnected target");

    eprintln!(
        "Compose Big through the mirror: {} nodes, first snapshot {:.1} ms (agent read {:.1} ms); \
         warm snapshot p50 {:.2} ms, p95 {:.2} ms; tap acknowledged {:.1} ms, tree changed \
         {:.1} ms after the tap; killed agent reported disconnected after {:.1} ms",
        read.node_count,
        ms(cold),
        ms(read.took),
        common::percentile(&mut warm.clone(), 50.0),
        common::percentile(&mut warm, 95.0),
        ms(acknowledged),
        ms(changed),
        ms(detected),
    );
    Ok(())
}

/// Spec scenario "App restarted" (Agent lifecycle), through the driver: the
/// same agent and session serve the mirror after the app is force-stopped
/// and relaunched.
#[test]
fn the_same_agent_serves_the_mirror_after_the_app_is_force_stopped_and_relaunched() {
    let test = "the_same_agent_serves_the_mirror_after_the_app_is_force_stopped_and_relaunched";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || app_restart(&device));
}

fn app_restart(device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    launch_compose(device, "big")?;
    let _fixture = Fixture(device.clone());
    let attached = attach(device, &mut leftovers)?;
    let session =
        Session::start(attached, SessionOptions::default(), None).unwrap_or_else(|e| panic!("{e}"));
    let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());
    let big = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    compose_in_front(device, "the first snapshot")?;
    if !big.text.contains("cell-1") {
        return Err(Disturbed("the first snapshot shows another tab".into()));
    }

    device.shell(&format!("am force-stop {COMPOSE}")).unwrap();
    launch_compose(device, "login")?;
    let found = mirror.wait(
        &Selector::identifier("login-username"),
        Condition::Appear,
        Duration::from_secs(10),
    );
    compose_in_front(device, "the relaunch")?;
    let found = found.unwrap_or_else(|e| panic!("{e}"));
    assert!(found.element.is_some());
    assert!(session.close_reason().is_none(), "the session was lost");
    let (pid, socket) = (session.agent().pid, session.agent().socket.clone());
    assert!(agent_alive(device, pid, &socket), "the agent was restarted");
    session.stop(Duration::from_secs(3)).unwrap();
    Ok(())
}

/// On an emulator with its controller, frames arrive on the session's
/// event stream with hashes that leave the status bar out: a tab switch
/// changes the hash, and the frames encode as JPEG.
#[test]
fn controller_frames_arrive_on_the_event_stream_with_status_bar_free_hashes() {
    let test = "controller_frames_arrive_on_the_event_stream_with_status_bar_free_hashes";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || controller_frames(&serial, &device));
}

fn controller_frames(serial: &str, device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    launch_compose(device, "login")?;
    let _fixture = Fixture(device.clone());
    let endpoint = emulator::resolve(serial).unwrap_or_else(|e| panic!("{e}"));
    let link =
        ControllerLink::connect(&endpoint, Options::default()).unwrap_or_else(|e| panic!("{e}"));
    let attached = attach(device, &mut leftovers)?;
    let session = Session::start(attached, SessionOptions::default(), Some(link))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(session.uses_controller());
    let mut backend = session.backend();

    // Tap the List tab, found in a tree read, so that the screen changes.
    let tree = backend
        .tree
        .read(&ReadRequest {
            windows: None,
            reason: ReadReason::Attach,
        })
        .unwrap();
    compose_in_front(device, "the tree read")?;
    let Some(bounds) = find_bounds(&tree.snapshot, "tab-list") else {
        return Err(Disturbed("no tab-list in the tree".into()));
    };
    let at = mirror::Point::new(
        f64::from(bounds.left + bounds.right) / 2.0 / tree.scale,
        f64::from(bounds.top + bounds.bottom) / 2.0 / tree.scale,
    );
    backend
        .input
        .perform(&mirror::backend::Input::Tap { at, hold: None })
        .unwrap();
    let mut frames = Vec::new();
    let until = Instant::now() + Duration::from_secs(3);
    while let Some(item) = backend.events.next(until) {
        if let StreamItem::Frame(meta) = item.item {
            frames.push(meta);
        }
    }
    compose_in_front(device, "the frames")?;
    assert!(
        frames.len() >= 2,
        "{} frames in 3 s after a tab switch",
        frames.len()
    );
    let (first, last) = (frames[0], frames[frames.len() - 1]);
    assert_ne!(
        first.hash, last.hash,
        "the tab switch left the hash unchanged"
    );
    assert!(frames.windows(2).all(|w| w[0].seq < w[1].seq));
    assert_eq!((last.width, last.height), (540, 1200));
    let source = backend.frames.as_mut().expect("a frame source");
    let latest = source.latest().expect("the latest frame");
    let jpeg = source
        .encode(&mirror::backend::EncodeRequest {
            seq: latest.seq,
            width: 450,
            height: 1000,
            quality: 80,
            rotation: latest.rotation,
        })
        .unwrap();
    assert_eq!(&jpeg[..2], [0xFF, 0xD8]);
    let mut hashes: Vec<u64> = frames.iter().map(|f| f.hash).collect();
    hashes.sort_unstable();
    hashes.dedup();
    eprintln!(
        "{} frames in the 3 s after a tab switch, {} distinct hashes; latest encoded to {} bytes",
        frames.len(),
        hashes.len(),
        jpeg.len()
    );
    session.stop(Duration::from_secs(3)).unwrap();
    Ok(())
}
