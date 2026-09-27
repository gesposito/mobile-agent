//! Refs: the `eN` handles snapshots print, as agent-browser numbers them.
//!
//! A ref names one element for as long as the element survives from read to
//! read. Numbers only grow and are never reused, so a ref whose element is
//! gone can only be refused as stale, never resolve to another element.

use core::fmt;
use std::collections::HashMap;

use crate::snapshot::{ElementId, Role};

/// Issued refs whose elements are gone are forgotten, oldest first, beyond
/// this many entries; a forgotten ref is still reported stale.
const KEEP: usize = 8192;

/// A ref, printed `eN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ref(pub u32);

impl Ref {
    /// Parses a ref in any form agent-browser accepts: `@e5`, `ref=e5` or
    /// `e5`, surrounding whitespace ignored. Numbers start at 1 and have no
    /// leading zero.
    pub fn parse(input: &str) -> Option<Ref> {
        let s = input.trim();
        let s = s
            .strip_prefix('@')
            .or_else(|| s.strip_prefix("ref="))
            .unwrap_or(s);
        let digits = s.strip_prefix('e')?;
        if digits.is_empty()
            || digits.starts_with('0')
            || !digits.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        digits.parse().ok().map(Ref)
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.0)
    }
}

/// What a ref was issued for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Issued {
    pub element: ElementId,
    pub role: Role,
    pub name: String,
    /// The replica generation the ref was issued in.
    pub generation: u64,
}

/// What the table knows about a ref.
pub(crate) enum Lookup<'a> {
    /// Never issued.
    Unknown,
    /// Issued, then forgotten because its element is long gone.
    Forgotten,
    Issued(&'a Issued),
}

/// Every ref one mirror issued.
pub(crate) struct RefTable {
    next: u32,
    by_element: HashMap<ElementId, Ref>,
    issued: HashMap<Ref, Issued>,
}

impl RefTable {
    pub fn new() -> RefTable {
        RefTable {
            next: 1,
            by_element: HashMap::new(),
            issued: HashMap::new(),
        }
    }

    /// The element's ref, issuing the next number when it has none.
    pub fn assign(&mut self, element: ElementId, role: Role, name: &str, generation: u64) -> Ref {
        if let Some(&reference) = self.by_element.get(&element) {
            if let Some(issued) = self.issued.get_mut(&reference) {
                issued.role = role;
                if issued.name != name {
                    issued.name = name.to_owned();
                }
            }
            return reference;
        }
        let reference = Ref(self.next);
        self.next += 1;
        self.by_element.insert(element, reference);
        self.issued.insert(
            reference,
            Issued {
                element,
                role,
                name: name.to_owned(),
                generation,
            },
        );
        reference
    }

    /// The element's ref, if it has one.
    pub fn of(&self, element: ElementId) -> Option<Ref> {
        self.by_element.get(&element).copied()
    }

    pub fn lookup(&self, reference: Ref) -> Lookup<'_> {
        if reference.0 == 0 || reference.0 >= self.next {
            return Lookup::Unknown;
        }
        match self.issued.get(&reference) {
            Some(issued) => Lookup::Issued(issued),
            None => Lookup::Forgotten,
        }
    }

    /// Forgets the elements that `live` no longer holds once the table grows
    /// past its bound, oldest refs first.
    pub fn prune(&mut self, live: impl Fn(ElementId) -> bool) {
        if self.issued.len() <= KEEP {
            return;
        }
        let mut dead: Vec<Ref> = self
            .issued
            .iter()
            .filter(|(_, issued)| !live(issued.element))
            .map(|(reference, _)| *reference)
            .collect();
        dead.sort_unstable();
        let excess = self.issued.len() - KEEP;
        for reference in dead.into_iter().take(excess) {
            if let Some(issued) = self.issued.remove(&reference) {
                self.by_element.remove(&issued.element);
            }
        }
    }
}
