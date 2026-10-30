//! Who a process is: its executable, its bundle and whether it is system UI.
//! A Simulator's processes run on the host's kernel, so this answers the
//! same inside the guest and on the host.

use std::ffi::{c_int, c_void};
use std::path::{Path, PathBuf};

use objc2::rc::autoreleasepool;
use objc2_foundation::{NSBundle, NSString};

use crate::elements::{Owner, SPRINGBOARD};

/// Where a runtime keeps the processes that draw system UI, SpringBoard
/// among them; applications live in `/Applications` or the device's data.
const CORE_SERVICES: &str = "/System/Library/CoreServices/";

/// The size of the buffer `proc_pidpath` fills: libproc's
/// `PROC_PIDPATHINFO_MAXSIZE`, `4 * MAXPATHLEN` in `sys/proc_info.h`, which
/// is 4096 bytes. libproc is macOS's, so libc declares the constant for macOS
/// alone and the iOS SDKs ship no header with it; it is declared here for
/// the guest and the host alike.
const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;

unsafe extern "C" {
    /// libproc's call that writes the executable path of process `pid` into
    /// `buffer`, at most `buffersize` bytes, and returns its length, or 0 or
    /// less when it fails, with libc's macOS signature. libproc is macOS's,
    /// so libc declares the call for macOS alone. A Simulator's processes run
    /// on the host's kernel and the iPhoneSimulator SDK's libSystem exports
    /// the call, so it is declared here and answers inside the guest as it
    /// does on the host.
    fn proc_pidpath(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;
}

/// The executable `pid` runs.
pub fn executable(pid: i32) -> Option<PathBuf> {
    let mut buffer = vec![0u8; PROC_PIDPATHINFO_MAXSIZE];
    // SAFETY: the buffer holds as many bytes as the call is told.
    let length = unsafe {
        proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast::<c_void>(),
            buffer.len() as u32,
        )
    };
    if length <= 0 {
        return None;
    }
    buffer.truncate(length as usize);
    String::from_utf8(buffer).ok().map(PathBuf::from)
}

/// The process `pid`: its bundle identifier and name when it runs from an
/// application bundle, and whether it is system UI.
pub fn owner(pid: i32) -> Owner {
    let path = executable(pid);
    let (bundle_id, bundle_name) = path.as_deref().and_then(bundle_of).unwrap_or((None, None));
    let name = bundle_name
        .or_else(|| {
            path.as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let system = bundle_id.as_deref() == Some(SPRINGBOARD)
        || path
            .as_deref()
            .is_some_and(|path| path.to_string_lossy().contains(CORE_SERVICES));
    Owner {
        pid,
        bundle_id,
        name,
        system,
    }
}

/// The identifier and display name of the application bundle holding
/// `executable`.
fn bundle_of(executable: &Path) -> Option<(Option<String>, Option<String>)> {
    let app = executable
        .ancestors()
        .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))?;
    autoreleasepool(|_| {
        let bundle = NSBundle::bundleWithPath(&NSString::from_str(&app.to_string_lossy()))?;
        let identifier = bundle.bundleIdentifier().map(|id| id.to_string());
        let name = ["CFBundleDisplayName", "CFBundleName"]
            .iter()
            .find_map(|key| {
                bundle
                    .objectForInfoDictionaryKey(&NSString::from_str(key))
                    .and_then(|value| value.downcast::<NSString>().ok())
                    .map(|value| value.to_string())
                    .filter(|value| !value.is_empty())
            });
        Some((identifier, name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_has_an_executable_and_a_name() {
        let pid = std::process::id() as i32;
        let path = executable(pid).expect("this process's executable");
        assert!(path.is_absolute(), "{}", path.display());
        let owner = owner(pid);
        assert_eq!(owner.pid, pid);
        assert!(!owner.name.is_empty());
        assert!(!owner.system);
    }

    #[test]
    fn a_process_that_does_not_exist_has_no_executable() {
        assert_eq!(executable(-1), None);
        assert_eq!(owner(-1).bundle_id, None);
    }

    #[test]
    fn a_bundle_is_found_above_its_executable() {
        let finder = Path::new("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder");
        let (identifier, name) = bundle_of(finder).expect("the Finder bundle");
        assert_eq!(identifier.as_deref(), Some("com.apple.finder"));
        assert!(name.is_some());
    }
}
