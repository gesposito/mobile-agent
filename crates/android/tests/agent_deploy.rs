//! Integration tests for agent deployment (task 5.2): first attach,
//! compatible reuse, incompatible replacement, conflicts with other holders
//! of UI automation, and stopping.
//!
//! They run only when `MOBILE_AGENT_ANDROID_SERIAL` names a device, one at a
//! time, and need the agent jar (`agent-android/build.sh`). Every agent they
//! start is stopped at the end, or killed by pid if the test fails first; the
//! agents also exit on their own 10 s after their host goes away. The device
//! may be shared: a test that another user disturbs (its agent or
//! `uiautomator` takes UI automation) runs again once the device is quiet.

mod common;

use std::time::{Duration, Instant};

use android::adb::{AdbServer, Device, ShellPacket, ShellStream};
use android::agent::{
    AGENT_VERSION, AgentJar, AgentState, Attached, DeployError, DeployOptions, Deployer,
    Incompatibility, JAR_MODE, Outcome, Records, Stopped, push_jar,
};
use common::{
    Disturbed, Leftovers, agent_alive, agent_jar, agent_reporting, device_lock, disturbed_by,
    others_on, retry_when_disturbed, serial_or_skip,
};
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

fn forced() -> DeployOptions {
    DeployOptions {
        force: true,
        ..options()
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

/// A snapshot's node count; a failure while another user of the device
/// runs an agent (whose `--force` kills ours) is a disturbance.
fn snapshot_nodes(device: &Device, attached: &Attached) -> Result<u32, Disturbed> {
    let reply = attached.connection.call(
        Call::Snapshot(SnapshotParams::default()),
        Duration::from_secs(10),
    );
    match reply {
        Ok(Reply::Snapshot(s)) => Ok(s.node_count),
        Ok(other) => panic!("a snapshot answered with {other:?}"),
        Err(error) => match others_on(device, &[attached.agent.pid]) {
            Some(who) => Err(Disturbed(format!("snapshot: {error}, while {who}"))),
            None => panic!("snapshot: {error}"),
        },
    }
}

/// Attaches with `deployer` and has the agent killed if the test fails.
fn attach(
    deployer: &Deployer,
    options: &DeployOptions,
    leftovers: &mut Leftovers,
) -> Result<Attached, Disturbed> {
    let attached = deployer
        .attach(options)
        .map_err(|e| disturbed_by(deployer.device(), e))?;
    leftovers.track(attached.agent.pid, &attached.agent.socket);
    Ok(attached)
}

/// A disturbance when an agent this test did not start runs.
fn only_ours(deployer: &Deployer, ours: &[u32]) -> Result<(), Disturbed> {
    let others: Vec<_> = deployer
        .running()
        .unwrap()
        .into_iter()
        .filter(|agent| !ours.contains(&agent.pid))
        .collect();
    if others.is_empty() {
        Ok(())
    } else {
        Err(Disturbed(format!("another agent runs: {others:?}")))
    }
}

/// Spec scenario "First attach" (android-driver, Agent deployment without
/// installation), then "Daemon shutdown" (Agent lifecycle) through the
/// driver's stop.
#[test]
fn a_first_attach_starts_the_agent_and_installs_nothing() {
    let test = "a_first_attach_starts_the_agent_and_installs_nothing";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || first_attach(&device));
}

fn first_attach(device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    let deployer = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
    match deployer.state().unwrap() {
        AgentState::Absent => {}
        other => return Err(Disturbed(format!("an agent runs: {other:?}"))),
    }
    let before = packages(device);

    let attached = attach(&deployer, &options(), &mut leftovers)?;
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
        device,
        attached.agent.pid,
        &attached.agent.socket
    ));

    let nodes = snapshot_nodes(device, &attached)?;
    assert!(nodes > 0, "the first snapshot is empty");
    assert_eq!(packages(device), before, "a package was installed");
    let jars = device
        .shell(&format!("ls /data/local/tmp/{}.jar", attached.agent.socket))
        .unwrap();
    assert!(!jars.success(), "the pushed jar is still there");
    only_ours(&deployer, &[attached.agent.pid])?;
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
        !agent_alive(device, pid, &socket),
        "the agent outlived stop"
    );
    only_ours(&deployer, &[])?;
    assert_eq!(deployer.state().unwrap(), AgentState::Absent);
    assert_eq!(deployer.records().get(device.serial()), None);
    Ok(())
}

/// Spec scenario "Compatible agent reused": a later session, here a new
/// deployer reading the records a restarted daemon would, attaches to the
/// running agent and no process is started.
#[test]
fn a_compatible_agent_is_reused_without_starting_a_process() {
    let test = "a_compatible_agent_is_reused_without_starting_a_process";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || compatible_reuse(&device));
}

fn compatible_reuse(device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    let dir = records_dir("reuse");
    let first = Deployer::new(device.clone(), agent_jar(), Records::in_dir(&dir).unwrap());
    let first = attach(&first, &options(), &mut leftovers)?;
    let (pid, socket) = (first.agent.pid, first.agent.socket.clone());
    // The session ends; the agent stays.
    drop(first);

    let later = Deployer::new(device.clone(), agent_jar(), Records::in_dir(&dir).unwrap());
    let started = Instant::now();
    let attached = attach(&later, &options(), &mut leftovers)?;
    let took = started.elapsed();
    assert_eq!(attached.outcome, Outcome::Reused);
    assert_eq!(attached.agent.pid, pid);
    assert_eq!(attached.agent.socket, socket);
    assert_eq!(attached.timings.push, None, "nothing is pushed on reuse");
    only_ours(&later, &[pid])?;
    assert!(snapshot_nodes(device, &attached)? > 0);
    eprintln!(
        "reuse attach {:.1} ms (listing {:.1} ms, handshake {:.1} ms)",
        ms(took),
        ms(attached.timings.listing),
        ms(attached.timings.handshake)
    );

    attached.stop(Duration::from_secs(3)).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    Ok(())
}

/// Spec scenario "Incompatible agent replaced": an agent of another build
/// (the current dex reporting 0.0.9) runs from a previous session; the
/// driver stops it and starts the current build.
#[test]
fn an_incompatible_agent_is_replaced_by_the_current_build() {
    let test = "an_incompatible_agent_is_replaced_by_the_current_build";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || incompatible_replacement(&device));
}

fn incompatible_replacement(device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    let records = Records::in_memory();
    let old = Deployer::new(device.clone(), agent_reporting("0.0.9"), records.clone());
    let old = attach(&old, &options(), &mut leftovers)?;
    assert_eq!(old.agent.agent, "mobile-agent-android 0.0.9");
    let (old_pid, old_socket) = (old.agent.pid, old.agent.socket.clone());
    drop(old);

    let current = Deployer::new(device.clone(), agent_jar(), records.clone());
    only_ours(&current, &[old_pid])?;
    match current.state().unwrap() {
        AgentState::Incompatible(agent) => assert_eq!(agent.pid, old_pid),
        other => panic!("an older build's state is {other:?}"),
    }
    let attached = attach(&current, &options(), &mut leftovers)?;
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
    assert!(!agent_alive(device, old_pid, &old_socket));
    assert_ne!(attached.agent.pid, old_pid);
    assert_eq!(attached.agent.agent, current.jar().agent_name());
    assert!(snapshot_nodes(device, &attached)? > 0);
    eprintln!(
        "replacement attach {:.1} ms, of which stopping the older build {:.1} ms",
        ms(attached.timings.total),
        ms(attached.timings.replace.unwrap())
    );
    attached.stop(Duration::from_secs(3)).unwrap();
    Ok(())
}

/// An agent of another build that reports the same version, as the agent
/// before and after the window metadata (task 3.6) does, runs from a
/// previous session: the driver replaces it. The other build is the jar's
/// own dex pushed bare, which `app_process` loads alike.
#[test]
fn an_agent_of_another_build_of_the_same_version_is_replaced() {
    let test = "an_agent_of_another_build_of_the_same_version_is_replaced";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || other_build(&device));
}

fn other_build(device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    let records = Records::in_memory();
    let jar = agent_jar();
    let bare = AgentJar::from_bytes(common::dex_of(jar.bytes()), AGENT_VERSION);
    assert_ne!(bare.build(), jar.build());
    let old = Deployer::new(device.clone(), bare.clone(), records.clone());
    let old = attach(&old, &options(), &mut leftovers)?;
    assert_eq!(old.agent.agent, jar.agent_name(), "the same version");
    let (old_pid, old_socket) = (old.agent.pid, old.agent.socket.clone());
    drop(old);

    let current = Deployer::new(device.clone(), jar.clone(), records);
    only_ours(&current, &[old_pid])?;
    match current.state().unwrap() {
        AgentState::Incompatible(agent) => assert_eq!(agent.pid, old_pid),
        other => panic!("another build's state is {other:?}"),
    }
    let attached = attach(&current, &options(), &mut leftovers)?;
    let Outcome::Launched(launched) = &attached.outcome else {
        panic!("another build was reused");
    };
    let replaced = launched
        .replaced
        .as_ref()
        .expect("the other build replaced");
    assert_eq!(replaced.pid, old_pid);
    assert_eq!(
        replaced.reason,
        Incompatibility::Build {
            running: bare.build(),
            deploying: jar.build(),
        }
    );
    assert_eq!(replaced.stopped, Stopped::Exited, "shutdown ends it");
    assert!(!agent_alive(device, old_pid, &old_socket));
    match current.state().unwrap() {
        AgentState::Running(agent) => assert_eq!(agent.pid, attached.agent.pid),
        other => panic!("the current build's state is {other:?}"),
    }
    attached.stop(Duration::from_secs(3)).unwrap();
    Ok(())
}

/// `uiautomator events` started on a stream, with its pid, once it holds UI
/// automation; dropping the stream ends it (adbd sends SIGHUP).
fn start_uiautomator_events(device: &Device) -> Result<(ShellStream, u32), Disturbed> {
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
    let running = |device: &Device| {
        device
            .shell(&format!("grep -q uiautomator /proc/{pid}/cmdline"))
            .is_ok_and(|out| out.success())
    };
    let held = (0..100).any(|_| {
        std::thread::sleep(Duration::from_millis(100));
        device
            .shell("dumpsys accessibility")
            .unwrap()
            .stdout_text()
            .contains("Ui Automation")
    });
    if !held || !running(device) {
        return Err(Disturbed(
            "uiautomator events did not take UI automation".into(),
        ));
    }
    Ok((stream, pid))
}

/// Spec scenario "Another tool holds automation" (Single ownership of UI
/// automation), and the force option.
#[test]
fn another_tool_holding_automation_is_a_conflict_naming_it_until_forced() {
    let test = "another_tool_holding_automation_is_a_conflict_naming_it_until_forced";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || tool_conflict(&device));
}

fn tool_conflict(device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    let deployer = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
    let (holder, holder_pid) = start_uiautomator_events(device)?;

    let error = match deployer.attach(&options()) {
        Err(DeployError::Conflict(conflict)) => DeployError::Conflict(conflict),
        Err(other) => return Err(disturbed_by(device, other)),
        Ok(attached) => {
            leftovers.track(attached.agent.pid, &attached.agent.socket);
            return Err(Disturbed(
                "uiautomator events lost the role before the attach".into(),
            ));
        }
    };
    let DeployError::Conflict(conflict) = &error else {
        panic!("expected a conflict, got {error}");
    };
    let named = conflict.holder.as_ref().expect("a likely holder");
    if named.pid != Some(holder_pid) {
        return Err(Disturbed(format!("another holder was named: {named}")));
    }
    assert_eq!(named.name, "uiautomator");
    let message = error.to_string();
    assert!(message.contains("force"), "{message}");
    assert!(
        deployer.running().unwrap().is_empty(),
        "the refused agent stayed"
    );
    eprintln!("{message}");

    let forced = attach(&deployer, &forced(), &mut leftovers)?;
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
    Ok(())
}

/// Single ownership: an agent this host did not start (another daemon's,
/// with its own records) is another holder; force takes over from it.
#[test]
fn an_agent_another_host_started_is_a_conflict_until_forced() {
    let test = "an_agent_another_host_started_is_a_conflict_until_forced";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || foreign_agent(&device));
}

fn foreign_agent(device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    let other_host = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
    let theirs = attach(&other_host, &options(), &mut leftovers)?;

    let ours = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
    let started = Instant::now();
    let error = match ours.attach(&options()) {
        Err(DeployError::Conflict(conflict)) => DeployError::Conflict(conflict),
        Err(other) => return Err(disturbed_by(device, other)),
        Ok(attached) => {
            leftovers.track(attached.agent.pid, &attached.agent.socket);
            return Err(Disturbed("the other host's agent went away".into()));
        }
    };
    let DeployError::Conflict(conflict) = &error else {
        unreachable!("matched above");
    };
    common::under("the conflict", started.elapsed(), Duration::from_secs(1));
    let holder = conflict.holder.as_ref().expect("the other agent named");
    if holder.pid != Some(theirs.agent.pid) {
        return Err(Disturbed(format!("another holder was named: {holder}")));
    }
    assert_eq!(holder.name, "mobile-agent");
    assert!(agent_alive(device, theirs.agent.pid, &theirs.agent.socket));

    let forced = attach(&ours, &forced(), &mut leftovers)?;
    let Outcome::Launched(launched) = &forced.outcome else {
        panic!("force reused the other host's agent");
    };
    assert_eq!(
        launched.took_over.as_ref().map(|h| h.name.as_str()),
        Some("mobile-agent")
    );
    assert!(!agent_alive(device, theirs.agent.pid, &theirs.agent.socket));
    forced.stop(Duration::from_secs(3)).unwrap();
    Ok(())
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

/// The launch detaches the agent: it is reparented to init with SIGHUP and
/// SIGPIPE ignored, so dropping the launch stream and writing to its closed
/// stdout leave it serving. A root peer, which the agent refuses and logs on
/// stdout, makes it write; that part needs `su` (emulator images have it).
#[test]
fn the_agent_outlives_its_launch_stream_and_a_closed_stdout() {
    let test = "the_agent_outlives_its_launch_stream_and_a_closed_stdout";
    let Some(serial) = serial_or_skip(test) else {
        return;
    };
    let _device = device_lock();
    let device = device(&serial);
    retry_when_disturbed(test, &device, || detached(&serial, &device));
}

fn detached(serial: &str, device: &Device) -> Result<(), Disturbed> {
    let mut leftovers = Leftovers::new(device);
    let deployer = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
    let attached = attach(&deployer, &options(), &mut leftovers)?;
    let (pid, socket) = (attached.agent.pid, attached.agent.socket.clone());

    let status = device
        .shell(&format!("cat /proc/{pid}/status"))
        .unwrap()
        .stdout_text();
    let field = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name))
            .map(str::trim)
            .unwrap_or_else(|| panic!("no {name} in {status}"))
            .to_owned()
    };
    assert_eq!(field("PPid:"), "1", "the agent is not reparented to init");
    let ignored = u64::from_str_radix(&field("SigIgn:"), 16).unwrap();
    // Signal n is bit n - 1 of the mask: SIGHUP (1) and SIGPIPE (13).
    const HUP_AND_PIPE: u64 = 0x1001;
    assert_eq!(ignored & HUP_AND_PIPE, HUP_AND_PIPE, "SigIgn {ignored:#x}");

    let root = device.shell("su 0 id -u").unwrap();
    if root.success() && root.stdout_text().trim() == "0" {
        let jar = format!("/data/local/tmp/{}.jar", common::unique_name("peer"));
        push_jar(device, deployer.jar(), &jar).unwrap();
        let out = device
            .shell(&format!(
                "su 0 sh -c 'CLASSPATH={jar} app_process / {} connect-check --socket {socket}'",
                android::agent::MAIN_CLASS
            ))
            .unwrap();
        let _ = device.shell(&format!("rm -f {jar}"));
        let text = out.stdout_text();
        assert!(text.contains("connect-check uid=0 bytes=0"), "{text}");
        std::thread::sleep(Duration::from_millis(300));
    } else {
        eprintln!("no su on {serial}: the agent was not made to write to its closed stdout");
    }
    assert!(agent_alive(device, pid, &socket), "the agent died");
    assert!(snapshot_nodes(device, &attached)? > 0);
    attached.stop(Duration::from_secs(3)).unwrap();
    Ok(())
}
