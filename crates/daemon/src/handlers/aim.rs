//! Element arguments: a ref as snapshots print it (`@e3`, `ref=e3`, `e3`)
//! or a selector: `#id` or `id=<id>` for a test identifier, `text=<text>` or
//! any other text for an element whose name or value contains it, and
//! `role=<role>`.
//!
//! A session acts only on refs its own snapshots listed on the target,
//! which keeps refs isolated between sessions. A selector waits for a match
//! up to the command's timeout. Either first brings the replica up to the
//! display (`Attached::catch_up`), so an element is aimed at where it is.

use std::time::Duration;

use mirror::{Condition, Element, MirrorError, Ref, Role, Selector, SnapshotOptions, Target};

use crate::wire::Failure;
use crate::wire::code::{self, ELEMENT_NOT_FOUND};
use crate::worker::Attached;

/// An element argument.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Aim {
    Ref(Ref),
    Select(Selector),
}

const ROLES: [Role; 28] = [
    Role::Button,
    Role::Link,
    Role::Textbox,
    Role::Searchbox,
    Role::Checkbox,
    Role::Radio,
    Role::Switch,
    Role::Slider,
    Role::Combobox,
    Role::Tab,
    Role::Menuitem,
    Role::Option,
    Role::Heading,
    Role::Cell,
    Role::Listitem,
    Role::Text,
    Role::Image,
    Role::List,
    Role::Grid,
    Role::Tablist,
    Role::Toolbar,
    Role::Alert,
    Role::Dialog,
    Role::Progressbar,
    Role::Keyboard,
    Role::Webview,
    Role::Group,
    Role::Generic,
];

impl Aim {
    pub fn parse(text: &str) -> Result<Aim, Failure> {
        let trimmed = text.trim();
        if let Some(reference) = Ref::parse(trimmed) {
            return Ok(Aim::Ref(reference));
        }
        let invalid = |what: &str| {
            Failure::new(
                code::INVALID_ARGUMENT,
                format!("{text:?} is not {what}; use a ref such as @e3, #id, text=... or role=..."),
            )
        };
        if trimmed.starts_with('@') || trimmed.starts_with("ref=") {
            return Err(invalid("a ref"));
        }
        if let Some(id) = trimmed
            .strip_prefix('#')
            .or_else(|| trimmed.strip_prefix("id="))
        {
            if id.is_empty() {
                return Err(invalid("an identifier"));
            }
            return Ok(Aim::Select(Selector::identifier(id)));
        }
        if let Some(name) = trimmed.strip_prefix("role=") {
            let role = ROLES
                .into_iter()
                .find(|role| role.as_str() == name)
                .ok_or_else(|| invalid("a role"))?;
            return Ok(Aim::Select(Selector {
                role: Some(role),
                ..Selector::default()
            }));
        }
        let text = trimmed.strip_prefix("text=").unwrap_or(trimmed);
        if text.is_empty() {
            return Err(invalid("an element"));
        }
        Ok(Aim::Select(Selector::text(text)))
    }
}

/// Fails unless `session`'s snapshots of the target listed `reference`.
pub(crate) fn check_seen(
    target: &mut Attached,
    session: &str,
    reference: Ref,
) -> Result<(), Failure> {
    let id = target.info.id.clone();
    if target.session(session).seen.contains(&reference) {
        return Ok(());
    }
    Err(Failure::new(
        mirror::code::UNKNOWN_REF,
        format!(
            "@{reference} is not a ref from session \"{session}\"'s snapshots of {id}; take a snapshot and use the refs it prints"
        ),
    ))
}

/// The first element on screen that `selector` matches, waiting up to
/// `timeout`; a timeout becomes `element_not_found` with the screen seen.
fn find(target: &mut Attached, selector: &Selector, timeout: Duration) -> Result<Element, Failure> {
    match target.mirror.wait(selector, Condition::Appear, timeout) {
        Ok(outcome) => outcome
            .element
            .ok_or_else(|| Failure::internal("a wait for an element returned none")),
        Err(MirrorError::WaitTimeout {
            waited, summary, ..
        }) => Err(Failure::new(
            ELEMENT_NOT_FOUND,
            format!(
                "no element matches {selector} after {} ms; the screen shows {summary}",
                waited.as_millis()
            ),
        )),
        Err(error) => Err(error.into()),
    }
}

/// The element an argument names, as the replica shows it now.
pub(crate) fn element(
    target: &mut Attached,
    session: &str,
    aim: &Aim,
    timeout: Duration,
) -> Result<Element, Failure> {
    target.catch_up()?;
    match aim {
        Aim::Ref(reference) => {
            check_seen(target, session, *reference)?;
            Ok(target.mirror.resolve(*reference)?)
        }
        Aim::Select(selector) => find(target, selector, timeout),
    }
}

/// Where an action on an argument lands: a ref, which the mirror checks
/// for being stale, covered or scrolled away, or for a selector the center
/// of the element it matched.
pub(crate) fn action_target(
    target: &mut Attached,
    session: &str,
    aim: &Aim,
    timeout: Duration,
) -> Result<(Target, Option<Ref>), Failure> {
    target.catch_up()?;
    match aim {
        Aim::Ref(reference) => {
            check_seen(target, session, *reference)?;
            Ok((Target::Ref(*reference), Some(*reference)))
        }
        Aim::Select(selector) => {
            let element = find(target, selector, timeout)?;
            Ok(match element.reference {
                Some(reference) => (Target::Ref(reference), Some(reference)),
                None => (Target::Point(element.bounds.center()), None),
            })
        }
    }
}

/// A ref for the element an argument names, for text entry: a selector's
/// element gets one from an interactive listing if no snapshot listed it.
pub(crate) fn text_ref(
    target: &mut Attached,
    session: &str,
    aim: &Aim,
    timeout: Duration,
) -> Result<Ref, Failure> {
    target.catch_up()?;
    match aim {
        Aim::Ref(reference) => {
            check_seen(target, session, *reference)?;
            Ok(*reference)
        }
        Aim::Select(selector) => {
            let element = find(target, selector, timeout)?;
            if let Some(reference) = element.reference {
                return Ok(reference);
            }
            let listing = target.mirror.snapshot(&SnapshotOptions::interactive())?;
            listing
                .elements
                .iter()
                .find(|listed| listed.id == element.id)
                .and_then(|listed| listed.reference)
                .ok_or_else(|| {
                    Failure::new(
                        code::INVALID_ARGUMENT,
                        format!("{selector} matches a {} that takes no text", element.role),
                    )
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_refs_identifiers_texts_or_roles() {
        for text in ["@e3", "ref=e3", "e3", " @e3 "] {
            assert_eq!(Aim::parse(text).unwrap(), Aim::Ref(Ref(3)), "{text}");
        }
        assert_eq!(
            Aim::parse("#login-submit").unwrap(),
            Aim::Select(Selector::identifier("login-submit"))
        );
        assert_eq!(
            Aim::parse("id=x").unwrap(),
            Aim::Select(Selector::identifier("x"))
        );
        assert_eq!(
            Aim::parse("text=Sign in").unwrap(),
            Aim::Select(Selector::text("Sign in"))
        );
        assert_eq!(
            Aim::parse("Sign in").unwrap(),
            Aim::Select(Selector::text("Sign in"))
        );
        assert_eq!(
            Aim::parse("role=switch").unwrap(),
            Aim::Select(Selector {
                role: Some(Role::Switch),
                ..Selector::default()
            })
        );
        for bad in ["@x1", "ref=", "#", "role=nothing", "  "] {
            assert_eq!(
                Aim::parse(bad).unwrap_err().code,
                code::INVALID_ARGUMENT,
                "{bad}"
            );
        }
    }
}
