//! One thread per target owns the target's mirror. Commands for a target
//! run on it in arrival order while commands for other targets run on
//! theirs, and between commands it keeps the mirror current, pumping it
//! whenever a poll or a frame-driven re-read falls due.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mirror::{Action, ActionOutcome, Mirror, MirrorError, Ref, Snapshot};

use crate::driver::{AppControl, Capture, Driver, Report, Size, TargetInfo};
use crate::signals::Observer;
use crate::wire::{Failure, Success};

/// The longest the thread waits between pumps when nothing falls due.
const IDLE_PUMP: Duration = Duration::from_secs(1);

/// What a command returns.
pub(crate) type Outcome = Result<Success, Failure>;

type Task = Box<dyn FnOnce(&mut Attached) -> Outcome + Send>;

enum Job {
    Run(Task, Sender<Outcome>),
    Stop,
}

/// What one session keeps about one target.
#[derive(Default)]
pub(crate) struct SessionOnTarget {
    /// The last snapshot the session took, the baseline of `diff snapshot`.
    pub last: Option<Snapshot>,
    /// The refs the session's snapshots listed: a session acts only on refs
    /// issued to it, so refs stay isolated between sessions.
    pub seen: HashSet<Ref>,
    /// The app the session's `open` launched, which `close` stops.
    pub app: Option<String>,
}

/// An attached target, as commands see it.
pub(crate) struct Attached {
    pub info: TargetInfo,
    pub mirror: Mirror,
    pub apps: Box<dyn AppControl>,
    /// Screenshots the driver takes on demand, if it takes them.
    pub capture: Option<Box<dyn Capture>>,
    /// The screen size in points, for gestures no element anchors.
    pub display: Size,
    /// The change signals the target reported, beside the mirror.
    pub observer: Observer,
    /// Whether the target has a frame source.
    pub frames: bool,
    /// When the daemon last injected input.
    pub last_action: Option<Instant>,
    sessions: HashMap<String, SessionOnTarget>,
    /// The session whose delta snapshots the mirror's delta state follows;
    /// another session's first delta snapshot starts again in full.
    pub delta_session: Option<String>,
    status: Arc<Mutex<Status>>,
}

impl Attached {
    /// Injects `action` through the mirror and notes when.
    pub fn act(&mut self, action: &Action) -> Result<ActionOutcome, MirrorError> {
        let outcome = self.mirror.act(action)?;
        self.last_action = Some(Instant::now());
        Ok(outcome)
    }

    /// What `session` keeps about this target.
    pub fn session(&mut self, name: &str) -> &mut SessionOnTarget {
        self.sessions.entry(name.to_owned()).or_default()
    }

    /// Records the app in front for `status`.
    pub fn set_app(&self, app: Option<String>) {
        lock(&self.status).app = app;
    }
}

/// What `status` reports about a target's worker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Status {
    pub attached: bool,
    /// The app a session opened last, until it is closed.
    pub app: Option<String>,
    /// Why the last attach failed.
    pub error: Option<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the driver reports about the attached target, while it is.
type ReportSlot = Arc<Mutex<Option<Arc<dyn Report>>>>;

/// A target's worker thread.
pub(crate) struct Worker {
    handle: Handle,
    status: Arc<Mutex<Status>>,
    report: ReportSlot,
    thread: Option<JoinHandle<()>>,
}

/// Sends commands to a worker; cheap to clone.
#[derive(Clone)]
pub(crate) struct Handle {
    jobs: Sender<Job>,
}

impl Worker {
    /// Starts the worker of `info`; it attaches through `driver` when the
    /// first command arrives.
    pub fn spawn(info: TargetInfo, driver: Arc<dyn Driver>) -> Worker {
        let (jobs, queue) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let report: ReportSlot = Arc::new(Mutex::new(None));
        let (shared_status, shared_report) = (Arc::clone(&status), Arc::clone(&report));
        let name = format!("target {}", info.id);
        let thread = std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                serve(
                    &info,
                    driver.as_ref(),
                    &queue,
                    &shared_status,
                    &shared_report,
                );
            })
            .ok();
        Worker {
            handle: Handle { jobs },
            status,
            report,
            thread,
        }
    }

    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    /// What the driver reports about the target, while it is attached.
    pub fn report(&self) -> Option<Arc<dyn Report>> {
        lock(&self.report).clone()
    }

    /// Asks the worker to stop, which detaches its target.
    pub fn signal_stop(&self) {
        let _ = self.handle.jobs.send(Job::Stop);
    }

    /// Waits until `deadline` for the worker to finish detaching.
    pub fn join(mut self, deadline: Instant) {
        if let Some(thread) = self.thread.take() {
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}

impl Handle {
    /// Runs `task` on the worker and waits for its outcome.
    pub fn run<F>(&self, task: F) -> Outcome
    where
        F: FnOnce(&mut Attached) -> Outcome + Send + 'static,
    {
        let (reply, outcome) = mpsc::channel();
        self.jobs
            .send(Job::Run(Box::new(task), reply))
            .map_err(|_| Failure::internal("the target's worker has stopped"))?;
        outcome
            .recv()
            .map_err(|_| Failure::internal("the target's worker stopped during the command"))?
    }
}

fn serve(
    info: &TargetInfo,
    driver: &dyn Driver,
    queue: &Receiver<Job>,
    status: &Arc<Mutex<Status>>,
    report: &ReportSlot,
) {
    let mut attached: Option<Attached> = None;
    loop {
        let wait = attached
            .as_ref()
            .and_then(|target| target.mirror.next_wakeup())
            .map_or(IDLE_PUMP, |due| {
                due.saturating_duration_since(Instant::now())
                    .clamp(Duration::from_millis(1), IDLE_PUMP)
            });
        match queue.recv_timeout(wait) {
            Ok(Job::Run(task, reply)) => {
                if attached.is_none() {
                    match driver.attach(info) {
                        Ok(attachment) => {
                            let mut current = lock(status);
                            current.attached = true;
                            current.error = None;
                            drop(current);
                            *lock(report) = attachment.report;
                            let observer = Observer::default();
                            let mut backend = attachment.backend;
                            let frames = backend.frames.is_some();
                            backend.events = observer.tee(backend.events);
                            attached = Some(Attached {
                                info: info.clone(),
                                mirror: Mirror::new(backend, attachment.config),
                                apps: attachment.apps,
                                capture: attachment.capture,
                                display: attachment.display,
                                observer,
                                frames,
                                last_action: None,
                                sessions: HashMap::new(),
                                delta_session: None,
                                status: Arc::clone(status),
                            });
                        }
                        Err(error) => {
                            lock(status).error = Some(error.message.clone());
                            let _ = reply.send(Err(error.into()));
                            continue;
                        }
                    }
                }
                let result = match attached.as_mut() {
                    Some(target) => catch_unwind(AssertUnwindSafe(move || task(target))),
                    None => continue,
                };
                let outcome = result.unwrap_or_else(|_| {
                    *lock(report) = None;
                    attached = None;
                    lock(status).attached = false;
                    Err(Failure::internal(format!(
                        "the command failed unexpectedly on {}; the daemon detached the target and attaches again on the next command",
                        info.id
                    )))
                });
                let _ = reply.send(outcome);
            }
            Ok(Job::Stop) | Err(RecvTimeoutError::Disconnected) => {
                // Dropping the attachment detaches the target.
                *lock(report) = None;
                drop(attached);
                lock(status).attached = false;
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(target) = attached.as_mut() {
                    let _ = target.mirror.pump();
                }
            }
        }
    }
}
