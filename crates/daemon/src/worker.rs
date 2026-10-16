//! One thread per target owns the target's mirror. Commands for a target
//! run on it in arrival order while commands for other targets run on
//! theirs, and between commands it keeps the mirror current, pumping it
//! whenever a poll or a frame-driven re-read falls due. A target whose
//! connection is gone is detached, and the next command attaches it again.

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
    /// The last snapshot the session took, the baseline of `diff snapshot`
    /// and of `wait` with `change`.
    pub last: Option<Snapshot>,
    /// The latest frame change that snapshot saw, the frames' baseline of
    /// `wait` with `change`.
    pub frames: Option<Instant>,
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
    /// Whether a keyboard wait has seen a keyboard window that draws no
    /// keys since it last saw a keyboard with keys, as while a hardware
    /// keyboard serves the target's inputs: the keyboard waits then end at
    /// the field's focus, as a number pad brings up no window at all
    /// (rn-app-fixes, design D57).
    pub keys_hidden_seen: bool,
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
            keys_hidden_seen: false,
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
    /// How many times the worker attached the target.
    pub attaches: u32,
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
    /// The target as the listing that started the worker found it.
    info: TargetInfo,
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
        let target = info.clone();
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
            info: target,
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

    /// The target the worker serves.
    pub fn info(&self) -> &TargetInfo {
        &self.info
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

/// Whether a failure with `code` says the target's connection is gone, as
/// the mirror and every driver report it of a target they lost: the
/// physical iPhone's driver of a runner whose connection fell silent and
/// whose restart failed among them (design D26 of physical-ios). The
/// target is then detached, so that the next command attaches it again,
/// which starts over, rather than fail as the attachment does until the
/// daemon stops.
fn gone(code: &str) -> bool {
    code == mirror::code::DISCONNECTED
}

/// Detaches the target: dropping its attachment stops what the driver
/// started for it.
fn detach(attached: &mut Option<Attached>, status: &Mutex<Status>, report: &ReportSlot) {
    *lock(report) = None;
    *attached = None;
    lock(status).attached = false;
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
                            current.attaches += 1;
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
                    detach(&mut attached, status, report);
                    Err(Failure::internal(format!(
                        "the command failed unexpectedly on {}; the daemon detached the target and attaches again on the next command",
                        info.id
                    )))
                });
                // The command has its answer before the target is detached.
                let lost = matches!(&outcome, Err(failure) if gone(&failure.code));
                let _ = reply.send(outcome);
                if lost {
                    detach(&mut attached, status, report);
                }
            }
            Ok(Job::Stop) | Err(RecvTimeoutError::Disconnected) => {
                detach(&mut attached, status, report);
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                let lost = attached
                    .as_mut()
                    .is_some_and(|target| target.mirror.pump().is_err_and(|e| gone(e.code())));
                if lost {
                    detach(&mut attached, status, report);
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
    use mirror::backend::ReadReason;
    use mirror::{
        Action, DEFAULT_MIN_REREAD_INTERVAL, DEFAULT_POLL_INTERVAL, DEFAULT_STILL_POLL_INTERVAL,
        RefreshMode, SettleOptions, SnapshotOptions, Target,
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

    /// Physical-ios, tasks 5.2 and 9.1: a fake iPhone's windows send no
    /// change events, so the mirror re-reads them as it re-reads a
    /// Simulator's when frames change, no sooner than its minimum interval
    /// after the last read; while the screen is still, its frames slow and
    /// present, the poll waits for the still-poll interval (design D26).
    #[test]
    fn a_fake_iphones_windows_are_re_read_on_frame_changes_and_polled_when_still() {
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
            Some(read.started + DEFAULT_STILL_POLL_INTERVAL),
            "a still screen is read again on the still poll"
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

    /// Physical-ios, task 9.1 (design D26): an idle fake iPhone whose frames
    /// hold still is not polled, where a fake Simulator's windows without
    /// events, whose frames stream, are polled every second, and so are a
    /// fake iPhone's whose screenshots could not be subscribed to.
    #[test]
    fn an_idle_fake_iphone_with_still_frames_is_not_polled_where_others_are() {
        for (spec, subscribed, polled) in [
            ("ios-device", true, false),
            ("ios", true, true),
            ("ios-device", false, true),
        ] {
            let driver = FakeDriver::parse(spec)
                .unwrap()
                .with_device_frames(subscribed);
            let info = driver.discover().unwrap().remove(0);
            let attachment = driver.attach(&info, &AttachOptions::default()).unwrap();
            let mut target = Attached::new(&info, attachment, &Arc::default());
            target
                .mirror
                .snapshot(&SnapshotOptions::interactive())
                .unwrap();
            let first = target.mirror.timestamps().last_read.unwrap();
            let what = format!("{spec}, screenshots subscribed: {subscribed}");
            if polled {
                assert_eq!(
                    target.mirror.next_wakeup(),
                    Some(first.started + DEFAULT_POLL_INTERVAL),
                    "{what}"
                );
            }
            target
                .mirror
                .run_until(Instant::now() + DEFAULT_POLL_INTERVAL + Duration::from_millis(300))
                .unwrap();
            let last = target.mirror.timestamps().last_read.unwrap();
            assert_eq!(last.started > first.started, polled, "{what}: {last:?}");
            if polled {
                assert_eq!(last.reason, ReadReason::Poll, "{what}");
            }
        }
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

    /// A tap at a point of `target`'s screen, through the mirror.
    fn tap_point(target: &mut Attached) -> Outcome {
        target
            .mirror
            .act(&Action::Tap {
                target: Target::Point(mirror::Point::new(20.0, 20.0)),
                hold: None,
            })
            .map(|_| Success::new(serde_json::json!({})))
            .map_err(Failure::from)
    }

    /// An interactive snapshot of `target`'s screen.
    fn look(target: &mut Attached) -> Outcome {
        target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .map(|_| Success::new(serde_json::json!({})))
            .map_err(Failure::from)
    }

    /// The runner a fake iPhone's worker reports, as JSON.
    fn runner_of(worker: &Worker) -> serde_json::Value {
        let report = worker.report().expect("attached").report();
        serde_json::Value::Object(report.data)["runner"].clone()
    }

    /// Physical-ios, task 9.3 (design D26; spec `ios-device-driver`,
    /// "Runner connection lost"): on a fake iPhone whose runner's connection
    /// is lost and whose restart fails, the command in flight fails
    /// `disconnected` and the worker detaches the target; the next command
    /// attaches again, which ends the runner the loss left on the phone
    /// before its own starts, and succeeds; the status counts the new
    /// attach. A fake Simulator served by its own worker beside it is
    /// untouched.
    #[test]
    fn a_lost_runner_connection_detaches_the_target_and_the_next_command_attaches_again() {
        let fake = Arc::new(
            FakeDriver::parse("ios-device,ios")
                .unwrap()
                .with_lost_runner(Duration::from_millis(300)),
        );
        let driver: Arc<dyn Driver> = Arc::clone(&fake) as Arc<dyn Driver>;
        let listed = driver.discover().unwrap();
        let phone = Worker::spawn(listed[0].clone(), Arc::clone(&driver));
        let simulator = Worker::spawn(listed[1].clone(), Arc::clone(&driver));
        let run = |worker: &Worker, task: fn(&mut Attached) -> Outcome| {
            worker.handle().run(AttachOptions::default(), task)
        };
        run(&phone, look).expect("the phone serves");
        run(&simulator, look).expect("the Simulator serves");
        assert_eq!(phone.status().attaches, 1);
        let silent = fake.runners_on(&listed[0].id);
        assert_eq!(silent.len(), 1, "{silent:?}");

        std::thread::sleep(Duration::from_millis(400));
        let lost = run(&phone, tap_point).expect_err("the runner's connection is lost");
        assert_eq!(lost.code, "disconnected", "{lost:?}");
        assert!(
            lost.message.contains("starting it again failed"),
            "{lost:?}"
        );
        // The command has its answer first, then the worker detaches.
        let deadline = Instant::now() + Duration::from_secs(2);
        while phone.status().attached && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !phone.status().attached,
            "detached once the loss was reported"
        );
        assert!(phone.report().is_none());
        assert_eq!(
            fake.runners_on(&listed[0].id),
            silent,
            "the silent runner is still on the phone"
        );

        run(&phone, tap_point).expect("attached again, the tap goes through");
        let status = phone.status();
        assert!(status.attached);
        assert_eq!(status.attaches, 2, "the status counts the new attach");
        let runner = runner_of(&phone);
        assert_eq!(runner["leftovers"], serde_json::json!(silent), "{runner}");
        assert_eq!(runner["state"], "ready", "{runner}");
        let now = fake.runners_on(&listed[0].id);
        assert_eq!(now.len(), 1);
        assert_ne!(now, silent, "the new attach's own runner");

        let untouched = simulator.status();
        assert!(untouched.attached);
        assert_eq!(untouched.attaches, 1);
        run(&simulator, tap_point).expect("the Simulator still serves");
        for worker in [phone, simulator] {
            worker.signal_stop();
            worker.join(Instant::now() + Duration::from_secs(2));
        }
    }

    /// Physical-ios, task 9.3 (design D26): a loss a poll between commands
    /// finds, here on a fake iPhone without screenshots, whose windows are
    /// polled every second, detaches the target too, so the next command
    /// attaches again rather than fail first.
    #[test]
    fn a_lost_runner_connection_a_poll_finds_detaches_the_target_before_the_next_command() {
        let driver: Arc<dyn Driver> = Arc::new(
            FakeDriver::parse("ios-device")
                .unwrap()
                .with_device_frames(false)
                .with_lost_runner(Duration::from_millis(300)),
        );
        let phone = Worker::spawn(driver.discover().unwrap().remove(0), driver);
        phone
            .handle()
            .run(AttachOptions::default(), look)
            .expect("serves");
        let deadline = Instant::now() + Duration::from_secs(3);
        while phone.status().attached && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !phone.status().attached,
            "the poll's failed read detached it"
        );
        phone
            .handle()
            .run(AttachOptions::default(), look)
            .expect("attached again");
        assert_eq!(phone.status().attaches, 2);
        phone.signal_stop();
        phone.join(Instant::now() + Duration::from_secs(2));
    }

    /// Text through the runner of `target`, into the field with the focus,
    /// if any.
    fn enter_text(target: &mut Attached) -> Outcome {
        target
            .mirror
            .act(&Action::Text {
                text: String::new(),
                into: None,
                method: protocol::message::TextMethod::DIRECT,
            })
            .map(|_| Success::new(serde_json::json!({})))
            .map_err(Failure::from)
    }

    /// Physical-ios, task 9.7 (design D26; spec `ios-device-driver`,
    /// "Failure containment"): on a fake iPhone whose runner stalls, as the
    /// phone's did on its 43rd emptying fill in task 9.6, the command that
    /// meets the stall fails `runner_stalled`, and the worker keeps the
    /// target attached, its status counting the runner's restart and naming
    /// the stall; the next command is served by the same attachment. The
    /// command that meets a second stall fails `disconnected`, and the
    /// worker detaches the target, whose release ends the stalled runner, so
    /// the next command attaches again; stopping the worker of a phone whose
    /// runner stalled ends the runner within the daemon's 5 s stop.
    #[test]
    fn a_fake_iphones_stalled_runner_is_started_again_and_a_second_stall_detaches_the_target() {
        let fake = Arc::new(FakeDriver::parse("ios-device").unwrap());
        let driver: Arc<dyn Driver> = Arc::clone(&fake) as Arc<dyn Driver>;
        let info = driver.discover().unwrap().remove(0);
        let phone = Worker::spawn(info.clone(), Arc::clone(&driver));
        let run =
            |task: fn(&mut Attached) -> Outcome| phone.handle().run(AttachOptions::default(), task);
        run(look).expect("the phone serves");

        fake.stall_runner(&info.id);
        let stalled = run(enter_text).expect_err("the stall");
        assert_eq!(stalled.code, "runner_stalled", "{stalled:?}");
        assert!(stalled.message.contains("stalled"), "{stalled:?}");
        let status = phone.status();
        assert!(status.attached, "kept attached");
        assert_eq!(status.attaches, 1);
        let runner = runner_of(&phone);
        assert_eq!(runner["restarts"], 1, "{runner}");
        assert_eq!(runner["state"], "ready", "{runner}");
        assert!(
            runner["lastDeath"]
                .as_str()
                .is_some_and(|death| death.contains("stalled")),
            "{runner}"
        );
        run(enter_text).expect("served after the restart");
        assert_eq!(phone.status().attaches, 1, "no new attach");

        fake.stall_runner(&info.id);
        let lost = run(enter_text).expect_err("the second stall");
        assert_eq!(lost.code, "disconnected", "{lost:?}");
        let deadline = Instant::now() + Duration::from_secs(2);
        while phone.status().attached && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!phone.status().attached, "detached at the second stall");
        assert!(
            fake.runners_on(&info.id).is_empty(),
            "the release ended the stalled runner"
        );
        run(enter_text).expect("attached again");
        assert_eq!(phone.status().attaches, 2);

        fake.stall_runner(&info.id);
        let stopping = Instant::now();
        phone.signal_stop();
        assert!(phone.join(stopping + crate::registry::STOP_WAIT), "stopped");
        assert!(stopping.elapsed() < crate::registry::STOP_WAIT);
        assert!(
            fake.runners_on(&info.id).is_empty(),
            "the stop ended the stalled runner"
        );
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
