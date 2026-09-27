//! Errors of the adb client.

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

/// Why an adb request failed.
#[derive(Debug)]
pub enum Error {
    /// Nothing accepted a connection at the server's address.
    ServerUnavailable { addr: SocketAddr, source: io::Error },
    /// The adb server, or adbd behind it, answered `FAIL` to `request`.
    Fail { request: String, message: String },
    /// The connection serving `request` failed or passed its deadline.
    Io { request: String, source: io::Error },
    /// The peer answered `request` with bytes the protocol does not allow.
    Protocol { request: String, detail: String },
    /// `request` cannot be sent as given.
    Invalid { request: String, detail: String },
    /// A local file to push could not be read.
    Local { path: PathBuf, source: io::Error },
}

impl Error {
    /// The message of a `FAIL` answer, such as `closed` for a device service
    /// that nothing listens on.
    pub fn fail_message(&self) -> Option<&str> {
        match self {
            Error::Fail { message, .. } => Some(message),
            _ => None,
        }
    }

    /// Whether a connection passed its read or write deadline.
    pub fn is_timeout(&self) -> bool {
        matches!(
            self,
            Error::Io { source, .. }
                if matches!(source.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock)
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ServerUnavailable { addr, source } => write!(
                f,
                "no adb server at {addr} ({source}); start one with `adb start-server`"
            ),
            Error::Fail { request, message } => write!(f, "{request}: {message}"),
            Error::Io { request, source } => write!(f, "{request}: {source}"),
            Error::Protocol { request, detail } => write!(f, "{request}: {detail}"),
            Error::Invalid { request, detail } => write!(f, "{request}: {detail}"),
            Error::Local { path, source } => write!(f, "reading {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::ServerUnavailable { source, .. }
            | Error::Io { source, .. }
            | Error::Local { source, .. } => Some(source),
            _ => None,
        }
    }
}
