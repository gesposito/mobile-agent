//! Task 4.1: the entry point, the abstract-socket server and the hello-first
//! handshake through the protocol crate (spec device-protocol, design D3).

use std::time::{Duration, Instant};

use agent_android_host_tests::{Agent, Launch, device, ms};
use protocol::capability::Capabilities;
use protocol::code;
use protocol::message::{Call, FramesParams, Method, Reply};
use protocol::session::{DisconnectReason, HandshakeError, HostConfig};

#[test]
fn the_agent_speaks_first_with_a_hello_naming_protocol_1_and_its_token() {
    let Some(dev) = device("the_agent_speaks_first_with_a_hello_naming_protocol_1_and_its_token")
    else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default());
    assert_eq!(agent.ready["socket"], agent.socket);
    assert_eq!(
        agent.ready["uid"], "2000",
        "the agent runs as the shell user"
    );
    let t0 = Instant::now();
    let mut client = agent.open(agent.token.clone());
    let negotiated = client
        .handshake(Duration::from_secs(10))
        .unwrap_or_else(|e| panic!("{e}\n{}", agent.transcript()));
    let handshake = t0.elapsed();
    assert_eq!(negotiated.version, 1);
    assert_eq!(negotiated.agent_versions, vec![1]);
    let version = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../VERSION"))
        .expect("read VERSION");
    assert_eq!(
        negotiated.agent,
        format!("mobile-agent-android {}", version.trim())
    );
    println!(
        "attach: ready line {:.1} ms after spawn, handshake {:.1} ms",
        ms(agent.ready_after),
        ms(handshake)
    );
}

#[test]
fn a_host_expecting_another_token_refuses_the_hello_and_the_agent_keeps_serving() {
    let Some(dev) =
        device("a_host_expecting_another_token_refuses_the_hello_and_the_agent_keeps_serving")
    else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default());
    let mut stranger = agent.open(Some("placeholder-other-token".to_owned()));
    let refused = stranger.handshake(Duration::from_secs(10));
    assert!(
        refused.is_err(),
        "the host accepted a hello with another token"
    );
    assert_eq!(
        stranger.closed(),
        Some(&DisconnectReason::Handshake(HandshakeError::TokenMismatch))
    );
    let mut client = agent.connect();
    assert!(client.negotiated().is_some());
    let reply = client.call(Call::Unknown(Method(999)));
    assert!(reply.is_err());
    assert!(agent.alive());
}

#[test]
fn an_unknown_method_gets_a_permanent_error_and_the_connection_stays_usable() {
    let Some(dev) =
        device("an_unknown_method_gets_a_permanent_error_and_the_connection_stays_usable")
    else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    for _ in 0..2 {
        let error = client
            .call(Call::Unknown(Method(999)))
            .expect_err("an unknown method succeeded");
        assert_eq!(error.code, code::UNKNOWN_METHOD);
        assert!(!error.retryable);
        assert!(error.message.contains("999"), "{}", error.message);
    }
}

#[test]
fn the_agent_pings_a_silent_host_every_second() {
    let Some(dev) = device("the_agent_pings_a_silent_host_every_second") else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default());
    // A host that answers pings but sends none of its own for 10 s.
    let mut client = agent_android_host_tests::Client::with_config(
        agent.device(),
        &agent.socket,
        HostConfig {
            token: agent.token.clone(),
            ping_interval: Duration::from_secs(10),
            peer_timeout: Duration::from_secs(30),
            ..HostConfig::default()
        },
    )
    .expect("open the socket");
    client
        .handshake(Duration::from_secs(10))
        .expect("handshake");
    client.idle(Duration::from_millis(2600));
    assert!(
        client.pings >= 2,
        "{} pings in 2.6 s of host silence",
        client.pings
    );
    assert!(
        client.closed().is_none(),
        "the session closed: {:?}",
        client.closed()
    );
}

#[test]
fn a_later_connection_reuses_the_running_agent() {
    let Some(dev) = device("a_later_connection_reuses_the_running_agent") else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default());
    let first = agent.connect();
    first.disconnect();
    std::thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();
    let mut second = agent.connect();
    println!("reattach to a running agent: {:.1} ms", ms(t0.elapsed()));
    assert!(second.negotiated().is_some());
    assert!(agent.alive(), "the agent exited between connections");
    let running = dev.processes_matching(&agent.socket);
    assert_eq!(
        running.iter().filter(|l| !l.contains("timeout")).count(),
        1,
        "exactly one agent process serves the socket: {running:?}"
    );
    let _ = second.call(Call::Unknown(Method(999)));
}

#[test]
fn shutdown_is_answered_before_the_agent_exits() {
    let Some(dev) = device("shutdown_is_answered_before_the_agent_exits") else {
        return;
    };
    let mut agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let reply = client.call(Call::Shutdown).expect("shutdown failed");
    assert_eq!(reply, Reply::Empty);
    let status = agent
        .wait_exit(Duration::from_secs(5))
        .unwrap_or_else(|| panic!("the agent did not exit:\n{}", agent.transcript()));
    assert!(status.success(), "exit status {status}");
}

#[test]
fn a_host_speaking_only_a_newer_version_is_refused_and_the_agent_exits_for_replacement() {
    let Some(dev) = device(
        "a_host_speaking_only_a_newer_version_is_refused_and_the_agent_exits_for_replacement",
    ) else {
        return;
    };
    let mut agent = Agent::launch(&dev, Launch::default());
    let mut client = agent_android_host_tests::Client::with_config(
        agent.device(),
        &agent.socket,
        HostConfig {
            versions: vec![2],
            token: agent.token.clone(),
            ..HostConfig::default()
        },
    )
    .expect("open the socket");
    let refused = client.handshake(Duration::from_secs(10));
    let reason = refused.expect_err("a host speaking only version 2 accepted the agent");
    assert!(reason.contains("[2]") && reason.contains("[1]"), "{reason}");
    agent
        .wait_exit(Duration::from_secs(5))
        .unwrap_or_else(|| panic!("the refused agent kept running:\n{}", agent.transcript()));
}

#[test]
fn a_peer_that_is_not_the_shell_user_gets_no_hello() {
    let Some(dev) = device("a_peer_that_is_not_the_shell_user_gets_no_hello") else {
        return;
    };
    // The peer runs as root through su, which userdebug builds have. An app's
    // uid cannot serve: app_process aborts in an app's SELinux domain.
    if !dev.shell_output("su 0 true").status.success() {
        println!(
            "skipping the peer check: su is not available on {}",
            dev.serial
        );
        return;
    }
    let mut agent = Agent::launch(&dev, Launch::default());
    let jar = format!("/data/local/tmp/mobile-agent-peer-check-{}.jar", agent.pid);
    dev.push(&agent_android_host_tests::agent::jar_path(), &jar);
    let out = dev.shell_output(&format!(
        "su 0 sh -c 'CLASSPATH={jar} app_process / dev.mobileagent.agent.Main connect-check --socket {}'",
        agent.socket
    ));
    dev.shell(&format!("rm -f {jar}"));
    let text = String::from_utf8_lossy(&out.stdout).replace('\r', "");
    let line = text
        .lines()
        .find(|l| l.starts_with("connect-check "))
        .unwrap_or_else(|| panic!("the root peer printed nothing: {text}"));
    assert!(
        line.contains("uid=0 bytes=0"),
        "the root peer received bytes: {line}"
    );
    let refusal = agent
        .wait_line(Duration::from_secs(2), |l| l.starts_with("refused "))
        .unwrap_or_else(|| panic!("the agent logged no refusal:\n{}", agent.transcript()));
    assert!(refusal.contains("uid=0 "), "{refusal}");
    let mut client = agent.connect();
    assert!(client.call(Call::Unknown(Method(999))).is_err());
}

#[test]
fn the_agent_closes_a_connection_whose_host_stops_answering() {
    let Some(dev) = device("the_agent_closes_a_connection_whose_host_stops_answering") else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    // Neither read nor answer: the agent's pings go unanswered.
    std::thread::sleep(Duration::from_millis(4500));
    client.idle(Duration::from_millis(500));
    assert!(
        client.closed().is_some(),
        "the agent kept a silent host's connection open for 4.5 s"
    );
    assert!(
        agent.alive(),
        "the agent exited instead of waiting for a host"
    );
}

#[test]
fn every_ping_answer_keeps_a_quiet_session_open_past_the_peer_timeout() {
    let Some(dev) = device("every_ping_answer_keeps_a_quiet_session_open_past_the_peer_timeout")
    else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    client.idle(Duration::from_millis(4000));
    assert!(client.closed().is_none(), "closed: {:?}", client.closed());
    let error = client
        .call(Call::Unknown(Method(999)))
        .expect_err("unknown method");
    assert_eq!(error.code, code::UNKNOWN_METHOD);
}

#[test]
fn the_hello_advertises_every_capability_this_build_implements() {
    let Some(dev) = device("the_hello_advertises_every_capability_this_build_implements") else {
        return;
    };
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let negotiated = client.negotiated().expect("negotiated").clone();
    let expected = Capabilities::TREE
        | Capabilities::INPUT
        | Capabilities::SCREENSHOT
        | Capabilities::EVENTS
        | Capabilities::TEXT_SET
        | Capabilities::BOUNDS;
    assert_eq!(
        negotiated.capabilities, expected,
        "{}",
        negotiated.capabilities
    );
    // Frame streaming is a later change (the emulator's controller serves it).
    assert!(!negotiated.capabilities.contains(Capabilities::FRAMES));
    let error = client
        .call(Call::FramesStart(FramesParams::default()))
        .expect_err("frames_start was served");
    assert_eq!(error.code, code::CAPABILITY_MISSING);
    assert!(
        error.message.contains("frame streaming"),
        "{}",
        error.message
    );
    // Its lifecycle events name the role and any capability this device lost.
    let ready = client.wait_event(Duration::from_secs(5), |_| true);
    assert!(ready.is_some());
    let lifecycle = client.lifecycle();
    assert_eq!(lifecycle[0].0, "ready");
    assert!(
        lifecycle.iter().all(|(kind, _)| kind != "warning"),
        "{lifecycle:?}"
    );
}
