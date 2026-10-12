//! Refs: the `eN` handles snapshots print, as agent-browser numbers them.
//!
//! A ref names one control: an element, and for an element without an
//! identifier, also its accessible name, unless the element keeps its role
//! and its place, its parent and its position among the parent's children.
//! It names that control for as long as the element survives from read to
//! read and, without an identifier, keeps the name or the place: an element
//! renamed and moved, as a screen transition renames a button and lays the
//! form out again, is another control, whose snapshot gives it a fresh ref,
//! while a field whose label changes in place keeps its ref (rn-app-fixes,
//! design D39). Numbers only grow and are never reused, so a ref whose
//! control is gone can only be refused as stale, never resolve to another
//! control.

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
    /// The element's position among its parent's children when the ref last
    /// named it; its parent stays the same element while it survives.
    pub place: u32,
    /// The replica generation the ref was issued in.
    pub generation: u64,
    /// The element, having no identifier, was renamed and given a fresh
    /// ref: this one named another control.
    pub retired: bool,
}

/// What the table knows about a ref.
pub(crate) enum Lookup<'a> {
    /// Never issued.
    Unknown,
    /// Issued, then forgotten because its element is long gone.
    Forgotten,
    Issued(&'a Issued),
}

/// Whether the ref `issued` describes still names its element as `role`,
/// `name` and `place` give it: by any name when the element is identified,
/// else under the same name, or in the same role and place.
pub(crate) fn names(issued: &Issued, role: Role, name: &str, place: u32, identified: bool) -> bool {
    identified || issued.name == name || (issued.role == role && issued.place == place)
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

    /// The element's ref, issuing the next number when it has none, or when
    /// it has no identifier (`identified` false) and its ref was issued under
    /// another name in another role or place, which retires that ref.
    pub fn assign(
        &mut self,
        element: ElementId,
        (role, name, place): (Role, &str, u32),
        identified: bool,
        generation: u64,
    ) -> Ref {
        if let Some(&reference) = self.by_element.get(&element)
            && let Some(issued) = self.issued.get_mut(&reference)
        {
            if names(issued, role, name, place, identified) {
                issued.role = role;
                issued.place = place;
                if issued.name != name {
                    issued.name = name.to_owned();
                }
                return reference;
            }
            issued.retired = true;
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
                place,
                generation,
                retired: false,
            },
        );
        reference
    }

    /// The element's ref, if it has one that names it as `role`, `name` and
    /// `place` give it (see [`names`]).
    pub fn of(
        &self,
        element: ElementId,
        (role, name, place): (Role, &str, u32),
        identified: bool,
    ) -> Option<Ref> {
        let reference = *self.by_element.get(&element)?;
        let issued = self.issued.get(&reference)?;
        names(issued, role, name, place, identified).then_some(reference)
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

    /// Forgets retired refs and those of the elements that `live` no longer
    /// holds once the table grows past its bound, oldest refs first.
    pub fn prune(&mut self, live: impl Fn(ElementId) -> bool) {
        if self.issued.len() <= KEEP {
            return;
        }
        let mut dead: Vec<Ref> = self
            .issued
            .iter()
            .filter(|(_, issued)| issued.retired || !live(issued.element))
            .map(|(reference, _)| *reference)
            .collect();
        dead.sort_unstable();
        let excess = self.issued.len() - KEEP;
        for reference in dead.into_iter().take(excess) {
            if let Some(issued) = self.issued.remove(&reference)
                && self.by_element.get(&issued.element) == Some(&reference)
            {
                self.by_element.remove(&issued.element);
            }
        }
    }
}
