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

use mirror::{Mirror, MirrorError, Ref, Snapshot};

use crate::driver::{
    AppControl, AttachOptions, Attachment, Capture, Driver, InputNotes, InputSession, Report,
    Screen, TargetInfo,
};
use crate::wire::{Failure, Success};

/// The longest the thread waits between pumps when nothing falls due.
const IDLE_PUMP: Duration = Duration::from_secs(1);

/// How much longer than the mirror's minimum re-read interval a tree read
/// waits for the re-read that changed frames call for.
const CATCH_UP_MARGIN: Duration = Duration::from_millis(100);

/// How long a tree read lets the mirror run before it looks again whether
/// a read began after the last frame change: a read due for another
/// reason, such as input, runs in the first slice.
const CATCH_UP_SLICE: Duration = Duration::from_millis(5);

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
    /// What the driver knows about the input it dispatched last.
    pub input_notes: Option<Arc<dyn InputNotes>>,
    /// The input session, for a driver whose first input waits for it.
    pub input_session: Option<Box<dyn InputSession>>,
    /// The screen, for gestures no element anchors; read as it is when a
    /// command needs it.
    pub display: Box<dyn Screen>,
    /// Whether the target has a frame source.
    pub frames: bool,
    sessions: HashMap<String, SessionOnTarget>,
    /// The session whose delta snapshots the mirror's delta state follows;
    /// another session's first delta snapshot starts again in full.
    pub delta_session: Option<String>,
    status: Arc<Mutex<Status>>,
}

impl Attached {
    /// The target `info` as `attachment` attached it, its status shared
    /// with its worker.
    pub(crate) fn new(
        info: &TargetInfo,
        attachment: Attachment,
        status: &Arc<Mutex<Status>>,
    ) -> Attached {
        let frames = attachment.backend.frames.is_some();
        Attached {
            info: info.clone(),
            mirror: Mirror::new(attachment.backend, attachment.config),
            apps: attachment.apps,
            capture: attachment.capture,
            input_notes: attachment.input_notes,
            input_session: attachment.input_session,
            display: attachment.display,
            frames,
            sessions: HashMap::new(),
            delta_session: None,
            status: Arc::clone(status),
        }
    }

    /// Brings the replica up to the display before the tree is read. On a
    /// window that sends no change events, frames that changed after its
    /// latest read began call for another read, which the mirror schedules
    /// no sooner than its minimum re-read interval after that read, or at
    /// once for the window of a field it has just pasted or set text into;
    /// this waits for it, so a snapshot right after a tap shows what the
    /// tap did rather than what the tap's own re-read caught. The mirror
    /// times both: what arrived counts once drained, without reading the
    /// tree.
    pub fn catch_up(&mut self) -> Result<(), MirrorError> {
        if !self.frames {
            return Ok(());
        }
        // The re-read falls due within the minimum interval of the last
        // read's end, which has passed.
        let deadline = Instant::now() + self.mirror.config().min_reread_interval + CATCH_UP_MARGIN;
        loop {
            self.mirror.drain();
            let stamps = self.mirror.timestamps();
            let Some(changed) = stamps.last_frame_change else {
                return Ok(());
            };
            if stamps.last_read.is_some_and(|read| read.started > changed) {
                return Ok(());
            }
            // No window waits for frames: they send change events.
            let Some(due) = self.mirror.next_wakeup() else {
                return Ok(());
            };
            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }
            let until = due.min(now + CATCH_UP_SLICE).clamp(now, deadline);
            self.mirror.run_until(until)?;
        }
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
    /// Disconnects as the thread ends, its target detached.
    ended: Receiver<()>,
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
        let (ending, ended) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let report: ReportSlot = Arc::new(Mutex::new(None));
        let (shared_status, shared_report) = (Arc::clone(&status), Arc::clone(&report));
        let name = format!("target {}", info.id);
        let thread = std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                // Dropped as the thread ends, after the target is detached.
                let _ending: Sender<()> = ending;
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
            ended,
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

    /// Waits until `deadline` for the worker to finish detaching, returning
    /// as soon as it has; whether it has.
    pub fn join(mut self, deadline: Instant) -> bool {
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let wait = deadline.saturating_duration_since(Instant::now());
        match self.ended.recv_timeout(wait) {
            Err(RecvTimeoutError::Timeout) => false,
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                true
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
            Ok(Job::Run(task, options, reply)) => {
                if attached.is_none() {
                    match driver.attach(info, &options) {
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
    use crate::driver::Launch;
    use crate::fake::FakeDriver;
    use mirror::{
        Action, DEFAULT_MIN_REREAD_INTERVAL, DEFAULT_POLL_INTERVAL, RefreshMode, SettleOptions,
        SnapshotOptions, Target,
    };

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
            .mirror
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

    /// Taps the element `id` names on `target`'s screen, as an interactive
    /// snapshot lists it.
    fn tap(target: &mut Attached, id: &str) {
        let snapshot = target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        let reference = ref_of(&snapshot, id);
        target
            .mirror
            .act(&Action::Tap {
                target: Target::Ref(reference),
                hold: None,
            })
            .unwrap();
    }

    /// Physical-ios, task 5.2 (design D23): on a fake iPhone, whose frames
    /// are screenshots at the device's rate, the re-read a tap calls for
    /// waits for the tap's first changed frame, so a snapshot right after
    /// the tap shows what the tap did, with no frame-driven re-read to wait
    /// for; on a fake Simulator, whose frames are a stream, the tap's
    /// re-read runs at once, before the app has changed.
    #[test]
    fn on_a_fake_iphone_a_snapshot_right_after_a_tap_shows_what_the_tap_did() {
        for (spec, opened) in [("ios-device", true), ("ios", false)] {
            let mut target = attached(spec, Duration::from_millis(120));
            tap(&mut target, "launcher-fixture");
            let after = target
                .mirror
                .snapshot(&SnapshotOptions::interactive())
                .unwrap();
            assert_eq!(
                after.text.contains("id=tab-login"),
                opened,
                "{spec}:\n{}",
                after.text
            );
        }
    }

    /// Physical-ios, task 5.2: a fake iPhone's windows send no change
    /// events, so the mirror re-reads them as it re-reads a Simulator's: on
    /// its poll while the screen is still, and when frames change no sooner
    /// than its minimum interval after the last read.
    #[test]
    fn a_fake_iphones_windows_are_re_read_on_the_poll_and_on_frame_changes() {
        let mut target = attached("ios-device", Duration::ZERO);
        let launcher = target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        assert!(
            launcher
                .windows
                .iter()
                .all(|window| window.source.mode == RefreshMode::Reread),
            "{:?}",
            launcher.windows
        );
        let read = target.mirror.timestamps().last_read.unwrap();
        assert_eq!(
            target.mirror.next_wakeup(),
            Some(read.started + DEFAULT_POLL_INTERVAL),
            "a still screen is read again on the poll"
        );
        // The Anim tab's spinner keeps the frames changing.
        let anim = Launch {
            relaunch: false,
            env: vec![("FIXTURE_INITIAL_TAB".into(), "anim".into())],
        };
        target
            .apps
            .launch("dev.mobileagent.fixture.expo", &anim)
            .unwrap();
        target.mirror.note_window_change();
        target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        let read = target.mirror.timestamps().last_read.unwrap();
        std::thread::sleep(Duration::from_millis(150));
        target.mirror.drain();
        assert!(
            target
                .mirror
                .timestamps()
                .last_frame_change
                .is_some_and(|at| at > read.started),
            "the spinner changed the frames"
        );
        assert_eq!(
            target.mirror.next_wakeup(),
            Some(read.finished + DEFAULT_MIN_REREAD_INTERVAL),
            "changed frames call for a read the minimum interval after the last"
        );
    }

    /// Physical-ios, task 5.2 (design D23): settle on a fake iPhone after a
    /// tap whose effect shows later than the quiet window holds for the
    /// tap's first changed frame and counts it, where a fake Simulator,
    /// whose frames are a stream, settles a quiet window after the tap,
    /// before the change.
    #[test]
    fn settle_on_a_fake_iphone_holds_for_the_first_changed_frame_after_a_tap() {
        for (spec, counted) in [("ios-device", true), ("ios", false)] {
            let mut target = attached(spec, Duration::from_millis(300));
            tap(&mut target, "launcher-fixture");
            let settle = target.mirror.settle(&SettleOptions::default()).unwrap();
            assert!(settle.settled, "{spec}: {settle:?}");
            assert_eq!(settle.frame_changes > 0, counted, "{spec}: {settle:?}");
        }
    }

    /// Physical-ios, task 5.2 (design D23, "without any frames"): a fake
    /// iPhone whose screenshots could not be subscribed to settles on the
    /// probe-less re-read path for windows without events: re-read every
    /// minimum interval while waiting, a re-read finding the tap's change.
    #[test]
    fn settle_on_a_fake_iphone_without_frames_re_reads_every_minimum_interval() {
        let driver = FakeDriver::parse("ios-device")
            .unwrap()
            .with_latency(Duration::from_millis(150))
            .with_device_frames(false);
        let info = driver.discover().unwrap().remove(0);
        let attachment = driver.attach(&info, &AttachOptions::default()).unwrap();
        assert!(attachment.backend.frames.is_none());
        assert!(attachment.config.slow_frames.is_some());
        let mut target = Attached::new(&info, attachment, &Arc::default());
        tap(&mut target, "launcher-fixture");
        let tapped = Instant::now();
        let settle = target.mirror.settle(&SettleOptions::default()).unwrap();
        assert!(settle.settled, "{settle:?}");
        assert!(
            settle.tree_changes > 0,
            "a re-read found the change: {settle:?}"
        );
        let read = target.mirror.timestamps().last_read.unwrap();
        assert!(
            read.started >= tapped + DEFAULT_MIN_REREAD_INTERVAL - Duration::from_millis(20),
            "the change showed in a re-read a minimum interval after the tap's"
        );
        let after = target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        assert!(after.text.contains("id=tab-login"), "{}", after.text);
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
