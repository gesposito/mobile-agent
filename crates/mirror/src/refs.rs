//! Refs: the `eN` handles snapshots print, as agent-browser numbers them.
//!
//! A ref names one element for as long as the element survives from read to
//! read. Numbers only grow and are never reused, so a ref whose element is
//! gone can only be refused as stale, never resolve to another element.

use core::fmt;

/// A ref, printed `eN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ref(pub u32);

impl Ref {
    /// Parses a ref in any form agent-browser accepts: `@e5`, `ref=e5` or
    /// `e5`, surrounding whitespace ignored.
    pub fn parse(_input: &str) -> Option<Ref> {
        todo!("task 7.2")
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.0)
    }
}
