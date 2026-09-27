//! A blocking client over one agent connection: the protocol crate's
//! `HostSession` fed from a service stream, as the daemon's session will be.

use std::collections::HashMap;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

use protocol::Error;
use protocol::event::{Event, EventPayload, Stream};
use protocol::frame::{DEFAULT_MAX_FRAME, FrameDecoder};
use protocol::message::{Call, Message, Reply};
use protocol::session::{DisconnectReason, HostConfig, HostEvent, HostSession, Negotiated};

use crate::adb::Device;

/// One connection to an agent.
pub struct Client {
    stream: TcpStream,
    session: HostSession,
    /// Frames as they arrive, decoded a second time to observe what the
    /// session handles on its own (pings).
    tap: FrameDecoder,
    responses: HashMap<u32, Result<Reply, Error>>,
    /// Events in arrival order, with the host time each arrived.
    pub events: Vec<(Instant, Event)>,
    /// Session notices other than responses and events: gaps and resyncs.
    pub notices: Vec<HostEvent>,
    /// Pings the agent sent.
    pub pings: u32,
    closed: Option<DisconnectReason>,
    negotiated: Option<Negotiated>,
    buf: Vec<u8>,
}

impl Client {
    /// Opens `localabstract:<socket>` on the device; `token` is what the
    /// agent's hello must carry.
    pub fn connect(device: &Device, socket: &str, token: Option<String>) -> io::Result<Client> {
        Client::with_config(
            device,
            socket,
            HostConfig {
                token,
                ..HostConfig::default()
            },
        )
    }

    /// Opens the socket with a custom host configuration.
    pub fn with_config(device: &Device, socket: &str, config: HostConfig) -> io::Result<Client> {
        let stream = device.service_stream(&format!("localabstract:{socket}"))?;
        Ok(Client {
            stream,
            session: HostSession::new(config, Instant::now()),
            tap: FrameDecoder::new(u32::MAX),
            responses: HashMap::new(),
            events: Vec::new(),
            notices: Vec::new(),
            pings: 0,
            closed: None,
            negotiated: None,
            buf: vec![0; 256 * 1024],
        })
    }

    /// Runs the session until the handshake completes or fails.
    pub fn handshake(&mut self, timeout: Duration) -> Result<Negotiated, String> {
        let end = Instant::now() + timeout;
        while self.negotiated.is_none() {
            if let Some(reason) = &self.closed {
                return Err(reason.to_string());
            }
            if Instant::now() >= end {
                return Err("no hello within the timeout".to_owned());
            }
            self.pump(end);
        }
        Ok(self.negotiated.clone().expect("checked above"))
    }

    pub fn negotiated(&self) -> Option<&Negotiated> {
        self.negotiated.as_ref()
    }

    pub fn closed(&self) -> Option<&DisconnectReason> {
        self.closed.as_ref()
    }

    /// Sends a request and returns its id.
    pub fn send(&mut self, call: Call, deadline: Option<Duration>) -> Result<u32, Error> {
        let id = self.session.send_request(call, deadline, Instant::now())?;
        self.flush();
        Ok(id)
    }

    /// Waits for the answer to request `id`.
    pub fn wait(&mut self, id: u32, timeout: Duration) -> Result<Reply, Error> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(result) = self.responses.remove(&id) {
                return result;
            }
            if let Some(reason) = &self.closed {
                return Err(Error::disconnected(reason));
            }
            assert!(
                Instant::now() < end,
                "no answer to request {id} within {timeout:?}"
            );
            self.pump(end);
        }
    }

    /// Sends a request and waits up to 30 s for its answer.
    pub fn call(&mut self, call: Call) -> Result<Reply, Error> {
        self.call_within(call, None, Duration::from_secs(30))
    }

    /// Sends a request with an optional deadline and waits for its answer.
    pub fn call_within(
        &mut self,
        call: Call,
        deadline: Option<Duration>,
        timeout: Duration,
    ) -> Result<Reply, Error> {
        let id = self.send(call, deadline)?;
        self.wait(id, timeout)
    }

    /// Like [`Client::call`], also returning the time from sending to answer.
    pub fn timed(&mut self, call: Call) -> (Result<Reply, Error>, Duration) {
        let t0 = Instant::now();
        let result = self.call(call);
        (result, t0.elapsed())
    }

    /// Reads and handles whatever arrives for `duration`.
    pub fn idle(&mut self, duration: Duration) {
        let end = Instant::now() + duration;
        while Instant::now() < end && self.closed.is_none() {
            self.pump(end);
        }
    }

    /// Waits until an event matching `pred` arrives, returning it; events
    /// already received are searched first.
    pub fn wait_event(
        &mut self,
        timeout: Duration,
        pred: impl Fn(&Event) -> bool,
    ) -> Option<Event> {
        let end = Instant::now() + timeout;
        let mut seen = 0;
        loop {
            if let Some((_, e)) = self.events[seen..].iter().find(|(_, e)| pred(e)) {
                return Some(e.clone());
            }
            seen = self.events.len();
            if Instant::now() >= end || self.closed.is_some() {
                return None;
            }
            self.pump(end);
        }
    }

    /// Events on `stream` received so far.
    pub fn events_on(&self, stream: Stream) -> Vec<Event> {
        self.events
            .iter()
            .filter(|(_, e)| e.stream() == stream)
            .map(|(_, e)| e.clone())
            .collect()
    }

    /// Lifecycle events received so far, as (kind name, message).
    pub fn lifecycle(&self) -> Vec<(String, String)> {
        self.events
            .iter()
            .filter_map(|(_, e)| match &e.payload {
                EventPayload::Lifecycle(l) => Some((l.change.to_string(), l.message.clone())),
                _ => None,
            })
            .collect()
    }

    /// Closes the connection from this side.
    pub fn disconnect(self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    /// Stops reading and writing but keeps the object, for tests that go on
    /// inspecting what arrived.
    pub fn shutdown_socket(&self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    /// Reads once, up to `until` or the session's next timer, and handles it.
    fn pump(&mut self, until: Instant) {
        let now = Instant::now();
        let wake = self
            .session
            .poll_timeout()
            .map_or(until, |t| t.min(until))
            .max(now + Duration::from_millis(1));
        let wait = wake.saturating_duration_since(now);
        self.stream
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))))
            .expect("set a read timeout");
        match self.stream.read(&mut self.buf) {
            Ok(0) => self.session.receive_eof(Instant::now()),
            Ok(n) => {
                let bytes = &self.buf[..n];
                self.tap.push(bytes);
                while let Some(frame) = self.tap.next_frame() {
                    if let Ok(payload) = frame
                        && let Ok(Message::Ping(_)) = Message::decode(payload)
                    {
                        self.pings += 1;
                    }
                }
                self.session.receive(bytes, Instant::now());
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => self.session.receive_eof(Instant::now()),
        }
        let now = Instant::now();
        if self.session.poll_timeout().is_some_and(|t| t <= now) {
            self.session.handle_timeout(now);
        }
        self.flush();
        while let Some(event) = self.session.poll_event() {
            match event {
                HostEvent::Ready(n) => self.negotiated = Some(n),
                HostEvent::Response { id, result, .. } => {
                    self.responses.insert(id, result);
                }
                HostEvent::Event(e) => self.events.push((Instant::now(), e)),
                HostEvent::Closed(reason) => self.closed = Some(reason),
                other => self.notices.push(other),
            }
        }
    }

    fn flush(&mut self) {
        while let Some(bytes) = self.session.poll_transmit() {
            if self.stream.write_all(&bytes).is_err() {
                self.session.receive_eof(Instant::now());
                break;
            }
        }
    }
}

/// The default frame limit, re-exported for tests that lower it.
pub const DEFAULT_LIMIT: u32 = DEFAULT_MAX_FRAME;
