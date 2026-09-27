//! Helpers shared by the integration tests, which run against a live device
//! only when [`SERIAL_VAR`] names one.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

/// Environment variable naming the device the integration tests drive.
pub const SERIAL_VAR: &str = "MOBILE_AGENT_ANDROID_SERIAL";

/// The serial under test, or `None` after printing why `test` is skipped.
pub fn serial_or_skip(test: &str) -> Option<String> {
    match std::env::var(SERIAL_VAR) {
        Ok(serial) if !serial.trim().is_empty() => Some(serial.trim().to_owned()),
        _ => {
            eprintln!("skipped {test}: set {SERIAL_VAR} to a connected device's serial to run it");
            None
        }
    }
}

/// A name no other test in any run on this host uses: a tag, the process id
/// and a counter. Safe as a file name and an abstract socket name.
pub fn unique_name(tag: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "mobile-agent-it-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Nearest-rank percentile `p` (0 to 100) of samples; sorts them in place.
pub fn percentile(samples: &mut [f64], p: f64) -> f64 {
    assert!(!samples.is_empty(), "no samples");
    samples.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * samples.len() as f64).ceil() as usize;
    samples[rank.clamp(1, samples.len()) - 1]
}

/// Deterministic bytes that no transport compression shrinks (xorshift64*).
pub fn pattern(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        out.extend_from_slice(&state.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes());
    }
    out.truncate(len);
    out
}
