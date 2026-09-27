//! Messages to ROCKit remote proxies, from spike S2.
//!
//! Every object CoreSimDeviceIO vends through `-[SimDeviceIOClient ioPorts]`
//! (IO ports, display descriptors, descriptor states) is an instance of a
//! generated `ROCKRemoteProxy-<uuid>-ROCKImpersonateable-<protocol>` class
//! that answers its protocol's methods by message forwarding. objc2's
//! debug-build check of `msg_send!` looks the method up on the receiver's
//! class, finds nothing, and panics with "method not found"; release builds
//! skip that check. `send` calls `objc_msgSend` directly instead and, in debug
//! builds, checks the Rust signature against the proxy's own
//! `-methodSignatureForSelector:`, the signature forwarding marshals by.
//!
//! The raw call goes through objc2's `EncodeArguments::__invoke`, public but
//! hidden from its docs: the same call `msg_send!` makes after its check. Its
//! `C-unwind` ABI lets an exception reach `objc::guarded`.

use std::ffi::CStr;

use objc2::encode::{EncodeArguments, EncodeReturn, Encoding};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2_foundation::NSMethodSignature;

/// Sends `sel` with `args` to `receiver` without objc2's class-based check.
///
/// # Safety
/// As for `msg_send!`: `A` and `R` must match the method's signature, which
/// debug builds assert against the receiver's `-methodSignatureForSelector:`.
pub unsafe fn send<A: EncodeArguments, R: EncodeReturn>(
    receiver: &AnyObject,
    sel: Sel,
    args: A,
) -> R {
    if cfg!(debug_assertions)
        && let Err(mismatch) = check_signature(receiver, sel, A::ENCODINGS, &R::ENCODING_RETURN)
    {
        panic!(
            "forwarded send to {}: {mismatch}",
            receiver.class().name().to_string_lossy()
        );
    }
    let receiver = (receiver as *const AnyObject).cast_mut();
    // SAFETY: the caller guarantees the signature; objc_msgSend is the
    // dispatch function for non-struct returns on arm64.
    unsafe { A::__invoke(objc2::ffi::objc_msgSend, receiver, sel, args) }
}

/// Sends a message that returns an object, retaining the result.
///
/// # Safety
/// As for `send`, with an object-returning method that follows the default
/// (autoreleased return) convention.
pub unsafe fn send_object<A: EncodeArguments>(
    receiver: &AnyObject,
    sel: Sel,
    args: A,
) -> Option<Retained<AnyObject>> {
    // SAFETY: forwarded to the caller.
    let object: *mut AnyObject = unsafe { send(receiver, sel, args) };
    // SAFETY: a +0 (autoreleased or unowned) object pointer or nil.
    unsafe { Retained::retain_autoreleased(object) }
}

/// The proxy's own signature for `sel` as (return type, argument types),
/// without the receiver and selector.
fn signature(receiver: &AnyObject, sel: Sel) -> Option<(String, Vec<String>)> {
    // SAFETY: NSObject and NSProxy both implement this, and ROCKit's proxies
    // answer it from the protocol they impersonate.
    let signature: Option<Retained<NSMethodSignature>> =
        unsafe { msg_send![receiver, methodSignatureForSelector: sel] };
    let signature = signature?;
    let text = |ptr: std::ptr::NonNull<std::ffi::c_char>| {
        // SAFETY: NSMethodSignature returns NUL-terminated type strings.
        unsafe { CStr::from_ptr(ptr.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    };
    let returns = text(signature.methodReturnType());
    let arguments = (2..signature.numberOfArguments())
        .map(|index| text(signature.getArgumentTypeAtIndex(index)))
        .collect();
    Some((returns, arguments))
}

fn check_signature(
    receiver: &AnyObject,
    sel: Sel,
    args: &[Encoding],
    returns: &Encoding,
) -> Result<(), String> {
    let name = sel.name().to_string_lossy();
    let (expected_return, expected_args) =
        signature(receiver, sel).ok_or_else(|| format!("-{name} has no method signature"))?;
    if !returns.equivalent_to_str(&expected_return) {
        return Err(format!(
            "-{name} returns {expected_return}, Rust expects {returns}"
        ));
    }
    if expected_args.len() != args.len() {
        return Err(format!(
            "-{name} takes {} arguments, Rust passes {}",
            expected_args.len(),
            args.len()
        ));
    }
    for (index, (expected, actual)) in expected_args.iter().zip(args).enumerate() {
        if !actual.equivalent_to_str(expected) {
            return Err(format!(
                "-{name} argument {index} is {expected}, Rust passes {actual}"
            ));
        }
    }
    Ok(())
}
