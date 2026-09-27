//! Runs the mirror's scripted [`FakeTarget`] in real time.
//!
//! The fake target keeps its own clock, which moves only when told to. Here
//! every call first advances it to the system clock, so the mirror's waits,
//! polls and settle windows take real time, as they do on a device. Input
//! goes to the fake target, which records it, and to a [`Device`], whose new
//! screen the target then shows along with a frame and, on Android, a
//! window event, the way an agent reports a change.

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

/// How often the spinning Anim tab produces a frame: 60 fps.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// The fake device's app window, which window events name.
const APP_WINDOW: u32 = 1;

struct Shared {
    target: FakeTarget,
    device: Mutex<Device>,
    animation: Mutex<Animation>,
}

#[derive(Default)]
struct Animation {
    last: Option<Instant>,
    frames: u64,
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

    /// A new frame every frame interval while the spinner turns.
    fn animate(&self, now: Instant) {
        if !lock(&self.device).animating() {
            return;
        }
        let mut animation = lock(&self.animation);
        if animation
            .last
            .is_some_and(|last| now < last + FRAME_INTERVAL)
        {
            return;
        }
        animation.last = Some(now);
        animation.frames += 1;
        let hash = 0xa11a_0000_0000_0000 ^ animation.frames;
        self.target.after(Duration::ZERO, Step::Frame(hash));
    }

    /// Shows the device's screen from now on: the tree, a frame with its
    /// hash, and on Android a window event.
    fn show(&self, device: &Device) {
        let read = read(&device.windows(), device);
        let hash = hash(&read);
        self.target.after(Duration::ZERO, Step::Show(read));
        if device.kit.platform == Platform::Android {
            self.target.after(
                Duration::ZERO,
                Step::Event(EventPayload::Window(WindowChange {
                    change: WindowChangeKind::STATE,
                    window: APP_WINDOW,
                })),
            );
        }
        self.target.after(Duration::ZERO, Step::Frame(hash));
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

/// A fake device of `platform`, attached: it shows its launcher.
pub(crate) fn attach(platform: Platform) -> Attachment {
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
        animation: Mutex::new(Animation::default()),
    });
    shared.show(&lock(&shared.device));
    let part = || Part {
        shared: Arc::clone(&shared),
        target: target.clone(),
    };
    Attachment {
        backend: Backend {
            tree: Box::new(part()),
            events: Box::new(part()),
            frames: Some(Box::new(part())),
            input: Box::new(part()),
            clock: Box::new(part()),
        },
        apps: Box::new(part()),
        display: Size {
            width: kit.width(),
            height: kit.height(),
        },
        config: MirrorConfig::default(),
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
            self.shared.show(&device);
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
        let mut device = lock(&self.shared.device);
        device.launch(app, launch)?;
        self.shared.show(&device);
        Ok(())
    }

    fn terminate(&mut self, app: &str) -> Result<(), DriverError> {
        let mut device = lock(&self.shared.device);
        device.terminate(app)?;
        self.shared.show(&device);
        Ok(())
    }
}
