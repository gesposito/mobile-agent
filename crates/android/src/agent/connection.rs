//! One connection to an agent: the protocol crate's `HostSession` driven over
//! the agent's socket (an adb service stream to `localabstract:<socket>`).
//!
//! A reader thread feeds the session what arrives, answers pings, acts on
//! deadlines and liveness, hands responses to the callers waiting for them
//! and every other session event to an [`EventSink`]. Events that arrive
//! before a sink is set are kept and replayed to it, so a warning the agent
//! sends right after its hello is not lost. Requests may come from any
//! thread; each waits for its own response.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use protocol::Error;
use protocol::message::{Call, Reply};
use protocol::session::{
    DEADLINE_GRACE, DisconnectReason, HostConfig, HostEvent, HostSession, Negotiated,
};

/// Receives the session's events other than responses, with the host time
/// each arrived: protocol events, sequence gaps and resyncs, and finally
/// `Closed`.
pub type EventSink = Box<dyn FnMut(Instant, HostEvent) + Send>;

/// The longest the reader blocks in one read, so that deadlines of requests
/// sent meanwhile are acted on within this bound.
const MAX_READ_WAIT: Duration = Duration::from_millis(100);

/// How long a caller waits past a request's deadline and the session's
/// grace period before giving up on its own.
const WAIT_MARGIN: Duration = Duration::from_secs(1);

/// A live connection to one agent. Dropping it closes the connection; the
/// agent keeps running.
pub struct Connection {
    shared: Arc<Shared>,
    reader: Option<JoinHandle<()>>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    /// Taken after `state` when both are held, so events reach the sink in
    /// the order the session produced them.
    sink: Mutex<Sink>,
    /// The socket, for closing it from any thread.
    socket: TcpStream,
}

struct State {
    session: HostSession,
    writer: TcpStream,
    responses: HashMap<u32, Result<Reply, Error>>,
    /// Requests whose callers stopped waiting; their answers are dropped.
    abandoned: HashSet<u32>,
    negotiated: Option<Negotiated>,
    closed: Option<DisconnectReason>,
}

#[derive(Default)]
struct Sink {
    sink: Option<EventSink>,
    buffered: VecDeque<(Instant, HostEvent)>,
}

impl State {
    /// Writes what the session has queued; a failed write ends the session.
    fn flush(&mut self, now: Instant) {
        while let Some(bytes) = self.session.poll_transmit() {
            if self.writer.write_all(&bytes).is_err() {
                self.session.receive_eof(now);
            }
        }
    }

    /// Takes the session's events: responses to their callers, the handshake
    /// and the close into the state, and the rest for the sink. Returns
    /// whether a waiting caller may have something new.
    fn collect(&mut self, now: Instant, events: &mut Vec<(Instant, HostEvent)>) -> bool {
        let mut changed = false;
        while let Some(event) = self.session.poll_event() {
            changed = true;
            match event {
                HostEvent::Ready(negotiated) => self.negotiated = Some(negotiated),
                HostEvent::Response { id, result, .. } => {
                    if !self.abandoned.remove(&id) {
                        self.responses.insert(id, result);
                    }
                }
                HostEvent::Closed(reason) => {
                    self.closed = Some(reason.clone());
                    events.push((now, HostEvent::Closed(reason)));
                }
                other => events.push((now, other)),
            }
        }
        changed
    }
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // The session is only ever changed through complete calls, so a
        // panic elsewhere leaves it consistent.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sink(&self) -> MutexGuard<'_, Sink> {
        self.sink.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Hands what the session produced to waiting callers and the sink,
    /// keeping the session's order: the sink is locked before `state` is
    /// released.
    fn settle(&self, mut state: MutexGuard<'_, State>, now: Instant) {
        state.flush(now);
        let mut events = Vec::new();
        if state.collect(now, &mut events) {
            self.changed.notify_all();
        }
        if events.is_empty() {
            return;
        }
        let mut sink = self.sink();
        drop(state);
        match &mut sink.sink {
            Some(deliver) => {
                for (at, event) in events {
                    deliver(at, event);
                }
            }
            None => sink.buffered.extend(events),
        }
    }
}

impl Connection {
    /// Starts a host session over `stream`, which the agent speaks first on.
    pub fn open(stream: TcpStream, config: HostConfig) -> std::io::Result<Connection> {
        let _ = stream.set_nodelay(true);
        let writer = stream.try_clone()?;
        let reading = stream.try_clone()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                session: HostSession::new(config, Instant::now()),
                writer,
                responses: HashMap::new(),
                abandoned: HashSet::new(),
                negotiated: None,
                closed: None,
            }),
            changed: Condvar::new(),
            sink: Mutex::new(Sink::default()),
            socket: stream,
        });
        let for_reader = Arc::clone(&shared);
        let reader = thread::Builder::new()
            .name("android-agent-connection".into())
            .spawn(move || read_loop(&for_reader, reading))?;
        Ok(Connection {
            shared,
            reader: Some(reader),
        })
    }

    /// Waits until the handshake completes, or fails with the reason the
    /// session closed: a refused hello (another token, no common protocol
    /// version), silence past the peer timeout, or the end of the stream.
    /// `timeout` caps the wait; past it the connection is closed and the
    /// failure is silence.
    pub fn handshake(&self, timeout: Duration) -> Result<Negotiated, DisconnectReason> {
        let limit = Instant::now() + timeout;
        let mut state = self.shared.state();
        loop {
            if let Some(negotiated) = &state.negotiated {
                return Ok(negotiated.clone());
            }
            if let Some(reason) = &state.closed {
                return Err(reason.clone());
            }
            let now = Instant::now();
            if now >= limit {
                drop(state);
                self.close();
                return Err(DisconnectReason::Silent(timeout));
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, limit - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// What the handshake settled, once it has.
    pub fn negotiated(&self) -> Option<Negotiated> {
        self.shared.state().negotiated.clone()
    }

    /// Sends `call` with `deadline` and waits for its answer: the agent's, or
    /// the session's own `deadline_exceeded` or `disconnected` error.
    pub fn call(&self, call: Call, deadline: Duration) -> Result<Reply, Error> {
        let method = call.method();
        let now = Instant::now();
        let mut state = self.shared.state();
        if let Some(reason) = &state.closed {
            return Err(Error::disconnected(reason));
        }
        let id = state.session.send_request(call, Some(deadline), now)?;
        self.shared.settle(state, now);
        let limit = now + deadline + DEADLINE_GRACE + WAIT_MARGIN;
        let mut state = self.shared.state();
        loop {
            if let Some(result) = state.responses.remove(&id) {
                return result;
            }
            if let Some(reason) = &state.closed {
                return Err(Error::disconnected(reason));
            }
            let now = Instant::now();
            if now >= limit {
                state.abandoned.insert(id);
                let ms = u32::try_from(deadline.as_millis()).unwrap_or(u32::MAX);
                return Err(Error::deadline_exceeded(method, id, ms));
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, limit - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Sets where events go from now on, after replaying those that arrived
    /// while no sink was set.
    pub fn set_sink(&self, mut sink: EventSink) {
        let mut slot = self.shared.sink();
        for (at, event) in slot.buffered.drain(..) {
            sink(at, event);
        }
        slot.sink = Some(sink);
    }

    /// Why the connection closed, once it has.
    pub fn close_reason(&self) -> Option<DisconnectReason> {
        self.shared.state().closed.clone()
    }

    pub fn is_closed(&self) -> bool {
        self.close_reason().is_some()
    }

    /// Closes the connection from this side, failing pending calls.
    pub fn close(&self) {
        let mut state = self.shared.state();
        if state.closed.is_none() {
            state.session.close();
        }
        self.shared.settle(state, Instant::now());
        let _ = self.shared.socket.shutdown(Shutdown::Both);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Feeds the session what arrives until it closes.
fn read_loop(shared: &Shared, mut stream: TcpStream) {
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let wait = {
            let state = shared.state();
            if state.closed.is_some() {
                return;
            }
            let now = Instant::now();
            state
                .session
                .poll_timeout()
                .map_or(MAX_READ_WAIT, |due| due.saturating_duration_since(now))
                .clamp(Duration::from_millis(1), MAX_READ_WAIT)
        };
        let read = match stream.set_read_timeout(Some(wait)) {
            Ok(()) => stream.read(&mut buf),
            // macOS refuses socket options once the peer has reset it.
            Err(e) => Err(e),
        };
        let now = Instant::now();
        let mut state = shared.state();
        match read {
            Ok(0) => state.session.receive_eof(now),
            Ok(n) => state.session.receive(&buf[..n], now),
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => state.session.receive_eof(now),
        }
        if state.session.poll_timeout().is_some_and(|due| due <= now) {
            state.session.handle_timeout(now);
        }
        shared.settle(state, now);
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
