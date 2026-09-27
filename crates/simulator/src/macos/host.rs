//! The host's versions: the CoreSimulator that vends `SimDevice`, which is
//! what the input transport depends on (idb's
//! `SimulatorHIDTransportSelection`, spike S3), and the selected Xcode.

use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSBundle, NSData, NSDictionary, NSPropertyListMutabilityOptions, NSPropertyListSerialization,
    NSString,
};

use crate::macos::coresim::developer_dir;
use crate::macos::objc::{self, guarded};
use crate::select::HostVersions;

/// An Xcode's marketing version and build, such as 27.0 and 27A266a.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xcode {
    pub version: String,
    pub build: String,
}

/// The versions of this host, read without starting any process. Loads
/// CoreSimulator first, since its version comes from the class it vends.
pub fn detect() -> HostVersions {
    let _ = objc::load_core_simulator();
    HostVersions {
        core_simulator: core_simulator(),
        xcode: xcode(&developer_dir()).map(|xcode| format!("{} ({})", xcode.version, xcode.build)),
    }
}

/// `CFBundleVersion` of the bundle `SimDevice` comes from. The Xcode
/// installer replaces this system framework, so it, not the selected Xcode,
/// decides the transport.
pub fn core_simulator() -> Option<String> {
    let class = objc::class(c"SimDevice").ok()?;
    guarded("+[NSBundle bundleForClass:]", || {
        // SAFETY: a registered class is all this asks for.
        let bundle = unsafe { NSBundle::bundleForClass(class) };
        let version = bundle.objectForInfoDictionaryKey(&NSString::from_str("CFBundleVersion"))?;
        string_value(&version)
    })
    .ok()
    .flatten()
}

/// The Xcode that `developer_dir` belongs to, from its `version.plist`.
pub fn xcode(developer_dir: &Path) -> Option<Xcode> {
    let plist = read_plist(&developer_dir.join("../version.plist"))?;
    let value = |key: &str| {
        guarded("-[NSDictionary objectForKey:]", || {
            plist
                .objectForKey(&NSString::from_str(key))
                .and_then(|value| string_value(&value))
        })
        .ok()
        .flatten()
    };
    Some(Xcode {
        version: value("CFBundleShortVersionString")?,
        build: value("ProductBuildVersion").unwrap_or_default(),
    })
}

/// A property list file's top-level dictionary.
fn read_plist(path: &Path) -> Option<Retained<NSDictionary<NSString, AnyObject>>> {
    let bytes = std::fs::read(path).ok()?;
    let data = NSData::with_bytes(&bytes);
    let plist = guarded(
        "+[NSPropertyListSerialization propertyListWithData:options:format:error:]",
        || {
            // SAFETY: a null format pointer is allowed.
            unsafe {
                NSPropertyListSerialization::propertyListWithData_options_format_error(
                    &data,
                    NSPropertyListMutabilityOptions::Immutable,
                    std::ptr::null_mut(),
                )
            }
        },
    )
    .ok()?
    .ok()?;
    if !objc::is_kind_of(&plist, c"NSDictionary") {
        return None;
    }
    // SAFETY: just checked that it is a dictionary; plist keys are strings.
    Some(unsafe { Retained::cast_unchecked::<NSDictionary<NSString, AnyObject>>(plist) })
}

fn string_value(value: &AnyObject) -> Option<String> {
    objc::is_kind_of(value, c"NSString").then(|| {
        // SAFETY: just checked that it is an NSString.
        unsafe { &*(value as *const AnyObject).cast::<NSString>() }.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_loads_core_simulator_and_reads_its_version() {
        let version = detect()
            .core_simulator
            .expect("SimDevice's bundle has a version");
        assert!(
            crate::version::Version::parse(&version).is_some(),
            "{version}"
        );
    }

    #[test]
    fn the_selected_xcode_has_a_version_and_build() {
        let xcode = xcode(&developer_dir()).expect("version.plist is readable");
        assert!(
            crate::version::Version::parse(&xcode.version).is_some(),
            "{xcode:?}"
        );
        assert!(!xcode.build.is_empty(), "{xcode:?}");
    }

    #[test]
    fn a_directory_without_an_xcode_has_no_version() {
        assert_eq!(xcode(Path::new("/nonexistent/Contents/Developer")), None);
    }
}
