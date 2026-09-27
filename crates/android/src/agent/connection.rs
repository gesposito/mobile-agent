//! One connection to an agent: the protocol crate's `HostSession` driven over
//! the agent's socket (an adb service stream to `localabstract:<socket>`).
//!
//! A reader thread feeds the session what arrives, answers pings, acts on
//! deadlines and liveness, hands responses to the callers waiting for them
//! and every other session event to an [`EventSink`]. Events that arrive
//! before a sink is set are kept and replayed to it, so a warning the agent
//! sends right after its hello is not lost. Requests may come from any
//! thread; each waits for its own response.

use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use protocol::Error;
use protocol::message::{Call, Reply};
use protocol::session::{DisconnectReason, HostConfig, HostEvent, HostSession, Negotiated};

/// Receives the session's events other than responses, with the host time
/// each arrived: protocol events, sequence gaps and resyncs, and finally
/// `Closed`.
pub type EventSink = Box<dyn FnMut(Instant, HostEvent) + Send>;

/// A live connection to one agent. Dropping it closes the connection; the
/// agent keeps running.
pub struct Connection {
    shared: Arc<Shared>,
    reader: Option<JoinHandle<()>>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    stream: TcpStream,
}

struct State {
    session: HostSession,
}

impl Connection {
    /// Starts a host session over `stream`, which the agent speaks first on.
    pub fn open(stream: TcpStream, config: HostConfig) -> std::io::Result<Connection> {
        let _ = (stream, config);
        todo!("task 5.2")
    }

    /// Waits until the handshake completes, or fails with the reason the
    /// session closed: a refused hello (another token, no common protocol
    /// version), silence past the peer timeout, or the end of the stream.
    /// `timeout` caps the wait.
    pub fn handshake(&self, timeout: Duration) -> Result<Negotiated, DisconnectReason> {
        let _ = timeout;
        todo!("task 5.2")
    }

    /// What the handshake settled, once it has.
    pub fn negotiated(&self) -> Option<Negotiated> {
        todo!("task 5.2")
    }

    /// Sends `call` with `deadline` and waits for its answer: the agent's, or
    /// the session's own `deadline_exceeded` or `disconnected` error.
    pub fn call(&self, call: Call, deadline: Duration) -> Result<Reply, Error> {
        let _ = (call, deadline);
        todo!("task 5.2")
    }

    /// Sets where events go from now on, after replaying those that arrived
    /// while no sink was set.
    pub fn set_sink(&self, sink: EventSink) {
        let _ = sink;
        todo!("task 5.2")
    }

    /// Why the connection closed, once it has.
    pub fn close_reason(&self) -> Option<DisconnectReason> {
        todo!("task 5.2")
    }

    pub fn is_closed(&self) -> bool {
        self.close_reason().is_some()
    }

    /// Closes the connection from this side, failing pending calls.
    pub fn close(&self) {
        todo!("task 5.2")
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = (&self.shared, &self.reader);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_agent::{FakeAgent, config};
    use protocol::code;
    use protocol::event::{EventPayload, Lifecycle, LifecycleKind, TreeChange, TreeChangeKind};
    use protocol::message::{InputAck, TapParams};
    use protocol::session::HandshakeError;
    use std::sync::mpsc;

    const WAIT: Duration = Duration::from_secs(5);

    fn host(token: Option<&str>) -> HostConfig {
        HostConfig {
            token: token.map(str::to_owned),
            ..HostConfig::default()
        }
    }

    fn tap() -> Call {
        Call::Tap(TapParams {
            x: 10,
            y: 20,
            duration_ms: None,
        })
    }

    #[test]
    fn the_handshake_completes_with_the_token_the_agent_was_started_with() {
        let agent = FakeAgent::start(config(Some("t0k3n")), Box::new(|_| None));
        let connection = Connection::open(agent.connect(), host(Some("t0k3n"))).unwrap();
        let negotiated = connection.handshake(WAIT).unwrap();
        assert_eq!(negotiated.version, 1);
        assert_eq!(negotiated.agent, "mobile-agent-android 0.1.0");
        assert_eq!(connection.negotiated(), Some(negotiated));
        agent.wait_for(WAIT, |log| log.ready);
    }

    #[test]
    fn a_hello_with_another_token_is_refused() {
        let agent = FakeAgent::start(config(Some("other")), Box::new(|_| None));
        let connection = Connection::open(agent.connect(), host(Some("t0k3n"))).unwrap();
        assert_eq!(
            connection.handshake(WAIT),
            Err(DisconnectReason::Handshake(HandshakeError::TokenMismatch))
        );
        assert!(connection.is_closed());
    }

    /// The agent exits when a host refuses its version, so that the host can
    /// start the version it needs.
    #[test]
    fn an_agent_speaking_only_an_older_protocol_is_refused_with_version_mismatch() {
        let mut old = config(Some("t"));
        old.versions = vec![0];
        let agent = FakeAgent::start(old, Box::new(|_| None));
        let connection = Connection::open(agent.connect(), host(Some("t"))).unwrap();
        assert_eq!(
            connection.handshake(WAIT),
            Err(DisconnectReason::Handshake(
                HandshakeError::NoCommonVersion {
                    host: vec![1],
                    agent: vec![0],
                }
            ))
        );
        agent.wait_for(WAIT, |log| log.closed.is_some());
        match &agent.log().closed {
            Some(DisconnectReason::Handshake(HandshakeError::Refused(error))) => {
                assert_eq!(error.code, code::VERSION_MISMATCH)
            }
            other => panic!("the agent saw {other:?}"),
        }
    }

    #[test]
    fn a_call_returns_the_agents_reply_and_carries_its_deadline() {
        let agent = FakeAgent::start(
            config(None),
            Box::new(|_| Some(Ok(Reply::Input(InputAck { accepted: true })))),
        );
        let connection = Connection::open(agent.connect(), host(None)).unwrap();
        connection.handshake(WAIT).unwrap();
        let reply = connection.call(tap(), Duration::from_millis(1500)).unwrap();
        assert_eq!(reply, Reply::Input(InputAck { accepted: true }));
        let log = agent.log();
        assert_eq!(log.requests.len(), 1);
        assert_eq!(log.requests[0].call, tap());
        assert_eq!(log.requests[0].deadline_ms, Some(1500));
    }

    #[test]
    fn calls_from_several_threads_each_get_their_own_answer() {
        let agent = FakeAgent::start(
            config(None),
            Box::new(|request| {
                let accepted = matches!(&request.call, Call::Tap(t) if t.x % 2 == 0);
                Some(Ok(Reply::Input(InputAck { accepted })))
            }),
        );
        let connection = Arc::new(Connection::open(agent.connect(), host(None)).unwrap());
        connection.handshake(WAIT).unwrap();
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let connection = Arc::clone(&connection);
                std::thread::spawn(move || {
                    let call = Call::Tap(TapParams {
                        x: i,
                        y: 0,
                        duration_ms: None,
                    });
                    (i, connection.call(call, Duration::from_secs(2)).unwrap())
                })
            })
            .collect();
        for worker in workers {
            let (i, reply) = worker.join().unwrap();
            assert_eq!(
                reply,
                Reply::Input(InputAck {
                    accepted: i % 2 == 0
                })
            );
        }
    }

    #[test]
    fn events_reach_the_sink_in_order_including_those_before_it_was_set() {
        let agent = FakeAgent::start(config(None), Box::new(|_| None));
        let connection = Connection::open(agent.connect(), host(None)).unwrap();
        connection.handshake(WAIT).unwrap();
        agent.emit(EventPayload::Lifecycle(Lifecycle {
            change: LifecycleKind::WARNING,
            message: "input injection is unavailable".into(),
        }));
        // Give the warning time to arrive before the sink exists.
        std::thread::sleep(Duration::from_millis(100));
        let (tx, rx) = mpsc::channel();
        connection.set_sink(Box::new(move |_, event| {
            let _ = tx.send(event);
        }));
        agent.emit(EventPayload::Tree(TreeChange {
            change: TreeChangeKind::TEXT,
            window: 7,
            path: vec![0, 2],
        }));
        let first = rx.recv_timeout(WAIT).unwrap();
        let second = rx.recv_timeout(WAIT).unwrap();
        match (first, second) {
            (HostEvent::Event(a), HostEvent::Event(b)) => {
                assert!(matches!(a.payload, EventPayload::Lifecycle(_)), "{a:?}");
                assert_eq!(a.seq, 1);
                assert!(matches!(b.payload, EventPayload::Tree(_)), "{b:?}");
                assert_eq!(b.seq, 1);
            }
            other => panic!("unexpected events {other:?}"),
        }
    }

    #[test]
    fn the_end_of_the_stream_fails_pending_calls_and_reports_the_close() {
        let agent = FakeAgent::start(config(None), Box::new(|_| None));
        let connection = Arc::new(Connection::open(agent.connect(), host(None)).unwrap());
        connection.handshake(WAIT).unwrap();
        let (tx, rx) = mpsc::channel();
        connection.set_sink(Box::new(move |_, event| {
            let _ = tx.send(event);
        }));
        let caller = {
            let connection = Arc::clone(&connection);
            std::thread::spawn(move || connection.call(tap(), Duration::from_secs(30)))
        };
        agent.wait_for(WAIT, |log| log.requests.len() == 1);
        let closed_at = Instant::now();
        agent.close();
        let error = caller.join().unwrap().unwrap_err();
        assert_eq!(error.code, code::DISCONNECTED);
        assert!(error.message.contains("reconnect"), "{}", error.message);
        assert!(closed_at.elapsed() < Duration::from_secs(1));
        assert_eq!(connection.close_reason(), Some(DisconnectReason::Eof));
        let closed = rx.recv_timeout(WAIT).unwrap();
        assert_eq!(closed, HostEvent::Closed(DisconnectReason::Eof));
        let later = connection.call(tap(), Duration::from_secs(1)).unwrap_err();
        assert_eq!(later.code, code::DISCONNECTED);
    }

    #[test]
    fn the_reader_answers_pings_while_the_host_is_idle() {
        let mut quick = config(None);
        quick.ping_interval = Duration::from_millis(50);
        quick.peer_timeout = Duration::from_millis(300);
        let agent = FakeAgent::start(quick, Box::new(|_| None));
        let connection = Connection::open(agent.connect(), host(None)).unwrap();
        connection.handshake(WAIT).unwrap();
        std::thread::sleep(Duration::from_millis(900));
        assert!(
            agent.log().closed.is_none(),
            "the agent gave up on a silent host: {:?}",
            agent.log().closed
        );
        assert!(!connection.is_closed());
    }

    #[test]
    fn closing_from_this_side_fails_later_calls() {
        let agent = FakeAgent::start(config(None), Box::new(|_| None));
        let connection = Connection::open(agent.connect(), host(None)).unwrap();
        connection.handshake(WAIT).unwrap();
        connection.close();
        assert_eq!(connection.close_reason(), Some(DisconnectReason::Closed));
        let error = connection.call(tap(), Duration::from_secs(1)).unwrap_err();
        assert_eq!(error.code, code::DISCONNECTED);
        agent.wait_for(WAIT, |log| log.closed.is_some());
    }
}
