//! The device agent's end of the connection.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::{DisconnectReason, HandshakeError, Negotiated, PEER_TIMEOUT, PING_INTERVAL, Phase};
use crate::capability::Capabilities;
use crate::error::Error;
use crate::event::{EventPayload, Stream};
use crate::frame::{DEFAULT_MAX_FRAME, FrameDecoder, MIN_MAX_FRAME, Oversized, write_frame};
use crate::message::{
    Call, DecodeError, Hello, Message, Method, Pong, Reply, Request, Response, Welcome, kind,
};

/// Settings for an [`AgentSession`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConfig {
    /// Protocol versions the agent speaks.
    pub versions: Vec<u16>,
    pub capabilities: Capabilities,
    /// The largest frame payload the agent accepts.
    pub max_frame: u32,
    /// Agent name and version for the hello.
    pub agent: String,
    /// The per-launch token the host started the agent with.
    pub token: Option<String>,
    pub ping_interval: Duration,
    pub peer_timeout: Duration,
}

impl Default for AgentConfig {
    fn default() -> AgentConfig {
        AgentConfig {
            versions: crate::SUPPORTED_VERSIONS.to_vec(),
            capabilities: Capabilities::NONE,
            max_frame: DEFAULT_MAX_FRAME,
            agent: String::new(),
            token: None,
            ping_interval: PING_INTERVAL,
            peer_timeout: PEER_TIMEOUT,
        }
    }
}

/// What happened on an [`AgentSession`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// The host accepted the hello.
    Ready(Negotiated),
    /// A request to work on; answer it with [`AgentSession::respond`].
    Request(Request),
    /// The session closed.
    Closed(DisconnectReason),
}

/// What became of an answer passed to [`AgentSession::respond`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Responded {
    /// Sent as given.
    Sent,
    /// Over the negotiated frame size: a `frame_too_large` error went instead.
    TooLarge,
    /// Not sent: the request was already answered, by its deadline for
    /// instance, or never arrived.
    NotInFlight,
}

#[derive(Debug)]
enum State {
    AwaitingWelcome,
    Ready(Negotiated),
    Closed(DisconnectReason),
}

/// A request handed to the agent and not answered yet.
#[derive(Debug)]
struct InFlight {
    method: Method,
    deadline_ms: Option<u32>,
    deadline: Option<Instant>,
}

/// The device agent's end of the connection.
#[derive(Debug)]
pub struct AgentSession {
    config: AgentConfig,
    state: State,
    decoder: FrameDecoder,
    outbox: Vec<u8>,
    events: VecDeque<AgentEvent>,
    in_flight: HashMap<u32, InFlight>,
}

impl AgentSession {
    /// Starts the session; the hello is ready to send at once.
    pub fn new(config: AgentConfig, now: Instant) -> AgentSession {
        let _ = now;
        let hello = Message::Hello(Hello {
            versions: config.versions.clone(),
            capabilities: config.capabilities,
            max_frame: config.max_frame,
            agent: config.agent.clone(),
            token: config.token.clone(),
        });
        let mut session = AgentSession {
            // Wide enough for a refusal even when the agent asks for less.
            decoder: FrameDecoder::new(config.max_frame.max(MIN_MAX_FRAME)),
            config,
            state: State::AwaitingWelcome,
            outbox: Vec::new(),
            events: VecDeque::new(),
            in_flight: HashMap::new(),
        };
        session.send_control(&hello);
        session
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

    /// Answers requests whose deadline passed and keeps the connection alive.
    pub fn handle_timeout(&mut self, now: Instant) {
        if self.phase() != Phase::Ready {
            return;
        }
        let mut expired: Vec<u32> = self
            .in_flight
            .iter()
            .filter(|(_, f)| f.deadline.is_some_and(|at| at <= now))
            .map(|(id, _)| *id)
            .collect();
        expired.sort_unstable();
        for id in expired {
            let flight = self.in_flight.remove(&id).expect("listed above");
            let error =
                Error::deadline_exceeded(flight.method, id, flight.deadline_ms.unwrap_or(0));
            self.send_response(id, flight.method, Err(error));
        }
    }

    /// When `handle_timeout` must next run; `None` once closed.
    pub fn poll_timeout(&self) -> Option<Instant> {
        if self.is_closed() {
            return None;
        }
        self.in_flight.values().filter_map(|f| f.deadline).min()
    }

    /// The instant request `id` must be answered by, if it has a deadline.
    pub fn deadline(&self, id: u32) -> Option<Instant> {
        self.in_flight.get(&id).and_then(|f| f.deadline)
    }

    /// Answers request `id`.
    pub fn respond(&mut self, id: u32, result: Result<Reply, Error>) -> Responded {
        match self.in_flight.remove(&id) {
            Some(flight) => self.send_response(id, flight.method, result),
            None => Responded::NotInFlight,
        }
    }

    /// Sends an event on its payload's stream with the stream's next sequence
    /// number and returns the number. `None` when the session is not ready,
    /// or when the event exceeds the negotiated frame size; its number is
    /// used up then, so the host sees the gap and asks for a resync.
    pub fn emit(&mut self, payload: EventPayload, time_us: u64) -> Option<u64> {
        let _ = (payload, time_us);
        todo!("emit")
    }

    /// Uses up the next sequence number of `stream` without sending an event,
    /// as when a full event queue drops one. The host sees the gap: a
    /// lossless stream then asks for a resync, a lossy one counts a drop.
    pub fn skip_event(&mut self, stream: Stream) -> Option<u64> {
        let _ = stream;
        todo!("skip_event")
    }

    /// The last sequence number sent or skipped on `stream`; 0 before the first.
    pub fn last_seq(&self, stream: Stream) -> u64 {
        let _ = stream;
        todo!("last_seq")
    }

    /// Bytes to write to the connection.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        if self.outbox.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.outbox))
        }
    }

    pub fn poll_event(&mut self) -> Option<AgentEvent> {
        self.events.pop_front()
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

    fn phase(&self) -> Phase {
        match self.state {
            State::AwaitingWelcome => Phase::Handshake,
            State::Ready(_) => Phase::Ready,
            State::Closed(_) => Phase::Closed,
        }
    }

    fn handle(&mut self, msg: Message, now: Instant) {
        match (self.phase(), msg) {
            (Phase::Handshake, Message::Welcome(welcome)) => self.accept_welcome(welcome),
            (Phase::Ready, Message::Request(request)) => self.accept_request(request, now),
            (Phase::Ready, Message::Ping(ping)) => {
                self.send_control(&Message::Pong(Pong { nonce: ping.nonce }));
            }
            (Phase::Ready, Message::Pong(_)) | (Phase::Closed, _) => {}
            (phase, other) => {
                let expected = if phase == Phase::Handshake {
                    "a welcome"
                } else {
                    "a request, a ping or a pong"
                };
                self.close_with(DisconnectReason::Protocol(format!(
                    "expected {expected} from the host, received a {}",
                    kind::name(other.kind())
                )));
            }
        }
    }

    fn accept_welcome(&mut self, welcome: Welcome) {
        if let Some(error) = welcome.error {
            self.close_with(DisconnectReason::Handshake(HandshakeError::Refused(error)));
            return;
        }
        let Some(version) = welcome.version else {
            self.close_with(DisconnectReason::Protocol(
                "a welcome with neither a version nor an error".to_owned(),
            ));
            return;
        };
        if !self.config.versions.contains(&version) {
            self.close_with(DisconnectReason::Handshake(
                HandshakeError::VersionNotOffered {
                    version,
                    offered: self.config.versions.clone(),
                },
            ));
            return;
        }
        let max_frame = welcome.max_frame.min(self.config.max_frame);
        if max_frame < MIN_MAX_FRAME {
            self.close_with(DisconnectReason::Handshake(
                HandshakeError::FrameLimitTooSmall { limit: max_frame },
            ));
            return;
        }
        let negotiated = Negotiated {
            version,
            max_frame,
            capabilities: self.config.capabilities,
            agent: self.config.agent.clone(),
            host_versions: welcome.versions,
            agent_versions: self.config.versions.clone(),
        };
        self.decoder.set_limit(max_frame);
        self.state = State::Ready(negotiated.clone());
        self.events.push_back(AgentEvent::Ready(negotiated));
    }

    fn accept_request(&mut self, request: Request, now: Instant) {
        if self.in_flight.contains_key(&request.id) {
            self.close_with(DisconnectReason::Protocol(format!(
                "request id {} reused while in flight",
                request.id
            )));
            return;
        }
        let method = request.call.method();
        if let Call::Unknown(_) = request.call {
            self.send_response(request.id, method, Err(Error::unknown_method(method)));
            return;
        }
        let needed = method.required_capability();
        if !self.config.capabilities.contains(needed) {
            let missing = Capabilities(needed.0 & !self.config.capabilities.0);
            self.send_response(
                request.id,
                method,
                Err(Error::capability_missing(method, missing)),
            );
            return;
        }
        let deadline = request
            .deadline_ms
            .map(|ms| now + Duration::from_millis(u64::from(ms)));
        self.in_flight.insert(
            request.id,
            InFlight {
                method,
                deadline_ms: request.deadline_ms,
                deadline,
            },
        );
        self.events.push_back(AgentEvent::Request(request));
    }

    /// Answers a request the host sent over the limit, whose payload was dropped.
    fn handle_oversized(&mut self, oversized: Oversized) {
        if oversized.kind == Some(kind::REQUEST)
            && let (Some(id), Some(method)) = (oversized.id, oversized.method)
        {
            let error =
                Error::frame_too_large(method, id, u64::from(oversized.size), oversized.limit);
            self.send_response(id, method, Err(error));
        }
    }

    /// Writes a response, or a `frame_too_large` error in its place when it
    /// exceeds the negotiated limit.
    fn send_response(
        &mut self,
        id: u32,
        method: Method,
        result: Result<Reply, Error>,
    ) -> Responded {
        let State::Ready(negotiated) = &self.state else {
            return Responded::NotInFlight;
        };
        let limit = negotiated.max_frame;
        let response = Message::Response(Response { id, method, result });
        match write_frame(&response, limit, &mut self.outbox) {
            Ok(()) => Responded::Sent,
            Err(too_large) => {
                let refusal = Message::Response(Response {
                    id,
                    method,
                    result: Err(Error::frame_too_large(
                        method,
                        id,
                        too_large.size as u64,
                        limit,
                    )),
                });
                write_frame(&refusal, limit, &mut self.outbox)
                    .expect("an error response fits in the smallest frame limit");
                Responded::TooLarge
            }
        }
    }

    /// Queues a hello, ping or pong: messages far below any negotiable limit.
    fn send_control(&mut self, msg: &Message) {
        write_frame(msg, u32::MAX, &mut self.outbox).expect("a control message fits in a frame");
    }

    fn close_with(&mut self, reason: DisconnectReason) {
        self.in_flight.clear();
        self.state = State::Closed(reason.clone());
        self.events.push_back(AgentEvent::Closed(reason));
    }
}
