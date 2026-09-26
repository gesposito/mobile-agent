//! The daemon's end of a device connection.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::{
    DEADLINE_GRACE, DisconnectReason, HandshakeError, Negotiated, PEER_TIMEOUT, PING_INTERVAL,
    Phase, select_version,
};
use crate::capability::Capabilities;
use crate::error::{Error, code};
use crate::event::{Event, Stream};
use crate::frame::{DEFAULT_MAX_FRAME, FrameDecoder, MIN_MAX_FRAME, Oversized, write_frame};
use crate::message::{
    Call, DecodeError, Hello, Message, Method, Pong, Reply, Request, Response, Welcome, kind,
};
use crate::sequence::StreamTracker;

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
    /// The agent could not resynchronize the stream, which stays out of sync.
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
}

impl HostSession {
    pub fn new(config: HostConfig, now: Instant) -> HostSession {
        let _ = now;
        HostSession {
            decoder: FrameDecoder::new(config.max_frame.max(MIN_MAX_FRAME)),
            config,
            state: State::AwaitingHello,
            outbox: Vec::new(),
            events: VecDeque::new(),
            pending: HashMap::new(),
            next_id: 1,
        }
    }

    /// Feeds bytes read from the connection.
    pub fn receive(&mut self, bytes: &[u8], now: Instant) {
        if self.is_closed() {
            return;
        }
        self.decoder.push(bytes);
        while !self.is_closed() {
            let frame = match self.decoder.next_frame() {
                None => break,
                Some(Ok(payload)) => Ok(Message::decode(payload)),
                Some(Err(oversized)) => Err(oversized),
            };
            match frame {
                Ok(Ok(msg)) => self.handle(msg, now),
                // A kind a newer agent may send; nothing to do with it.
                Ok(Err(DecodeError::UnknownKind(_))) => {}
                Ok(Err(e)) => self.close_with(DisconnectReason::Protocol(e.to_string())),
                Err(oversized) => self.handle_oversized(oversized),
            }
        }
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
            self.events.push_back(HostEvent::Response {
                id,
                method: pending.method,
                result: Err(error),
            });
        }
    }

    /// When `handle_timeout` must next run; `None` once closed.
    pub fn poll_timeout(&self) -> Option<Instant> {
        if self.is_closed() {
            return None;
        }
        self.pending.values().filter_map(|p| p.expires).min()
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

    /// Asks the agent to resynchronize `stream`; the outcome arrives as
    /// [`HostEvent::Resynced`] or [`HostEvent::ResyncFailed`].
    pub fn resync(&mut self, stream: Stream, now: Instant) -> Result<u32, Error> {
        let _ = (stream, now);
        todo!("resync")
    }

    /// Sequence state of the event streams.
    pub fn streams(&self) -> &StreamTracker {
        todo!("streams")
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
        let _ = now;
        match (self.phase(), msg) {
            (Phase::Handshake, Message::Hello(hello)) => self.accept_hello(hello),
            (Phase::Ready, Message::Response(response)) => self.accept_response(response),
            (Phase::Ready, Message::Event(event)) => self.events.push_back(HostEvent::Event(event)),
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
        self.events.push_back(HostEvent::Response {
            id: response.id,
            method: pending.method,
            result,
        });
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
            self.events.push_back(HostEvent::Response {
                id,
                method: pending.method,
                result: Err(error),
            });
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
        let mut ids: Vec<u32> = self.pending.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let pending = self.pending.remove(&id).expect("listed above");
            self.events.push_back(HostEvent::Response {
                id,
                method: pending.method,
                result: Err(Error::disconnected(&reason)),
            });
        }
        self.state = State::Closed(reason.clone());
        self.events.push_back(HostEvent::Closed(reason));
    }
}
