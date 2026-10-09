//! The Objective-C shim's XCTest calls (`shim/Runner.m`), linked into the
//! same test bundle as this library, behind the [`Shim`] trait, which the
//! tests replace with a fake. On hosts other than iOS, [`Xct`] fails every
//! call, so the library builds and tests there.

/// The processes in front, as XCTest's accessibility client lists them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Foreground {
    /// `-[XCAXClient_iOS activeApplications]`, the foreground first.
    pub active: Vec<i32>,
    /// `-[XCAXClient_iOS systemApplication]`: SpringBoard.
    pub system: Option<i32>,
}

/// A hardware button `-[XCUIDevice pressButton:]` presses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Home,
    VolumeUp,
    VolumeDown,
}

/// The XCTest calls the runner makes. Points are the portrait screen's.
pub trait Shim: Send + 'static {
    /// Pixels per point: `UIScreen`'s scale.
    fn scale(&self) -> f64;

    /// The processes in front.
    fn foreground(&self) -> Foreground;

    /// The bundle identifier of `pid`'s application, from the daemon
    /// session's `requestApplicationSpecifierForPID:reply:`.
    fn bundle(&self, pid: i32) -> Option<String>;

    /// XCTest's snapshot of the application element of `pid`, everything
    /// under it included, as the JSON `xctree` reads.
    fn tree(&self, pid: i32) -> Result<String, String>;

    /// A touch at `point` held for `hold` seconds.
    fn tap(&self, point: (f64, f64), hold: f64) -> Result<(), String>;

    /// A touch from `from` to `to` over `duration` seconds that rests `rest`
    /// seconds at `to` before it lifts, when `rest` is above 0.
    fn swipe(
        &self,
        from: (f64, f64),
        to: (f64, f64),
        duration: f64,
        rest: f64,
    ) -> Result<(), String>;

    fn button(&self, button: Button) -> Result<(), String>;

    /// Types `text` into the element with keyboard focus.
    fn type_text(&self, text: &str) -> Result<(), String>;

    /// Sets the value of the element of `bundle`'s application that has
    /// keyboard focus to `text`.
    fn set_text(&self, bundle: &str, text: &str) -> Result<(), String>;

    /// A PNG of the screen.
    fn screenshot(&self) -> Result<Vec<u8>, String>;
}

/// The shim of the bundle this library is linked into.
#[derive(Clone, Copy, Debug, Default)]
pub struct Xct;

#[cfg(target_os = "ios")]
mod ffi {
    use std::ffi::{c_char, c_void};

    // Every call returns 0 on success; on failure it returns another value
    // and sets `err` to a message the caller frees with `ma_free`.
    unsafe extern "C" {
        pub fn ma_free(p: *mut c_void);
        pub fn ma_screen_scale() -> f64;
        pub fn ma_foreground() -> *mut c_char;
        pub fn ma_bundle_for_pid(pid: i32) -> *mut c_char;
        pub fn ma_tree(pid: i32, json: *mut *mut c_char, err: *mut *mut c_char) -> i32;
        pub fn ma_tap(x: f64, y: f64, hold: f64, err: *mut *mut c_char) -> i32;
        pub fn ma_swipe(
            x1: f64,
            y1: f64,
            x2: f64,
            y2: f64,
            duration: f64,
            rest: f64,
            err: *mut *mut c_char,
        ) -> i32;
        pub fn ma_button(button: i32, err: *mut *mut c_char) -> i32;
        pub fn ma_type(text: *const c_char, frequency: u64, err: *mut *mut c_char) -> i32;
        pub fn ma_set_text(
            bundle: *const c_char,
            text: *const c_char,
            err: *mut *mut c_char,
        ) -> i32;
        pub fn ma_screenshot(out: *mut *mut u8, len: *mut usize, err: *mut *mut c_char) -> i32;
    }
}

#[cfg(target_os = "ios")]
mod calls {
    use std::ffi::{CStr, CString, c_char, c_void};

    use super::ffi;

    /// A string the shim allocated, taken and freed.
    pub fn take(p: *mut c_char) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: the shim returns NUL-terminated strings it allocated.
        let text = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        // SAFETY: allocated by the shim with malloc.
        unsafe { ffi::ma_free(p.cast::<c_void>()) };
        Some(text)
    }

    /// A call's status and error message as a result.
    pub fn status(status: i32, err: *mut c_char) -> Result<(), String> {
        let message = take(err);
        if status == 0 {
            Ok(())
        } else {
            Err(message.unwrap_or_else(|| format!("the shim failed with status {status}")))
        }
    }

    /// `text` as a C string, any NUL left out.
    pub fn c(text: &str) -> CString {
        CString::new(text.replace('\0', "")).expect("no NUL is left")
    }
}

/// How fast `_XCT_sendString:maximumFrequency:completion:` types, characters
/// a second: WebDriverAgent's default, which spike S10 measured.
pub const TYPING_FREQUENCY: u64 = 60;

#[cfg(target_os = "ios")]
impl Shim for Xct {
    fn scale(&self) -> f64 {
        // SAFETY: no arguments.
        unsafe { ffi::ma_screen_scale() }
    }

    fn foreground(&self) -> Foreground {
        // SAFETY: no arguments; the result is the shim's JSON or null.
        let text = calls::take(unsafe { ffi::ma_foreground() }).unwrap_or_default();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let pid = |v: &serde_json::Value| {
            v.as_i64()
                .filter(|pid| *pid > 0)
                .and_then(|pid| i32::try_from(pid).ok())
        };
        Foreground {
            active: value["active"]
                .as_array()
                .map(|pids| pids.iter().filter_map(pid).collect())
                .unwrap_or_default(),
            system: pid(&value["system"]),
        }
    }

    fn bundle(&self, pid: i32) -> Option<String> {
        // SAFETY: a plain integer argument.
        calls::take(unsafe { ffi::ma_bundle_for_pid(pid) })
    }

    fn tree(&self, pid: i32) -> Result<String, String> {
        let (mut json, mut err) = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: valid out pointers.
        let status = unsafe { ffi::ma_tree(pid, &mut json, &mut err) };
        calls::status(status, err)?;
        calls::take(json).ok_or_else(|| "the shim returned no tree".to_owned())
    }

    fn tap(&self, (x, y): (f64, f64), hold: f64) -> Result<(), String> {
        let mut err = std::ptr::null_mut();
        // SAFETY: a valid out pointer.
        let status = unsafe { ffi::ma_tap(x, y, hold, &mut err) };
        calls::status(status, err)
    }

    fn swipe(
        &self,
        (x1, y1): (f64, f64),
        (x2, y2): (f64, f64),
        duration: f64,
        rest: f64,
    ) -> Result<(), String> {
        let mut err = std::ptr::null_mut();
        // SAFETY: a valid out pointer.
        let status = unsafe { ffi::ma_swipe(x1, y1, x2, y2, duration, rest, &mut err) };
        calls::status(status, err)
    }

    fn button(&self, button: Button) -> Result<(), String> {
        // XCUIDeviceButton's values.
        let value = match button {
            Button::Home => 1,
            Button::VolumeUp => 2,
            Button::VolumeDown => 3,
        };
        let mut err = std::ptr::null_mut();
        // SAFETY: a valid out pointer.
        let status = unsafe { ffi::ma_button(value, &mut err) };
        calls::status(status, err)
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        let text = calls::c(text);
        let mut err = std::ptr::null_mut();
        // SAFETY: a valid C string and out pointer.
        let status = unsafe { ffi::ma_type(text.as_ptr(), TYPING_FREQUENCY, &mut err) };
        calls::status(status, err)
    }

    fn set_text(&self, bundle: &str, text: &str) -> Result<(), String> {
        let (bundle, text) = (calls::c(bundle), calls::c(text));
        let mut err = std::ptr::null_mut();
        // SAFETY: valid C strings and out pointer.
        let status = unsafe { ffi::ma_set_text(bundle.as_ptr(), text.as_ptr(), &mut err) };
        calls::status(status, err)
    }

    fn screenshot(&self) -> Result<Vec<u8>, String> {
        let (mut out, mut len, mut err) = (std::ptr::null_mut(), 0usize, std::ptr::null_mut());
        // SAFETY: valid out pointers.
        let status = unsafe { ffi::ma_screenshot(&mut out, &mut len, &mut err) };
        calls::status(status, err)?;
        // SAFETY: the shim malloc'd `len` bytes at `out`.
        let data = unsafe { std::slice::from_raw_parts(out, len) }.to_vec();
        // SAFETY: allocated by the shim with malloc.
        unsafe { ffi::ma_free(out.cast::<std::ffi::c_void>()) };
        Ok(data)
    }
}

#[cfg(not(target_os = "ios"))]
impl Shim for Xct {
    fn scale(&self) -> f64 {
        1.0
    }

    fn foreground(&self) -> Foreground {
        Foreground::default()
    }

    fn bundle(&self, _pid: i32) -> Option<String> {
        None
    }

    fn tree(&self, _pid: i32) -> Result<String, String> {
        Err(UNAVAILABLE.into())
    }

    fn tap(&self, _point: (f64, f64), _hold: f64) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn swipe(&self, _: (f64, f64), _: (f64, f64), _: f64, _: f64) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn button(&self, _button: Button) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn type_text(&self, _text: &str) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn set_text(&self, _bundle: &str, _text: &str) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn screenshot(&self) -> Result<Vec<u8>, String> {
        Err(UNAVAILABLE.into())
    }
}

/// What every call answers off iOS.
#[cfg(not(target_os = "ios"))]
const UNAVAILABLE: &str = "the XCTest shim exists only in the runner on iOS";
