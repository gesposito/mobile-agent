//! Element arguments: a ref as snapshots print it (`@e3`, `ref=e3`, `e3`)
//! or a selector: `#id` or `id=<id>` for a test identifier, `text=<text>` or
//! any other text for an element whose name or value contains it,
//! `text==<text>` for one whose whole name or value is the text, and
//! `role=<role>`.
//!
//! A session acts only on refs its own snapshots listed on the target,
//! which keeps refs isolated between sessions; the listing of the screen a
//! timeout prints, a `wait_timeout`'s or an `element_not_found`'s, is one
//! of them (real-app-fixes, design D4). A ref whose element is gone, which
//! an action or a query names, stands for its twin when the replica holds
//! one (`Mirror::twin`), the element the app rebuilt in its place, whose
//! ref the session's refs gain and the reply names in `resolved`
//! (action-verdicts, design D3); without one it fails `stale_ref`. A
//! selector an action or a query names resolves any element the replica
//! holds, the first on screen and not covered before the rest, and waits up
//! to the command's timeout only for one the tree does not hold yet
//! (rn-app-fixes, designs D51 and D53): an action on a covered element, or
//! one out of view, then takes the path an action on its ref takes and is
//! refused naming the cover. Either first brings the replica up to the
//! display (`Attached::catch_up`), so an element is aimed at where it is.
//!
//! A wait for an exact text on screen, and the look of `scroll --until`,
//! go through the full listing, where every element on screen and not
//! covered is listed with its computed name and value, and wait between
//! looks while the mirror keeps the replica current.

use std::time::{Duration, Instant};

use mirror::{
    Condition, Element, MirrorError, Ref, Role, Selector, Snapshot, SnapshotOptions,
    SnapshotSummary, Target, WaitOutcome,
};
use serde_json::{Value, json};

use crate::wire::code::{self, ELEMENT_NOT_FOUND};
use crate::wire::{Failure, Success};
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

/// A stale ref that stood for its twin (design D3 of action-verdicts): the
/// ref the request named and the twin's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub stale: Ref,
    pub twin: Ref,
}

/// `success` with the twin its request's stale ref stood for, when it
/// stood for one: `resolved`, `{"stale": "@e221", "ref": "@e240"}`.
pub(crate) fn with_resolved(mut success: Success, resolved: Option<Resolved>) -> Success {
    if let (Some(resolved), Value::Object(data)) = (resolved, &mut success.data) {
        data.insert(
            "resolved".into(),
            json!({"stale": format!("@{}", resolved.stale), "ref": format!("@{}", resolved.twin)}),
        );
    }
    success
}

/// The ref an action or a query on `reference`, which `session`'s
/// snapshots must have listed, goes to: `reference` while it names its
/// element, else, once it is stale, its twin's ([`mirror::Mirror::twin`]),
/// which the session's refs gain, with the resolution. A stale ref without
/// a twin fails `stale_ref`, as the mirror words it; any other failure to
/// resolve the ref is left to the command, which meets it where it acts.
fn current(
    target: &mut Attached,
    session: &str,
    reference: Ref,
) -> Result<(Ref, Option<Resolved>), Failure> {
    check_seen(target, session, reference)?;
    let stale = match target.mirror.resolve(reference) {
        Err(stale @ MirrorError::StaleRef { .. }) => stale,
        _ => return Ok((reference, None)),
    };
    let Some(twin) = target
        .mirror
        .twin(reference)
        .and_then(|twin| twin.reference)
    else {
        return Err(stale.into());
    };
    target.session(session).seen.insert(twin);
    Ok((
        twin,
        Some(Resolved {
            stale: reference,
            twin,
        }),
    ))
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
/// `timeout`; a timeout becomes `element_not_found` with the screen seen,
/// whose refs go to `session`.
fn find(
    target: &mut Attached,
    session: &str,
    selector: &Selector,
    timeout: Duration,
) -> Result<Element, Failure> {
    find_in(target, session, (selector, Condition::Appear), timeout)
}

/// The first element that `selector` matches as `condition` looks for it,
/// waiting up to `timeout`; a timeout becomes `element_not_found` with the
/// screen seen, whose refs go to `session` ([`summary`]).
fn find_in(
    target: &mut Attached,
    session: &str,
    (selector, condition): (&Selector, Condition),
    timeout: Duration,
) -> Result<Element, Failure> {
    match target.mirror.wait(selector, condition, timeout) {
        Ok(outcome) => outcome
            .element
            .ok_or_else(|| Failure::internal("a wait for an element returned none")),
        Err(MirrorError::WaitTimeout { waited, .. }) => {
            let summary = summary(target, session)?;
            Err(Failure::new(
                ELEMENT_NOT_FOUND,
                format!(
                    "no element matches {selector} after {} ms; the screen shows {summary}",
                    waited.as_millis()
                ),
            ))
        }
        Err(error) => Err(error.into()),
    }
}

/// Waits until an element matching `selector` meets `condition`, as the
/// mirror's wait does, up to `timeout`; its timeout fails with
/// `wait_timeout` and the screen seen, whose refs go to `session`
/// ([`summary`]).
pub(crate) fn wait(
    target: &mut Attached,
    session: &str,
    (selector, condition): (&Selector, Condition),
    timeout: Duration,
) -> Result<WaitOutcome, Failure> {
    match target.mirror.wait(selector, condition, timeout) {
        Ok(outcome) => Ok(outcome),
        Err(MirrorError::WaitTimeout { waited, .. }) => {
            let summary = summary(target, session)?;
            Err(Failure::new(
                mirror::code::WAIT_TIMEOUT,
                format!(
                    "waited {} ms for {selector} to {}; the screen now shows {summary}",
                    waited.as_millis(),
                    condition.name()
                ),
            ))
        }
        Err(error) => Err(error.into()),
    }
}

/// The element an argument names as the replica holds it, in view or not,
/// as a query reads it (rn-app-fixes, design D51): a ref's element, or its
/// twin's for a stale ref ([`current`]), with the resolution, or the first
/// the replica holds that a selector matches, waiting up to `timeout` for
/// one; a timeout becomes `element_not_found` with the screen seen. On
/// the fourth production-app run's feed scrolled to its end, `get box` on rows
/// scrolled above, which the tree held, waited out its 10 s for a match on
/// screen.
pub(crate) fn held(
    target: &mut Attached,
    session: &str,
    aim: &Aim,
    timeout: Duration,
) -> Result<(Element, Option<Resolved>), Failure> {
    target.catch_up()?;
    match aim {
        Aim::Ref(reference) => {
            let (reference, resolved) = current(target, session, *reference)?;
            Ok((target.mirror.resolve(reference)?, resolved))
        }
        Aim::Select(selector) => {
            find_in(target, session, (selector, Condition::Attached), timeout).map(|e| (e, None))
        }
        Aim::Exact(text) => {
            let selector = exact(text);
            find_in(target, session, (&selector, Condition::Attached), timeout).map(|e| (e, None))
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

/// The switch an element stands for, as the full listing holds it
/// (real-app-fixes, design D6): `click` presses its control
/// ([`Switch::control`]), and `get value` and `get text` read it.
pub(crate) struct Switch {
    listing: Vec<Element>,
    at: usize,
}

impl Switch {
    /// The switch itself.
    pub(crate) fn element(&self) -> &Element {
        &self.listing[self.at]
    }

    /// The control the switch draws: the first switch the listing lists
    /// below it, as the Simulator's reader gives UIKit's switch cells and
    /// SwiftUI's toggles, or the switch itself.
    pub(crate) fn control(&self) -> &Element {
        let control = below(&self.listing, self.at)
            .find(|&k| self.listing[k].role == Role::Switch)
            .unwrap_or(self.at);
        &self.listing[control]
    }
}

/// The places of the elements `elements` lists below the one at `at`.
fn below(elements: &[Element], at: usize) -> impl Iterator<Item = usize> + '_ {
    let depth = elements[at].depth;
    (at + 1..elements.len()).take_while(move |&k| elements[k].depth > depth)
}

/// The switch `element` stands for, from the full listing (real-app-fixes,
/// design D6): `element` itself when it is a switch, else the first switch
/// below it that shares its identifier, as the table cell around Settings'
/// Bold Text switch, else the nearest switch above it that shares it, as
/// the label inside that switch. `None` when `element` is no switch and no
/// switch shares its identifier so, or the listing does not hold it. In
/// task 3.3 the identifier the switch shares named the table cell around
/// it, whose centre, on the label, flipped it 0 times in 10; in the second
/// pass on real apps `get value` by that identifier read the cell, which
/// holds no state, while the switch was on.
pub(crate) fn switch_of(
    target: &mut Attached,
    element: &Element,
) -> Result<Option<Switch>, Failure> {
    if element.role != Role::Switch && element.identifier.is_none() {
        return Ok(None);
    }
    let listing = target.mirror.snapshot(&SnapshotOptions::full())?.elements;
    let Some(at) = listing.iter().position(|listed| listed.id == element.id) else {
        return Ok(None);
    };
    let shares = |k: &usize| {
        listing[*k].role == Role::Switch && listing[*k].identifier == element.identifier
    };
    let switch = if element.role == Role::Switch {
        Some(at)
    } else {
        below(&listing, at).find(shares).or_else(|| {
            let mut depth = listing[at].depth;
            (0..at)
                .rev()
                .filter(|&k| {
                    let above = listing[k].depth < depth;
                    if above {
                        depth = listing[k].depth;
                    }
                    above
                })
                .find(shares)
        })
    };
    Ok(switch.map(|at| Switch { listing, at }))
}

/// The element an argument names, as the replica shows it now: for a stale
/// ref its twin's ([`current`]), with the resolution.
pub(crate) fn element(
    target: &mut Attached,
    session: &str,
    aim: &Aim,
    timeout: Duration,
) -> Result<(Element, Option<Resolved>), Failure> {
    target.catch_up()?;
    match aim {
        Aim::Ref(reference) => {
            let (reference, resolved) = current(target, session, *reference)?;
            Ok((target.mirror.resolve(reference)?, resolved))
        }
        Aim::Select(selector) => find(target, session, selector, timeout).map(|e| (e, None)),
        Aim::Exact(text) => find_exact(target, session, text, timeout).map(|e| (e, None)),
    }
}

/// Where an action lands, as [`action_target`] gives it: where the tap
/// goes, the ref it names, the element a selector matched and the twin a
/// stale ref stood for.
pub(crate) type Landing = (Target, Option<Ref>, Option<Element>, Option<Resolved>);

/// Where an action on an argument lands: a ref, which the mirror checks
/// for being stale, covered or scrolled away, the twin's for a stale ref
/// ([`current`]), with the resolution, or for a selector the center of the
/// element it matched among those the replica holds, on screen or not
/// ([`find_held`]); with the element a selector matched.
pub(crate) fn action_target(
    target: &mut Attached,
    session: &str,
    aim: &Aim,
    timeout: Duration,
) -> Result<Landing, Failure> {
    target.catch_up()?;
    let (aimed, reference, found) = match aim {
        Aim::Ref(reference) => {
            let (reference, resolved) = current(target, session, *reference)?;
            return Ok((Target::Ref(reference), Some(reference), None, resolved));
        }
        Aim::Select(selector) => aimed_at(find_held(target, session, selector, timeout)?),
        Aim::Exact(text) => aimed_at(find_held(target, session, &exact(text), timeout)?),
    };
    Ok((aimed, reference, found, None))
}

/// The element an action's selector names, as the replica holds it
/// (rn-app-fixes, design D53): the first on screen and not covered that
/// `selector` matches, else the first the replica holds, in view or not,
/// within one read, as a query finds it; only for an element the tree does
/// not hold yet does it wait, up to `timeout`, a timeout becoming
/// `element_not_found` with the screen seen. On the fifth production-app run a
/// click by text on a text a button covered whole, a click by identifier on
/// a control a footer's view held and a fill by identifier on a field
/// behind a dialog waited out their 10 s for the element on screen and
/// failed `element_not_found`, where their refs answered `element_covered`
/// in 51 to 93 ms.
fn find_held(
    target: &mut Attached,
    session: &str,
    selector: &Selector,
    timeout: Duration,
) -> Result<Element, Failure> {
    find_in(target, session, (selector, Condition::Attached), timeout)
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

/// Where text for the element an argument names goes, for text entry, as
/// `action`, the command given `given`, enters it: its ref, which a
/// selector's element, found among those the replica holds ([`find_held`]),
/// gets from an interactive listing if no snapshot listed it, once a tap
/// reaches it: one covered, or out of view, is refused as a tap on it is
/// (`act::within_reach`, rn-app-fixes design D53). An identifier that names
/// a node taking no text and holding exactly one field gives the field's
/// ref, with the node it named (design D39): on the production-app run's code
/// step the identifier named a React Native code input's view, around a
/// field without an area. One that names such a node holding no field and
/// without a ref gives the node, for the input a tap on it focuses (design
/// D44): on the third production-app run's second visit the code input's view
/// held nothing, the app's input not in the tree. A stale ref gives its
/// twin's ([`current`]), with the resolution.
pub(crate) fn text_ref(
    target: &mut Attached,
    session: &str,
    (aim, given, action): (&Aim, &str, &str),
    timeout: Duration,
) -> Result<(TextTarget, Option<Resolved>), Failure> {
    target.catch_up()?;
    match aim {
        Aim::Ref(reference) => {
            let (reference, resolved) = current(target, session, *reference)?;
            Ok((TextTarget::Field(reference), resolved))
        }
        Aim::Select(selector) => {
            selected_text_ref(target, session, (selector, given, action), timeout)
                .map(|aimed| (aimed, None))
        }
        Aim::Exact(text) => {
            let element = find_held(target, session, &exact(text), timeout)?;
            super::act::within_reach(target, action, &element, given)?;
            let reference = listed_ref(target, &element, &format!("exact text {text:?}"))?;
            Ok((TextTarget::Field(reference), None))
        }
    }
}

/// Where text for the element `selector` names goes, as [`text_ref`] has
/// it.
fn selected_text_ref(
    target: &mut Attached,
    session: &str,
    (selector, given, action): (&Selector, &str, &str),
    timeout: Duration,
) -> Result<TextTarget, Failure> {
    let element = find_held(target, session, selector, timeout)?;
    super::act::within_reach(target, action, &element, given)?;
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
/// do, whose refs go to `session` ([`summary`]).
pub(crate) fn wait_exact(
    target: &mut Attached,
    session: &str,
    (text, condition): (&str, Condition),
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
            let summary = summary(target, session)?;
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
fn find_exact(
    target: &mut Attached,
    session: &str,
    text: &str,
    timeout: Duration,
) -> Result<Element, Failure> {
    match wait_exact(target, session, (text, Condition::Appear), timeout) {
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

/// The screen as a timeout shows it: the replica's revision, its windows,
/// the elements with refs and the first lines of the interactive listing,
/// whose refs go to `session`, the listing becoming its last snapshot as a
/// snapshot does, so that a ref the timeout prints is one the session can
/// act on (real-app-fixes, design D4).
fn summary(target: &mut Attached, session: &str) -> Result<SnapshotSummary, Failure> {
    let snapshot: Snapshot = target.mirror.snapshot(&SnapshotOptions::interactive())?;
    let lines: Vec<String> = snapshot.text.lines().map(str::to_owned).collect();
    let age = snapshot
        .windows
        .iter()
        .map(|window| window.source.age)
        .min()
        .unwrap_or_default();
    let summary = SnapshotSummary {
        revision: snapshot.revision,
        age,
        windows: snapshot.windows.len(),
        elements: snapshot.refs().len(),
        omitted: lines.len().saturating_sub(SUMMARY_LINES),
        lines: lines.into_iter().take(SUMMARY_LINES).collect(),
        verdict: snapshot.verdict.clone(),
    };
    super::view::remember(target, session, snapshot);
    Ok(summary)
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
