//! The runner's protocol end (design D20): a listener on the device's
//! loopback, which the host reaches through usbmuxd's `Connect`, one host
//! connection at a time, the protocol's `AgentSession` driven over it, and
//! every request run on one worker thread, so the connection keeps answering
//! pings while a slow request runs. A request that arrives during another is
//! answered `busy`. Adapted from the Simulator reader's `guest/server.rs`,
//! which serves one Unix socket.

use std::io::{self, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use protocol::Capabilities;
use protocol::message::{Call, Reply};
use protocol::session::{AgentConfig, AgentEvent, AgentSession, DisconnectReason};

/// What the runner advertises: tree reads, input, screenshots and direct
/// text setting. It sends no change events and streams no frames: the host
/// takes frames itself (design D23).
pub const CAPABILITIES: Capabilities = Capabilities(
    Capabilities::TREE.0
        | Capabilities::INPUT.0
        | Capabilities::SCREENSHOT.0
        | Capabilities::TEXT_SET.0,
);

/// What answers the requests: the shim's XCTest calls on a device, a fake in
/// tests. It runs on the worker thread alone.
pub trait Handler: Send + 'static {
    fn handle(&mut self, call: Call) -> Result<Reply, protocol::Error>;
}

/// The hello the runner sends: its name and version, its capabilities and
/// the protocol's defaults.
pub fn config() -> AgentConfig {
    AgentConfig {
        capabilities: CAPABILITIES,
        agent: crate::AGENT.to_owned(),
        ..AgentConfig::default()
    }
}

/// Listens on `address`, starts the worker that owns `handler`, and serves
/// one host connection after another on a thread of its own; returns the
/// address listened on once it listens.
pub fn start<H: Handler>(address: SocketAddr, handler: H) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(address)?;
    let bound = listener.local_addr()?;
    let mut worker = Worker::start(handler)?;
    thread::Builder::new()
        .name("runner-listener".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let _ = stream.set_nodelay(true);
                match serve(stream, &mut worker) {
                    Ok(reason) => eprintln!("mobile-agent runner: the connection ended: {reason}"),
                    Err(error) => eprintln!("mobile-agent runner: the connection failed: {error}"),
                }
            }
        })?;
    Ok(bound)
}

struct Job {
    id: u32,
    call: Call,
}

struct Done {
    id: u32,
    result: Result<Reply, protocol::Error>,
}

/// The thread that owns the handler.
struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// Readable once a result is waiting.
    wake: UnixStream,
}

impl Worker {
    fn start<H: Handler>(mut handler: H) -> io::Result<Worker> {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Done>();
        let (wake, mut waker) = UnixStream::pair()?;
        thread::Builder::new()
            .name("runner-worker".into())
            .spawn(move || {
                for job in job_rx {
                    let result = handler.handle(job.call);
                    if done_tx.send(Done { id: job.id, result }).is_err() {
                        return;
                    }
                    let _ = waker.write_all(&[1]);
                }
            })?;
        Ok(Worker { jobs, done, wake })
    }
}

/// Serves the host on `stream` until the session closes, and returns why.
fn serve(mut stream: TcpStream, worker: &mut Worker) -> io::Result<DisconnectReason> {
    let mut session = AgentSession::new(config(), Instant::now());
    let mut buffer = vec![0u8; 64 * 1024];
    // The results of an earlier connection's requests are stale.
    while worker.done.try_recv().is_ok() {}
    // The request the worker runs.
    let mut busy: Option<u32> = None;
    loop {
        while let Some(event) = session.poll_event() {
            match event {
                AgentEvent::Ready(_) => {}
                AgentEvent::Closed(reason) => {
                    flush(&mut stream, &mut session)?;
                    return Ok(reason);
                }
                AgentEvent::Request(request) => match request.call {
                    Call::Shutdown => {
                        session.respond(request.id, Ok(Reply::Empty));
                        flush(&mut stream, &mut session)?;
                        return Ok(DisconnectReason::Closed);
                    }
                    call => {
                        if let Some(running) = busy {
                            session.respond(
                                request.id,
                                Err(protocol::Error::new(
                                    protocol::code::BUSY,
                                    format!("the runner is still serving request {running}"),
                                )),
                            );
                        } else {
                            busy = Some(request.id);
                            let job = Job {
                                id: request.id,
                                call,
                            };
                            if worker.jobs.send(job).is_err() {
                                return Err(io::Error::other("the worker ended"));
                            }
                        }
                    }
                },
            }
        }
        flush(&mut stream, &mut session)?;

        let now = Instant::now();
        let mut timeout = session.poll_timeout().map_or(Duration::from_secs(1), |at| {
            at.saturating_duration_since(now)
        });
        if busy.is_some() {
            timeout = timeout.min(Duration::from_millis(500));
        }
        let (readable, woken) = wait(&stream, &worker.wake, timeout)?;
        if readable {
            match stream.read(&mut buffer) {
                Ok(0) => session.receive_eof(Instant::now()),
                Ok(n) => session.receive(&buffer[..n], Instant::now()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        if woken {
            let mut drained = [0u8; 64];
            let _ = worker.wake.read(&mut drained);
            while let Ok(done) = worker.done.try_recv() {
                if busy == Some(done.id) {
                    busy = None;
                }
                session.respond(done.id, done.result);
            }
        }
        session.handle_timeout(Instant::now());
    }
}

fn flush(stream: &mut TcpStream, session: &mut AgentSession) -> io::Result<()> {
    while let Some(bytes) = session.poll_transmit() {
        stream.write_all(&bytes)?;
    }
    Ok(())
}

/// Waits until the connection or the worker has something, or `timeout`.
fn wait(stream: &TcpStream, wake: &UnixStream, timeout: Duration) -> io::Result<(bool, bool)> {
    let mut fds = [
        libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: wake.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let millis = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
    // SAFETY: two valid pollfds.
    let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, millis) };
    if ready < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok((false, false))
        } else {
            Err(error)
        };
    }
    let flagged =
        |fd: &libc::pollfd| fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0;
    Ok((flagged(&fds[0]), flagged(&fds[1])))
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::path::Path;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;
    use std::time::{Duration, Instant};

    use protocol::frame::HEADER_LEN;
    use protocol::message::{
        ImageFormat, InputAck, Message, Method, Rotation, Screenshot, ScreenshotParams,
        SnapshotParams, TapParams,
    };
    use protocol::session::{HostConfig, HostEvent, HostSession};

    use super::*;

    /// A handler that records each call, answers after `delay`, and fails a
    /// tap at x = -1 as the platform would.
    struct Recorder {
        calls: Sender<Call>,
        delay: Duration,
    }

    impl Handler for Recorder {
        fn handle(&mut self, call: Call) -> Result<Reply, protocol::Error> {
            thread::sleep(self.delay);
            let _ = self.calls.send(call.clone());
            match call {
                Call::Tap(TapParams { x: -1, .. }) => Err(protocol::Error::new(
                    protocol::code::INTERNAL,
                    "the synthesizer refused",
                )),
                Call::Tap(_) | Call::Swipe(_) | Call::Key(_) => {
                    Ok(Reply::Input(InputAck { accepted: true }))
                }
                Call::Screenshot(_) => Ok(Reply::Screenshot(Screenshot {
                    format: ImageFormat::PNG,
                    width: 750,
                    height: 1334,
                    rotation: Rotation::R0,
                    data: vec![0x89, b'P', b'N', b'G'],
                })),
                other => Err(protocol::Error::unknown_method(other.method())),
            }
        }
    }

    fn serve(delay: Duration) -> (SocketAddr, Receiver<Call>) {
        let (calls, recorded) = mpsc::channel();
        let address = start((Ipv4Addr::LOCALHOST, 0).into(), Recorder { calls, delay })
            .expect("the runner listens");
        (address, recorded)
    }

    /// The host's end of one connection, driven by hand.
    struct Host {
        stream: TcpStream,
        session: HostSession,
    }

    impl Host {
        fn connect(address: SocketAddr) -> Host {
            let stream = TcpStream::connect(address).expect("connects");
            stream
                .set_read_timeout(Some(Duration::from_millis(20)))
                .expect("a read timeout");
            Host {
                stream,
                session: HostSession::new(HostConfig::default(), Instant::now()),
            }
        }

        /// Pumps the connection until an event matches, for up to `limit`.
        fn until(&mut self, limit: Duration, wanted: impl Fn(&HostEvent) -> bool) -> HostEvent {
            let started = Instant::now();
            let mut buffer = [0u8; 4096];
            loop {
                while let Some(bytes) = self.session.poll_transmit() {
                    self.stream.write_all(&bytes).expect("writes");
                }
                while let Some(event) = self.session.poll_event() {
                    if wanted(&event) {
                        return event;
                    }
                }
                assert!(
                    started.elapsed() < limit,
                    "nothing matched within {limit:?}"
                );
                match self.stream.read(&mut buffer) {
                    Ok(0) => self.session.receive_eof(Instant::now()),
                    Ok(n) => self.session.receive(&buffer[..n], Instant::now()),
                    Err(_) => {}
                }
                self.session.handle_timeout(Instant::now());
            }
        }

        fn ready(&mut self) {
            self.until(Duration::from_secs(2), |event| {
                matches!(event, HostEvent::Ready(_))
            });
        }

        fn request(&mut self, call: Call) -> u32 {
            self.session
                .send_request(call, Some(Duration::from_secs(5)), Instant::now())
                .expect("the request goes out")
        }

        fn response(&mut self) -> (u32, Result<Reply, protocol::Error>) {
            self.response_within(Duration::from_secs(3))
        }

        fn response_within(&mut self, limit: Duration) -> (u32, Result<Reply, protocol::Error>) {
            match self.until(limit, |event| matches!(event, HostEvent::Response { .. })) {
                HostEvent::Response { id, result, .. } => (id, result),
                other => panic!("a response, not {other:?}"),
            }
        }
    }

    #[test]
    fn the_hello_names_the_runner_its_version_and_its_capabilities() {
        let (address, _) = serve(Duration::ZERO);
        let mut host = Host::connect(address);
        host.ready();
        let negotiated = host.session.negotiated().expect("negotiated").clone();
        assert_eq!(negotiated.agent, crate::AGENT);
        assert_eq!(
            negotiated.agent,
            format!("mobile-agent-ios-runner {}", crate::VERSION)
        );
        assert_eq!(
            negotiated.capabilities,
            Capabilities::TREE
                | Capabilities::INPUT
                | Capabilities::SCREENSHOT
                | Capabilities::TEXT_SET
        );
        assert_eq!(negotiated.capabilities, CAPABILITIES);
        assert_eq!(negotiated.version, 1);
    }

    #[test]
    fn requests_are_answered_through_the_handler_and_its_failures_pass_through() {
        let (address, recorded) = serve(Duration::ZERO);
        let mut host = Host::connect(address);
        host.ready();
        let tap = Call::Tap(TapParams {
            x: 100,
            y: 200,
            duration_ms: None,
        });
        let id = host.request(tap.clone());
        assert_eq!(
            host.response(),
            (id, Ok(Reply::Input(InputAck { accepted: true })))
        );
        assert_eq!(recorded.recv_timeout(Duration::from_secs(1)), Ok(tap));

        let id = host.request(Call::Tap(TapParams {
            x: -1,
            y: 0,
            duration_ms: None,
        }));
        let (answered, result) = host.response();
        assert_eq!(answered, id);
        assert_eq!(result.expect_err("refused").code, protocol::code::INTERNAL);

        let id = host.request(Call::Screenshot(ScreenshotParams::default()));
        let (answered, result) = host.response();
        assert_eq!(answered, id);
        assert!(matches!(result, Ok(Reply::Screenshot(s)) if s.width == 750));
    }

    #[test]
    fn a_request_during_another_is_answered_busy() {
        let (address, _) = serve(Duration::from_millis(300));
        let mut host = Host::connect(address);
        host.ready();
        let first = host.request(Call::Snapshot(SnapshotParams::default()));
        let second = host.request(Call::Screenshot(ScreenshotParams::default()));
        let answers = [host.response(), host.response()];
        let busy = answers
            .iter()
            .find(|(id, _)| *id == second)
            .expect("the second");
        assert_eq!(
            busy.1.as_ref().expect_err("busy").code,
            protocol::code::BUSY
        );
        assert!(answers.iter().any(|(id, _)| *id == first));
    }

    #[test]
    fn the_connection_answers_pings_while_a_slow_request_runs() {
        let (address, _) = serve(Duration::from_millis(3500));
        let mut host = Host::connect(address);
        host.ready();
        let id = host.request(Call::Screenshot(ScreenshotParams::default()));
        let (answered, result) = host.response_within(Duration::from_secs(6));
        assert_eq!(answered, id);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn shutdown_ends_the_connection_and_the_runner_takes_the_next() {
        let (address, recorded) = serve(Duration::ZERO);
        let mut host = Host::connect(address);
        host.ready();
        host.request(Call::Shutdown);
        let (_, result) = host.response();
        assert_eq!(result, Ok(Reply::Empty));
        host.until(Duration::from_secs(2), |event| {
            matches!(event, HostEvent::Closed(_))
        });
        drop(host);

        let mut again = Host::connect(address);
        again.ready();
        let tap = Call::Tap(TapParams {
            x: 1,
            y: 2,
            duration_ms: Some(600),
        });
        let id = again.request(tap.clone());
        assert_eq!(again.response().0, id);
        assert_eq!(recorded.recv_timeout(Duration::from_secs(1)), Ok(tap));
    }

    #[test]
    fn a_host_that_goes_away_leaves_the_runner_listening() {
        let (address, _) = serve(Duration::ZERO);
        let mut host = Host::connect(address);
        host.ready();
        drop(host);
        let mut again = Host::connect(address);
        again.ready();
    }

    const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../crates/protocol/golden");

    /// The frame in `name`, and the message it holds.
    fn golden(name: &str) -> (Vec<u8>, Message) {
        let bytes = std::fs::read(Path::new(GOLDEN).join(format!("{name}.bin")))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let message =
            Message::decode(&bytes[HEADER_LEN..]).unwrap_or_else(|e| panic!("{name}: {e}"));
        (bytes, message)
    }

    /// Reads one frame from `stream` and decodes it.
    fn read_message(stream: &mut TcpStream) -> Message {
        let mut header = [0u8; HEADER_LEN];
        stream.read_exact(&mut header).expect("a frame header");
        let mut payload = vec![0u8; u32::from_be_bytes(header) as usize];
        stream.read_exact(&mut payload).expect("a frame's payload");
        Message::decode(&payload).expect("a message")
    }

    /// The protocol's golden frames, written to the runner byte for byte
    /// after the golden welcome: each request reaches the handler as the
    /// call the frame holds, or is answered by the session for a method the
    /// runner does not advertise or know, with the request's id and method.
    #[test]
    fn the_golden_frames_decode_in_the_runners_build() {
        let (address, recorded) = serve(Duration::ZERO);
        let mut stream = TcpStream::connect(address).expect("connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("a read timeout");
        let Message::Hello(hello) = read_message(&mut stream) else {
            panic!("the runner speaks first, with its hello");
        };
        assert_eq!(hello.agent, crate::AGENT);
        stream.write_all(&golden("welcome").0).expect("the welcome");

        let mut names: Vec<String> = std::fs::read_dir(GOLDEN)
            .expect("the golden frames")
            .map(|entry| entry.expect("an entry").path())
            .filter_map(|path| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
            .filter(|name| name.starts_with("request_") && name != "request_shutdown")
            .collect();
        names.sort();
        names.push("request_shutdown".into());
        assert!(names.len() >= 15, "{names:?}");
        for name in &names {
            let (bytes, message) = golden(name);
            let Message::Request(request) = message else {
                panic!("{name} holds a request");
            };
            stream.write_all(&bytes).expect("the request");
            let response = loop {
                match read_message(&mut stream) {
                    Message::Response(response) => break response,
                    Message::Ping(ping) => {
                        let mut pong = Vec::new();
                        protocol::frame::write_frame(
                            &Message::Pong(protocol::message::Pong { nonce: ping.nonce }),
                            u32::MAX,
                            &mut pong,
                        )
                        .expect("a pong");
                        stream.write_all(&pong).expect("the pong");
                    }
                    other => panic!("{name}: {other:?}"),
                }
            };
            assert_eq!(response.id, request.id, "{name}");
            assert_eq!(response.method, request.call.method(), "{name}");
            if request.deadline_ms == Some(0) {
                // The session answers a request whose deadline is already
                // past before the runner serves it.
                assert_eq!(
                    response.result.expect_err(name).code,
                    protocol::code::DEADLINE_EXCEEDED,
                    "{name}"
                );
                let _ = recorded.recv_timeout(Duration::from_millis(100));
                continue;
            }
            let method = request.call.method();
            match method {
                Method::FRAMES_START | Method::FRAMES_STOP | Method::RESYNC | Method::BOUNDS => {
                    assert_eq!(
                        response.result.expect_err(name).code,
                        protocol::code::CAPABILITY_MISSING,
                        "{name}"
                    );
                }
                Method::SHUTDOWN => assert_eq!(response.result, Ok(Reply::Empty)),
                _ if matches!(request.call, Call::Unknown(_)) => {
                    assert_eq!(
                        response.result.expect_err(name).code,
                        protocol::code::UNKNOWN_METHOD,
                        "{name}"
                    );
                }
                _ => assert_eq!(
                    recorded.recv_timeout(Duration::from_secs(1)),
                    Ok(request.call),
                    "{name} reached the handler as the call it holds"
                ),
            }
        }
    }
}
