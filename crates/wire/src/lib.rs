//! The daemon's requests and replies as types (design D3 of the
//! `mcp-server` change): the request envelope, one struct of parameters per
//! action under the names the daemon reads ([`Command`]), and one struct of
//! reply data per action under the names its handlers write ([`reply`]),
//! with the JSON schemas of all of them behind the `schema` feature.
//!
//! The reply types are held to the handlers' replies by the daemon's tests
//! (`crates/daemon/tests/replies.rs`), which read every action's reply on a
//! fake target with unknown fields refused. See `README.md`.

mod param;
pub mod reply;
mod request;

pub use param::Param;
pub use request::*;
