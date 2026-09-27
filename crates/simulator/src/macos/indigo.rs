//! The legacy Indigo transport (design D6, spike S3), for hosts whose
//! CoreSimulator predates 1155.4 and the dtuhidd service.
//!
//! SimulatorKit's `SimDeviceLegacyHIDClient` takes a `malloc`ed Indigo
//! message over a Mach port. The private `IndigoHIDMessageForMouseNSEvent`
//! builds it; its Xcode 27 prototype is `(CGPoint *, CGPoint *,
//! IndigoHIDTarget, NSEventType, NSSize, IndigoHIDEdge)`. The builder divides
//! the point by the size, so a unit size keeps a ratio as it is; the size
//! goes in floating-point registers and must be passed explicitly (iosef
//! #15). The builder emits a multi-touch message, which is re-enveloped by
//! hand as the single-touch message idb sends. It keeps unsynchronized
//! global state and returns nil for a message built within 16 ms of the
//! previous one, so every build in this process holds one lock and keeps
//! that spacing.
//!
//! Only single-finger touch goes this way; keys, buttons, two fingers and
//! screen-edge touches are refused. On this host S3 found Indigo touch
//! working before and after dtuhidd was activated.

use std::ffi::{CStr, c_void};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::encode::{Encode, Encoding};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSError, NSSize};

use crate::error::{Error, Failure, Result};
use crate::gesture::{Phase, Ratio, TouchStep};
use crate::macos::coresim::{Device, developer_dir};
use crate::macos::input::Transport;
use crate::macos::objc::{self, Shared, guarded};
use crate::model::{Button, Edge, TransportKind};

/// The Swift class's runtime name; `NSClassFromString` also knows it as
/// `SimulatorKit.SimDeviceLegacyHIDClient`.
const CLIENT_CLASSES: [&CStr; 2] = [
    c"_TtC12SimulatorKit24SimDeviceLegacyHIDClient",
    c"SimulatorKit.SimDeviceLegacyHIDClient",
];

/// The main screen's digitizer.
const TARGET_DIGITIZER: u32 = 0x32;
/// The `NSEventType`s the builder turns into touch phases.
const LEFT_MOUSE_DOWN: usize = 1;
const LEFT_MOUSE_UP: usize = 2;
const LEFT_MOUSE_DRAGGED: usize = 6;
/// `IndigoHIDEdgeNone`.
const EDGE_NONE: u32 = 0;
/// The builder refuses a message within 16 ms of the previous one.
const BUILD_SPACING: Duration = Duration::from_millis(17);
/// How long a send's completion may take.
const COMPLETION: Duration = Duration::from_secs(5);

/// `IndigoHIDMessageForMouseNSEvent`.
type MouseMessageFn =
    unsafe extern "C" fn(*mut CGPoint, *const CGPoint, u32, usize, NSSize, u32) -> *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGPoint {
    x: f64,
    y: f64,
}

/// The offsets idb's `Indigo.h` documents for a single-touch message.
mod layout {
    /// `IndigoMessage`: the Mach header, `innerSize`, `eventType`, payload.
    pub const PAYLOAD: usize = 0x20;
    /// `IndigoPayload`'s `event` union, where the `IndigoTouch` sits.
    pub const EVENT: usize = 0x30;
    /// Swift's `sizeof(IndigoPayload)`, the stride a hand-built message uses.
    pub const PAYLOAD_STRIDE: usize = 0x90;
    /// `sizeof(IndigoMessage)`.
    pub const MESSAGE: usize = 0xB0;
    /// `sizeof(IndigoTouch)`.
    pub const TOUCH: usize = 0x78;
    pub const INNER_SIZE: usize = 0x18;
    pub const EVENT_TYPE: usize = 0x1C;
    pub const EVENT_KIND: usize = PAYLOAD;
    pub const TIMESTAMP: usize = PAYLOAD + 0x04;
    /// `IndigoTouch.xRatio` and `.yRatio`, from the touch.
    pub const X_RATIO: usize = 0x0C;
    pub const Y_RATIO: usize = 0x14;
    /// `IndigoTouch.field1` and `.field2`, which mark the second copy.
    pub const FIELD1: usize = 0x00;
    pub const FIELD2: usize = 0x04;
}

unsafe extern "C" {
    fn mach_absolute_time() -> u64;
}

/// A message pointer typed as SimulatorKit declares it, so objc2's
/// debug-build signature check accepts it; an encoding without fields matches
/// the struct whatever its fields.
#[repr(transparent)]
struct IndigoMessage(*mut c_void);

// SAFETY: a pointer to the struct, which is what the encoding says.
unsafe impl Encode for IndigoMessage {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("IndigoHIDMessageStruct", &[]));
}

/// When the builder last ran, process-wide.
static BUILDER: Mutex<Option<Instant>> = Mutex::new(None);

fn builder_lock() -> MutexGuard<'static, Option<Instant>> {
    BUILDER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// SimulatorKit: Xcode 27 keeps it in `SharedFrameworks`, earlier Xcodes in
/// the developer directory.
fn simulator_kit() -> PathBuf {
    let developer = developer_dir();
    let binary = "SimulatorKit.framework/Versions/A/SimulatorKit";
    let shared = developer.join("../SharedFrameworks").join(binary);
    if shared.exists() {
        return shared;
    }
    developer.join("Library/PrivateFrameworks").join(binary)
}

/// One device's legacy HID client and the builder.
pub struct Indigo {
    client: Shared,
    build: MouseMessageFn,
    completions: DispatchRetained<DispatchQueue>,
}

impl Indigo {
    pub fn open(device: &Device) -> Result<Indigo> {
        let framework = simulator_kit();
        objc::load(&framework.to_string_lossy()).map_err(|reason| Error::Load {
            what: format!("SimulatorKit ({})", framework.display()),
            reason,
        })?;
        let address =
            objc::symbol("IndigoHIDMessageForMouseNSEvent").ok_or_else(|| Error::Load {
                what: "IndigoHIDMessageForMouseNSEvent".into(),
                reason: "SimulatorKit does not export it".into(),
            })?;
        // SAFETY: the Xcode 27 prototype, from SimulatorKit's assertion
        // strings and idb's and iosef's bindings; S3 called it this way.
        let build = unsafe { std::mem::transmute::<*mut c_void, MouseMessageFn>(address) };
        let class = CLIENT_CLASSES
            .iter()
            .find_map(|name| objc::class(name).ok())
            .ok_or_else(|| Error::Load {
                what: "SimDeviceLegacyHIDClient".into(),
                reason: "SimulatorKit registers no such class".into(),
            })?;

        const CALL: &str = "-[SimDeviceLegacyHIDClient initWithDevice:error:]";
        let client = guarded(CALL, || {
            // `alloc` and the initializer are sent apart, as idb does,
            // because the initializer can decline and return nil.
            // SAFETY: `alloc` returns a +1 instance or nil.
            let allocated: *mut AnyObject = unsafe { msg_send![class, alloc] };
            if allocated.is_null() {
                return Err(Error::Framework {
                    call: "+[SimDeviceLegacyHIDClient alloc]".into(),
                    failure: Failure::Nil,
                });
            }
            let mut error: Option<Retained<NSError>> = None;
            // SAFETY: consumes `allocated`; takes a SimDevice and an NSError
            // out pointer and returns a +1 client or nil.
            let client: *mut AnyObject =
                unsafe { msg_send![allocated, initWithDevice: device.object(), error: &mut error] };
            // SAFETY: the initializer's +1 result or nil.
            match (unsafe { Retained::from_raw(client) }, error) {
                (Some(client), _) => Ok(client),
                (None, Some(error)) => Err(objc::ns_error(CALL, &error)),
                (None, None) => Err(Error::Framework {
                    call: CALL.into(),
                    failure: Failure::Nil,
                }),
            }
        })??;
        Ok(Indigo {
            client: Shared::new(client),
            build,
            completions: DispatchQueue::new(
                "dev.mobileagent.simulator.indigo",
                DispatchQueueAttr::SERIAL,
            ),
        })
    }

    /// Builds one contact's message under the process-wide builder lock.
    fn message(&self, ratio: Ratio, event_type: usize) -> Result<*mut c_void> {
        let mut last = builder_lock();
        if let Some(last) = *last
            && let Some(wait) = (last + BUILD_SPACING).checked_duration_since(Instant::now())
        {
            thread::sleep(wait);
        }
        let mut point = CGPoint {
            x: ratio.x,
            y: ratio.y,
        };
        let source = guarded("IndigoHIDMessageForMouseNSEvent", || {
            // SAFETY: a valid point, no second point, the main digitizer, a
            // mouse event type, a unit size and no edge; the result is a
            // `malloc`ed message or NULL.
            unsafe {
                (self.build)(
                    &mut point,
                    std::ptr::null(),
                    TARGET_DIGITIZER,
                    event_type,
                    NSSize {
                        width: 1.0,
                        height: 1.0,
                    },
                    EDGE_NONE,
                )
            }
        })?;
        *last = Some(Instant::now());
        if source.is_null() {
            return Err(Error::Framework {
                call: "IndigoHIDMessageForMouseNSEvent".into(),
                failure: Failure::Nil,
            });
        }
        Ok(single_touch(source, ratio))
    }

    /// Sends a built message, which the client frees, and waits for its
    /// completion.
    fn deliver(&self, message: *mut c_void) -> Result<()> {
        const CALL: &str =
            "-[SimDeviceLegacyHIDClient sendWithMessage:freeWhenDone:completionQueue:completion:]";
        let (tx, rx) = mpsc::channel();
        let completion = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: the client passes nil or a live NSError.
            let _ = tx.send(unsafe { objc::completion_error(CALL, error) });
        });
        guarded(CALL, || {
            // SAFETY: a malloc'ed message the client frees, a queue and a
            // `void (^)(NSError *)` completion.
            unsafe {
                let _: () = msg_send![
                    &*self.client,
                    sendWithMessage: IndigoMessage(message),
                    freeWhenDone: true,
                    completionQueue: &*self.completions,
                    completion: &*completion
                ];
            }
        })?;
        objc::wait(rx, CALL, COMPLETION)?
    }
}

impl Transport for Indigo {
    fn kind(&self) -> TransportKind {
        TransportKind::Indigo
    }

    fn touch(&self, step: &TouchStep) -> Result<()> {
        let [contact] = step.contacts[..] else {
            return Err(Error::Unsupported {
                what: "a two-finger gesture over the Indigo transport".into(),
            });
        };
        if step.edge != Edge::None {
            return Err(Error::Unsupported {
                what: "a screen-edge touch over the Indigo transport".into(),
            });
        }
        let event_type = match step.phase {
            Phase::Down => LEFT_MOUSE_DOWN,
            Phase::Move => LEFT_MOUSE_DRAGGED,
            Phase::Up => LEFT_MOUSE_UP,
        };
        let message = self.message(contact, event_type)?;
        self.deliver(message)
    }

    fn button(&self, _button: Button, _down: bool) -> Result<()> {
        Err(Error::Unsupported {
            what: "a hardware button over the Indigo transport".into(),
        })
    }

    fn key(&self, _usage: u32, _down: bool) -> Result<()> {
        Err(Error::Unsupported {
            what: "a key over the Indigo transport, which CoreSimulator 1155.4 and later drop"
                .into(),
        })
    }

    /// Every send already waited for its completion.
    fn flush(&self) -> Result<()> {
        Ok(())
    }
}

/// Re-envelopes the builder's multi-touch message as the single-touch
/// (event type 0x02) two-payload message the guest expects and frees the
/// builder's; the layout is idb's `SimulatorIndigoHID.touchMessage`.
fn single_touch(source: *mut c_void, ratio: Ratio) -> *mut c_void {
    let size = layout::MESSAGE + layout::PAYLOAD_STRIDE;
    // SAFETY: the builder returns a `malloc`ed message of at least
    // `layout::EVENT + layout::TOUCH` bytes; `calloc` zeroes the new one.
    unsafe {
        let destination = libc::calloc(1, size).cast::<u8>();
        assert!(!destination.is_null(), "calloc of {size} bytes failed");
        destination
            .add(layout::INNER_SIZE)
            .cast::<u32>()
            .write_unaligned(layout::PAYLOAD_STRIDE as u32);
        destination.add(layout::EVENT_TYPE).write(0x02);
        destination
            .add(layout::EVENT_KIND)
            .cast::<u32>()
            .write_unaligned(0x0000_000B);
        destination
            .add(layout::TIMESTAMP)
            .cast::<u64>()
            .write_unaligned(mach_absolute_time());

        // The builder's contact, with the ratios written over its own
        // normalization, which a caller holding a ratio already did.
        let touch = source.cast::<u8>().add(layout::EVENT);
        touch
            .add(layout::X_RATIO)
            .cast::<f64>()
            .write_unaligned(ratio.x);
        touch
            .add(layout::Y_RATIO)
            .cast::<f64>()
            .write_unaligned(ratio.y);
        std::ptr::copy_nonoverlapping(touch, destination.add(layout::EVENT), layout::TOUCH);

        // The payload is duplicated and the copy marked as the second.
        let first = destination.add(layout::PAYLOAD);
        let second = first.add(layout::PAYLOAD_STRIDE);
        std::ptr::copy_nonoverlapping(first, second, layout::PAYLOAD_STRIDE);
        let second_touch = second.add(layout::EVENT - layout::PAYLOAD);
        second_touch
            .add(layout::FIELD1)
            .cast::<u32>()
            .write_unaligned(1);
        second_touch
            .add(layout::FIELD2)
            .cast::<u32>()
            .write_unaligned(2);

        libc::free(source);
        destination.cast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hand_built_message_matches_idbs_layout() {
        assert_eq!(layout::MESSAGE + layout::PAYLOAD_STRIDE, 0x140);
        assert_eq!(layout::EVENT - layout::PAYLOAD, 0x10);
        assert_eq!(layout::PAYLOAD + layout::PAYLOAD_STRIDE, 0xB0);
        assert_eq!(layout::EVENT + layout::X_RATIO, 0x3C);
        assert_eq!(layout::EVENT + layout::Y_RATIO, 0x44);
    }

    #[test]
    fn simulator_kit_is_found_for_the_selected_xcode() {
        let path = simulator_kit();
        assert!(path.exists(), "{} does not exist", path.display());
    }
}
