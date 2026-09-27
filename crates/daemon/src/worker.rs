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

use crate::driver::{
    AppControl, AttachOptions, Attachment, Capture, Driver, Report, Size, TargetInfo,
};
use crate::signals::Observer;
use crate::wire::{Failure, Success};

/// The longest the thread waits between pumps when nothing falls due.
const IDLE_PUMP: Duration = Duration::from_secs(1);

/// How much longer than the mirror's minimum re-read interval a tree read
/// waits for the re-read that changed frames call for.
const CATCH_UP_MARGIN: Duration = Duration::from_millis(100);

/// What a command returns.
pub(crate) type Outcome = Result<Success, Failure>;

type Task = Box<dyn FnOnce(&mut Attached) -> Outcome + Send>;

enum Job {
    /// A command, and how to attach when the target is not attached.
    Run(Task, AttachOptions, Sender<Outcome>),
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
    /// The target `info` as `attachment` attached it, its status shared
    /// with its worker.
    fn new(info: &TargetInfo, attachment: Attachment, status: &Arc<Mutex<Status>>) -> Attached {
        let observer = Observer::default();
        let mut backend = attachment.backend;
        let frames = backend.frames.is_some();
        backend.events = observer.tee(backend.events);
        backend.tree = observer.tee_reads(backend.tree);
        Attached {
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
        }
    }

    /// Brings the replica up to the display before the tree is read. On a
    /// window that sends no change events, frames that changed after its
    /// latest read began call for another read, which the mirror schedules
    /// no sooner than its minimum re-read interval after that read; this
    /// waits for it, so a snapshot right after a tap shows what the tap
    /// did rather than what the tap's own re-read caught.
    pub fn catch_up(&mut self) -> Result<(), MirrorError> {
        if !self.frames {
            return Ok(());
        }
        self.mirror.pump()?;
        let Some(changed) = self.observer.last_frame_change() else {
            return Ok(());
        };
        // The re-read falls due within the minimum interval of the last
        // read's end, which has passed.
        let deadline = Instant::now() + self.mirror.config().min_reread_interval + CATCH_UP_MARGIN;
        while self
            .observer
            .last_read_started()
            .is_none_or(|started| started <= changed)
        {
            // No window waits for frames: they send change events.
            let Some(due) = self.mirror.next_wakeup() else {
                return Ok(());
            };
            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }
            self.mirror.run_until(due.clamp(now, deadline))?;
        }
        Ok(())
    }

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
    /// Whether the attached target has a frame source; `None` until it is
    /// attached.
    pub frames: Option<bool>,
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
    /// Runs `task` on the worker and waits for its outcome, attaching with
    /// `options` when the target is not attached.
    pub fn run<F>(&self, options: AttachOptions, task: F) -> Outcome
    where
        F: FnOnce(&mut Attached) -> Outcome + Send + 'static,
    {
        let (reply, outcome) = mpsc::channel();
        self.jobs
            .send(Job::Run(Box::new(task), options, reply))
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
            Ok(Job::Run(task, _options, reply)) => {
                if attached.is_none() {
                    match driver.attach(info, &AttachOptions::default()) {
                        Ok(attachment) => {
                            let mut current = lock(status);
                            current.attached = true;
                            current.frames = Some(attachment.backend.frames.is_some());
                            current.error = None;
                            drop(current);
                            *lock(report) = attachment.report.clone();
                            attached = Some(Attached::new(info, attachment, status));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeDriver;
    use mirror::{SnapshotOptions, Target};

    /// The first fake target `spec` names, attached, showing each change
    /// `latency` after its cause.
    fn attached(spec: &str, latency: Duration) -> Attached {
        let driver = FakeDriver::parse(spec).unwrap().with_latency(latency);
        let info = driver.discover().unwrap().remove(0);
        let attachment = driver.attach(&info, &AttachOptions::default()).unwrap();
        Attached::new(&info, attachment, &Arc::default())
    }

    fn ref_of(snapshot: &Snapshot, id: &str) -> Ref {
        snapshot
            .elements
            .iter()
            .find(|element| element.identifier.as_deref() == Some(id))
            .and_then(|element| element.reference)
            .unwrap_or_else(|| panic!("no ref for {id} in:\n{}", snapshot.text))
    }

    /// On a Simulator the re-read a tap calls for runs before the app has
    /// changed; the frames that change next call for another read, no
    /// sooner than the mirror's minimum interval after the first.
    #[test]
    fn a_tree_read_waits_for_the_re_read_that_changed_frames_call_for() {
        let mut target = attached("ios", Duration::from_millis(60));
        let launcher = target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        let fixture = ref_of(&launcher, "launcher-fixture");
        target
            .act(&Action::Tap {
                target: Target::Ref(fixture),
                hold: None,
            })
            .unwrap();
        // The tap's re-read, before the app shows the change.
        target.mirror.pump().unwrap();
        std::thread::sleep(Duration::from_millis(120));
        target.catch_up().unwrap();
        let after = target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        assert!(after.text.contains("id=tab-login"), "{}", after.text);
    }

    #[test]
    fn a_tree_read_does_not_wait_when_no_frame_changed_since_the_last_read() {
        let mut target = attached("ios", Duration::ZERO);
        target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        let started = Instant::now();
        target.catch_up().unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "waited {:?}",
            started.elapsed()
        );
    }

    /// A driver that records the options each attach got.
    struct Recording {
        fake: FakeDriver,
        seen: Arc<Mutex<Vec<AttachOptions>>>,
    }

    impl Driver for Recording {
        fn name(&self) -> &str {
            "fake"
        }

        fn discover(&self) -> Result<Vec<TargetInfo>, crate::driver::DriverError> {
            self.fake.discover()
        }

        fn attach(
            &self,
            target: &TargetInfo,
            options: &AttachOptions,
        ) -> Result<Attachment, crate::driver::DriverError> {
            self.seen.lock().unwrap().push(*options);
            if options.force {
                self.fake.attach(target, options)
            } else {
                Err(crate::driver::DriverError::new(
                    "automation_conflict",
                    "another process holds UI automation; run the command again with --force",
                ))
            }
        }
    }

    /// Task 8.11: `--force` reaches the attach, which a conflict refused
    /// without it.
    #[test]
    fn a_forced_command_attaches_with_force_after_an_attach_refused_without() {
        let fake = FakeDriver::parse("android").unwrap();
        let info = fake.discover().unwrap().remove(0);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let driver = Arc::new(Recording {
            fake,
            seen: Arc::clone(&seen),
        });
        let worker = Worker::spawn(info, driver);
        let handle = worker.handle();
        let refused = handle
            .run(AttachOptions::default(), |_| {
                Ok(Success::new(serde_json::json!({})))
            })
            .unwrap_err();
        assert_eq!(refused.code, "automation_conflict");
        assert!(!worker.status().attached);
        handle
            .run(AttachOptions { force: true }, |_| {
                Ok(Success::new(serde_json::json!({})))
            })
            .expect("the forced attach succeeds");
        assert!(worker.status().attached);
        assert_eq!(
            *seen.lock().unwrap(),
            [
                AttachOptions { force: false },
                AttachOptions { force: true }
            ]
        );
        worker.signal_stop();
        worker.join(Instant::now() + Duration::from_secs(2));
    }

    #[test]
    fn a_target_without_frames_never_waits() {
        let mut target = attached("android:R58M123", Duration::from_millis(60));
        target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        let started = Instant::now();
        target.catch_up().unwrap();
        assert!(started.elapsed() < Duration::from_millis(50));
    }
}
