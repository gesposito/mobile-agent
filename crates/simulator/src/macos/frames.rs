//! Frame subscriptions and screenshots on a device's main display (design D7,
//! spike S2).
//!
//! The present callback only counts the tick and wakes a worker thread. The
//! worker reads the cached surface, downscales it into its buffer and hashes
//! it; when the pixels changed it encodes the frame and queues it for the
//! consumer. Presents that arrive while the worker is busy coalesce into its
//! next read, and the throttle skips presents instead of reading late.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::NSUUID;

use crate::error::{Error, Result};
use crate::frames::{FrameQueue, FrameSource, QUEUE_CAPACITY};
use crate::macos::coresim::Device;
use crate::macos::display::{
    self, Callbacks, Display, ObjectBlock, Surface, TwoObjectBlock, VoidBlock,
};
use crate::macos::encode::Encoder;
use crate::model::{Frame, FrameOptions, FrameStats, ImageFormat, Screenshot, ScreenshotOptions};
use crate::pipeline::{self, FrameView, Throttle, fit_within, scale_bgra};

/// How long SimScreen gets to report the current surface after registration
/// before the subscription asks for it; S2 saw it in 1.8 ms.
const SURFACE_WAIT: Duration = Duration::from_secs(2);

/// Reads of a surface the render server rewrote meanwhile before a
/// screenshot takes the last one anyway.
const SCREENSHOT_ATTEMPTS: u32 = 3;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// CPU time of the calling thread.
fn thread_cpu() -> Duration {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `time` is a valid out pointer for this clock.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
    Duration::new(time.tv_sec as u64, time.tv_nsec as u32)
}

/// What the callbacks, the worker and the consumer share.
struct Shared {
    started: Instant,
    surface: Mutex<Option<Arc<Surface>>>,
    pending: Mutex<bool>,
    wake: Condvar,
    stop: AtomicBool,
    presents: AtomicU64,
    reads: AtomicU64,
    delivered: AtomicU64,
    torn: AtomicU64,
    callback_cpu_ns: AtomicU64,
    worker_cpu_ns: AtomicU64,
}

impl Shared {
    fn new() -> Shared {
        Shared {
            started: Instant::now(),
            surface: Mutex::new(None),
            pending: Mutex::new(false),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            presents: AtomicU64::new(0),
            reads: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            torn: AtomicU64::new(0),
            callback_cpu_ns: AtomicU64::new(0),
            worker_cpu_ns: AtomicU64::new(0),
        }
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn on_present(&self) {
        let start = thread_cpu();
        self.presents.fetch_add(1, Ordering::Relaxed);
        *lock(&self.pending) = true;
        self.wake.notify_one();
        let spent = thread_cpu().saturating_sub(start);
        self.callback_cpu_ns
            .fetch_add(spent.as_nanos() as u64, Ordering::Relaxed);
    }

    fn on_surface(&self, surface: Option<Surface>) {
        if let Some(surface) = surface {
            *lock(&self.surface) = Some(Arc::new(surface));
        }
    }

    fn surface(&self) -> Option<Arc<Surface>> {
        lock(&self.surface).clone()
    }

    /// Takes a present that is already pending.
    fn take_pending(&self) -> bool {
        std::mem::take(&mut *lock(&self.pending))
    }

    /// Takes the next present, waiting up to `timeout`, or until one comes or
    /// the subscription stops with `None`.
    fn wait_present(&self, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut pending = lock(&self.pending);
        loop {
            if self.stopping() {
                return false;
            }
            if *pending {
                *pending = false;
                return true;
            }
            let slice = match deadline {
                None => Duration::from_millis(50),
                Some(deadline) => match deadline.checked_duration_since(Instant::now()) {
                    Some(remaining) => remaining.min(Duration::from_millis(50)),
                    None => return false,
                },
            };
            pending = self
                .wake
                .wait_timeout(pending, slice)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Sleeps until `until`; false when the subscription stopped meanwhile.
    fn sleep_until(&self, until: Instant) -> bool {
        loop {
            if self.stopping() {
                return false;
            }
            let Some(remaining) = until.checked_duration_since(Instant::now()) else {
                return true;
            };
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }
}

/// Runs a callback body, keeping a panic from unwinding into libdispatch.
fn contained(body: impl FnOnce()) {
    let _ = panic::catch_unwind(AssertUnwindSafe(body));
}

/// The blocks registered with the display and the UUID they are registered
/// under.
struct Registration {
    uuid: Retained<NSUUID>,
    frame: RcBlock<dyn Fn()>,
    surfaces: RcBlock<dyn Fn(*mut AnyObject, *mut AnyObject)>,
    properties: RcBlock<dyn Fn(*mut AnyObject)>,
    legacy_surface: RcBlock<dyn Fn(*mut AnyObject)>,
    legacy_damage: RcBlock<dyn Fn(*mut AnyObject)>,
}

// SAFETY: blocks are reference counted with atomic operations and capture
// only `Arc<Shared>`, which is Send and Sync; NSUUID is immutable.
unsafe impl Send for Registration {}

impl Registration {
    fn new(shared: &Arc<Shared>) -> Registration {
        let frame = {
            let shared = shared.clone();
            RcBlock::with_encoding::<_, _, _, VoidBlock>(move || contained(|| shared.on_present()))
        };
        let surfaces = {
            let shared = shared.clone();
            RcBlock::with_encoding::<_, _, _, TwoObjectBlock>(
                move |framebuffer: *mut AnyObject, _masked: *mut AnyObject| {
                    // SAFETY: the render server passes nil or a live IOSurface.
                    contained(|| shared.on_surface(unsafe { Surface::from_object(framebuffer) }))
                },
            )
        };
        let properties =
            RcBlock::with_encoding::<_, _, _, ObjectBlock>(|_properties: *mut AnyObject| {});
        let legacy_surface = {
            let shared = shared.clone();
            RcBlock::with_encoding::<_, _, _, ObjectBlock>(move |surface: *mut AnyObject| {
                // SAFETY: CoreSimulator passes nil or a live IOSurface.
                contained(|| shared.on_surface(unsafe { Surface::from_object(surface) }))
            })
        };
        let legacy_damage = {
            let shared = shared.clone();
            RcBlock::with_encoding::<_, _, _, ObjectBlock>(move |_rectangles: *mut AnyObject| {
                contained(|| shared.on_present())
            })
        };
        Registration {
            uuid: NSUUID::new(),
            frame,
            surfaces,
            properties,
            legacy_surface,
            legacy_damage,
        }
    }

    fn callbacks(&self) -> Callbacks<'_> {
        Callbacks {
            frame: &self.frame,
            surfaces: &self.surfaces,
            properties: &self.properties,
            legacy_surface: &self.legacy_surface,
            legacy_damage: &self.legacy_damage,
        }
    }
}

/// A frame subscription on one device's main display.
pub struct Subscription {
    shared: Arc<Shared>,
    queue: Arc<FrameQueue>,
    worker: Option<JoinHandle<()>>,
    display: Display,
    registration: Registration,
    /// The SimScreen callbacks run on this queue, so it must outlive the
    /// registration.
    _callback_queue: DispatchRetained<DispatchQueue>,
    encoder: &'static str,
}

impl Subscription {
    pub fn start(device: &Device, options: &FrameOptions) -> Result<Subscription> {
        let display = Display::main(device)?;
        let encoder = options
            .encode
            .then(|| Encoder::new(options.quality))
            .transpose()?;
        let encoder_name = encoder.as_ref().map_or("none", Encoder::name);
        let shared = Arc::new(Shared::new());
        let queue = Arc::new(FrameQueue::new(QUEUE_CAPACITY));
        let registration = Registration::new(&shared);
        let callback_queue = display::callback_queue("dev.mobileagent.simulator.frames");
        display.register(
            &registration.uuid,
            &callback_queue,
            &registration.callbacks(),
        )?;

        // SimScreen reports the current surface at registration; the older
        // registration does not (S2), so it asks at once.
        let wait = if display.sim_screen {
            SURFACE_WAIT
        } else {
            Duration::ZERO
        };
        let registered = Instant::now();
        while shared.surface().is_none() && registered.elapsed() < wait {
            thread::sleep(Duration::from_millis(1));
        }
        if shared.surface().is_none() {
            match display.framebuffer_surface() {
                Ok(Some(surface)) => shared.on_surface(Some(surface)),
                failed => {
                    let _ = display.unregister(&registration.uuid);
                    return Err(failed.err().unwrap_or_else(|| Error::Frame {
                        reason: "the display has no surface".into(),
                    }));
                }
            }
        }

        let worker = {
            let (shared, queue, options) = (shared.clone(), queue.clone(), *options);
            thread::Builder::new()
                .name("simulator-frames".into())
                .spawn(move || read_frames(&shared, &queue, &options, encoder))
                .map_err(|error| Error::Frame {
                    reason: format!("the frame thread did not start: {error}"),
                })
        };
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                let _ = display.unregister(&registration.uuid);
                return Err(error);
            }
        };
        Ok(Subscription {
            shared,
            queue,
            worker: Some(worker),
            display,
            registration,
            _callback_queue: callback_queue,
            encoder: encoder_name,
        })
    }
}

impl FrameSource for Subscription {
    fn next(&self, timeout: Option<Duration>) -> Option<Frame> {
        self.queue.pop(timeout)
    }

    fn stats(&self) -> FrameStats {
        let shared = &self.shared;
        let nanos = shared.callback_cpu_ns.load(Ordering::Relaxed)
            + shared.worker_cpu_ns.load(Ordering::Relaxed);
        FrameStats {
            presents: shared.presents.load(Ordering::Relaxed),
            reads: shared.reads.load(Ordering::Relaxed),
            delivered: shared.delivered.load(Ordering::Relaxed),
            torn: shared.torn.load(Ordering::Relaxed),
            dropped: self.queue.dropped(),
            cpu: Duration::from_nanos(nanos),
            elapsed: shared.started.elapsed(),
            encoder: self.encoder,
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = self.display.unregister(&self.registration.uuid);
        self.queue.close();
    }
}

/// Closes the queue when the worker ends, however it ends.
struct CloseOnExit<'a>(&'a FrameQueue);

impl Drop for CloseOnExit<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// The worker: reads the frame on screen at once, then a frame on each
/// present the throttle lets through, delivering those whose pixels changed.
fn read_frames(
    shared: &Shared,
    queue: &FrameQueue,
    options: &FrameOptions,
    mut encoder: Option<Encoder>,
) {
    let _close = CloseOnExit(queue);
    // Frames are display work, so the thread asks for the QoS the render
    // server's own callbacks run at, which keeps it off the efficiency cores.
    // SAFETY: sets the calling thread's QoS class; no pointers are involved.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0)
    };
    let throttle = Throttle::new(options.max_fps);
    let max_dimension = options.max_dimension as usize;
    let mut scaled = Vec::new();
    let mut last_hash = None;
    let mut last_read: Option<Instant> = None;
    let mut sequence = 0;
    loop {
        if let Some(last) = last_read {
            let present = match throttle.resume_at(last) {
                Some(resume) => {
                    if !shared.sleep_until(resume) {
                        break;
                    }
                    // A present that came during the sleep is stale: reading
                    // it now could tear against the next one. Read on a fresh
                    // present, or on the stale one when none follows, since
                    // then the display has stopped.
                    let stale = shared.take_pending();
                    shared.wait_present(Some(throttle.fresh_wait()))
                        || stale
                        || shared.wait_present(None)
                }
                None => shared.wait_present(None),
            };
            if !present {
                break;
            }
        }
        if shared.stopping() {
            break;
        }
        last_read = Some(Instant::now());
        let Some(surface) = shared.surface() else {
            continue;
        };
        let captured = Instant::now();
        let captured_at = SystemTime::now();
        let read = surface.read(|view| {
            let (width, height) = fit_within(view.width(), view.height(), max_dimension);
            scale_bgra(&view, width, height, &mut scaled);
            (width, height)
        });
        shared.reads.fetch_add(1, Ordering::Relaxed);
        if let Ok(read) = read {
            let (width, height) = read.value;
            let hash = pipeline::pixels_hash(&scaled);
            if last_hash != Some(hash) {
                let jpeg = match encoder.as_mut() {
                    Some(encoder) => encoder.encode(&scaled, width, height).ok(),
                    None => None,
                };
                if jpeg.is_some() || encoder.is_none() {
                    let view = FrameView::new(&scaled, width, height, width * 4)
                        .expect("a packed frame of its own size");
                    last_hash = Some(hash);
                    sequence += 1;
                    if read.rewritten {
                        shared.torn.fetch_add(1, Ordering::Relaxed);
                    }
                    queue.push(Frame {
                        sequence,
                        captured_at,
                        captured,
                        width: width as u32,
                        height: height as u32,
                        jpeg,
                        hash,
                        content_hash: pipeline::content_hash(&view),
                        torn: read.rewritten,
                    });
                    shared.delivered.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        shared
            .worker_cpu_ns
            .store(thread_cpu().as_nanos() as u64, Ordering::Relaxed);
    }
}

/// A JPEG of the display now. A read the render server tore is retried.
pub fn screenshot(device: &Device, options: &ScreenshotOptions) -> Result<Screenshot> {
    let display = Display::main(device)?;
    let surface = display.framebuffer_surface()?.ok_or_else(|| Error::Frame {
        reason: "the display has no surface".into(),
    })?;
    let mut encoder = Encoder::new(options.quality)?;
    let mut scaled = Vec::new();
    let mut attempt = 1;
    loop {
        let captured_at = SystemTime::now();
        let read = surface.read(|view| {
            let (width, height) =
                fit_within(view.width(), view.height(), options.max_dimension as usize);
            scale_bgra(&view, width, height, &mut scaled);
            (width, height)
        })?;
        if read.rewritten && attempt < SCREENSHOT_ATTEMPTS {
            attempt += 1;
            continue;
        }
        let (width, height) = read.value;
        let data = match options.format {
            ImageFormat::Jpeg => encoder.encode(&scaled, width, height)?,
            ImageFormat::Bgra => unimplemented!("task 6.3: raw screenshots"),
        };
        return Ok(Screenshot {
            captured_at,
            width: width as u32,
            height: height as u32,
            format: options.format,
            data,
        });
    }
}
