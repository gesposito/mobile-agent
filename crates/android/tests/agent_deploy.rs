//! Integration tests for agent deployment (task 5.2): first attach,
//! compatible reuse, incompatible replacement, conflicts with other holders
//! of UI automation, and stopping.
//!
//! They run only when `MOBILE_AGENT_ANDROID_SERIAL` names a device, one at a
//! time, and need the agent jar (`agent-android/build.sh`). Every agent they
//! start is stopped at the end, or killed by pid if the test fails first; the
//! agents also exit on their own 10 s after their host goes away.

mod common;

use std::time::{Duration, Instant};

use android::adb::{AdbServer, Device, ShellPacket};
use android::agent::{
    AGENT_VERSION, AgentJar, AgentState, Attached, DeployError, DeployOptions, Deployer,
    Incompatibility, JAR_MODE, Outcome, Records, Stopped, push_jar,
};
use common::{Leftovers, agent_alive, agent_jar, agent_reporting, device_lock, serial_or_skip};
use protocol::message::{Call, Reply, SnapshotParams};

fn device(serial: &str) -> Device {
    AdbServer::from_env().device(serial)
}

fn options() -> DeployOptions {
    DeployOptions {
        idle_exit: Duration::from_secs(10),
        ..DeployOptions::default()
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// A records directory unique to this run.
fn records_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(common::unique_name(&format!("records-{tag}")))
}

/// The packages installed on the device, sorted.
fn packages(device: &Device) -> Vec<String> {
    let out = device.shell("pm list packages").unwrap().stdout_text();
    let mut list: Vec<String> = out.lines().map(|l| l.trim().to_owned()).collect();
    list.sort();
    list
}

fn snapshot_nodes(attached: &Attached) -> u32 {
    let reply = attached
        .connection
        .call(
            Call::Snapshot(SnapshotParams::default()),
            Duration::from_secs(10),
        )
        .unwrap_or_else(|e| panic!("snapshot: {e}"));
    match reply {
        Reply::Snapshot(s) => s.node_count,
        other => panic!("a snapshot answered with {other:?}"),
    }
}

fn track(leftovers: &mut Leftovers, attached: &Attached) {
    leftovers.track(attached.agent.pid, &attached.agent.socket);
}

/// Spec scenario "First attach" (android-driver, Agent deployment without
/// installation), then "Daemon shutdown" (Agent lifecycle) through the
/// driver's stop.
#[test]
fn a_first_attach_starts_the_agent_and_installs_nothing() {
    let Some(serial) = serial_or_skip("a_first_attach_starts_the_agent_and_installs_nothing")
    else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    let mut leftovers = Leftovers::new(&device);
    let deployer = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
    assert_eq!(deployer.state().unwrap(), AgentState::Absent);
    let before = packages(&device);

    let attached = deployer
        .attach(&options())
        .unwrap_or_else(|e| panic!("{e}"));
    track(&mut leftovers, &attached);
    let Outcome::Launched(launched) = &attached.outcome else {
        panic!("a first attach reused {:?}", attached.outcome);
    };
    assert_eq!(launched.replaced, None);
    assert_eq!(launched.ready.pid, attached.agent.pid);
    assert_eq!(
        launched.ready.uid,
        Some(2000),
        "the agent runs as the shell user"
    );
    assert_eq!(attached.agent.protocol, protocol::PROTOCOL_VERSION);
    assert_eq!(attached.agent.agent, deployer.jar().agent_name());
    assert!(agent_alive(
        &device,
        attached.agent.pid,
        &attached.agent.socket
    ));

    let nodes = snapshot_nodes(&attached);
    assert!(nodes > 0, "the first snapshot is empty");
    assert_eq!(packages(&device), before, "a package was installed");
    let jars = device
        .shell(&format!("ls /data/local/tmp/{}.jar", attached.agent.socket))
        .unwrap();
    assert!(!jars.success(), "the pushed jar is still there");
    match deployer.state().unwrap() {
        AgentState::Running(agent) => assert_eq!(agent.pid, attached.agent.pid),
        other => panic!("the state of a running agent is {other:?}"),
    }
    let t = attached.timings;
    eprintln!(
        "first attach {:.1} ms: listing {:.1} ms, push {:.1} ms, spawn to ready {:.1} ms, \
         handshake {:.1} ms; first snapshot {nodes} nodes",
        ms(t.total),
        ms(t.listing),
        ms(t.push.unwrap()),
        ms(t.spawn_to_ready.unwrap()),
        ms(t.handshake),
    );

    let (pid, socket) = (attached.agent.pid, attached.agent.socket.clone());
    let stopped = attached.stop(Duration::from_secs(3)).unwrap();
    assert_eq!(stopped, Stopped::Exited);
    assert!(
        !agent_alive(&device, pid, &socket),
        "the agent outlived stop"
    );
    assert_eq!(deployer.state().unwrap(), AgentState::Absent);
    assert_eq!(deployer.records().get(&serial), None);
}

/// Spec scenario "Compatible agent reused": a later session, here a new
/// deployer reading the records a restarted daemon would, attaches to the
/// running agent and no process is started.
#[test]
fn a_compatible_agent_is_reused_without_starting_a_process() {
    let Some(serial) = serial_or_skip("a_compatible_agent_is_reused_without_starting_a_process")
    else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    let mut leftovers = Leftovers::new(&device);
    let dir = records_dir("reuse");

    let first = Deployer::new(device.clone(), agent_jar(), Records::in_dir(&dir).unwrap())
        .attach(&options())
        .unwrap_or_else(|e| panic!("{e}"));
    track(&mut leftovers, &first);
    let (pid, socket) = (first.agent.pid, first.agent.socket.clone());
    // The session ends; the agent stays.
    drop(first);

    let later = Deployer::new(device.clone(), agent_jar(), Records::in_dir(&dir).unwrap());
    let started = Instant::now();
    let attached = later.attach(&options()).unwrap_or_else(|e| panic!("{e}"));
    let took = started.elapsed();
    assert_eq!(attached.outcome, Outcome::Reused);
    assert_eq!(attached.agent.pid, pid);
    assert_eq!(attached.agent.socket, socket);
    assert_eq!(attached.timings.push, None, "nothing is pushed on reuse");
    let agents = later.running().unwrap();
    assert_eq!(agents.len(), 1, "another agent process started: {agents:?}");
    assert!(snapshot_nodes(&attached) > 0);
    eprintln!(
        "reuse attach {:.1} ms (listing {:.1} ms, handshake {:.1} ms)",
        ms(took),
        ms(attached.timings.listing),
        ms(attached.timings.handshake)
    );

    attached.stop(Duration::from_secs(3)).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Spec scenario "Incompatible agent replaced": an agent of another build
/// (the current dex reporting 0.0.9) runs from a previous session; the
/// driver stops it and starts the current build.
#[test]
fn an_incompatible_agent_is_replaced_by_the_current_build() {
    let Some(serial) = serial_or_skip("an_incompatible_agent_is_replaced_by_the_current_build")
    else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    let mut leftovers = Leftovers::new(&device);
    let records = Records::in_memory();

    let old_build = agent_reporting("0.0.9");
    let old = Deployer::new(device.clone(), old_build, records.clone())
        .attach(&options())
        .unwrap_or_else(|e| panic!("{e}"));
    track(&mut leftovers, &old);
    assert_eq!(old.agent.agent, "mobile-agent-android 0.0.9");
    let (old_pid, old_socket) = (old.agent.pid, old.agent.socket.clone());
    drop(old);

    let current = Deployer::new(device.clone(), agent_jar(), records.clone());
    match current.state().unwrap() {
        AgentState::Incompatible(agent) => assert_eq!(agent.pid, old_pid),
        other => panic!("an older build's state is {other:?}"),
    }
    let attached = current.attach(&options()).unwrap_or_else(|e| panic!("{e}"));
    track(&mut leftovers, &attached);
    let Outcome::Launched(launched) = &attached.outcome else {
        panic!("the older build was reused");
    };
    let replaced = launched
        .replaced
        .as_ref()
        .expect("the older build replaced");
    assert_eq!(replaced.pid, old_pid);
    assert_eq!(replaced.socket, old_socket);
    assert_eq!(
        replaced.reason,
        Incompatibility::Version {
            reported: "mobile-agent-android 0.0.9".into(),
            expected: format!("mobile-agent-android {AGENT_VERSION}"),
        }
    );
    assert_eq!(replaced.stopped, Stopped::Exited, "shutdown ends it");
    assert!(!agent_alive(&device, old_pid, &old_socket));
    assert_ne!(attached.agent.pid, old_pid);
    assert_eq!(attached.agent.agent, current.jar().agent_name());
    assert!(snapshot_nodes(&attached) > 0);
    eprintln!(
        "replacement attach {:.1} ms, of which stopping the older build {:.1} ms",
        ms(attached.timings.total),
        ms(attached.timings.replace.unwrap())
    );
    attached.stop(Duration::from_secs(3)).unwrap();
}

/// The pid of `uiautomator events` started on a stream, once it holds UI
/// automation; dropping the stream ends it (adbd sends SIGHUP).
fn start_uiautomator_events(device: &Device) -> (android::adb::ShellStream, u32) {
    let mut stream = device
        .shell_stream("echo $$; exec uiautomator events")
        .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut out = String::new();
    while !out.contains('\n') {
        match stream.read_packet().unwrap() {
            Some(ShellPacket::Stdout(data)) => out.push_str(&String::from_utf8_lossy(&data)),
            other => panic!("uiautomator events did not start: {other:?}"),
        }
    }
    let pid: u32 = out.lines().next().unwrap().trim().parse().unwrap();
    let held = (0..100).any(|_| {
        std::thread::sleep(Duration::from_millis(100));
        device
            .shell("dumpsys accessibility")
            .unwrap()
            .stdout_text()
            .contains("Ui Automation")
    });
    assert!(held, "uiautomator events did not take UI automation");
    (stream, pid)
}

/// Spec scenario "Another tool holds automation" (Single ownership of UI
/// automation), and the force option.
#[test]
fn another_tool_holding_automation_is_a_conflict_naming_it_until_forced() {
    let Some(serial) =
        serial_or_skip("another_tool_holding_automation_is_a_conflict_naming_it_until_forced")
    else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    let mut leftovers = Leftovers::new(&device);
    let deployer = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
    let (holder, holder_pid) = start_uiautomator_events(&device);

    let error = deployer.attach(&options()).unwrap_err();
    let DeployError::Conflict(conflict) = &error else {
        panic!("expected a conflict, got {error}");
    };
    let named = conflict.holder.as_ref().expect("a likely holder");
    assert_eq!(named.name, "uiautomator");
    assert_eq!(named.pid, Some(holder_pid));
    let message = error.to_string();
    assert!(message.contains("force"), "{message}");
    assert!(
        deployer.running().unwrap().is_empty(),
        "the refused agent stayed"
    );
    eprintln!("{message}");

    let forced = deployer
        .attach(&DeployOptions {
            force: true,
            ..options()
        })
        .unwrap_or_else(|e| panic!("{e}"));
    track(&mut leftovers, &forced);
    let Outcome::Launched(launched) = &forced.outcome else {
        panic!("force reused an agent");
    };
    let took_over = launched
        .took_over
        .as_ref()
        .expect("the holder force stopped");
    assert_eq!(took_over.name, "uiautomator");
    assert_eq!(took_over.pid, Some(holder_pid));
    drop(holder);
    forced.stop(Duration::from_secs(3)).unwrap();
}

/// Single ownership: an agent this host did not start (another daemon's,
/// with its own records) is another holder; force takes over from it.
#[test]
fn an_agent_another_host_started_is_a_conflict_until_forced() {
    let Some(serial) = serial_or_skip("an_agent_another_host_started_is_a_conflict_until_forced")
    else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    let mut leftovers = Leftovers::new(&device);
    let theirs = Deployer::new(device.clone(), agent_jar(), Records::in_memory())
        .attach(&options())
        .unwrap_or_else(|e| panic!("{e}"));
    track(&mut leftovers, &theirs);

    let ours = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
    let started = Instant::now();
    let error = ours.attach(&options()).unwrap_err();
    let DeployError::Conflict(conflict) = &error else {
        panic!("expected a conflict, got {error}");
    };
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the conflict took {:?}",
        started.elapsed()
    );
    let holder = conflict.holder.as_ref().expect("the other agent named");
    assert_eq!(holder.name, "mobile-agent");
    assert_eq!(holder.pid, Some(theirs.agent.pid));
    assert!(agent_alive(&device, theirs.agent.pid, &theirs.agent.socket));

    let forced = ours
        .attach(&DeployOptions {
            force: true,
            ..options()
        })
        .unwrap_or_else(|e| panic!("{e}"));
    track(&mut leftovers, &forced);
    let Outcome::Launched(launched) = &forced.outcome else {
        panic!("force reused the other host's agent");
    };
    assert_eq!(
        launched.took_over.as_ref().map(|h| h.name.as_str()),
        Some("mobile-agent")
    );
    assert!(!agent_alive(
        &device,
        theirs.agent.pid,
        &theirs.agent.socket
    ));
    forced.stop(Duration::from_secs(3)).unwrap();
}

/// The jar arrives read-only: adbd copies the owner's bits to group and
/// other, so a writable mode would arrive writable by everyone.
#[test]
fn the_jar_is_pushed_read_only() {
    let Some(serial) = serial_or_skip("the_jar_is_pushed_read_only") else {
        return;
    };
    let device = device(&serial);
    let remote = format!("/data/local/tmp/{}.jar", common::unique_name("jar-mode"));
    let jar = AgentJar::from_bytes(vec![0u8; 1000], AGENT_VERSION);
    push_jar(&device, &jar, &remote).unwrap();
    let stat = device.shell(&format!("stat -c %a {remote}")).unwrap();
    let _ = device.shell(&format!("rm -f {remote}"));
    assert_eq!(stat.stdout_text().trim(), format!("{JAR_MODE:o}"));
}
