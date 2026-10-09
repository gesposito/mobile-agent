//! The values the driver takes and returns. Nothing here depends on the
//! `idevice` crate, so these types exist on every platform.

use crate::readiness::Readiness;

/// The environment variable that names the development team the runner is
/// signed for. It stays in the environment, never in a tracked file.
pub const TEAM_VARIABLE: &str = "MOBILE_AGENT_IOS_TEAM";

/// How usbmuxd reaches a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    Usb,
    /// Wi-Fi sync: usbmuxd lists a phone with it on a second time, on the
    /// network. This driver uses USB only (design D18).
    Network,
    /// A connection type this driver does not know, as usbmuxd named it.
    Other(String),
}

/// One entry of usbmuxd's device listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuxEntry {
    pub udid: String,
    /// usbmuxd's number for the device on this connection, which its
    /// `Connect` takes.
    pub device_id: u32,
    pub connection: Connection,
}

/// Whether this Mac holds a pairing record the device accepts: what a
/// lockdown session with it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    /// A lockdown session started with the host's pairing record.
    Trusted,
    /// usbmuxd holds no pairing record for the device: it never trusted
    /// this Mac, or its record was deleted.
    NotPaired,
    /// The device refused the host's pairing record (`InvalidHostID`): its
    /// trust settings were reset, or it was paired with another record.
    Refused,
    /// Lockdown did not answer, or answered something else; the reason.
    Unreachable(String),
}

/// The developer disk image, as the image mounter's `CopyDevices` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskImage {
    /// A developer disk image is mounted: its `DiskImageType` (`Personalized`
    /// from iOS 17, `Developer` before) and mount path.
    Mounted {
        image_type: String,
        mount_path: Option<String>,
    },
    /// The image mounter lists no developer disk image.
    NotMounted,
    /// The image mounter could not be asked; the reason.
    Unreadable(String),
}

/// What the driver read from a device to tell its readiness. A value is
/// `None` where an earlier check stopped the reading: Developer Mode and the
/// lock need a lockdown session, the disk image Developer Mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub trust: Trust,
    /// Lockdown's `DeveloperModeStatus` in the `com.apple.security.mac.amfi`
    /// domain.
    pub developer_mode: Option<bool>,
    pub disk_image: Option<DiskImage>,
    /// Lockdown's `PasswordProtected`, which follows the lock on iOS 26.6.2
    /// (spike S9). CoreDevice's `getlockstate` answers that it is not
    /// implemented there, so the driver never asks it.
    pub password_protected: Option<bool>,
}

impl Probe {
    /// A device whose lockdown session did not start: nothing past the trust
    /// was read.
    pub fn untrusted(trust: Trust) -> Probe {
        Probe {
            trust,
            developer_mode: None,
            disk_image: None,
            password_protected: None,
        }
    }
}

/// One iOS device attached over USB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The device identifier, which every command takes; names are not
    /// unique, so the driver never addresses a device by name.
    pub udid: String,
    /// Lockdown's `DeviceName`, as the owner named the phone.
    pub name: Option<String>,
    /// Lockdown's `ProductType`, such as `iPhone14,6`.
    pub model: Option<String>,
    /// Lockdown's `ProductVersion`, such as `26.6.2`.
    pub ios_version: Option<String>,
    /// Lockdown's `BuildVersion`, such as `23G90`.
    pub build: Option<String>,
    pub probe: Probe,
    /// What stands between the device and a session, or `Ready`.
    pub readiness: Readiness,
}
