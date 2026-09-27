//! The host daemon (design D1): one process per user holds every device
//! connection, keeps a mirror per attached target, and serves the
//! `mobile-agent` CLI on a Unix socket with agent-browser's newline-delimited
//! JSON, one request and one reply per line.
//!
//! One daemon owns every target, because Android permits one UI automation
//! owner per device; sessions are claims inside it rather than daemons of
//! their own, and commands for different targets run concurrently on
//! per-target worker threads. Targets come from [`driver::Driver`]s: the
//! Simulator and Android drivers register in [`driver::registered`], and
//! the fake driver in [`fake`] serves every test that needs no device.

mod devices;
pub mod driver;
pub mod fake;
mod handlers;
mod keys;
pub mod paths;
mod registry;
mod server;
mod signals;
pub mod wire;
mod worker;

pub use server::{DEFAULT_IDLE_TIMEOUT, Exit, Options, parse_duration, run};
