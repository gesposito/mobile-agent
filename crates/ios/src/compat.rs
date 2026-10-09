//! Which iOS versions the driver supports, and the compatibility table
//! `doctor` prints (spec `ios-device-driver`, "Compatibility statement").
//!
//! Every private or undocumented interface the driver uses on the device is
//! a [`CallSite`] with where it was validated: iOS 26.6.2 on an iPhone SE
//! (3rd generation) under Xcode 27.0, the only device and version this change
//! measures. A device outside the supported range is refused with its version
//! named ([`crate::Readiness::UnsupportedIos`]).

use crate::version::Version;

/// The first supported iOS: lockdown's `CoreDeviceProxy` carries the
/// CoreDevice tunnel over usbmuxd from iOS 17.4; before it the tunnel needs
/// RemotePairing over the USB Ethernet link, which this driver does not
/// implement (research note, "The CoreDevice tunnel, RemoteXPC and RSD").
pub const FIRST_SUPPORTED_IOS: &str = "17.4";
/// The first iOS past the supported range: iOS 27 drops `dtuhidd`'s reports
/// unless a display stream runs, a path gated for later (design, Open
/// Questions).
pub const FIRST_UNSUPPORTED_IOS: &str = "27";
/// Where the driver and its tests ran.
pub const VALIDATED_IOS: &str = "26.6.2";
pub const VALIDATED_XCODE: &str = "27.0";
/// The `idevice` crate the driver is built on, pinned with `=` as its README
/// asks: breaking changes come with every point release before 0.2.
pub const IDEVICE: &str = "0.1.68";
/// Where the compatibility notes are.
pub const NOTES: &str = "the Compatibility section of crates/ios/README.md";

/// Whether the driver supports a device running iOS `version`.
pub fn supported_ios(version: &str) -> bool {
    let _ = (version, Version::parse);
    todo!()
}

/// The supported range in words.
pub fn supported_range() -> String {
    format!(
        "iOS {FIRST_SUPPORTED_IOS} up to but not including {FIRST_UNSUPPORTED_IOS}, validated on {VALIDATED_IOS}"
    )
}

/// A private or undocumented interface the driver uses, and where it was
/// validated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallSite {
    /// The call, message or service, as the device names it.
    pub call: &'static str,
    /// The service or daemon that answers it.
    pub service: &'static str,
    /// The iOS and Xcode it was exercised on, and by what.
    pub validated_on: &'static str,
    /// What the driver avoids or works around at this call site.
    pub note: &'static str,
}

/// Every interface the driver uses on the device, with where it was
/// validated.
pub fn call_sites() -> Vec<CallSite> {
    todo!()
}

/// The table as aligned text: the supported range, then every call site.
pub fn render() -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ios_17_4_up_to_27_is_supported() {
        for version in ["17.4", "17.4.1", "18.0", "26.6.2", "26.9"] {
            assert!(supported_ios(version), "{version}");
        }
        for version in ["16.7.10", "17.3.1", "17", "27.0", "27", "28.1", "", "beta"] {
            assert!(!supported_ios(version), "{version}");
        }
    }

    #[test]
    fn every_call_site_names_the_ios_and_xcode_it_was_validated_on() {
        let sites = call_sites();
        assert!(!sites.is_empty());
        for site in &sites {
            assert!(
                site.validated_on.contains("iOS 26.6.2")
                    && site.validated_on.contains("Xcode 27.0"),
                "{} names no validated versions: {}",
                site.call,
                site.validated_on
            );
        }
        let calls: Vec<&str> = sites.iter().map(|site| site.call).collect();
        for expected in [
            "ListDevices",
            "ReadPairRecord",
            "DeveloperModeStatus",
            "PasswordProtected",
            "CopyDevices",
        ] {
            assert!(
                calls.iter().any(|call| call.contains(expected)),
                "no call site for {expected}: {calls:?}"
            );
        }
        // The lock comes from lockdown because CoreDevice's lock state is not
        // implemented on iOS 26.6.2 (S9); the table says so at the call.
        let lock = sites
            .iter()
            .find(|site| site.call.contains("PasswordProtected"))
            .expect("the lock's call site");
        assert!(lock.note.contains("getlockstate"), "{}", lock.note);
    }

    #[test]
    fn the_table_renders_the_range_and_every_call_site() {
        let text = render();
        for part in [
            "17.4",
            "27",
            "26.6.2",
            "27.0",
            "0.1.68",
            "ListDevices",
            "PasswordProtected",
        ] {
            assert!(text.contains(part), "{part:?} is missing from:\n{text}");
        }
    }
}
