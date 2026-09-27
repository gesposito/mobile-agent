//! Task 6.6: the foreground tree, read by a reader spawned into the
//! Simulator (simulator-driver spec, "Foreground tree read" and "No test
//! runner and no per-app build"; design D6, spike S4).

#![cfg(target_os = "macos")]

mod common;

use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use protocol::snapshot::{NodeFlags, WindowType};
use simulator::{
    DriverOptions, Frames, Input, Lifecycle, ReaderState, SimulatorDriver, Tree, TreeBackend,
    TreeChoice, TreeOptions,
};

use common::{ExpoOnLogin, SWIFTUI_BUNDLE};

/// Where the SwiftUI fixture's Login screen puts its elements on an iPhone
/// 17, in points, as the reader read them in spike S4 and on this host.
const USERNAME: [f64; 4] = [16.0, 78.0, 370.0, 34.0];
const PASSWORD: [f64; 4] = [16.0, 128.0, 370.0, 34.0];
const SIGN_IN: [f64; 4] = [16.0, 178.0, 75.33, 34.33];
const TAB_LOGIN: [f64; 4] = [25.0, 795.0, 77.0, 54.0];

fn has(tree: &simulator::ForegroundTree, identifier: &str) -> bool {
    tree.find(identifier).is_some()
}

#[test]
fn swiftui_read_returns_identifiers_and_frames() {
    let Some(udid) = common::gated("swiftui_read_returns_identifiers_and_frames") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };

    let pid = common::show_swiftui(&driver, &udid, "login");
    // Until the process a relaunch replaced is gone, it can still own the
    // foreground, so the read waits for the launched one.
    let login = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        tree.foreground.pid == pid && has(tree, "login-status")
    });
    assert_eq!(login.backend, TreeBackend::Reader);
    assert!(!login.is_degraded(), "{:?}", login.degraded);
    assert_eq!(login.foreground.pid, pid);
    assert_eq!(login.foreground.bundle_id.as_deref(), Some(SWIFTUI_BUNDLE));
    assert!(!login.foreground.system);
    assert_eq!(login.alert, None);
    let window = &login.snapshot.windows[0];
    assert_eq!(window.kind, WindowType::APPLICATION);
    assert_eq!(window.title.as_deref(), Some("FixtureSwiftUI"));

    for (identifier, class, frame) in [
        ("login-username", "XCUIElementTypeTextField", USERNAME),
        ("login-password", "XCUIElementTypeSecureTextField", PASSWORD),
        ("login-submit", "XCUIElementTypeButton", SIGN_IN),
        ("tab-login", "XCUIElementTypeButton", TAB_LOGIN),
    ] {
        let node = login.find(identifier).unwrap_or_else(|| {
            panic!("{identifier} is missing: {:?}", common::identifiers(&login))
        });
        assert_eq!(login.class_of(node), class, "{identifier}");
        assert!(
            common::close(login.frame(node), frame, 1.0),
            "{identifier} is at {:?}, not {frame:?}",
            login.frame(node)
        );
        assert!(node.flags.contains(NodeFlags::VISIBLE), "{identifier}");
    }
    let username = login.find("login-username").unwrap();
    assert_eq!(username.desc.as_deref(), Some("Username"));
    assert_eq!(username.text, None, "an empty field has no text");
    assert!(username.flags.contains(NodeFlags::EDITABLE));
    let password = login.find("login-password").unwrap();
    assert!(password.flags.contains(NodeFlags::PASSWORD));
    assert_eq!(
        login.find("login-submit").unwrap().desc.as_deref(),
        Some("Sign in")
    );
    assert_eq!(
        login.find("login-status").unwrap().desc.as_deref(),
        Some("Signed out")
    );
    for tab in ["login", "list", "big", "anim", "system"] {
        assert!(has(&login, &format!("tab-{tab}")), "tab-{tab}");
    }
    assert!(
        login
            .find("tab-login")
            .unwrap()
            .flags
            .contains(NodeFlags::SELECTED)
    );

    let mut reads = Vec::new();
    for _ in 0..5 {
        let read = driver
            .snapshot(&udid, &TreeOptions::default())
            .expect("a warm read");
        reads.push(read.elapsed);
    }
    reads.sort();
    eprintln!(
        "SwiftUI Login: {} nodes, warm reads {:?} to {:?}",
        login.snapshot.node_count, reads[0], reads[4]
    );

    // The 500-cell screen: every cell with its identifier and a frame.
    common::show_swiftui(&driver, &udid, "big");
    let big = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        has(tree, "cell-500")
    });
    let cells: Vec<_> = big
        .nodes()
        .into_iter()
        .map(|(_, node)| node)
        .filter(|node| {
            node.res_id
                .as_deref()
                .is_some_and(|id| id.starts_with("cell-"))
        })
        .collect();
    assert_eq!(cells.len(), 500);
    assert!(cells.iter().all(|cell| {
        let [_, _, width, height] = big.frame(cell);
        width > 0.0 && height > 0.0
    }));
    assert!(has(&big, "big-count"));
    let mut big_reads = Vec::new();
    for _ in 0..5 {
        big_reads.push(
            driver
                .snapshot(&udid, &TreeOptions::default())
                .expect("a warm read")
                .elapsed,
        );
    }
    big_reads.sort();
    eprintln!(
        "SwiftUI Big: {} nodes, warm reads {:?} to {:?}",
        big.snapshot.node_count, big_reads[0], big_reads[4]
    );
    driver.terminate(&udid, SWIFTUI_BUNDLE).expect("terminated");
}

#[test]
fn a_permission_alert_is_reported_as_system_owned() {
    let Some(udid) = common::gated("a_permission_alert_is_reported_as_system_owned") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::reset_location(&udid, SWIFTUI_BUNDLE);
    common::show_swiftui(&driver, &udid, "system");
    let screen = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        has(tree, "system-permission")
    });
    let request = screen.center(screen.find("system-permission").unwrap());
    driver
        .tap(&udid, request)
        .expect("a tap on the request button");

    let alert = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        tree.foreground.system && tree.alert.is_some()
    });
    eprintln!(
        "permission alert: {} (pid {}), {:?}",
        alert.foreground.name, alert.foreground.pid, alert.alert
    );
    assert_eq!(
        alert.foreground.bundle_id.as_deref(),
        Some("com.apple.springboard"),
        "system UI owns the foreground"
    );
    assert_eq!(alert.snapshot.windows[0].kind, WindowType::SYSTEM);
    assert!(!alert.is_degraded(), "{:?}", alert.degraded);
    let found = alert.alert.as_ref().unwrap();
    assert!(
        found
            .title
            .as_deref()
            .is_some_and(|title| title.contains("FixtureSwiftUI")),
        "{found:?}"
    );
    let buttons: Vec<&str> = found.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(
        buttons,
        ["Allow Once", "Allow While Using App", "Don’t Allow"]
    );

    // Dismissing it hands the foreground back to the application. The
    // alert is in the tree as soon as its presentation starts, and UIKit
    // ignores touches until the animation ends, so the display settles first.
    common::settle(&driver, &udid);
    let deny = found
        .buttons
        .iter()
        .find(|b| b.label == "Don’t Allow")
        .unwrap();
    driver
        .tap(&udid, deny.center)
        .expect("a tap on Don’t Allow");
    let back = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        !tree.foreground.system
            && tree
                .find("system-permission-status")
                .and_then(|node| node.desc.as_deref())
                == Some("Permission: denied")
    });
    assert_eq!(back.foreground.bundle_id.as_deref(), Some(SWIFTUI_BUNDLE));
    common::reset_location(&udid, SWIFTUI_BUNDLE);
    driver.terminate(&udid, SWIFTUI_BUNDLE).expect("terminated");
}

#[test]
fn in_app_alert_content_is_cut() {
    let Some(udid) = common::gated("in_app_alert_content_is_cut") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_swiftui(&driver, &udid, "system");
    let screen = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        has(tree, "system-alert")
    });
    let open = screen.center(screen.find("system-alert").unwrap());
    driver.tap(&udid, open).expect("a tap on the alert button");

    let covered = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        tree.alert.is_some()
    });
    assert!(!covered.foreground.system, "the application owns its alert");
    let alert = covered.alert.as_ref().unwrap();
    assert_eq!(alert.title.as_deref(), Some("In-app alert"));
    assert_eq!(
        alert.message.as_deref(),
        Some("This alert is presented by the app.")
    );
    assert_eq!(alert.buttons.len(), 1, "{:?}", alert.buttons);
    assert_eq!(alert.buttons[0].label, "OK");
    assert_eq!(
        alert.buttons[0].identifier.as_deref(),
        Some("inapp-alert-ok")
    );
    assert!(
        has(&covered, "system-permission"),
        "the payload keeps what the alert covers, for the mirror"
    );
    let uncovered = covered.uncovered();
    let kept: Vec<Option<&str>> = uncovered
        .windows
        .iter()
        .filter_map(|window| window.root.as_ref())
        .flat_map(|root| {
            let mut ids = Vec::new();
            let mut stack = vec![root];
            while let Some(node) = stack.pop() {
                ids.push(node.res_id.as_deref());
                stack.extend(node.children.iter());
            }
            ids
        })
        .collect();
    assert!(kept.contains(&Some("inapp-alert-ok")));
    for covered_id in ["system-permission", "system-alert", "tab-login"] {
        assert!(!kept.contains(&Some(covered_id)), "{covered_id} is covered");
    }
    eprintln!(
        "in-app alert: {} nodes read, {} left once covered content is cut",
        covered.snapshot.node_count, uncovered.node_count
    );

    // UIKit ignores touches until the alert's presentation ends.
    common::settle(&driver, &udid);
    driver
        .tap(&udid, alert.buttons[0].center)
        .expect("a tap on OK");
    common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        tree.alert.is_none() && has(tree, "system-alert")
    });
    driver.terminate(&udid, SWIFTUI_BUNDLE).expect("terminated");
}

#[test]
fn target_status_reports_the_reader_and_its_readiness() {
    let Some(udid) = common::gated("target_status_reports_the_reader_and_its_readiness") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    let before = common::installed_bundles(&udid);
    common::show_swiftui(&driver, &udid, "login");
    assert_eq!(driver.status(&udid).expect("status").tree, None);
    common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        has(tree, "login-status")
    });

    let status = driver.status(&udid).expect("status");
    let tree = status.tree.expect("the tree status");
    assert_eq!(tree.backend, TreeBackend::Reader);
    let first = tree.reader.expect("the reader");
    assert!(
        first.name.starts_with("simulator-reader "),
        "{}",
        first.name
    );
    assert_eq!(first.state, ReaderState::Ready);
    assert!(first.pid > 0);
    let ready = first.ready_in.expect("a readiness time");
    eprintln!(
        "reader {} (pid {}) from {}: ready {ready:?} after its spawn; {}",
        first.name,
        first.pid,
        first.path.display(),
        tree.reason
    );
    // It runs inside the Simulator, from its path on the host.
    let command = Command::new("ps")
        .args(["-o", "comm=", "-p", &first.pid.to_string()])
        .output()
        .expect("ps runs");
    assert_eq!(
        String::from_utf8_lossy(&command.stdout).trim(),
        first.path.to_string_lossy()
    );

    // A second reader, spawned from the same file, is ready within the
    // spec's 500 ms on a quiet host.
    common::wait_for_quiet_host(6.0, Duration::from_secs(300));
    let mut warm = Vec::new();
    for _ in 0..3 {
        driver.reset_target(&udid).expect("reset");
        driver
            .snapshot(&udid, &TreeOptions::default())
            .expect("a read through a new reader");
        let reader = driver
            .status(&udid)
            .expect("status")
            .tree
            .and_then(|tree| tree.reader)
            .expect("the new reader");
        assert_ne!(reader.pid, first.pid);
        warm.push(reader.ready_in.expect("a readiness time"));
    }
    warm.sort();
    eprintln!("warm spawns ready in {warm:?}");
    assert!(
        warm[1] < Duration::from_millis(500),
        "the median warm spawn took {:?}",
        warm[1]
    );

    // Nothing was installed.
    assert_eq!(common::installed_bundles(&udid), before);
    driver.terminate(&udid, SWIFTUI_BUNDLE).expect("terminated");
}

#[test]
fn fresh_simulator() {
    let Some(_) = common::gated("fresh_simulator") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    // Created, booted, and the prebuilt fixtures installed: no build tool
    // runs, and the reader needs no installation.
    let udid = common::fresh_device(&driver);
    common::show_swiftui(&driver, &udid, "login");
    let start = Instant::now();
    let login = common::snapshot_until(&driver, &udid, Duration::from_secs(20), |tree| {
        has(tree, "tab-list")
    });
    eprintln!(
        "fresh Simulator: first usable tree {:?} after the launch returned, from {}",
        start.elapsed(),
        login.backend
    );
    assert_eq!(login.backend, TreeBackend::Reader);

    let stream = driver
        .frames(&udid, &simulator::FrameOptions::default())
        .expect("frames");
    assert!(stream.next(Duration::from_secs(2)).is_some(), "a frame");
    let tab = login.center(login.find("tab-list").unwrap());
    driver.tap(&udid, tab).expect("a tap on the List tab");
    let list = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        has(tree, "row-1")
    });
    assert!(
        list.find("tab-list")
            .unwrap()
            .flags
            .contains(NodeFlags::SELECTED)
    );
    assert!(
        stream.next(Duration::from_secs(2)).is_some(),
        "the tap drew"
    );
    let status = driver.status(&udid).expect("status");
    assert!(status.tree.and_then(|tree| tree.reader).is_some());
}

#[test]
fn sparse_tree_flagged() {
    let Some(udid) = common::gated("sparse_tree_flagged") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open_with(DriverOptions {
        tree: TreeChoice::Only(TreeBackend::Translator),
        ..DriverOptions::default()
    })
    .expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    // Right after a launch the translator reports the application without
    // children for seconds while the screen already shows it (spike S4).
    // Keep the first such read taken while the screen shows content.
    let mut sparse = None;
    for _ in 0..5 {
        common::show_swiftui(&driver, &udid, "login");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(4) && sparse.is_none() {
            if let Ok(tree) = driver.snapshot(&udid, &TreeOptions::default())
                && tree.foreground.bundle_id.as_deref() == Some(SWIFTUI_BUNDLE)
                && tree.snapshot.node_count <= 1
            {
                let populated =
                    common::spread(&common::raw_screenshot(&driver, &udid), (60.0, 780.0));
                if populated > 2.0 {
                    sparse = Some((tree, populated));
                }
            }
        }
        if sparse.is_some() {
            break;
        }
    }
    let (sparse, populated) = sparse
        .expect("a read that held only the application element while the screen was populated");
    eprintln!(
        "sparse read: {:?}; the screen shows the fixture (brightness spread {populated:.1})",
        sparse.degraded
    );
    assert!(sparse.is_degraded());
    let reasons: Vec<&str> = sparse.degraded.iter().map(|d| d.code()).collect();
    assert!(reasons.contains(&"sparse_tree"), "{reasons:?}");
    assert!(reasons.contains(&"fallback_backend"), "{reasons:?}");
    let text: Vec<String> = sparse.degraded.iter().map(ToString::to_string).collect();
    assert!(
        text.iter()
            .any(|reason| reason.contains("only its application element")),
        "{text:?}"
    );

    // Once the application answers, the read is no longer sparse, though
    // the translator still marks it degraded.
    let full = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        has(tree, "login-username")
    });
    let reasons: Vec<&str> = full.degraded.iter().map(|d| d.code()).collect();
    assert_eq!(reasons, ["fallback_backend"]);
    driver.terminate(&udid, SWIFTUI_BUNDLE).expect("terminated");
}

#[test]
fn the_translator_stands_in_when_the_reader_cannot_run() {
    let Some(udid) = common::gated("the_translator_stands_in_when_the_reader_cannot_run") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open_with(DriverOptions {
        reader: Some("/nonexistent/simulator-reader".into()),
        ..DriverOptions::default()
    })
    .expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_swiftui(&driver, &udid, "login");
    let tree = common::snapshot_until(&driver, &udid, Duration::from_secs(10), |tree| {
        has(tree, "login-username")
    });
    assert_eq!(tree.backend, TreeBackend::Translator);
    let fallback = tree
        .degraded
        .iter()
        .find(|reason| reason.code() == "fallback_backend")
        .expect("marked as the fallback");
    eprintln!("{fallback}");
    assert!(fallback.to_string().contains("/nonexistent"), "{fallback}");
    let status = driver.status(&udid).expect("status").tree.expect("tree");
    assert_eq!(status.backend, TreeBackend::Translator);
    assert!(
        status.reason.contains("reader cannot run"),
        "{}",
        status.reason
    );
    driver.terminate(&udid, SWIFTUI_BUNDLE).expect("terminated");
}

#[test]
fn each_simulator_has_its_own_reader() {
    let Some(shared) = common::gated("each_simulator_has_its_own_reader") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let fresh = common::fresh_device(&driver);
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: shared.clone(),
    };
    common::show_swiftui(&driver, &shared, "big");
    common::show_swiftui(&driver, &fresh, "login");
    common::snapshot_until(&driver, &shared, Duration::from_secs(10), |t| {
        has(t, "cell-500")
    });
    common::snapshot_until(&driver, &fresh, Duration::from_secs(20), |t| {
        has(t, "login-status")
    });

    // Both read at once, each its own screen.
    let (big, login) = thread::scope(|scope| {
        let (driver, shared, fresh) = (&driver, &shared, &fresh);
        let one = scope.spawn(move || driver.snapshot(shared, &TreeOptions::default()));
        let two = scope.spawn(move || driver.snapshot(fresh, &TreeOptions::default()));
        (
            one.join()
                .expect("a thread")
                .expect("the shared Simulator's tree"),
            two.join()
                .expect("a thread")
                .expect("the fresh Simulator's tree"),
        )
    });
    assert!(has(&big, "cell-500") && !has(&big, "login-status"));
    assert!(has(&login, "login-status") && !has(&login, "cell-500"));
    let reader = |udid: &str| {
        driver
            .status(udid)
            .expect("status")
            .tree
            .and_then(|tree| tree.reader)
            .expect("a reader")
    };
    assert_ne!(reader(&shared).pid, reader(&fresh).pid);
    driver
        .terminate(&shared, SWIFTUI_BUNDLE)
        .expect("terminated");
}

/// A build without the reader reads through the translator from its first
/// read on, within a bounded time: the Bluesky smoke test's daemon, built
/// without it, hung on that read for good (tasks.md 6.11). Run with
/// `--no-default-features --features turbojpeg`. The read runs on a thread
/// of its own, so a read that hangs fails the test instead of hanging it.
#[cfg(not(feature = "reader"))]
#[test]
fn a_build_without_the_reader_reads_through_the_translator() {
    let Some(udid) = common::gated("a_build_without_the_reader_reads_through_the_translator")
    else {
        return;
    };
    let _serial = common::exclusive();
    let driver = std::sync::Arc::new(SimulatorDriver::open().expect("the driver opens"));
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    common::show_swiftui(&driver, &udid, "login");
    let (done, finished) = std::sync::mpsc::channel();
    {
        let (driver, udid) = (std::sync::Arc::clone(&driver), udid.clone());
        thread::spawn(move || {
            let start = Instant::now();
            let tree = loop {
                match driver.snapshot(&udid, &TreeOptions::default()) {
                    Ok(tree) if has(&tree, "login-username") => break Ok(tree),
                    Err(error) if !error.retryable() => break Err(error),
                    _ if start.elapsed() > Duration::from_secs(20) => {
                        break Err(simulator::Error::Timeout {
                            what: "a full translator read".into(),
                            after: start.elapsed(),
                        });
                    }
                    _ => thread::sleep(Duration::from_millis(100)),
                }
            };
            let _ = done.send(tree);
        });
    }
    let tree = finished
        .recv_timeout(Duration::from_secs(30))
        .expect("the read returned instead of hanging")
        .expect("the translator read the screen");
    assert_eq!(tree.backend, TreeBackend::Translator);
    let fallback = tree
        .degraded
        .iter()
        .find(|reason| reason.code() == "fallback_backend")
        .expect("marked as the fallback");
    eprintln!("{fallback}");
    assert!(
        fallback.to_string().contains("`reader` feature"),
        "{fallback}"
    );
    let status = driver.status(&udid).expect("status").tree.expect("tree");
    assert_eq!(status.backend, TreeBackend::Translator);
    assert_eq!(status.reader, None, "nothing was spawned");
    driver.terminate(&udid, SWIFTUI_BUNDLE).expect("terminated");
}
