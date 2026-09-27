//! Errors the mirror reports, each with a stable snake_case code.

use core::fmt;

use crate::backend::BackendError;

/// Codes of the errors the mirror raises itself. Backend errors keep the
/// driver's code.
pub mod code {
    /// The target's connection is gone. The same code as the protocol's.
    pub const DISCONNECTED: &str = protocol::code::DISCONNECTED;
}

/// Why a mirror operation failed.
#[derive(Clone, Debug, PartialEq)]
pub enum MirrorError {
    /// The driver failed; its error is passed through.
    Backend(BackendError),
    /// The target is gone.
    Disconnected(String),
}

impl MirrorError {
    /// The stable machine-readable code.
    pub fn code(&self) -> &str {
        match self {
            MirrorError::Backend(e) => &e.code,
            MirrorError::Disconnected(_) => code::DISCONNECTED,
        }
    }

    /// Whether the same operation can succeed when repeated.
    pub fn retryable(&self) -> bool {
        match self {
            MirrorError::Backend(e) => e.retryable,
            MirrorError::Disconnected(_) => false,
        }
    }
}

impl fmt::Display for MirrorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MirrorError::Backend(e) => write!(f, "{}", e.message),
            MirrorError::Disconnected(why) => write!(
                f,
                "the target is disconnected ({why}); reconnect to it before acting"
            ),
        }
    }
}

impl std::error::Error for MirrorError {}

impl From<BackendError> for MirrorError {
    fn from(e: BackendError) -> MirrorError {
        MirrorError::Backend(e)
    }
}
