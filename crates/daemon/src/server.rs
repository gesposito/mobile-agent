//! The daemon process: startup and replacement of a daemon of another
//! version, the socket, one thread per connection, the idle exit and the
//! cleanup of the sidecar files.
//!
//! Startups serialize on a lock file. Under it a new daemon looks for one
//! already serving the socket: one of the same version stays and the new
//! one leaves; one of another version is asked to shut down (then sent
//! SIGTERM and SIGKILL if it does not) and replaced. The new daemon writes
//! its pid and version files before it binds the socket, so a client that
//! reaches the socket finds the version of the daemon behind it. A daemon
//! removes only files that are still its own when it exits.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::driver;
use crate::handlers;
use crate::paths::{self, Paths};
use crate::registry::Registry;
use crate::wire::{self, Failure, Request, Success, code};
use crate::worker::Outcome;

/// How long the daemon stays up without requests unless told otherwise.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// The longest request line the daemon reads.
const MAX_LINE: u64 = 16 << 20;

/// How long a connection may take to send its request.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a daemon of another version gets to leave on request.
const REPLACE_WAIT: Duration = Duration::from_secs(5);

const SIGKILL: i32 = 9;
const SIGTERM: i32 = 15;
const EPERM: i32 = 1;

unsafe extern "C" {
    safe fn kill(pid: i32, signal: i32) -> i32;
}

/// How a daemon runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub socket_dir: PathBuf,
    /// `None` never exits for idleness.
    pub idle_timeout: Option<Duration>,
    /// Fake targets, as `MOBILE_AGENT_FAKE_TARGETS` lists them.
    pub fake: Option<String>,
    /// How long fake targets take to show a change.
    pub fake_latency: Duration,
    /// Device drivers to register, as `MOBILE_AGENT_DRIVERS` lists them;
    /// `None` registers the host's device drivers unless fake targets are
    /// named (see [`driver::device_drivers`]).
    pub drivers: Option<String>,
}

impl Options {
    /// Reads `--socket-dir <dir>`, `--idle-timeout <time>`,
    /// `--fake <targets>` and `--fake-latency <time>`, each falling back to
    /// its environment variable: `MOBILE_AGENT_SOCKET_DIR` (then the runtime
    /// directory, see [`paths::default_dir`]), `MOBILE_AGENT_IDLE_TIMEOUT_MS`,
    /// `MOBILE_AGENT_FAKE_TARGETS` and `MOBILE_AGENT_FAKE_LATENCY_MS`.
    pub fn from_args(args: impl IntoIterator<Item = OsString>) -> Result<Options, String> {
        let mut socket_dir = None;
        let mut idle = None;
        let mut fake = None;
        let mut latency = None;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let arg = arg.to_string_lossy().into_owned();
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) if name.starts_with("--") => {
                    (name.to_owned(), Some(value.to_owned()))
                }
                _ => (arg.clone(), None),
            };
            let mut value = || -> Result<OsString, String> {
                match &inline {
                    Some(value) => Ok(OsString::from(value)),
                    None => args.next().ok_or_else(|| format!("{name} needs a value")),
                }
            };
            match name.as_str() {
                "--socket-dir" => socket_dir = Some(PathBuf::from(value()?)),
                "--idle-timeout" => idle = Some(value()?.to_string_lossy().into_owned()),
                "--fake" => fake = Some(value()?.to_string_lossy().into_owned()),
                "--fake-latency" => latency = Some(value()?.to_string_lossy().into_owned()),
                other => return Err(format!("unknown argument {other}; see --help")),
            }
        }
        let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
        let idle = match idle.or_else(|| env("MOBILE_AGENT_IDLE_TIMEOUT_MS")) {
            Some(text) => {
                parse_duration(&text).map_err(|e| format!("idle timeout {text:?}: {e}"))?
            }
            None => DEFAULT_IDLE_TIMEOUT,
        };
        let fake_latency = match latency.or_else(|| env("MOBILE_AGENT_FAKE_LATENCY_MS")) {
            Some(text) => {
                parse_duration(&text).map_err(|e| format!("fake latency {text:?}: {e}"))?
            }
            None => Duration::ZERO,
        };
        Ok(Options {
            socket_dir: socket_dir.unwrap_or_else(paths::default_dir),
            idle_timeout: (!idle.is_zero()).then_some(idle),
            fake: fake.or_else(|| env("MOBILE_AGENT_FAKE_TARGETS")),
            fake_latency,
            drivers: None,
        })
    }
}

/// Parses a duration as agent-browser's `--idle-timeout` does: whole
/// milliseconds, or a whole number with `s`, `m` or `h`.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let (number, unit) = match text.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (&text[..i], &text[i..]),
        _ => (text, ""),
    };
    let n: u64 = number
        .parse()
        .map_err(|_| "expected whole milliseconds, or a number with s, m or h".to_owned())?;
    let ms = match unit {
        "" => Some(n),
        "s" => n.checked_mul(1000),
        "m" => n.checked_mul(60_000),
        "h" => n.checked_mul(3_600_000),
        _ => {
            return Err(format!(
                "unknown unit {unit:?}; use s, m, h or none for milliseconds"
            ));
        }
    };
    ms.map(Duration::from_millis)
        .ok_or_else(|| "the duration is too long".to_owned())
}

/// How [`run`] ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The daemon served until shut down or idle.
    Stopped,
    /// A daemon of this version already serves the socket and keeps it.
    AlreadyRunning { pid: Option<u32> },
}

/// The state every connection shares.
pub(crate) struct Daemon {
    registry: Mutex<Registry>,
    pub paths: Paths,
    pub version: String,
    pub started: Instant,
    pub idle: Option<Duration>,
    activity: Mutex<Activity>,
    changed: Condvar,
    stopping: AtomicBool,
}

struct Activity {
    last: Instant,
    in_flight: usize,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Daemon {
    pub fn registry(&self) -> MutexGuard<'_, Registry> {
        lock(&self.registry)
    }

    fn begin(&self) {
        let mut activity = lock(&self.activity);
        activity.in_flight += 1;
        activity.last = Instant::now();
    }

    fn end(&self) {
        let mut activity = lock(&self.activity);
        activity.in_flight -= 1;
        activity.last = Instant::now();
        self.changed.notify_all();
    }

    fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// Stops accepting connections: the accept loop wakes on a connection
    /// of our own and sees the flag.
    fn stop(&self) {
        if !self.stopping.swap(true, Ordering::SeqCst) {
            self.changed.notify_all();
            let _ = UnixStream::connect(self.paths.socket());
        }
    }
}

/// Runs a daemon until it is shut down or stays idle for its timeout.
pub fn run(options: Options) -> Result<Exit, String> {
    let drivers = driver::registered(&driver::Registration {
        fake: options.fake.as_deref(),
        fake_latency: options.fake_latency,
        drivers: options.drivers.as_deref(),
        state_dir: &options.socket_dir,
    })?;
    let paths = Paths::new(options.socket_dir);
    paths.check_socket_len()?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(paths.dir())
        .map_err(|e| format!("cannot create {}: {e}", paths.dir().display()))?;
    let version = paths::own_version();
    let lock_file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.lock())
        .map_err(|e| format!("cannot open {}: {e}", paths.lock().display()))?;
    lock_file
        .lock()
        .map_err(|e| format!("cannot lock {}: {e}", paths.lock().display()))?;
    if let Ok(stream) = UnixStream::connect(paths.socket()) {
        let pid = read_pid(&paths);
        let recorded = fs::read_to_string(paths.version()).unwrap_or_default();
        if recorded.trim() == version {
            return Ok(Exit::AlreadyRunning { pid });
        }
        replace(&paths, stream, pid, recorded.trim());
    }
    for file in [paths.socket(), paths.pid(), paths.version()] {
        let _ = fs::remove_file(file);
    }
    write_file(&paths.pid(), &format!("{}\n", std::process::id()))?;
    write_file(&paths.version(), &format!("{version}\n"))?;
    let listener = UnixListener::bind(paths.socket())
        .map_err(|e| format!("cannot listen on {}: {e}", paths.socket().display()))?;
    let socket_id = file_id(&paths.socket());
    drop(lock_file);

    let daemon = Arc::new(Daemon {
        registry: Mutex::new(Registry::new(drivers)),
        paths: paths.clone(),
        version: version.clone(),
        started: Instant::now(),
        idle: options.idle_timeout,
        activity: Mutex::new(Activity {
            last: Instant::now(),
            in_flight: 0,
        }),
        changed: Condvar::new(),
        stopping: AtomicBool::new(false),
    });
    eprintln!(
        "mobile-agent-daemon: serving {} (pid {}, version {version})",
        paths.socket().display(),
        std::process::id()
    );
    if let Some(idle) = daemon.idle {
        let watched = Arc::clone(&daemon);
        std::thread::spawn(move || watch_idle(&watched, idle));
    }
    for stream in listener.incoming() {
        if daemon.stopping() {
            break;
        }
        match stream {
            Ok(stream) => {
                let served = Arc::clone(&daemon);
                std::thread::spawn(move || serve(&served, stream));
            }
            Err(error) => eprintln!("mobile-agent-daemon: accept failed: {error}"),
        }
    }
    drop(listener);
    daemon.registry().stop();
    if socket_id.is_some() && file_id(&paths.socket()) == socket_id {
        let _ = fs::remove_file(paths.socket());
    }
    if read_pid(&paths) == Some(std::process::id()) {
        let _ = fs::remove_file(paths.version());
        let _ = fs::remove_file(paths.pid());
    }
    Ok(Exit::Stopped)
}

/// Writes a sidecar file whole, through a temporary file and a rename.
fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    let temporary = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&temporary, contents)
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn read_pid(paths: &Paths) -> Option<u32> {
    fs::read_to_string(paths.pid()).ok()?.trim().parse().ok()
}

/// The device and inode of a file, to tell our socket from a successor's.
fn file_id(path: &Path) -> Option<(u64, u64)> {
    fs::symlink_metadata(path)
        .ok()
        .map(|meta| (meta.dev(), meta.ino()))
}

fn process_exists(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(EPERM)
}

/// Waits up to `limit` for `done`.
fn wait_for(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Asks the daemon of another version on `stream` to shut down and waits
/// for it to finish its cleanup; signals it if it does not.
fn replace(paths: &Paths, mut stream: UnixStream, pid: Option<u32>, recorded: &str) {
    eprintln!(
        "mobile-agent-daemon: replacing the daemon of version {recorded:?} (pid {})",
        pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string())
    );
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.write_all(b"{\"id\":\"replace\",\"action\":\"shutdown\"}\n");
    let mut reply = String::new();
    let _ = BufReader::new(&stream).read_line(&mut reply);
    drop(stream);
    let gone = || match pid {
        Some(pid) => !process_exists(pid) || read_pid(paths) != Some(pid),
        None => UnixStream::connect(paths.socket()).is_err(),
    };
    if wait_for(REPLACE_WAIT, gone) {
        return;
    }
    let Some(pid) = pid.and_then(|pid| i32::try_from(pid).ok()) else {
        return;
    };
    kill(pid, SIGTERM);
    if !wait_for(Duration::from_secs(1), gone) {
        kill(pid, SIGKILL);
        wait_for(Duration::from_secs(1), gone);
    }
}

/// Exits the daemon once no request has run for `idle`.
fn watch_idle(daemon: &Daemon, idle: Duration) {
    let mut activity = lock(&daemon.activity);
    loop {
        if daemon.stopping() {
            return;
        }
        if activity.in_flight > 0 {
            activity = daemon
                .changed
                .wait(activity)
                .unwrap_or_else(PoisonError::into_inner);
            continue;
        }
        let deadline = activity.last + idle;
        let now = Instant::now();
        if now >= deadline {
            drop(activity);
            eprintln!(
                "mobile-agent-daemon: no requests for {} ms, exiting",
                idle.as_millis()
            );
            daemon.stop();
            return;
        }
        activity = daemon
            .changed
            .wait_timeout(activity, deadline - now)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

/// Serves one connection: request lines in, reply lines out.
fn serve(daemon: &Daemon, stream: UnixStream) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    loop {
        let mut line = String::new();
        match reader.by_ref().take(MAX_LINE).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        daemon.begin();
        let reply = respond(daemon, line);
        let written = writer.write_all(reply.line.as_bytes()).is_ok();
        daemon.end();
        if reply.stop {
            daemon.stop();
            return;
        }
        if reply.close || !written {
            return;
        }
    }
}

struct Reply {
    line: String,
    /// Shut the daemon down once the reply is out.
    stop: bool,
    /// Close the connection once the reply is out.
    close: bool,
}

fn respond(daemon: &Daemon, line: &str) -> Reply {
    if wire::looks_like_http(line) {
        let refusal: Outcome = Err(Failure::new(
            code::INVALID_REQUEST,
            "this socket speaks newline-delimited JSON, not HTTP",
        ));
        return Reply {
            line: wire::reply_line(&serde_json::Value::Null, &refusal),
            stop: false,
            close: true,
        };
    }
    let request = match Request::parse(line) {
        Ok(request) => request,
        Err(failure) => {
            let id = failure.id.clone().unwrap_or(serde_json::Value::Null);
            return Reply {
                line: wire::reply_line(&id, &Err(failure)),
                stop: false,
                close: false,
            };
        }
    };
    if request.action == "shutdown" {
        let stopped: Outcome = Ok(Success::new(json!({"stopped": true})));
        return Reply {
            line: wire::reply_line(&request.id, &stopped),
            stop: true,
            close: true,
        };
    }
    let outcome = handlers::handle(daemon, &request);
    Reply {
        line: wire::reply_line(&request.id, &outcome),
        stop: false,
        close: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_agent_browser_reads_idle_timeouts() {
        assert_eq!(parse_duration("10"), Ok(Duration::from_millis(10)));
        assert_eq!(parse_duration("10s"), Ok(Duration::from_secs(10)));
        assert_eq!(parse_duration("3m"), Ok(Duration::from_secs(180)));
        assert_eq!(parse_duration("1h"), Ok(Duration::from_secs(3600)));
        assert_eq!(parse_duration("0"), Ok(Duration::ZERO));
        assert!(parse_duration("10M").is_err());
        assert!(parse_duration("10x").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("s").is_err());
    }

    #[test]
    fn flags_override_the_environment_and_zero_disables_the_idle_exit() {
        let options = Options::from_args(
            [
                "--socket-dir",
                "/tmp/x",
                "--idle-timeout=0",
                "--fake",
                "ios",
            ]
            .map(OsString::from),
        )
        .unwrap();
        assert_eq!(options.socket_dir, PathBuf::from("/tmp/x"));
        assert_eq!(options.idle_timeout, None);
        assert_eq!(options.fake.as_deref(), Some("ios"));
        assert!(Options::from_args([OsString::from("--bogus")]).is_err());
    }

    #[test]
    fn the_drivers_flag_names_the_device_drivers_to_register() {
        let options =
            Options::from_args(["--drivers", "android", "--fake=ios"].map(OsString::from)).unwrap();
        assert_eq!(options.drivers.as_deref(), Some("android"));
        assert_eq!(options.fake.as_deref(), Some("ios"));
        let inline = Options::from_args([OsString::from("--drivers=none")]).unwrap();
        assert_eq!(inline.drivers.as_deref(), Some("none"));
        assert!(Options::from_args([OsString::from("--drivers")]).is_err());
    }
}
