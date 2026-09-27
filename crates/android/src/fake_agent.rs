//! A scripted agent for unit tests: the protocol crate's `AgentSession` on a
//! local TCP listener, standing in for the agent's abstract socket behind an
//! adb service stream.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use protocol::event::EventPayload;
use protocol::message::{Reply, Request};
use protocol::session::{AgentConfig, AgentEvent, AgentSession, DisconnectReason};
use protocol::{Capabilities, Error};

/// Answers a request, or `None` to leave it unanswered.
pub(crate) type Handler = Box<dyn FnMut(&Request) -> Option<Result<Reply, Error>> + Send>;

enum Control {
    Emit(EventPayload),
    /// Close the connection as if the process died.
    Close,
}

/// What the fake agent saw.
#[derive(Default)]
pub(crate) struct Log {
    pub requests: Vec<Request>,
    pub closed: Option<DisconnectReason>,
    pub ready: bool,
}

pub(crate) struct FakeAgent {
    addr: std::net::SocketAddr,
    control: Sender<Control>,
    log: Arc<Mutex<Log>>,
    handle: Option<JoinHandle<()>>,
}

/// An agent config as the real agent's hello fills it.
pub(crate) fn config(token: Option<&str>) -> AgentConfig {
    AgentConfig {
        capabilities: Capabilities::TREE
            | Capabilities::INPUT
            | Capabilities::SCREENSHOT
            | Capabilities::EVENTS
            | Capabilities::TEXT_SET,
        agent: "mobile-agent-android 0.1.0".into(),
        token: token.map(str::to_owned),
        ..AgentConfig::default()
    }
}

impl FakeAgent {
    /// Listens for one connection and serves it with `handler`.
    pub(crate) fn start(config: AgentConfig, handler: Handler) -> FakeAgent {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let (control, commands) = mpsc::channel();
        let log = Arc::new(Mutex::new(Log::default()));
        let shared = Arc::clone(&log);
        let handle = thread::spawn(move || serve(listener, config, handler, commands, shared));
        FakeAgent {
            addr,
            control,
            log,
            handle: Some(handle),
        }
    }

    /// A connection to the fake agent, as `Device::open_abstract` returns one.
    pub(crate) fn connect(&self) -> TcpStream {
        TcpStream::connect(self.addr).unwrap()
    }

    pub(crate) fn emit(&self, payload: EventPayload) {
        let _ = self.control.send(Control::Emit(payload));
    }

    pub(crate) fn close(&self) {
        let _ = self.control.send(Control::Close);
    }

    pub(crate) fn log(&self) -> std::sync::MutexGuard<'_, Log> {
        self.log.lock().unwrap()
    }

    /// Waits until `f` holds for the log, or panics after `timeout`.
    pub(crate) fn wait_for(&self, timeout: Duration, f: impl Fn(&Log) -> bool) {
        let end = Instant::now() + timeout;
        while !f(&self.log()) {
            assert!(Instant::now() < end, "the fake agent never got there");
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for FakeAgent {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Close);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(
    listener: TcpListener,
    config: AgentConfig,
    mut handler: Handler,
    commands: Receiver<Control>,
    log: Arc<Mutex<Log>>,
) {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if Instant::now() > deadline || matches!(commands.try_recv(), Ok(Control::Close)) {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(e) => panic!("accept: {e}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(5)))
        .unwrap();
    let started = Instant::now();
    let mut session = AgentSession::new(config, Instant::now());
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        while let Some(bytes) = session.poll_transmit() {
            if stream.write_all(&bytes).is_err() {
                session.receive_eof(Instant::now());
            }
        }
        while let Ok(command) = commands.try_recv() {
            match command {
                Control::Emit(payload) => {
                    let time_us = started.elapsed().as_micros() as u64;
                    session.emit(payload, time_us);
                }
                Control::Close => {
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
            }
        }
        while let Some(bytes) = session.poll_transmit() {
            let _ = stream.write_all(&bytes);
        }
        match stream.read(&mut buf) {
            Ok(0) => session.receive_eof(Instant::now()),
            Ok(n) => session.receive(&buf[..n], Instant::now()),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => session.receive_eof(Instant::now()),
        }
        let now = Instant::now();
        if session.poll_timeout().is_some_and(|t| t <= now) {
            session.handle_timeout(now);
        }
        while let Some(event) = session.poll_event() {
            match event {
                AgentEvent::Ready(_) => log.lock().unwrap().ready = true,
                AgentEvent::Request(request) => {
                    log.lock().unwrap().requests.push(request.clone());
                    if let Some(result) = handler(&request) {
                        session.respond(request.id, result);
                    }
                }
                AgentEvent::Closed(reason) => {
                    log.lock().unwrap().closed = Some(reason);
                    while let Some(bytes) = session.poll_transmit() {
                        let _ = stream.write_all(&bytes);
                    }
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
            }
        }
    }
}
