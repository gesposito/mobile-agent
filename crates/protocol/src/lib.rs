//! Wire protocol between the host daemon and device agents (design D4).
//!
//! Every message is one CBOR item in a length-prefixed frame. The crate holds
//! the message schema ([`message`], [`snapshot`], [`event`]), structured errors
//! ([`error`]) and the capability set ([`capability`]); it holds no device code.
//! See `README.md` for the byte-level layout rules the Kotlin agent follows.

#[macro_use]
mod macros;

pub mod capability;
mod cbor;
pub mod error;
pub mod event;
pub mod frame;
pub mod message;
pub mod snapshot;

pub use capability::Capabilities;
pub use error::{Error, code};
pub use message::Message;

/// The protocol version this crate speaks by default.
pub const PROTOCOL_VERSION: u16 = 1;

/// Every protocol version this crate can speak, lowest first.
pub const SUPPORTED_VERSIONS: &[u16] = &[PROTOCOL_VERSION];
