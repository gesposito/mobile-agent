//! The errors the driver returns, each with a stable code in the
//! repository's snake_case style.

use std::fmt;
use std::time::Duration;

use crate::model::MountAttempt;
use crate::readiness::Readiness;

/// The driver's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything the driver can fail with.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The physical iOS driver runs only on macOS.
    PlatformUnsupported,
    /// usbmuxd could not be asked for its devices.
    Usbmuxd { reason: String },
    /// No device attached over USB has this identifier. Devices are
    /// addressed by identifier only, since names are not unique.
    NotFound { target: String },
    /// Something stands between the device and the operation; the code is
    /// the state's own.
    NotReady {
        target: String,
        readiness: Readiness,
    },
    /// The driver's runtime or a host command could not start.
    Io { what: String, reason: String },
    /// The device went away, or its tunnel did and a new one could not be
    /// opened: unplugged, rebooted or asleep; the cause.
    Disconnected { target: String, cause: String },
    /// The device's RSD handshake does not list a service the operation
    /// needs.
    ServiceMissing {
        target: String,
        service: String,
        listed: usize,
    },
    /// A wait ran out.
    Timeout { what: String, after: Duration },
    /// The device refused a call, or answered it in a way the driver does
    /// not read: the call and what came back.
    Device { call: String, reason: String },
    /// The path given to install is not a bundle a device takes; why.
    InvalidBundle { path: String, reason: String },
    /// The installation proxy refused the bundle; its reason.
    InstallFailed { bundle_id: String, reason: String },
    /// The application is not installed on the device.
    AppNotInstalled { target: String, bundle_id: String },
    /// DVT refused the launch of an installed application; its reason.
    LaunchFailed { bundle_id: String, reason: String },
    /// The developer disk image is still not mounted after every way of
    /// mounting it was tried; each attempt and why it failed. `offline` when
    /// a failure says this Mac could not reach Apple's signing server, which
    /// the mount needs.
    DiskImage {
        target: String,
        attempts: Vec<MountAttempt>,
        offline: bool,
    },
}

impl Error {
    /// A stable name for the error, for machine-readable output: a readiness
    /// state's own code for [`Error::NotReady`].
    pub fn code(&self) -> &'static str {
        match self {
            Error::PlatformUnsupported => "platform_unsupported",
            Error::Usbmuxd { .. } => "usbmuxd_unavailable",
            Error::NotFound { .. } => "not_found",
            Error::NotReady { readiness, .. } => readiness.code(),
            Error::Io { .. } => "io",
            Error::Disconnected { .. } => "disconnected",
            Error::ServiceMissing { .. } => "service_missing",
            Error::Timeout { .. } => "timeout",
            Error::Device { .. } => "device_error",
            Error::InvalidBundle { .. } => "invalid_bundle",
            Error::InstallFailed { .. } => "install_failed",
            Error::AppNotInstalled { .. } => "app_not_installed",
            Error::LaunchFailed { .. } => "launch_failed",
            Error::DiskImage { .. } => Readiness::DiskImageUnmounted.code(),
        }
    }

    /// Whether repeating the same request can succeed without a person
    /// changing anything: a wait that ran out.
    pub fn retryable(&self) -> bool {
        matches!(self, Error::Timeout { .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::PlatformUnsupported => {
                f.write_str("the physical iOS device driver needs macOS with Xcode installed")
            }
            Error::Usbmuxd { reason } => write!(f, "usbmuxd could not list devices: {reason}"),
            Error::NotFound { target } => write!(
                f,
                "no iOS device attached over USB has the identifier {target:?}; devices are addressed by identifier, which devices lists"
            ),
            Error::NotReady { target, readiness } => {
                write!(
                    f,
                    "iOS device {target} is not ready: {}",
                    readiness.message()
                )
            }
            Error::Io { what, reason } => write!(f, "{what}: {reason}"),
            Error::Disconnected { target, cause } => write!(
                f,
                "iOS device {target} is disconnected ({cause}); plug it in again over USB, unlocked, and connect again"
            ),
            Error::ServiceMissing {
                target,
                service,
                listed,
            } => write!(
                f,
                "iOS device {target} does not list {service} among its {listed} RSD services; it is listed on iOS 26.6.2 with Xcode 27.0's developer disk image mounted"
            ),
            Error::Timeout { what, after } => write!(f, "{what} did not happen within {after:?}"),
            Error::Device { call, reason } => write!(f, "{call} failed on the device: {reason}"),
            Error::InvalidBundle { path, reason } => {
                write!(f, "{path} cannot be installed on a device: {reason}")
            }
            Error::InstallFailed { bundle_id, reason } => {
                write!(f, "the installation proxy refused {bundle_id}: {reason}")
            }
            Error::AppNotInstalled { target, bundle_id } => write!(
                f,
                "{bundle_id} is not installed on iOS device {target}; install its bundle first"
            ),
            Error::LaunchFailed { bundle_id, reason } => {
                write!(f, "DVT refused to launch {bundle_id}: {reason}")
            }
            Error::DiskImage {
                target,
                attempts,
                offline,
            } => {
                write!(
                    f,
                    "the developer disk image could not be mounted on iOS device {target}"
                )?;
                if *offline {
                    f.write_str(
                        "; this Mac seems to be offline, and the mount needs Apple's signing server for the device's ticket",
                    )?;
                }
                for attempt in attempts {
                    write!(
                        f,
                        "; {} after {:.1} s: {}",
                        attempt.path,
                        attempt.elapsed.as_secs_f64(),
                        attempt.failure.as_deref().unwrap_or("mounted")
                    )?;
                }
                if attempts.is_empty() {
                    f.write_str("; no way of mounting it could be tried")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_that_is_not_ready_fails_with_the_states_code_and_step() {
        let error = Error::NotReady {
            target: "placeholder-udid-phone".into(),
            readiness: Readiness::DeveloperModeOff,
        };
        assert_eq!(error.code(), "developer_mode_off");
        let message = error.to_string();
        assert!(message.contains("placeholder-udid-phone"), "{message}");
        assert!(message.contains("Developer Mode"), "{message}");
        let locked = Error::NotReady {
            target: "placeholder-udid-phone".into(),
            readiness: Readiness::Locked,
        };
        assert_eq!(locked.code(), "device_locked");
    }
}
