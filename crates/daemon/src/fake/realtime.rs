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
use crate::driver::{AppControl, Attachment, DriverError, FixedScreen, Launch, Platform, Size};

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
    /// When the frame stream starts, when it starts after the attach.
    frames_from: Option<Instant>,
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
        now
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
        self.show_after(device, change, self.latency);
    }

    /// Shows the device's screen after `delay`.
    fn show_after(&self, device: &Device, change: Change, delay: Duration) {
        let read = read(&device.windows(), device);
        let hash = hash(&read);
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
/// frame source when `frames` is set, whose stream starts `frame_delay`
/// after the attach, and shows each change `latency` after the input or
/// launch that caused it.
pub(crate) fn attach(
    platform: Platform,
    frames: bool,
    latency: Duration,
    frame_delay: Duration,
) -> Attachment {
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
        frames_from: (!frame_delay.is_zero()).then(|| target.clock().now() + frame_delay),
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
        config: MirrorConfig::default(),
        capture: None,
        report: None,
        input_notes: None,
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
        if launch.relaunch && device.runs(app) {
            // The stopped process leaves the screen at once; the new one
            // shows its own after the latency, as a relaunched app does.
            device.terminate(app)?;
            self.shared
                .show_after(&device, Change::Screen, Duration::ZERO);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Task 8.13: a frame stream that starts after the attach, as an
    /// emulator's does once its subscription is up, delivers nothing before
    /// it starts and then the screen shown when it starts.
    #[test]
    fn a_late_frame_stream_delivers_nothing_before_it_starts_then_the_screen_shown() {
        let attached = Instant::now();
        let mut late = attach(
            Platform::Android,
            true,
            Duration::ZERO,
            Duration::from_millis(150),
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
        let mut prompt = attach(Platform::Android, true, Duration::ZERO, Duration::ZERO);
        let at_once = frames_within(&mut prompt, Duration::from_millis(50));
        assert_eq!(at_once.len(), 1, "{at_once:?}");
        assert_eq!(at_once[0].hash, first[0].hash, "the same launcher");
    }
}
