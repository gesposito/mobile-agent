//! Task 4.1: the entry point, the abstract-socket server and the hello-first
//! handshake through the protocol crate (spec device-protocol, design D3).

use std::time::{Duration, Instant};

use agent_android_host_tests::{Agent, Launch, device, ms};
use protocol::code;
use protocol::message::{Call, Method, Reply};
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
    let mut client = agent.connect();
    // The client answers pings but never pings first within this window,
    // because every ping from the agent is a sign of life.
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
    let app = agent_android_host_tests::fixture::COMPOSE;
    agent_android_host_tests::fixture::require(&dev, app);
    let mut agent = Agent::launch(&dev, Launch::default());
    // The debuggable fixture's uid, through run-as, opens the abstract socket.
    let out = dev.shell_output(&format!(
        "run-as {app} sh -c 'nc -U -w 3 -W 2 @{} </dev/null | wc -c'",
        agent.socket
    ));
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let bytes: u64 = text
        .lines()
        .last()
        .and_then(|l| l.trim().parse().ok())
        .unwrap_or(0);
    assert_eq!(bytes, 0, "a peer of another uid received {bytes} bytes");
    // Either SELinux refused the connection or the agent refused the peer.
    let refusal = agent.wait_line(Duration::from_secs(2), |l| l.starts_with("refused "));
    println!(
        "app-uid peer: {} ({})",
        refusal
            .as_deref()
            .unwrap_or("connection refused before the agent saw it"),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    if let Some(line) = refusal {
        assert!(line.contains("uid=10"), "{line}");
    }
    let mut client = agent.connect();
    assert!(client.call(Call::Unknown(Method(999))).is_err());
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
