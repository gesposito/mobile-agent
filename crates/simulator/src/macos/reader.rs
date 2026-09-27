//! The reader spawned into each booted Simulator to read its tree (design
//! D6, spike S4).
//!
//! `build.rs` builds it from `reader/` for `aarch64-apple-ios-sim` and this
//! crate embeds it; the first spawn writes it into the host's cache, from
//! where every Simulator runs it. It is spawned through CoreSimulator, never
//! `xcrun simctl spawn`, whose own start misses the 500 ms readiness gate.
//! The host listens on a Unix socket; the reader loads its runtime, then
//! connects and speaks the device protocol (`crates/protocol`) as an agent
//! with the tree capability, so the connection itself means it can read.
//! Readiness is the time from the spawn call to its hello.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use protocol::message::{Call, Reply, SnapshotParams};
use protocol::session::{HostConfig, HostEvent, HostSession};
use protocol::snapshot::Snapshot;

use crate::error::{Error, Failure, Result};
use crate::macos::coresim::Device;
use crate::model::{ReaderState, ReaderStatus, TreeStatus};
use crate::tree::TreeBackend;

/// The program's name.
pub const NAME: &str = "simulator-reader";

/// The reader `build.rs` built for the Simulator; empty when it could not.
static EMBEDDED: &[u8] = include_bytes!(env!("SIMULATOR_READER_BINARY"));
/// Why `build.rs` could not build it; empty when it did.
const MISSING: &str = include_str!(env!("SIMULATOR_READER_MISSING"));

/// How long a reader gets from its spawn to its hello. S4 measured 292 ms at
/// the median, and about a second on a host under a first-boot load.
const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// How often an idle connection is serviced, so the reader's pings are
/// answered well within the protocol's 3 s peer timeout.
const KEEPALIVE: Duration = Duration::from_millis(250);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Where the driver keeps the reader and its logs.
fn cache_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Caches"))
        .filter(|caches| caches.is_dir())
        .unwrap_or_else(std::env::temp_dir)
        .join("dev.mobileagent.simulator")
}

/// FNV-1a, to name the cached reader after its content.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The reader built into this crate, written into the host's cache once per
/// content; or why there is none.
pub fn embedded() -> std::result::Result<PathBuf, String> {
    static INSTALLED: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    INSTALLED.get_or_init(install).clone()
}

fn install() -> std::result::Result<PathBuf, String> {
    if EMBEDDED.is_empty() {
        return Err(format!("this build has no reader: {}", MISSING.trim()));
    }
    let dir = cache_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = dir.join(format!("{NAME}-{:016x}", fnv1a(EMBEDDED)));
    if fs::metadata(&path).is_ok_and(|meta| meta.len() == EMBEDDED.len() as u64) {
        return Ok(path);
    }
    let partial = dir.join(format!(".{NAME}-{}", std::process::id()));
    let written = fs::write(&partial, EMBEDDED)
        .and_then(|()| fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)))
        .and_then(|()| fs::rename(&partial, &path));
    if let Err(error) = written {
        let _ = fs::remove_file(&partial);
        return Err(format!("writing {}: {error}", path.display()));
    }
    Ok(path)
}

/// A socket path in the host's temporary directory, short enough for
/// `sun_path`.
fn socket_path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "mobile-agent-reader-{}-{}.sock",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let temporary = std::env::temp_dir().join(&name);
    if temporary.as_os_str().len() < 100 {
        temporary
    } else {
        PathBuf::from("/tmp").join(name)
    }
}

/// A per-spawn token the reader's hello must carry.
fn token() -> String {
    let mut bytes = [0u8; 16];
    if File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .is_err()
    {
        let seed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        bytes = (seed ^ u128::from(std::process::id())).to_le_bytes();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The end of a log, for a diagnostic.
fn tail(log: &Path) -> String {
    let text = fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().rev().take(5).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
}

/// A termination status as the words for it: an exit status, or a signal.
fn describe_status(status: i32) -> String {
    let signal = status & 0x7f;
    if signal == 0 {
        format!("exited with status {}", (status >> 8) & 0xff)
    } else {
        format!("was killed by signal {signal}")
    }
}

/// The connection to one reader and its protocol session.
struct Link {
    stream: UnixStream,
    session: HostSession,
    buffer: Vec<u8>,
}

impl Link {
    fn flush(&mut self) -> io::Result<()> {
        while let Some(bytes) = self.session.poll_transmit() {
            self.stream.write_all(&bytes)?;
        }
        Ok(())
    }

    /// Reads what arrives within `wait` into the session, runs its timers
    /// and sends what it has to.
    fn pump(&mut self, wait: Duration) -> io::Result<()> {
        let mut poll = libc::pollfd {
            fd: self.stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = wait.as_millis().min(i32::MAX as u128) as libc::c_int;
        // SAFETY: one valid pollfd.
        let ready = unsafe { libc::poll(&mut poll, 1, millis) };
        if ready > 0 {
            match self.stream.read(&mut self.buffer) {
                Ok(0) => self.session.receive_eof(Instant::now()),
                Ok(n) => {
                    let (session, buffer) = (&mut self.session, &self.buffer);
                    session.receive(&buffer[..n], Instant::now());
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    self.session.receive_eof(Instant::now());
                    return Err(error);
                }
            }
        }
        self.session.handle_timeout(Instant::now());
        self.flush()
    }
}

/// What spawning a reader came to.
pub enum Spawned {
    Ready(Reader),
    /// It cannot serve this device, and why; the translator reads instead.
    Unusable(String),
}

/// A reader running in one Simulator.
pub struct Reader {
    link: Arc<Mutex<Link>>,
    closed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    keepalive: Option<JoinHandle<()>>,
    /// The termination status CoreSimulator reported, once it exited.
    ended: Arc<Mutex<Option<i32>>>,
    pid: i32,
    path: PathBuf,
    name: String,
    spawned_at: SystemTime,
    ready_in: Duration,
}

impl Reader {
    /// Spawns `program` into `device`, whose screen has `scale` pixels per
    /// point, and waits for its hello. Its standard error goes to `log`.
    /// Fails only when a private call raises; a reader that cannot start is
    /// [`Spawned::Unusable`].
    pub fn spawn(device: &Device, program: &Path, scale: f64, log: &Path) -> Result<Spawned> {
        let socket = socket_path();
        let _ = fs::remove_file(&socket);
        let listener = match UnixListener::bind(&socket) {
            Ok(listener) => listener,
            Err(error) => {
                return Ok(Spawned::Unusable(format!(
                    "listening on {}: {error}",
                    socket.display()
                )));
            }
        };
        let _cleanup = RemoveOnDrop(socket.clone());
        if let Err(error) = listener.set_nonblocking(true) {
            return Ok(Spawned::Unusable(error.to_string()));
        }
        let token = token();
        let arguments = vec![
            "connect".to_owned(),
            socket.to_string_lossy().into_owned(),
            "--token".to_owned(),
            token.clone(),
            "--scale".to_owned(),
            scale.to_string(),
        ];
        let log_file = File::create(log).ok();
        let ended = Arc::new(Mutex::new(None));
        let started = Instant::now();
        let spawned_at = SystemTime::now();
        let spawned = {
            let ended = Arc::clone(&ended);
            device.spawn(
                program,
                &arguments,
                log_file.as_ref().map(AsRawFd::as_raw_fd),
                move |status| *lock(&ended) = Some(status),
            )
        };
        let pid = match spawned {
            Ok(pid) => pid,
            Err(
                error @ Error::Framework {
                    failure: Failure::Exception { .. } | Failure::Panic(_),
                    ..
                },
            ) => return Err(error),
            Err(error) => {
                return Ok(Spawned::Unusable(format!(
                    "spawning {} failed: {error}",
                    program.display()
                )));
            }
        };
        let unusable = |why: String| {
            // A reader that has not ended is stopped; once CoreSimulator has
            // reported its end, its pid may belong to another process.
            if lock(&ended).is_none() {
                // SAFETY: kill has no memory preconditions.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
            let logged = tail(log);
            Spawned::Unusable(if logged.is_empty() {
                why
            } else {
                format!("{why}; it wrote: {logged}")
            })
        };

        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Ok(unusable(format!("accepting its connection: {error}"))),
            }
            if let Some(status) = *lock(&ended) {
                return Ok(unusable(format!(
                    "the reader {} before connecting",
                    describe_status(status)
                )));
            }
            if started.elapsed() > READY_TIMEOUT {
                return Ok(unusable(format!(
                    "the reader did not connect within {READY_TIMEOUT:?}"
                )));
            }
            thread::sleep(Duration::from_millis(1));
        };
        if let Err(error) = stream.set_nonblocking(false) {
            return Ok(unusable(error.to_string()));
        }
        let mut link = Link {
            stream,
            session: HostSession::new(
                HostConfig {
                    token: Some(token),
                    ..HostConfig::default()
                },
                Instant::now(),
            ),
            buffer: vec![0u8; 256 * 1024],
        };
        let negotiated = loop {
            if let Err(error) = link.pump(Duration::from_millis(20)) {
                return Ok(unusable(format!("its connection failed: {error}")));
            }
            match link.session.poll_event() {
                Some(HostEvent::Ready(negotiated)) => break negotiated,
                Some(HostEvent::Closed(reason)) => {
                    return Ok(unusable(format!("its handshake failed: {reason}")));
                }
                _ if started.elapsed() > READY_TIMEOUT => {
                    return Ok(unusable(format!(
                        "the reader sent no hello within {READY_TIMEOUT:?}"
                    )));
                }
                _ => {}
            }
        };
        let ready_in = started.elapsed();

        let link = Arc::new(Mutex::new(link));
        let closed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let keepalive = {
            let (link, closed, stop) = (Arc::clone(&link), Arc::clone(&closed), Arc::clone(&stop));
            thread::Builder::new()
                .name(format!("{NAME}-{pid}"))
                .spawn(move || keep_alive(&link, &closed, &stop))
                .ok()
        };
        Ok(Spawned::Ready(Reader {
            link,
            closed,
            stop,
            keepalive,
            ended,
            pid,
            path: program.to_path_buf(),
            name: negotiated.agent,
            spawned_at,
            ready_in,
        }))
    }

    /// Whether the reader can still serve reads.
    pub fn alive(&self) -> bool {
        !self.closed.load(Ordering::Relaxed) && lock(&self.ended).is_none()
    }

    /// Reads the foreground's tree. A failure is the reader's answer, or the
    /// protocol's `disconnected` when the reader went away.
    pub fn snapshot(
        &self,
        params: SnapshotParams,
        timeout: Duration,
    ) -> std::result::Result<Snapshot, protocol::Error> {
        let mut link = lock(&self.link);
        let id =
            link.session
                .send_request(Call::Snapshot(params), Some(timeout), Instant::now())?;
        loop {
            if let Err(error) = link.flush() {
                self.closed.store(true, Ordering::Relaxed);
                return Err(protocol::Error::disconnected(error));
            }
            while let Some(event) = link.session.poll_event() {
                match event {
                    HostEvent::Response {
                        id: answered,
                        result,
                        ..
                    } if answered == id => {
                        return match result {
                            Ok(Reply::Snapshot(snapshot)) => Ok(snapshot),
                            Ok(other) => Err(protocol::Error::new(
                                protocol::code::PROTOCOL_ERROR,
                                format!("the reader answered a snapshot with {other:?}"),
                            )),
                            Err(error) => {
                                if error.code == protocol::code::DISCONNECTED {
                                    self.closed.store(true, Ordering::Relaxed);
                                }
                                Err(error)
                            }
                        };
                    }
                    HostEvent::Closed(reason) => {
                        self.closed.store(true, Ordering::Relaxed);
                        return Err(protocol::Error::disconnected(reason));
                    }
                    _ => {}
                }
            }
            if let Err(error) = link.pump(Duration::from_millis(100)) {
                self.closed.store(true, Ordering::Relaxed);
                return Err(protocol::Error::disconnected(error));
            }
        }
    }

    /// The reader as target status reports it.
    pub fn status(&self, spawns: u32) -> ReaderStatus {
        let state = match *lock(&self.ended) {
            Some(status) => ReaderState::Exited(Some(status)),
            None if self.closed.load(Ordering::Relaxed) => {
                ReaderState::Failed("its connection ended".into())
            }
            None => ReaderState::Ready,
        };
        ReaderStatus {
            name: self.name.clone(),
            path: self.path.clone(),
            pid: self.pid,
            spawned_at: self.spawned_at,
            ready_in: Some(self.ready_in),
            state,
            spawns,
        }
    }
}

impl Drop for Reader {
    /// Asks the reader to exit; it also exits when the connection closes.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        {
            let mut link = lock(&self.link);
            if !link.session.is_closed()
                && link
                    .session
                    .send_request(Call::Shutdown, None, Instant::now())
                    .is_ok()
            {
                let _ = link.flush();
            }
            let _ = link.stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(keepalive) = self.keepalive.take() {
            let _ = keepalive.join();
        }
    }
}

/// Services an idle connection: answers the reader's pings, pings it, and
/// notices when it is gone.
fn keep_alive(link: &Mutex<Link>, closed: &AtomicBool, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(KEEPALIVE);
        let Ok(mut link) = link.try_lock() else {
            continue;
        };
        if link.pump(Duration::ZERO).is_err() || link.session.is_closed() {
            closed.store(true, Ordering::Relaxed);
            return;
        }
        while link.session.poll_event().is_some() {}
    }
}

/// Removes the listening socket once the reader is connected, or gave up.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Spawns that fail in a row before a device's reads stay on the translator.
const SPAWN_ATTEMPTS: u32 = 2;

/// A device's tree backend: its reader once spawned, and what failed.
#[derive(Default)]
pub struct TreeSlot {
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    reader: Option<Arc<Reader>>,
    /// The status of the last reader, once it is gone.
    last: Option<ReaderStatus>,
    spawns: u32,
    /// Spawns that failed in a row.
    failures: u32,
    /// Why the reader cannot run on this device, once it is known for good.
    unusable: Option<String>,
    /// The backend of the last read, and why.
    backend: Option<(TreeBackend, String)>,
}

impl TreeSlot {
    /// The device's reader, spawned with `spawn` when none runs. The inner
    /// error says why no reader can serve the read.
    pub fn reader(
        &self,
        spawn: impl FnOnce() -> Result<Spawned>,
    ) -> Result<std::result::Result<Arc<Reader>, String>> {
        let mut state = lock(&self.state);
        if let Some(reader) = state.reader.as_ref().filter(|reader| reader.alive()) {
            return Ok(Ok(Arc::clone(reader)));
        }
        if let Some(gone) = state.reader.take() {
            state.last = Some(gone.status(state.spawns));
        }
        if let Some(why) = &state.unusable {
            return Ok(Err(why.clone()));
        }
        state.spawns += 1;
        match spawn()? {
            Spawned::Ready(reader) => {
                state.failures = 0;
                let reader = Arc::new(reader);
                state.reader = Some(Arc::clone(&reader));
                Ok(Ok(reader))
            }
            Spawned::Unusable(why) => {
                state.failures += 1;
                if state.failures >= SPAWN_ATTEMPTS {
                    state.unusable = Some(why.clone());
                }
                Ok(Err(why))
            }
        }
    }

    /// Records that the reader cannot run on this device at all.
    pub fn give_up(&self, why: String) {
        lock(&self.state).unusable = Some(why);
    }

    /// Stops using `reader`, whose connection ended.
    pub fn lost(&self, reader: &Arc<Reader>) {
        let mut state = lock(&self.state);
        if state
            .reader
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, reader))
        {
            state.reader = None;
            state.last = Some(reader.status(state.spawns));
        }
    }

    /// Records which backend read last, and why.
    pub fn used(&self, backend: TreeBackend, reason: String) {
        lock(&self.state).backend = Some((backend, reason));
    }

    /// The device's tree reading as target status reports it; `None` before
    /// the first read.
    pub fn status(&self) -> Option<TreeStatus> {
        let state = lock(&self.state);
        let (backend, reason) = state.backend.clone()?;
        let reader = state
            .reader
            .as_ref()
            .map(|reader| reader.status(state.spawns))
            .or_else(|| state.last.clone());
        Some(TreeStatus {
            backend,
            reason,
            reader,
        })
    }
}

/// The log a device's reader writes its standard error to.
pub fn log_path(udid: &str) -> PathBuf {
    let dir = cache_dir();
    let _ = fs::create_dir_all(&dir);
    dir.join(format!("{NAME}-{udid}.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_embedded_a_reader_for_the_simulator() {
        assert!(
            !EMBEDDED.is_empty(),
            "no reader was built into this crate: {MISSING}"
        );
        // A Mach-O arm64 executable: MH_MAGIC_64, CPU_TYPE_ARM64.
        assert_eq!(&EMBEDDED[..4], &[0xcf, 0xfa, 0xed, 0xfe]);
        assert_eq!(&EMBEDDED[4..8], &[0x0c, 0x00, 0x00, 0x01]);
        let path = embedded().expect("the reader is written to the cache");
        let meta = fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), EMBEDDED.len() as u64);
        assert_eq!(meta.permissions().mode() & 0o111, 0o111, "executable");
        assert_eq!(embedded().unwrap(), path, "written once");
    }

    #[test]
    fn socket_paths_fit_sun_path_and_differ() {
        let (one, two) = (socket_path(), socket_path());
        assert_ne!(one, two);
        assert!(one.as_os_str().len() < 104, "{}", one.display());
    }

    #[test]
    fn tokens_differ() {
        let (one, two) = (token(), token());
        assert_eq!(one.len(), 32);
        assert_ne!(one, two);
    }

    #[test]
    fn statuses_read_as_exits_or_signals() {
        assert_eq!(describe_status(3 << 8), "exited with status 3");
        assert_eq!(describe_status(9), "was killed by signal 9");
    }
}
