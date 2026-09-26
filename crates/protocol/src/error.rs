//! Structured errors (spec: Structured errors): a stable snake_case code, a
//! human-readable message, and whether retrying the same request is meaningful.

use core::fmt;

use minicbor::{Decode, Encode};

use crate::capability::Capabilities;
use crate::message::Method;

/// An error carried in a response or welcome, or produced by the host
/// session on the agent's behalf (a missing capability, a passed deadline,
/// a lost connection).
///
/// On the wire: `{0: code, 1: message, 2: retryable}`.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Error {
    /// Stable snake_case code; see [`code`].
    #[n(0)]
    pub code: String,
    /// Human-readable description, including any hint for the user.
    #[n(1)]
    pub message: String,
    /// Whether sending the same request again can succeed.
    #[n(2)]
    pub retryable: bool,
}

/// The error codes this protocol version defines. Agents may send others;
/// [`retryable`] returns `None` for those.
pub mod code {
    /// The agent does not implement the method. Not retryable.
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    /// The parameters are missing or malformed. Not retryable.
    pub const INVALID_PARAMS: &str = "invalid_params";
    /// The method needs a capability the agent did not advertise; the host
    /// refused to send it. Not retryable.
    pub const CAPABILITY_MISSING: &str = "capability_missing";
    /// The request's deadline passed before the work finished. Retryable: the
    /// agent stays responsive and the same request can finish another time.
    pub const DEADLINE_EXCEEDED: &str = "deadline_exceeded";
    /// The agent is still finishing an abandoned request. Retryable.
    pub const BUSY: &str = "busy";
    /// The foreground window changed during acquisition. Retryable.
    pub const WINDOW_CHANGED: &str = "window_changed";
    /// The message exceeds the negotiated maximum frame size. Not retryable.
    pub const FRAME_TOO_LARGE: &str = "frame_too_large";
    /// The two sides share no protocol version. Not retryable.
    pub const VERSION_MISMATCH: &str = "version_mismatch";
    /// The agent's hello carried the wrong per-launch token. Not retryable.
    pub const TOKEN_MISMATCH: &str = "token_mismatch";
    /// The negotiated maximum frame size is below the protocol minimum. Not
    /// retryable.
    pub const FRAME_LIMIT_TOO_SMALL: &str = "frame_limit_too_small";
    /// The handshake has not completed yet. Retryable once it has.
    pub const NOT_READY: &str = "not_ready";
    /// The connection to the agent is gone. Not retryable on this connection;
    /// the message carries a reconnect hint.
    pub const DISCONNECTED: &str = "disconnected";
    /// A malformed or unexpected message. Not retryable.
    pub const PROTOCOL_ERROR: &str = "protocol_error";
    /// An unexpected failure inside the agent. Not retryable.
    pub const INTERNAL: &str = "internal";

    /// Every code above.
    pub const ALL: &[&str] = &[
        UNKNOWN_METHOD,
        INVALID_PARAMS,
        CAPABILITY_MISSING,
        DEADLINE_EXCEEDED,
        BUSY,
        WINDOW_CHANGED,
        FRAME_TOO_LARGE,
        VERSION_MISMATCH,
        TOKEN_MISMATCH,
        FRAME_LIMIT_TOO_SMALL,
        NOT_READY,
        DISCONNECTED,
        PROTOCOL_ERROR,
        INTERNAL,
    ];
}

/// Whether retrying is meaningful for a code this protocol version defines,
/// or `None` for any other code.
pub fn retryable(code: &str) -> Option<bool> {
    match code {
        code::DEADLINE_EXCEEDED | code::BUSY | code::WINDOW_CHANGED | code::NOT_READY => Some(true),
        code::UNKNOWN_METHOD
        | code::INVALID_PARAMS
        | code::CAPABILITY_MISSING
        | code::FRAME_TOO_LARGE
        | code::VERSION_MISMATCH
        | code::TOKEN_MISMATCH
        | code::FRAME_LIMIT_TOO_SMALL
        | code::DISCONNECTED
        | code::PROTOCOL_ERROR
        | code::INTERNAL => Some(false),
        _ => None,
    }
}

impl Error {
    /// An error with `code`, retryable as [`retryable`] says (not retryable
    /// for codes this version does not define).
    pub fn new(code: &str, message: impl Into<String>) -> Error {
        Error {
            code: code.to_owned(),
            message: message.into(),
            retryable: retryable(code).unwrap_or(false),
        }
    }

    /// The agent does not implement `method`.
    pub fn unknown_method(method: Method) -> Error {
        Error::new(
            code::UNKNOWN_METHOD,
            format!("{method} is not a method this agent implements"),
        )
    }

    /// `method` needs `missing`, which the agent did not advertise.
    pub fn capability_missing(method: Method, missing: Capabilities) -> Error {
        Error::new(
            code::CAPABILITY_MISSING,
            format!(
                "the agent did not advertise {} ({missing}), which {method} needs",
                missing.label()
            ),
        )
    }

    /// Request `id` for `method` did not finish within `deadline_ms`.
    pub fn deadline_exceeded(method: Method, id: u32, deadline_ms: u32) -> Error {
        Error::new(
            code::DEADLINE_EXCEEDED,
            format!(
                "the {method} request {id} did not finish within its {deadline_ms} ms deadline"
            ),
        )
    }

    /// The `method` message for request `id` is `size` bytes, over `limit`.
    pub fn frame_too_large(method: Method, id: u32, size: u64, limit: u32) -> Error {
        Error::new(
            code::FRAME_TOO_LARGE,
            format!(
                "the {method} message for request {id} is {size} bytes, over the negotiated maximum frame size of {limit} bytes"
            ),
        )
    }

    /// The connection is gone because of `reason`; carries the reconnect hint.
    pub fn disconnected(reason: impl fmt::Display) -> Error {
        Error::new(
            code::DISCONNECTED,
            format!(
                "the agent is disconnected ({reason}); reconnect to the target, which restarts or redeploys the agent"
            ),
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
