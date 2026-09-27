//! The main display of a device, from spike S2: its descriptor among the
//! device's IO ports, the SimScreen callbacks that tick once per present, and
//! the IOSurface they hand over.
//!
//! Everything reached from `-[SimDevice io]` onward is a ROCKit forwarding
//! proxy, so those calls go through `forward::send`, and blocks handed to
//! ROCKit carry their signature (`RcBlock::with_encoding`); without one,
//! registration raises "Block is missing signature field".

use std::ffi::CStr;
use std::ptr::NonNull;

use block2::{Block, ManualBlockEncoding};
use dispatch2::{DispatchQoS, DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, AnyProtocol};
use objc2::{msg_send, sel};
use objc2_core_foundation::{CFRetained, CFType};
use objc2_foundation::{NSArray, NSUUID};
use objc2_io_surface::{IOSurfaceLockOptions, IOSurfaceRef};

use crate::error::{Error, Result};
use crate::macos::coresim::Device;
use crate::macos::forward;
use crate::macos::objc::{Shared, guarded};
use crate::pipeline::FrameView;

/// `kCVPixelFormatType_32BGRA`, the FourCC 'BGRA', which S2 found on every
/// surface.
const PIXEL_FORMAT_BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// A retained IOSurface.
pub struct Surface(CFRetained<IOSurfaceRef>);

// SAFETY: IOSurfaceRef is a thread-safe CoreFoundation object; its lock only
// coordinates CPU access with the render server.
unsafe impl Send for Surface {}
// SAFETY: as above; every accessor is a read or a lock and unlock pair.
unsafe impl Sync for Surface {}

/// What `Surface::read` returns: the closure's value and whether the surface
/// was written while it was read.
pub struct Read<R> {
    pub value: R,
    /// The seed moved between the lock and the end of the read: the render
    /// server does not wait for a reader's lock, so the pixels may be torn.
    pub rewritten: bool,
}

impl Surface {
    /// Retains `object` if it is an IOSurface; nil or another type is `None`.
    ///
    /// # Safety
    /// `object` must be nil or a live Objective-C or CoreFoundation object.
    pub unsafe fn from_object(object: *mut AnyObject) -> Option<Surface> {
        let object = NonNull::new(object.cast::<CFType>())?;
        // SAFETY: the caller passes a live object; CF types are ObjC objects.
        let object = unsafe { CFRetained::retain(object) };
        object.downcast::<IOSurfaceRef>().ok().map(Surface)
    }

    pub fn width(&self) -> usize {
        self.0.width()
    }

    pub fn height(&self) -> usize {
        self.0.height()
    }

    /// Locks the surface read-only, hands `read` a view of its BGRA pixels
    /// and unlocks it.
    pub fn read<R>(&self, read: impl FnOnce(FrameView<'_>) -> R) -> Result<Read<R>> {
        let format = self.0.pixel_format();
        if format != PIXEL_FORMAT_BGRA {
            return Err(Error::Frame {
                reason: format!(
                    "the surface's pixel format is {:?}, not BGRA",
                    String::from_utf8_lossy(&format.to_be_bytes())
                ),
            });
        }
        let mut seed = 0u32;
        // SAFETY: `seed` is a valid out pointer.
        let status = unsafe { self.0.lock(IOSurfaceLockOptions::ReadOnly, &mut seed) };
        if status != 0 {
            return Err(Error::Frame {
                reason: format!("IOSurfaceLock returned {status:#x}"),
            });
        }
        let base = self.0.base_address().cast::<u8>();
        // SAFETY: while locked, the base address maps `alloc_size` bytes.
        let data = unsafe { std::slice::from_raw_parts(base.as_ptr(), self.0.alloc_size()) };
        let result = FrameView::new(data, self.width(), self.height(), self.0.bytes_per_row())
            .map(read)
            .map_err(|error| Error::Frame {
                reason: format!("surface geometry: {error}"),
            });
        let rewritten = self.0.seed() != seed;
        let mut unlock_seed = 0u32;
        // SAFETY: balances the lock above with the same options.
        unsafe {
            self.0
                .unlock(IOSurfaceLockOptions::ReadOnly, &mut unlock_seed)
        };
        result.map(|value| Read { value, rewritten })
    }
}

/// `void (^)(void)`, the per-present tick.
pub struct VoidBlock;

// SAFETY: the encoding of `void (^)(void)` on a 64-bit target.
unsafe impl ManualBlockEncoding for VoidBlock {
    type Arguments = ();
    type Return = ();
    const ENCODING_CSTR: &'static CStr = c"v8@?0";
}

/// `void (^)(id)`.
pub struct ObjectBlock;

// SAFETY: the encoding of `void (^)(id)` on a 64-bit target.
unsafe impl ManualBlockEncoding for ObjectBlock {
    type Arguments = (*mut AnyObject,);
    type Return = ();
    const ENCODING_CSTR: &'static CStr = c"v16@?0@8";
}

/// `void (^)(id, id)`, the surfaces callback.
pub struct TwoObjectBlock;

// SAFETY: the encoding of `void (^)(id, id)` on a 64-bit target.
unsafe impl ManualBlockEncoding for TwoObjectBlock {
    type Arguments = (*mut AnyObject, *mut AnyObject);
    type Return = ();
    const ENCODING_CSTR: &'static CStr = c"v24@?0@8@16";
}

/// A serial queue for the display callbacks, at the QoS the render server
/// donates when it delivers them.
pub fn callback_queue(label: &str) -> DispatchRetained<DispatchQueue> {
    let attributes = DispatchQueueAttr::with_qos_class(
        DispatchQueueAttr::SERIAL,
        DispatchQoS::UserInteractive,
        0,
    );
    DispatchQueue::new(label, Some(&attributes))
}

/// The blocks a subscription registers. SimScreen takes the first three;
/// the older registration takes the last two, the damage rectangles serving
/// as the per-present tick.
pub struct Callbacks<'a> {
    /// Once per present, whether or not pixels changed.
    pub frame: &'a Block<dyn Fn()>,
    /// With the framebuffer and masked framebuffer surfaces, when they
    /// change and once at registration.
    pub surfaces: &'a Block<dyn Fn(*mut AnyObject, *mut AnyObject)>,
    /// With the screen properties.
    pub properties: &'a Block<dyn Fn(*mut AnyObject)>,
    /// With the surface, when it changes; not at registration (S2).
    pub legacy_surface: &'a Block<dyn Fn(*mut AnyObject)>,
    /// With the damaged rectangles, once per present.
    pub legacy_damage: &'a Block<dyn Fn(*mut AnyObject)>,
}

/// A device's main display: the IO port descriptor that renders to an
/// IOSurface and reports `displayClass == 0`.
pub struct Display {
    descriptor: Shared,
    /// The descriptor conforms to CoreSimDeviceIO's SimScreen, whose
    /// callbacks are preferred; otherwise the older surface-change and
    /// damage-rectangle callbacks stand in (S2).
    pub sim_screen: bool,
}

fn protocol(name: &CStr) -> Result<&'static AnyProtocol> {
    AnyProtocol::get(name).ok_or_else(|| Error::Load {
        what: format!("protocol {}", name.to_string_lossy()),
        reason: "it is not registered; CoreSimDeviceIO did not load".into(),
    })
}

fn conforms(object: &AnyObject, protocol: &AnyProtocol) -> Result<bool> {
    guarded("-conformsToProtocol:", || {
        // SAFETY: `conformsToProtocol:` takes a protocol and returns BOOL;
        // ROCKit's proxies implement it.
        unsafe { msg_send![object, conformsToProtocol: protocol] }
    })
}

impl Display {
    /// Finds the main display among the device's IO ports.
    pub fn main(device: &Device) -> Result<Display> {
        let io = device.io()?;
        let ports: Option<Retained<NSArray<AnyObject>>> =
            guarded("-[SimDeviceIOClient ioPorts]", || {
                // SAFETY: returns an NSArray of port proxies.
                unsafe { msg_send![&*io, ioPorts] }
            })?;
        let ports = ports.map(|ports| ports.to_vec()).unwrap_or_default();
        let renderable = protocol(c"SimDisplayRenderable")?;
        let surface_renderable = protocol(c"SimDisplayIOSurfaceRenderable")?;
        let screen = protocol(c"SimScreen")?;
        let mut classes = Vec::new();
        for port in &ports {
            let descriptor = guarded("-[SimDeviceIOPortInterface descriptor]", || {
                // SAFETY: `descriptor` takes nothing and returns an object.
                unsafe { forward::send_object(port, sel!(descriptor), ()) }
            })?;
            let Some(descriptor) = descriptor else {
                continue;
            };
            if !conforms(&descriptor, renderable)? || !conforms(&descriptor, surface_renderable)? {
                continue;
            }
            let state = guarded("-[SimDisplayRenderable state]", || {
                // SAFETY: `state` takes nothing and returns an object.
                unsafe { forward::send_object(&descriptor, sel!(state), ()) }
            })?;
            let Some(state) = state else { continue };
            let display_class: u16 = guarded("-[SimDisplayDescriptorState displayClass]", || {
                // SAFETY: `displayClass` returns an unsigned short.
                unsafe { forward::send(&state, sel!(displayClass), ()) }
            })?;
            classes.push(display_class);
            if display_class == 0 {
                let sim_screen = conforms(&descriptor, screen)?;
                return Ok(Display {
                    descriptor: Shared::new(descriptor),
                    sim_screen,
                });
            }
        }
        Err(Error::Frame {
            reason: format!(
                "no main display among {} IO ports (display classes {classes:?})",
                ports.len()
            ),
        })
    }

    /// `-framebufferSurface`: a synchronous round trip to the render server,
    /// 0.3 ms at the median in S2.
    pub fn framebuffer_surface(&self) -> Result<Option<Surface>> {
        guarded(
            "-[SimDisplayIOSurfaceRenderable framebufferSurface]",
            || {
                // SAFETY: takes nothing and returns an IOSurface or nil.
                let surface =
                    unsafe { forward::send_object(&self.descriptor, sel!(framebufferSurface), ()) };
                // SAFETY: the pointer comes from a live retained object.
                surface.and_then(|surface| unsafe {
                    Surface::from_object(Retained::as_ptr(&surface).cast_mut())
                })
            },
        )
    }

    /// Registers `callbacks` under `uuid`. SimScreen delivers them on
    /// `queue`, the older registration on CoreSimulator's own queues. The
    /// blocks and the queue must outlive the registration.
    pub fn register(
        &self,
        uuid: &NSUUID,
        queue: &DispatchQueue,
        callbacks: &Callbacks<'_>,
    ) -> Result<()> {
        if self.sim_screen {
            return guarded(
                "-[SimScreen registerScreenCallbacksWithUUID:callbackQueue:frameCallback:surfacesChangedCallback:propertiesChangedCallback:]",
                // SAFETY: the signature `v@@@?@?@?` with signed blocks, as S2
                // found; the debug build checks it against the proxy's.
                || unsafe {
                    forward::send::<_, ()>(
                        &self.descriptor,
                        sel!(registerScreenCallbacksWithUUID:callbackQueue:frameCallback:surfacesChangedCallback:propertiesChangedCallback:),
                        (
                            uuid,
                            queue,
                            callbacks.frame,
                            callbacks.surfaces,
                            callbacks.properties,
                        ),
                    )
                },
            );
        }
        guarded(
            "-[SimDisplayIOSurfaceRenderable registerCallbackWithUUID:ioSurfacesChangeCallback:]",
            // SAFETY: an NSUUID and a signed `void (^)(id)` block.
            || unsafe {
                forward::send::<_, ()>(
                    &self.descriptor,
                    sel!(registerCallbackWithUUID:ioSurfacesChangeCallback:),
                    (uuid, callbacks.legacy_surface),
                )
            },
        )?;
        guarded(
            "-[SimDisplayRenderable registerCallbackWithUUID:damageRectanglesCallback:]",
            // SAFETY: an NSUUID and a signed `void (^)(id)` block.
            || unsafe {
                forward::send::<_, ()>(
                    &self.descriptor,
                    sel!(registerCallbackWithUUID:damageRectanglesCallback:),
                    (uuid, callbacks.legacy_damage),
                )
            },
        )
    }

    /// Removes the registration made under `uuid`.
    pub fn unregister(&self, uuid: &NSUUID) -> Result<()> {
        if self.sim_screen {
            return guarded("-[SimScreen unregisterScreenCallbacksWithUUID:]", || {
                // SAFETY: takes the NSUUID given at registration.
                unsafe {
                    forward::send::<_, ()>(
                        &self.descriptor,
                        sel!(unregisterScreenCallbacksWithUUID:),
                        (uuid,),
                    )
                }
            });
        }
        guarded(
            "-[SimDisplayIOSurfaceRenderable unregisterIOSurfacesChangeCallbackWithUUID:]",
            // SAFETY: takes the NSUUID given at registration.
            || unsafe {
                forward::send::<_, ()>(
                    &self.descriptor,
                    sel!(unregisterIOSurfacesChangeCallbackWithUUID:),
                    (uuid,),
                )
            },
        )?;
        guarded(
            "-[SimDisplayRenderable unregisterDamageRectanglesCallbackWithUUID:]",
            // SAFETY: takes the NSUUID given at registration.
            || unsafe {
                forward::send::<_, ()>(
                    &self.descriptor,
                    sel!(unregisterDamageRectanglesCallbackWithUUID:),
                    (uuid,),
                )
            },
        )
    }
}
