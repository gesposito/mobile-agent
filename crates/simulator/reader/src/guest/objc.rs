//! Objective-C and dyld plumbing: loading the runtime's private images by
//! path, resolving C functions with `dlsym`, looking classes up by name, and
//! containing what a private call raises. Ported from spike S4.

use std::any::Any;
use std::ffi::{CStr, CString, c_void};
use std::fmt;
use std::panic::{self, AssertUnwindSafe};

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::exception::{self, Exception};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSString;

/// How a private call or a load failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// An image, symbol or class is missing.
    Missing(String),
    /// The call raised an Objective-C exception.
    Exception {
        call: String,
        name: String,
        reason: String,
    },
    /// Rust code inside the call panicked, such as objc2's debug-build
    /// signature check.
    Panic { call: String, message: String },
    /// The call returned nil where a value was required.
    Nil { call: String },
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Missing(what) => f.write_str(what),
            Failure::Exception { call, name, reason } => {
                write!(f, "{call} raised {name}: {reason}")
            }
            Failure::Panic { call, message } => write!(f, "{call} panicked: {message}"),
            Failure::Nil { call } => write!(f, "{call} returned nil"),
        }
    }
}

/// Runs one private call. An Objective-C exception it raises becomes a
/// failure instead of unwinding through Rust frames, which would abort; so
/// does a Rust panic inside it.
pub fn guarded<R>(call: &str, body: impl FnOnce() -> R) -> Result<R, Failure> {
    let caught = panic::catch_unwind(AssertUnwindSafe(|| {
        exception::catch(AssertUnwindSafe(body))
    }));
    match caught {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(exception)) => {
            let (name, reason) = describe(exception.as_deref());
            Err(Failure::Exception {
                call: call.to_owned(),
                name,
                reason,
            })
        }
        Err(panic) => Err(Failure::Panic {
            call: call.to_owned(),
            message: panic_message(panic.as_ref()),
        }),
    }
}

/// The text a panic was raised with.
pub fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a message".into())
}

/// The name and reason of a caught exception. Objective-C can throw any
/// object, so only an `NSException` is asked for them, inside a catch of its
/// own.
fn describe(exception: Option<&Exception>) -> (String, String) {
    let Some(exception) = exception else {
        return ("nil".into(), "a nil exception was raised".into());
    };
    let object: &AnyObject = exception;
    let described = exception::catch(AssertUnwindSafe(|| {
        let ns_exception = AnyClass::get(c"NSException").is_some_and(|class| {
            // SAFETY: `isKindOfClass:` takes a class and returns BOOL.
            unsafe { msg_send![object, isKindOfClass: class] }
        });
        ns_exception.then(|| {
            // SAFETY: an NSException's `name` and `reason` are NSString or nil.
            let (name, reason): (Option<Retained<NSString>>, Option<Retained<NSString>>) =
                unsafe { (msg_send![object, name], msg_send![object, reason]) };
            (
                name.map(|n| n.to_string()).unwrap_or_default(),
                reason.map(|r| r.to_string()).unwrap_or_default(),
            )
        })
    }));
    match described {
        Ok(Some(pair)) => pair,
        _ => (
            object.class().name().to_string_lossy().into_owned(),
            format!("{exception:?}"),
        ),
    }
}

/// `dlopen` with `RTLD_NOW | RTLD_GLOBAL`. Inside the Simulator a path
/// resolves against the runtime root, so guest paths are written as the
/// guest sees them.
pub fn load(path: &str) -> Result<(), Failure> {
    let c_path =
        CString::new(path).map_err(|_| Failure::Missing(format!("{path} has a NUL byte")))?;
    // SAFETY: `c_path` is a valid C string.
    let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if handle.is_null() {
        // SAFETY: dlerror returns NULL or a NUL-terminated string owned by dyld.
        let reason = unsafe { libc::dlerror() };
        let reason = if reason.is_null() {
            "unknown error".to_owned()
        } else {
            // SAFETY: checked non-NULL above.
            unsafe { CStr::from_ptr(reason) }
                .to_string_lossy()
                .into_owned()
        };
        return Err(Failure::Missing(format!("dlopen {path}: {reason}")));
    }
    Ok(())
}

/// The address of a C function in the images loaded so far.
pub fn symbol(name: &CStr) -> Result<*mut c_void, Failure> {
    // SAFETY: RTLD_DEFAULT searches every loaded image; `name` is valid.
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if address.is_null() {
        Err(Failure::Missing(format!(
            "dlsym {}: not found",
            name.to_string_lossy()
        )))
    } else {
        Ok(address)
    }
}

/// A registered class, looked up by name so nothing links against it.
pub fn class(name: &CStr) -> Result<&'static AnyClass, Failure> {
    AnyClass::get(name).ok_or_else(|| {
        Failure::Missing(format!(
            "class {} is not registered",
            name.to_string_lossy()
        ))
    })
}

/// `AXUIElementRef`, a Core Foundation object that the snapshot and AXRuntime
/// methods type as `^{__AXUIElement=}`. Encoding it as such keeps objc2's
/// debug-build signature checks on for those calls.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct AXUIElementRef(pub *mut c_void);

// SAFETY: a pointer to the opaque `__AXUIElement` struct, as the runtime's
// method signatures declare it.
unsafe impl Encode for AXUIElementRef {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("__AXUIElement", &[]));
}

// SAFETY: as above, one level of indirection further.
unsafe impl RefEncode for AXUIElementRef {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}

impl AXUIElementRef {
    pub fn is_null(self) -> bool {
        self.0.is_null()
    }
}

#[cfg(test)]
mod tests {
    use objc2::runtime::NSObject;
    use objc2_foundation::NSArray;

    use super::*;

    #[test]
    fn an_exception_becomes_a_failure_naming_the_call() {
        let empty = NSArray::<NSObject>::new();
        let result = guarded("-[NSArray objectAtIndex:]", || {
            // SAFETY: a valid message whose only fault is the index, which
            // raises NSRangeException.
            let _: Option<Retained<AnyObject>> =
                unsafe { msg_send![&*empty, objectAtIndex: 5usize] };
        });
        match result {
            Err(Failure::Exception { call, name, reason }) => {
                assert_eq!(call, "-[NSArray objectAtIndex:]");
                assert_eq!(name, "NSRangeException");
                assert!(reason.contains("index 5"), "{reason}");
            }
            other => panic!("expected a contained exception, got {other:?}"),
        }
    }

    #[test]
    fn a_panic_becomes_a_failure_naming_the_call() {
        let result = guarded("a call", || -> u32 { panic!("a bug") });
        assert_eq!(
            result,
            Err(Failure::Panic {
                call: "a call".into(),
                message: "a bug".into()
            })
        );
    }

    #[test]
    fn missing_images_symbols_and_classes_are_named() {
        let image = load("/nonexistent/Framework.framework/Framework").unwrap_err();
        assert!(image.to_string().contains("/nonexistent"), "{image}");
        let symbol = symbol(c"no_such_function_anywhere").unwrap_err();
        assert!(symbol.to_string().contains("no_such_function_anywhere"));
        assert!(class(c"NoSuchClassAnywhere").is_err());
        assert!(class(c"NSObject").is_ok());
    }
}
