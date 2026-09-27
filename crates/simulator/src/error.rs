//! The errors the driver returns.

use std::fmt;
use std::time::Duration;

use crate::model::DeviceState;

/// The driver's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// How a call into a private Apple framework failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The call raised an Objective-C exception, which the driver caught.
    Exception { name: String, reason: String },
    /// The call reported an `NSError`.
    NsError {
        domain: String,
        code: i64,
        description: String,
    },
    /// The call returned nil where a value was required.
    Nil,
    /// The call's completion handler did not run within this time.
    NoCompletion(Duration),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Exception { name, reason } => write!(f, "raised {name}: {reason}"),
            Failure::NsError {
                domain,
                code,
                description,
            } => write!(f, "failed with {domain} {code}: {description}"),
            Failure::Nil => f.write_str("returned nil"),
            Failure::NoCompletion(after) => write!(f, "did not complete within {after:?}"),
        }
    }
}

/// Everything the driver can fail with.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// The Simulator driver runs only on macOS.
    PlatformUnsupported,
    /// A private framework, class or symbol could not be loaded.
    Load { what: String, reason: String },
    /// A call into a private framework failed.
    Framework { call: String, failure: Failure },
    /// No device has this UDID or name.
    NotFound { target: String },
    /// Several devices have this name, so it needs a UDID.
    Ambiguous { target: String, matches: usize },
    /// The device is not in the state the operation needs.
    InvalidState {
        target: String,
        state: DeviceState,
        needed: &'static str,
    },
    /// A wait ran out.
    Timeout { what: String, after: Duration },
    /// A file or process operation on the host failed.
    Io { what: String, reason: String },
    /// An argument is unusable.
    InvalidArgument(String),
    /// A frame could not be read from the display or encoded.
    Frame { reason: String },
}

impl Error {
    /// A stable name for the error, for machine-readable output.
    pub fn code(&self) -> &'static str {
        match self {
            Error::PlatformUnsupported => "platform_unsupported",
            Error::Load { .. } => "framework_unavailable",
            Error::Framework { .. } => "framework_error",
            Error::NotFound { .. } => "not_found",
            Error::Ambiguous { .. } => "ambiguous_target",
            Error::InvalidState { .. } => "invalid_state",
            Error::Timeout { .. } => "timeout",
            Error::Io { .. } => "io",
            Error::InvalidArgument(_) => "invalid_argument",
            Error::Frame { .. } => "frame_failed",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::PlatformUnsupported => {
                f.write_str("the iOS Simulator driver needs macOS with Xcode installed")
            }
            Error::Load { what, reason } => write!(f, "could not load {what}: {reason}"),
            Error::Framework { call, failure } => write!(f, "{call} {failure}"),
            Error::NotFound { target } => write!(f, "no Simulator has the UDID or name {target:?}"),
            Error::Ambiguous { target, matches } => {
                write!(f, "{matches} Simulators are named {target:?}; pass a UDID")
            }
            Error::InvalidState {
                target,
                state,
                needed,
            } => write!(
                f,
                "Simulator {target} is {state}, and this needs it {needed}"
            ),
            Error::Timeout { what, after } => write!(f, "{what} did not happen within {after:?}"),
            Error::Io { what, reason } => write!(f, "{what}: {reason}"),
            Error::InvalidArgument(reason) => f.write_str(reason),
            Error::Frame { reason } => write!(f, "frame capture failed: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_framework_failure_names_the_call_and_the_exception() {
        let error = Error::Framework {
            call: "-[SimDevice bootAsyncWithOptions:completionQueue:completionHandler:]".into(),
            failure: Failure::Exception {
                name: "NSInvalidArgumentException".into(),
                reason: "unrecognized selector".into(),
            },
        };
        assert_eq!(error.code(), "framework_error");
        assert_eq!(
            error.to_string(),
            "-[SimDevice bootAsyncWithOptions:completionQueue:completionHandler:] raised \
             NSInvalidArgumentException: unrecognized selector"
        );
    }

    #[test]
    fn an_invalid_state_names_the_device_its_state_and_the_need() {
        let error = Error::InvalidState {
            target: "7B29".into(),
            state: DeviceState::Shutdown,
            needed: "booted",
        };
        assert_eq!(
            error.to_string(),
            "Simulator 7B29 is Shutdown, and this needs it booted"
        );
    }
}
