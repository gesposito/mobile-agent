//! The errors the driver returns, each with a stable code in the
//! repository's snake_case style.

use std::fmt;

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
        }
    }

    /// Whether repeating the same request can succeed without a person
    /// changing anything: none of these.
    pub fn retryable(&self) -> bool {
        false
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
