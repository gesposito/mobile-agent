//! The daemon's requests and replies as types (design D3 of the
//! `mcp-server` change): the request envelope, one struct of parameters per
//! action under the names the daemon reads, and one struct of reply data per
//! action under the names its handlers write, with the JSON schemas of all of
//! them behind the `schema` feature.

pub mod reply;
mod request;

pub use request::*;
