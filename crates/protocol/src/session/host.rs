//! The daemon's end of a device connection.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::{
    DEADLINE_GRACE, DisconnectReason, HandshakeError, Negotiated, PEER_TIMEOUT, PING_INTERVAL,
    Phase, RESYNC_BACKOFF, RESYNC_BACKOFF_MAX, select_version,
};
use crate::capability::Capabilities;
use crate::error::{Error, code};
use crate::event::{Event, Stream};
use crate::frame::{DEFAULT_MAX_FRAME, FrameDecoder, MIN_MAX_FRAME, Oversized, write_frame};
use crate::liveness::{Alive, Liveness};
use crate::message::{
    Call, DecodeError, Hello, Message, Method, Ping, Pong, Reply, Request, Response, ResyncParams,
    Welcome, kind,
};
use crate::sequence::{SeqCheck, StreamTracker};

/// Settings for a [`HostSession`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostConfig {
    /// Protocol versions the daemon speaks.
    pub versions: Vec<u16>,
    /// The largest frame payload the daemon accepts.
    pub max_frame: u32,
    /// The per-launch token the agent's hello must carry, if any.
    pub token: Option<String>,
    pub ping_interval: Duration,
    pub peer_timeout: Duration,
    pub deadline_grace: Duration,
}

impl Default for HostConfig {
    fn default() -> HostConfig {
        HostConfig {
            versions: crate::SUPPORTED_VERSIONS.to_vec(),
            max_frame: DEFAULT_MAX_FRAME,
            token: None,
            ping_interval: PING_INTERVAL,
            peer_timeout: PEER_TIMEOUT,
            deadline_grace: DEADLINE_GRACE,
        }
    }
}

/// What happened on a [`HostSession`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    /// The handshake completed.
    Ready(Negotiated),
    /// The answer to a request: the agent's, or an error the session reports
    /// on its behalf (a passed deadline, an oversized frame, a lost connection).
    Response {
        id: u32,
        method: Method,
        result: Result<Reply, Error>,
    },
    /// An event, in order.
    Event(Event),
    /// A lossless stream skipped or repeated a sequence number; its events
    /// are dropped until the resync the session requested completes.
    OutOfSync {
        stream: Stream,
        expected: u64,
        received: u64,
    },
    /// The stream is in sync again: `seq` is the last number the agent sent
    /// on it before its state was reset.
    Resynced { stream: Stream, seq: u64 },
    /// The agent could not resynchronize the stream, which stays out of
    /// sync: a resync asked for on demand failed, or an automatic one failed
    /// in a way no retry fixes. An automatic resync that fails in a way a
    /// retry can fix, such as a busy agent, is sent again after a backoff
    /// instead, until the stream is in sync.
    ResyncFailed { stream: Stream, error: Error },
    /// A frame over the limit that answered no pending request.
    FrameTooLarge(Oversized),
    /// The session closed; later requests fail at once.
    Closed(DisconnectReason),
}

#[derive(Debug)]
enum State {
    AwaitingHello,
    Ready(Negotiated),
    Closed(DisconnectReason),
}

/// A request sent and not answered yet.
#[derive(Debug)]
struct Pending {
    method: Method,
    deadline_ms: Option<u32>,
    /// When the host stops waiting for the agent's answer: the deadline plus
    /// the grace period.
    expires: Option<Instant>,
    /// Set for resync requests; their outcome is reported as a resync, not
    /// as a response.
    resync: Option<ResyncOf>,
}

/// A resync request's stream, and whether the session sent it on its own
/// after a sequence gap, in which case it is retried.
#[derive(Clone, Copy, Debug)]
struct ResyncOf {
    stream: Stream,
    automatic: bool,
}

/// An automatic resync that failed and is to be sent again.
#[derive(Clone, Copy, Debug)]
struct Retry {
    /// Tries that failed in a row.
    failures: u32,
    /// When it is due; `None` until the session next learns the time.
    at: Option<Instant>,
}

/// The daemon's end of a device connection.
#[derive(Debug)]
pub struct HostSession {
    config: HostConfig,
    state: State,
    decoder: FrameDecoder,
    outbox: Vec<u8>,
    events: VecDeque<HostEvent>,
    pending: HashMap<u32, Pending>,
    next_id: u32,
    liveness: Liveness,
    next_nonce: u64,
    streams: StreamTracker,
    /// Failed automatic resyncs to send again, by stream.
    retries: HashMap<Stream, Retry>,
    /// Failures in a row of the automatic resync in flight, by stream, so a
    /// retry that fails again backs off longer.
    in_flight: HashMap<Stream, u32>,
}

impl HostSession {
    /// Starts waiting for the agent's hello; it must arrive within the peer
    /// timeout.
    pub fn new(config: HostConfig, now: Instant) -> HostSession {
        HostSession {
            decoder: FrameDecoder::new(config.max_frame.max(MIN_MAX_FRAME)),
            liveness: Liveness::new(config.ping_interval, config.peer_timeout, now),
            config,
            state: State::AwaitingHello,
            outbox: Vec::new(),
            events: VecDeque::new(),
            pending: HashMap::new(),
            next_id: 1,
            next_nonce: 0,
            streams: StreamTracker::new(),
            retries: HashMap::new(),
            in_flight: HashMap::new(),
        }
    }

    /// Feeds bytes read from the connection.
    pub fn receive(&mut self, bytes: &[u8], now: Instant) {
        if self.is_closed() {
            return;
        }
        self.decoder.push(bytes);
        // Bytes that only feed the discard of an oversized frame are no sign
        // of life: a corrupt length must not keep a dead peer alive.
        let mut alive = !self.decoder.is_discarding();
        while !self.is_closed() {
            let frame = match self.decoder.next_frame() {
                None => break,
                Some(Ok(payload)) => Ok(Message::decode(payload)),
                Some(Err(oversized)) => Err(oversized),
            };
            alive = true;
            match frame {
                Ok(Ok(msg)) => self.handle(msg, now),
                // A kind a newer agent may send; nothing to do with it.
                Ok(Err(DecodeError::UnknownKind(_))) => {}
                Ok(Err(e)) => self.close_with(DisconnectReason::Protocol(e.to_string())),
                Err(oversized) => self.handle_oversized(oversized),
            }
        }
        if alive {
            self.liveness.heard(now);
        }
        self.schedule_retries(now);
    }

    /// The connection reached its end.
    pub fn receive_eof(&mut self, now: Instant) {
        let _ = now;
        if !self.is_closed() {
            self.close_with(DisconnectReason::Eof);
        }
    }

    /// Acts on deadlines and liveness that are due at `now`.
    pub fn handle_timeout(&mut self, now: Instant) {
        if self.is_closed() {
            return;
        }
        let mut expired: Vec<u32> = self
            .pending
            .iter()
            .filter(|(_, p)| p.expires.is_some_and(|at| at <= now))
            .map(|(id, _)| *id)
            .collect();
        expired.sort_unstable();
        for id in expired {
            let pending = self.pending.remove(&id).expect("listed above");
            let error =
                Error::deadline_exceeded(pending.method, id, pending.deadline_ms.unwrap_or(0));
            self.resolve(id, pending, Err(error));
        }
        self.schedule_retries(now);
        self.send_due_retries(now);
        match self.liveness.check(now, self.phase() == Phase::Ready) {
            Alive::Yes => {}
            Alive::Ping => {
                let nonce = self.next_nonce;
                self.next_nonce = self.next_nonce.wrapping_add(1);
                self.send_control(&Message::Ping(Ping { nonce }));
            }
            Alive::Dead(silence) => self.close_with(DisconnectReason::Silent(silence)),
        }
    }

    /// When `handle_timeout` must next run; `None` once closed.
    pub fn poll_timeout(&self) -> Option<Instant> {
        if self.is_closed() {
            return None;
        }
        let liveness = self.liveness.next_check(self.phase() == Phase::Ready);
        let deadline = self.pending.values().filter_map(|p| p.expires).min();
        let retry = self.retries.values().filter_map(|r| r.at).min();
        Some(
            [deadline, retry]
                .into_iter()
                .flatten()
                .fold(liveness, Instant::min),
        )
    }

    /// Sends a request, returning its id. Fails at once, sending nothing, when
    /// the session is not ready or closed, when the agent did not advertise
    /// the capability the method needs, or when the request exceeds the
    /// negotiated frame size.
    pub fn send_request(
        &mut self,
        call: Call,
        deadline: Option<Duration>,
        now: Instant,
    ) -> Result<u32, Error> {
        self.send(call, deadline, now, None)
    }

    /// Asks the agent to resynchronize `stream`; the outcome arrives as
    /// [`HostEvent::Resynced`] or [`HostEvent::ResyncFailed`].
    pub fn resync(&mut self, stream: Stream, now: Instant) -> Result<u32, Error> {
        self.send(
            Call::Resync(ResyncParams { stream }),
            None,
            now,
            Some(ResyncOf {
                stream,
                automatic: false,
            }),
        )
    }

    /// Sends the resync the session asks for on its own after a gap on
    /// `stream`, the `failures`-th try; a failure to send it is reported at
    /// once, since no retry sends it either.
    fn resync_automatically(&mut self, stream: Stream, failures: u32, now: Instant) {
        let sent = self.send(
            Call::Resync(ResyncParams { stream }),
            None,
            now,
            Some(ResyncOf {
                stream,
                automatic: true,
            }),
        );
        match sent {
            Ok(_) => {
                self.in_flight.insert(stream, failures);
            }
            Err(error) => self
                .events
                .push_back(HostEvent::ResyncFailed { stream, error }),
        }
    }

    /// Gives each failed automatic resync without a time the time it is
    /// due: the backoff after its latest failure.
    fn schedule_retries(&mut self, now: Instant) {
        for retry in self.retries.values_mut() {
            if retry.at.is_none() {
                let doublings = retry.failures.saturating_sub(1).min(16);
                let wait = RESYNC_BACKOFF
                    .saturating_mul(1 << doublings)
                    .min(RESYNC_BACKOFF_MAX);
                retry.at = Some(now + wait);
            }
        }
    }

    /// Sends the automatic resyncs due at `now` again.
    fn send_due_retries(&mut self, now: Instant) {
        if self.phase() != Phase::Ready {
            return;
        }
        let mut due: Vec<(Stream, u32)> = self
            .retries
            .iter()
            .filter(|(_, retry)| retry.at.is_some_and(|at| at <= now))
            .map(|(stream, retry)| (*stream, retry.failures))
            .collect();
        due.sort_unstable_by_key(|(stream, _)| stream.0);
        for (stream, failures) in due {
            self.retries.remove(&stream);
            self.resync_automatically(stream, failures, now);
        }
    }

    /// Sequence state of the event streams.
    pub fn streams(&self) -> &StreamTracker {
        &self.streams
    }

    fn send(
        &mut self,
        call: Call,
        deadline: Option<Duration>,
        now: Instant,
        resync: Option<ResyncOf>,
    ) -> Result<u32, Error> {
        let (capabilities, limit) = match &self.state {
            State::Ready(n) => (n.capabilities, n.max_frame),
            State::AwaitingHello => {
                return Err(Error::new(
                    code::NOT_READY,
                    "the agent has not finished its handshake",
                ));
            }
            State::Closed(reason) => return Err(Error::disconnected(reason)),
        };
        let method = call.method();
        let needed = method.required_capability();
        if !capabilities.contains(needed) {
            let missing = Capabilities(needed.0 & !capabilities.0);
            return Err(Error::capability_missing(method, missing));
        }
        let id = self.allocate_id();
        let deadline_ms = deadline.map(|d| u32::try_from(d.as_millis()).unwrap_or(u32::MAX));
        let request = Message::Request(Request {
            id,
            deadline_ms,
            call,
        });
        write_frame(&request, limit, &mut self.outbox)
            .map_err(|e| Error::frame_too_large(method, id, e.size as u64, e.limit))?;
        let expires = deadline.map(|d| now + d + self.config.deadline_grace);
        self.pending.insert(
            id,
            Pending {
                method,
                deadline_ms,
                expires,
                resync,
            },
        );
        Ok(id)
    }

    /// Bytes to write to the connection.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        if self.outbox.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.outbox))
        }
    }

    pub fn poll_event(&mut self) -> Option<HostEvent> {
        self.events.pop_front()
    }

    /// What the handshake settled, once it has.
    pub fn negotiated(&self) -> Option<&Negotiated> {
        match &self.state {
            State::Ready(n) => Some(n),
            _ => None,
        }
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.state, State::Closed(_))
    }

    /// Why the session closed, once it has.
    pub fn close_reason(&self) -> Option<&DisconnectReason> {
        match &self.state {
            State::Closed(reason) => Some(reason),
            _ => None,
        }
    }

    /// Closes the session from this side, failing every pending request.
    pub fn close(&mut self) {
        if !self.is_closed() {
            self.close_with(DisconnectReason::Closed);
        }
    }

    fn phase(&self) -> Phase {
        match self.state {
            State::AwaitingHello => Phase::Handshake,
            State::Ready(_) => Phase::Ready,
            State::Closed(_) => Phase::Closed,
        }
    }

    fn handle(&mut self, msg: Message, now: Instant) {
        match (self.phase(), msg) {
            (Phase::Handshake, Message::Hello(hello)) => self.accept_hello(hello),
            (Phase::Ready, Message::Response(response)) => self.accept_response(response),
            (Phase::Ready, Message::Event(event)) => self.accept_event(event, now),
            (Phase::Ready, Message::Ping(ping)) => {
                self.send_control(&Message::Pong(Pong { nonce: ping.nonce }));
            }
            (Phase::Ready, Message::Pong(_)) | (Phase::Closed, _) => {}
            (phase, other) => {
                let expected = if phase == Phase::Handshake {
                    "a hello"
                } else {
                    "a response, an event, a ping or a pong"
                };
                self.close_with(DisconnectReason::Protocol(format!(
                    "expected {expected} from the agent, received a {}",
                    kind::name(other.kind())
                )));
            }
        }
    }

    fn accept_hello(&mut self, hello: Hello) {
        match self.negotiate(&hello) {
            Ok(negotiated) => {
                self.send_control(&Message::Welcome(Welcome {
                    versions: self.config.versions.clone(),
                    version: Some(negotiated.version),
                    max_frame: negotiated.max_frame,
                    error: None,
                }));
                self.decoder.set_limit(negotiated.max_frame);
                self.state = State::Ready(negotiated.clone());
                self.events.push_back(HostEvent::Ready(negotiated));
            }
            Err(failure) => {
                // The agent learns why before the connection closes.
                self.send_control(&Message::Welcome(Welcome {
                    versions: self.config.versions.clone(),
                    version: None,
                    max_frame: self.config.max_frame.min(hello.max_frame),
                    error: Some(failure.to_error()),
                }));
                self.close_with(DisconnectReason::Handshake(failure));
            }
        }
    }

    fn negotiate(&self, hello: &Hello) -> Result<Negotiated, HandshakeError> {
        if let Some(token) = &self.config.token
            && hello.token.as_ref() != Some(token)
        {
            return Err(HandshakeError::TokenMismatch);
        }
        let version = select_version(&self.config.versions, &hello.versions).ok_or_else(|| {
            HandshakeError::NoCommonVersion {
                host: self.config.versions.clone(),
                agent: hello.versions.clone(),
            }
        })?;
        let max_frame = self.config.max_frame.min(hello.max_frame);
        if max_frame < MIN_MAX_FRAME {
            return Err(HandshakeError::FrameLimitTooSmall { limit: max_frame });
        }
        Ok(Negotiated {
            version,
            max_frame,
            capabilities: hello.capabilities,
            agent: hello.agent.clone(),
            host_versions: self.config.versions.clone(),
            agent_versions: hello.versions.clone(),
        })
    }

    fn accept_response(&mut self, response: Response) {
        // Absent: already answered by its deadline, or never sent. Dropped.
        let Some(pending) = self.pending.remove(&response.id) else {
            return;
        };
        let result = if response.method == pending.method {
            response.result
        } else {
            Err(Error::new(
                code::PROTOCOL_ERROR,
                format!(
                    "the agent answered {} request {} as {}",
                    pending.method, response.id, response.method
                ),
            ))
        };
        self.resolve(response.id, pending, result);
    }

    /// Reports the outcome of a request: as a response, or for resync
    /// requests as the stream's new state. An automatic resync that failed
    /// in a way a retry can fix is queued to be sent again instead, while
    /// the session lives.
    fn resolve(&mut self, id: u32, pending: Pending, result: Result<Reply, Error>) {
        let Some(ResyncOf { stream, automatic }) = pending.resync else {
            self.events.push_back(HostEvent::Response {
                id,
                method: pending.method,
                result,
            });
            return;
        };
        let failures = if automatic {
            self.in_flight.remove(&stream).unwrap_or(0)
        } else {
            0
        };
        match result {
            Ok(Reply::Resync(ack)) if ack.stream == stream => {
                self.streams.resynced(stream, ack.seq);
                self.events.push_back(HostEvent::Resynced {
                    stream,
                    seq: ack.seq,
                });
            }
            Ok(other) => self.events.push_back(HostEvent::ResyncFailed {
                stream,
                error: Error::new(
                    code::PROTOCOL_ERROR,
                    format!("the resync of the {stream} stream was answered with {other:?}"),
                ),
            }),
            Err(error) if automatic && error.retryable && !self.is_closed() => {
                self.retries.insert(
                    stream,
                    Retry {
                        failures: failures + 1,
                        at: None,
                    },
                );
            }
            Err(error) => self
                .events
                .push_back(HostEvent::ResyncFailed { stream, error }),
        }
    }

    fn accept_event(&mut self, event: Event, now: Instant) {
        let stream = event.stream();
        match self.streams.check(stream, event.seq) {
            SeqCheck::Deliver { .. } => self.events.push_back(HostEvent::Event(event)),
            SeqCheck::Gap { expected, received } => {
                self.events.push_back(HostEvent::OutOfSync {
                    stream,
                    expected,
                    received,
                });
                self.retries.remove(&stream);
                self.resync_automatically(stream, 0, now);
            }
            SeqCheck::Discard => {}
        }
    }

    fn handle_oversized(&mut self, oversized: Oversized) {
        if oversized.kind == Some(kind::RESPONSE)
            && let Some(id) = oversized.id
            && let Some(pending) = self.pending.remove(&id)
        {
            let error = Error::frame_too_large(
                pending.method,
                id,
                u64::from(oversized.size),
                oversized.limit,
            );
            self.resolve(id, pending, Err(error));
            return;
        }
        self.events.push_back(HostEvent::FrameTooLarge(oversized));
    }

    /// Queues a hello, welcome, ping or pong: messages far below any
    /// negotiable frame limit.
    fn send_control(&mut self, msg: &Message) {
        write_frame(msg, u32::MAX, &mut self.outbox).expect("a control message fits in a frame");
    }

    fn allocate_id(&mut self) -> u32 {
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if id != 0 && !self.pending.contains_key(&id) {
                return id;
            }
        }
    }

    fn close_with(&mut self, reason: DisconnectReason) {
        self.retries.clear();
        let mut ids: Vec<u32> = self.pending.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let pending = self.pending.remove(&id).expect("listed above");
            self.resolve(id, pending, Err(Error::disconnected(&reason)));
        }
        self.state = State::Closed(reason.clone());
        self.events.push_back(HostEvent::Closed(reason));
    }
}
