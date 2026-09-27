//! Task 4.2: UI automation ownership (spec android-driver, Single ownership
//! of UI automation; design D2).

use std::time::Duration;

use agent_android_host_tests::agent::fields;
use agent_android_host_tests::holder::{UiautomatorEvents, wait_role_free};
use agent_android_host_tests::{Agent, Launch, device, fixture};
use protocol::message::{Call, Reply};

#[test]
fn the_agent_holds_ui_automation_with_interactive_windows_once_ready() {
    let Some(dev) = device("the_agent_holds_ui_automation_with_interactive_windows_once_ready")
    else {
        return;
    };
    wait_role_free(&dev, Duration::from_secs(60));
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let ready = &agent.ready;
    assert_eq!(ready["ownership"], "held", "{ready:?}");
    // Two interactive windows at least, the status bar and the app, which the
    // window list only reports once FLAG_RETRIEVE_INTERACTIVE_WINDOWS is set.
    let windows: u32 = ready["windows"].parse().expect("a window count");
    assert!(windows >= 2, "{windows} interactive windows at ready");
    assert_eq!(ready["active"], fixture::COMPOSE, "{ready:?}");
    assert!(dev.ui_automation_registered());
    println!(
        "attach: ready {:.1} ms after spawn; connect {} ms, active window after {} ms",
        agent_android_host_tests::ms(agent.ready_after),
        ready["connect_ms"],
        ready["window_wait_ms"]
    );
    let mut client = agent.connect();
    let lifecycle = client.wait_event(Duration::from_secs(5), |_| true);
    assert!(lifecycle.is_some());
    assert_eq!(client.lifecycle()[0].0, "ready");
    assert!(
        client.lifecycle()[0].1.contains("UI automation held"),
        "{:?}",
        client.lifecycle()
    );
}

#[test]
fn an_attach_while_uiautomator_events_holds_the_role_fails_naming_it() {
    let Some(dev) = device("an_attach_while_uiautomator_events_holds_the_role_fails_naming_it")
    else {
        return;
    };
    let holder = UiautomatorEvents::start(&dev);
    let mut agent = Agent::spawn(&dev, Launch::default());
    let line = agent
        .wait_line(Duration::from_secs(20), |l| l.starts_with("conflict "))
        .unwrap_or_else(|| panic!("no conflict report:\n{}", agent.transcript()));
    agent.adopt_pid(&line);
    let report = fields(&line);
    assert_eq!(report["holder"], "uiautomator", "{line}");
    assert_eq!(report["holder_pid"], holder.pid.to_string(), "{line}");
    let message = line.split_once("message=").map(|(_, m)| m).unwrap_or("");
    assert!(
        message.contains("uiautomator") && message.contains(&holder.pid.to_string()),
        "{message}"
    );
    assert!(message.contains("--force"), "{message}");
    let status = agent
        .wait_exit(Duration::from_secs(10))
        .unwrap_or_else(|| panic!("the refused agent kept running:\n{}", agent.transcript()));
    assert_eq!(status.code(), Some(3), "{}", agent.transcript());
    assert!(holder.alive(), "the agent ended the holder without --force");
}

#[test]
fn a_second_agent_names_the_first_as_the_holder() {
    let Some(dev) = device("a_second_agent_names_the_first_as_the_holder") else {
        return;
    };
    wait_role_free(&dev, Duration::from_secs(60));
    let first = Agent::launch(&dev, Launch::default());
    let mut second = Agent::spawn(&dev, Launch::default());
    let line = second
        .wait_line(Duration::from_secs(20), |l| l.starts_with("conflict "))
        .unwrap_or_else(|| panic!("no conflict report:\n{}", second.transcript()));
    second.adopt_pid(&line);
    let report = fields(&line);
    assert_eq!(report["holder_pid"], first.pid.to_string(), "{line}");
    assert!(line.contains(&first.socket), "{line}");
    second.wait_exit(Duration::from_secs(10)).expect("exit");
    assert!(first.alive());
}

#[test]
fn force_takes_the_role_over_from_uiautomator_events() {
    let Some(dev) = device("force_takes_the_role_over_from_uiautomator_events") else {
        return;
    };
    let holder = UiautomatorEvents::start(&dev);
    let agent = Agent::launch(&dev, Launch::default().arg("--force"));
    assert_eq!(agent.ready["ownership"], "held", "{:?}", agent.ready);
    assert_eq!(agent.ready["forced"], format!("uiautomator:{}", holder.pid));
    assert!(!holder.alive(), "the holder survived --force");
    let mut client = agent.connect();
    let lifecycle = client.wait_event(Duration::from_secs(5), |_| true);
    assert!(lifecycle.is_some());
    assert!(
        client.lifecycle()[0].1.contains("took UI automation over"),
        "{:?}",
        client.lifecycle()
    );
}

#[test]
fn shutdown_releases_the_role_for_the_next_owner() {
    let Some(dev) = device("shutdown_releases_the_role_for_the_next_owner") else {
        return;
    };
    wait_role_free(&dev, Duration::from_secs(60));
    let mut agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    assert_eq!(client.call(Call::Shutdown).expect("shutdown"), Reply::Empty);
    agent.wait_exit(Duration::from_secs(5)).expect("exit");
    assert!(
        !dev.ui_automation_registered(),
        "the role outlived the agent"
    );
    let next = Agent::launch(&dev, Launch::default());
    assert_eq!(next.ready["ownership"], "held");
}
