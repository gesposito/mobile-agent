//! Errors the mirror reports, each with a stable snake_case code.

use core::fmt;

use crate::backend::BackendError;
use crate::refs::Ref;
use crate::snapshot::Role;

/// Codes of the errors the mirror raises itself. Backend errors keep the
/// driver's code.
pub mod code {
    /// The ref's element no longer exists, or the ref was issued before the
    /// replica was rebuilt; the message says to take a new snapshot.
    pub const STALE_REF: &str = "stale_ref";
    /// The input is not a ref this mirror issued.
    pub const UNKNOWN_REF: &str = "unknown_ref";
    /// The target's connection is gone. The same code as the protocol's.
    pub const DISCONNECTED: &str = protocol::code::DISCONNECTED;
}

/// Why a ref is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaleCause {
    /// Its element left the screen.
    Removed,
}

/// Why a mirror operation failed.
#[derive(Clone, Debug, PartialEq)]
pub enum MirrorError {
    /// A ref whose element is gone.
    StaleRef {
        reference: Ref,
        cause: StaleCause,
        /// The element's role and name when the ref was issued.
        was: Option<(Role, String)>,
    },
    /// Not a ref this mirror issued.
    UnknownRef(String),
    /// The driver failed; its error is passed through.
    Backend(BackendError),
    /// The target is gone.
    Disconnected(String),
}

impl MirrorError {
    /// The stable machine-readable code.
    pub fn code(&self) -> &str {
        match self {
            MirrorError::StaleRef { .. } => code::STALE_REF,
            MirrorError::UnknownRef(_) => code::UNKNOWN_REF,
            MirrorError::Backend(e) => &e.code,
            MirrorError::Disconnected(_) => code::DISCONNECTED,
        }
    }

    /// Whether the same operation can succeed when repeated.
    pub fn retryable(&self) -> bool {
        match self {
            MirrorError::Backend(e) => e.retryable,
            MirrorError::StaleRef { .. }
            | MirrorError::UnknownRef(_)
            | MirrorError::Disconnected(_) => false,
        }
    }
}

impl fmt::Display for MirrorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MirrorError::StaleRef {
                reference,
                cause,
                was,
            } => {
                let what = match was {
                    Some((role, name)) if !name.is_empty() => format!("{role} {name:?}"),
                    Some((role, _)) => role.to_string(),
                    None => "element".to_owned(),
                };
                let why = match cause {
                    StaleCause::Removed => {
                        format!("the {what} it named is no longer on the screen")
                    }
                };
                write!(
                    f,
                    "{reference} is stale: {why}. Take a new snapshot and use its refs"
                )
            }
            MirrorError::UnknownRef(input) => write!(
                f,
                "{input} is not a ref from this target's snapshots; take a snapshot and use the refs it prints, such as @e1"
            ),
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
