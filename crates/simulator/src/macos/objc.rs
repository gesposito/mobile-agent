//! Objective-C runtime plumbing: loading the private frameworks, looking
//! classes up by name, containing the exceptions that private calls raise, and
//! turning `NSError`s into driver errors. Ported from spikes S2 to S4.

use std::ffi::{CStr, CString, c_void};
use std::ops::Deref;
use std::panic::AssertUnwindSafe;
use std::sync::OnceLock;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::exception::{self, Exception};
use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{NSError, NSString};

use crate::error::{Error, Failure, Result};

/// CoreSimulator, which Xcode installs outside its bundle.
pub const CORE_SIMULATOR: &str =
    "/Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/CoreSimulator";
/// CoreSimDeviceIO, re-exported by CoreSimulator; loaded explicitly as well.
pub const CORE_SIM_DEVICE_IO: &str = "/Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/Frameworks/CoreSimDeviceIO.framework/Versions/A/CoreSimDeviceIO";

/// Loads CoreSimulator and CoreSimDeviceIO, once per process.
pub fn load_core_simulator() -> Result<()> {
    static LOADED: OnceLock<Result<(), (String, String)>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            for (what, path) in [
                ("CoreSimulator", CORE_SIMULATOR),
                ("CoreSimDeviceIO", CORE_SIM_DEVICE_IO),
            ] {
                load(path).map_err(|reason| (what.to_string(), reason))?;
            }
            Ok(())
        })
        .clone()
        .map_err(|(what, reason)| Error::Load { what, reason })
}

/// `dlopen` with `RTLD_NOW | RTLD_GLOBAL`, so later `dlsym` lookups with
/// `RTLD_DEFAULT` find the image's symbols.
pub fn load(path: &str) -> std::result::Result<(), String> {
    let c_path = CString::new(path).map_err(|_| format!("{path} contains a NUL byte"))?;
    // SAFETY: `c_path` is a valid C string; dlopen has no other precondition.
    let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if handle.is_null() {
        // SAFETY: dlerror returns NULL or a NUL-terminated string owned by dyld.
        let reason = unsafe { libc::dlerror() };
        return Err(if reason.is_null() {
            format!("dlopen {path} failed")
        } else {
            // SAFETY: checked non-NULL above.
            unsafe { CStr::from_ptr(reason) }
                .to_string_lossy()
                .into_owned()
        });
    }
    Ok(())
}

/// The address of a C symbol in the images loaded so far.
pub fn symbol(name: &str) -> Option<*mut c_void> {
    let name = CString::new(name).ok()?;
    // SAFETY: RTLD_DEFAULT searches every loaded image; `name` is valid.
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    (!address.is_null()).then_some(address)
}

/// A registered class, looked up by name so nothing links against it.
pub fn class(name: &CStr) -> Result<&'static AnyClass> {
    AnyClass::get(name).ok_or_else(|| Error::Load {
        what: format!("class {}", name.to_string_lossy()),
        reason: "it is not registered; its framework did not load".into(),
    })
}

/// Runs one private call. An Objective-C exception it raises becomes an
/// error instead of unwinding through Rust frames, which would abort, and the
/// objects it autoreleases are released when it returns.
pub fn guarded<R>(call: &str, body: impl FnOnce() -> R) -> Result<R> {
    autoreleasepool(|_| exception::catch(AssertUnwindSafe(body))).map_err(|exception| {
        Error::Framework {
            call: call.to_string(),
            failure: exception_failure(exception.as_deref()),
        }
    })
}

/// A guarded call that reports failure through an `NSError`.
pub fn guarded_ns<R>(
    call: &str,
    body: impl FnOnce() -> std::result::Result<R, Retained<NSError>>,
) -> Result<R> {
    guarded(call, body)?.map_err(|error| ns_error(call, &error))
}

/// The driver error for an `NSError` that `call` reported.
pub fn ns_error(call: &str, error: &NSError) -> Error {
    Error::Framework {
        call: call.to_string(),
        failure: Failure::NsError {
            domain: error.domain().to_string(),
            code: error.code() as i64,
            description: error.localizedDescription().to_string(),
        },
    }
}

/// Converts the `NSError *` a completion block receives; nil is success.
///
/// # Safety
/// `error` must be nil or point to a live `NSError`.
pub unsafe fn completion_error(call: &str, error: *mut NSError) -> Result<()> {
    // SAFETY: forwarded to the caller.
    match unsafe { error.as_ref() } {
        None => Ok(()),
        Some(error) => Err(ns_error(call, error)),
    }
}

/// The name and reason of a caught exception. Objective-C can throw any
/// object, so only an `NSException` is asked for them.
fn exception_failure(exception: Option<&Exception>) -> Failure {
    let Some(exception) = exception else {
        return Failure::Exception {
            name: "nil".into(),
            reason: "a nil exception was raised".into(),
        };
    };
    let object: &AnyObject = exception;
    autoreleasepool(|_| {
        let is_ns_exception = object.class().responds_to(objc2::sel!(isKindOfClass:))
            && AnyClass::get(c"NSException").is_some_and(|class| {
                // SAFETY: `isKindOfClass:` takes a class and returns BOOL.
                unsafe { msg_send![object, isKindOfClass: class] }
            });
        if !is_ns_exception {
            return Failure::Exception {
                name: object.class().name().to_string_lossy().into_owned(),
                reason: format!("{exception:?}"),
            };
        }
        // SAFETY: `name` and `reason` of an NSException return NSString or nil.
        let (name, reason): (Option<Retained<NSString>>, Option<Retained<NSString>>) =
            unsafe { (msg_send![object, name], msg_send![object, reason]) };
        Failure::Exception {
            name: string(name),
            reason: string(reason),
        }
    })
}

/// A string property's value, empty for nil.
pub fn string(value: Option<Retained<NSString>>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

/// Whether `object` is an instance of the class named `name` or a subclass.
pub fn is_kind_of(object: &AnyObject, name: &CStr) -> bool {
    AnyClass::get(name).is_some_and(|class| {
        guarded("-isKindOfClass:", || {
            // SAFETY: `isKindOfClass:` takes a class and returns BOOL.
            unsafe { msg_send![object, isKindOfClass: class] }
        })
        .unwrap_or(false)
    })
}

/// Waits for the value a completion block sends.
pub fn wait<T>(receiver: Receiver<T>, call: &str, after: Duration) -> Result<T> {
    receiver.recv_timeout(after).map_err(|_| Error::Framework {
        call: call.to_string(),
        failure: Failure::NoCompletion(after),
    })
}

/// The serial queue CoreSimulator runs this process's completion blocks on.
/// The blocks only hand their result to a waiting thread.
pub fn completion_queue() -> &'static DispatchQueue {
    static QUEUE: OnceLock<DispatchRetained<DispatchQueue>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        DispatchQueue::new(
            "dev.mobileagent.simulator.completion",
            DispatchQueueAttr::SERIAL,
        )
    })
}

/// A retained CoreSimulator object shared between threads.
///
/// CoreSimulator's service context, device sets, devices, runtimes and device
/// types can be messaged from any thread: they synchronize internally, which
/// `simctl`, idb and spikes S2 to S4 all rely on.
#[derive(Clone)]
pub struct Shared(Retained<AnyObject>);

// SAFETY: see the type's documentation; only such objects are wrapped.
unsafe impl Send for Shared {}
// SAFETY: as above.
unsafe impl Sync for Shared {}

impl Shared {
    pub fn new(object: Retained<AnyObject>) -> Shared {
        Shared(object)
    }
}

impl Deref for Shared {
    type Target = AnyObject;

    fn deref(&self) -> &AnyObject {
        &self.0
    }
}

/// Moves a value into a block that another thread runs.
pub struct SendBox<T>(pub T);

// SAFETY: used for CoreSimulator objects and Foundation values handed from a
// completion queue to the one thread waiting for them, which alone uses them
// afterwards.
unsafe impl<T> Send for SendBox<T> {}

#[cfg(test)]
mod tests {
    use objc2::runtime::NSObject;
    use objc2_foundation::NSArray;

    use super::*;

    #[test]
    fn an_objective_c_exception_becomes_an_error_naming_the_call() {
        let empty = NSArray::<NSObject>::new();
        let result = guarded("-[NSArray objectAtIndex:]", || {
            // SAFETY: a valid message whose only fault is the index, which
            // raises NSRangeException.
            let _: Option<Retained<AnyObject>> =
                unsafe { msg_send![&*empty, objectAtIndex: 5usize] };
        });
        match result {
            Err(Error::Framework {
                call,
                failure: Failure::Exception { name, reason },
            }) => {
                assert_eq!(call, "-[NSArray objectAtIndex:]");
                assert_eq!(name, "NSRangeException");
                assert!(reason.contains("index 5"), "{reason}");
            }
            other => panic!("expected a contained exception, got {other:?}"),
        }
    }

    #[test]
    fn a_call_that_does_not_raise_returns_its_value() {
        assert_eq!(guarded("a sum", || 2 + 2), Ok(4));
    }

    #[test]
    fn core_simulator_loads_and_registers_its_classes() {
        load_core_simulator().expect("CoreSimulator loads");
        for name in [c"SimServiceContext", c"SimDeviceSet", c"SimDevice"] {
            class(name).expect("the class is registered");
        }
        assert!(matches!(
            class(c"NoSuchClassAnywhere"),
            Err(Error::Load { .. })
        ));
    }
}
