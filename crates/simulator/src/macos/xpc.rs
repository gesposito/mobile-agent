//! Hand-written bindings to libxpc, plus the two private `_4sim` functions
//! that turn a guest service's Mach port into a host connection (spike S3).
//! The published XPC crates run bindgen over the public `xpc.h` and bind no
//! `_4sim` symbol, so a few `extern "C"` declarations and three `dlsym`
//! lookups replace a libclang build dependency.

use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::ptr::NonNull;

use block2::{Block, RcBlock};
use dispatch2::DispatchQueue;

use crate::macos::objc;

/// An opaque `xpc_object_t`; every XPC object is reference counted.
pub type Object = *mut c_void;
type Connection = *mut c_void;
type Type = *const c_void;

unsafe extern "C" {
    fn xpc_retain(object: Object) -> Object;
    fn xpc_release(object: Object);
    fn xpc_get_type(object: Object) -> Type;
    fn xpc_copy_description(object: Object) -> *mut c_char;

    fn xpc_dictionary_create_empty() -> Object;
    fn xpc_dictionary_set_string(dictionary: Object, key: *const c_char, value: *const c_char);
    fn xpc_dictionary_set_bool(dictionary: Object, key: *const c_char, value: bool);
    fn xpc_dictionary_set_uint64(dictionary: Object, key: *const c_char, value: u64);
    fn xpc_dictionary_set_double(dictionary: Object, key: *const c_char, value: f64);
    fn xpc_dictionary_set_value(dictionary: Object, key: *const c_char, value: Object);
    fn xpc_dictionary_get_string(dictionary: Object, key: *const c_char) -> *const c_char;

    fn xpc_connection_set_target_queue(connection: Connection, queue: *const c_void);
    fn xpc_connection_set_event_handler(connection: Connection, handler: *mut c_void);
    fn xpc_connection_resume(connection: Connection);
    fn xpc_connection_cancel(connection: Connection);
    fn xpc_connection_send_message(connection: Connection, message: Object);
    fn xpc_connection_send_message_with_reply(
        connection: Connection,
        message: Object,
        queue: *const c_void,
        handler: *mut c_void,
    );
    fn xpc_connection_send_barrier(connection: Connection, barrier: *mut c_void);

    /// `XPC_TYPE_ERROR` is `&_xpc_type_error`.
    static _xpc_type_error: c_void;
    /// `XPC_ERROR_KEY_DESCRIPTION`.
    static _xpc_error_key_description: *const c_char;
}

// The readers only the tests use.
#[cfg(test)]
unsafe extern "C" {
    fn xpc_type_get_name(ty: Type) -> *const c_char;
    fn xpc_dictionary_get_bool(dictionary: Object, key: *const c_char) -> bool;
    fn xpc_dictionary_get_uint64(dictionary: Object, key: *const c_char) -> u64;
    fn xpc_dictionary_get_double(dictionary: Object, key: *const c_char) -> f64;
    fn xpc_dictionary_get_value(dictionary: Object, key: *const c_char) -> Object;
    fn xpc_dictionary_get_count(dictionary: Object) -> usize;
}

fn c(text: &str) -> CString {
    CString::new(text).unwrap_or_else(|_| CString::new(text.replace('\0', "")).expect("no NUL"))
}

/// A retained XPC object.
pub struct Value(Object);

// SAFETY: the dictionaries built here are not mutated once handed over, and
// libxpc's reference counting is thread-safe.
unsafe impl Send for Value {}
// SAFETY: as above; every accessor is a read.
unsafe impl Sync for Value {}

impl Drop for Value {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a live object this value owns one reference to.
        unsafe { xpc_release(self.0) };
    }
}

impl Value {
    /// An empty dictionary.
    pub fn dictionary() -> Value {
        // SAFETY: returns a new +1 dictionary.
        Value(unsafe { xpc_dictionary_create_empty() })
    }

    /// Retains a borrowed object, such as one a handler receives.
    ///
    /// # Safety
    /// `object` must be nil or a live XPC object.
    pub unsafe fn retaining(object: Object) -> Option<Value> {
        NonNull::new(object)?;
        // SAFETY: the caller guarantees `object` is live.
        Some(Value(unsafe { xpc_retain(object) }))
    }

    /// The XPC type's name, such as `dictionary`, `uint64` or `error`.
    #[cfg(test)]
    pub fn type_name(&self) -> String {
        // SAFETY: the object is live; the name is a static string.
        unsafe {
            CStr::from_ptr(xpc_type_get_name(xpc_get_type(self.0)))
                .to_string_lossy()
                .into_owned()
        }
    }

    pub fn is_error(&self) -> bool {
        // SAFETY: the object is live; `_xpc_type_error` is libxpc's own.
        unsafe { xpc_get_type(self.0) == (&raw const _xpc_type_error).cast() }
    }

    /// `xpc_copy_description`, for diagnostics.
    pub fn description(&self) -> String {
        // SAFETY: the object is live; the returned buffer is ours to free.
        unsafe {
            let raw = xpc_copy_description(self.0);
            if raw.is_null() {
                return String::new();
            }
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            libc::free(raw.cast());
            text
        }
    }

    /// An error's description, or the object's whole description.
    pub fn error_reason(&self) -> String {
        // SAFETY: the key is libxpc's own constant.
        let key = unsafe { CStr::from_ptr(_xpc_error_key_description) };
        // SAFETY: the object is live; the result is nil or borrowed.
        let described = unsafe {
            let raw = xpc_dictionary_get_string(self.0, key.as_ptr());
            (!raw.is_null()).then(|| CStr::from_ptr(raw).to_string_lossy().into_owned())
        };
        described.unwrap_or_else(|| self.description())
    }

    pub fn set_string(&self, key: &str, value: &str) {
        let (key, value) = (c(key), c(value));
        // SAFETY: both strings outlive the call, which copies them.
        unsafe { xpc_dictionary_set_string(self.0, key.as_ptr(), value.as_ptr()) };
    }

    pub fn set_bool(&self, key: &str, value: bool) {
        let key = c(key);
        // SAFETY: `key` outlives the call.
        unsafe { xpc_dictionary_set_bool(self.0, key.as_ptr(), value) };
    }

    pub fn set_uint64(&self, key: &str, value: u64) {
        let key = c(key);
        // SAFETY: `key` outlives the call.
        unsafe { xpc_dictionary_set_uint64(self.0, key.as_ptr(), value) };
    }

    pub fn set_double(&self, key: &str, value: f64) {
        let key = c(key);
        // SAFETY: `key` outlives the call.
        unsafe { xpc_dictionary_set_double(self.0, key.as_ptr(), value) };
    }

    pub fn set_value(&self, key: &str, value: &Value) {
        let key = c(key);
        // SAFETY: `key` outlives the call; the dictionary retains `value`.
        unsafe { xpc_dictionary_set_value(self.0, key.as_ptr(), value.0) };
    }

    #[cfg(test)]
    pub fn string(&self, key: &str) -> Option<String> {
        let key = c(key);
        // SAFETY: `key` outlives the call; the result is nil or borrowed.
        unsafe {
            let raw = xpc_dictionary_get_string(self.0, key.as_ptr());
            (!raw.is_null()).then(|| CStr::from_ptr(raw).to_string_lossy().into_owned())
        }
    }

    #[cfg(test)]
    pub fn bool(&self, key: &str) -> bool {
        let key = c(key);
        // SAFETY: `key` outlives the call.
        unsafe { xpc_dictionary_get_bool(self.0, key.as_ptr()) }
    }

    /// The `uint64` at `key`; `None` when absent or of another type, which
    /// libxpc would both read as 0.
    #[cfg(test)]
    pub fn uint64(&self, key: &str) -> Option<u64> {
        if self.value(key)?.type_name() != "uint64" {
            return None;
        }
        let key = c(key);
        // SAFETY: `key` outlives the call.
        Some(unsafe { xpc_dictionary_get_uint64(self.0, key.as_ptr()) })
    }

    #[cfg(test)]
    pub fn double(&self, key: &str) -> Option<f64> {
        if self.value(key)?.type_name() != "double" {
            return None;
        }
        let key = c(key);
        // SAFETY: `key` outlives the call.
        Some(unsafe { xpc_dictionary_get_double(self.0, key.as_ptr()) })
    }

    /// The object at `key`, retained.
    #[cfg(test)]
    pub fn value(&self, key: &str) -> Option<Value> {
        let key = c(key);
        // SAFETY: `key` outlives the call; the result is nil or borrowed and
        // `retaining` takes its own reference.
        unsafe { Value::retaining(xpc_dictionary_get_value(self.0, key.as_ptr())) }
    }

    #[cfg(test)]
    pub fn count(&self) -> usize {
        // SAFETY: the object is live; 0 for a non-dictionary.
        unsafe { xpc_dictionary_get_count(self.0) }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description())
    }
}

/// The private libxpc functions that connect a host process to a guest
/// service, resolved with `dlsym` as idb and iosef do, so a host without
/// them reports that instead of failing to launch.
#[derive(Clone, Copy)]
pub struct Sim4Symbols {
    endpoint_create_mach_port: unsafe extern "C" fn(u32, u64, u64) -> Object,
    connection_create_from_endpoint: unsafe extern "C" fn(Object) -> Connection,
    connection_enable_sim2host: unsafe extern "C" fn(Connection),
}

impl Sim4Symbols {
    pub fn resolve() -> Result<Sim4Symbols, String> {
        let find =
            |name: &str| objc::symbol(name).ok_or_else(|| format!("dlsym({name}) found nothing"));
        let endpoint = find("xpc_endpoint_create_mach_port_4sim")?;
        let connection = find("xpc_connection_create_from_endpoint")?;
        let sim2host = find("xpc_connection_enable_sim2host_4sim")?;
        // SAFETY: the prototypes idb and iosef use and spike S3 verified:
        // `xpc_endpoint_create_mach_port_4sim(mach_port_t, uint64_t,
        // uint64_t)`, `xpc_connection_create_from_endpoint(xpc_endpoint_t)`
        // and `xpc_connection_enable_sim2host_4sim(xpc_connection_t)`.
        unsafe {
            Ok(Sim4Symbols {
                endpoint_create_mach_port: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(u32, u64, u64) -> Object,
                >(endpoint),
                connection_create_from_endpoint: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(Object) -> Connection,
                >(connection),
                connection_enable_sim2host: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(Connection),
                >(sim2host),
            })
        }
    }

    /// An unresumed connection to the guest service behind `port`, marked
    /// simulator-to-host, without which the service sees the peer connect
    /// but never receives a payload. Consumes the port's send right.
    pub fn connect(&self, port: u32) -> Option<RawConnection> {
        // SAFETY: a port from `-[SimDevice lookup:error:]`; the trailing
        // arguments are 0 as in idb and iosef. The result is +1.
        let endpoint = unsafe { (self.endpoint_create_mach_port)(port, 0, 0) };
        let endpoint = Value(NonNull::new(endpoint)?.as_ptr());
        // SAFETY: a live endpoint; the result is +1.
        let connection = unsafe { (self.connection_create_from_endpoint)(endpoint.0) };
        NonNull::new(connection)?;
        // SAFETY: a fresh, unresumed connection.
        unsafe { (self.connection_enable_sim2host)(connection) };
        Some(RawConnection(connection))
    }
}

/// An `xpc_connection_t`, cancelled and released on drop.
pub struct RawConnection(Connection);

// SAFETY: XPC connections support concurrent sending and cancellation.
unsafe impl Send for RawConnection {}
// SAFETY: as above.
unsafe impl Sync for RawConnection {}

impl Drop for RawConnection {
    fn drop(&mut self) {
        // SAFETY: the connection is live. XPC keeps a suspended connection
        // alive, so cancel precedes release; cancelling twice is allowed.
        unsafe {
            xpc_connection_cancel(self.0);
            xpc_release(self.0);
        }
    }
}

impl RawConnection {
    pub fn set_target_queue(&self, queue: &DispatchQueue) {
        // SAFETY: the connection is unresumed and the queue outlives it.
        unsafe { xpc_connection_set_target_queue(self.0, (queue as *const DispatchQueue).cast()) };
    }

    /// Installs the event handler, which libxpc copies.
    pub fn set_event_handler(&self, handler: &Block<dyn Fn(Object)>) {
        // SAFETY: a live block of the right signature; libxpc copies it.
        unsafe {
            xpc_connection_set_event_handler(self.0, (handler as *const Block<_>).cast_mut().cast())
        };
    }

    pub fn resume(&self) {
        // SAFETY: the connection is live and has its handler.
        unsafe { xpc_connection_resume(self.0) };
    }

    pub fn cancel(&self) {
        // SAFETY: the connection is live; cancelling twice is allowed.
        unsafe { xpc_connection_cancel(self.0) };
    }

    /// Hands `message` to XPC: returning means XPC took it, not that the
    /// peer read it.
    pub fn send(&self, message: &Value) {
        // SAFETY: both objects are live; XPC retains what it needs.
        unsafe { xpc_connection_send_message(self.0, message.0) };
    }

    /// Sends `message` and runs `reply` on `queue` with the peer's answer,
    /// or with an error object when XPC answers instead of the peer.
    pub fn send_with_reply(
        &self,
        message: &Value,
        queue: &DispatchQueue,
        reply: impl Fn(Object) + 'static,
    ) {
        let handler = RcBlock::new(reply);
        // SAFETY: every argument is live; libxpc copies the block.
        unsafe {
            xpc_connection_send_message_with_reply(
                self.0,
                message.0,
                (queue as *const DispatchQueue).cast(),
                (&*handler as *const Block<_>).cast_mut().cast(),
            )
        };
    }

    /// Runs `barrier` once XPC has sent everything sent before this call.
    pub fn send_barrier(&self, barrier: impl Fn() + 'static) {
        let handler = RcBlock::new(barrier);
        // SAFETY: the connection is live; libxpc copies the block.
        unsafe {
            xpc_connection_send_barrier(self.0, (&*handler as *const Block<_>).cast_mut().cast())
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dictionary_keeps_its_scalars_and_their_types() {
        let dictionary = Value::dictionary();
        dictionary.set_string("messageType", "IndigoDigitizerEvent");
        dictionary.set_bool("isBarrier", true);
        dictionary.set_uint64("state", 2);
        dictionary.set_double("x", 0.25);
        assert_eq!(dictionary.type_name(), "dictionary");
        assert_eq!(dictionary.count(), 4);
        assert_eq!(
            dictionary.string("messageType").as_deref(),
            Some("IndigoDigitizerEvent")
        );
        assert!(dictionary.bool("isBarrier"));
        assert_eq!(dictionary.uint64("state"), Some(2));
        assert_eq!(dictionary.double("x"), Some(0.25));
        assert_eq!(dictionary.uint64("x"), None, "a double is not a uint64");
        assert!(dictionary.value("missing").is_none());
        assert!(!dictionary.is_error());
    }

    #[test]
    fn the_sim4_symbols_resolve_on_this_host() {
        Sim4Symbols::resolve().expect("libxpc exports the _4sim functions");
    }
}
