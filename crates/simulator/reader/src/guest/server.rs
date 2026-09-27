//! Serves the device protocol (`crates/protocol`) to the host over one Unix
//! socket connection: the agent's hello, the host's welcome, then `snapshot`
//! requests until the host shuts the reader down or goes away.
//!
//! Reads run on a worker thread that owns the private runtime, so the
//! connection keeps answering pings while a slow read runs. A request that
//! arrives during a read is answered `busy`. A read that runs past
//! [`READ_BUDGET`] means a private call hung, and the process exits, which
//! the host sees as the end of the connection.

use std::io::{self, Read as _, Write as _};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use objc2::rc::autoreleasepool;
use protocol::message::{Call, Reply, SnapshotParams};
use protocol::session::DisconnectReason;
use protocol::session::{AgentConfig, AgentEvent, AgentSession};
use protocol::snapshot::Snapshot;

/// How long one read may take before the reader gives up on the process.
pub const READ_BUDGET: Duration = Duration::from_secs(20);

/// The exit status of a reader that ended a hung read.
pub const HUNG_READ_STATUS: i32 = 70;

/// What answers snapshot requests: the runtime inside the Simulator, a fake
/// in tests.
pub trait Source {
    fn read(&self, params: &SnapshotParams) -> Result<Snapshot, protocol::Error>;
}

struct Job {
    id: u32,
    params: SnapshotParams,
}

struct Done {
    id: u32,
    result: Result<Snapshot, protocol::Error>,
}

/// The thread that owns the source.
pub struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// Readable once a result is waiting.
    wake: UnixStream,
}

impl Worker {
    /// Starts the worker, which builds its source with `make` on its own
    /// thread; returns once the source is built, or with why it could not be.
    pub fn start<S, F>(make: F) -> Result<Worker, String>
    where
        S: Source,
        F: FnOnce() -> Result<S, String> + Send + 'static,
    {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Done>();
        let (built_tx, built_rx) = mpsc::channel::<Result<(), String>>();
        let (wake, mut waker) = UnixStream::pair().map_err(|e| e.to_string())?;
        thread::Builder::new()
            .name("reader".into())
            .spawn(move || {
                let source = match autoreleasepool(|_| make()) {
                    Ok(source) => {
                        let _ = built_tx.send(Ok(()));
                        source
                    }
                    Err(reason) => {
                        let _ = built_tx.send(Err(reason));
                        return;
                    }
                };
                for job in job_rx {
                    let result = autoreleasepool(|_| source.read(&job.params));
                    if done_tx.send(Done { id: job.id, result }).is_err() {
                        return;
                    }
                    let _ = waker.write_all(&[1]);
                }
            })
            .map_err(|e| e.to_string())?;
        built_rx
            .recv()
            .map_err(|_| "the reader thread ended while loading".to_owned())??;
        Ok(Worker { jobs, done, wake })
    }
}

/// Serves the host on `stream` until the session closes, and returns why.
pub fn serve(
    mut stream: UnixStream,
    config: AgentConfig,
    worker: Worker,
) -> io::Result<DisconnectReason> {
    let mut session = AgentSession::new(config, Instant::now());
    let mut buffer = vec![0u8; 64 * 1024];
    let mut wake = worker.wake;
    // The request being read, and since when.
    let mut busy: Option<(u32, Instant)> = None;
    loop {
        while let Some(event) = session.poll_event() {
            match event {
                AgentEvent::Ready(_) => {}
                AgentEvent::Closed(reason) => {
                    flush(&mut stream, &mut session)?;
                    return Ok(reason);
                }
                AgentEvent::Request(request) => match request.call {
                    Call::Snapshot(params) => {
                        if let Some((reading, _)) = busy {
                            session.respond(
                                request.id,
                                Err(protocol::Error::new(
                                    protocol::code::BUSY,
                                    format!("the reader is still reading for request {reading}"),
                                )),
                            );
                        } else {
                            busy = Some((request.id, Instant::now()));
                            let job = Job {
                                id: request.id,
                                params,
                            };
                            if worker.jobs.send(job).is_err() {
                                return Err(io::Error::other("the reader thread ended"));
                            }
                        }
                    }
                    Call::Shutdown => {
                        session.respond(request.id, Ok(Reply::Empty));
                        flush(&mut stream, &mut session)?;
                        return Ok(DisconnectReason::Closed);
                    }
                    other => {
                        session.respond(
                            request.id,
                            Err(protocol::Error::unknown_method(other.method())),
                        );
                    }
                },
            }
        }
        flush(&mut stream, &mut session)?;

        if let Some((id, since)) = busy
            && since.elapsed() > READ_BUDGET
        {
            eprintln!("simulator-reader: read {id} ran past {READ_BUDGET:?}; exiting");
            // SAFETY: _exit has no preconditions; nothing needs unwinding.
            unsafe { libc::_exit(HUNG_READ_STATUS) };
        }

        let now = Instant::now();
        let mut timeout = session.poll_timeout().map_or(Duration::from_secs(1), |at| {
            at.saturating_duration_since(now)
        });
        if busy.is_some() {
            timeout = timeout.min(Duration::from_secs(1));
        }
        let (readable, woken) = wait(&stream, &wake, timeout)?;
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
            let _ = wake.read(&mut drained);
            while let Ok(done) = worker.done.try_recv() {
                if busy.is_some_and(|(id, _)| id == done.id) {
                    busy = None;
                }
                session.respond(done.id, done.result.map(Reply::Snapshot));
            }
        }
        session.handle_timeout(Instant::now());
    }
}

fn flush(stream: &mut UnixStream, session: &mut AgentSession) -> io::Result<()> {
    while let Some(bytes) = session.poll_transmit() {
        stream.write_all(&bytes)?;
    }
    Ok(())
}

/// Waits until the connection or the worker has something, or `timeout`.
fn wait(stream: &UnixStream, wake: &UnixStream, timeout: Duration) -> io::Result<(bool, bool)> {
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
    use std::sync::{Arc, Mutex};

    use protocol::Capabilities;
    use protocol::message::Method;
    use protocol::session::{HostConfig, HostEvent, HostSession};
    use protocol::snapshot::Truncation;

    use super::*;

    /// A source that answers after `delay` with an empty snapshot whose
    /// `node_count` counts the reads, or fails when asked for depth 0.
    struct Fake {
        delay: Duration,
        reads: Arc<Mutex<u32>>,
    }

    impl Source for Fake {
        fn read(&self, params: &SnapshotParams) -> Result<Snapshot, protocol::Error> {
            thread::sleep(self.delay);
            if params.max_depth == Some(0) {
                return Err(protocol::Error {
                    code: "no_foreground".into(),
                    message: "nothing in front".into(),
                    retryable: true,
                });
            }
            let mut reads = self.reads.lock().unwrap();
            *reads += 1;
            Ok(Snapshot {
                strings: vec![],
                windows: vec![],
                node_count: *reads,
                truncated: Truncation::EMPTY,
                seq: 0,
            })
        }
    }

    fn agent_config() -> AgentConfig {
        AgentConfig {
            capabilities: Capabilities::TREE,
            agent: "simulator-reader test".into(),
            token: Some("t".into()),
            ..AgentConfig::default()
        }
    }

    /// A host end driven by hand over a socket.
    struct Host {
        stream: UnixStream,
        session: HostSession,
    }

    impl Host {
        fn new(stream: UnixStream) -> Host {
            stream
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            Host {
                stream,
                session: HostSession::new(
                    HostConfig {
                        token: Some("t".into()),
                        ..HostConfig::default()
                    },
                    Instant::now(),
                ),
            }
        }

        /// Pumps the connection until an event matches, for up to `limit`.
        fn until(
            &mut self,
            limit: Duration,
            mut wanted: impl FnMut(&HostEvent) -> bool,
        ) -> HostEvent {
            let start = Instant::now();
            let mut buffer = [0u8; 4096];
            loop {
                while let Some(bytes) = self.session.poll_transmit() {
                    self.stream.write_all(&bytes).unwrap();
                }
                while let Some(event) = self.session.poll_event() {
                    if wanted(&event) {
                        return event;
                    }
                }
                assert!(start.elapsed() < limit, "nothing matched within {limit:?}");
                match self.stream.read(&mut buffer) {
                    Ok(0) => self.session.receive_eof(Instant::now()),
                    Ok(n) => self.session.receive(&buffer[..n], Instant::now()),
                    Err(_) => {}
                }
                self.session.handle_timeout(Instant::now());
            }
        }

        fn request(&mut self, call: Call) -> u32 {
            self.session
                .send_request(call, Some(Duration::from_secs(30)), Instant::now())
                .expect("the request goes out")
        }
    }

    fn start(delay: Duration) -> (Host, thread::JoinHandle<io::Result<DisconnectReason>>) {
        let (agent_end, host_end) = UnixStream::pair().unwrap();
        let worker = Worker::start(move || {
            Ok(Fake {
                delay,
                reads: Arc::new(Mutex::new(0)),
            })
        })
        .unwrap();
        let server = thread::spawn(move || serve(agent_end, agent_config(), worker));
        let mut host = Host::new(host_end);
        host.until(Duration::from_secs(2), |e| matches!(e, HostEvent::Ready(_)));
        (host, server)
    }

    fn response(event: HostEvent) -> (u32, Result<Reply, protocol::Error>) {
        match event {
            HostEvent::Response { id, result, .. } => (id, result),
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn a_snapshot_request_is_answered_after_the_handshake() {
        let (mut host, server) = start(Duration::ZERO);
        let negotiated = host.session.negotiated().unwrap().clone();
        assert!(negotiated.capabilities.contains(Capabilities::TREE));
        assert_eq!(negotiated.agent, "simulator-reader test");
        let id = host.request(Call::Snapshot(SnapshotParams::default()));
        let (answered, result) = response(host.until(Duration::from_secs(2), |e| {
            matches!(e, HostEvent::Response { .. })
        }));
        assert_eq!(answered, id);
        assert!(matches!(result, Ok(Reply::Snapshot(s)) if s.node_count == 1));

        let failing = host.request(Call::Snapshot(SnapshotParams {
            max_depth: Some(0),
            ..SnapshotParams::default()
        }));
        let (answered, result) = response(host.until(Duration::from_secs(2), |e| {
            matches!(e, HostEvent::Response { .. })
        }));
        assert_eq!(answered, failing);
        let error = result.unwrap_err();
        assert_eq!(error.code, "no_foreground");
        assert!(error.retryable);

        host.request(Call::Shutdown);
        let (_, result) = response(host.until(
            Duration::from_secs(2),
            |e| matches!(e, HostEvent::Response { method, .. } if *method == Method::SHUTDOWN),
        ));
        assert_eq!(result, Ok(Reply::Empty));
        assert_eq!(server.join().unwrap().unwrap(), DisconnectReason::Closed);
    }

    #[test]
    fn a_request_during_a_read_is_answered_busy() {
        let (mut host, _server) = start(Duration::from_millis(400));
        let first = host.request(Call::Snapshot(SnapshotParams::default()));
        let second = host.request(Call::Snapshot(SnapshotParams::default()));
        let mut answers = Vec::new();
        while answers.len() < 2 {
            answers.push(response(host.until(Duration::from_secs(3), |e| {
                matches!(e, HostEvent::Response { .. })
            })));
        }
        let busy = answers.iter().find(|(id, _)| *id == second).unwrap();
        assert_eq!(busy.1.as_ref().unwrap_err().code, protocol::code::BUSY);
        let read = answers.iter().find(|(id, _)| *id == first).unwrap();
        assert!(read.1.is_ok());
    }

    #[test]
    fn the_connection_stays_alive_through_a_read_longer_than_the_peer_timeout() {
        let (mut host, _server) = start(Duration::from_millis(3500));
        let id = host.request(Call::Snapshot(SnapshotParams::default()));
        let (answered, result) = response(host.until(Duration::from_secs(6), |e| {
            matches!(e, HostEvent::Response { .. } | HostEvent::Closed(_))
        }));
        assert_eq!(answered, id);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn the_reader_stops_when_the_host_goes_away() {
        let (host, server) = start(Duration::ZERO);
        drop(host);
        assert_eq!(server.join().unwrap().unwrap(), DisconnectReason::Eof);
    }

    #[test]
    fn a_source_that_cannot_load_says_why() {
        let error = Worker::start(|| -> Result<Fake, String> { Err("no runtime here".into()) })
            .err()
            .expect("the worker refuses");
        assert_eq!(error, "no runtime here");
    }
}
