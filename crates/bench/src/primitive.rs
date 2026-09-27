//! The primitives the benchmark-harness spec measures, and the fixed string the
//! text-entry primitive types.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The string every tool types for [`Primitive::TextEntry`]: 36 keys that need
/// no shift on either platform's keyboard, as spike S3 typed them.
pub const TEXT_ENTRY: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

/// One measured primitive. Its serialized name is [`Primitive::id`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Primitive {
    /// CLI process start: from the process starting to its request reaching the daemon.
    CliStart,
    /// From no daemon and no device-side helper to a target ready for commands.
    ColdAttach,
    /// The first snapshot after the application is launched.
    FirstSnapshot,
    /// A snapshot while the replica is warm.
    WarmSnapshot,
    /// From a tap request to the input being dispatched to the device; the
    /// Simulator's 20 ms gate is on this.
    TapDispatch,
    /// From a tap request to the first changed frame.
    TapToFrameChange,
    /// From a tap request to the first tree delta.
    TapToTreeDelta,
    /// Typing [`TEXT_ENTRY`] into a focused field.
    TextEntry,
    /// One screenshot.
    Screenshot,
    /// From an action to the settle wait resolving.
    Settle,
}

impl Primitive {
    /// The nine primitives listed by the spec's "Measured primitives"
    /// requirement, in its order.
    pub const LISTED: [Primitive; 9] = [
        Primitive::CliStart,
        Primitive::ColdAttach,
        Primitive::FirstSnapshot,
        Primitive::WarmSnapshot,
        Primitive::TapToFrameChange,
        Primitive::TapToTreeDelta,
        Primitive::TextEntry,
        Primitive::Screenshot,
        Primitive::Settle,
    ];

    /// Every primitive: the listed nine and tap dispatch, which the Simulator
    /// gate needs.
    pub const ALL: [Primitive; 10] = [
        Primitive::CliStart,
        Primitive::ColdAttach,
        Primitive::FirstSnapshot,
        Primitive::WarmSnapshot,
        Primitive::TapDispatch,
        Primitive::TapToFrameChange,
        Primitive::TapToTreeDelta,
        Primitive::TextEntry,
        Primitive::Screenshot,
        Primitive::Settle,
    ];

    /// Stable identifier used in reports, gate files and fake scripts.
    pub fn id(self) -> &'static str {
        match self {
            Primitive::CliStart => "cli-start",
            Primitive::ColdAttach => "cold-attach",
            Primitive::FirstSnapshot => "first-snapshot",
            Primitive::WarmSnapshot => "warm-snapshot",
            Primitive::TapDispatch => "tap-dispatch",
            Primitive::TapToFrameChange => "tap-to-frame-change",
            Primitive::TapToTreeDelta => "tap-to-tree-delta",
            Primitive::TextEntry => "text-entry",
            Primitive::Screenshot => "screenshot",
            Primitive::Settle => "settle",
        }
    }

    /// The spec's words for the primitive.
    pub fn label(self) -> &'static str {
        match self {
            Primitive::CliStart => "CLI process start",
            Primitive::ColdAttach => "cold attach",
            Primitive::FirstSnapshot => "first snapshot after launch",
            Primitive::WarmSnapshot => "warm snapshot",
            Primitive::TapDispatch => "tap dispatch",
            Primitive::TapToFrameChange => "tap to frame change",
            Primitive::TapToTreeDelta => "tap to tree delta",
            Primitive::TextEntry => "text entry",
            Primitive::Screenshot => "screenshot",
            Primitive::Settle => "settle detection",
        }
    }
}

impl fmt::Display for Primitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_names_are_the_ids() {
        for primitive in Primitive::ALL {
            let json = serde_json::to_string(&primitive).unwrap();
            assert_eq!(json, format!("\"{}\"", primitive.id()));
            let back: Primitive = serde_json::from_str(&json).unwrap();
            assert_eq!(back, primitive);
        }
    }

    #[test]
    fn every_listed_primitive_is_in_all_and_only_tap_dispatch_is_added() {
        for primitive in Primitive::LISTED {
            assert!(Primitive::ALL.contains(&primitive));
        }
        let added: Vec<_> = Primitive::ALL
            .into_iter()
            .filter(|p| !Primitive::LISTED.contains(p))
            .collect();
        assert_eq!(added, [Primitive::TapDispatch]);
    }

    #[test]
    fn the_text_entry_string_is_fixed_and_unshifted() {
        assert_eq!(TEXT_ENTRY.len(), 36);
        assert!(
            TEXT_ENTRY
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }
}
