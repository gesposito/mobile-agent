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

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mirror::MirrorConfig;
use mirror::backend::{
    Backend, BackendError, Clock, EncodeRequest, EventStream, FrameMeta, FrameSource, Input,
    InputAck, InputSink, ReadRequest, StreamItem, Timed, TreeRead, TreeSource,
};
use mirror::testing::{FakeTarget, Step};
use protocol::event::{EventPayload, WindowChange, WindowChangeKind};
use protocol::message::Rotation;

use super::app::Device;
use super::tree::{self, Win};
use crate::driver::{AppControl, Attachment, DriverError, Launch, Platform, Size};

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

struct Shared {
    target: FakeTarget,
    device: Mutex<Device>,
    visual: Mutex<Visual>,
    /// Whether the target produces frames; Android devices do not (D7).
    frames: bool,
    /// How long a change takes to show, as a slow app or device takes.
    latency: Duration,
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
        self.animate(now);
        now
    }

    /// The next frame, once a frame interval has passed: a transition's,
    /// ending with the screen's final frame, or the turning spinner's.
    fn animate(&self, now: Instant) {
        if !self.frames {
            return;
        }
        let spinning = lock(&self.device).animating();
        let mut visual = lock(&self.visual);
        if visual.last.is_some_and(|last| now < last + FRAME_INTERVAL) {
            return;
        }
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
        visual.last = Some(now);
        self.target.after(Duration::ZERO, Step::Frame(hash));
    }

    /// Shows the device's screen after the latency: the tree, on Android a
    /// window event, and a frame, the first of a transition when `change`
    /// moved to another screen.
    fn show(&self, device: &Device, change: Change) {
        let read = read(&device.windows(), device);
        let hash = hash(&read);
        let delay = self.latency;
        self.target.after(delay, Step::Show(read));
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
        let first = match change {
            Change::Content => {
                visual.transition = None;
                hash
            }
            Change::Screen => {
                let from = self.target.clock().now() + delay;
                visual.transition = Some(Transition {
                    from,
                    until: from + TRANSITION,
                });
                visual.last = Some(from);
                visual.count += 1;
                mix(hash, visual.count)
            }
        };
        self.target.after(delay, Step::Frame(first));
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

/// A fake device of `platform`, attached: it shows its launcher. It has a
/// frame source when `frames` is set, and shows each change `latency`
/// after the input or launch that caused it.
pub(crate) fn attach(platform: Platform, frames: bool, latency: Duration) -> Attachment {
    let device = Device::new(platform);
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
        display: Size {
            width: kit.width(),
            height: kit.height(),
        },
        config: MirrorConfig::default(),
        capture: None,
        report: None,
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
        self.target.latest()
    }

    fn encode(&mut self, request: &EncodeRequest) -> Result<Vec<u8>, BackendError> {
        self.target.encode(request)
    }
}

impl InputSink for Part {
    fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError> {
        self.shared.sync();
        let mut ack = self.target.perform(input)?;
        let mut device = lock(&self.shared.device);
        let before = device.screen();
        let changed = match input {
            Input::Tap { at, .. } => {
                let hit = tree::hit(&device.windows(), at.x, at.y);
                device.tap(hit.as_deref())
            }
            Input::Swipe { from, to, .. } => device.swipe((from.x, from.y), (to.x, to.y)),
            Input::Key { code, meta } => device.key(*code, *meta),
            Input::Text { text, node, method } => {
                let field = node.map(|n| tree::identifier_of(&device.windows(), n.window, n.node));
                let taken = match field {
                    Some(None) => None,
                    Some(Some(id)) => device.text(Some(&id), text, *method),
                    None => device.text(None, text, *method),
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
        device.launch(app, launch)?;
        self.shared.show(&device, Change::Screen);
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
        Ok(())
    }
}
