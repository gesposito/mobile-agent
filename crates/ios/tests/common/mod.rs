//! Helpers shared by the device tests.
//!
//! Every test here drives a physical iPhone, so each one first checks
//! `MOBILE_AGENT_IOS_UDID`. When it is unset, as in CI, the test prints why
//! it skipped and passes. Set it to the identifier of an iPhone attached
//! over USB, trusted, unlocked, with Developer Mode on and the developer
//! disk image mounted, and `MOBILE_AGENT_IOS_TEAM` to the team its runner
//! is signed for:
//!
//! ```sh
//! MOBILE_AGENT_IOS_UDID=<udid> MOBILE_AGENT_IOS_TEAM=<team> cargo test -p ios
//! ```
//!
//! Neither value enters a tracked file, and no test prints them.

#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard};

/// The variable that enables the device tests.
pub const UDID_VARIABLE: &str = "MOBILE_AGENT_IOS_UDID";

/// The device to drive, or `None` after printing why the test is skipped.
pub fn gated(test: &str) -> Option<String> {
    match std::env::var(UDID_VARIABLE) {
        Ok(udid) if !udid.trim().is_empty() => Some(udid.trim().to_owned()),
        _ => {
            eprintln!(
                "{test}: skipped; set {UDID_VARIABLE} to the identifier of an iPhone attached \
                 over USB, trusted, unlocked, with Developer Mode on"
            );
            None
        }
    }
}

/// Serializes the tests of one binary that change what the device runs.
/// Cargo runs test binaries one after another, so this covers the rest.
pub fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
