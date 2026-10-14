//! Element arguments: a ref as snapshots print it (`@e3`, `ref=e3`, `e3`)
//! or a selector: `#id` or `id=<id>` for a test identifier, `text=<text>` or
//! any other text for an element whose name or value contains it,
//! `text==<text>` for one whose whole name or value is the text, and
//! `role=<role>`.
//!
//! A session acts only on refs its own snapshots listed on the target,
//! which keeps refs isolated between sessions. A selector waits for a match
//! up to the command's timeout. Either first brings the replica up to the
//! display (`Attached::catch_up`), so an element is aimed at where it is.
//!
//! The mirror's selectors match text by containment, so an exact-text
//! selector looks through the full listing instead, where every element on
//! screen and not covered is listed with its computed name and value, and
//! waits between looks while the mirror keeps the replica current.

use std::time::{Duration, Instant};

use mirror::{
    Condition, Element, MirrorError, Ref, Role, Selector, Snapshot, SnapshotOptions,
    SnapshotSummary, Target, WaitOutcome,
};

use crate::wire::Failure;
use crate::wire::code::{self, ELEMENT_NOT_FOUND};
use crate::worker::Attached;

/// An element argument.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Aim {
    Ref(Ref),
    Select(Selector),
    /// `text==<text>`: an element whose name or value is exactly the text.
    Exact(String),
}

/// How often an exact-text wait looks at the screen again; the mirror
/// applies what the target reports and reads what falls due in between.
const EXACT_SLICE: Duration = Duration::from_millis(10);

/// Lines of the interactive listing a timeout shows, as the mirror's own
/// wait timeouts do.
const SUMMARY_LINES: usize = 20;

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
        if let Some(text) = trimmed.strip_prefix("text==") {
            if text.is_empty() {
                return Err(invalid("an exact text"));
            }
            return Ok(Aim::Exact(text.to_owned()));
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
    find_in(target, selector, Condition::Appear, timeout)
}

/// The first element that `selector` matches as `condition` looks for it,
/// waiting up to `timeout`; a timeout becomes `element_not_found` with the
/// screen seen.
fn find_in(
    target: &mut Attached,
    selector: &Selector,
    condition: Condition,
    timeout: Duration,
) -> Result<Element, Failure> {
    match target.mirror.wait(selector, condition, timeout) {
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

/// The element an argument names as the replica holds it, in view or not,
/// as a query reads it (rn-app-fixes, design D51): a ref's element, or the
/// first the replica holds that a selector matches, waiting up to `timeout`
/// for one; a timeout becomes `element_not_found` with the screen seen. On
/// the fourth acceptance run's feed scrolled to its end, `get box` on rows
/// scrolled above, which the tree held, waited out its 10 s for a match on
/// screen.
pub(crate) fn held(
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
        Aim::Select(selector) => find_in(target, selector, Condition::Attached, timeout),
        Aim::Exact(text) => {
            let selector = exact(text);
            find_in(target, &selector, Condition::Attached, timeout)
        }
    }
}

/// A selector for an element whose whole name or value is `text`.
pub(crate) fn exact(text: &str) -> Selector {
    Selector {
        text: Some(text.to_owned()),
        exact: true,
        ..Selector::default()
    }
}

/// Whether `element` is in view: a tap reaches part of it, inside what its
/// scroll containers show and from under what covers it.
pub(crate) fn in_view(target: &Attached, element: &Element) -> bool {
    target
        .mirror
        .visibility(element.id)
        .is_some_and(|seen| seen.fraction > 0.0)
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
        Aim::Exact(text) => find_exact(target, text, timeout),
    }
}

/// Where an action on an argument lands: a ref, which the mirror checks
/// for being stale, covered or scrolled away, or for a selector the center
/// of the element it matched; with the element a selector matched.
pub(crate) fn action_target(
    target: &mut Attached,
    session: &str,
    aim: &Aim,
    timeout: Duration,
) -> Result<(Target, Option<Ref>, Option<Element>), Failure> {
    target.catch_up()?;
    match aim {
        Aim::Ref(reference) => {
            check_seen(target, session, *reference)?;
            Ok((Target::Ref(*reference), Some(*reference), None))
        }
        Aim::Select(selector) => Ok(aimed_at(find(target, selector, timeout)?)),
        Aim::Exact(text) => Ok(aimed_at(find_exact(target, text, timeout)?)),
    }
}

/// Where an action on a found element lands: its ref, which the mirror
/// checks, else its center; with the element.
fn aimed_at(element: Element) -> (Target, Option<Ref>, Option<Element>) {
    match element.reference {
        Some(reference) => (Target::Ref(reference), Some(reference), Some(element)),
        None => (Target::Point(element.bounds.center()), None, Some(element)),
    }
}

/// Where text for an argument goes.
pub(crate) enum TextTarget {
    /// The element a ref names.
    Field(Ref),
    /// The one field inside the node an identifier named, with that node
    /// (rn-app-fixes, design D39).
    Within(Ref, Element),
    /// The node an identifier named, which takes no text, holds no field and
    /// has no ref, with the selector that found it: the text goes to the
    /// input a tap on it gives the focus, if the keyboard comes up (design
    /// D44).
    Raising(Element, Selector),
}

/// Where text for the element an argument names goes, for text entry: its
/// ref, which a selector's element gets from an interactive listing if no
/// snapshot listed it. An identifier that names a node taking no text and
/// holding exactly one field gives the field's ref, with the node it named
/// (rn-app-fixes, design D39): on the acceptance run's code step the
/// identifier named a React Native code input's view, around a field
/// without an area. One that names such a node holding no field and without
/// a ref gives the node, for the input a tap on it focuses (design D44): on
/// the third acceptance run's second visit the code input's view held
/// nothing, the app's input not in the tree.
pub(crate) fn text_ref(
    target: &mut Attached,
    session: &str,
    aim: &Aim,
    timeout: Duration,
) -> Result<TextTarget, Failure> {
    target.catch_up()?;
    match aim {
        Aim::Ref(reference) => {
            check_seen(target, session, *reference)?;
            Ok(TextTarget::Field(*reference))
        }
        Aim::Select(selector) => {
            let element = find(target, selector, timeout)?;
            let takes_text = matches!(element.role, Role::Textbox | Role::Searchbox);
            if selector.identifier.is_none() || takes_text {
                return Ok(TextTarget::Field(listed_ref(
                    target,
                    &element,
                    &selector.to_string(),
                )?));
            }
            if let Some(field) = target.mirror.field_within(element.id)
                && let Some(reference) = field.reference
            {
                return Ok(TextTarget::Within(reference, element));
            }
            match listed_ref(target, &element, &selector.to_string()) {
                Ok(reference) => Ok(TextTarget::Field(reference)),
                Err(_) if target.mirror.fields_within(element.id) == Some(0) => {
                    Ok(TextTarget::Raising(element, selector.clone()))
                }
                Err(failure) => Err(failure),
            }
        }
        Aim::Exact(text) => {
            let element = find_exact(target, text, timeout)?;
            let reference = listed_ref(target, &element, &format!("exact text {text:?}"))?;
            Ok(TextTarget::Field(reference))
        }
    }
}

/// The ref of `element`, from an interactive listing if no snapshot listed
/// it; `described` names the argument that found it.
fn listed_ref(target: &mut Attached, element: &Element, described: &str) -> Result<Ref, Failure> {
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
                format!("{described} matches a {} that takes no text", element.role),
            )
        })
}

/// Whether `element`'s whole name or value is `text`.
fn is_exactly(element: &Element, text: &str) -> bool {
    element.name == text || element.value.as_deref() == Some(text)
}

/// The first element of the full listing, on screen and not covered, whose
/// name or value is exactly `text`, and enabled when `enabled` says so.
fn exact_on_screen(
    target: &mut Attached,
    text: &str,
    enabled: bool,
) -> Result<Option<Element>, Failure> {
    let listing = target.mirror.snapshot(&SnapshotOptions::full())?;
    Ok(listing
        .elements
        .into_iter()
        .find(|element| is_exactly(element, text) && (!enabled || element.enabled)))
}

/// Waits until an element whose name or value is exactly `text` meets
/// `condition`, looking again every [`EXACT_SLICE`]; fails at `timeout`
/// with `wait_timeout` and a summary of the screen, as the mirror's waits
/// do.
pub(crate) fn wait_exact(
    target: &mut Attached,
    text: &str,
    condition: Condition,
    timeout: Duration,
) -> Result<WaitOutcome, Failure> {
    let started = Instant::now();
    let deadline = started + timeout;
    loop {
        let found = exact_on_screen(target, text, condition == Condition::Enabled)?;
        let holds = match condition {
            Condition::Disappear | Condition::Detached => found.is_none(),
            Condition::Appear | Condition::Enabled | Condition::Attached => found.is_some(),
        };
        if holds {
            return Ok(WaitOutcome {
                elapsed: started.elapsed(),
                element: found,
            });
        }
        let now = Instant::now();
        if now >= deadline {
            let summary = summary(target)?;
            return Err(Failure::new(
                mirror::code::WAIT_TIMEOUT,
                format!(
                    "waited {} ms for exact text {text:?} to {}; the screen now shows {summary}",
                    now.duration_since(started).as_millis(),
                    condition.name()
                ),
            ));
        }
        target.mirror.run_until((now + EXACT_SLICE).min(deadline))?;
    }
}

/// The first element on screen whose name or value is exactly `text`,
/// waiting up to `timeout`; a timeout becomes `element_not_found` with the
/// screen seen.
fn find_exact(target: &mut Attached, text: &str, timeout: Duration) -> Result<Element, Failure> {
    match wait_exact(target, text, Condition::Appear, timeout) {
        Ok(outcome) => outcome
            .element
            .ok_or_else(|| Failure::internal("a wait for an element returned none")),
        Err(failure) if failure.code == mirror::code::WAIT_TIMEOUT => Err(Failure::new(
            ELEMENT_NOT_FOUND,
            failure.message.replacen(
                &format!("for exact text {text:?} to appear"),
                &format!("for an element whose name or value is exactly {text:?}"),
                1,
            ),
        )),
        Err(failure) => Err(failure),
    }
}

/// The screen as a wait's timeout shows it: the replica's revision, its
/// windows, the elements with refs and the first lines of the interactive
/// listing.
fn summary(target: &mut Attached) -> Result<SnapshotSummary, Failure> {
    let snapshot: Snapshot = target.mirror.snapshot(&SnapshotOptions::interactive())?;
    let lines: Vec<String> = snapshot.text.lines().map(str::to_owned).collect();
    let age = snapshot
        .windows
        .iter()
        .map(|window| window.source.age)
        .min()
        .unwrap_or_default();
    Ok(SnapshotSummary {
        revision: snapshot.revision,
        age,
        windows: snapshot.windows.len(),
        elements: snapshot.refs().len(),
        omitted: lines.len().saturating_sub(SUMMARY_LINES),
        lines: lines.into_iter().take(SUMMARY_LINES).collect(),
        verdict: snapshot.verdict,
    })
}

/// The element `aim` names if it is on screen now, not covered and not
/// scrolled away, without waiting.
pub(crate) fn on_screen(
    target: &mut Attached,
    session: &str,
    aim: &Aim,
) -> Result<Option<Element>, Failure> {
    let selector = match aim {
        Aim::Ref(reference) => {
            check_seen(target, session, *reference)?;
            Selector::reference(*reference)
        }
        Aim::Select(selector) => selector.clone(),
        Aim::Exact(text) => return exact_on_screen(target, text, false),
    };
    match target
        .mirror
        .wait(&selector, Condition::Appear, Duration::ZERO)
    {
        Ok(outcome) => Ok(outcome.element),
        Err(MirrorError::WaitTimeout { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_double_equals_is_an_exact_text_selector() {
        assert_eq!(
            Aim::parse("text==Row 1").unwrap(),
            Aim::Exact("Row 1".into())
        );
        assert_eq!(
            Aim::parse(" text==Sign in ").unwrap(),
            Aim::Exact("Sign in".into())
        );
        assert_eq!(
            Aim::parse("text==a=b").unwrap(),
            Aim::Exact("a=b".into()),
            "only the first == separates"
        );
        assert_eq!(
            Aim::parse("text=Row 1").unwrap(),
            Aim::Select(Selector::text("Row 1"))
        );
    }

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
        for bad in ["@x1", "ref=", "#", "role=nothing", "  ", "text=="] {
            assert_eq!(
                Aim::parse(bad).unwrap_err().code,
                code::INVALID_ARGUMENT,
                "{bad}"
            );
        }
    }
}
