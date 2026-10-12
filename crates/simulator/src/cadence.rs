//! The pace of typed keys (simulator-driver spec, "Text entry"): the pause
//! between one key press and the next.

use std::time::Duration;

/// The pause between two keys `type_text_at` types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cadence {
    /// From one key's release to the next key's press.
    pub gap: Duration,
}

impl Cadence {
    /// Native fields, UIKit's and SwiftUI's. Spike S3 typed exactly with
    /// 60 ms; this shorter gap kept every character and its order in the
    /// fixtures' fields.
    pub const NATIVE: Cadence = Cadence {
        gap: Duration::from_millis(15),
    };
}

impl Default for Cadence {
    fn default() -> Cadence {
        Cadence::NATIVE
    }
}
