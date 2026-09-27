//! The quality verdict every snapshot carries (spec: Quality verdict on
//! every snapshot): the backend that produced the data and whether, and why,
//! it is degraded.

use crate::backend::BackendInfo;
use crate::refs::Ref;

/// How far a snapshot can be trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// The backend that read the tree.
    pub backend: BackendInfo,
    /// Whether any reason below applies.
    pub degraded: bool,
    pub reasons: Vec<Degradation>,
}

impl Verdict {
    /// What a consumer can do about each reason, in order.
    pub fn advice(&self) -> Vec<String> {
        self.reasons.iter().map(Degradation::advice).collect()
    }
}

/// Why a snapshot is degraded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Degradation {
    /// The tree came from a fallback backend with known gaps, such as the
    /// Simulator's host translator.
    FallbackBackend { name: String },
    /// A request bound cut the read short.
    Truncated {
        depth: bool,
        nodes: bool,
        windows: bool,
    },
    /// An application window came back as its root alone, as a failed or
    /// premature read does, although the screen is populated.
    SparseTree { window: u32 },
    /// Siblings that read the same (class, identifier and content) are told
    /// apart only by their position, so their refs can swap when the
    /// siblings move.
    SiblingCollision { refs: Vec<Ref> },
    /// The agent reported reduced ability or lost its UI automation role.
    AgentWarning(String),
    /// The target disconnected; the snapshot is the last screen the mirror
    /// saw.
    Disconnected(String),
}

impl Degradation {
    /// A snake_case name for the reason.
    pub fn code(&self) -> &'static str {
        match self {
            Degradation::FallbackBackend { .. } => "fallback_backend",
            Degradation::Truncated { .. } => "truncated",
            Degradation::SparseTree { .. } => "sparse_tree",
            Degradation::SiblingCollision { .. } => "sibling_collision",
            Degradation::AgentWarning(_) => "agent_warning",
            Degradation::Disconnected(_) => "disconnected",
        }
    }

    /// What a consumer can do about it.
    pub fn advice(&self) -> String {
        match self {
            Degradation::FallbackBackend { name } => format!(
                "the tree comes from the fallback backend {name}, which can flatten containers and miss elements; check the frame before trusting an absent element"
            ),
            Degradation::Truncated {
                depth,
                nodes,
                windows,
            } => {
                let mut limits = Vec::new();
                if *depth {
                    limits.push("depth");
                }
                if *nodes {
                    limits.push("node count");
                }
                if *windows {
                    limits.push("windows");
                }
                format!(
                    "the read stopped at its {} limit; raise the limit or read a narrower part of the screen",
                    limits.join(" and ")
                )
            }
            Degradation::SparseTree { window } => format!(
                "window {window} returned only its application root although the screen is populated; retry the snapshot in a moment or use the frame"
            ),
            Degradation::AgentWarning(message) => {
                format!("the agent reported: {message}")
            }
            Degradation::Disconnected(reason) => format!(
                "the target is disconnected ({reason}); this is the last screen seen, reconnect before acting"
            ),
            Degradation::SiblingCollision { refs } => {
                let refs: Vec<String> = refs.iter().map(Ref::to_string).collect();
                format!(
                    "{} are identical siblings told apart only by position; take a new snapshot right before acting on one of them",
                    refs.join(", ")
                )
            }
        }
    }
}
