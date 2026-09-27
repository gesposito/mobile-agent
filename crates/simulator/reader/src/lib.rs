//! The Simulator driver's accessibility reader: a program built for
//! `aarch64-apple-ios-sim` that the driver spawns inside a booted Simulator,
//! where it reads the foreground application's tree through the runtime's
//! private XCTAutomationSupport client and serves it to the host over the
//! device protocol (design D6, spike S4).
//!
//! - [`elements`] is the tree model both of the driver's read paths fill in,
//!   and its conversion into the protocol's snapshot payload. It is plain
//!   Rust; the driver's host translator uses it too.
//! - `guest` (Apple platforms) is the reader proper: the runtime bindings,
//!   the snapshot extraction and the protocol server. It builds for macOS as
//!   well, where its tests run, although the runtime it loads exists only
//!   inside a Simulator.
//!
//! `crates/simulator/README.md` describes how the driver builds, ships and
//! spawns it.

pub mod elements;

#[cfg(target_vendor = "apple")]
pub mod guest;

/// The error codes the reader answers a failed read with, besides the
/// protocol's own (`window_changed` when the foreground changes during the
/// read, `busy`, `internal`).
pub mod codes {
    /// No process owns the foreground yet: accessibility is still starting,
    /// or an application is launching. Retryable.
    pub const NO_FOREGROUND: &str = "no_foreground";
    /// The runtime's snapshot call failed, as it does for about a second
    /// while an application starts (spike S4). Retryable.
    pub const SNAPSHOT_FAILED: &str = "snapshot_failed";
    /// A private call raised an Objective-C exception; the message is
    /// `<call> raised <name>: <reason>`. Not retryable.
    pub const PRIVATE_API_EXCEPTION: &str = "private_api_exception";
}

/// The reader's name and version, as its hello announces them.
pub const AGENT: &str = concat!("simulator-reader ", env!("CARGO_PKG_VERSION"));
