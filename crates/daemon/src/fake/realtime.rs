//! Runs the mirror's scripted [`FakeTarget`] in real time.
//!
//! The fake target keeps its own clock, which moves only when told to. Here
//! every call first advances it to the system clock, so the mirror's waits,
//! polls and settle windows take real time, as they do on a device. Input
//! goes to the fake target, which records it, and to a [`Device`], whose new
//! screen the target then shows, after the target's latency, with a window
//! event on Android and, on targets with frames, a frame. A move to another
//! screen animates: its frames change every 16 ms for 96 ms before the
//! final one, as a transition does, while the tree changes at its start.
//! The frame stream can start some time after the attach, as an emulator's
//! does once its subscription is up: frames captured before are never
//! delivered, and the first one delivered shows the screen as it is then.
//! A fixture app whose process starts can show a launch splash first,
//! input can wait for an input session that activates some time after it
//! starts, as a Simulator's does, and detaching can take time, as stopping
//! the Android agent does: the last part dropped waits it out. The fixture's
//! list can glide on after a stroke whose finger lifts moving, as far as the
//! platform's fling carries it, the finger's speed its release velocity.
//! Text whose method the target may choose goes in whole, unless the typing
//! mode types it key by key as a Simulator does. A keyboard can come up some
//! time after a field took the focus, shown then with a frame, and the
//! placeholder app's layout moves its submit button above the keyboard some
//! time after that, in no frame.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mirror::backend::{
    Backend, BackendError, Clock, EncodeRequest, EventStream, FrameMeta, FrameSource, Input,
    InputAck, InputSink, ReadRequest, Release, StreamItem, Timed, TreeRead, TreeSource,
};
use mirror::testing::{FakeTarget, Step};
use mirror::{MirrorConfig, SlowFrames};
use protocol::event::{EventPayload, WindowChange, WindowChangeKind};
use protocol::message::{Rotation, TextMethod};

use super::app::{self, Device, Entry};
use super::tree::{self, Win};
use super::{Timing, Typing};
use crate::devices::ios::{self as device, Facts};
use crate::driver::{
    AppControl, Attachment, DriverError, FixedScreen, InputNotes, InputSession, Launch, Platform,
    Report, Size, TargetReport,
};

/// How often the event stream looks for items while it waits.
const POLL: Duration = Duration::from_millis(2);

/// How often the spinning Anim tab and a transition produce a frame: about
/// 60 fps.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// How long a move to another screen animates.
const TRANSITION: Duration = Duration::from_millis(96);

/// The base of the spinner's frame hashes.
const SPINNER: u64 = 0xa11a_0000_0000_0000;

/// The fake device's app window, which window events name.
const APP_WINDOW: u32 = 1;

/// The longest text that typing modes which type keys type key by key, as
/// the Simulator driver's `TextMethod::Auto` does; longer text is pasted.
const KEYBOARD_LIMIT: usize = 32;

/// How long after the keyboard came up the placeholder app's layout moves
/// its submit button above it, as React Native moves a view out of the
/// keyboard's way once the keyboard has shown. The move shows in no frame
/// (see [`Change::Layout`]).
const LIFT: Duration = Duration::from_millis(50);

/// How a fake target's frames come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Frames {
    /// No frame source, as on an Android device (design D7).
    None,
    /// A stream, as an emulator's controller or a Simulator's display.
    Stream,
    /// Screenshots polled at a physical iPhone's rate (design D23), or,
    /// unless `subscribed`, a phone whose screenshots could not be
    /// subscribed to, which has no frame source.
    Screenshots { subscribed: bool },
}

impl Frames {
    /// Whether the target has a frame source.
    fn any(self) -> bool {
        matches!(
            self,
            Frames::Stream | Frames::Screenshots { subscribed: true }
        )
    }
}

/// How text entered as `method` asks reaches a field under `typing`: keys
/// add to what the field holds; text whose method the target may choose is
/// typed key by key in its place when `typing` types keys and it is short
/// enough, else it goes in whole, as direct text does.
fn entry(method: TextMethod, text: &str, typing: Typing) -> Entry {
    let typed =
        matches!(typing, Typing::Keys | Typing::Reorder) && text.chars().count() <= KEYBOARD_LIMIT;
    if method == TextMethod::KEYS {
        Entry::Keys { replace: false }
    } else if method == TextMethod::AUTO && typed {
        Entry::Keys { replace: true }
    } else {
        Entry::Whole
    }
}

struct Shared {
    target: FakeTarget,
    device: Mutex<Device>,
    visual: Mutex<Visual>,
    /// Whether the target produces frames; Android devices do not (D7).
    frames: bool,
    /// How long a change takes to show, as a slow app or device takes.
    latency: Duration,
    /// When the frame stream starts, when it starts after the attach.
    frames_from: Option<Instant>,
    /// How long a fixture app shows its launch splash when its process
    /// starts.
    splash: Duration,
    /// How long the input session takes to activate once started.
    activation: Duration,
    session: Mutex<Session>,
    /// How long detaching takes.
    stop: Duration,
    /// Whether a stroke that lifts moving glides the list on.
    fling: bool,
    /// How the fields take text whose method the target may choose.
    typing: Typing,
    /// When the keyboard that is coming up comes up.
    keyboard_due: Mutex<Option<Instant>>,
    /// When the placeholder app's layout moves its submit button above the
    /// keyboard, once the keyboard is up.
    lift_due: Mutex<Option<Instant>>,
    /// Frames the event stream delivered.
    delivered: AtomicU64,
    /// For a fake iPhone, its screenshots: when the captures started and
    /// their interval, frames showing only at captures; `None` for a
    /// stream. Its frames off, the screenshots could not be subscribed to.
    captures: Option<Captures>,
}

/// A fake iPhone's screenshots.
#[derive(Clone, Copy)]
struct Captures {
    from: Instant,
    every: Duration,
    subscribed: bool,
}

impl Captures {
    /// The capture at or after `at`.
    fn at_or_after(self, at: Instant) -> Instant {
        let since = at.saturating_duration_since(self.from).as_nanos();
        let every = self.every.as_nanos().max(1);
        let n = u32::try_from(since.div_ceil(every)).unwrap_or(u32::MAX);
        self.from + self.every * n
    }

    /// The capture at or before `at`.
    fn at_or_before(self, at: Instant) -> Instant {
        let since = at.saturating_duration_since(self.from).as_nanos();
        let every = self.every.as_nanos().max(1);
        let n = u32::try_from(since / every).unwrap_or(u32::MAX);
        self.from + self.every * n
    }
}

impl Drop for Shared {
    /// The target is detached once every part of it is dropped, which takes
    /// the stop time on the thread that drops the last one, as the Android
    /// agent's stop takes its time on the thread that detaches the target.
    fn drop(&mut self) {
        if !self.stop.is_zero() {
            std::thread::sleep(self.stop);
        }
    }
}

/// The input session: started by the first input, or ahead of it.
#[derive(Default)]
struct Session {
    started: Option<Instant>,
    /// Whether it started ahead of any input, so the next input is its
    /// first.
    ahead: bool,
    /// How long the last input waited for the session to activate: some
    /// time, none at all for the first input of a session started ahead.
    waited: Option<Duration>,
}

/// What the frames show.
#[derive(Default)]
struct Visual {
    /// The hash of the screen shown last, as its final frame has it.
    screen: u64,
    /// The transition running, if any.
    transition: Option<Transition>,
    /// When the last frame was produced.
    last: Option<Instant>,
    /// Frames produced so far, to give each its own hash.
    count: u64,
    /// Whether a frame stream that starts after the attach has delivered
    /// its first frame.
    started: bool,
}

#[derive(Clone, Copy)]
struct Transition {
    from: Instant,
    until: Instant,
}

/// How a screen changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    /// Within the screen: text, a toggle, a scroll position.
    Content,
    /// To another screen, which animates.
    Screen,
    /// Within the screen, a view the layout moved, which the target shows
    /// without an event and in no frame the mirror takes in before its next
    /// read: the React Native smoke test's submit button, which the keyboard
    /// moved up while the replica kept its old bounds, the frame-driven
    /// re-read not yet due (design D32).
    Layout,
}

/// A hash for the `n`th frame derived from `base`, never `base` itself.
fn mix(base: u64, n: u64) -> u64 {
    base ^ n.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Advances the fake clock to the system clock and returns the time.
    fn sync(&self) -> Instant {
        let fake = self.target.clock().now();
        let real = Instant::now();
        if real > fake {
            self.target.advance(real - fake);
        }
        let now = real.max(fake);
        self.start_frames(now);
        self.animate(now);
        self.raise(now);
        self.lift(now);
        now
    }

    /// Brings up the keyboard that was coming once it is due, showing it as
    /// any change within the screen shows, with a frame.
    fn raise(&self, now: Instant) {
        let due = *lock(&self.keyboard_due);
        if due.is_none_or(|due| now < due) {
            return;
        }
        *lock(&self.keyboard_due) = None;
        let mut device = lock(&self.device);
        if device.raise_keyboard() {
            self.show_after(&device, Change::Content, Duration::ZERO);
            self.follow(&device, Duration::ZERO);
        }
    }

    /// Moves the placeholder app's submit button above the keyboard once
    /// the move is due, showing it as a layout change.
    fn lift(&self, now: Instant) {
        let due = *lock(&self.lift_due);
        if due.is_none_or(|due| now < due) {
            return;
        }
        *lock(&self.lift_due) = None;
        let mut device = lock(&self.device);
        if device.lift() {
            self.show_after(&device, Change::Layout, Duration::ZERO);
        }
    }

    /// Schedules what follows a change shown `delay` from now: a keyboard
    /// that is coming up comes up as long after the focus as its slide says,
    /// and the placeholder app's layout moves its submit button [`LIFT`]
    /// after the keyboard came up.
    fn follow(&self, device: &Device, delay: Duration) {
        let now = self.target.clock().now();
        let mut keyboard = lock(&self.keyboard_due);
        *keyboard = device
            .keyboard_coming()
            .and_then(|after| keyboard.or(Some(now + delay + after)));
        let mut lift = lock(&self.lift_due);
        *lift = if device.lift_pending() {
            lift.or(Some(now + delay + LIFT))
        } else {
            None
        };
    }

    /// Delivers the first frame of a frame stream that starts after the
    /// attach, once it starts: the screen shown then.
    fn start_frames(&self, now: Instant) {
        let Some(from) = self.frames_from else {
            return;
        };
        if !self.frames || now < from {
            return;
        }
        let mut visual = lock(&self.visual);
        if visual.started {
            return;
        }
        visual.started = true;
        drop(visual);
        let shown = hash(&self.target.screen());
        self.target.after(Duration::ZERO, Step::Frame(shown));
    }

    /// Whether `item` is a frame captured before the frame stream started,
    /// which the stream never delivers.
    fn withheld(&self, item: &Timed<StreamItem>) -> bool {
        match (&item.item, self.frames_from) {
            (StreamItem::Frame(meta), Some(from)) => meta.at < from,
            _ => false,
        }
    }

    /// The next frame, once a frame interval has passed, or for a fake
    /// iPhone at its next capture: a transition's, ending with the screen's
    /// final frame, or the turning spinner's.
    fn animate(&self, now: Instant) {
        if !self.frames {
            return;
        }
        let spinning = lock(&self.device).animating();
        let mut visual = lock(&self.visual);
        let shown = match self.captures {
            Some(captures) => {
                let capture = captures.at_or_before(now);
                if visual.last.is_some_and(|last| capture <= last) {
                    return;
                }
                capture
            }
            None if visual.last.is_some_and(|last| now < last + FRAME_INTERVAL) => return,
            None => now,
        };
        let hash = match visual.transition {
            Some(transition) if now < transition.from => return,
            Some(transition) if now >= transition.until => {
                visual.transition = None;
                visual.screen
            }
            Some(_) => {
                visual.count += 1;
                mix(visual.screen, visual.count)
            }
            None if spinning => {
                visual.count += 1;
                mix(SPINNER, visual.count)
            }
            None => return,
        };
        visual.last = Some(shown);
        self.target.after(Duration::ZERO, Step::Frame(hash));
    }

    /// How long from now until the frame that shows a change `delay` from
    /// now: then on a stream, at the next capture on a fake iPhone.
    fn frame_delay(&self, delay: Duration) -> Duration {
        match self.captures {
            Some(captures) => {
                let now = self.target.clock().now();
                captures
                    .at_or_after(now + delay)
                    .saturating_duration_since(now)
            }
            None => delay,
        }
    }

    /// Shows the device's screen after the latency: the tree, on Android a
    /// window event, and a frame, the first of a transition when `change`
    /// moved to another screen.
    fn show(&self, device: &Device, change: Change) {
        self.show_after(device, change, self.latency);
    }

    /// Shows the device's screen after `delay`.
    fn show_after(&self, device: &Device, change: Change, delay: Duration) {
        self.show_windows(&device.windows(), device, change, delay);
    }

    /// Shows `windows` on the device after `delay`.
    fn show_windows(&self, windows: &[Win], device: &Device, change: Change, delay: Duration) {
        let read = read(windows, device);
        let hash = hash(&read);
        self.target.after(delay, Step::Show(read));
        if change == Change::Layout {
            // The final frame shows it, whenever one comes next.
            lock(&self.visual).screen = hash;
            return;
        }
        if device.kit.platform == Platform::Android {
            self.target.after(
                delay,
                Step::Event(EventPayload::Window(WindowChange {
                    change: WindowChangeKind::STATE,
                    window: APP_WINDOW,
                })),
            );
        }
        if !self.frames {
            return;
        }
        let mut visual = lock(&self.visual);
        visual.screen = hash;
        let shown = self.frame_delay(delay);
        let first = match change {
            Change::Content | Change::Layout => {
                visual.transition = None;
                hash
            }
            Change::Screen => {
                let from = self.target.clock().now() + delay;
                visual.transition = Some(Transition {
                    from,
                    until: from + TRANSITION,
                });
                visual.last = Some(self.target.clock().now() + shown);
                visual.count += 1;
                mix(hash, visual.count)
            }
        };
        if self.captures.is_some() {
            visual.last = Some(self.target.clock().now() + shown);
        }
        self.target.after(shown, Step::Frame(first));
    }

    /// Starts the input session if it has not started; says how long input
    /// arriving now waits for it to activate, `None` when not at all.
    fn enter_session(&self) -> Option<Duration> {
        if self.activation.is_zero() {
            return None;
        }
        let now = Instant::now();
        let mut session = lock(&self.session);
        let started = *session.started.get_or_insert(now);
        let first = std::mem::take(&mut session.ahead);
        let wait = (started + self.activation)
            .checked_duration_since(now)
            .filter(|wait| !wait.is_zero());
        session.waited = wait.or(first.then_some(Duration::ZERO));
        wait
    }
}

fn read(windows: &[Win], device: &Device) -> TreeRead {
    tree::tree_read(windows, device.kit.scale(), device.kit.backend())
}

fn hash(read: &TreeRead) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{read:?}").hash(&mut hasher);
    hasher.finish()
}

/// What the fake input session tells the daemon.
struct FakeSession(Arc<Shared>);

impl InputSession for FakeSession {
    fn start(&mut self) -> Result<(), DriverError> {
        let mut session = lock(&self.0.session);
        if session.started.is_none() {
            session.started = Some(Instant::now());
            session.ahead = true;
        }
        Ok(())
    }

    fn active(&mut self) -> bool {
        let started = lock(&self.0.session).started;
        started.is_some_and(|started| Instant::now() >= started + self.0.activation)
    }
}

impl InputNotes for FakeSession {
    fn activation_wait(&self) -> Option<Duration> {
        lock(&self.0.session).waited
    }
}

/// What `status` shows about a fake iPhone: what the physical iOS driver
/// reports of a phone whose runner, the one `agent-ios` builds, said hello,
/// its input going through `dtuhidd` and its frames from screenshots at the
/// device's rate.
struct DeviceReport(Arc<Shared>);

impl Report for DeviceReport {
    fn report(&self) -> TargetReport {
        let runner = ios::RunnerStatus {
            state: ios::RunnerState::Ready,
            agent: Some(agent_ios::AGENT.to_owned()),
            capabilities: protocol::Capabilities::TREE
                | protocol::Capabilities::INPUT
                | protocol::Capabilities::SCREENSHOT
                | protocol::Capabilities::TEXT_SET,
            ..ios::RunnerStatus::starting("the hello")
        };
        let status = ios::TargetStatus {
            runner: Some(runner),
            ..ios::TargetStatus::default()
        };
        let subscribed = self.0.captures.is_some_and(|captures| captures.subscribed);
        let facts = Facts {
            ios_version: Some(ios::VALIDATED_IOS.to_owned()),
            frames: self.0.delivered.load(Ordering::Relaxed),
            capture_rate: subscribed.then_some(f64::from(device::CAPTURES_A_SECOND)),
            frame_error: (!subscribed).then(|| "the fake iPhone's screenshots are off".to_owned()),
            ..Facts::default()
        };
        device::status_report(&status, &facts)
    }
}

/// A fake device of `platform`, attached: it shows its launcher. Its frames
/// come as `frames` says, and a fake whose frames are screenshots reports
/// itself as the physical iOS driver reports an iPhone; `timing` says when
/// its frames start, how late each change shows after the input or launch
/// that caused it, how long a launch splash lasts, when its input session
/// activates and how long detaching it takes. With `fling`, a stroke whose
/// finger lifts moving glides the list on; `typing` says how its fields take
/// text whose method the target may choose. With `keyboard`, its keyboard
/// stays up over every screen of the fixture apps.
pub(crate) fn attach(
    platform: Platform,
    frames: Frames,
    timing: Timing,
    fling: bool,
    typing: Typing,
    keyboard: bool,
) -> Attachment {
    let subscribed = match frames {
        Frames::Screenshots { subscribed } => Some(subscribed),
        Frames::None | Frames::Stream => None,
    };
    let frames = frames.any();
    let Timing {
        latency,
        frame_delay,
        splash,
        activation,
        stop,
        slide,
    } = timing;
    let device = Device::new(platform)
        .with_keyboard(keyboard)
        .with_slide(slide);
    let kit = device.kit;
    let pixels = |points: f64| (points * kit.scale()).round() as u32;
    let target = FakeTarget::new(read(&device.windows(), &device));
    // The Android agent and the Simulator's reader return every window
    // with each read, so a window that appears, such as SpringBoard's
    // prompt, arrives with the next read.
    target.whole_reads(true);
    target.frame_size(pixels(kit.width()), pixels(kit.height()), Rotation::R0);
    let shared = Arc::new(Shared {
        target: target.clone(),
        device: Mutex::new(device),
        visual: Mutex::new(Visual::default()),
        frames,
        latency,
        frames_from: (!frame_delay.is_zero()).then(|| target.clock().now() + frame_delay),
        splash,
        activation,
        session: Mutex::new(Session::default()),
        stop,
        fling,
        typing,
        keyboard_due: Mutex::new(None),
        lift_due: Mutex::new(None),
        delivered: AtomicU64::new(0),
        captures: subscribed.map(|subscribed| Captures {
            from: target.clock().now(),
            every: device::CAPTURE_INTERVAL,
            subscribed,
        }),
    });
    shared.show(&lock(&shared.device), Change::Content);
    let part = || Part {
        shared: Arc::clone(&shared),
        target: target.clone(),
    };
    Attachment {
        backend: Backend {
            tree: Box::new(part()),
            events: Box::new(part()),
            frames: frames.then(|| Box::new(part()) as Box<dyn FrameSource>),
            input: Box::new(part()),
            clock: Box::new(part()),
        },
        apps: Box::new(part()),
        display: Box::new(FixedScreen(Size {
            width: kit.width(),
            height: kit.height(),
        })),
        // A fake iPhone's frames come as slowly as a physical one's.
        config: MirrorConfig {
            slow_frames: subscribed.map(|_| SlowFrames::every(device::CAPTURE_INTERVAL)),
            ..MirrorConfig::default()
        },
        capture: None,
        report: subscribed.map(|_| Arc::new(DeviceReport(Arc::clone(&shared))) as Arc<dyn Report>),
        input_notes: (!activation.is_zero())
            .then(|| Arc::new(FakeSession(Arc::clone(&shared))) as Arc<dyn InputNotes>),
        input_session: (!activation.is_zero())
            .then(|| Box::new(FakeSession(Arc::clone(&shared))) as Box<dyn InputSession>),
    }
}

/// One of the backend's parts; all share the target and the device.
struct Part {
    shared: Arc<Shared>,
    target: FakeTarget,
}

impl TreeSource for Part {
    fn read(&mut self, request: &ReadRequest) -> Result<TreeRead, BackendError> {
        self.shared.sync();
        self.target.read(request)
    }
}

impl EventStream for Part {
    fn next(&mut self, deadline: Instant) -> Option<Timed<StreamItem>> {
        loop {
            let now = self.shared.sync();
            if let Some(item) = self.target.next(now) {
                if self.shared.withheld(&item) {
                    continue;
                }
                if matches!(item.item, StreamItem::Frame(_)) {
                    self.shared.delivered.fetch_add(1, Ordering::Relaxed);
                }
                return Some(item);
            }
            if now >= deadline {
                return None;
            }
            std::thread::sleep((deadline - now).min(POLL));
        }
    }
}

impl FrameSource for Part {
    fn latest(&self) -> Option<FrameMeta> {
        self.shared.sync();
        let from = self.shared.frames_from;
        self.target
            .latest()
            .filter(|meta| from.is_none_or(|from| meta.at >= from))
    }

    fn encode(&mut self, request: &EncodeRequest) -> Result<Vec<u8>, BackendError> {
        self.target.encode(request)
    }
}

impl InputSink for Part {
    fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError> {
        // Input waits for the session to activate, as held input does.
        if let Some(wait) = self.shared.enter_session() {
            std::thread::sleep(wait);
        }
        self.shared.sync();
        let mut ack = self.target.perform(input)?;
        let mut device = lock(&self.shared.device);
        let before = device.screen();
        let changed = match input {
            Input::Tap { at, .. } => {
                let hit = tree::hit(&device.windows(), at.x, at.y);
                device.tap(hit.as_deref(), (at.x, at.y))
            }
            Input::Swipe {
                from,
                to,
                duration,
                release,
            } => {
                // The list scrolls up and down, so the finger's speed along
                // that axis is its release velocity.
                let glide = if self.shared.fling && *release == Release::Moving {
                    let speed = (from.y - to.y).abs() / duration.as_secs_f64();
                    app::glide(device.kit.platform, speed)
                } else {
                    0.0
                };
                device.swipe((from.x, from.y), (to.x, to.y), glide)
            }
            Input::Key { code, meta } => device.key(*code, *meta),
            Input::Text { text, node, method } => {
                let typing = self.shared.typing;
                let entry = entry(*method, text, typing);
                ack.method = Some(match entry {
                    Entry::Keys { .. } => TextMethod::KEYS,
                    Entry::Whole => TextMethod::DIRECT,
                });
                let field = node.map(|n| tree::identifier_of(&device.windows(), n.window, n.node));
                let taken = match field {
                    Some(None) => None,
                    Some(Some(id)) => device.text(Some(&id), text, entry, typing),
                    None => device.text(None, text, entry, typing),
                };
                ack.accepted = taken.is_some();
                taken.unwrap_or(false)
            }
        };
        if changed {
            let change = if device.screen() == before {
                Change::Content
            } else {
                Change::Screen
            };
            self.shared.show(&device, change);
        }
        self.shared.follow(&device, self.shared.latency);
        Ok(ack)
    }
}

impl Clock for Part {
    fn now(&self) -> Instant {
        self.shared.sync()
    }
}

impl AppControl for Part {
    fn launch(&mut self, app: &str, launch: &Launch) -> Result<(), DriverError> {
        self.shared.sync();
        let mut device = lock(&self.shared.device);
        let starts = launch.relaunch || !device.runs(app);
        if launch.relaunch && device.runs(app) {
            // The stopped process leaves the screen at once; the new one
            // shows its own after the latency, as a relaunched app does.
            device.terminate(app)?;
            self.shared
                .show_after(&device, Change::Screen, Duration::ZERO);
        }
        device.launch(app, launch)?;
        let splash = self.shared.splash;
        if starts && !splash.is_zero() && device.is_fixture(app) {
            // A process that starts shows its launch splash first.
            let latency = self.shared.latency;
            self.shared.show_windows(
                &device.splash_windows(app),
                &device,
                Change::Screen,
                latency,
            );
            self.shared
                .show_after(&device, Change::Screen, latency + splash);
        } else {
            self.shared.show(&device, Change::Screen);
        }
        self.shared.follow(&device, self.shared.latency);
        Ok(())
    }

    fn terminate(&mut self, app: &str) -> Result<(), DriverError> {
        self.shared.sync();
        let mut device = lock(&self.shared.device);
        let before = device.screen();
        device.terminate(app)?;
        let change = if device.screen() == before {
            Change::Content
        } else {
            Change::Screen
        };
        self.shared.show(&device, change);
        self.shared.follow(&device, self.shared.latency);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task 10.23: text whose method the target may choose is typed key by
    /// key in a field's place under a typing mode that types keys, up to the
    /// Simulator's 32 characters, and goes in whole otherwise; direct text
    /// always goes in whole and keys always add to the field.
    #[test]
    fn text_is_typed_as_the_simulator_types_it_only_under_a_typing_mode_that_types_keys() {
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        let fill = Entry::Keys { replace: true };
        for typing in [Typing::Keys, Typing::Reorder] {
            assert_eq!(entry(TextMethod::AUTO, "nobel", typing), fill);
            assert_eq!(entry(TextMethod::AUTO, &long[..32], typing), fill);
            assert_eq!(entry(TextMethod::AUTO, long, typing), Entry::Whole);
        }
        for typing in [Typing::Whole, Typing::Max(4)] {
            assert_eq!(entry(TextMethod::AUTO, "nobel", typing), Entry::Whole);
        }
        for typing in [Typing::Whole, Typing::Keys, Typing::Reorder] {
            assert_eq!(entry(TextMethod::DIRECT, "nobel", typing), Entry::Whole);
            assert_eq!(
                entry(TextMethod::KEYS, "nobel", typing),
                Entry::Keys { replace: false }
            );
        }
    }

    /// The frames an attachment's event stream delivers within `window`.
    fn frames_within(attachment: &mut Attachment, window: Duration) -> Vec<FrameMeta> {
        let deadline = Instant::now() + window;
        let mut frames = Vec::new();
        while let Some(item) = attachment.backend.events.next(deadline) {
            if let StreamItem::Frame(meta) = item.item {
                frames.push(meta);
            }
        }
        frames
    }

    /// Task 8.15: a fixture app whose process starts shows its launch
    /// splash, one image, for the splash's length before its first screen.
    #[test]
    fn a_launch_shows_the_splash_before_the_first_screen() {
        let timing = Timing {
            splash: Duration::from_millis(150),
            ..Timing::default()
        };
        let mut attachment = attach(
            Platform::Ios,
            Frames::None,
            timing,
            false,
            Typing::Whole,
            false,
        );
        let read = |attachment: &mut Attachment| {
            let request = ReadRequest {
                windows: None,
                reason: mirror::backend::ReadReason::Attach,
            };
            format!("{:?}", attachment.backend.tree.read(&request).unwrap())
        };
        attachment
            .apps
            .launch(super::super::app::FIXTURE, &Launch::default())
            .unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let splash = read(&mut attachment);
        assert!(splash.contains("SplashScreenLegacy"), "{splash}");
        assert!(!splash.contains("login-submit"), "{splash}");
        std::thread::sleep(Duration::from_millis(150));
        let screen = read(&mut attachment);
        assert!(screen.contains("login-submit"), "{screen}");
        assert!(!screen.contains("SplashScreenLegacy"), "{screen}");
    }

    /// Task 8.15: input waits for the input session to activate, which
    /// starts with the first input or ahead of it.
    #[test]
    fn input_waits_for_the_session_to_activate_unless_it_started_ahead() {
        let timing = Timing {
            activation: Duration::from_millis(150),
            ..Timing::default()
        };
        let tap = Input::Tap {
            at: mirror::Point::new(1.0, 1.0),
            hold: None,
        };
        let mut cold = attach(
            Platform::Ios,
            Frames::None,
            timing,
            false,
            Typing::Whole,
            false,
        );
        let started = Instant::now();
        cold.backend.input.perform(&tap).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(150));
        assert!(cold.input_notes.unwrap().activation_wait().is_some());

        let mut warm = attach(
            Platform::Ios,
            Frames::None,
            timing,
            false,
            Typing::Whole,
            false,
        );
        let session = warm.input_session.as_mut().unwrap();
        session.start().unwrap();
        assert!(!session.active());
        std::thread::sleep(Duration::from_millis(150));
        assert!(session.active());
        let started = Instant::now();
        warm.backend.input.perform(&tap).unwrap();
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(
            warm.input_notes.unwrap().activation_wait(),
            Some(Duration::ZERO),
            "the first input of the session, which waited no time"
        );
    }

    /// Physical-ios, task 5.2 (design D23): a fake iPhone's frames are
    /// screenshots polled at the device's rate, about 19 a second, so the
    /// Anim tab's spinner, which a Simulator's stream shows 60 times a
    /// second, shows in frames a capture interval apart; its attachment
    /// sets the mirror for slow frames, and its windows send no change
    /// events.
    #[test]
    fn a_fake_iphones_frames_come_at_the_devices_rate() {
        let anim = Launch {
            relaunch: false,
            env: vec![("FIXTURE_INITIAL_TAB".into(), "anim".into())],
        };
        let polled = Frames::Screenshots { subscribed: true };
        let mut phone = attach(
            Platform::Ios,
            polled,
            Timing::default(),
            false,
            Typing::Whole,
            false,
        );
        assert_eq!(
            phone.config.slow_frames,
            Some(SlowFrames::every(device::CAPTURE_INTERVAL))
        );
        phone.apps.launch(app::FIXTURE, &anim).unwrap();
        let frames = frames_within(&mut phone, Duration::from_millis(600));
        assert!(
            (5..=13).contains(&frames.len()),
            "{} frames in 600 ms: {frames:?}",
            frames.len()
        );
        for pair in frames.windows(2) {
            let apart = pair[1].at - pair[0].at;
            assert!(
                apart + Duration::from_millis(5) >= device::CAPTURE_INTERVAL,
                "frames {apart:?} apart: {frames:?}"
            );
        }
        let request = ReadRequest {
            windows: None,
            reason: mirror::backend::ReadReason::Attach,
        };
        let read = phone.backend.tree.read(&request).unwrap();
        assert!(
            read.snapshot
                .windows
                .iter()
                .all(|window| window.emits_events == Some(false)),
            "{:?}",
            read.snapshot.windows
        );

        let mut simulator = attach(
            Platform::Ios,
            Frames::Stream,
            Timing::default(),
            false,
            Typing::Whole,
            false,
        );
        assert_eq!(simulator.config.slow_frames, None);
        simulator.apps.launch(app::FIXTURE, &anim).unwrap();
        let streamed = frames_within(&mut simulator, Duration::from_millis(600));
        assert!(streamed.len() > 20, "{} frames", streamed.len());
    }

    /// Task 8.13: a frame stream that starts after the attach, as an
    /// emulator's does once its subscription is up, delivers nothing before
    /// it starts and then the screen shown when it starts.
    #[test]
    fn a_late_frame_stream_delivers_nothing_before_it_starts_then_the_screen_shown() {
        let attached = Instant::now();
        let mut late = attach(
            Platform::Android,
            Frames::Stream,
            Timing {
                frame_delay: Duration::from_millis(150),
                ..Timing::default()
            },
            false,
            Typing::Whole,
            false,
        );
        assert!(frames_within(&mut late, Duration::from_millis(100)).is_empty());
        assert!(late.backend.frames.as_ref().unwrap().latest().is_none());
        let first = frames_within(&mut late, Duration::from_millis(150));
        assert_eq!(first.len(), 1, "{first:?}");
        assert!(first[0].at >= attached + Duration::from_millis(150));
        assert_eq!(
            late.backend.frames.as_ref().unwrap().latest(),
            Some(first[0])
        );

        // The same device without a delay shows its launcher's frame at once.
        let mut prompt = attach(
            Platform::Android,
            Frames::Stream,
            Timing::default(),
            false,
            Typing::Whole,
            false,
        );
        let at_once = frames_within(&mut prompt, Duration::from_millis(50));
        assert_eq!(at_once.len(), 1, "{at_once:?}");
        assert_eq!(at_once[0].hash, first[0].hash, "the same launcher");
    }
}
