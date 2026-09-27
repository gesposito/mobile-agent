//! Measurements of the driver, not checks: ignored by default and run with
//!
//! ```sh
//! MOBILE_AGENT_ANDROID_SERIAL=emulator-5554 cargo test --release -p android \
//!   --test measure -- --ignored --nocapture
//! ```
//!
//! On the Compose fixture's Big tab they time launched and reused attaches,
//! the first and warm snapshots through the mirror, taps through the mirror
//! until the tree shows the tab they select (on the agent and on the
//! controller when the emulator has one), and how long a killed agent takes
//! to be reported disconnected. The device must be otherwise quiet.

mod common;

use std::time::{Duration, Instant};

use android::adb::{AdbServer, Device};
use android::agent::{Attached, DeployOptions, Deployer, Records};
use android::emulator::Options;
use android::session::{ControllerChoice, Session, SessionOptions, select_paths};
use common::{
    Fixture, Leftovers, agent_jar, device_lock, launch_compose, percentile, serial_or_skip,
    wait_until_quiet,
};
use mirror::{Action, Degradation, Mirror, MirrorConfig, SnapshotOptions, Target};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// p50, p95 and the maximum of `samples`, in milliseconds.
fn summary(samples: &[f64]) -> String {
    let mut sorted = samples.to_vec();
    format!(
        "p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms (n={})",
        percentile(&mut sorted, 50.0),
        percentile(&mut sorted, 95.0),
        sorted.iter().copied().fold(f64::MIN, f64::max),
        samples.len()
    )
}

fn options() -> DeployOptions {
    DeployOptions {
        idle_exit: Duration::from_secs(10),
        ..DeployOptions::default()
    }
}

fn attach(deployer: &Deployer, leftovers: &mut Leftovers) -> Attached {
    let attached = deployer
        .attach(&options())
        .unwrap_or_else(|e| panic!("{e}"));
    leftovers.track(attached.agent.pid, &attached.agent.socket);
    attached
}

/// Taps the tab `tab` through the mirror and returns how long the tree took
/// to show `shows`.
fn tap_until(mirror: &mut Mirror, tab: &str, shows: &str) -> Duration {
    let snapshot = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    let reference = snapshot
        .elements
        .iter()
        .find(|e| e.identifier.as_deref() == Some(tab))
        .and_then(|e| e.reference)
        .unwrap_or_else(|| panic!("no {tab} in {}", snapshot.text));
    let tapped = Instant::now();
    mirror
        .act(&Action::Tap {
            target: Target::Ref(reference),
            hold: None,
        })
        .unwrap();
    loop {
        let after = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
        if after.text.contains(shows) {
            return tapped.elapsed();
        }
        assert!(
            tapped.elapsed() < Duration::from_secs(5),
            "{tab} never showed {shows}"
        );
        mirror
            .run_until(Instant::now() + Duration::from_millis(20))
            .unwrap();
    }
}

#[test]
#[ignore = "a measurement; run with --ignored on a quiet device"]
fn measure_the_driver() {
    let Some(serial) = serial_or_skip("measure_the_driver") else {
        return;
    };
    let _device = device_lock();
    let device: Device = AdbServer::from_env().device(&serial);
    wait_until_quiet(&device);
    let mut leftovers = Leftovers::new(&device);
    launch_compose(&device, "big").unwrap_or_else(|d| panic!("{}", d.0));
    let _fixture = Fixture(device.clone());

    // Launched attaches: each agent is stopped before the next.
    let (mut total, mut ready) = (Vec::new(), Vec::new());
    for _ in 0..10 {
        let deployer = Deployer::new(device.clone(), agent_jar(), Records::in_memory());
        let attached = attach(&deployer, &mut leftovers);
        total.push(ms(attached.timings.total));
        ready.push(ms(attached.timings.spawn_to_ready.unwrap()));
        attached.stop(Duration::from_secs(3)).unwrap();
    }
    eprintln!("launched attach: {}", summary(&total));
    eprintln!("  launch stream to ready line: {}", summary(&ready));

    // Reused attaches of one running agent.
    let records = Records::in_memory();
    let deployer = Deployer::new(device.clone(), agent_jar(), records.clone());
    drop(attach(&deployer, &mut leftovers));
    let mut reused = Vec::new();
    for _ in 0..10 {
        let attached = attach(&deployer, &mut leftovers);
        reused.push(ms(attached.timings.total));
    }
    eprintln!("reused attach: {}", summary(&reused));

    // The mirror on the agent path.
    let attached = attach(&deployer, &mut leftovers);
    let session = Session::start(attached, SessionOptions::default(), None).unwrap();
    let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());
    let reads = session.reads();
    let started = Instant::now();
    mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    let first = started.elapsed();
    let read = session.last_read().unwrap();
    eprintln!(
        "first snapshot of Compose Big ({} nodes): {:.1} ms, {} tree reads, the last {:.1} ms",
        read.node_count,
        ms(first),
        session.reads() - reads,
        ms(read.took)
    );
    let mut warm = Vec::new();
    while warm.len() < 100 {
        // Only between the Compose window's polls, which read the device.
        if let Some(due) = mirror.next_wakeup()
            && due.saturating_duration_since(Instant::now()) < Duration::from_millis(50)
        {
            mirror.run_until(due + Duration::from_millis(50)).unwrap();
        }
        let reads = session.reads();
        let t = Instant::now();
        mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
        let took = t.elapsed();
        if session.reads() == reads {
            warm.push(ms(took));
        }
    }
    eprintln!(
        "warm snapshot of Compose Big from the replica: {}",
        summary(&warm)
    );

    let (mut to_login, mut to_big) = (Vec::new(), Vec::new());
    for _ in 0..5 {
        to_login.push(ms(tap_until(&mut mirror, "tab-login", "login-username")));
        to_big.push(ms(tap_until(&mut mirror, "tab-big", "cell-500")));
    }
    eprintln!(
        "agent path, tap to tree change, Big to Login: {}",
        summary(&to_login)
    );
    eprintln!(
        "agent path, tap to tree change, Login to Big: {}",
        summary(&to_big)
    );
    session.stop(Duration::from_secs(3)).unwrap();

    // The mirror on the controller, when the emulator has one.
    let selection = select_paths(&serial, true, &ControllerChoice::Auto, Options::default());
    if selection.link.is_some() {
        let attached = attach(&deployer, &mut leftovers);
        let session =
            Session::start_selected(attached, SessionOptions::default(), selection).unwrap();
        let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());
        let (mut to_login, mut to_big) = (Vec::new(), Vec::new());
        for _ in 0..5 {
            to_login.push(ms(tap_until(&mut mirror, "tab-login", "login-username")));
            to_big.push(ms(tap_until(&mut mirror, "tab-big", "cell-500")));
        }
        eprintln!(
            "controller path, tap to tree change, Big to Login: {}",
            summary(&to_login)
        );
        eprintln!(
            "controller path, tap to tree change, Login to Big: {}",
            summary(&to_big)
        );
        eprintln!("{}", session.status());
        session.stop(Duration::from_secs(3)).unwrap();
    } else {
        eprintln!("no controller: {:?}", selection.paths.warning);
    }

    // A killed agent, reported disconnected.
    let mut detected = Vec::new();
    for _ in 0..5 {
        let attached = attach(&deployer, &mut leftovers);
        let session = Session::start(attached, SessionOptions::default(), None).unwrap();
        let mut mirror = Mirror::new(session.backend(), MirrorConfig::default());
        mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
        let (pid, socket) = (session.agent().pid, session.agent().socket.clone());
        let killed = Instant::now();
        device
            .shell(&format!(
                "grep -qF {socket} /proc/{pid}/cmdline && kill -9 {pid}"
            ))
            .unwrap();
        loop {
            let pumped = mirror.run_until(Instant::now() + Duration::from_millis(5));
            let gone = pumped.is_err()
                || mirror
                    .snapshot(&SnapshotOptions::interactive())
                    .is_ok_and(|s| {
                        s.verdict
                            .reasons
                            .iter()
                            .any(|r| matches!(r, Degradation::Disconnected(_)))
                    });
            if gone {
                detected.push(ms(killed.elapsed()));
                break;
            }
            assert!(killed.elapsed() < Duration::from_secs(5));
        }
    }
    eprintln!("killed agent to disconnected: {}", summary(&detected));
}
