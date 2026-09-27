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
    /// Siblings that read the same (class, identifier and content) are told
    /// apart only by their position, so their refs can swap when the
    /// siblings move.
    SiblingCollision { refs: Vec<Ref> },
}

impl Degradation {
    /// A snake_case name for the reason.
    pub fn code(&self) -> &'static str {
        match self {
            Degradation::SiblingCollision { .. } => "sibling_collision",
        }
    }

    /// What a consumer can do about it.
    pub fn advice(&self) -> String {
        match self {
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
