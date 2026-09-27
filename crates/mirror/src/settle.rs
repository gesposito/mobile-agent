//! The settle predicate (spec: Settle predicate, design D8): the screen is
//! settled once neither the tree nor the frames have changed for a quiet
//! window, bounded by a maximum after which the result says what kept
//! changing instead of failing.

use std::time::Duration;

/// The quiet window: spike S1 measured Android content changes every 101 to
/// 116 ms during a continuing change, with gaps up to 187 ms inside a
/// scroll, so a shorter window would end inside a change.
pub const DEFAULT_QUIET_WINDOW: Duration = Duration::from_millis(200);

/// The longest a settle wait lasts.
pub const DEFAULT_SETTLE_MAX: Duration = Duration::from_secs(3);

/// How to wait for the screen to settle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettleOptions {
    pub quiet: Duration,
    pub max: Duration,
    pub signals: Signals,
}

impl Default for SettleOptions {
    fn default() -> SettleOptions {
        SettleOptions {
            quiet: DEFAULT_QUIET_WINDOW,
            max: DEFAULT_SETTLE_MAX,
            signals: Signals::TreeAndFrames,
        }
    }
}

impl Signals {
    pub(crate) fn tree(self) -> bool {
        self != Signals::FramesOnly
    }

    pub(crate) fn frames(self) -> bool {
        self != Signals::TreeOnly
    }
}

/// Which changes hold a settle wait.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Signals {
    #[default]
    TreeAndFrames,
    /// Tree changes only, for screens whose content moves on its own, such
    /// as video.
    TreeOnly,
    FramesOnly,
}

/// A kind of change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Signal {
    /// Tree change events, or deltas between reads of a window that sends
    /// none.
    Tree,
    /// Frame hash changes.
    Frames,
}

/// How a settle wait ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settle {
    pub settled: bool,
    /// From the start of the wait to its end.
    pub elapsed: Duration,
    /// When not settled, the signals that changed within the last quiet
    /// window.
    pub still_changing: Vec<Signal>,
    /// Tree changes seen while waiting.
    pub tree_changes: u64,
    /// Frame hash changes seen while waiting.
    pub frame_changes: u64,
}

impl Settle {
    /// Why the screen is still changing, such as `frame changes`.
    pub fn reason(&self) -> Option<String> {
        if self.settled || self.still_changing.is_empty() {
            return None;
        }
        let names: Vec<&str> = self
            .still_changing
            .iter()
            .map(|signal| match signal {
                Signal::Tree => "tree changes",
                Signal::Frames => "frame changes",
            })
            .collect();
        Some(names.join(" and "))
    }
}
