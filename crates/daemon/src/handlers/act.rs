//! Actions: taps, text, keys and gestures, injected through the mirror so
//! it knows to re-read windows that send no change events.

use std::time::{Duration, Instant};

use mirror::backend::Release;
use mirror::{
    Action, Condition, DEFAULT_QUIET_WINDOW, Element, ElementId, MirrorError, Point, Rect, Ref,
    Role, Selector, SettleOptions, SnapshotOptions, Target, Timestamps, Visibility,
};
use protocol::message::TextMethod;
use serde_json::{Map, Value, json};

use super::aim::{self, Aim, Resolved, TextTarget};
use super::view::element_json;
use super::{Call, millis, timeout};
use crate::driver::{Platform, TargetClass};
use crate::keys;
use crate::wire::code::{
    self, COVERED_BY_KEYBOARD, INPUT_REJECTED, STROKE_TOO_SHORT, TEXT_MISMATCH,
};
use crate::wire::{Failure, Success, arg, needed};
use crate::worker::{Attached, Outcome};

/// How long a swipe takes when the command names no duration, and each of
/// a scroll's strokes.
const SWIPE_DURATION: Duration = Duration::from_millis(::wire::SWIPE_MS);

/// The least of the scrolled area a scroll strokes through with the keyboard
/// up, along the scroll's axis, in points (design D41): less, and what the
/// keyboard and a header pinned over the list leave holds no stroke that
/// moves the list's content, and the scroll asks to close the keyboard.
const STROKE_ROOM: f64 = 120.0;

/// The longest stroke of a scroll, as a share of the scrolled area.
const MAX_STROKE: f64 = 0.8;

/// The most strokes a scroll takes.
const MAX_STROKES: u32 = 20;

/// What a scroll leaves out rather than make a stroke of it, in points.
const SUB_POINT: f64 = 0.5;

/// The longest stroke `platform` still takes for a tap, in points; a
/// scroll's strokes must be longer. Android's is its touch slop,
/// `config_viewConfigurationTouchSlop`, which the API 36 emulator's
/// framework declares as 8 dp (the mirror's points on Android); on the
/// Compose fixture's list, strokes of up to 8 points opened the row they
/// started on and 9 points scrolled. UIKit's scroll views start to pan
/// only past 10 points: on the Simulator, strokes of up to 10 points opened
/// the Expo fixture's row and 11 points scrolled.
fn tap_slop(platform: Platform) -> f64 {
    match platform {
        Platform::Android => 8.0,
        Platform::Ios => 10.0,
    }
}

/// How `platform` is named in messages.
fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Android => "Android",
        Platform::Ios => "iOS",
    }
}

/// Points as messages give them: to a tenth.
fn points(value: f64) -> String {
    format!("{}", (value * 10.0).round() / 10.0)
}

/// How a scroll of `amount` points goes when a stroke may be `longest`
/// points at most: as few strokes as fit, all of one length, so no short
/// last stroke is left over, and at most [`MAX_STROKES`] of them. A
/// remainder of [`SUB_POINT`] or less is left out.
fn split(amount: f64, longest: f64) -> (u32, f64) {
    let count = ((amount - SUB_POINT) / longest)
        .ceil()
        .clamp(1.0, f64::from(MAX_STROKES));
    // At most MAX_STROKES, so the cast keeps the value.
    let strokes = count as u32;
    (strokes, (amount / count).min(longest))
}

/// Refuses strokes of `step` points that `platform` takes for taps; the
/// scroll asked for `amount` points in strokes of at most `longest`.
fn check_stroke(platform: Platform, step: f64, amount: f64, longest: f64) -> Result<(), Failure> {
    let slop = tap_slop(platform);
    if step > slop {
        return Ok(());
    }
    let name = platform_name(platform);
    let message = if longest <= slop || amount > slop {
        format!(
            "the scrolled area takes strokes of {} points at most (80% of its extent), no longer than the {} points {name} takes for a tap, so they would tap what lies under them; scroll the screen or a larger element",
            points(longest),
            points(slop),
        )
    } else {
        format!(
            "a scroll stroke of {} points is no longer than the {} points {name} takes for a tap, so it would tap what lies under it; scroll more than {} points",
            points(step),
            points(slop),
            points(slop),
        )
    };
    Err(Failure::new(STROKE_TOO_SHORT, message).with_data(json!({"stroke": step, "tapSlop": slop})))
}

/// How long a tap's effects are watched for at most.
const WATCH_MAX: Duration = Duration::from_secs(1);

/// How long the watch waits for a tap's first effect when the tap was the
/// first input of a new input session: on a Simulator the first tap after
/// dtuhidd activated changed the screen more than 300 ms after its
/// dispatch, where a warm tap's first frame follows in tens of ms.
const FRESH_FIRST_CHANGE: Duration = Duration::from_millis(1500);

/// How long such a tap's effects are watched for at most.
const FRESH_WATCH_MAX: Duration = Duration::from_millis(2500);

/// How often the watch looks at what arrived; the spans come from the
/// signals' own times, not from these looks.
const WATCH_SLICE: Duration = Duration::from_millis(5);

/// The most strokes a scroll that looks for an element takes.
const UNTIL_STROKES: u32 = 50;

/// How long the screen may take to settle after each stroke of a scroll
/// that looks for an element, so a check never races a moving list.
const UNTIL_SETTLE: Duration = Duration::from_millis(1500);

/// Strokes in a row that change nothing before a scroll that looks for an
/// element gives up: the content has stopped moving.
const STILL_STROKES: u32 = 2;

/// The least share of an element in view, its centre in view too, at which
/// a scroll that looks for it stops.
const MOSTLY: f64 = 0.5;

/// How far past the edge it came over a stroke that brings a partly shown
/// element in carries it, in points.
const MARGIN: f64 = 8.0;

/// The fastest a stroke that brings a partly shown element in moves the
/// finger, in points a second. The stroke comes to rest before the finger
/// lifts, as every stroke of a scroll that looks for an element does; were
/// the platform still to take the finger as moving when it lifts, the
/// content would glide on the farther the faster the finger moved, and a
/// glide that carried the element out of view again would undo the stroke.
const BRING_IN_SPEED: f64 = 400.0;

/// The most of what the scroll container shows along the scroll's axis
/// that a stroke looking for an element carries the content. Between two
/// looks the content moves at most three quarters of what the container
/// shows, so at one look at least the element shows whole, or at least as
/// much of it as a quarter of what the container shows; the next stroke
/// then brings it in (see [`decide`]). A stroke whose
/// finger lifts moving carried the fixtures' lists farther than they show:
/// a 600-point stroke over 300 ms, 1,103 to 1,319 points on the API 36
/// emulator, where they show about 780, and the Expo fixture's 1,522 to
/// 1,608 on the iOS 26.5 Simulator, where it shows about 730. So every
/// stroke comes to rest before the finger lifts, and the content goes no
/// farther than the finger.
const REACH: f64 = 0.75;

/// Edges within half a point of each other meet.
const EDGE: f64 = 0.5;

/// What a tap set off, from the moment of its dispatch.
struct Spans {
    dispatch: Duration,
    /// To the capture of the first frame whose hash changed.
    frame_change: Option<Duration>,
    /// To the mirror's first tree delta.
    tree_delta: Option<Duration>,
    /// How long the effects were watched.
    watched: Duration,
    /// Why the watch stopped early: it failed, or a cancel ended it.
    problem: Option<MirrorError>,
}

/// Watches the effects of a tap dispatched at `start`, when the replica
/// was at `revision`: until the first frame change (when the target has
/// frames) and the first tree delta have both arrived, or until nothing has
/// changed for the settle quiet window since the tap or the last change,
/// or for [`WATCH_MAX`]. After a `fresh` activation of the input transport
/// the first change is awaited up to [`FRESH_FIRST_CHANGE`] and the watch
/// lasts up to [`FRESH_WATCH_MAX`]. The mirror keeps reading meanwhile, so
/// a window without change events is re-read as it would be for a
/// snapshot, and times every change: what arrived counts once drained.
fn watch(
    target: &mut Attached,
    start: Instant,
    revision: u64,
    dispatch: Duration,
    fresh: bool,
) -> Spans {
    let (first_wait, max) = if fresh {
        (FRESH_FIRST_CHANGE, FRESH_WATCH_MAX)
    } else {
        (DEFAULT_QUIET_WINDOW, WATCH_MAX)
    };
    let first_frame_change =
        |target: &Attached| target.mirror.frame_changes_since(start).first().copied();
    let tree_delta = |target: &Attached| {
        target
            .mirror
            .deltas()
            .iter()
            .find(|delta| delta.revision > revision)
            .map(|delta| delta.at)
    };
    let mut problem = None;
    let (frame, tree) = loop {
        target.mirror.drain();
        let frame = first_frame_change(target);
        let tree = tree_delta(target);
        if tree.is_some() && (frame.is_some() || !target.frames) {
            break (frame, tree);
        }
        let now = Instant::now();
        // The last tree change covers events, noted window changes and the
        // reads that found a delta, which all came after the tap's dispatch
        // when they came after `start`.
        let stamps = target.mirror.timestamps();
        let since_start = |at: Option<Instant>| at.filter(|&at| at >= start);
        let last_change =
            since_start(stamps.last_frame_change).max(since_start(stamps.last_tree_change));
        // Quiet since the tap: the first change may still come; quiet since
        // a change: the effects are over.
        let quiet_until = match last_change {
            Some(at) => at.max(start) + DEFAULT_QUIET_WINDOW,
            None => start + first_wait,
        };
        let deadline = start + max;
        if now >= quiet_until || now >= deadline {
            break (frame, tree);
        }
        if let Err(error) = target.mirror.run_until((now + WATCH_SLICE).min(deadline)) {
            problem = Some(error);
            target.mirror.drain();
            break (first_frame_change(target), tree_delta(target));
        }
    };
    let span = |at: Option<Instant>| at.map(|at| at.saturating_duration_since(start));
    Spans {
        dispatch,
        frame_change: span(frame),
        tree_delta: span(tree),
        watched: start.elapsed(),
        problem,
    }
}

fn rejected(what: &str) -> Failure {
    Failure::new(
        INPUT_REJECTED,
        format!("the target did not accept the {what}; take a snapshot to see the screen"),
    )
}

fn at(point: Point) -> Value {
    json!([point.x, point.y])
}

fn reference_json(reference: Option<mirror::Ref>) -> Value {
    reference.map_or(Value::Null, |r| format!("@{r}").into())
}

/// How much of `element`, which `given` names, is in view before `action`,
/// the command, taps it, and where the tap goes: the centre of the largest
/// part of it a tap reaches, inside its visible bounds and under no view
/// drawn over it, at least 8 points on each side (design D37), which the
/// mirror aims a tap on a ref at, and a tap aimed at a point here, as for
/// an element a selector found without a ref. A covered element is not a
/// matter of bounds out of date: a modal, or a view drawn over all of it,
/// or over so much of it that no such part is left (design D31, D38),
/// takes the tap, so the mirror refuses a tap on its ref with
/// `element_covered`, naming what a tap at its centre would land on, and
/// nothing is read again or tapped; a point aimed at one fails the same
/// way here. Nothing a tap reaches of an uncovered element can be bounds
/// the replica holds from before a layout change that no event reported and
/// no read has seen yet: in the production-app smoke test the keyboard moved
/// a submit button up, the replica kept its old bounds under the keyboard,
/// and the click tapped the number pad there (design D32). So the
/// element's window is read again once, and the tap goes where that read
/// has the element. The mirror reads a window on demand only after text it
/// entered (task 10.29), so the read covers every window, as every read of
/// a Simulator's reader does. Fails with `element_offscreen`, naming the
/// element, when nothing of it a tap reaches is in view after the read, and
/// with `element_not_found` when a selector's element has left the screen.
fn sight(
    target: &mut Attached,
    action: &str,
    element: &Element,
    given: &str,
    aimed: &mut Target,
) -> Result<Option<Visibility>, Failure> {
    if let Some(refused) = covered(target, element, given, aimed, false) {
        return refused;
    }
    let Some(seen) = target.mirror.visibility(element.id) else {
        return Ok(None);
    };
    if let Some(part) = seen.reachable {
        if let Target::Point(point) = aimed {
            *point = part.center();
        }
        return Ok(Some(seen));
    }
    target.mirror.note_window_change();
    target.mirror.pump()?;
    if let Some(refused) = covered(target, element, given, aimed, true) {
        return refused;
    }
    let Some(seen) = target.mirror.visibility(element.id) else {
        return match aimed {
            // The mirror fails the tap with stale_ref.
            Target::Ref(_) => Ok(None),
            Target::Point(_) => Err(Failure::new(
                code::ELEMENT_NOT_FOUND,
                format!(
                    "{given} left the screen when its window was read again; take a snapshot to see the screen"
                ),
            )),
        };
    };
    let Some(part) = seen.reachable else {
        return Err(offscreen(target, action, element, given, &seen));
    };
    if let Target::Point(point) = aimed {
        *point = part.center();
    }
    Ok(Some(seen))
}

/// Refuses `element`, which `given` names for text that `action`, the
/// command, enters, when a selector found it where no tap reaches it,
/// covered or out of view, as the path of a tap on it ([`sight`]) refuses
/// it: with `element_covered` naming what a tap at its centre would land on,
/// or with `element_offscreen` once its window was read again (rn-app-fixes,
/// design D53). An element the read brings into view is let through.
pub(super) fn within_reach(
    target: &mut Attached,
    action: &str,
    element: &Element,
    given: &str,
) -> Result<(), Failure> {
    let reached = target
        .mirror
        .visibility(element.id)
        .is_some_and(|seen| seen.reachable.is_some());
    if reached {
        return Ok(());
    }
    let mut aimed = Target::Point(element.bounds.center());
    sight(target, action, element, given, &mut aimed).map(drop)
}

/// The refusal of a tap on `element`, which `given` names, aimed as
/// `aimed`, when something covers it ([`mirror::Mirror::covered_by`]): for a
/// ref the mirror's own, which `sight` leaves to it; for a point,
/// `element_covered` naming what a tap at the element's centre would land
/// on, with advice to bring the element out from under it rather than to
/// press it, which on a consent screen accepts the consent. The keyboard
/// covers nothing until the windows were `read_again`: its coming up moves
/// a layout without an event (design D32), and the read finds the element
/// where it is.
fn covered(
    target: &Attached,
    element: &Element,
    given: &str,
    aimed: &Target,
    read_again: bool,
) -> Option<Result<Option<Visibility>, Failure>> {
    let by = target.mirror.covered_by(element.id)?;
    if !read_again && by == mirror::KEYBOARD {
        return None;
    }
    Some(match aimed {
        Target::Ref(_) => Ok(None),
        Target::Point(_) => Err(Failure::new(
            mirror::code::ELEMENT_COVERED,
            format!(
                "{given} lies under the {by}, which a tap there would land on; scroll it out from under the {by}, or close what covers it, then take a new snapshot"
            ),
        )),
    })
}

/// The failure of `action`, the command, on `element`, which `given` names,
/// nothing of which a tap reaches in view as `seen` has it after its window
/// was read again, none of it or too little for a tap: the message names
/// the element, what keeps it out of view and how to bring it in.
fn offscreen(
    target: &Attached,
    action: &str,
    element: &Element,
    given: &str,
    seen: &Visibility,
) -> Failure {
    let named = if element.name.is_empty() {
        format!("{given} ({})", element.role)
    } else {
        format!("{given} ({} {:?})", element.role, element.name)
    };
    let keyboard = target
        .mirror
        .uncovered(seen.bounds, Some(element.id))
        .keyboard;
    // A sliver in view tells the list it lies in where the rest does not.
    let shown = seen.bounds.intersect(&seen.viewport);
    let centre = if shown.is_empty() {
        seen.bounds.center()
    } else {
        shown.center()
    };
    let scrolls = target.mirror.scroll_viewport(centre, true).is_some()
        || target.mirror.scroll_viewport(centre, false).is_some();
    let close = match target.info.platform {
        Platform::Android => "press Back to close the keyboard",
        Platform::Ios => "close the keyboard, as the field's return key does in most apps",
    };
    let sliver = seen.fraction > 0.0;
    let covers = if sliver {
        "the keyboard covers all but a sliver of it"
    } else {
        "the keyboard covers all of it"
    };
    let lies = if sliver {
        "too little of it for a tap lies"
    } else {
        "none of it lies"
    };
    let (why, advice) = match (keyboard, scrolls) {
        (true, true) => (
            covers.to_owned(),
            format!("{close}, or scroll --until {given} brings it above the keyboard"),
        ),
        (true, false) => (covers.to_owned(), close.to_owned()),
        (false, true) => (
            format!("{lies} in what shows of its list"),
            format!("scroll --until {given} brings it into view"),
        ),
        (false, false) => (
            format!("{lies} in what shows of its window"),
            "take a snapshot to see what lies over it".to_owned(),
        ),
    };
    Failure::new(
        mirror::code::ELEMENT_OFFSCREEN,
        format!(
            "{named} is out of view: {why}, also when its window was read again, so a tap there would not reach it; {advice}, then {action} it again"
        ),
    )
    .with_data(json!({"ref": reference_json(element.reference), "visible": 0.0}))
}

/// How long a click holds a switch on a Simulator (real-app-fixes, design
/// D6): in spike S14 a plain tap, its up within 0.1 ms of its down, flipped
/// neither UIKit's Bold Text switch nor SwiftUI's Animate toggle, a UISwitch
/// underneath, 0 times in 10 each, while holds of 40, 80 and 150 ms flipped
/// both 10 times in 10; 40 ms is the shortest measured.
const SWITCH_HOLD: Duration = Duration::from_millis(40);

/// The control a click on `element` presses on a Simulator
/// (real-app-fixes, design D6): that of the switch `element` stands for
/// ([`aim::switch_of`]), the first switch the full listing lists below it,
/// or the switch itself. `None` when `element` stands for no switch. In
/// spike S14 taps at the centre of Settings' labelled Bold Text switch, 330
/// points wide, landed on its label and flipped it 0 times in 40 at any
/// hold, while the 63-point control inside it flipped 30 times in 30 with a
/// hold.
fn switch_control(target: &mut Attached, element: &Element) -> Result<Option<Element>, Failure> {
    Ok(aim::switch_of(target, element)?.map(|switch| switch.control().clone()))
}

/// Taps the element the request names, or a point, and watches what the tap
/// sets off. On a Simulator a switch, or an element that holds a switch
/// sharing its identifier or that such a switch holds, takes a press held
/// [`SWITCH_HOLD`] on that switch's control ([`switch_control`]), unless the
/// request names its own hold; elsewhere a switch takes a tap, as Android's
/// flip on one (real-app-fixes, design D6). A point off the screen is
/// refused ([`on_the_screen`]). The reply names what the tap landed on
/// ([`landed`]), and the twin a stale ref stood for (`aim::Resolved`).
pub(super) fn click(target: &mut Attached, call: &Call, args: &::wire::Click) -> Outcome {
    let session = call.session.as_str();
    let hold = arg("hold", &args.hold)?.map(Duration::from_millis);
    let (mut aimed, reference, clicked, found, resolved) =
        match (arg("x", &args.x)?, arg("y", &args.y)?) {
            (Some(x), Some(y)) => {
                let point = Point::new(x, y);
                on_the_screen(target, point)?;
                (Target::Point(point), None, format!("{x} {y}"), None, None)
            }
            _ => {
                let given = needed(
                    "click",
                    "target",
                    args.target.as_deref().unwrap_or_default(),
                )?;
                let aim = Aim::parse(given)?;
                let (aimed, reference, found, resolved) =
                    aim::action_target(target, session, &aim, timeout(&args.timeout, call)?)?;
                (aimed, reference, given.to_owned(), found, resolved)
            }
        };
    // The replica first takes in what arrived before the tap, so nothing
    // the tap did not cause is counted as its effect.
    target.mirror.pump()?;
    // How much of the element is in view where the tap goes in; a ref the
    // mirror cannot resolve fails the tap below.
    let element =
        found.or_else(|| reference.and_then(|reference| target.mirror.resolve(reference).ok()));
    // A switch on a Simulator takes a press held on the control it draws
    // (real-app-fixes, design D6).
    let simulator = target.info.class == TargetClass::Simulator;
    let control = match &element {
        Some(element) if simulator => switch_control(target, element)?,
        _ => None,
    };
    let (element, hold) = match control {
        Some(control) => {
            aimed = control
                .reference
                .map_or(Target::Point(control.bounds.center()), Target::Ref);
            (Some(control), hold.or(Some(SWITCH_HOLD)))
        }
        None => (element, hold),
    };
    let seen = match &element {
        Some(element) => sight(target, "click", element, &clicked, &mut aimed)?,
        None => None,
    };
    let revision = target.mirror.revision();
    let started = Instant::now();
    let outcome = target.mirror.act(&Action::Tap {
        target: aimed,
        hold,
    })?;
    let dispatch = started.elapsed();
    if !outcome.accepted {
        return Err(rejected("tap"));
    }
    let point = outcome.at.unwrap_or_default();
    // What the tap landed on, taken before the watch takes in what it changed.
    let (hit, missed) = landed(target, session, point, element.as_ref(), &clicked);
    let fresh = target
        .input_notes
        .as_ref()
        .and_then(|notes| notes.activation_wait())
        .is_some();
    let spans = watch(target, started, revision, dispatch, fresh);
    let mut data = Map::new();
    data.insert("clicked".into(), clicked.as_str().into());
    data.insert("ref".into(), reference_json(reference));
    data.insert("hit".into(), hit);
    data.insert("x".into(), point.x.into());
    data.insert("y".into(), point.y.into());
    data.insert("accepted".into(), outcome.accepted.into());
    data.insert("dispatchMs".into(), millis(spans.dispatch).into());
    // Absent, not zero, when the signal never came or the target has none.
    if let Some(frame) = spans.frame_change {
        data.insert("frameChangeMs".into(), millis(frame).into());
    }
    if let Some(tree) = spans.tree_delta {
        data.insert("treeDeltaMs".into(), millis(tree).into());
    }
    data.insert("watchedMs".into(), millis(spans.watched).into());
    let mut warnings: Vec<String> = missed.into_iter().collect();
    if let Some(seen) = seen {
        // A cut edge names the axis; otherwise lists scroll up and down.
        let vertical = !cut_along(&seen, false);
        if !mostly_in_view(&seen, vertical) {
            warnings.push(format!(
                "{clicked} was not mostly in view ({}), so the tap went to the part of it in view, at {point}; scroll --until {clicked} brings it into view first",
                in_view(&seen)
            ));
        }
    }
    // The tap went in, so the click answers what it did whatever ended
    // the watch, a session's cancel among them (design D7 of mcp-server).
    match spans.problem {
        Some(MirrorError::Cancelled) => warnings.push(format!(
            "the tap went in, and a cancel ended the watch of its effects after {} ms; take a snapshot to see what it did",
            spans.watched.as_millis()
        )),
        Some(problem) => warnings.push(format!(
            "the tap went in, but watching its effects failed: {problem}"
        )),
        None => {}
    }
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    Ok(aim::with_resolved(
        Success::new(Value::Object(data)).warn(warning),
        resolved,
    ))
}

/// What a tap at `point` landed on as the replica held it at dispatch, as a
/// click's reply gives it (design D1 of action-verdicts): the element
/// ([`mirror::Mirror::hit`]) by its role, its name and the ref `session`'s
/// snapshots listed it under, null when they did not, or null for no element
/// with a ref under the point; and, when the click named `aimed`, which
/// `clicked` gives, and the tap landed on an element that is neither it,
/// nor part of it, nor what holds it, the warning that names both. The
/// daemon checked what a tap reaches of `aimed` before the tap, so such a
/// tap met a layout that moved under the point after `aimed` was found, or
/// a view over it that cuts nothing of what a tap reaches of it, as a banner
/// system UI draws in a window of its own, inset from the screen's sides;
/// the tap went in all the same, since the replica can lag the app.
fn landed(
    target: &mut Attached,
    session: &str,
    point: Point,
    aimed: Option<&Element>,
    clicked: &str,
) -> (Value, Option<String>) {
    let Some(hit) = target.mirror.hit(point) else {
        return (Value::Null, None);
    };
    let held = hit
        .reference
        .filter(|reference| target.session(session).seen.contains(reference));
    let missed = aimed
        .filter(|aimed| {
            !target.mirror.holds(aimed.id, hit.id) && !target.mirror.holds(hit.id, aimed.id)
        })
        .map(|aimed| {
            let named = match held {
                Some(reference) => format!("@{reference} ({})", listed_as(&hit)),
                None => listed_as(&hit),
            };
            format!(
                "the tap at {point} landed on {named}, not on {clicked} ({}): the layout moved under the point or a view lies over it; take a snapshot to see what the tap did",
                listed_as(aimed)
            )
        });
    let json = json!({"role": hit.role.as_str(), "name": hit.name, "ref": reference_json(held)});
    (json, missed)
}

/// An element as the listing names it: by its role and, when it has one,
/// its name, as `button "Sign in"` or `keyboard`.
fn listed_as(element: &Element) -> String {
    if element.name.is_empty() {
        element.role.to_string()
    } else {
        format!("{} {:?}", element.role, element.name)
    }
}

/// Refuses `point` when it lies off the screen, in points as a click takes
/// them, with a message that gives the screen's size in points and says a
/// screenshot's pixels are scaled: on the demo pass a model took a 460x1000
/// screenshot's pixels for the 402x874 points of the screen
/// (real-app-fixes, design D5).
fn on_the_screen(target: &mut Attached, point: Point) -> Result<(), Failure> {
    let screen = screen(target)?;
    let inside = |value: f64, limit: f64| (0.0..=limit).contains(&value);
    if inside(point.x, screen.width()) && inside(point.y, screen.height()) {
        return Ok(());
    }
    Err(Failure::new(
        code::INVALID_ARGUMENT,
        format!(
            "({}, {}) lies off the screen, {}x{} points: click takes points, not a screenshot's pixels, which are scaled; a screenshot's reply gives its pixels beside the screen's points",
            points(point.x),
            points(point.y),
            points(screen.width()),
            points(screen.height())
        ),
    ))
}

/// The text `action` enters, which may be empty.
fn text_param(action: &str, text: Option<&str>) -> Result<String, Failure> {
    text.map(str::to_owned).ok_or_else(|| {
        Failure::new(
            code::INVALID_ARGUMENT,
            format!("{action} needs a \"text\" string"),
        )
    })
}

fn method_name(method: Option<TextMethod>) -> Value {
    method
        .and_then(TextMethod::name)
        .map_or(Value::Null, Value::from)
}

/// How long `fill` reads back a field that shows other text than it entered
/// before it takes that text as what the field holds: on the iOS 26.5
/// Simulator a paste showed in the fixtures' fields 84 to 462 ms after
/// Command-V (task 10.23). The mirror reads the window of a field it pasted
/// or set text into again as soon as a frame changes after its last read,
/// until a read shows the field's text changed, and other windows, and a
/// field typed into, no sooner than its minimum interval after their last
/// read.
const READ_BACK_MAX: Duration = Duration::from_secs(1);

/// How long after an entry typed as keys a field must still show the text
/// before `fill` takes it: an app that owns its field's value, as a
/// controlled React Native `TextInput`, can set it again between the keys
/// and after them. The settle's quiet window; agent-device holds typed text
/// 0.2 s.
const READ_BACK_HOLD: Duration = DEFAULT_QUIET_WINDOW;

/// How long after an entry by `method` a field must still show the text
/// before `fill` takes it: [`READ_BACK_HOLD`] after keys, and nothing after
/// text that went in in one piece, pasted on a Simulator or set directly on
/// Android, which leaves an app that sets its field's value again no keys
/// to interleave with.
fn hold_after(method: Option<TextMethod>) -> Duration {
    if method == Some(TextMethod::KEYS) {
        READ_BACK_HOLD
    } else {
        Duration::ZERO
    }
}

/// How often a read-back looks at the field; the mirror applies what the
/// target reports and reads what falls due in between.
const READ_BACK_SLICE: Duration = Duration::from_millis(10);

/// The characters a secure field shows for each of its own: the bullet iOS
/// and Android show, and the black circle of some Android fonts.
const MASKS: [char; 2] = ['\u{2022}', '\u{25CF}'];

/// How a field shows the text `fill` entered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    /// As the text itself.
    Text,
    /// As as many mask characters, a secure field's.
    Length,
}

impl Shown {
    fn name(self) -> &'static str {
        match self {
            Shown::Text => "text",
            Shown::Length => "length",
        }
    }
}

/// How a field whose element is named `name` and that shows `value`
/// compares with the `text` `fill` entered: as the text; as an empty read
/// of a field named as the text, since the Simulator's reader leaves out a
/// value that repeats the placeholder; or as as many mask characters, a
/// secure field's, some of them perhaps showing the text's own. `None` is
/// other text.
fn compare(value: Option<&str>, name: &str, text: &str) -> Option<Shown> {
    let value = value.unwrap_or_default();
    if value == text || (value.is_empty() && text == name) {
        return Some(Shown::Text);
    }
    let mask = |c: char| MASKS.contains(&c);
    let masked = value.chars().count() == text.chars().count()
        && value.chars().any(mask)
        && value
            .chars()
            .zip(text.chars())
            .all(|(shown, typed)| shown == typed || mask(shown));
    masked.then_some(Shown::Length)
}

/// Where an entry puts its text in a field: in place of what the field
/// holds, as `fill` enters it, or where the caret is, after what it holds
/// unless the field selected it, as `type` enters keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placed {
    Over,
    After,
}

/// How a field whose element is named `name`, which showed `before` before
/// `type` entered `text`, shows it once it shows `value` (real-app-fixes,
/// design D3): as the text after what it held, or as the text alone, which
/// replaced what the field selected or a placeholder, as [`compare`] takes
/// it; as as many mask characters as it held and the text has, a secure
/// field's, some perhaps showing their own; or as the words the entry
/// changed, which spell the text, as [`compare_name`] compares a name, for
/// keys that went in between words. `None` is other text.
fn compare_typed(
    value: Option<&str>,
    before: Option<&str>,
    name: &str,
    text: &str,
) -> Option<Shown> {
    let (shown, held) = (value.unwrap_or_default(), before.unwrap_or_default());
    if shown == format!("{held}{text}") {
        return Some(Shown::Text);
    }
    if let Some(compared) = compare(value, name, text) {
        return Some(compared);
    }
    let mask = |c: char| MASKS.contains(&c);
    let whole: Vec<char> = held.chars().chain(text.chars()).collect();
    let masked = shown.chars().count() == whole.len()
        && shown.chars().any(mask)
        && shown
            .chars()
            .zip(whole)
            .all(|(shown, entered)| shown == entered || mask(shown));
    if masked {
        return Some(Shown::Length);
    }
    compare_name(shown, held, text)
}

/// Where a read-back took what a field shows from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// The field's own value.
    Value,
    /// Its descendants' values or texts, joined in order.
    Descendants,
    /// Its accessible name, which changed with the entry.
    Name,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Value => "value",
            Source::Descendants => "descendants",
            Source::Name => "name",
        }
    }
}

/// What a field shows, for a read-back.
#[derive(Debug, PartialEq)]
enum Reading {
    /// Its own value, `None` for none.
    Own(Option<String>),
    /// What its descendants show, their values or texts joined in order.
    Below(String),
    /// Its accessible name, which is not the name it had before the entry.
    Name(String),
    /// Nothing: no value of its own, nothing below it, and its name as it
    /// was before the entry.
    Nothing,
}

/// Whether `value`, the own value of a field that is not a text field,
/// shows the text `fill` entered, `text`, the field's value having been
/// `reference` in the reading the read-back compares with, the one after
/// the emptying or before the entry, or none to compare with (design D56):
/// a value that is no placeholder, as long as the text at least and more
/// than separators and empty boxes' dashes, and that changed with the
/// entry. On the sixth production-app run the code step's node showed "4" with
/// the keyboard up, a single box's placeholder or the last digit an
/// emptying left, which the read-back took for what the field held where
/// the node's name spelled the code.
fn value_shown(value: &str, reference: Option<&str>, text: &str) -> bool {
    let placeholder = spelled(value).is_empty() || value.chars().count() < text.chars().count();
    !placeholder && reference.is_none_or(|held| held != value)
}

/// What `element` shows, for a read-back (design D33), the text `fill`
/// entered being `text`: its own value when it is a text field, whose empty
/// value the platform reports as none, or when its value shows the text,
/// one that changed with the entry and is no placeholder ([`value_shown`],
/// design D56); else its descendants' values or texts, joined in the order
/// of the full listing, as a React Native code input built of boxes keeps
/// its characters in its boxes' texts; else its accessible name when it is
/// not the name it had in the reading compared with, `before`'s, as a React
/// Native code input that spells its digits in its name (design D39); else
/// nothing. A name that did not change is a label, never compared.
/// `before` holds that reading's name and value, the value `None` before
/// any entry, when nothing changed with one yet.
fn reading(
    target: &mut Attached,
    element: &Element,
    (name, value): (&str, Option<&str>),
    text: &str,
) -> Result<Reading, Failure> {
    let text_field = matches!(element.role, Role::Textbox | Role::Searchbox);
    let own = element.value.as_deref().is_some_and(|shown| {
        if text_field {
            !shown.is_empty()
        } else {
            value_shown(shown, value, text)
        }
    });
    if own || text_field {
        return Ok(Reading::Own(element.value.clone()));
    }
    let listing = target.mirror.snapshot(&SnapshotOptions::full())?;
    let below = shown_below(&listing.elements, element.id);
    Ok(if !below.is_empty() {
        Reading::Below(below)
    } else if element.name != name {
        Reading::Name(element.name.clone())
    } else {
        Reading::Nothing
    })
}

/// Whether `c` separates the characters a name spells: a space, a comma, or
/// a dash or an underscore, which code fields show in their empty boxes
/// (design D43).
fn separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, ',' | '-' | '_' | '\u{2010}'..='\u{2015}' | '\u{2212}')
}

/// `text` without its separators.
fn spelled(text: &str) -> String {
    text.chars().filter(|&c| !separator(c)).collect()
}

/// The words of `name`, the runs of characters between separators, as byte
/// ranges.
fn words(name: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (at, c) in name.char_indices() {
        match (separator(c), start) {
            (true, Some(from)) => {
                out.push((from, at));
                start = None;
            }
            (false, None) => start = Some(at),
            _ => {}
        }
    }
    if let Some(from) = start {
        out.push((from, name.len()));
    }
    out
}

/// The words of `name` and how many of them it shares with `before`, the
/// name the field had before an entry, at its start and at its end: the
/// words between are the part the entry changed (design D43).
fn shared(name: &str, before: &str) -> (Vec<(usize, usize)>, usize, usize) {
    let (after, was) = (words(name), words(before));
    let same = |a: &(usize, usize), b: &(usize, usize)| name[a.0..a.1] == before[b.0..b.1];
    let lead = after
        .iter()
        .zip(&was)
        .take_while(|(a, b)| same(a, b))
        .count();
    let room = after.len().min(was.len()) - lead;
    let tail = after
        .iter()
        .rev()
        .zip(was.iter().rev())
        .take(room)
        .take_while(|(a, b)| same(a, b))
        .count();
    (after, lead, tail)
}

/// The part of `name` that an entry changed from `before`, the name the
/// field had before it (design D43): what lies between the words the two
/// names share at their start and those they share at their end, from the
/// first word that differs to the last. Empty when the entry only took
/// words away.
fn changed_part<'a>(name: &'a str, before: &str) -> &'a str {
    let (after, lead, tail) = shared(name, before);
    match after[lead..after.len() - tail] {
        [(from, to)] => &name[from..to],
        [(from, _), .., (_, to)] => &name[from..to],
        [] => "",
    }
}

/// How a field whose name is `name`, `before` before the entry, shows the
/// `text` `fill` entered (design D43): as the text when the part of the name
/// the entry changed spells it, both without their separators, as a code
/// field names its boxes, a dash for each empty one, before its label. That
/// part takes in the words next to it that the names share when they spell
/// the text with it, which the entry wrote again: a code entered over one it
/// starts as. Never the whole name for its own sake, and never by length: a
/// part of bullets, one for each character, tells nothing but how many
/// there are.
fn compare_name(name: &str, before: &str, text: &str) -> Option<Shown> {
    let (after, lead, tail) = shared(name, before);
    let want = spelled(text);
    let spells = |from: usize, to: usize| {
        after[from..to]
            .iter()
            .map(|&(a, b)| &name[a..b])
            .collect::<String>()
            == want
    };
    let end = after.len() - tail;
    (0..=lead)
        .rev()
        .any(|from| (end..=after.len()).any(|to| spells(from, to)))
        .then_some(Shown::Text)
}

/// What a field whose name is `name`, `before` before the entry, holds, as a
/// mismatch names it: the part of the name the entry changed, or only its
/// bullets when it spells nothing else, so the message names them by their
/// number, as it names a masked value.
fn held_in_name(name: &str, before: &str) -> String {
    let part = changed_part(name, before);
    let letters = spelled(part);
    if !letters.is_empty() && letters.chars().all(|c| MASKS.contains(&c)) {
        letters
    } else {
        part.to_owned()
    }
}

/// What the elements of a full listing below the element `id` show, in
/// order: each one's value, else the name of a text, or of a node without
/// traits, that has nothing listed below it.
fn shown_below(elements: &[Element], id: ElementId) -> String {
    let Some(at) = elements.iter().position(|element| element.id == id) else {
        return String::new();
    };
    let depth = elements[at].depth;
    let below: Vec<&Element> = elements[at + 1..]
        .iter()
        .take_while(|element| element.depth > depth)
        .collect();
    let mut shown = String::new();
    for (k, element) in below.iter().enumerate() {
        let leaf = below
            .get(k + 1)
            .is_none_or(|next| next.depth <= element.depth);
        match element.value.as_deref().filter(|value| !value.is_empty()) {
            Some(value) => shown.push_str(value),
            None if leaf && matches!(element.role, Role::Text | Role::Generic) => {
                shown.push_str(&element.name);
            }
            None => {}
        }
    }
    shown
}

/// What a read-back found in the field.
#[derive(Debug, PartialEq)]
enum ReadBack {
    /// It shows the text as `held`, read from `from`, which the read-back
    /// took `after` the entry.
    Shown {
        shown: Shown,
        from: Source,
        after: Duration,
        held: Option<String>,
    },
    /// It showed other text, `held`, `None` for none, read from `from`, until
    /// the read-back's bound, `after` the entry.
    Other {
        held: Option<String>,
        from: Source,
        after: Duration,
    },
    /// It showed nothing to read back, neither a value nor anything below
    /// it, and the read-back stopped looking `after` the entry.
    Unreadable { after: Duration },
    /// It left the screen before it showed the text.
    Gone,
}

/// Whether the replica shows the screen as the app has held it since
/// `hold`: its latest read began at or after `hold`, or `hold` has passed
/// and nothing changed since that read began, neither the tree nor, when
/// `rereads` says windows that send no change events are read again as
/// frames change, a frame.
fn held(stamps: &Timestamps, hold: Instant, now: Instant, rereads: bool) -> bool {
    let Some(read) = stamps.last_read else {
        return false;
    };
    if read.started >= hold {
        return true;
    }
    let changed = (rereads && stamps.last_frame_change.is_some_and(|at| at > read.started))
        || stamps.last_tree_change.is_some_and(|at| at > read.finished);
    now >= hold && !changed
}

/// What a fill reads back: the field a ref names, with what identifies it
/// should the app put a new node in its place ([`Identity`]), or the node
/// an identifier names that takes no text itself, whose tap gave the focus
/// to an input outside the tree (design D44).
enum Field {
    Ref(Ref, Identity),
    Node(Selector),
}

/// What identifies a field once the app has put a new node in its place,
/// which leaves the ref that named it stale: its identifier and role, else
/// its bounds and role (real-app-fixes, task 5.5). On the second pass on
/// real apps a sheet's title field took a new node with the same
/// identifier, role and bounds while `fill` entered its text, every element
/// of the sheet taking a new ref, and the read-back, which followed the
/// ref, said the field had left the screen.
struct Identity {
    identifier: Option<String>,
    role: Role,
    bounds: Rect,
}

impl Identity {
    fn of(element: &Element) -> Identity {
        Identity {
            identifier: element.identifier.clone(),
            role: element.role,
            bounds: element.bounds,
        }
    }

    /// Whether `element` is the field this identifies.
    fn names(&self, element: &Element) -> bool {
        element.role == self.role
            && match &self.identifier {
                Some(identifier) => element.identifier.as_ref() == Some(identifier),
                None => element.bounds == self.bounds,
            }
    }
}

impl Field {
    /// The field `element` is, which `reference` names.
    fn named(reference: Ref, element: &Element) -> Field {
        Field::Ref(reference, Identity::of(element))
    }

    /// The element as the replica shows it now: the one the ref names, or
    /// once the ref is stale the first of the full listing that the field's
    /// identity names; `None` once it is gone.
    fn element(&self, target: &mut Attached) -> Result<Option<Element>, Failure> {
        match self {
            Field::Ref(reference, identity) => match target.mirror.resolve(*reference) {
                Ok(element) => Ok(Some(element)),
                Err(MirrorError::StaleRef { .. }) => {
                    let listing = target.mirror.snapshot(&SnapshotOptions::full())?;
                    Ok(listing
                        .elements
                        .into_iter()
                        .find(|element| identity.names(element)))
                }
                Err(error) => Err(error.into()),
            },
            Field::Node(selector) => {
                match target
                    .mirror
                    .wait(selector, Condition::Appear, Duration::ZERO)
                {
                    Ok(outcome) => Ok(outcome.element),
                    Err(MirrorError::WaitTimeout { .. }) => Ok(None),
                    Err(error) => Err(error.into()),
                }
            }
        }
    }

    /// The ref replies name the field by, if it has one.
    fn reference(&self) -> Option<Ref> {
        match self {
            Field::Ref(reference, _) => Some(*reference),
            Field::Node(_) => None,
        }
    }

    /// Follows the field to its twin once its ref is stale, the field the
    /// app rebuilt in its place (`Mirror::twin`, design D3 of
    /// action-verdicts), whose ref `session`'s refs gain; says which ref it
    /// left, `None` while the ref names the field, when it has no twin, and
    /// for a node.
    fn follow(&mut self, target: &mut Attached, session: &str) -> Option<Ref> {
        let Field::Ref(stale, _) = *self else {
            return None;
        };
        if !matches!(
            target.mirror.resolve(stale),
            Err(MirrorError::StaleRef { .. })
        ) {
            return None;
        }
        let twin = target.mirror.twin(stale)?;
        let reference = twin.reference?;
        target.session(session).seen.insert(reference);
        *self = Field::named(reference, &twin);
        Some(stale)
    }
}

/// Follows `field` to its twin once its ref is stale ([`Field::follow`]),
/// which `resolved` then names beside `named`, the ref the request named
/// the field by, when it named one.
fn follow_field(
    target: &mut Attached,
    session: &str,
    field: &mut Field,
    (named, resolved): (Option<Ref>, &mut Option<Resolved>),
) {
    if field.follow(target, session).is_none() {
        return;
    }
    if let (Some(stale), Some(twin)) = (named, field.reference()) {
        *resolved = Some(Resolved { stale, twin });
    }
}

/// Reads back `field` after an entry by `method` that ended at `entered`,
/// from what it shows ([`reading`]): until the field shows `text` and has
/// held it since the hold after that method ends ([`hold_after`]), which
/// after text that went in in one piece takes the first read that shows it,
/// or until it has shown other text up to [`READ_BACK_MAX`] after the entry,
/// the mirror reading it again as reads fall due meanwhile. What the field
/// shows at the bound stands. A field that shows nothing to read back once
/// the screen has held still since a settle's quiet window after the entry
/// ([`READ_BACK_HOLD`]) is unreadable. `before` is the field's name and
/// value in the reading compared with, as the emptying left them or before
/// the entry (see [`reading`]); its value, for keys `type` placed after what
/// the field held, is what the text adds to ([`compare_typed`]). What the
/// field holds is its value, what lies below it, or the part of its name the
/// entry changed.
fn read_back(
    target: &mut Attached,
    field: &Field,
    text: &str,
    (entered, method, placed): (Instant, Option<TextMethod>, Placed),
    before: (&str, Option<&str>),
) -> Result<ReadBack, Failure> {
    let hold = entered + hold_after(method);
    let quiet = entered + READ_BACK_HOLD;
    let deadline = entered + READ_BACK_MAX;
    loop {
        let Some(element) = field.element(target)? else {
            return Ok(ReadBack::Gone);
        };
        let reading = reading(target, &element, before, text)?;
        let now = Instant::now();
        let after = now.saturating_duration_since(entered);
        let rereads = target.mirror.next_wakeup().is_some();
        let stamps = target.mirror.timestamps();
        let (shown, from, held_now) = match &reading {
            Reading::Own(value) => (
                match placed {
                    Placed::Over => compare(value.as_deref(), &element.name, text),
                    Placed::After => compare_typed(value.as_deref(), before.1, &element.name, text),
                },
                Source::Value,
                value.clone(),
            ),
            Reading::Below(below) => (
                compare(Some(below), &element.name, text),
                Source::Descendants,
                Some(below.clone()),
            ),
            Reading::Name(name) => (
                compare_name(name, before.0, text),
                Source::Name,
                Some(held_in_name(name, before.0)),
            ),
            Reading::Nothing => (None, Source::Value, None),
        };
        match (shown, reading) {
            (Some(shown), _) if now >= deadline || held(&stamps, hold, now, rereads) => {
                return Ok(ReadBack::Shown {
                    shown,
                    from,
                    after,
                    held: held_now,
                });
            }
            (None, Reading::Nothing) if now >= deadline || held(&stamps, quiet, now, rereads) => {
                return Ok(ReadBack::Unreadable { after });
            }
            (None, _) if now >= deadline => {
                return Ok(ReadBack::Other {
                    held: held_now,
                    from,
                    after,
                });
            }
            _ => {}
        }
        let until = if now < hold { hold } else { deadline };
        target
            .mirror
            .run_until((now + READ_BACK_SLICE).min(until))?;
    }
}

/// Whether `fill` types `text` as keys on a target of `class`: a Simulator
/// types a text of at most 32 characters that a US keyboard types and
/// pastes any other (`simulator::TextMethod::resolve`); a physical iPhone's
/// runner sets the text of a field that holds some, and Android sets it.
fn typed_as_keys(class: TargetClass, text: &str) -> bool {
    class == TargetClass::Simulator
        && !text.is_empty()
        && simulator::TextMethod::Auto.resolve(text) == simulator::TextMethod::Keyboard
}

/// Where `field` holds text that keys `fill` types replace, when it holds
/// any and the text goes in as keys ([`typed_as_keys`]), so that `fill`
/// empties it first (design D49): its own value, unless a field that is not
/// a text field shows a placeholder there ([`value_shown`]), what lies below
/// it, or, for a field known only by its name, a name that spells the text
/// already, where keys over the field would leave the name as it was and
/// nothing to read back. `None` for a field that holds nothing, for a
/// paste, and for a field gone.
fn held_text(target: &mut Attached, field: &Field, text: &str) -> Result<Option<Source>, Failure> {
    if !typed_as_keys(target.info.class, text) {
        return Ok(None);
    }
    let Some(element) = field.element(target)? else {
        return Ok(None);
    };
    Ok(
        match reading(target, &element, (&element.name, None), text)? {
            Reading::Own(value) => value
                .is_some_and(|value| !value.is_empty())
                .then_some(Source::Value),
            Reading::Below(_) => Some(Source::Descendants),
            Reading::Name(_) | Reading::Nothing => {
                let want = spelled(text);
                (!want.is_empty() && spelled(&element.name).contains(&want)).then_some(Source::Name)
            }
        },
    )
}

/// How long `fill` waits, once it has emptied a field, for the replica to
/// show it empty before its keys go out (design D49), as long as the
/// Simulator's own wait for an empty read (task 4.5): a field that never
/// reads empty, as one that formats what it holds, gets the keys at the
/// bound.
const EMPTY_WAIT: Duration = Duration::from_secs(1);

/// Waits until the replica shows `field`, emptied at `at`, empty where it
/// held its text, as `source` says: its value empty, nothing below it, or
/// its name other than `before`, a value that shows no text `text` would be
/// aside ([`value_shown`]); at most [`EMPTY_WAIT`]. Returns the field's name
/// and value then, the reading the read-back compares with (designs D49
/// and D56).
fn emptied(
    target: &mut Attached,
    field: &Field,
    (at, source): (Instant, Source),
    (before, text): (&str, &str),
) -> Result<(String, Option<String>), Failure> {
    let deadline = at + EMPTY_WAIT;
    loop {
        let Some(element) = field.element(target)? else {
            return Ok((before.to_owned(), None));
        };
        let empty = match reading(target, &element, (before, None), text)? {
            Reading::Own(value) => value.is_none_or(|value| value.is_empty()),
            Reading::Below(_) => false,
            Reading::Name(_) => true,
            Reading::Nothing => source != Source::Name,
        };
        let now = Instant::now();
        if empty || now >= deadline {
            return Ok((element.name, element.value));
        }
        target
            .mirror
            .run_until((now + READ_BACK_SLICE).min(deadline))?;
    }
}

/// How long `fill` waits on iOS, from the tap that focused a field, for the
/// keyboard's window to show in the replica before it enters the text all
/// the same: a field with an input view of its own shows no keyboard
/// (design D34).
const KEYBOARD_WAIT: Duration = Duration::from_secs(1);

/// How often that wait looks at the replica; the mirror applies what the
/// target reports and reads what falls due in between.
const KEYBOARD_SLICE: Duration = Duration::from_millis(10);

/// Whether the replica shows a keyboard's window over the screen: an
/// input-method window that cuts what a stroke may go through
/// (`Mirror::uncovered`), as one does that shows what the keyboard draws.
fn keyboard_up(target: &mut Attached) -> Result<bool, Failure> {
    let screen = screen(target)?;
    Ok(target.mirror.uncovered(screen, None).keyboard)
}

/// Whether a hardware keyboard serves the input a tap gave the focus, so
/// that it takes keys at once and no keyboard will show (design D52): the
/// replica shows a keyboard window that draws nothing
/// (`Mirror::keys_hidden`), as iOS keeps the keyboard's window, its keys
/// below the screen, while one serves the input in focus, and `field`, the
/// field the tap was for, shows the focus; for an input the tree does not
/// show, `field` being `None`, the window alone tells. On the fifth
/// production-app run, with the Simulator's hardware keyboard connected, `fill`
/// waited out its second for keys that never showed, and the node of the
/// code step that holds no field was refused for want of a keyboard.
fn served_without_keys(target: &mut Attached, field: Option<Ref>) -> bool {
    target.mirror.keys_hidden()
        && field.is_none_or(|field| {
            target
                .mirror
                .resolve(field)
                .is_ok_and(|element| element.focused)
        })
}

/// Whether the tap made at `tapped` has given the focus as far as the
/// replica tells: `field`, the field the tap was for, shows it; a field
/// that takes no text itself, as a code field named by its boxes, or an
/// input the tree does not show, `field` being `None`, cannot, and a read
/// begun after the tap stands for it.
fn focus_taken(target: &mut Attached, tapped: Instant, field: Option<Ref>) -> bool {
    let read_since = target
        .mirror
        .timestamps()
        .last_read
        .is_some_and(|read| read.started >= tapped);
    match field.map(|field| target.mirror.resolve(field)) {
        Some(Ok(element)) if matches!(element.role, Role::Textbox | Role::Searchbox) => {
            element.focused
        }
        Some(Ok(_)) | None => read_since,
        Some(Err(_)) => false,
    }
}

/// Gets the field `reference` names, which `given` names, ready for text on
/// iOS (design D34): a field without the focus, while no keyboard is up, is
/// tapped, and the text waits until the replica shows the keyboard's window,
/// or a keyboard window that draws nothing while the field has the focus, a
/// hardware keyboard serving it (design D52), or, once a keyboard window
/// drawing no keys has been seen, the focus alone (design D57), at most
/// [`KEYBOARD_WAIT`] from the tap ([`keyboard_within`]). In the production-app smoke test the first keys typed into a
/// just-focused field were lost in 3 of 8 runs while the Simulator's
/// keyboard slid in; a paste is Command-V, a key too, so text waits
/// whichever way it goes in. A field with the focus, or a keyboard up
/// already, pays nothing. The tap goes where the field is in view, as a
/// click's does ([`sight`]).
fn ready_keyboard(target: &mut Attached, reference: Ref, given: &str) -> Result<(), Failure> {
    let element = target.mirror.resolve(reference)?;
    if element.focused || keyboard_up(target)? {
        return Ok(());
    }
    let aimed = Target::Ref(reference);
    tap_to_focus(target, (&element, Some(reference)), given, aimed, "fill")?;
    Ok(())
}

/// Taps `element`, which `given` names, as `aimed`, to give the focus to
/// `field`, the field it is or holds, as `action`, the command, does; on iOS
/// waits, as [`ready_keyboard`] does, until the replica shows the keyboard's
/// window, or one that draws nothing while the field has the focus, at most
/// [`KEYBOARD_WAIT`] from the tap.
fn tap_to_focus(
    target: &mut Attached,
    (element, field): (&Element, Option<Ref>),
    given: &str,
    aimed: Target,
    action: &str,
) -> Result<(), Failure> {
    let tapped = tap_for_text(target, element, given, aimed, action)?;
    if target.info.platform == Platform::Ios {
        keyboard_within(target, tapped, field)?;
    }
    Ok(())
}

/// Taps `element`, which `given` names, as `aimed`, where it is in view, as
/// a click does ([`sight`]), for text that `action`, the command, enters;
/// says when.
fn tap_for_text(
    target: &mut Attached,
    element: &Element,
    given: &str,
    mut aimed: Target,
    action: &str,
) -> Result<Instant, Failure> {
    sight(target, action, element, given, &mut aimed)?;
    let tapped = Instant::now();
    let outcome = target.mirror.act(&Action::Tap {
        target: aimed,
        hold: None,
    })?;
    if !outcome.accepted {
        return Err(rejected("tap that focuses the field"));
    }
    Ok(tapped)
}

/// Waits until the replica shows the keyboard's window, at most
/// [`KEYBOARD_WAIT`] from `tapped`, or a keyboard window that draws nothing
/// while `field`, the field the tap gave the focus, shows it, or, for an
/// input the tree does not show, at once ([`served_without_keys`]); whether
/// one came. Once a wait has seen a keyboard window that draws no keys, and
/// until one sees a keyboard with keys, the wait ends at the focus
/// ([`focus_taken`], design D57): under a hardware keyboard a number pad
/// brings up no keyboard window at all, and the sixth production-app run's code
/// fill waited out the second there.
fn keyboard_within(
    target: &mut Attached,
    tapped: Instant,
    field: Option<Ref>,
) -> Result<bool, Failure> {
    let deadline = tapped + KEYBOARD_WAIT;
    loop {
        target.mirror.pump()?;
        if keyboard_up(target)? {
            target.keys_hidden_seen = false;
            return Ok(true);
        }
        if target.mirror.keys_hidden() {
            target.keys_hidden_seen = true;
        }
        if served_without_keys(target, field)
            || (target.keys_hidden_seen && focus_taken(target, tapped, field))
        {
            return Ok(true);
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        target
            .mirror
            .run_until((now + KEYBOARD_SLICE).min(deadline))?;
    }
}

/// Taps `node`, which `given` names and `selector` found, a node that takes
/// no text, holds no field and has no ref, for text that `action`, the
/// command, enters, and waits until the keyboard is up, at most
/// [`KEYBOARD_WAIT`] from the tap (design D44), or a keyboard window that
/// draws nothing comes up, a hardware keyboard serving the input the tap
/// focused (design D52): on the third production-app run's second visit to the
/// code step, the code input's view held nothing, the app's input not in
/// the tree, and a tap on the view gave that input the focus. Fails with
/// `invalid_argument`, as an entry into a node that takes no text does,
/// when no keyboard comes up, and when a field the tree shows has the focus
/// after the tap, as the keyboard then serves that field.
fn raise_keyboard(
    target: &mut Attached,
    (node, selector): (&Element, &Selector),
    given: &str,
    action: &str,
) -> Result<(), Failure> {
    let aimed = Target::Point(node.bounds.center());
    let tapped = tap_for_text(target, node, given, aimed, action)?;
    let refused = |why: String| {
        Failure::new(
            code::INVALID_ARGUMENT,
            format!(
                "{selector} matches a {} that takes no text and holds no field, {why}",
                node.role
            ),
        )
    };
    if !keyboard_within(target, tapped, None)? {
        return Err(refused(format!(
            "and no keyboard came up within {} ms of a tap on it",
            KEYBOARD_WAIT.as_millis()
        )));
    }
    if let Some(focused) = focused_field(target)? {
        return Err(refused(format!(
            "and after a tap on it the keyboard serves the {} {:?}, which has the focus",
            focused.role, focused.name
        )));
    }
    Ok(())
}

/// The field the tree shows with the focus, from the full listing: a text
/// or search field marked focused, which the keyboard serves; `None` when
/// the tree shows none, as for an input outside it.
fn focused_field(target: &mut Attached) -> Result<Option<Element>, Failure> {
    let listing = target.mirror.snapshot(&SnapshotOptions::full())?;
    Ok(listing
        .elements
        .into_iter()
        .find(|element| element.focused && matches!(element.role, Role::Textbox | Role::Searchbox)))
}

/// How long `fill` waits, once it has tapped to close iOS's edit menu, for
/// the replica to show the menu gone.
const MENU_WAIT: Duration = Duration::from_secs(1);

/// Closes iOS's edit menu when the replica shows one up once `fill` has
/// read back its paste into `field` (design D55): the tap on the field in
/// focus that readies a paste opens the menu, and a paste that lands
/// nothing leaves it up over the screen, as on the sixth production-app run,
/// where Command was lost. A tap on the field near its left end closes
/// it, as a tap on the field in focus does on the Simulator, the caret
/// going to the field's start, away from a clear button at its right end;
/// on a field in focus without the menu that tap would open it, so it goes
/// only once the read-back has ended and the replica shows the menu.
/// Waits at most [`MENU_WAIT`] for the menu to go.
fn close_edit_menu(target: &mut Attached, field: &Field) -> Result<(), Failure> {
    if target.info.platform != Platform::Ios || target.mirror.edit_menu().is_none() {
        return Ok(());
    }
    let Some(element) = field.element(target)? else {
        return Ok(());
    };
    let Some(reach) = target
        .mirror
        .visibility(element.id)
        .and_then(|seen| seen.reachable)
    else {
        return Ok(());
    };
    let at = Point::new(
        reach.left + mirror::REACH_FLOOR.min(reach.width()) / 2.0,
        reach.center().y,
    );
    let outcome = target.mirror.act(&Action::Tap {
        target: Target::Point(at),
        hold: None,
    })?;
    if !outcome.accepted {
        return Err(rejected("tap that closes the edit menu"));
    }
    let deadline = Instant::now() + MENU_WAIT;
    loop {
        target.mirror.pump()?;
        let now = Instant::now();
        if target.mirror.edit_menu().is_none() || now >= deadline {
            return Ok(());
        }
        target
            .mirror
            .run_until((now + KEYBOARD_SLICE).min(deadline))?;
    }
}

/// The warning of an entry that an identifier `given` named through `node`,
/// a node that takes no text and holds no field, a tap on which brought the
/// keyboard up (design D44).
fn raised(given: &str, node: &Element) -> String {
    format!(
        "{given} ({} {:?}) takes no text and holds no field, so the text went where a tap on it put the focus, carried by the keyboard it brought up",
        node.role, node.name
    )
}

/// Enters `text` into the field `into` names, or the focused field, as
/// `method` asks; says the method the target used.
fn enter(
    target: &mut Attached,
    into: Option<Ref>,
    text: &str,
    method: TextMethod,
) -> Result<Option<TextMethod>, Failure> {
    let outcome = target.mirror.act(&Action::Text {
        text: text.to_owned(),
        into,
        method,
    })?;
    if !outcome.accepted {
        return Err(rejected("text; fill takes a text field"));
    }
    Ok(outcome.method)
}

/// How `fill` entered its text by `method` on a target of `class`, as
/// messages say: in one piece, a Simulator pastes it, a physical iPhone's
/// runner sets the field's value and Android sets it directly.
fn entered_by(class: TargetClass, method: Option<TextMethod>) -> &'static str {
    if method == Some(TextMethod::KEYS) {
        return "typed the text";
    }
    match class {
        TargetClass::Simulator => "pasted the text",
        TargetClass::IosDevice => "set the text through the runner",
        TargetClass::AndroidEmulator | TargetClass::AndroidDevice => "set the text directly",
    }
}

/// What a field showing `value` holds, as messages name it.
fn described(value: Option<&str>) -> String {
    match value.filter(|value| !value.is_empty()) {
        None => "nothing".to_owned(),
        Some(value) if value.chars().all(|c| MASKS.contains(&c)) => {
            format!("{} masked characters", value.chars().count())
        }
        Some(value) => format!("{value:?}"),
    }
}

/// Why a fill's text was not verified, as its reply's `reason` says: the
/// field showed neither a value nor anything below it.
const NO_VALUE_TO_READ: &str = "no_value_to_read";

/// What a fill's read-back concluded.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Check {
    /// The field showed the text as `shown`, read from `from`, `after` the
    /// entry.
    Verified {
        shown: Shown,
        from: Source,
        after: Duration,
    },
    /// The field showed nothing to read back; the read-back stopped looking
    /// `after` the entry.
    Unverified { after: Duration },
    /// The field was not read back: an empty fill on Android, or a field that
    /// left the screen.
    Skipped,
}

/// A fill's reply: the field as `given`, the `text` as the request gave it,
/// the field's ref, null for a node without one, the method the text the
/// field holds went in by, what the read-back concluded and how long after
/// the entry, and whether `fill` entered the text again. `verified` is how
/// the field showed the text, `false` when it showed nothing to read back
/// (`reason` says so), and null when it was not read back; `readFrom` is
/// where the read-back took it from.
fn filled(
    given: &str,
    text: &str,
    reference: Option<Ref>,
    method: Option<TextMethod>,
    check: Check,
    retried: bool,
) -> Success {
    let (verified, from, reason, after) = match check {
        Check::Verified { shown, from, after } => (
            Value::from(shown.name()),
            Value::from(from.name()),
            Value::Null,
            Some(after),
        ),
        Check::Unverified { after } => (
            Value::Bool(false),
            Value::Null,
            Value::from(NO_VALUE_TO_READ),
            Some(after),
        ),
        Check::Skipped => (Value::Null, Value::Null, Value::Null, None),
    };
    Success::new(json!({
        "filled": given,
        "text": text,
        "ref": reference_json(reference),
        "method": method_name(method),
        "verified": verified,
        "readFrom": from,
        "reason": reason,
        "readBackMs": after.map(millis),
        "retried": retried,
    }))
}

/// Gets `field`, which an identifier `given` named through `holder`, a node
/// that takes no text around it, ready for text, and says which field to
/// enter the text into: a field with an area takes a tap of its own, as any
/// field does ([`ready_keyboard`] on iOS); one without an area, as a React
/// Native code input's, which a tap cannot reach, takes the tap on the
/// node around it, and the text as the field in focus (design D39), as a
/// click on the node and a `type` did in the production-app run.
fn ready_held(
    target: &mut Attached,
    (field, holder): (&Element, &Element),
    given: &str,
    action: &str,
) -> Result<Option<Ref>, Failure> {
    let reference = field.reference;
    if !field.bounds.is_empty() {
        if action == "fill"
            && target.info.platform == Platform::Ios
            && let Some(reference) = reference
        {
            ready_keyboard(target, reference, given)?;
        }
        return Ok(reference);
    }
    if !field.focused {
        let aimed = Target::Point(holder.bounds.center());
        tap_to_focus(target, (holder, field.reference), given, aimed, action)?;
    }
    Ok(None)
}

/// The warning of an entry into `field` that an identifier `given` named
/// through the node around it.
fn held_field(given: &str, field: &Element) -> String {
    let reference = field
        .reference
        .map(|r| format!(" @{r}"))
        .unwrap_or_default();
    format!(
        "{given} takes no text itself, so the text went into the {} {:?}{reference} it holds",
        field.role, field.name
    )
}

/// The warning of `action`, `fill` or `type`, whose field `given` showed
/// nothing to read back.
fn unreadable(given: &str, action: &str) -> String {
    format!(
        "{given} shows neither a value nor anything below it, so {action} could not read its text back; a screenshot shows what it holds"
    )
}

/// The failure of a fill of `text` whose field `given` holds `value` after
/// `fill` entered the text as `tries` say: the message names both, the text
/// by its length where the field shows nothing or mask characters, as a
/// secure field does.
fn mismatch(
    given: &str,
    reference: Option<Ref>,
    text: &str,
    value: Option<&str>,
    tries: &str,
) -> Failure {
    let masked =
        value.is_some_and(|value| !value.is_empty() && value.chars().all(|c| MASKS.contains(&c)));
    let entered = if value.is_some_and(|value| !value.is_empty()) && !masked {
        format!("{text:?}")
    } else {
        format!("{} characters", text.chars().count())
    };
    Failure::new(
        TEXT_MISMATCH,
        format!(
            "{given} holds {} where fill entered {entered}, after it {tries}; a field that limits or formats its text shows it so: take a snapshot to see the field, then fill it again or go on if it holds what you meant",
            described(value)
        ),
    )
    .with_data(json!({"ref": reference_json(reference), "value": value, "masked": masked}))
}

/// Fills the field the request names: enters the text, typed or pasted as
/// the target chooses, and reads the field back, holding typed text
/// [`READ_BACK_HOLD`] and taking text that went in in one piece from the
/// first read that shows it, from the field's value or, for a field without
/// one, through its descendants, or through its name when the entry changed
/// it (see [`reading`]). An identifier that names a node taking no text
/// around exactly one field fills that field, which the reply's warning
/// names (see [`ready_held`]); one that names such a node holding no field
/// and without a ref taps it and, once the keyboard is up, enters the text
/// where the tap put the focus and reads it back through the node (see
/// [`raise_keyboard`]). A field that shows other
/// text gets the text again in one piece, pasted on a Simulator, set through
/// the runner on a physical iPhone and set directly on Android, and is read
/// back again, iOS's edit menu that paste left up closed after
/// (see [`close_edit_menu`]); a field that still shows
/// other text fails the fill with `text_mismatch`. A field that shows
/// nothing to read back, neither a value nor anything below it, leaves the
/// fill unverified, with a warning, and gets the text once. On Android an
/// empty field reads as its hint, so an empty fill there is not read back.
/// On iOS a field without the focus is focused first and gets its text once
/// the keyboard is up (see [`ready_keyboard`]). A stale ref fills the twin
/// it stands for, which the reply names (`aim::Resolved`).
pub(super) fn fill(target: &mut Attached, call: &Call, args: &::wire::Fill) -> Outcome {
    let session = call.session.as_str();
    let text = text_param("fill", args.text.as_deref())?;
    let given = needed("fill", "target", &args.target)?;
    let aim = Aim::parse(given)?;
    let (aimed, mut resolved) = aim::text_ref(
        target,
        session,
        (&aim, given, "fill"),
        timeout(&args.timeout, call)?,
    )?;
    let named = match aim {
        Aim::Ref(reference) => Some(reference),
        Aim::Select(_) | Aim::Exact(_) => None,
    };
    let filled = fill_into(
        target,
        (session, given, &text),
        aimed,
        (named, &mut resolved),
    );
    filled.map(|filled| aim::with_resolved(filled, resolved))
}

/// Fills the field `aimed` stands for, which `given` named, with `text`,
/// as [`fill`] does, in `session`; `resolved` names the twin the field's
/// ref stood for, beside `named`, the ref the request gave, when it gave
/// one.
fn fill_into(
    target: &mut Attached,
    (session, given, text): (&str, &str, &str),
    aimed: TextTarget,
    (named, resolved): (Option<Ref>, &mut Option<Resolved>),
) -> Outcome {
    let (platform, class) = (target.info.platform, target.info.class);
    let (mut field, into, held) = match aimed {
        TextTarget::Within(reference, holder) => {
            let field = target.mirror.resolve(reference)?;
            let into = ready_held(target, (&field, &holder), given, "fill")?;
            (
                Field::named(reference, &field),
                into,
                Some(held_field(given, &field)),
            )
        }
        TextTarget::Field(reference) => {
            let field = Field::named(reference, &target.mirror.resolve(reference)?);
            if platform == Platform::Ios {
                ready_keyboard(target, reference, given)?;
            }
            (field, Some(reference), None)
        }
        TextTarget::Raising(node, selector) => {
            raise_keyboard(target, (&node, &selector), given, "fill")?;
            (Field::Node(selector), None, Some(raised(given, &node)))
        }
    };
    // The tap that readied the field can have put it in a new node, as a
    // map app's search field took one once that tap raised its card: the
    // text goes into its twin (design D3 of action-verdicts).
    let follows = into.is_some();
    follow_field(target, session, &mut field, (named, &mut *resolved));
    let into = if follows { field.reference() } else { None };
    let mut reference = field.reference();
    // The reading a read-back compares a changed name and value with
    // (designs D39 and D56).
    let mut before: (String, Option<String>) = field
        .element(target)?
        .map(|element| (element.name, element.value))
        .unwrap_or_default();
    let note = |warning: Option<String>| match (held.clone(), warning) {
        (Some(held), Some(warning)) => Some(format!("{held}; {warning}")),
        (held, warning) => held.or(warning),
    };
    // Keys go into an emptied field, the one the emptying gave the focus,
    // and the read-back compares with the field as the emptying left it
    // (design D49).
    let mut keys_into = into;
    if let Some(source) = held_text(target, &field, text)? {
        enter(target, into, "", TextMethod::AUTO)?;
        before = emptied(target, &field, (Instant::now(), source), (&before.0, text))?;
        keys_into = None;
    }
    let before = (before.0.as_str(), before.1.as_deref());
    let first = enter(target, keys_into, text, TextMethod::AUTO)?;
    let entered = Instant::now();
    if text.is_empty() && platform == Platform::Android {
        return Ok(filled(given, text, reference, first, Check::Skipped, false).warn(note(None)));
    }
    let gone =
        |by: &str| format!("{given} left the screen after fill {by}, so it could not be read back");
    let read = read_back(target, &field, text, (entered, first, Placed::Over), before)?;
    let value = match read {
        ReadBack::Shown {
            shown, from, after, ..
        } => {
            let check = Check::Verified { shown, from, after };
            return Ok(filled(given, text, reference, first, check, false).warn(note(None)));
        }
        ReadBack::Unreadable { after } => {
            let check = Check::Unverified { after };
            return Ok(filled(given, text, reference, first, check, false)
                .warn(note(Some(unreadable(given, "fill")))));
        }
        ReadBack::Gone => {
            return Ok(filled(given, text, reference, first, Check::Skipped, false)
                .warn(note(Some(gone(entered_by(class, first))))));
        }
        ReadBack::Other { held, .. } => held,
    };
    let first_by = entered_by(class, first);
    follow_field(target, session, &mut field, (named, &mut *resolved));
    let into = if follows { field.reference() } else { None };
    reference = field.reference();
    let second = match enter(target, into, text, TextMethod::DIRECT) {
        Ok(second) => second,
        Err(failure) => {
            let tries = format!(
                "{first_by}, and entering it again in one piece failed: {}",
                failure.message
            );
            return Err(mismatch(given, reference, text, value.as_deref(), &tries));
        }
    };
    let entered = Instant::now();
    let second_by = entered_by(class, second);
    let again = || {
        let it = second_by.replacen("the text", "it", 1);
        format!(
            "{given} showed {} after fill {first_by}, so fill {it} instead",
            described(value.as_deref())
        )
    };
    let read = read_back(
        target,
        &field,
        text,
        (entered, second, Placed::Over),
        before,
    )?;
    close_edit_menu(target, &field)?;
    match read {
        ReadBack::Shown {
            shown, from, after, ..
        } => {
            let check = Check::Verified { shown, from, after };
            Ok(filled(given, text, reference, second, check, true).warn(note(Some(again()))))
        }
        ReadBack::Unreadable { after } => {
            let check = Check::Unverified { after };
            let warning = format!("{}; {}", again(), unreadable(given, "fill"));
            Ok(filled(given, text, reference, second, check, true).warn(note(Some(warning))))
        }
        ReadBack::Gone => {
            let warning = format!("{}; {}", again(), gone(second_by));
            Ok(filled(given, text, reference, second, Check::Skipped, true)
                .warn(note(Some(warning))))
        }
        ReadBack::Other { held: still, .. } => {
            let tries = if first_by == second_by {
                format!("{first_by} twice")
            } else {
                format!(
                    "{first_by}, then {}",
                    second_by.replacen("the text", "it", 1)
                )
            };
            Err(mismatch(given, reference, text, still.as_deref(), &tries))
        }
    }
}

/// Why `type` did not read its text back, as its reply's `reason` says: no
/// field in the tree has the focus (real-app-fixes, design D3).
const NO_FOCUSED_FIELD: &str = "no_focused_field";

/// A type's reply (real-app-fixes, design D3): the text as the request gave
/// it, under `typed`; the field's ref, null for the focused field and for a
/// node holding none; the method the keys went in by; and what the
/// read-back of the field found, as `fill`'s reply has it: `text`, what the
/// field holds; `verified`, how it showed the text added to what it held,
/// `false` when it showed other text or nothing (`reason`: `text_mismatch`,
/// `no_value_to_read`), and null when it was not read back (`reason`:
/// `no_focused_field` when no field in the tree had the focus); `readFrom`;
/// `readBackMs`.
fn typed(
    text: &str,
    reference: Option<Ref>,
    method: Option<TextMethod>,
    read: Option<&ReadBack>,
    unread: Option<&str>,
) -> Success {
    let (held, verified, from, reason, after) = match read {
        Some(ReadBack::Shown {
            shown,
            from,
            after,
            held,
        }) => (
            held.clone(),
            Value::from(shown.name()),
            Value::from(from.name()),
            Value::Null,
            Some(*after),
        ),
        Some(ReadBack::Other { held, from, after }) => (
            held.clone(),
            Value::Bool(false),
            Value::from(from.name()),
            Value::from(TEXT_MISMATCH),
            Some(*after),
        ),
        Some(ReadBack::Unreadable { after }) => (
            None,
            Value::Bool(false),
            Value::Null,
            Value::from(NO_VALUE_TO_READ),
            Some(*after),
        ),
        Some(ReadBack::Gone) | None => (None, Value::Null, Value::Null, unread.into(), None),
    };
    Success::new(json!({
        "typed": text,
        "ref": reference_json(reference),
        "method": method_name(method),
        "text": held,
        "verified": verified,
        "readFrom": from,
        "reason": reason,
        "readBackMs": after.map(millis),
    }))
}

/// Types the request's text as keys into the field it names, or into the
/// focused one, and reads that field back as `fill` does (real-app-fixes,
/// design D3): the field the keys went into, or without a target the field
/// the tree shows with the focus, holding typed text [`READ_BACK_HOLD`],
/// the text verified once the field shows it added to what it held. A field
/// that shows other text, as one that dropped keys, is named in a warning
/// that advises `fill`: `type` adds to what a field holds, so typing the
/// text again would double it. With no field in the tree holding the focus
/// the text goes where the focus is and is not read back, a warning saying
/// so; nor is an empty text. An identifier takes the field inside a view,
/// or the keyboard a view brings up, and a stale ref the twin it stands
/// for, as `fill`'s do.
pub(super) fn type_text(target: &mut Attached, call: &Call, args: &::wire::TypeText) -> Outcome {
    let session = call.session.as_str();
    let text = text_param("type", args.text.as_deref())?;
    // The field the keys go into, as the request names it, and the one the
    // read-back reads, with how warnings name it.
    let mut resolved = None;
    let (into, named, read, warning) = match args.target.as_deref() {
        Some(given) => {
            let aim = Aim::parse(given)?;
            let wait = timeout(&args.timeout, call)?;
            let (aimed, resolution) = aim::text_ref(target, session, (&aim, given, "type"), wait)?;
            resolved = resolution;
            match aimed {
                TextTarget::Within(reference, holder) => {
                    let field = target.mirror.resolve(reference)?;
                    let into = ready_held(target, (&field, &holder), given, "type")?;
                    let read = (Field::named(reference, &field), given.to_owned());
                    (
                        into,
                        Some(reference),
                        Some(read),
                        Some(held_field(given, &field)),
                    )
                }
                TextTarget::Field(reference) => {
                    let field = Field::named(reference, &target.mirror.resolve(reference)?);
                    let read = (field, given.to_owned());
                    (Some(reference), Some(reference), Some(read), None)
                }
                TextTarget::Raising(node, selector) => {
                    raise_keyboard(target, (&node, &selector), given, "type")?;
                    let warning = raised(given, &node);
                    (
                        None,
                        None,
                        Some((Field::Node(selector), given.to_owned())),
                        Some(warning),
                    )
                }
            }
        }
        None => {
            target.catch_up()?;
            let read = focused_field(target)?.and_then(|field| {
                let named = if field.name.is_empty() {
                    format!("the focused {}", field.role)
                } else {
                    format!("the focused {} {:?}", field.role, field.name)
                };
                field
                    .reference
                    .map(|reference| (Field::named(reference, &field), named))
            });
            (None, None, read, None)
        }
    };
    let before: (String, Option<String>) = match &read {
        Some((field, _)) => field
            .element(target)?
            .map(|element| (element.name, element.value))
            .unwrap_or_default(),
        None => Default::default(),
    };
    let outcome = target.mirror.act(&Action::Text {
        text: text.clone(),
        into,
        method: TextMethod::KEYS,
    })?;
    let entered = Instant::now();
    if !outcome.accepted {
        return Err(rejected(
            "text; type takes a text field or the focused field",
        ));
    }
    let note = |read_back: Option<String>| match (warning.clone(), read_back) {
        (Some(warning), Some(read_back)) => Some(format!("{warning}; {read_back}")),
        (warning, read_back) => warning.or(read_back),
    };
    let method = outcome.method;
    let Some((field, given)) = read.filter(|_| !text.is_empty()) else {
        let unread = (!text.is_empty()).then_some(NO_FOCUSED_FIELD);
        let warning = unread.map(|_| {
            "no field in the tree has the focus, so type could not read back where its text went; take a snapshot or a screenshot to see it".to_owned()
        });
        let success = typed(&text, named, method, None, unread).warn(note(warning));
        return Ok(aim::with_resolved(success, resolved));
    };
    let before = (before.0.as_str(), before.1.as_deref());
    let read = read_back(
        target,
        &field,
        &text,
        (entered, method, Placed::After),
        before,
    )?;
    let warning = match &read {
        ReadBack::Shown { .. } => None,
        ReadBack::Other { held, .. } => Some(format!(
            "{given} holds {} where type typed {text:?}: a key was lost, or the field limits or formats its text; type adds to what a field holds, so it does not type again: fill the field to replace what it holds",
            described(held.as_deref())
        )),
        ReadBack::Unreadable { .. } => Some(unreadable(&given, "type")),
        ReadBack::Gone => Some(format!(
            "{given} left the screen after type typed the text, so it could not be read back"
        )),
    };
    let success = typed(&text, named, method, Some(&read), None).warn(note(warning));
    Ok(aim::with_resolved(success, resolved))
}

pub(super) fn press(target: &mut Attached, _call: &Call, args: &::wire::Press) -> Outcome {
    let key = needed("press", "key", &args.key)?;
    let code = keys::resolve(target.info.platform, key)?;
    let outcome = target.mirror.act(&Action::Key {
        code: code.code,
        meta: code.meta,
    })?;
    if !outcome.accepted {
        return Err(rejected("key"));
    }
    Ok(Success::new(
        json!({"pressed": key, "code": code.code, "meta": code.meta}),
    ))
}

/// A direction a finger or the content moves in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn parse(text: &str) -> Result<Direction, Failure> {
        match text.to_ascii_lowercase().as_str() {
            "up" => Ok(Direction::Up),
            "down" => Ok(Direction::Down),
            "left" => Ok(Direction::Left),
            "right" => Ok(Direction::Right),
            _ => Err(Failure::new(
                code::INVALID_ARGUMENT,
                format!("{text:?} is not a direction; use up, down, left or right"),
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
            Direction::Left => "left",
            Direction::Right => "right",
        }
    }

    fn opposite(self) -> Direction {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }

    fn unit(self) -> (f64, f64) {
        match self {
            Direction::Up => (0.0, -1.0),
            Direction::Down => (0.0, 1.0),
            Direction::Left => (-1.0, 0.0),
            Direction::Right => (1.0, 0.0),
        }
    }

    fn vertical(self) -> bool {
        matches!(self, Direction::Up | Direction::Down)
    }
}

/// A finger stroke of `distance` points in `direction` through the center of
/// `area`, kept inside it.
fn stroke(area: Rect, direction: Direction, distance: f64) -> (Point, Point) {
    let center = area.center();
    let (dx, dy) = direction.unit();
    let half = distance / 2.0;
    let inside = |x: f64, y: f64| {
        Point::new(
            x.clamp(area.left + 1.0, (area.right - 1.0).max(area.left + 1.0)),
            y.clamp(area.top + 1.0, (area.bottom - 1.0).max(area.top + 1.0)),
        )
    };
    (
        inside(center.x - dx * half, center.y - dy * half),
        inside(center.x + dx * half, center.y + dy * half),
    )
}

/// The screen as it is now, which a rotation turns.
fn screen(target: &mut Attached) -> Result<Rect, Failure> {
    let size = target.display.size()?;
    Ok(Rect::new(0.0, 0.0, size.width, size.height))
}

/// What a scroll strokes: an element, or the screen.
struct Scrolled {
    /// Its bounds, in points.
    bounds: Rect,
    /// The element; `None` for the screen.
    element: Option<ElementId>,
    /// How messages name it: the element as the command gave it, or the
    /// screen.
    name: String,
}

/// A rectangle's extent along the vertical axis, or the horizontal one.
fn extent(rect: &Rect, vertical: bool) -> f64 {
    let (start, end) = along(rect, vertical);
    (end - start).max(0.0)
}

/// The longest stroke along the vertical axis, or the horizontal one, that
/// `area` takes: [`MAX_STROKE`] of its extent, and a point at least.
fn longest_in(area: &Rect, vertical: bool) -> f64 {
    (extent(area, vertical) * MAX_STROKE).max(1.0)
}

/// What of `scrolled` a stroke along the vertical axis, or the horizontal
/// one, goes through: its bounds less the system and input-method windows
/// above it (`Mirror::uncovered`), so no stroke comes down on a system bar
/// or a keyboard across it. A keyboard takes a stroke that comes down on
/// its keys for typing: on a Nexus 5X with Gboard up, `scroll --until`
/// stroked across the keyboard 50 times, and glide typing entered text into
/// the focused field 49 times (task 10.32). A window cuts the bounds as it
/// cuts visible bounds, only where it spans them from one side to the
/// other, so one floating inside them, as Gboard's stylus toolbar, cuts
/// nothing. With the keyboard up the stroke goes through what shows of the
/// scroll container that shows the most of what is left, less a header the
/// app pins over it (design D41): the production-app run's `scroll up 300`
/// stroked the middle of what the keyboard left of the screen, on a fixed
/// header, and moved nothing. Fails with `covered_by_keyboard`, after
/// `strokes` strokes, when the keyboard leaves too little for a stroke
/// longer than a tap, where the bounds alone take one, or less than
/// [`STROKE_ROOM`] of bounds as long as that.
fn stroke_area(
    target: &Attached,
    scrolled: &Scrolled,
    vertical: bool,
    strokes: u32,
) -> Result<Rect, Failure> {
    let uncovered = target.mirror.uncovered(scrolled.bounds, scrolled.element);
    let mut area = uncovered.area;
    if uncovered.keyboard
        && let Some(shown) = target.mirror.scroll_container_in(area, vertical)
    {
        area = area.intersect(&shown);
    }
    let platform = target.info.platform;
    let slop = tap_slop(platform);
    let short =
        longest_in(&area, vertical) <= slop && longest_in(&scrolled.bounds, vertical) > slop;
    let cramped =
        extent(&area, vertical) < STROKE_ROOM && extent(&scrolled.bounds, vertical) >= STROKE_ROOM;
    if uncovered.keyboard && (short || cramped) {
        let covered = if area.is_empty() {
            format!("the keyboard covers {}", scrolled.name)
        } else {
            format!(
                "the keyboard covers all but {} points of {}, too few for a stroke that moves it",
                points(extent(&area, vertical)),
                scrolled.name
            )
        };
        let close = match platform {
            Platform::Android => "press Back to close the keyboard",
            Platform::Ios => "close the keyboard, as the field's return key does in most apps",
        };
        return Err(Failure::new(
            COVERED_BY_KEYBOARD,
            format!(
                "{covered}, and a stroke that came down on the keyboard would type rather than scroll; {close}, then scroll again"
            ),
        )
        .with_data(json!({"strokes": strokes})));
    }
    Ok(area)
}

/// A point parameter, `[x, y]` in points.
fn point_param(
    key: &str,
    param: &::wire::Param<Option<[f64; 2]>>,
) -> Result<Option<Point>, Failure> {
    Ok(arg(key, param)?.map(|[x, y]| Point::new(x, y)))
}

pub(super) fn swipe(target: &mut Attached, _call: &Call, args: &::wire::Swipe) -> Outcome {
    let duration = Duration::from_millis(arg("duration", &args.duration)?);
    let (from, to, swiped) = match (
        point_param("from", &args.from)?,
        point_param("to", &args.to)?,
    ) {
        (Some(from), Some(to)) => (from, to, Value::Bool(true)),
        _ => {
            let direction = Direction::parse(&args.direction)?;
            let distance = arg("distance", &args.distance)?;
            let (from, to) = stroke(screen(target)?, direction, distance);
            (from, to, direction.name().into())
        }
    };
    let outcome = target.mirror.act(&Action::Swipe {
        from,
        to,
        duration,
        release: Release::Moving,
    })?;
    if !outcome.accepted {
        return Err(rejected("swipe"));
    }
    Ok(Success::new(
        json!({"swiped": swiped, "from": at(from), "to": at(to)}),
    ))
}

pub(super) fn scroll(target: &mut Attached, call: &Call, args: &::wire::Scroll) -> Outcome {
    let session = call.session.as_str();
    let direction = Direction::parse(&args.direction)?;
    let amount = arg("amount", &args.amount)?;
    let mut resolved = None;
    let scrolled = match args.target.as_deref() {
        Some(given) => {
            let aim = Aim::parse(given)?;
            let (element, resolution) =
                aim::element(target, session, &aim, timeout(&args.timeout, call)?)?;
            resolved = resolution;
            Scrolled {
                bounds: element.bounds,
                element: Some(element.id),
                name: given.to_owned(),
            }
        }
        None => {
            // The replica takes in what arrived, a keyboard that came up
            // included, before it says where the strokes may come down.
            target.mirror.pump()?;
            Scrolled {
                bounds: screen(target)?,
                element: None,
                name: "the screen".to_owned(),
            }
        }
    };
    let vertical = direction.vertical();
    let area = stroke_area(target, &scrolled, vertical, 0)?;
    // The content moves in the direction, so the finger moves the other way;
    // a long scroll takes several strokes, none of them short enough to be a
    // tap.
    let longest = longest_in(&area, vertical);
    let platform = target.info.platform;
    if let Some(until) = args.until.as_deref() {
        let step = amount.max(1.0).min(longest);
        check_stroke(platform, step, amount, longest)?;
        return scroll_until(
            target,
            call,
            &args.timeout,
            (&scrolled, direction, amount),
            until,
        )
        .map(|success| aim::with_resolved(success, resolved));
    }
    let (count, step) = split(amount.max(0.0), longest);
    check_stroke(platform, step, amount, longest)?;
    let (from, to) = stroke(area, direction.opposite(), step);
    let mut strokes = 0u32;
    while strokes < count {
        let outcome = target.mirror.act(&Action::Swipe {
            from,
            to,
            duration: SWIPE_DURATION,
            release: Release::Moving,
        })?;
        if !outcome.accepted {
            return Err(rejected("scroll"));
        }
        strokes += 1;
    }
    let success = Success::new(json!({
        "scrolled": true,
        "direction": direction.name(),
        "amount": amount,
        "strokes": strokes,
        "from": at(from),
        "to": at(to),
    }));
    Ok(aim::with_resolved(success, resolved))
}

/// A rectangle's start and end along the vertical axis, or the horizontal
/// one.
fn along(rect: &Rect, vertical: bool) -> (f64, f64) {
    if vertical {
        (rect.top, rect.bottom)
    } else {
        (rect.left, rect.right)
    }
}

/// Whether the edge an element's bounds are cut off at lies across the
/// vertical axis, or the horizontal one.
fn cut_along(seen: &Visibility, vertical: bool) -> bool {
    seen.cut.is_some_and(|edge| {
        matches!(edge, mirror::Direction::Above | mirror::Direction::Below) == vertical
    })
}

/// Whether an element is mostly in view for a scroll along the vertical
/// axis, or the horizontal one: its centre inside the visible bounds, and
/// at least [`MOSTLY`] of it inside them with its bounds cut off at no edge
/// where more of it may lie beyond (see [`Visibility::cut`]); or, longer
/// than the visible bounds along the axis, or cut off along it, filling
/// them along the axis, so that no more of it can show.
fn mostly_in_view(seen: &Visibility, vertical: bool) -> bool {
    if !seen.viewport.contains(seen.bounds.center()) {
        return false;
    }
    let (start, end) = along(&seen.bounds, vertical);
    let (view_start, view_end) = along(&seen.viewport, vertical);
    let (shown_start, shown_end) = along(&seen.bounds.intersect(&seen.viewport), vertical);
    let fills = shown_start <= view_start + EDGE && shown_end >= view_end - EDGE;
    let longer = end - start > view_end - view_start + EDGE || cut_along(seen, vertical);
    (fills && longer) || (seen.cut.is_none() && seen.fraction >= MOSTLY)
}

/// What a scroll that looks for an element does once the element is on
/// screen.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Next {
    /// The element is mostly in view: the scroll stops.
    Stop,
    /// A finger stroke of `length` points toward `finger` brings the element
    /// in.
    Stroke { finger: Direction, length: f64 },
    /// No stroke along the scroll's axis brings more of the element in.
    Stuck,
}

/// What a scroll along the vertical axis, or the horizontal one, does about
/// an element on screen that is in view as `seen` says, in strokes of at
/// most `step` points on a platform that takes strokes of `slop` points or
/// less for taps. An element not mostly in view is brought in by one
/// stroke, whichever way that takes, no longer than the step: one that
/// sticks out of the visible bounds just far enough to carry it [`MARGIN`]
/// points inside, and one whose bounds end at an edge where more of it may
/// lie beyond (Android's clipped bounds), or that is longer than the
/// visible bounds, far enough to carry its middle to theirs. The slop is
/// added, since the content moves only once the finger has gone that far.
fn decide(seen: &Visibility, vertical: bool, step: f64, slop: f64) -> Next {
    if mostly_in_view(seen, vertical) {
        return Next::Stop;
    }
    let (start, end) = along(&seen.bounds, vertical);
    let (view_start, view_end) = along(&seen.viewport, vertical);
    if view_end - view_start <= 0.0 {
        return Next::Stuck;
    }
    // How far the content must move toward the axis's start, up or left;
    // negative toward its end.
    let shift = if cut_along(seen, vertical) || end - start > view_end - view_start - 2.0 * MARGIN {
        (start + end) / 2.0 - (view_start + view_end) / 2.0
    } else if end > view_end {
        end - view_end + MARGIN
    } else if start < view_start {
        start - view_start - MARGIN
    } else {
        0.0
    };
    if shift.abs() < 1.0 {
        return Next::Stuck;
    }
    let finger = match (vertical, shift > 0.0) {
        (true, true) => Direction::Up,
        (true, false) => Direction::Down,
        (false, true) => Direction::Left,
        (false, false) => Direction::Right,
    };
    Next::Stroke {
        finger,
        length: (shift.abs() + slop).min(step),
    }
}

/// How long a stroke that looks for an element is, in a scroll along the
/// vertical axis, or the horizontal one, of `area` in strokes of at most
/// `step` points on a platform that takes strokes of `slop` points or less
/// for taps: the step, and at most [`REACH`] of the extent along the axis of
/// `viewport`, the visible bounds of the scroll container the stroke moves,
/// or of `area` when there is none or it shows nothing, with the slop added,
/// since the content moves only once the finger has gone that far.
fn search_stroke(step: f64, viewport: Option<Rect>, area: Rect, vertical: bool, slop: f64) -> f64 {
    let shown = viewport
        .map(|viewport| extent(&viewport, vertical))
        .filter(|&shown| shown > 0.0)
        .unwrap_or_else(|| extent(&area, vertical));
    (REACH * shown + slop).min(step)
}

/// How long a stroke of `length` points that brings an element in takes:
/// as long as a swipe, or longer, so the finger moves at most
/// [`BRING_IN_SPEED`].
fn bring_in_duration(length: f64) -> Duration {
    SWIPE_DURATION.max(Duration::from_secs_f64(length / BRING_IN_SPEED))
}

/// A share from 0 to 1 as replies give it: to a hundredth.
fn share(fraction: f64) -> f64 {
    (fraction * 100.0).round() / 100.0
}

/// How much of an element is in view, in words: the edge its bounds are
/// cut off at while more of it may lie beyond, else its share in view.
fn in_view(seen: &Visibility) -> String {
    let edge = |edge: mirror::Direction| match edge {
        mirror::Direction::Above => "top",
        mirror::Direction::Below => "bottom",
        mirror::Direction::Left => "left",
        mirror::Direction::Right => "right",
    };
    match seen.cut {
        Some(cut) => format!(
            "cut off at the {} edge of its scroll container, which scrolls on",
            edge(cut)
        ),
        None => format!("{}% of it in view", (seen.fraction * 100.0).round()),
    }
}

/// Scrolls `scrolled` `direction` in strokes of at most `amount` points
/// until the element `until` names is on screen, not covered and mostly in
/// view (see [`mostly_in_view`]), letting the screen settle after each
/// stroke so a check never races a moving list. Each stroke goes through
/// what the system bars and the keyboard leave of `scrolled` as the screen
/// is then (see [`stroke_area`]), since a stroke can bring the keyboard up
/// or send it away, and is at most [`MAX_STROKE`] of that. Every stroke
/// comes to rest before the finger lifts, so the content goes no farther
/// than the finger, and a stroke that looks for the element carries the
/// content at most [`REACH`] of what the scroll container it moves shows
/// (see [`search_stroke`]), so the element cannot pass through the
/// container between two looks. Once the element is on screen, each further
/// stroke brings it in rather than going on by the step (see [`decide`]).
/// Fails with `element_not_found` once the content stops moving or no
/// stroke along the axis brings more of the element in, after
/// [`UNTIL_STROKES`] strokes, or at the command's timeout; an element on
/// screen then gives its share in view as `data.visible`, and the failure
/// names the last stroke's start and end, as `data.from` and `data.to`, and
/// what moved after it ([`after_stroke`]): the third production-app run's scroll
/// stopped after strokes that went the other way, which its reply did not
/// say (rn-app-fixes, design D46).
fn scroll_until(
    target: &mut Attached,
    call: &Call,
    own_timeout: &::wire::Param<Option<u64>>,
    (scrolled, direction, amount): (&Scrolled, Direction, f64),
    until: &str,
) -> Outcome {
    let session = call.session.as_str();
    let aim = Aim::parse(until)?;
    let deadline = Instant::now() + timeout(own_timeout, call)?;
    let vertical = direction.vertical();
    let platform = target.info.platform;
    let slop = tap_slop(platform);
    let mut strokes = 0u32;
    let mut still = 0u32;
    let mut last = None;
    // Where the element was when the last stroke went, and whether the
    // screen changed after it.
    let (mut was, mut changed) = (None, false);
    loop {
        target.catch_up()?;
        let found = aim::on_screen(target, session, &aim)?;
        let seen = found
            .as_ref()
            .and_then(|element| target.mirror.visibility(element.id));
        let area = stroke_area(target, scrolled, vertical, strokes)?;
        let longest = longest_in(&area, vertical);
        let step = amount.max(1.0).min(longest);
        check_stroke(platform, step, amount, longest)?;
        let next = seen.map(|seen| decide(&seen, vertical, step, slop));
        let reach = search_stroke(
            step,
            target.mirror.scroll_viewport(area.center(), vertical),
            area,
            vertical,
            slop,
        );
        if let (Some(Next::Stop), Some(element), Some(seen)) = (next, &found, &seen) {
            if let Some(reference) = element.reference {
                target.session(session).seen.insert(reference);
            }
            let (from, to) = last.unzip();
            return Ok(Success::new(json!({
                "scrolled": strokes > 0,
                "direction": direction.name(),
                "amount": reach,
                "strokes": strokes,
                "until": until,
                "visible": share(seen.fraction),
                "element": element_json(element),
                "from": from.map(at),
                "to": to.map(at),
            })));
        }
        let now = Instant::now();
        let reason = if next == Some(Next::Stuck) {
            Some(format!(
                "scrolling {} does not bring more of it in",
                if vertical {
                    "up or down"
                } else {
                    "left or right"
                }
            ))
        } else if still >= STILL_STROKES {
            Some("the content stopped moving".to_owned())
        } else if strokes >= UNTIL_STROKES {
            Some(format!("{UNTIL_STROKES} strokes are the most it takes"))
        } else if now >= deadline {
            Some(format!(
                "the {} ms timeout passed",
                timeout(own_timeout, call)?.as_millis()
            ))
        } else {
            None
        };
        if let Some(reason) = reason {
            let count = if strokes == 1 { "stroke" } else { "strokes" };
            let last_one = last.map_or_else(String::new, |(from, to)| {
                let now = found.as_ref().map(|element| element.bounds);
                format!(
                    ", the last from {from} to {to}, {}",
                    after_stroke(was, now, changed, vertical)
                )
            });
            let (place, mut data) = match seen {
                Some(seen) => (
                    format!("on screen but not mostly in view ({})", in_view(&seen)),
                    json!({"strokes": strokes, "until": until, "visible": share(seen.fraction)}),
                ),
                None => (
                    "not on screen".to_owned(),
                    json!({"strokes": strokes, "until": until}),
                ),
            };
            if let Some((from, to)) = last {
                data["from"] = at(from);
                data["to"] = at(to);
            }
            let message = format!(
                "{until} was {place} after {strokes} {count} scrolling {}{last_one}: {reason}; take a snapshot to see where the content is",
                direction.name()
            );
            return Err(Failure::new(code::ELEMENT_NOT_FOUND, message).with_data(data));
        }
        let (finger, length, duration) = match next {
            Some(Next::Stroke { finger, length }) => (finger, length, bring_in_duration(length)),
            _ => (direction.opposite(), reach, SWIPE_DURATION),
        };
        let revision = target.mirror.revision();
        let (from, to) = stroke(area, finger, length);
        let outcome = target.mirror.act(&Action::Swipe {
            from,
            to,
            duration,
            release: Release::Resting,
        })?;
        if !outcome.accepted {
            return Err(rejected("scroll"));
        }
        strokes += 1;
        last = Some((from, to));
        was = found.as_ref().map(|element| element.bounds);
        target.mirror.settle(&SettleOptions {
            max: UNTIL_SETTLE.min(deadline.saturating_duration_since(Instant::now())),
            ..SettleOptions::default()
        })?;
        target.catch_up()?;
        // The replica shows whether the stroke moved the content once the
        // reads it called for have run, such as the full read after a
        // window event.
        target.mirror.pump()?;
        changed = target.mirror.revision() != revision;
        if changed {
            still = 0;
        } else {
            still += 1;
        }
    }
}

/// What moved after a scroll's last stroke, as its failure says it: the
/// element, `was` where it was before the stroke and `now` where it is,
/// along the vertical axis or the horizontal one, or else the screen, when
/// it `changed`, or nothing.
fn after_stroke(was: Option<Rect>, now: Option<Rect>, changed: bool, vertical: bool) -> String {
    if let (Some(was), Some(now)) = (was, now) {
        let shift = if vertical {
            now.top - was.top
        } else {
            now.left - was.left
        };
        if shift.abs() >= EDGE {
            let way = match (vertical, shift > 0.0) {
                (true, true) => "down",
                (true, false) => "up",
                (false, true) => "right",
                (false, false) => "left",
            };
            return format!("after which it moved {} points {way}", points(shift.abs()));
        }
    }
    if changed {
        "after which the screen changed".to_owned()
    } else {
        "after which nothing moved".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use mirror::backend::{BackendError, Input, InputAck, InputSink};

    use super::*;
    use crate::driver::{AttachOptions, Driver, DriverError, Screen, Size};
    use crate::fake::{FakeDriver, Slide, Typing};
    use crate::wire::Request;

    /// The fake fixture app.
    const FIXTURE: &str = "dev.mobileagent.fixture.expo";

    /// A fake target of `platform` whose fields take text as `typing` says
    /// and show each change `latency` after its cause, with the fixture app
    /// open on its Login tab.
    fn login_on(platform: &str, typing: Typing, latency: Duration) -> Attached {
        let driver = FakeDriver::parse(platform)
            .unwrap()
            .with_typing(typing)
            .with_latency(latency);
        let info = driver.discover().unwrap().remove(0);
        let attachment = driver.attach(&info, &AttachOptions::default()).unwrap();
        let mut target = Attached::new(&info, attachment, &Arc::new(Mutex::default()));
        let open = json!({"action": "open", "app": FIXTURE}).to_string();
        super::super::run_on(&mut target, "default", &request(&open)).unwrap();
        target
    }

    /// A fake Simulator whose fields take text as `typing` says, with the
    /// fixture app open on its Login tab.
    fn login(typing: Typing) -> Attached {
        login_on("ios", typing, Duration::ZERO)
    }

    /// `fill` of `text` into the field `field` names.
    fn fill_in(target: &mut Attached, field: &str, text: &str) -> Outcome {
        let fill = json!({"action": "fill", "target": field, "text": text}).to_string();
        super::super::run_on(target, "default", &request(&fill))
    }

    /// What the field `field` names holds, as `get value` reads it.
    fn held(target: &mut Attached, field: &str) -> Value {
        let get = json!({"action": "get", "what": "value", "target": field}).to_string();
        super::super::run_on(target, "default", &request(&get))
            .unwrap()
            .data["value"]
            .clone()
    }

    /// Task 10.23: on Bluesky's search field a fill of "nobel", typed as
    /// keys 15 ms apart, left "noelb" and replied success. A field that
    /// takes keys out of order gets the text pasted once the read-back
    /// finds its keys out of order, and the reply says so.
    #[test]
    fn a_fill_whose_keys_a_field_takes_out_of_order_pastes_the_text() {
        let mut target = login(Typing::Reorder);
        let reply = fill_in(&mut target, "#login-username", "nobel");
        let holds = held(&mut target, "#login-username");
        assert_eq!(
            holds, "nobel",
            "fill replied {reply:?} while the field holds {holds}"
        );
        let success = reply.unwrap();
        assert_eq!(success.data["method"], "direct", "{}", success.data);
        assert_eq!(success.data["retried"], true, "{}", success.data);
        assert_eq!(success.data["verified"], "text", "{}", success.data);
        assert_eq!(success.data["text"], "nobel", "{}", success.data);
        let warning = success.warning.unwrap_or_default();
        assert!(
            warning.contains("\"noelb\"") && warning.contains("pasted"),
            "the warning names what the keys left and the paste: {warning}"
        );
    }

    /// Physical-ios, task 5.3: on a physical iPhone the text entered again
    /// in one piece is the field's value set through the runner, not a
    /// paste, and fill's warning says so.
    #[test]
    fn a_fill_on_an_iphone_says_it_set_the_text_rather_than_pasted_it() {
        let mut target = login_on("ios-device", Typing::Reorder, Duration::ZERO);
        let reply = fill_in(&mut target, "#login-username", "nobel");
        let success = reply.unwrap();
        assert_eq!(success.data["method"], "direct", "{}", success.data);
        assert_eq!(success.data["retried"], true, "{}", success.data);
        let warning = success.warning.unwrap_or_default();
        assert!(
            warning.contains("set it through the runner") && !warning.contains("paste"),
            "{warning}"
        );
    }

    /// Task 10.23: a field that keeps other text than the fill's, however
    /// the text reaches it, fails the fill with `text_mismatch`, whose
    /// message names what the field holds.
    #[test]
    fn a_fill_that_a_field_cuts_short_fails_naming_what_the_field_holds() {
        let mut target = login(Typing::Max(4));
        let reply = fill_in(&mut target, "#login-username", "nobel");
        let holds = held(&mut target, "#login-username");
        let failure = reply.expect_err(&format!("the field holds {holds}"));
        assert_eq!(failure.code, TEXT_MISMATCH, "{}", failure.message);
        assert!(
            failure.message.contains("\"nobe\""),
            "the message names what the field holds: {}",
            failure.message
        );
        let data = failure.data.unwrap_or_default();
        assert_eq!(data["value"], "nobe", "{data}");
        assert_eq!(data["masked"], false, "{data}");
    }

    /// Task 10.23: a secure field shows a mask character for each of its
    /// own, so the read-back compares their number.
    #[test]
    fn a_secure_field_is_read_back_by_the_number_of_its_characters() {
        let mut target = login(Typing::Keys);
        let success = fill_in(&mut target, "#login-password", "hunter2").unwrap();
        assert_eq!(success.data["method"], "keys", "{}", success.data);
        assert_eq!(success.data["verified"], "length", "{}", success.data);
        assert_eq!(success.data["retried"], false, "{}", success.data);
        assert_eq!(held(&mut target, "#login-password"), "•••••••");
    }

    /// Task 10.23: a fill replies once the field shows the text, which can
    /// come late, as a paste showed up to 462 ms after Command-V on the
    /// Simulator.
    #[test]
    fn a_fill_replies_once_the_field_shows_its_text() {
        let latency = Duration::from_millis(300);
        let mut target = login_on("ios", Typing::Whole, latency);
        let success = fill_in(&mut target, "#login-username", "agent").unwrap();
        assert_eq!(held(&mut target, "#login-username"), "agent");
        assert_eq!(success.data["verified"], "text", "{}", success.data);
        assert!(
            success.data["readBackMs"]
                .as_f64()
                .is_some_and(|ms| ms >= 250.0),
            "{}",
            success.data
        );
    }

    /// Task 10.29: on the iOS 26.5 Simulator a paste showed 43 to 86 ms
    /// after Command-V, never in the read the mirror made at once after it,
    /// and the fill took it 252 to 315 ms after the paste, at the next read
    /// of the field's window, 200 ms after that read. The mirror reads the
    /// field's window again as the frame that shows the text arrives, so
    /// the fill takes text that shows 60 ms after the entry within one read
    /// of it.
    #[test]
    fn a_fill_takes_text_that_shows_after_the_first_read_as_its_frame_arrives() {
        let latency = Duration::from_millis(60);
        let mut target = login_on("ios", Typing::Whole, latency);
        let success = fill_in(&mut target, "#login-username", "agent").unwrap();
        let data = &success.data;
        assert_eq!(data["method"], "direct", "{data}");
        assert_eq!(data["verified"], "text", "{data}");
        assert!(
            data["readBackMs"]
                .as_f64()
                .is_some_and(|ms| (60.0..120.0).contains(&ms)),
            "{data}"
        );
        assert_eq!(held(&mut target, "#login-username"), "agent");
    }

    /// The login form's fields, the text `fill` enters into each and how
    /// the field shows it.
    const LOGIN_FIELDS: [(&str, &str, &str); 2] = [
        ("#login-username", "agent", "text"),
        ("#login-password", "hunter2", "length"),
    ];

    /// Task 10.30: text that goes in in one piece, set directly on Android
    /// or pasted on a Simulator, gives an app that sets its field's value
    /// again no keys to interleave with, so the read-back takes the first
    /// read that shows it. The fake's fields show a change at once, so that
    /// read comes within 100 ms, half the hold after keys.
    #[test]
    fn a_fill_whose_text_went_in_in_one_piece_takes_the_first_read_that_shows_it() {
        for platform in ["android", "ios"] {
            let mut target = login_on(platform, Typing::Whole, Duration::ZERO);
            for (field, text, shown) in LOGIN_FIELDS {
                let success = fill_in(&mut target, field, text).unwrap();
                let data = &success.data;
                assert_eq!(data["method"], "direct", "{platform} {field}: {data}");
                assert_eq!(data["verified"], shown, "{platform} {field}: {data}");
                assert_eq!(data["retried"], false, "{platform} {field}: {data}");
                assert_eq!(data["text"], text, "{platform} {field}: {data}");
                assert!(
                    data["readBackMs"].as_f64().is_some_and(|ms| ms < 100.0),
                    "{platform} {field}: {data}"
                );
            }
        }
    }

    /// Task 10.30: keys can interleave with an app that sets its field's
    /// value again after the edit, as Bluesky's controlled search field did
    /// (task 10.23), so a fill typed as keys takes the text only once the
    /// field has held it since 200 ms after the entry, on either platform.
    #[test]
    fn a_fill_typed_as_keys_takes_the_text_once_the_field_has_held_it_200_ms() {
        for platform in ["android", "ios"] {
            let mut target = login_on(platform, Typing::Keys, Duration::ZERO);
            for (field, text, shown) in LOGIN_FIELDS {
                let success = fill_in(&mut target, field, text).unwrap();
                let data = &success.data;
                assert_eq!(data["method"], "keys", "{platform} {field}: {data}");
                assert_eq!(data["verified"], shown, "{platform} {field}: {data}");
                assert_eq!(data["retried"], false, "{platform} {field}: {data}");
                assert!(
                    data["readBackMs"].as_f64().is_some_and(|ms| ms >= 200.0),
                    "{platform} {field}: {data}"
                );
            }
        }
    }

    /// Task 10.30: the text `fill` enters again after the field showed
    /// other text goes in in one piece, so the second read-back takes the
    /// first read that shows it, without the hold after the keys before
    /// it; the fake's field shows the paste at once, within 100 ms.
    #[test]
    fn a_fill_that_enters_its_text_again_in_one_piece_takes_the_first_read_that_shows_it() {
        let mut target = login(Typing::Reorder);
        let success = fill_in(&mut target, "#login-username", "nobel").unwrap();
        let data = &success.data;
        assert_eq!(data["retried"], true, "{data}");
        assert_eq!(data["method"], "direct", "{data}");
        assert_eq!(data["verified"], "text", "{data}");
        assert!(
            data["readBackMs"].as_f64().is_some_and(|ms| ms < 100.0),
            "{data}"
        );
        assert_eq!(held(&mut target, "#login-username"), "nobel");
    }

    /// Task 7.1 (task 6.5, N13): with the Simulator's hardware keyboard
    /// disconnected, the sixth production-app run's configuration lost Shift and
    /// Command, so the address field typed "@" as "2", the paste that
    /// followed landed nothing, and the tap on the field in focus that
    /// readied the paste left iOS's edit menu up over the screen. A fill
    /// whose paste leaves the menu up dismisses it: after the fill, which
    /// fails naming what the field holds, the menu is gone, the screen is
    /// listed, and the field keeps its text and the focus.
    #[test]
    fn a_fill_whose_paste_lands_nothing_dismisses_the_edit_menu_it_left_up() {
        let mut target = login(Typing::Unshifted);
        let failure =
            fill_in(&mut target, "#login-username", "a@b").expect_err("the paste lands nothing");
        assert_eq!(failure.code, TEXT_MISMATCH, "{}", failure.message);
        assert!(failure.message.contains("\"a2b\""), "{}", failure.message);
        let listing = target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        assert!(!listing.text.contains("Paste"), "{}", listing.text);
        let field = listing
            .by_identifier("login-username")
            .unwrap_or_else(|| panic!("the field is listed:\n{}", listing.text));
        assert!(field.focused, "{}", listing.text);
        assert!(listing.by_identifier("login-submit").is_some());
        assert_eq!(held(&mut target, "#login-username"), "a2b");
    }

    /// Task 10.23: an empty fill clears the field, which reads back empty;
    /// on Android an empty field reads as its hint, so an empty fill there
    /// is not read back.
    #[test]
    fn an_empty_fill_reads_the_field_back_empty_except_on_android() {
        let mut target = login(Typing::Whole);
        fill_in(&mut target, "#login-username", "agent").unwrap();
        let success = fill_in(&mut target, "#login-username", "").unwrap();
        assert_eq!(success.data["verified"], "text", "{}", success.data);
        assert_eq!(held(&mut target, "#login-username"), Value::Null);

        let mut android = login_on("android", Typing::Whole, Duration::ZERO);
        fill_in(&mut android, "#login-username", "agent").unwrap();
        let success = fill_in(&mut android, "#login-username", "").unwrap();
        assert!(success.data["verified"].is_null(), "{}", success.data);
        assert!(success.data["readBackMs"].is_null(), "{}", success.data);
        assert_eq!(held(&mut android, "#login-username"), Value::Null);
    }

    /// Task 10.23: a field's text is the fill's when it is the text, an
    /// empty read of a field named as the text (the Simulator's reader
    /// leaves out a value that repeats the placeholder), or as many mask
    /// characters, some of them in the text's place, as a secure field
    /// shows; anything else is other text.
    #[test]
    fn a_field_shows_the_text_exactly_as_its_mask_or_as_an_empty_placeholder() {
        let shows = |value: Option<&str>, name: &str, text: &str| compare(value, name, text);
        assert_eq!(shows(Some("nobel"), "Search", "nobel"), Some(Shown::Text));
        assert_eq!(shows(None, "Search", ""), Some(Shown::Text));
        assert_eq!(shows(Some(""), "Search", ""), Some(Shown::Text));
        assert_eq!(shows(None, "Search", "Search"), Some(Shown::Text));
        assert_eq!(
            shows(Some("•••••••"), "Password", "hunter2"),
            Some(Shown::Length)
        );
        assert_eq!(shows(Some("●●●"), "Password", "abc"), Some(Shown::Length));
        assert_eq!(
            shows(Some("••••••2"), "Password", "hunter2"),
            Some(Shown::Length)
        );
        assert_eq!(shows(Some("noelb"), "Search", "nobel"), None);
        assert_eq!(shows(Some("••••••"), "Password", "hunter2"), None);
        assert_eq!(shows(Some("••••••x"), "Password", "hunter2"), None);
        assert_eq!(shows(None, "Search", "nobel"), None);
        assert_eq!(shows(Some("it’s"), "Username", "it's"), None);
    }

    /// Design D43: a name is compared by the part of it the entry changed,
    /// what lies between what the names before and after share at their
    /// start and their end, with the separators a name spells characters
    /// with left out of it and of the text: the boxes of the production-app
    /// run's code field before its label, a code after a label, a name
    /// in front of a label, a cleared field's dashes, a code entered over
    /// one it starts as. Other digits, digits missing, and bullets for each
    /// digit, which tell nothing but the text's length, are other text.
    #[test]
    fn a_name_shows_the_text_in_the_part_the_entry_changed() {
        let shows = |name: &str, before: &str, text: &str| compare_name(name, before, text);
        let empty = "–, –, –, –, –, –, Verification code, required";
        let code = "1, 2, 3, 4, 5, 6, Verification code, required";
        assert_eq!(shows(code, empty, "123456"), Some(Shown::Text));
        assert_eq!(
            shows(
                "placeholder-generic-2, 1 2 3 4 5 6",
                "placeholder-generic-2",
                "123456"
            ),
            Some(Shown::Text)
        );
        assert_eq!(
            shows("Smoke Org, Name, required", "Name, required", "Smoke Org"),
            Some(Shown::Text)
        );
        assert_eq!(shows(empty, code, ""), Some(Shown::Text));
        let wrong = "1, 2, 3, 4, 9, 9, Verification code, required";
        assert_eq!(shows(wrong, empty, "123456"), None);
        // A code entered over one it starts as: the words the entry wrote
        // again count with those it changed.
        assert_eq!(shows(code, wrong, "123456"), Some(Shown::Text));
        assert_eq!(shows(wrong, code, "123456"), None);
        let short = "1, 2, 3, 4, –, –, Verification code, required";
        assert_eq!(shows(short, empty, "123456"), None);
        let masked = "•, •, •, •, •, •, Verification code, required";
        assert_eq!(shows(masked, empty, "123456"), None);
        assert_eq!(
            shows(
                "placeholder-generic-2, ••••••",
                "placeholder-generic-2",
                "123456"
            ),
            None
        );
    }

    /// A screen that turns to landscape once `turned` is set.
    struct Turning(Arc<AtomicBool>);

    impl Screen for Turning {
        fn size(&mut self) -> Result<Size, DriverError> {
            Ok(if self.0.load(Ordering::SeqCst) {
                Size {
                    width: 800.0,
                    height: 400.0,
                }
            } else {
                Size {
                    width: 400.0,
                    height: 800.0,
                }
            })
        }
    }

    fn request(line: &str) -> Request {
        crate::wire::parse(line).unwrap()
    }

    /// Notes that every input waited for its transport to activate.
    struct Fresh;

    impl crate::driver::InputNotes for Fresh {
        fn activation_wait(&self) -> Option<Duration> {
            Some(Duration::from_millis(12))
        }
    }

    /// A fake Simulator that shows every change 400 ms after its cause,
    /// with the input notes `notes`, its launcher snapshotted.
    fn slow_ios(notes: Option<Arc<dyn crate::driver::InputNotes>>) -> (Attached, String) {
        let driver = FakeDriver::parse("ios")
            .unwrap()
            .with_latency(Duration::from_millis(400));
        let info = driver.discover().unwrap().remove(0);
        let mut attachment = driver.attach(&info, &AttachOptions::default()).unwrap();
        attachment.input_notes = notes;
        let mut target = Attached::new(&info, attachment, &Arc::new(Mutex::default()));
        let listing = super::super::run_on(
            &mut target,
            "default",
            &request(r#"{"action": "snapshot", "interactive": true}"#),
        )
        .unwrap()
        .data;
        let fixture = listing["refs"]
            .as_object()
            .and_then(|refs| {
                refs.iter()
                    .find(|(_, entry)| entry["name"] == "Fixture")
                    .map(|(reference, _)| format!("@{reference}"))
            })
            .expect("the launcher's Fixture button");
        (target, fixture)
    }

    /// Task 8.11: after a fresh input-transport activation a tap's effects
    /// can come later than a warm tap's, so the click's watch waits longer
    /// for the first of them.
    #[test]
    fn a_click_after_a_fresh_activation_watches_long_enough_for_its_late_effects() {
        let (mut warm, fixture) = slow_ios(None);
        let click = request(&format!(r#"{{"action": "click", "target": "{fixture}"}}"#));
        let data = super::super::run_on(&mut warm, "default", &click)
            .unwrap()
            .data;
        assert!(
            data.get("frameChangeMs").is_none(),
            "a warm watch ends 200 ms after a tap that changed nothing yet: {data}"
        );

        let (mut fresh, fixture) = slow_ios(Some(Arc::new(Fresh)));
        let click = request(&format!(r#"{{"action": "click", "target": "{fixture}"}}"#));
        let data = super::super::run_on(&mut fresh, "default", &click)
            .unwrap()
            .data;
        for span in ["frameChangeMs", "treeDeltaMs"] {
            assert!(
                data[span].as_f64().is_some_and(|ms| ms >= 400.0),
                "{span} after a fresh activation: {data}"
            );
        }
    }

    /// Task 8.13: a scroll takes as few strokes as fit, all of one length,
    /// so no short last stroke is left over to be a tap.
    #[test]
    fn a_scroll_splits_into_equal_strokes_that_fit() {
        assert_eq!(split(480.0, 700.0), (1, 480.0));
        assert_eq!(split(1000.0, 700.0), (2, 500.0));
        assert_eq!(split(703.0, 700.0), (2, 351.5), "not 700 and a 3-point tap");
        assert_eq!(split(700.4, 700.0), (1, 700.0), "half a point is left out");
        assert_eq!(split(100_000.0, 700.0), (MAX_STROKES, 700.0));
    }

    /// Task 8.13: strokes no longer than the platform's tap threshold are
    /// refused, naming the threshold, whether the amount or the scrolled
    /// area makes them short.
    #[test]
    fn strokes_the_platform_takes_for_taps_are_refused() {
        assert!(check_stroke(Platform::Android, 8.5, 8.5, 700.0).is_ok());
        assert!(check_stroke(Platform::Ios, 10.5, 10.5, 700.0).is_ok());
        for (platform, slop) in [(Platform::Android, "8"), (Platform::Ios, "10")] {
            let short = check_stroke(platform, 2.0, 2.0, 700.0).unwrap_err();
            assert_eq!(short.code, STROKE_TOO_SHORT);
            assert!(
                short
                    .message
                    .contains(&format!("scroll more than {slop} points")),
                "{}",
                short.message
            );
            let narrow = check_stroke(platform, 6.4, 300.0, 6.4).unwrap_err();
            assert_eq!(narrow.code, STROKE_TOO_SHORT);
            assert!(
                narrow.message.contains("6.4 points at most"),
                "{}",
                narrow.message
            );
        }
    }

    /// Task 8.11: a gesture no element anchors follows the screen as it is
    /// when the command runs, so a rotated display is swiped across its
    /// middle.
    #[test]
    fn a_swipe_without_points_follows_the_screen_as_it_is_now() {
        let driver = FakeDriver::parse("android").unwrap();
        let info = driver.discover().unwrap().remove(0);
        let mut attachment = driver.attach(&info, &AttachOptions::default()).unwrap();
        let turned = Arc::new(AtomicBool::new(false));
        attachment.display = Box::new(Turning(Arc::clone(&turned)));
        let mut target = Attached::new(&info, attachment, &Arc::new(Mutex::default()));
        let swipe = request(r#"{"action": "swipe", "direction": "up"}"#);
        let upright = super::super::run_on(&mut target, "default", &swipe)
            .unwrap()
            .data;
        assert_eq!(upright["from"][0], 200.0, "{upright}");
        turned.store(true, Ordering::SeqCst);
        let rotated = super::super::run_on(&mut target, "default", &swipe)
            .unwrap()
            .data;
        assert_eq!(
            rotated["from"][0], 400.0,
            "the middle of the landscape screen: {rotated}"
        );
        assert!(
            rotated["from"][1].as_f64() < Some(400.0),
            "inside the landscape screen: {rotated}"
        );
    }

    /// What the mirror tells of an element at `bounds` inside `viewport`,
    /// its bounds cut off at `cut`.
    fn seen(bounds: [f64; 4], viewport: [f64; 4], cut: Option<mirror::Direction>) -> Visibility {
        let bounds = Rect::new(bounds[0], bounds[1], bounds[2], bounds[3]);
        let viewport = Rect::new(viewport[0], viewport[1], viewport[2], viewport[3]);
        let shown = bounds.intersect(&viewport);
        Visibility {
            bounds,
            viewport,
            fraction: shown.width() * shown.height() / (bounds.width() * bounds.height()),
            cut,
            reachable: (!shown.is_empty()).then_some(shown),
        }
    }

    /// The Expo fixture's list on the emulator, in points.
    const LIST: [f64; 4] = [0.0, 72.0, 408.0, 840.0];

    /// Whether `next` is a stroke toward `finger` of `length` points.
    fn is_stroke(next: Next, finger: Direction, length: f64) -> bool {
        matches!(next, Next::Stroke { finger: f, length: l } if f == finger && (l - length).abs() < 1e-9)
    }

    /// Task 10.15: a scroll that looks for an element stops once the
    /// element's centre and at least half of it are in view, not while a
    /// sliver of it shows: the frame-path run of the harness's list-scroll
    /// flow stopped with `row-200` 1.2 points tall at the list's edge.
    #[test]
    fn a_scroll_that_looks_for_an_element_stops_once_it_is_mostly_in_view() {
        let stops = |bounds, cut| decide(&seen(bounds, LIST, cut), true, 600.0, 8.0) == Next::Stop;
        assert!(stops([0.0, 400.0, 408.0, 448.0], None), "wholly in view");
        assert!(
            stops([0.0, 811.2, 408.0, 859.2], None),
            "60% in view, its centre too"
        );
        assert!(
            !stops([0.0, 816.0, 408.0, 864.0], None),
            "half in view, its centre on the edge"
        );
        assert!(!stops([0.0, 824.0, 408.0, 872.0], None), "a third in view");
        assert!(
            !stops([0.0, 838.8, 408.0, 840.0], Some(mirror::Direction::Below)),
            "a sliver Android cut off where the list scrolls on"
        );
        assert!(
            stops([0.0, 792.0, 408.0, 840.0], None),
            "a row at the end of a list that scrolls no further"
        );
        assert!(
            stops([0.0, 72.0, 408.0, 840.0], Some(mirror::Direction::Below)),
            "an element cut off at the list's edge that fills the list"
        );
        assert!(
            stops([0.0, -500.0, 408.0, 1412.0], None),
            "a frame longer than the list that fills it, its centre in view"
        );
        assert!(
            !stops([0.0, 300.0, 408.0, 2000.0], None),
            "a frame longer than the list, its centre out of view"
        );
    }

    /// Task 10.15: once the element is on screen, a stroke of the step
    /// could carry it past the far edge, so the next stroke only brings it
    /// in, whichever way that takes.
    #[test]
    fn a_partly_shown_element_is_brought_in_by_one_stroke_no_longer_than_the_step() {
        let next = |bounds, cut, step| decide(&seen(bounds, LIST, cut), true, step, 8.0);
        // 28 of its 48 points lie below the list: the content moves them and
        // 8 more up, the finger 8 more than the content.
        assert!(is_stroke(
            next([0.0, 820.0, 408.0, 868.0], None, 600.0),
            Direction::Up,
            44.0
        ));
        // 32 lie above it: down.
        assert!(is_stroke(
            next([0.0, 40.0, 408.0, 88.0], None, 600.0),
            Direction::Down,
            48.0
        ));
        // A sliver cut off at the bottom, of unknown length: its middle to
        // the list's, 383.4 points up.
        let sliver = [0.0, 838.8, 408.0, 840.0];
        let below = Some(mirror::Direction::Below);
        assert!(is_stroke(next(sliver, below, 600.0), Direction::Up, 391.4));
        assert!(is_stroke(next(sliver, below, 300.0), Direction::Up, 300.0));
        let above = Some(mirror::Direction::Above);
        assert!(is_stroke(
            next([0.0, 72.0, 408.0, 80.0], above, 600.0),
            Direction::Down,
            388.0
        ));
        // Across a horizontal list, an item sticking out on the right.
        let carousel = [0.0, 100.0, 408.0, 300.0];
        let item = seen([350.0, 100.0, 550.0, 300.0], carousel, None);
        assert!(is_stroke(
            decide(&item, false, 300.0, 8.0),
            Direction::Left,
            158.0
        ));
    }

    /// Task 10.15: what the scroll's axis does not cut off, no stroke along
    /// it brings in.
    #[test]
    fn no_stroke_brings_in_an_element_the_scroll_s_axis_does_not_cut_off() {
        let sliver = seen(
            [0.0, 838.8, 408.0, 840.0],
            LIST,
            Some(mirror::Direction::Below),
        );
        assert_eq!(decide(&sliver, false, 300.0, 8.0), Next::Stuck);
        let beyond_the_right_edge = seen([300.0, 400.0, 516.0, 448.0], LIST, None);
        assert_eq!(
            decide(&beyond_the_right_edge, true, 300.0, 8.0),
            Next::Stuck
        );
        let under_a_bar = seen([0.0, 400.0, 408.0, 448.0], [0.0, 400.0, 408.0, 400.0], None);
        assert_eq!(decide(&under_a_bar, true, 300.0, 8.0), Next::Stuck);
    }

    /// Task 10.16: a stroke that looks for an element carries the content at
    /// most three quarters of what the scroll container it moves shows along
    /// the scroll's axis, so no element passes through the container between
    /// two looks; the tap threshold is added, since the content moves only
    /// once the finger has gone that far, and the stroke is at most the step.
    #[test]
    fn a_stroke_that_looks_for_an_element_goes_at_most_three_quarters_of_what_its_list_shows() {
        let screen = Rect::new(0.0, 0.0, 408.0, 912.0);
        let list = Some(Rect::new(LIST[0], LIST[1], LIST[2], LIST[3]));
        assert_eq!(search_stroke(600.0, list, screen, true, 8.0), 584.0);
        assert_eq!(
            search_stroke(300.0, list, screen, true, 8.0),
            300.0,
            "the step"
        );
        let under_the_keyboard = Some(Rect::new(0.0, 72.0, 408.0, 472.0));
        assert_eq!(
            search_stroke(600.0, under_the_keyboard, screen, true, 8.0),
            308.0
        );
        let carousel = Some(Rect::new(0.0, 200.0, 408.0, 400.0));
        assert_eq!(search_stroke(600.0, carousel, screen, false, 10.0), 316.0);
        // Without a scroll container under the stroke, or with one that shows
        // nothing, the scrolled area stands for it.
        assert_eq!(search_stroke(700.0, None, screen, true, 8.0), 692.0);
        let hidden = Some(Rect::new(0.0, 400.0, 408.0, 400.0));
        assert_eq!(search_stroke(700.0, hidden, screen, true, 8.0), 692.0);
    }

    #[test]
    fn a_stroke_that_brings_an_element_in_moves_the_finger_at_most_400_points_a_second() {
        assert_eq!(bring_in_duration(44.0), SWIPE_DURATION);
        assert_eq!(bring_in_duration(400.0), Duration::from_secs(1));
    }

    /// The strokes a target was sent, each from where the finger came down
    /// to where it lifted, in points.
    type Strokes = Arc<Mutex<Vec<(Point, Point)>>>;

    /// An input sink that notes every stroke it passes on.
    struct Noting {
        inner: Box<dyn InputSink>,
        strokes: Strokes,
    }

    impl InputSink for Noting {
        fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError> {
            if let Input::Swipe { from, to, .. } = input {
                self.strokes.lock().unwrap().push((*from, *to));
            }
            self.inner.perform(input)
        }
    }

    /// A fake Android target with the fixture app open as `open` with `env`
    /// opens it, its keyboard up over every screen when `keyboard` is set,
    /// and the strokes it is sent.
    fn noting_strokes(env: &[&str], keyboard: bool) -> (Attached, Strokes) {
        let driver = FakeDriver::parse("android")
            .unwrap()
            .with_keyboard(keyboard);
        let info = driver.discover().unwrap().remove(0);
        let mut attachment = driver.attach(&info, &AttachOptions::default()).unwrap();
        let strokes = Strokes::default();
        attachment.backend.input = Box::new(Noting {
            inner: attachment.backend.input,
            strokes: Arc::clone(&strokes),
        });
        let mut target = Attached::new(&info, attachment, &Arc::new(Mutex::default()));
        let open = json!({"action": "open", "app": FIXTURE, "env": env}).to_string();
        super::super::run_on(&mut target, "default", &request(&open)).unwrap();
        (target, strokes)
    }

    /// `scroll` as `params` ask it.
    fn scroll_with(target: &mut Attached, params: Value) -> Outcome {
        let mut line = json!({"action": "scroll"});
        line.as_object_mut()
            .unwrap()
            .extend(params.as_object().unwrap().clone());
        super::super::run_on(target, "default", &request(&line.to_string()))
    }

    /// Where the fake Android keyboard starts, in points: it covers the
    /// screen from there down.
    const KEYBOARD_TOP: f64 = 600.0;

    /// Task 10.32: on a Nexus 5X with Gboard up, `scroll --until` stroked
    /// across the keyboard 50 times, each stroke starting low on its keys,
    /// and glide typing entered "by" into the focused field 49 times, so
    /// the screen never held still and the scroll ran out of strokes. A
    /// scroll of the screen strokes through what the system bars and the
    /// keyboard leave of it: with the fake keyboard over the screen below
    /// 600 points, every stroke keeps above it, the field keeps its text,
    /// and the scroll that looks for an element that is not there finds the
    /// screen still after two strokes.
    #[test]
    fn a_scroll_of_the_screen_strokes_above_the_keyboard() {
        let (mut target, strokes) = noting_strokes(&[], false);
        fill_in(&mut target, "#login-username", "agent").unwrap();
        for amount in [300.0, 600.0] {
            scroll_with(&mut target, json!({"direction": "down", "amount": amount})).unwrap();
        }
        let until = scroll_with(
            &mut target,
            json!({"direction": "down", "amount": 600, "until": "#row-150", "timeout": 3000}),
        );
        let strokes = strokes.lock().unwrap().clone();
        for (from, to) in &strokes {
            assert!(
                from.y < KEYBOARD_TOP && to.y < KEYBOARD_TOP,
                "every stroke above the keyboard: {strokes:?}"
            );
        }
        assert_eq!(held(&mut target, "#login-username"), "agent");
        let failure = until.expect_err("no row on the Login tab");
        assert!(
            failure.message.contains("the content stopped moving"),
            "{}",
            failure.message
        );
        assert_eq!(failure.data.unwrap_or_default()["strokes"], 2);
    }

    /// Task 10.32: a list partly under the keyboard takes strokes only
    /// through what the keyboard leaves of it, each at most 80% of that
    /// part's height. With the fake keyboard up over the List tab below 600
    /// points, `scroll --until` inside the list, which spans 24 to 840
    /// points, strokes between 24 and 600: 440 points that look for row 25
    /// (three quarters of the 576 that show, and the tap threshold), and
    /// 200 that bring it in from under the keyboard.
    #[test]
    fn a_scroll_inside_a_list_partly_under_the_keyboard_strokes_through_what_shows_of_it() {
        let (mut target, strokes) = noting_strokes(&["initial_tab=list"], true);
        let found = scroll_with(
            &mut target,
            json!({"direction": "down", "amount": 600, "until": "#row-25", "target": "#list"}),
        );
        let strokes = strokes.lock().unwrap().clone();
        for (from, to) in &strokes {
            assert!(
                [from.y, to.y].iter().all(|&y| y > 24.0 && y < KEYBOARD_TOP),
                "every stroke inside the list above the keyboard: {strokes:?}"
            );
            assert!(
                (from.y - to.y).abs() <= 0.8 * (KEYBOARD_TOP - 24.0),
                "no stroke longer than 80% of what shows: {strokes:?}"
            );
        }
        let data = found.unwrap().data;
        assert_eq!(data["strokes"], 2, "{data}");
        assert_eq!(data["amount"], 440.0, "{data}");
        assert_eq!(data["visible"], 1.0, "{data}");
        let lengths: Vec<f64> = strokes.iter().map(|(from, to)| from.y - to.y).collect();
        assert_eq!(lengths, [440.0, 200.0], "{strokes:?}");
    }

    /// Task 10.32: an element the keyboard covers leaves no room for a
    /// stroke, so a scroll inside it fails with `covered_by_keyboard`, with
    /// `until` or without, before it sends any stroke: on the fake, the List
    /// tab's button under the keyboard while the username field has focus.
    #[test]
    fn a_scroll_inside_an_element_the_keyboard_covers_fails_naming_the_keyboard() {
        let (mut target, strokes) = noting_strokes(&[], false);
        fill_in(&mut target, "#login-username", "agent").unwrap();
        for until in [None, Some("#row-1")] {
            let mut params = json!({"direction": "down", "amount": 100, "target": "#tab-list"});
            if let Some(until) = until {
                params["until"] = until.into();
            }
            let failure = scroll_with(&mut target, params).expect_err("the keyboard covers it");
            assert_eq!(failure.code, "covered_by_keyboard", "{}", failure.message);
            assert!(
                failure.message.contains("#tab-list") && failure.message.contains("press Back"),
                "the message names what the keyboard covers and how to close it: {}",
                failure.message
            );
        }
        assert!(strokes.lock().unwrap().is_empty());
        assert_eq!(held(&mut target, "#login-username"), "agent");
    }

    /// The placeholder app: one screen with the shapes the production-app
    /// smoke test met (`crate::fake`).
    const PLACEHOLDER: &str = "dev.mobileagent.placeholder";

    /// Every input a target was sent, with when it was sent.
    type Sent = Arc<Mutex<Vec<(Instant, Input)>>>;

    /// An input sink that notes every input it passes on, and when.
    struct Recording {
        inner: Box<dyn InputSink>,
        sent: Sent,
    }

    impl InputSink for Recording {
        fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError> {
            self.sent
                .lock()
                .unwrap()
                .push((Instant::now(), input.clone()));
            self.inner.perform(input)
        }
    }

    /// The first target `driver` lists, attached, with `app` open as `open`
    /// with `env` opens it, and the inputs it is sent from then on.
    fn recorded(driver: &FakeDriver, app: &str, env: &[&str]) -> (Attached, Sent) {
        let info = driver.discover().unwrap().remove(0);
        let mut attachment = driver.attach(&info, &AttachOptions::default()).unwrap();
        let sent = Sent::default();
        attachment.backend.input = Box::new(Recording {
            inner: attachment.backend.input,
            sent: Arc::clone(&sent),
        });
        let mut target = Attached::new(&info, attachment, &Arc::new(Mutex::default()));
        let open = json!({"action": "open", "app": app, "env": env}).to_string();
        super::super::run_on(&mut target, "default", &request(&open)).unwrap();
        sent.lock().unwrap().clear();
        (target, sent)
    }

    /// Where the taps among `sent` went, in points.
    fn taps(sent: &Sent) -> Vec<Point> {
        sent.lock()
            .unwrap()
            .iter()
            .filter_map(|(_, input)| match input {
                Input::Tap { at, .. } => Some(*at),
                _ => None,
            })
            .collect()
    }

    /// The ref an interactive snapshot gives the element named `name`, as
    /// `@eN`.
    fn ref_named(target: &mut Attached, name: &str) -> String {
        let listing = super::super::run_on(
            target,
            "default",
            &request(r#"{"action": "snapshot", "interactive": true}"#),
        )
        .unwrap()
        .data;
        listing["refs"]
            .as_object()
            .and_then(|refs| {
                refs.iter()
                    .find(|(_, entry)| entry["name"] == name)
                    .map(|(reference, _)| format!("@{reference}"))
            })
            .unwrap_or_else(|| panic!("no ref for {name}: {}", listing["snapshot"]))
    }

    /// `click` on what `aimed` names.
    fn click_on(target: &mut Attached, aimed: &str) -> Outcome {
        let click = json!({"action": "click", "target": aimed}).to_string();
        super::super::run_on(target, "default", &request(&click))
    }

    /// The text of the element `aimed` names, as `get text` reads it.
    fn text_of(target: &mut Attached, aimed: &str) -> Value {
        let get = json!({"action": "get", "what": "text", "target": aimed}).to_string();
        super::super::run_on(target, "default", &request(&get))
            .unwrap()
            .data["text"]
            .clone()
    }

    /// Design D32: in the production-app smoke test the keyboard's appearance
    /// moved a submit button up without an event, the replica still held its
    /// old bounds, none of them in view under the keyboard, and the click
    /// tapped the number pad there. On the placeholder app's screen the
    /// layout moves the button above the keyboard 50 ms after the keyboard
    /// came up, in no frame: a click on the button by the ref an earlier
    /// snapshot gave it reads the window again and taps the button where it
    /// is now, its centre at 499 points rather than 724.
    #[test]
    fn a_click_on_a_button_the_keyboard_moved_taps_it_where_it_is_now() {
        let driver = FakeDriver::parse("ios").unwrap();
        let (mut target, sent) = recorded(&driver, PLACEHOLDER, &[]);
        let submit = ref_named(&mut target, "Placeholder submit");
        click_on(&mut target, "#placeholder-code").unwrap();
        std::thread::sleep(Duration::from_millis(150));
        let reply = click_on(&mut target, &submit);
        let status = text_of(&mut target, "#placeholder-status");
        assert_eq!(
            status,
            "Submitted",
            "the click pressed the button: {reply:?}, taps {:?}",
            taps(&sent)
        );
        let success = reply.unwrap();
        assert_eq!(success.data["y"], 499.0, "{}", success.data);
        assert!(success.warning.is_none(), "{:?}", success.warning);
    }

    /// Design D32: a button the keyboard still covers once its window was
    /// read again fails the click with `element_offscreen`, the message
    /// naming the button and the keyboard, and nothing is tapped.
    #[test]
    fn a_click_on_a_button_the_keyboard_still_covers_fails_naming_it_and_taps_nothing() {
        let driver = FakeDriver::parse("ios").unwrap();
        let (mut target, sent) = recorded(&driver, PLACEHOLDER, &[]);
        click_on(&mut target, "#placeholder-code").unwrap();
        std::thread::sleep(Duration::from_millis(150));
        let tapped = taps(&sent).len();
        let failure = click_on(&mut target, "#placeholder-help").expect_err(&format!(
            "the keyboard covers the help button; taps {:?}",
            taps(&sent)
        ));
        assert_eq!(failure.code, "element_offscreen", "{}", failure.message);
        assert!(
            failure.message.contains("#placeholder-help")
                && failure.message.contains("\"Placeholder help\"")
                && failure.message.contains("keyboard"),
            "the message names the button and what covers it: {}",
            failure.message
        );
        assert_eq!(
            taps(&sent).len(),
            tapped,
            "nothing tapped: {:?}",
            taps(&sent)
        );
        assert_eq!(text_of(&mut target, "#placeholder-status"), "Not submitted");
    }

    /// Designs D32 and D37: a button partly in view keeps the click's
    /// warning, and the tap goes to the part of it in view: the keyboard
    /// covers all but the hint button's top 14 of 48 points, and the tap
    /// goes to their centre, where the production-app run's taps at the centres
    /// of submit buttons 1 to 35% in view hit the keyboard.
    #[test]
    fn a_click_on_a_button_partly_in_view_taps_the_part_in_view_with_a_warning() {
        let driver = FakeDriver::parse("ios").unwrap();
        let (mut target, sent) = recorded(&driver, PLACEHOLDER, &[]);
        click_on(&mut target, "#placeholder-code").unwrap();
        std::thread::sleep(Duration::from_millis(150));
        let success = click_on(&mut target, "#placeholder-hint").unwrap();
        let warning = success.warning.clone().unwrap_or_default();
        assert!(
            warning.contains("#placeholder-hint was not mostly in view (29% of it in view)"),
            "{warning}"
        );
        assert_eq!(success.data["y"], 532.0, "{}", success.data);
        assert_eq!(taps(&sent).last(), Some(&Point::new(201.0, 532.0)));
        assert_eq!(text_of(&mut target, "#placeholder-status"), "Hint shown");
    }

    /// The placeholder app on a fake Simulator, showing `board`, one of the
    /// screens of the production-app run, and the inputs it is sent.
    fn on_board(board: &str) -> (Attached, Sent) {
        let driver = FakeDriver::parse("ios").unwrap();
        recorded(
            &driver,
            PLACEHOLDER,
            &[&format!("PLACEHOLDER_SCREEN={board}")],
        )
    }

    /// Design D37, scenario "Card partly under a sticky header" (task 3.1;
    /// section 2 of the masked listings): on the feed the production-app run's
    /// click on a card 30% in view tapped its centre, (201, 99), on the
    /// heading of the header listed before the list, with the warning that
    /// the card was not mostly in view. The click taps the centre of the
    /// part of the card in view, below the header, and keeps the warning.
    #[test]
    fn a_click_on_a_card_partly_above_its_list_taps_the_part_in_view_with_a_warning() {
        let (mut target, sent) = on_board("feed");
        let card = ref_named(&mut target, "placeholder-button-4");
        let success = click_on(&mut target, &card).unwrap();
        assert_eq!(
            text_of(&mut target, "#placeholder-status"),
            "Tapped placeholder-id-4",
            "taps {:?}",
            taps(&sent)
        );
        assert_eq!(success.data["x"], 201.0, "{}", success.data);
        assert_eq!(success.data["y"], 139.5, "{}", success.data);
        let warning = success.warning.unwrap_or_default();
        assert!(
            warning.contains(&format!(
                "{card} was not mostly in view (30% of it in view)"
            )),
            "{warning}"
        );
    }

    /// Design D37, scenario "Heading half under a footer button" (task 3.1;
    /// section 1a of the masked listings): on the consent screen the
    /// production-app run's click on the heading half under the secondary
    /// footer button tapped its centre, the button's top edge, and declined
    /// the consent; its click by exact text on the 12-point text under the
    /// primary button tapped that button and consented. The click on the
    /// heading taps its upper half, with the warning; the click by text,
    /// taking the same path, finds 3 points of the text below the primary
    /// button, too few, and fails with `element_covered` naming the button,
    /// tapping nothing.
    #[test]
    fn clicks_on_what_footer_buttons_cover_in_part_tap_what_shows_or_are_refused_naming_the_button()
    {
        let (mut target, sent) = on_board("consent");
        let heading = ref_named(&mut target, "placeholder-heading-9");
        let success = click_on(&mut target, &heading).unwrap();
        assert_eq!(
            text_of(&mut target, "#placeholder-status"),
            "Tapped placeholder-heading-9",
            "taps {:?}",
            taps(&sent)
        );
        assert_eq!(success.data["y"], 714.5, "{}", success.data);
        let warning = success.warning.unwrap_or_default();
        assert!(warning.contains("(50% of it in view)"), "{warning}");

        let tapped = taps(&sent).len();
        let failure = click_on(&mut target, "text==placeholder-text-8").expect_err(&format!(
            "the primary button covers the text; taps {:?}",
            taps(&sent)
        ));
        assert_eq!(failure.code, "element_covered", "{}", failure.message);
        assert!(
            failure.message.contains("button \"placeholder-button-2\""),
            "{}",
            failure.message
        );
        assert_eq!(taps(&sent).len(), tapped, "nothing tapped");
        assert_eq!(
            text_of(&mut target, "#placeholder-status"),
            "Tapped placeholder-heading-9"
        );
    }

    /// Designs D37, D38 and D45 (tasks 3.1 and 4.3; section 1b of the
    /// masked listings, task 3.7's N6): on the creation tab the paste
    /// control lies 38 of its 48 points under the disabled primary button,
    /// and the second production-app run's clicks at its centre landed on that
    /// button, twice; its lower 10 points lie under the footer drawn over
    /// the list's lower part, outside what the list shows, and the third
    /// run's click tapped them and opened nothing. The click by identifier
    /// is refused with `element_covered`, naming the disabled button a tap
    /// at the control's centre would hit, and taps nothing.
    #[test]
    fn a_click_on_a_control_whose_strip_below_a_disabled_button_lies_under_a_footer_is_refused() {
        let (mut target, sent) = on_board("creation");
        let failure = click_on(&mut target, "#placeholder-id-8").expect_err(&format!(
            "the footer covers what the disabled button leaves; taps {:?}",
            taps(&sent)
        ));
        assert_eq!(failure.code, "element_covered", "{}", failure.message);
        assert!(
            failure.message.contains("button \"placeholder-button-21\""),
            "{}",
            failure.message
        );
        assert!(taps(&sent).is_empty(), "nothing tapped: {:?}", taps(&sent));
        assert_eq!(text_of(&mut target, "#placeholder-status"), "Not tapped");
    }

    /// Design D48 (task 5.1; task 4.7, N6): on the creation tab as React
    /// Native's accessibility lists it, the footer's view holds nothing, its
    /// tabs listed after it, and the fourth production-app run's click on the
    /// paste control tapped its lower 10 points, below the disabled button
    /// and under the footer, opening nothing. The click by identifier is
    /// refused with `element_covered`, naming the disabled button a tap at
    /// the control's centre would hit, and taps nothing.
    #[test]
    fn a_click_on_a_control_whose_strip_lies_under_a_footer_react_native_lists_empty_is_refused() {
        let (mut target, sent) = on_board("creation-rn");
        let failure = click_on(&mut target, "#placeholder-id-8").expect_err(&format!(
            "the footer covers what the disabled button leaves; taps {:?}",
            taps(&sent)
        ));
        assert_eq!(failure.code, "element_covered", "{}", failure.message);
        assert!(
            failure.message.contains("button \"placeholder-button-21\""),
            "{}",
            failure.message
        );
        assert!(taps(&sent).is_empty(), "nothing tapped: {:?}", taps(&sent));
        assert_eq!(text_of(&mut target, "#placeholder-status"), "Not tapped");
    }

    /// `click` on what `aimed` names, waiting at most `timeout` ms for a
    /// selector's element.
    fn click_within(target: &mut Attached, aimed: &str, timeout: u64) -> Outcome {
        let click = json!({"action": "click", "target": aimed, "timeout": timeout}).to_string();
        super::super::run_on(target, "default", &request(&click))
    }

    /// `fill` of `text` into what `aimed` names, waiting at most `timeout`
    /// ms for a selector's element.
    fn fill_within(target: &mut Attached, aimed: &str, text: &str, timeout: u64) -> Outcome {
        let fill = json!({"action": "fill", "target": aimed, "text": text, "timeout": timeout});
        super::super::run_on(target, "default", &request(&fill.to_string()))
    }

    /// The longest a refusal by selector may take: the time a ref takes, a
    /// read or two, well short of the selector's timeout of 3 s.
    const REFUSED_WITHIN: Duration = Duration::from_millis(500);

    /// Design D53, N12 of the fifth production-app run (task 6.2; task 5.6): a
    /// click by text on a text the declined consent's secondary button
    /// covers whole, as the run's 12-point text at 737 points under its
    /// Decline, answered `element_not_found` after the full timeout, 10,079
    /// ms, the selector waiting for the text on screen, where a ref answers
    /// `element_covered` at once. The selector resolves the text the replica
    /// holds, by containment or exact text, and the click takes the path a
    /// ref takes: refused naming the button within a read, nothing tapped.
    #[test]
    fn a_click_by_text_on_a_text_a_button_covers_whole_is_refused_naming_it_at_once() {
        let (mut target, sent) = on_board("consent");
        for aimed in ["text=placeholder-text-7", "text==placeholder-text-7"] {
            let started = Instant::now();
            let failure = click_within(&mut target, aimed, 3000)
                .expect_err(&format!("{aimed}: the button covers the text"));
            let took = started.elapsed();
            assert_eq!(
                failure.code, "element_covered",
                "{aimed}: {}",
                failure.message
            );
            assert!(
                failure.message.contains("button \"placeholder-button-1\""),
                "{aimed}: {}",
                failure.message
            );
            assert!(took < REFUSED_WITHIN, "{aimed}: {took:?}");
        }
        assert!(taps(&sent).is_empty(), "nothing tapped: {:?}", taps(&sent));
        assert_eq!(text_of(&mut target, "#placeholder-status"), "Not tapped");
    }

    /// Design D53, N12 and N6 of the fifth production-app run (task 6.2; task
    /// 5.6): on the creation tab as that run met it the footer's view holds
    /// the paste control and the button beside it whole, which the
    /// interactive listing leaves out, and a click by the control's
    /// identifier before any scroll answered `element_not_found` after the
    /// full timeout, 10,069 ms. It is refused with `element_covered` within
    /// a read, and nothing is tapped; the refusal names the footer, as the
    /// hint counting the control does, where the sixth run's named "the
    /// overlay" (design D57; task 6.5, N6).
    #[test]
    fn a_click_by_identifier_on_a_control_a_view_holds_whole_is_refused_at_once() {
        let (mut target, sent) = on_board("creation-held");
        let listing = target
            .mirror
            .snapshot(&SnapshotOptions::interactive())
            .unwrap();
        assert!(
            listing.text.contains("2 of them under the footer"),
            "{}",
            listing.text
        );
        let started = Instant::now();
        let failure = click_within(&mut target, "#placeholder-id-8", 3000)
            .expect_err("the footer's view holds the control");
        let took = started.elapsed();
        assert_eq!(failure.code, "element_covered", "{}", failure.message);
        assert!(
            failure.message.contains("lies under the footer, "),
            "{}",
            failure.message
        );
        assert!(took < REFUSED_WITHIN, "{took:?}");
        assert!(taps(&sent).is_empty(), "nothing tapped: {:?}", taps(&sent));
        assert_eq!(text_of(&mut target, "#placeholder-status"), "Not tapped");
    }

    /// Design D53, N12 of the fifth production-app run (task 6.2; task 5.6):
    /// behind a dialog the app marks modal, a fill by the identifier of the
    /// field under it answered `element_not_found` after the full timeout,
    /// 10,090 ms, where a click on a ref behind it answered
    /// `element_covered`, "covered by the modal view", in 93 ms. The fill is
    /// refused the same way within a read, and nothing is tapped or typed.
    #[test]
    fn a_fill_by_identifier_on_a_field_behind_a_dialog_is_refused_naming_it_at_once() {
        let (mut target, sent) = on_board("dialog");
        let started = Instant::now();
        let failure = fill_within(&mut target, "#placeholder-name", "placeholder-value", 3000)
            .expect_err("the dialog covers the field");
        let took = started.elapsed();
        assert_eq!(failure.code, "element_covered", "{}", failure.message);
        assert!(
            failure.message.contains("modal view"),
            "{}",
            failure.message
        );
        assert!(took < REFUSED_WITHIN, "{took:?}");
        assert!(taps(&sent).is_empty(), "nothing tapped: {:?}", taps(&sent));
        assert!(texts(&sent).is_empty(), "nothing typed: {:?}", texts(&sent));
    }

    /// Design D53: the selector waits only for an element the tree does not
    /// hold yet: a click and a fill on an identifier nothing holds wait out
    /// their timeout and fail with `element_not_found` naming the screen.
    #[test]
    fn a_selector_on_an_element_the_tree_does_not_hold_waits_its_timeout() {
        let (mut target, sent) = on_board("dialog");
        let timeout = 400;
        let started = Instant::now();
        let failure = click_within(&mut target, "#placeholder-nothing", timeout).unwrap_err();
        assert_eq!(failure.code, "element_not_found", "{}", failure.message);
        assert!(started.elapsed() >= Duration::from_millis(timeout));
        assert!(
            failure.message.contains("placeholder-button-2"),
            "{}",
            failure.message
        );
        let started = Instant::now();
        let failure = fill_within(
            &mut target,
            "#placeholder-nothing",
            "placeholder-value",
            timeout,
        )
        .unwrap_err();
        assert_eq!(failure.code, "element_not_found", "{}", failure.message);
        assert!(started.elapsed() >= Duration::from_millis(timeout));
        assert!(taps(&sent).is_empty(), "nothing tapped: {:?}", taps(&sent));
    }

    /// Design D48, scenario "React Native scroll view" (task 5.1): on the
    /// creation tab as React Native's accessibility lists it, `scroll
    /// --until` strokes through what the list shows, above the footer drawn
    /// over its lower part, its strokes as long as three quarters of that,
    /// so none comes down on the disabled button floating over the list's
    /// last part or on the footer, either of which would take the touch and
    /// move nothing, and in strokes of up to 600 points it reaches the
    /// list's last button.
    #[test]
    fn scroll_until_strokes_inside_what_a_scroll_view_shows_above_a_footer() {
        let (mut target, sent) = on_board("creation-rn");
        let reply = scroll_with(
            &mut target,
            json!({"direction": "down", "amount": 600, "until": "text==placeholder-button-22"}),
        );
        let strokes = swipes(&sent);
        let data = reply
            .unwrap_or_else(|failure| panic!("{failure:?}; strokes {strokes:?}"))
            .data;
        assert_eq!(data["visible"], 1.0, "{data}");
        assert!(!strokes.is_empty(), "{data}");
        for (start, end) in &strokes {
            for point in [start, end] {
                assert!(
                    point.y > 122.0 && point.y < 708.0,
                    "inside what the list shows, above the button: {strokes:?}"
                );
            }
        }
    }

    /// Design D37 (task 3.1; section 1b of the masked listings): the
    /// production-app run's `scroll --until` on the paste control answered at
    /// once, `visible` 1.0, no stroke, while the disabled button covered
    /// most of it. It judges arrival by the part a tap reaches: one stroke
    /// brings the control above the button, and the reply says so.
    #[test]
    fn scroll_until_does_not_call_a_control_mostly_under_a_disabled_button_visible() {
        let (mut target, _) = on_board("creation");
        let data = scroll_with(
            &mut target,
            json!({"direction": "down", "until": "#placeholder-id-8"}),
        )
        .unwrap()
        .data;
        assert_eq!(data["strokes"], 1, "{data}");
        assert_eq!(data["visible"], 1.0, "{data}");
        let bottom = data["element"]["bounds"][3].as_f64().unwrap_or_default();
        assert!(bottom <= 708.0, "above the disabled button: {data}");
    }

    /// Design D46, N5 of the third production-app run (task 4.4; task 3.7): on
    /// the creation tab `scroll down --until` the paste control failed with
    /// `element_not_found`, the content having stopped moving after three
    /// strokes, while a plain `scroll down 300` right after moved the content
    /// 300 points. A plain scroll of 300 points lifts the finger up through
    /// the list and moves its content 300 points; `scroll down --until` the
    /// control strokes the same way through the same place, and the control
    /// ends above the disabled button.
    #[test]
    fn scroll_until_on_the_creation_tab_strokes_the_way_a_plain_scroll_moves_the_list() {
        let paste_top = |target: &mut Attached| {
            target
                .mirror
                .snapshot(&SnapshotOptions::full())
                .unwrap()
                .by_identifier("placeholder-id-8")
                .map(|paste| paste.bounds.top)
        };
        let (mut plain, sent) = on_board("creation");
        scroll_with(&mut plain, json!({"direction": "down", "amount": 300})).unwrap();
        let strokes = swipes(&sent);
        assert_eq!(strokes.len(), 1, "{strokes:?}");
        let (from, to) = strokes[0];
        assert!(from.y > to.y, "the finger goes up: {strokes:?}");
        assert_eq!(paste_top(&mut plain), Some(426.0), "the content moves 300");

        let (mut target, sent) = on_board("creation");
        let reply = scroll_with(
            &mut target,
            json!({"direction": "down", "until": "#placeholder-id-8"}),
        );
        let strokes = swipes(&sent);
        let data = reply
            .unwrap_or_else(|failure| panic!("{failure:?}; strokes {strokes:?}"))
            .data;
        assert_eq!(data["visible"], 1.0, "{data}");
        assert!(!strokes.is_empty(), "{data}");
        for (start, end) in &strokes {
            assert!(
                start.y > end.y && start.x == from.x,
                "the plain scroll's way: {strokes:?}"
            );
        }
        let bottom = data["element"]["bounds"][3].as_f64().unwrap_or_default();
        assert!(bottom <= 708.0, "above the disabled button: {data}");
    }

    /// Design D46 (task 4.4; task 3.7's N5): with nothing drawn over the
    /// creation tab's list below the disabled button, the paste control fits
    /// below the button with the least movement, but the list stands at its
    /// top and cannot carry its content down. `scroll down --until` the
    /// control strokes the way a plain scroll does and brings the control
    /// above the button, rather than stroking toward a top the list cannot
    /// pass until the content stops moving.
    #[test]
    fn scroll_until_strokes_toward_a_side_the_list_can_carry_the_control_to() {
        let (mut target, sent) = on_board("creation-open");
        let reply = scroll_with(
            &mut target,
            json!({"direction": "down", "until": "#placeholder-id-8"}),
        );
        let strokes = swipes(&sent);
        let data = reply
            .unwrap_or_else(|failure| panic!("{failure:?}; strokes {strokes:?}"))
            .data;
        assert_eq!(data["visible"], 1.0, "{data}");
        for (start, end) in &strokes {
            assert!(start.y > end.y, "the finger goes up: {strokes:?}");
        }
        let bottom = data["element"]["bounds"][3].as_f64().unwrap_or_default();
        assert!(bottom <= 708.0, "above the disabled button: {data}");
    }

    /// Design D46 (task 4.4; task 3.7's N5): the third production-app run's
    /// `scroll --until` gave up, the content having stopped moving, "after 3
    /// strokes of 300 points down", and nothing in the reply said where the
    /// strokes ran, the other way as it was. A scroll that looks for an
    /// element and stops names its last stroke's start and end, in its
    /// message and its data, and says what moved after it: on the Login tab,
    /// which holds no row, nothing.
    #[test]
    fn a_scroll_until_that_stops_names_its_last_stroke_and_what_moved() {
        let (mut target, strokes) = noting_strokes(&[], false);
        let failure = scroll_with(
            &mut target,
            json!({"direction": "down", "amount": 600, "until": "#row-150", "timeout": 3000}),
        )
        .expect_err("no row on the Login tab");
        let strokes = strokes.lock().unwrap().clone();
        let (from, to) = *strokes.last().expect("strokes");
        assert!(
            failure.message.contains(&format!(
                "the last from {from} to {to}, after which nothing moved"
            )),
            "{}",
            failure.message
        );
        let data = failure.data.unwrap_or_default();
        assert_eq!(data["from"], json!([from.x, from.y]), "{data}");
        assert_eq!(data["to"], json!([to.x, to.y]), "{data}");
    }

    /// Designs D31 and D32: a footer listed after a button that lies wholly
    /// under it is drawn over the button, which is also 0% in view under the
    /// footer, a bar. A click on the button by the ref an earlier snapshot
    /// gave it fails with `element_covered`, naming the accept button a tap
    /// there would land on, without reading the windows again, which is for
    /// bounds out of date, and without a tap.
    #[test]
    fn a_click_on_a_button_a_footer_is_drawn_over_fails_covered_without_a_read_or_a_tap() {
        let driver = FakeDriver::parse("ios").unwrap();
        let (mut target, sent) = recorded(&driver, PLACEHOLDER, &[]);
        let terms = ref_named(&mut target, "Placeholder terms");
        click_on(&mut target, "#placeholder-hint").unwrap();
        ref_named(&mut target, "Placeholder accept");
        let tapped = taps(&sent).len();
        let changed = target.mirror.timestamps().last_tree_change;
        let failure = click_on(&mut target, &terms).expect_err(&format!(
            "the footer covers the terms button; taps {:?}",
            taps(&sent)
        ));
        assert_eq!(failure.code, "element_covered", "{}", failure.message);
        assert!(
            failure.message.contains("\"Placeholder accept\""),
            "the message names what a tap there would land on: {}",
            failure.message
        );
        assert_eq!(
            taps(&sent).len(),
            tapped,
            "nothing tapped: {:?}",
            taps(&sent)
        );
        assert_eq!(
            target.mirror.timestamps().last_tree_change,
            changed,
            "the windows were not read again"
        );
        assert_eq!(text_of(&mut target, "#placeholder-status"), "Hint shown");
    }

    /// Design D32: `click` makes no stroke; a row of a list the keyboard
    /// covers all of fails the click with `element_offscreen`, which names
    /// `scroll --until`, and `scroll --until` brings the row in with one
    /// stroke, after which the click opens it.
    #[test]
    fn a_row_under_the_keyboard_is_refused_by_click_and_brought_in_by_scroll_until() {
        for (platform, tab, row, detail) in [
            ("android", "initial_tab=list", "#row-13", "#detail-13"),
            ("ios", "FIXTURE_INITIAL_TAB=list", "#row-11", "#detail-11"),
        ] {
            let driver = FakeDriver::parse(platform).unwrap().with_keyboard(true);
            let (mut target, sent) = recorded(&driver, FIXTURE, &[tab]);
            let failure = click_on(&mut target, row).expect_err(&format!(
                "{platform}: the keyboard covers {row}; taps {:?}",
                taps(&sent)
            ));
            assert_eq!(
                failure.code, "element_offscreen",
                "{platform}: {}",
                failure.message
            );
            assert!(
                failure.message.contains(&format!("scroll --until {row}")),
                "{platform}: {}",
                failure.message
            );
            assert!(taps(&sent).is_empty(), "{platform}: {:?}", taps(&sent));
            let scrolled = scroll_with(
                &mut target,
                json!({"direction": "down", "amount": 300, "until": row, "target": "#list"}),
            )
            .unwrap()
            .data;
            assert_eq!(scrolled["strokes"], 1, "{platform}: {scrolled}");
            assert_eq!(scrolled["visible"], 1.0, "{platform}: {scrolled}");
            let opened = click_on(&mut target, row).unwrap();
            assert!(opened.warning.is_none(), "{platform}: {:?}", opened.warning);
            assert!(
                text_of(&mut target, detail)
                    .as_str()
                    .is_some_and(|text| text.starts_with("Detail")),
                "{platform}"
            );
        }
    }

    /// Where the strokes among `sent` went, from where each finger came down
    /// to where it lifted.
    fn swipes(sent: &Sent) -> Vec<(Point, Point)> {
        sent.lock()
            .unwrap()
            .iter()
            .filter_map(|(_, input)| match input {
                Input::Swipe { from, to, .. } => Some((*from, *to)),
                _ => None,
            })
            .collect()
    }

    /// Design D41 (task 3.2; section 3 of the masked listings): with the
    /// keyboard up the production-app run's `scroll up 300` stroked from (201,
    /// 119.5), the middle of what the keyboard left of the screen less 150
    /// points, on the header listed before the list, and moved nothing. A
    /// scroll of the screen with the keyboard up strokes inside what shows
    /// of the scroll container there: below the header's 122 points and
    /// above the keyboard's top at 539.
    #[test]
    fn a_scroll_with_the_keyboard_up_strokes_inside_the_list_below_its_header() {
        let (mut target, sent) = on_board("list");
        click_on(&mut target, "#placeholder-id-2").unwrap();
        let reply = scroll_with(&mut target, json!({"direction": "up", "amount": 300})).unwrap();
        let strokes = swipes(&sent);
        assert_eq!(strokes.len(), 1, "{}", reply.data);
        for (from, to) in &strokes {
            assert!(
                [from.y, to.y].iter().all(|&y| y > 122.0 && y < 539.0),
                "below the header, above the keyboard: {strokes:?}"
            );
        }
    }

    /// D41: under a header down to 430 points the keyboard leaves 109
    /// points of the list, fewer than the 120 a stroke needs, so the scroll
    /// fails with `covered_by_keyboard`, advising to close the keyboard,
    /// rather than stroking the header, and strokes nothing.
    #[test]
    fn a_scroll_of_a_list_the_keyboard_leaves_less_than_120_points_of_fails_naming_it() {
        let (mut target, sent) = on_board("list-low");
        click_on(&mut target, "#placeholder-id-2").unwrap();
        let failure =
            scroll_with(&mut target, json!({"direction": "up", "amount": 300})).expect_err(
                &format!("109 points to stroke; strokes {:?}", swipes(&sent)),
            );
        assert_eq!(failure.code, "covered_by_keyboard", "{}", failure.message);
        assert!(
            failure.message.contains("close the keyboard"),
            "{}",
            failure.message
        );
        assert!(swipes(&sent).is_empty(), "{:?}", swipes(&sent));
    }

    /// Design D33: a React Native code input built of boxes holds its code
    /// in the boxes' texts and has no value of its own, so in the production-app smoke test
    /// `fill` typed the code and then failed with `text_mismatch`. A field
    /// without a value is read back through its descendants' values or
    /// texts, in order: six digits are verified through the six boxes, with
    /// no retry and no warning. A field with a value is read back from it.
    #[test]
    fn a_fill_of_a_code_into_a_field_of_boxes_is_verified_through_their_texts() {
        for (platform, typing, method) in [
            ("ios", Typing::Keys, "keys"),
            ("android", Typing::Whole, "direct"),
        ] {
            let driver = FakeDriver::parse(platform).unwrap().with_typing(typing);
            let (mut target, _) = recorded(&driver, PLACEHOLDER, &[]);
            let success = fill_in(&mut target, "#placeholder-code", "123456").unwrap();
            let data = &success.data;
            assert_eq!(data["verified"], "text", "{platform}: {data}");
            assert_eq!(data["readFrom"], "descendants", "{platform}: {data}");
            assert_eq!(data["method"], method, "{platform}: {data}");
            assert_eq!(data["retried"], false, "{platform}: {data}");
            assert!(
                success.warning.is_none(),
                "{platform}: {:?}",
                success.warning
            );

            let (mut login, _) = recorded(&driver, FIXTURE, &[]);
            let success = fill_in(&mut login, "#login-username", "agent").unwrap();
            assert_eq!(
                success.data["readFrom"], "value",
                "{platform}: {}",
                success.data
            );
        }
    }

    /// Design D33: a field the tree shows without a value and with nothing
    /// below it, as iOS shows a React Native field inside a view marked
    /// accessible, takes the text, and the fill succeeds unverified, saying
    /// why, once the screen has held still a settle's quiet window after the
    /// entry, rather than failing with `text_mismatch`; the text goes in once.
    #[test]
    fn a_fill_into_a_field_with_no_value_to_read_succeeds_unverified() {
        for platform in ["ios", "android"] {
            let driver = FakeDriver::parse(platform)
                .unwrap()
                .with_typing(Typing::Keys);
            let (mut target, sent) = recorded(&driver, PLACEHOLDER, &[]);
            let reply = fill_in(&mut target, "#placeholder-secret", "agent");
            let success = reply.unwrap_or_else(|failure| panic!("{platform}: {failure:?}"));
            let data = &success.data;
            assert_eq!(data["verified"], false, "{platform}: {data}");
            assert_eq!(data["reason"], "no_value_to_read", "{platform}: {data}");
            assert!(data["readFrom"].is_null(), "{platform}: {data}");
            assert_eq!(data["retried"], false, "{platform}: {data}");
            assert!(
                data["readBackMs"]
                    .as_f64()
                    .is_some_and(|ms| (200.0..600.0).contains(&ms)),
                "{platform}: {data}"
            );
            let texts = sent
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, input)| matches!(input, Input::Text { .. }))
                .count();
            assert_eq!(texts, 1, "{platform}: the text went in once");
        }
    }

    /// The placeholder app on a fake Simulator, showing `board`, its fields
    /// taking text as `typing` says, and the inputs it is sent.
    fn typed_board(board: &str, typing: Typing) -> (Attached, Sent) {
        let driver = FakeDriver::parse("ios").unwrap().with_typing(typing);
        recorded(
            &driver,
            PLACEHOLDER,
            &[&format!("PLACEHOLDER_SCREEN={board}")],
        )
    }

    /// Design D39 (task 3.3; section 4 of the masked listings): on the code
    /// step's first visit the code field is a labelled node without traits
    /// whose accessible name spells the digits it holds, neither a value nor
    /// anything below it, so every fill of the production-app run answered
    /// `verified: false`, `no_value_to_read`, and a wrong code went
    /// unreported. The read-back takes the field's name when it changed
    /// with the entry: the code is verified through it, and a field that
    /// keeps four of the six digits fails with `text_mismatch` naming what
    /// it holds after the retry. A name that did not change is a label and
    /// is not compared: the form's secret field, whose name stays, is
    /// unverified as before.
    #[test]
    fn a_fill_of_a_code_into_a_field_whose_name_spells_it_is_verified_through_the_name() {
        let (mut target, _) = typed_board("code", Typing::Keys);
        let success = fill_in(&mut target, "#placeholder-id-3", "123456").unwrap();
        let data = &success.data;
        assert_eq!(data["verified"], "text", "{data}: {:?}", success.warning);
        assert_eq!(data["readFrom"], "name", "{data}");
        assert_eq!(data["retried"], false, "{data}");

        let (mut short, _) = typed_board("code", Typing::Max(4));
        let failure = fill_in(&mut short, "#placeholder-id-3", "123456").unwrap_err();
        assert_eq!(failure.code, TEXT_MISMATCH, "{}", failure.message);
        assert!(
            failure.message.contains("1 2 3 4") && failure.message.contains("\"123456\""),
            "the message names both: {}",
            failure.message
        );

        let driver = FakeDriver::parse("ios").unwrap().with_typing(Typing::Keys);
        let (mut form, _) = recorded(&driver, PLACEHOLDER, &[]);
        let success = fill_in(&mut form, "#placeholder-secret", "agent").unwrap();
        assert_eq!(success.data["verified"], false, "{}", success.data);
        assert_eq!(
            success.data["reason"], "no_value_to_read",
            "{}",
            success.data
        );
    }

    /// Design D43, defect 3 of the third production-app run (task 4.1; task
    /// 3.7): the code step's field names its six boxes before its label, a
    /// dash for each empty one, "–, –, –, –, –, –, Verification code,
    /// required" before the entry and "1, 2, 3, 4, 5, 6, Verification code,
    /// required" after, and every fill compared the code with the whole
    /// name, typed it, pasted it and failed with `text_mismatch`, the code
    /// right every time. The read-back compares the part of the name the
    /// entry changed, its separators left out: the code is verified through
    /// the name, typed once; a field that keeps four of the six digits fails
    /// naming the part that changed, not the label; and one that shows a
    /// bullet for each digit, which tells only the code's length, fails
    /// naming that length, keeping the code out of the message.
    #[test]
    fn a_fill_of_a_code_into_a_field_named_by_its_boxes_compares_what_the_entry_changed() {
        let (mut target, sent) = typed_board("code-named", Typing::Keys);
        let success = fill_in(&mut target, "#placeholder-id-3", "123456").unwrap();
        let data = &success.data;
        assert_eq!(data["verified"], "text", "{data}: {:?}", success.warning);
        assert_eq!(data["readFrom"], "name", "{data}");
        assert_eq!(data["retried"], false, "{data}");
        let texts = sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, input)| matches!(input, Input::Text { .. }))
            .count();
        assert_eq!(texts, 1, "the code went in once");

        let (mut short, _) = typed_board("code-named", Typing::Max(4));
        let failure = fill_in(&mut short, "#placeholder-id-3", "123456").unwrap_err();
        assert_eq!(failure.code, TEXT_MISMATCH, "{}", failure.message);
        assert!(
            failure
                .message
                .contains("holds \"1, 2, 3, 4\" where fill entered \"123456\""),
            "the message names the part that changed: {}",
            failure.message
        );
        assert_eq!(failure.data.unwrap_or_default()["value"], "1, 2, 3, 4");

        let (mut masked, _) = typed_board("code-masked", Typing::Keys);
        let failure = fill_in(&mut masked, "#placeholder-id-3", "123456").unwrap_err();
        assert_eq!(failure.code, TEXT_MISMATCH, "{}", failure.message);
        assert!(
            failure
                .message
                .contains("holds 6 masked characters where fill entered 6 characters"),
            "{}",
            failure.message
        );
        assert!(!failure.message.contains("123456"), "{}", failure.message);
    }

    /// The fixture's list on a fake Simulator scrolled to its end, row 200
    /// at its bottom and rows 1 to 186 above the screen, which the tree
    /// still holds.
    fn list_at_its_end() -> Attached {
        let driver = FakeDriver::parse("ios").unwrap();
        let (mut target, _) = recorded(&driver, FIXTURE, &["FIXTURE_INITIAL_TAB=list"]);
        let reply = scroll_with(
            &mut target,
            json!({"direction": "down", "amount": 600, "until": "#row-200"}),
        );
        reply.unwrap_or_else(|failure| panic!("the list scrolls to its end: {failure:?}"));
        target
    }

    /// A query: `get` of `what` of `aimed` or a `wait` for it in `state`,
    /// waiting at most `timeout` ms.
    fn query(
        target: &mut Attached,
        action: &str,
        what: &str,
        aimed: &str,
        timeout: u64,
    ) -> Outcome {
        let line = match action {
            "get" => json!({"action": "get", "what": what, "target": aimed, "timeout": timeout}),
            _ => json!({"action": "wait", "state": what, "target": aimed, "timeout": timeout}),
        };
        let line = line.to_string();
        match action {
            "get" => super::super::run_on(target, "default", &request(&line)),
            _ => super::super::run_on(target, "default", &request(&line)),
        }
    }

    /// Design D51, N10 of the fourth production-app run (task 5.4; task 4.7):
    /// on the feed scrolled to its end `get box` on an identifier and on a
    /// text of rows scrolled above failed `element_not_found` after the
    /// full 10 s, while the hint counted the rows above, which the tree
    /// held: a selector waited for an element on screen. `get` and a `wait`
    /// for presence resolve any element the replica holds, in view or not,
    /// at once, and say whether it is in view.
    #[test]
    fn get_and_a_wait_for_presence_resolve_a_row_scrolled_above_at_once_and_say_it_is_out_of_view()
    {
        let mut target = list_at_its_end();
        for aimed in ["#row-3", "text==Row 3"] {
            let started = Instant::now();
            let reply = query(&mut target, "get", "box", aimed, 2000);
            let data = reply
                .unwrap_or_else(|failure| panic!("{aimed}: {failure:?}"))
                .data;
            assert!(
                started.elapsed() < Duration::from_millis(500),
                "{aimed}: at once, not after {:?}",
                started.elapsed()
            );
            assert_eq!(data["inView"], false, "{aimed}: {data}");
            assert!(
                data["y"].as_f64().is_some_and(|y| y < 0.0),
                "{aimed}: above the screen: {data}"
            );
            assert_eq!(data["height"], 48.0, "{aimed}: {data}");
        }
        let text = query(&mut target, "get", "text", "#row-3", 2000)
            .unwrap()
            .data;
        assert_eq!(
            (&text["text"], &text["inView"]),
            (&json!("Row 3"), &json!(false))
        );
        let shown = query(&mut target, "get", "box", "#row-199", 2000)
            .unwrap()
            .data;
        assert_eq!(shown["inView"], true, "{shown}");

        let started = Instant::now();
        let waited = query(&mut target, "wait", "attached", "#row-3", 2000)
            .unwrap_or_else(|failure| panic!("{failure:?}"))
            .data;
        assert!(started.elapsed() < Duration::from_millis(500), "{waited}");
        assert_eq!(waited["element"]["id"], "row-3", "{waited}");
        assert_eq!(waited["inView"], false, "{waited}");
    }

    /// Design D51 (task 5.4): a wait for an element in view still waits for
    /// one in view, and only an element the tree does not hold, as a row
    /// once a row's detail has replaced the list, fails `element_not_found`
    /// naming what the screen lists.
    #[test]
    fn only_an_element_the_tree_does_not_hold_is_not_found_and_a_wait_for_one_in_view_still_waits()
    {
        let mut target = list_at_its_end();
        let failure = query(&mut target, "wait", "visible", "#row-3", 300).unwrap_err();
        assert_eq!(failure.code, "wait_timeout", "{}", failure.message);

        click_on(&mut target, "#row-200").unwrap();
        let failure = query(&mut target, "get", "box", "#row-3", 300).unwrap_err();
        assert_eq!(failure.code, "element_not_found", "{}", failure.message);
        assert!(
            failure.message.contains("the screen shows")
                && failure
                    .message
                    .contains("- button \"Back\" [id=detail-back"),
            "names what is listed: {}",
            failure.message
        );
    }

    /// The times the texts among `sent` went out, in order.
    fn text_times(sent: &Sent) -> Vec<Instant> {
        sent.lock()
            .unwrap()
            .iter()
            .filter(|(_, input)| matches!(input, Input::Text { .. }))
            .map(|(at, _)| *at)
            .collect()
    }

    /// Design D49, N8 of the fourth production-app run (task 5.2; task 4.7): a
    /// refill of the code the code step's field already holds answered
    /// `verified: false`, `no_value_to_read`, the field's name spelling the
    /// same digits before the entry and after it. A fill typed as keys into
    /// a field whose name spells the text already empties the field first,
    /// waits for the replica to show it emptied, and compares the name the
    /// entry leaves with the name of the emptied field: the code is
    /// verified through the name, typed once, into the field the emptying
    /// gave the focus.
    #[test]
    fn a_refill_of_the_code_a_field_holds_empties_it_first_and_is_verified_through_the_name() {
        for (board, targeted) in [("code-named", true), ("code-bare", false)] {
            let (mut target, sent) = typed_board(board, Typing::Keys);
            fill_in(&mut target, "#placeholder-id-3", "123456").unwrap();
            sent.lock().unwrap().clear();
            let reply = fill_in(&mut target, "#placeholder-id-3", "123456");
            let success = reply.unwrap_or_else(|failure| panic!("{board}: {failure:?}"));
            let data = &success.data;
            assert_eq!(
                data["verified"], "text",
                "{board}: {data}: {:?}",
                success.warning
            );
            assert_eq!(data["readFrom"], "name", "{board}: {data}");
            assert_eq!(data["retried"], false, "{board}: {data}");
            let entered = texts(&sent);
            assert_eq!(
                entered.len(),
                2,
                "{board}: emptied, then typed: {entered:?}"
            );
            assert_eq!(entered[0].0, "", "{board}: {entered:?}");
            assert_eq!(entered[0].1.is_some(), targeted, "{board}: {entered:?}");
            assert_eq!(entered[1], ("123456".to_owned(), None), "{board}");
        }
    }

    /// Design D56, N14 of the sixth production-app run (task 7.2; task 6.5):
    /// with the keyboard up and the hardware keyboard disconnected, a refill
    /// of the code the code step's field held on its second visit read back
    /// a value, "4", rather than the field's name and failed `text_mismatch`
    /// after typing and pasting, where with the hardware keyboard connected
    /// the same refill verified through the name: the field's node shows a
    /// value, a single box's placeholder or the last digit an emptying left,
    /// which the read-back took for what the field holds. A value that did
    /// not change with the entry, or that is shorter than the entry or
    /// spells nothing but placeholders, is not what the field holds: a first
    /// fill and a refill of the code verify through the name in both
    /// keyboard states, the refill emptying the field and typing the code
    /// once each, and the field holds the code.
    #[test]
    fn a_code_field_showing_a_placeholder_value_is_read_back_through_its_name_in_both_keyboard_states()
     {
        for slide in [Slide::AtOnce, Slide::Hidden] {
            let driver = FakeDriver::parse("ios")
                .unwrap()
                .with_typing(Typing::Keys)
                .with_slide(slide);
            let (mut target, sent) =
                recorded(&driver, PLACEHOLDER, &["PLACEHOLDER_SCREEN=code-valued"]);
            let first = fill_in(&mut target, "#placeholder-id-3", "123456")
                .unwrap_or_else(|failure| panic!("{slide:?}: the first fill: {failure:?}"));
            assert_eq!(first.data["readFrom"], "name", "{slide:?}: {}", first.data);
            sent.lock().unwrap().clear();
            let refill = fill_in(&mut target, "#placeholder-id-3", "123456")
                .unwrap_or_else(|failure| panic!("{slide:?}: the refill: {failure:?}"));
            let data = &refill.data;
            assert_eq!(data["verified"], "text", "{slide:?}: {data}");
            assert_eq!(data["readFrom"], "name", "{slide:?}: {data}");
            assert_eq!(data["retried"], false, "{slide:?}: {data}");
            let entered = texts(&sent);
            assert_eq!(
                entered,
                [(String::new(), None), ("123456".to_owned(), None)],
                "{slide:?}: emptied, then typed"
            );
            let listing = target.mirror.snapshot(&SnapshotOptions::full()).unwrap();
            assert_eq!(
                listing.by_identifier("placeholder-id-3").unwrap().name,
                "1, 2, 3, 4, 5, 6, Verification code, required",
                "{slide:?}: the field holds the code"
            );
        }
    }

    /// Design D56 (task 7.2): the own value of a field that is not a text
    /// field shows the entry only when it changed with it and is no
    /// placeholder: one as long as the entry at least, more than an empty
    /// box's dash or a separator, and other than the value the reading the
    /// read-back compares with held; a field's mask characters, one for
    /// each of the entry's, count, as a masked value is compared by its
    /// length.
    #[test]
    fn a_value_shows_the_entry_only_when_it_changed_with_it_and_is_no_placeholder() {
        let code = "123456";
        assert!(value_shown("123456", Some(""), code));
        assert!(value_shown("123499", None, code), "a wrong code is read");
        assert!(
            value_shown("\u{2022}".repeat(6).as_str(), Some(""), code),
            "a mask for each digit"
        );
        assert!(!value_shown("4", Some("4"), code), "the run's value");
        assert!(!value_shown("\u{2013}", Some("\u{2013}"), code));
        assert!(!value_shown("6", Some("\u{2013}"), code), "shorter");
        assert!(!value_shown("\u{2013}", None, code), "a placeholder");
        assert!(!value_shown("------", Some(""), code), "dashes alone");
        assert!(!value_shown("123456", Some("123456"), code), "unchanged");
        assert!(!value_shown("", None, code));
    }

    /// Design D49 (task 5.2): every fill typed as keys into a field that
    /// holds text empties it first, as task 4.5 did for keys whose first
    /// letter is a capital: the empty entry selects what the field holds and
    /// deletes it, and the keys go out once a read shows the field empty, a
    /// change that shows a latency after its cause, into the field the
    /// emptying gave the focus. A field that holds nothing gets its text
    /// once, and so does one a paste fills, as a text too long to type is.
    #[test]
    fn a_keyed_fill_into_a_field_holding_text_empties_it_and_types_once_it_reads_empty() {
        let latency = Duration::from_millis(60);
        let driver = FakeDriver::parse("ios")
            .unwrap()
            .with_typing(Typing::Keys)
            .with_latency(latency);
        let (mut target, sent) = recorded(&driver, FIXTURE, &[]);
        let first = fill_in(&mut target, "#login-username", "agent").unwrap();
        assert_eq!(first.data["verified"], "text", "{}", first.data);
        let entered = texts(&sent);
        assert_eq!(
            entered.len(),
            1,
            "an empty field gets its text once: {entered:?}"
        );
        assert_eq!(entered[0].0, "agent");

        sent.lock().unwrap().clear();
        let second = fill_in(&mut target, "#login-username", "smith").unwrap();
        assert_eq!(second.data["verified"], "text", "{}", second.data);
        assert_eq!(second.data["readFrom"], "value", "{}", second.data);
        assert_eq!(second.data["retried"], false, "{}", second.data);
        let entered = texts(&sent);
        assert_eq!(entered.len(), 2, "emptied, then typed: {entered:?}");
        assert!(
            entered[0].0.is_empty() && entered[0].1.is_some(),
            "{entered:?}"
        );
        assert_eq!(entered[1], ("smith".to_owned(), None));
        let times = text_times(&sent);
        assert!(
            times[1].duration_since(times[0]) >= latency,
            "the keys wait for the field to read empty: {:?}",
            times[1].duration_since(times[0])
        );
        assert_eq!(held(&mut target, "#login-username"), "smith");

        sent.lock().unwrap().clear();
        let long = "a".repeat(40);
        let pasted = fill_in(&mut target, "#login-username", &long).unwrap();
        assert_eq!(pasted.data["method"], "direct", "{}", pasted.data);
        assert_eq!(texts(&sent).len(), 1, "a paste keeps its path");
    }

    /// Design D39, N1 of the production-app run (task 3.3; section 4 of the
    /// masked listings): on the code step's second visit the code field's
    /// identifier named a labelled node holding a field without an area,
    /// which got no ref, and `fill` and `type` by the identifier failed
    /// with `invalid_argument`, while a click on it and a `type` worked. An
    /// identifier that names a node taking no text and holding exactly one
    /// field targets that field, which the reply names; a node holding no
    /// field, or two, still fails with `invalid_argument`.
    #[test]
    fn fill_and_type_by_the_identifier_of_a_fields_wrapper_go_into_the_field() {
        let (mut target, _) = typed_board("code-again", Typing::Keys);
        let success = fill_in(&mut target, "#placeholder-id-3", "123456").unwrap();
        let data = &success.data;
        assert_eq!(data["verified"], "text", "{data}: {:?}", success.warning);
        assert_eq!(data["readFrom"], "value", "{data}");
        let warning = success.warning.unwrap_or_default();
        assert!(
            warning.contains("#placeholder-id-3") && warning.contains("textbox"),
            "the reply names the field: {warning}"
        );

        let (mut typing, _) = typed_board("code-again", Typing::Keys);
        let type_in = json!({"action": "type", "target": "#placeholder-id-3", "text": "42"});
        let typed =
            super::super::run_on(&mut typing, "default", &request(&type_in.to_string())).unwrap();
        assert!(typed.warning.unwrap_or_default().contains("textbox"));
        assert!(
            text_of(&mut typing, "#placeholder-id-3")
                .as_str()
                .is_some_and(|name| name.ends_with("4 2")),
            "the field took the keys"
        );

        for wrapper in ["#placeholder-id-4", "#placeholder-id-5"] {
            let failure = fill_in(&mut target, wrapper, "1").unwrap_err();
            assert_eq!(
                failure.code, "invalid_argument",
                "{wrapper}: {}",
                failure.message
            );
        }
    }

    /// The texts among `sent`, with the node each was aimed at.
    fn texts(sent: &Sent) -> Vec<(String, Option<mirror::backend::NodeHandle>)> {
        sent.lock()
            .unwrap()
            .iter()
            .filter_map(|(_, input)| match input {
                Input::Text { text, node, .. } => Some((text.clone(), *node)),
                _ => None,
            })
            .collect()
    }

    /// Design D44, N1 of the third production-app run (task 4.2; task 3.7): on
    /// the code step's second visit the code field is a node with its
    /// identifier and nothing listed below it, the app's input not in the
    /// tree, so no field lies inside it for D39's rule and it gets no ref,
    /// and `fill` and `type` by the identifier failed with
    /// `invalid_argument`, while a click on it raised the keyboard and a
    /// `type` without a target landed the code. An identifier on such a node
    /// taps it, and once the keyboard is up the text goes to the input in
    /// focus, untargeted: the fill reads the code back through the node's
    /// name, and the reply names the node and says the keyboard carried the
    /// text.
    #[test]
    fn fill_and_type_by_the_identifier_of_a_node_that_raises_the_keyboard_go_to_the_input_it_focuses()
     {
        let (mut target, sent) = typed_board("code-bare", Typing::Keys);
        let reply = fill_in(&mut target, "#placeholder-id-3", "123456");
        let success = reply.unwrap_or_else(|failure| panic!("{failure:?}"));
        let data = &success.data;
        assert_eq!(data["verified"], "text", "{data}: {:?}", success.warning);
        assert_eq!(data["readFrom"], "name", "{data}");
        let warning = success.warning.clone().unwrap_or_default();
        assert!(
            warning.contains("#placeholder-id-3") && warning.contains("keyboard"),
            "the reply names the node and the keyboard: {warning}"
        );
        assert_eq!(taps(&sent).len(), 1, "{:?}", taps(&sent));
        assert_eq!(
            texts(&sent),
            [("123456".to_owned(), None)],
            "the code went in once, to the input in focus"
        );
        assert_eq!(
            text_of(&mut target, "#placeholder-id-3"),
            "1, 2, 3, 4, 5, 6, Verification code, required"
        );

        let (mut typing, sent) = typed_board("code-bare", Typing::Keys);
        let type_in = json!({"action": "type", "target": "#placeholder-id-3", "text": "42"});
        let typed = super::super::run_on(&mut typing, "default", &request(&type_in.to_string()))
            .unwrap_or_else(|failure| panic!("{failure:?}"));
        assert!(
            typed.warning.unwrap_or_default().contains("keyboard"),
            "{}",
            typed.data
        );
        assert_eq!(texts(&sent), [("42".to_owned(), None)]);
        assert!(
            text_of(&mut typing, "#placeholder-id-3")
                .as_str()
                .is_some_and(|name| name.starts_with("4, 2, \u{2013}")),
            "the input took the keys"
        );
    }

    /// Design D52 and D44, N11's knock-on on N1 (task 6.1; task 5.6): with
    /// a hardware keyboard connected, a tap on the code step's node holding
    /// no field gives the app's input the focus and brings up the
    /// keyboard's window drawing nothing, its keys below the screen, and
    /// `fill` and `type` by its identifier failed with `invalid_argument`,
    /// "no keyboard came up within 1000 ms of a tap on it". The window that
    /// came up tells that an input took the focus: the code goes in at once
    /// and reads back through the node's name.
    #[test]
    fn fill_by_the_identifier_of_a_node_that_brings_up_a_keyboard_drawing_nothing_types_at_once() {
        let driver = FakeDriver::parse("ios")
            .unwrap()
            .with_typing(Typing::Keys)
            .with_slide(Slide::Hidden);
        let (mut target, sent) = recorded(&driver, PLACEHOLDER, &["PLACEHOLDER_SCREEN=code-bare"]);
        let reply = fill_in(&mut target, "#placeholder-id-3", "123456");
        let success = reply.unwrap_or_else(|failure| panic!("{failure:?}"));
        let data = &success.data;
        assert_eq!(data["verified"], "text", "{data}: {:?}", success.warning);
        assert_eq!(data["readFrom"], "name", "{data}");
        assert_eq!(texts(&sent), [("123456".to_owned(), None)]);
        let waited = text_after_tap(&sent).expect("fill taps the node, then types");
        assert!(waited < Duration::from_millis(300), "{waited:?}");
    }

    /// Design D44: an identifier on a node that takes no text, holds no
    /// field and raises no keyboard when tapped still fails with
    /// `invalid_argument`, once the keyboard has had its second to come, and
    /// nothing is typed; and D39's rule comes first: an identifier on a node
    /// holding one field still fills that field.
    #[test]
    fn an_identifier_on_a_node_whose_tap_raises_no_keyboard_still_takes_no_text() {
        let (mut target, sent) = typed_board("code-again", Typing::Keys);
        let started = Instant::now();
        let failure = fill_in(&mut target, "#placeholder-id-4", "1").unwrap_err();
        let waited = started.elapsed();
        assert_eq!(failure.code, "invalid_argument", "{}", failure.message);
        assert!(waited >= KEYBOARD_WAIT, "{waited:?}");
        assert_eq!(taps(&sent).len(), 1, "{:?}", taps(&sent));
        assert!(texts(&sent).is_empty(), "{:?}", texts(&sent));

        let success = fill_in(&mut target, "#placeholder-id-3", "123456").unwrap();
        assert_eq!(success.data["readFrom"], "value", "{}", success.data);
        let warning = success.warning.unwrap_or_default();
        assert!(warning.contains("textbox"), "{warning}");
        assert!(!warning.contains("keyboard"), "{warning}");
    }

    /// Design D39, N2 of the production-app run (task 3.3; section 7 of the
    /// masked listings): on the paste sheet a fill by the ref of the title
    /// field, whose view gains a counter when the field takes the focus and
    /// whose name changes once it holds text, failed with `stale_ref`
    /// before anything was typed. The field keeps its ref across both, and
    /// the fill by it reads the field back.
    #[test]
    fn a_fill_by_ref_of_a_field_whose_label_changes_reads_it_back() {
        let (mut target, _) = typed_board("sheet", Typing::Keys);
        let field = ref_named(&mut target, "placeholder-textbox-1");
        let success = fill_in(&mut target, &field, "placeholder-value").unwrap();
        let data = &success.data;
        assert_eq!(data["verified"], "text", "{data}: {:?}", success.warning);
        assert_eq!(data["readFrom"], "value", "{data}");
        assert_eq!(data["ref"], field, "{data}");
        assert_eq!(
            held(&mut target, &field),
            "placeholder-value",
            "the field holds the text under its new name"
        );
    }

    /// Design D33: `text_mismatch` stays for a field that reads back other
    /// text after the text went in again in one piece, and names both the
    /// text entered and what the field holds, read from its value or through
    /// its boxes.
    #[test]
    fn a_fill_whose_field_reads_back_other_text_after_the_retry_fails_naming_both() {
        let driver = FakeDriver::parse("ios")
            .unwrap()
            .with_typing(Typing::Max(4));
        for (app, field, text, holds) in [
            (FIXTURE, "#login-username", "nobel", "nobe"),
            (PLACEHOLDER, "#placeholder-code", "123456", "1234"),
        ] {
            let (mut target, _) = recorded(&driver, app, &[]);
            let failure = fill_in(&mut target, field, text).unwrap_err();
            assert_eq!(failure.code, TEXT_MISMATCH, "{field}: {}", failure.message);
            assert!(
                failure.message.contains(&format!("holds \"{holds}\""))
                    && failure.message.contains(&format!("\"{text}\"")),
                "{field}: the message names both: {}",
                failure.message
            );
            assert_eq!(failure.data.unwrap_or_default()["value"], holds, "{field}");
        }
    }

    /// A 20-character text that starts with a lower-case letter, as the
    /// keys the production-app smoke test lost the first of.
    const TWENTY: &str = "placeholder-entry-20";

    /// A fake target of `platform` whose keyboard comes up as `slide` says
    /// and whose fields take text as `typing` says, with the fixture app open
    /// on its Login tab, and the inputs it is sent.
    fn sliding(platform: &str, slide: Slide, typing: Typing) -> (Attached, Sent) {
        let driver = FakeDriver::parse(platform)
            .unwrap()
            .with_slide(slide)
            .with_typing(typing);
        recorded(&driver, FIXTURE, &[])
    }

    /// How long after the first tap among `sent` the first text went out.
    fn text_after_tap(sent: &Sent) -> Option<Duration> {
        let sent = sent.lock().unwrap();
        let tap = sent
            .iter()
            .find(|(_, input)| matches!(input, Input::Tap { .. }))?
            .0;
        let text = sent
            .iter()
            .find(|(_, input)| matches!(input, Input::Text { .. }))?
            .0;
        text.checked_duration_since(tap)
    }

    /// Design D57 (task 7.3; task 6.5, N11): with the hardware keyboard
    /// connected the sixth production-app run's code fill took a second longer
    /// than with the software keyboard, 3.2 to 3.3 s against 2.3 to 2.6: a
    /// number pad brings up no keyboard window under a hardware keyboard, so
    /// the keyboard wait ran out. A session that has seen a keyboard window
    /// drawing no keys ends its keyboard waits at the field's focus, until
    /// it sees a keyboard with keys again: a fresh session's code fill waits
    /// as before, and after a fill of a text field brought up a keyboard
    /// window drawing nothing, the code fill types at once after its tap,
    /// the code read back through the field's name.
    #[test]
    fn after_a_keyboard_drawing_no_keys_the_keyboard_waits_end_at_the_focus() {
        let driver = FakeDriver::parse("ios")
            .unwrap()
            .with_typing(Typing::Keys)
            .with_slide(Slide::HiddenPad);
        let (mut target, sent) = recorded(&driver, PLACEHOLDER, &["PLACEHOLDER_SCREEN=code-named"]);
        let board = |target: &mut Attached, board: &str| {
            let env = [format!("PLACEHOLDER_SCREEN={board}")];
            let open = json!({"action": "open", "app": PLACEHOLDER, "relaunch": true, "env": env});
            super::super::run_on(target, "default", &request(&open.to_string())).unwrap();
        };
        let fresh = fill_in(&mut target, "#placeholder-id-3", "123456").unwrap();
        assert_eq!(fresh.data["readFrom"], "name", "{}", fresh.data);
        let waited = text_after_tap(&sent).expect("fill taps the field, then types");
        assert!(waited >= KEYBOARD_WAIT, "a fresh session waits: {waited:?}");

        board(&mut target, "list");
        fill_in(&mut target, "#placeholder-id-2", "agent").unwrap();
        board(&mut target, "code-named");
        sent.lock().unwrap().clear();
        let success = fill_in(&mut target, "#placeholder-id-3", "123456").unwrap();
        assert_eq!(success.data["verified"], "text", "{}", success.data);
        assert_eq!(success.data["readFrom"], "name", "{}", success.data);
        let waited = text_after_tap(&sent).expect("fill taps the field, then types");
        assert!(
            waited < Duration::from_millis(300),
            "the keys go once the tap is read: {waited:?}"
        );
    }

    /// Design D57 (task 7.3): the keyboard waits end at the focus only until
    /// a keyboard with keys is seen again, as when the Simulator's software
    /// keyboard comes back: a fill that brings up a keyboard with its keys
    /// puts the waits back as a fresh session has them.
    #[test]
    fn a_keyboard_with_keys_seen_again_puts_the_keyboard_waits_back() {
        let driver = FakeDriver::parse("ios")
            .unwrap()
            .with_typing(Typing::Keys)
            .with_slide(Slide::AtOnce);
        let (mut target, _) = recorded(&driver, FIXTURE, &[]);
        target.keys_hidden_seen = true;
        fill_in(&mut target, "#login-username", "agent").unwrap();
        assert!(!target.keys_hidden_seen);
    }

    /// Design D34: in the production-app smoke test the first keys typed into
    /// a just-focused field were lost in 3 of 8 runs while the Simulator's
    /// keyboard slid in. On iOS `fill` into a field without the focus, the
    /// keyboard down, taps the field, then types once the replica shows the
    /// keyboard's window: with a keyboard that comes up 400 ms after the
    /// focus, and a field that loses what comes sooner, the keys go out
    /// after the keyboard, within a read of it, and land whole at once.
    #[test]
    fn a_fill_on_ios_types_into_a_field_it_focused_once_the_keyboard_is_up() {
        let slide = Duration::from_millis(400);
        let (mut target, sent) = sliding("ios", Slide::After(slide), Typing::Keys);
        let success = fill_in(&mut target, "#login-username", TWENTY).unwrap();
        let data = &success.data;
        assert_eq!(data["retried"], false, "{data}: {:?}", success.warning);
        assert_eq!(data["method"], "keys", "{data}");
        assert_eq!(data["verified"], "text", "{data}");
        assert_eq!(held(&mut target, "#login-username"), TWENTY);
        let waited = text_after_tap(&sent).expect("fill taps the field, then types");
        assert!(
            waited >= slide && waited < slide + Duration::from_millis(350),
            "{waited:?}"
        );
    }

    /// Design D34: a paste is Command-V, a key the keyboard takes as it
    /// takes the others, so text longer than 32 characters, which the
    /// Simulator pastes, waits for the keyboard as typed text does.
    #[test]
    fn a_fill_on_ios_pastes_into_a_field_it_focused_once_the_keyboard_is_up() {
        let slide = Duration::from_millis(400);
        let (mut target, sent) = sliding("ios", Slide::After(slide), Typing::Keys);
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        let success = fill_in(&mut target, "#login-username", long).unwrap();
        let data = &success.data;
        assert_eq!(data["retried"], false, "{data}: {:?}", success.warning);
        assert_eq!(data["method"], "direct", "{data}");
        assert_eq!(held(&mut target, "#login-username"), long);
        let waited = text_after_tap(&sent).expect("fill taps the field, then pastes");
        assert!(waited >= slide, "{waited:?}");
    }

    /// Design D34: the wait is bounded: a field whose keyboard never comes,
    /// as a field with an input view of its own, gets its text 1 s after
    /// the tap.
    #[test]
    fn a_fill_on_ios_waits_at_most_1_s_for_a_keyboard_that_never_comes() {
        let (mut target, sent) = sliding("ios", Slide::Never, Typing::Keys);
        let success = fill_in(&mut target, "#login-username", TWENTY).unwrap();
        assert_eq!(success.data["verified"], "text", "{}", success.data);
        assert_eq!(success.data["retried"], false, "{}", success.data);
        let waited = text_after_tap(&sent).expect("fill taps the field, then types");
        assert!(
            waited >= Duration::from_secs(1) && waited < Duration::from_millis(1300),
            "{waited:?}"
        );
    }

    /// Design D52, N11 of the fifth production-app run (task 6.1): with a
    /// hardware keyboard connected iOS draws no keys, and keeps the
    /// keyboard's window, drawing nothing, while a field has the focus. Its
    /// keys never show, and `fill` waited out its second before typing,
    /// the code fill taking 3.2 to 3.3 s against 2.2 to 2.3 with the
    /// software keyboard. The focus tells instead: the keys go out once the
    /// field has it, and nothing is listed under the keyboard, so a click on
    /// a tab below the field taps it.
    #[test]
    fn a_fill_on_ios_with_a_hardware_keyboard_types_once_the_field_has_the_focus() {
        let (mut target, sent) = sliding("ios", Slide::Hidden, Typing::Keys);
        let success = fill_in(&mut target, "#login-username", TWENTY).unwrap();
        assert_eq!(success.data["verified"], "text", "{}", success.data);
        assert_eq!(success.data["retried"], false, "{}", success.data);
        assert_eq!(held(&mut target, "#login-username"), TWENTY);
        let waited = text_after_tap(&sent).expect("fill taps the field, then types");
        assert!(waited < Duration::from_millis(300), "{waited:?}");

        let listing = super::super::run_on(
            &mut target,
            "default",
            &request(r#"{"action": "snapshot", "interactive": true}"#),
        )
        .unwrap();
        let text = listing.data["snapshot"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(
            text.contains("tab-list") && !text.contains("under the keyboard"),
            "{text}"
        );
        sent.lock().unwrap().clear();
        click_on(&mut target, "#tab-list").unwrap_or_else(|failure| panic!("{failure:?}"));
        assert_eq!(taps(&sent).len(), 1, "{:?}", taps(&sent));
    }

    /// Design D34: a field that has the focus with the keyboard up gets no
    /// tap of fill's own and pays no wait.
    #[test]
    fn a_fill_on_ios_into_a_field_with_the_focus_and_the_keyboard_up_does_not_wait() {
        let slide = Duration::from_millis(600);
        let (mut target, sent) = sliding("ios", Slide::After(slide), Typing::Whole);
        fill_in(&mut target, "#login-username", "agent").unwrap();
        sent.lock().unwrap().clear();
        let started = Instant::now();
        let success = fill_in(&mut target, "#login-username", TWENTY).unwrap();
        let took = started.elapsed();
        assert!(took < Duration::from_millis(200), "{took:?}");
        assert!(taps(&sent).is_empty(), "{:?}", taps(&sent));
        assert_eq!(success.data["retried"], false, "{}", success.data);
        assert_eq!(held(&mut target, "#login-username"), TWENTY);
    }

    /// Design D34: the wait is iOS's alone: Android's text set types
    /// nothing, so `fill` there neither taps first nor waits for a keyboard,
    /// even one that comes up 600 ms after the focus.
    #[test]
    fn a_fill_on_android_neither_taps_first_nor_waits_for_the_keyboard() {
        let slide = Duration::from_millis(600);
        let (mut target, sent) = sliding("android", Slide::After(slide), Typing::Whole);
        let started = Instant::now();
        let success = fill_in(&mut target, "#login-username", TWENTY).unwrap();
        assert!(started.elapsed() < Duration::from_millis(300));
        assert!(taps(&sent).is_empty(), "{:?}", taps(&sent));
        assert_eq!(success.data["method"], "direct", "{}", success.data);
        assert_eq!(success.data["retried"], false, "{}", success.data);
    }

    /// `type` of `text` into the field `field` names, or without one into
    /// the focused field.
    fn type_into(target: &mut Attached, field: Option<&str>, text: &str) -> Outcome {
        let mut line = json!({"action": "type", "text": text});
        if let Some(field) = field {
            line["target"] = field.into();
        }
        super::super::run_on(target, "default", &request(&line.to_string()))
    }

    /// Design D3 of real-app-fixes: on the demo pass `type example.com`
    /// into Safari's address bar answered `✓ Done` alone. `type` reads back
    /// the field it typed into, the focused one when no target names it, as
    /// `fill` does, holding typed text 200 ms: the reply carries what the
    /// field holds, verified as the text added to what it held, and where
    /// the read-back took it from; a secure field reads back by its masked
    /// characters.
    #[test]
    fn type_reads_back_the_field_it_typed_into_the_focused_one_without_a_target() {
        let mut target = login(Typing::Keys);
        click_on(&mut target, "#login-username").unwrap();
        let success = type_into(&mut target, None, "example.com").unwrap();
        let data = &success.data;
        assert_eq!(data["verified"], "text", "{data}: {:?}", success.warning);
        assert_eq!(data["text"], "example.com", "{data}");
        assert_eq!(data["typed"], "example.com", "{data}");
        assert_eq!(data["readFrom"], "value", "{data}");
        assert_eq!(data["reason"], Value::Null, "{data}");
        assert!(
            data["readBackMs"]
                .as_f64()
                .is_some_and(|ms| ms >= millis(READ_BACK_HOLD)),
            "typed text is held as fill holds it: {data}"
        );
        assert_eq!(success.warning, None, "{data}");

        let more = type_into(&mut target, Some("#login-username"), "/path").unwrap();
        assert_eq!(more.data["verified"], "text", "{}", more.data);
        assert_eq!(more.data["text"], "example.com/path", "{}", more.data);
        let secret = type_into(&mut target, Some("#login-password"), "hunter2").unwrap();
        assert_eq!(secret.data["verified"], "length", "{}", secret.data);
        assert_eq!(secret.data["text"], "\u{2022}".repeat(7), "{}", secret.data);
    }

    /// Design D3: Safari's address bar took the demo pass's `type
    /// example.com` as "exampl", and Safari searched for it. A field that
    /// drops keys holds less than `type` typed: the reply says what it
    /// holds, unverified, and a warning names it and advises `fill`; the
    /// text is typed once, since `type` adds to what a field holds and
    /// typing it again would double it.
    #[test]
    fn type_into_a_field_that_drops_keys_warns_naming_what_it_holds_and_types_once() {
        let driver = FakeDriver::parse("ios")
            .unwrap()
            .with_typing(Typing::Max(6));
        let (mut target, sent) = recorded(&driver, FIXTURE, &[]);
        click_on(&mut target, "#login-username").unwrap();
        sent.lock().unwrap().clear();
        let success = type_into(&mut target, None, "example.com").unwrap();
        let data = &success.data;
        assert_eq!(data["verified"], false, "{data}");
        assert_eq!(data["reason"], "text_mismatch", "{data}");
        assert_eq!(data["text"], "exampl", "{data}");
        assert_eq!(data["readFrom"], "value", "{data}");
        let warning = success.warning.unwrap_or_default();
        assert!(
            warning.contains("\"exampl\"")
                && warning.contains("\"example.com\"")
                && warning.contains("fill"),
            "{warning}"
        );
        assert_eq!(
            texts(&sent),
            [("example.com".to_owned(), None)],
            "typed once"
        );
        assert_eq!(held(&mut target, "#login-username"), "exampl");
    }

    /// Design D3: with no field in the tree holding the focus, as on the
    /// code step whose input lies outside the tree, the keys go where the
    /// focus is and nothing tells what they did: the reply says the text
    /// was not read back, and a warning says no field shows the focus.
    #[test]
    fn type_with_no_focused_field_in_the_tree_says_it_did_not_read_its_text_back() {
        let (mut target, sent) = typed_board("code-bare", Typing::Keys);
        click_on(&mut target, "#placeholder-id-3").unwrap();
        sent.lock().unwrap().clear();
        let success = type_into(&mut target, None, "42").unwrap();
        let data = &success.data;
        assert_eq!(data["verified"], Value::Null, "{data}");
        assert_eq!(data["text"], Value::Null, "{data}");
        assert_eq!(data["reason"], "no_focused_field", "{data}");
        let warning = success.warning.unwrap_or_default();
        assert!(
            warning.contains("no field") && warning.contains("focus"),
            "{warning}"
        );
        assert_eq!(texts(&sent), [("42".to_owned(), None)]);
    }

    /// The taps among `sent`: where each went and how long it held.
    fn presses(sent: &Sent) -> Vec<(Point, Option<Duration>)> {
        sent.lock()
            .unwrap()
            .iter()
            .filter_map(|(_, input)| match input {
                Input::Tap { at, hold } => Some((*at, *hold)),
                _ => None,
            })
            .collect()
    }

    /// The settings app on the first target the fake `platform` lists, and
    /// the inputs it is sent.
    fn settings_on(platform: &str) -> (Attached, Sent) {
        let driver = FakeDriver::parse(platform).unwrap();
        let app = match platform {
            "android" => "com.android.settings",
            _ => "com.apple.Preferences",
        };
        recorded(&driver, app, &[])
    }

    /// Design D6 of real-app-fixes, spike S14: on the Simulator a plain
    /// tap, its up within 0.1 ms of its down, flipped neither Settings' Bold
    /// Text switch nor the fixture's Animate toggle, 0 of 10 each, where a
    /// 40 ms hold flipped both 10 of 10; and a tap at the centre of
    /// Settings' labelled switch, on its label, flipped it at no hold, where
    /// one on the control inside it did. A click on a switch there holds 40
    /// ms on the control inside a labelled one, whether the switch's
    /// identifier, its ref or the control's ref names it, and a hold the
    /// click names stands.
    #[test]
    fn a_click_on_a_simulator_s_switch_holds_40_ms_on_the_control_inside_it() {
        let (mut target, sent) = settings_on("ios");
        // The Wi-Fi switch spans 16 to 386 points across, from 134 to 182
        // down, its control the 63 points at its right end.
        let control = Point::new(354.5, 158.0);
        let held = Some(Duration::from_millis(40));
        click_on(&mut target, "#settings-wifi").unwrap();
        assert_eq!(presses(&sent), [(control, held)]);
        let switch = ref_named(&mut target, "Wi-Fi");
        // The control is the only element without a name.
        let inside = ref_named(&mut target, "");
        for aimed in [switch, inside] {
            sent.lock().unwrap().clear();
            click_on(&mut target, &aimed).unwrap();
            assert_eq!(presses(&sent), [(control, held)], "{aimed}");
        }
        sent.lock().unwrap().clear();
        let longer = json!({"action": "click", "target": "#settings-wifi", "hold": 300});
        super::super::run_on(&mut target, "default", &request(&longer.to_string())).unwrap();
        assert_eq!(
            presses(&sent),
            [(control, Some(Duration::from_millis(300)))]
        );
    }

    /// Design D6: the hold and the aim are the Simulator's: Android's
    /// switches flip on a tap, and a physical iPhone's were not measured, so
    /// a click on a switch there taps its centre plainly.
    #[test]
    fn a_click_on_a_switch_off_the_simulator_taps_its_centre_plainly() {
        for (platform, centre) in [
            ("android", Point::new(204.0, 120.0)),
            ("ios-device", Point::new(201.0, 158.0)),
        ] {
            let (mut target, sent) = settings_on(platform);
            click_on(&mut target, "#settings-wifi").unwrap();
            assert_eq!(presses(&sent), [(centre, None)], "{platform}");
        }
    }

    /// The listing line that starts `- <prefix>`, its leading spaces aside,
    /// in the listing `options` give.
    fn listed(target: &mut Attached, interactive: bool, prefix: &str) -> String {
        let line = json!({"action": "snapshot", "interactive": interactive}).to_string();
        let listing = super::super::run_on(target, "default", &request(&line))
            .unwrap()
            .data["snapshot"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        listing
            .lines()
            .map(str::trim_start)
            .find(|line| line.starts_with(&format!("- {prefix}")))
            .unwrap_or_else(|| panic!("no line {prefix} in:\n{listing}"))
            .to_owned()
    }

    /// Design D6 of real-app-fixes, task 3.3 on the Simulator's Settings:
    /// `#ENHANCE_TEXT_LEGIBILITY` named the table cell around Bold Text's
    /// labelled switch, which shares the identifier with the switch and the
    /// label inside it and comes first in the tree, so the click was a
    /// plain tap at the cell's centre, on the label, and flipped nothing in
    /// 10; On/Off Labels' named cell shares its identifier with its label
    /// and its control. A click on an element that holds a switch sharing
    /// its identifier, or that such a switch holds, presses that switch's
    /// control held 40 ms, whichever selector named the element, and the
    /// switch flips.
    #[test]
    fn a_click_on_an_element_holding_a_switch_that_shares_its_identifier_presses_its_control() {
        let driver = FakeDriver::parse("ios").unwrap();
        let (mut target, sent) = recorded(&driver, PLACEHOLDER, &["PLACEHOLDER_SCREEN=switches"]);
        let held = Some(Duration::from_millis(40));
        let (first, second) = (Point::new(336.5, 160.0), Point::new(330.5, 319.0));
        let on = |target: &mut Attached, name: &str| {
            listed(target, false, &format!("switch \"{name}\"")).contains("checked=true")
        };
        click_on(&mut target, "#placeholder-id-1").unwrap();
        assert_eq!(presses(&sent), [(first, held)], "the cell");
        assert!(on(&mut target, "placeholder-switch-1"));
        let label = listed(&mut target, true, "button \"placeholder-switch-1\"");
        let start = label.find("ref=e").expect("the label's ref") + 4;
        let digits: String = label[start + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        sent.lock().unwrap().clear();
        click_on(&mut target, &format!("@e{digits}")).unwrap();
        assert_eq!(presses(&sent), [(first, held)], "the label: {label}");
        assert!(!on(&mut target, "placeholder-switch-1"));
        sent.lock().unwrap().clear();
        click_on(&mut target, "#placeholder-id-2").unwrap();
        assert_eq!(presses(&sent), [(second, held)], "the named cell");
        assert!(on(&mut target, "placeholder-switch-2"));
    }

    /// Task 5.4 of real-app-fixes, on the Simulator's Settings in the second
    /// pass on real apps: `get value #ENHANCE_TEXT_LEGIBILITY` read the table
    /// cell around Bold Text's labelled switch, which the identifier names
    /// first, and answered `no value` with the switch on, while `click` by
    /// the same identifier presses the switch's control (design D6); `get
    /// text` there read the cell's empty name, and both reads of On/Off
    /// Labels' named cell its value, its switch's state as a raw 0. `get
    /// value` and `get text` on an element that holds a switch sharing its
    /// identifier, or that such a switch holds, read that switch, the one a
    /// click on the element finds: the cell by its identifier, the label
    /// inside the labelled switch by its ref, and the named cell by its
    /// identifier; a switch's state and name are no platform's, so on
    /// Android too.
    #[test]
    fn get_value_and_text_of_an_element_holding_a_switch_that_shares_its_identifier_read_it() {
        for platform in ["ios", "android"] {
            let driver = FakeDriver::parse(platform).unwrap();
            let (mut target, _) = recorded(&driver, PLACEHOLDER, &["PLACEHOLDER_SCREEN=switches"]);
            let label = listed(&mut target, true, "button \"placeholder-switch-1\"");
            let start = label.find("ref=e").expect("the label's ref") + 4;
            let digits: String = label[start + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            let label = format!("@e{digits}");
            let get = |target: &mut Attached, what: &str, aimed: &str| {
                let get = json!({"action": "get", "what": what, "target": aimed});
                super::super::run_on(target, "default", &request(&get.to_string()))
                    .unwrap()
                    .data
            };
            let rows = [
                ("#placeholder-id-1", "placeholder-switch-1"),
                (label.as_str(), "placeholder-switch-1"),
                ("#placeholder-id-2", "placeholder-switch-2"),
            ];
            for (aimed, name) in rows {
                let read = get(&mut target, "value", aimed);
                let state = (read["value"].clone(), read["checked"].clone());
                assert_eq!(state, (Value::Null, json!(false)), "{platform}: {aimed}");
                let text = get(&mut target, "text", aimed);
                assert_eq!(text["text"], name, "{platform}: {aimed}");
            }
            // Each row's control takes the tap that flips its switch.
            for (x, y) in [(336.5, 160.0), (330.5, 319.0)] {
                let tap = json!({"action": "click", "x": x, "y": y}).to_string();
                super::super::run_on(&mut target, "default", &request(&tap)).unwrap();
            }
            for (aimed, _) in rows {
                let read = get(&mut target, "value", aimed);
                let state = (read["value"].clone(), read["checked"].clone());
                assert_eq!(state, (Value::Null, json!(true)), "{platform}: {aimed}");
            }
        }
    }

    /// Task 5.5 of real-app-fixes, on the Simulator in the second pass on
    /// real apps: a New Event sheet's title field was `@e94` before `fill`
    /// typed into it and `@e114` after, with the same identifier, role and
    /// bounds and a clear button inside, the app having laid the sheet out
    /// again while the text went in, and the fill answered `✓ Done (not
    /// read back, typed)` with "left the screen", by its ref and by its
    /// identifier. The read-back finds the field again by what identified it
    /// before the entry, its identifier and role, else its bounds and role,
    /// in the replica as it is: on the placeholder app's rebuilt board,
    /// whose fields the app lays out again once they hold text, a fill by
    /// the ref of the field without an identifier and one by the other's
    /// identifier read their text back, and so does a type into the field
    /// in focus.
    #[test]
    fn fill_and_type_read_back_a_field_whose_node_the_app_replaces_as_the_text_goes_in() {
        let rebuilt = ["PLACEHOLDER_SCREEN=rebuilt"];
        let read_back = |outcome: Outcome, held: &str, platform: &str| {
            let success = outcome.unwrap_or_else(|failure| panic!("{platform}: {failure:?}"));
            let data = &success.data;
            assert_eq!(
                data["verified"], "text",
                "{platform}: {data}: {:?}",
                success.warning
            );
            assert_eq!(data["readFrom"], "value", "{platform}: {data}");
            assert_eq!(success.warning, None, "{platform}: {data}");
            if data.get("typed").is_some() {
                assert_eq!(data["text"], held, "{platform}: {data}");
            }
        };
        for platform in ["ios", "android"] {
            let driver = FakeDriver::parse(platform).unwrap();
            let (mut target, _) = recorded(&driver, PLACEHOLDER, &rebuilt);
            let plain = ref_named(&mut target, "placeholder-textbox-2");
            read_back(fill_in(&mut target, &plain, "later"), "later", platform);
            let filled = fill_in(&mut target, "#placeholder-id-1", "standup");
            read_back(filled, "standup", platform);
            assert_eq!(
                held(&mut target, "#placeholder-id-1"),
                "standup",
                "{platform}"
            );

            let driver = FakeDriver::parse(platform).unwrap();
            let (mut target, _) = recorded(&driver, PLACEHOLDER, &rebuilt);
            click_on(&mut target, "#placeholder-id-1").unwrap();
            let typed = type_into(&mut target, None, "standup");
            read_back(typed, "standup", platform);
        }
    }

    /// Task 5.6 of real-app-fixes, on the Simulator in the second pass on
    /// real apps: with a browser's address bar in focus the interactive
    /// listing counted the field under the keyboard, and a click on it, the
    /// field at (20, 510) 274x48, failed `element_offscreen`, the keyboard
    /// said to cover all of it. The reader bounds the keyboard's window by
    /// its host, which reached from 498 points down: its first 68 points the
    /// place the host keeps for the field's input accessory view, a node
    /// holding nothing, which the app draws in its own window, while the
    /// keyboard draws from 566. On the placeholder app's accessory board,
    /// its field in focus, the interactive listing lists the field and the
    /// button beside it while the button under the keys still counts under
    /// the keyboard, and a click on the field taps its centre.
    #[test]
    fn a_field_drawn_where_the_keyboard_keeps_its_accessory_view_s_place_is_listed_and_clicked() {
        let driver = FakeDriver::parse("ios").unwrap();
        let screen = ["PLACEHOLDER_SCREEN=accessory"];
        let (mut target, sent) = recorded(&driver, PLACEHOLDER, &screen);
        click_on(&mut target, "#placeholder-id-1").unwrap();
        let line = json!({"action": "snapshot", "interactive": true}).to_string();
        let listing = super::super::run_on(&mut target, "default", &request(&line))
            .unwrap()
            .data["snapshot"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        for listed in [
            "textbox \"placeholder-textbox-1\"",
            "button \"placeholder-button-2\"",
        ] {
            assert!(listing.contains(listed), "{listed} in:\n{listing}");
        }
        assert!(!listing.contains("placeholder-button-3"), "{listing}");
        assert!(
            listing.contains("(1 more actionable item under the keyboard"),
            "{listing}"
        );
        sent.lock().unwrap().clear();
        click_on(&mut target, "#placeholder-id-1").unwrap();
        assert_eq!(taps(&sent), [Point::new(157.0, 534.0)]);
    }

    /// `click` at the point `x`, `y`.
    fn click_at(target: &mut Attached, x: f64, y: f64) -> Outcome {
        let click = json!({"action": "click", "x": x, "y": y}).to_string();
        super::super::run_on(target, "default", &request(&click))
    }

    /// Design D1 of action-verdicts: a click's reply names the element its
    /// tap landed on as the replica held it at dispatch, by its role and
    /// name and by the ref the session's snapshots listed it under, null for
    /// a session whose snapshots did not, and warns of nothing when that is
    /// the element the click named.
    #[test]
    fn a_click_names_the_element_its_tap_landed_on_and_the_session_s_ref_for_it() {
        for platform in ["android", "ios"] {
            let mut target = login_on(platform, Typing::Whole, Duration::ZERO);
            let submit = ref_named(&mut target, "Sign in");
            let success = click_on(&mut target, &submit).unwrap();
            assert_eq!(
                success.data["hit"],
                json!({"role": "button", "name": "Sign in", "ref": submit}),
                "{platform}: {}",
                success.data
            );
            assert!(
                success.warning.is_none(),
                "{platform}: {:?}",
                success.warning
            );
            let click = json!({"action": "click", "target": "#login-submit"}).to_string();
            let other = super::super::run_on(&mut target, "other", &request(&click)).unwrap();
            assert_eq!(
                other.data["hit"],
                json!({"role": "button", "name": "Sign in", "ref": null}),
                "{platform}: a session whose snapshots never listed the button: {}",
                other.data
            );
            assert!(other.warning.is_none(), "{platform}: {:?}", other.warning);
        }
    }

    /// Design D1 of action-verdicts: a click by coordinates names what its
    /// tap landed on as a click on an element does: the element under the
    /// point, nothing where no element with a ref lies, and the keyboard
    /// where it draws, as the full listing names it.
    #[test]
    fn a_click_by_coordinates_names_the_element_under_the_point() {
        let mut target = login_on("android", Typing::Whole, Duration::ZERO);
        let submit = ref_named(&mut target, "Sign in");
        let on = click_at(&mut target, 200.0, 256.0).unwrap();
        assert_eq!(
            on.data["hit"],
            json!({"role": "button", "name": "Sign in", "ref": submit}),
            "{}",
            on.data
        );
        assert!(on.warning.is_none(), "{:?}", on.warning);
        let between = click_at(&mut target, 200.0, 84.0).unwrap();
        assert_eq!(between.data["hit"], Value::Null, "{}", between.data);
        click_on(&mut target, "#login-username").unwrap();
        let keys = click_at(&mut target, 200.0, 700.0).unwrap();
        assert_eq!(
            keys.data["hit"],
            json!({"role": "keyboard", "name": "", "ref": null}),
            "{}",
            keys.data
        );
    }

    /// Design D1 of action-verdicts, scenario "A tap that lands elsewhere":
    /// after the snapshot that listed a button, a banner of system UI's comes
    /// down over it in a window of its own, inset from the screen's sides, so
    /// that it cuts nothing of what a tap reaches of the button, and the
    /// click on the button's ref taps its centre, where the banner takes the
    /// tap. The reply names the banner's button as the element hit, without a
    /// ref, since no snapshot of the session listed it, and warns naming it
    /// and the button; the tap went in, as the status says. Once a snapshot
    /// listed the banner, a click by coordinates there names it by its ref.
    #[test]
    fn a_click_whose_tap_lands_on_a_view_over_its_target_names_it_and_warns() {
        for platform in ["android", "ios"] {
            let driver = FakeDriver::parse(platform).unwrap();
            let screen = ["PLACEHOLDER_SCREEN=banner"];
            let (mut target, sent) = recorded(&driver, PLACEHOLDER, &screen);
            let under = ref_named(&mut target, "placeholder-button-1");
            let call = ref_named(&mut target, "placeholder-button-2");
            let called = click_on(&mut target, &call).unwrap();
            assert_eq!(
                called.data["hit"],
                json!({"role": "button", "name": "placeholder-button-2", "ref": call}),
                "{platform}: {}",
                called.data
            );
            assert!(called.warning.is_none(), "{platform}: {:?}", called.warning);
            let landed = click_on(&mut target, &under).unwrap();
            let coordinate = |axis: &str| landed.data[axis].as_f64().unwrap_or_default();
            let point = Point::new(coordinate("x"), coordinate("y"));
            assert_eq!(
                taps(&sent).last(),
                Some(&point),
                "{platform}: the button's centre"
            );
            assert_eq!(
                landed.data["hit"],
                json!({"role": "button", "name": "placeholder-button-5", "ref": null}),
                "{platform}: {}",
                landed.data
            );
            let warning = landed.warning.clone().unwrap_or_default();
            assert!(
                warning.contains(&format!(
                    "the tap at {point} landed on button \"placeholder-button-5\""
                )) && warning
                    .contains(&format!("not on {under} (button \"placeholder-button-1\")")),
                "{platform}: {warning}"
            );
            assert_eq!(
                text_of(&mut target, "#placeholder-status"),
                "Tapped placeholder-id-5",
                "{platform}: the banner took the tap"
            );
            let banner = ref_named(&mut target, "placeholder-button-5");
            let again = click_at(&mut target, 201.0, 164.0).unwrap();
            assert_eq!(again.data["hit"]["ref"], banner.as_str(), "{platform}");
            assert!(again.warning.is_none(), "{platform}: {:?}", again.warning);
        }
    }

    /// `get value` of what `aimed` names.
    fn value_of(target: &mut Attached, aimed: &str) -> Outcome {
        let get = json!({"action": "get", "what": "value", "target": aimed}).to_string();
        super::super::run_on(target, "default", &request(&get))
    }

    /// The stale ref and the twin's ref a reply's `resolved` names.
    fn resolved(success: &Success) -> (String, String) {
        let resolved = &success.data["resolved"];
        let named = |key: &str| resolved[key].as_str().unwrap_or_default().to_owned();
        (named("stale"), named("ref"))
    }

    /// Design D3 of action-verdicts, scenario "A rebuilt field": on the
    /// second demo pass a map app's search box was stale on the call right
    /// after the snapshot `open` took, the app having laid its card out
    /// again with nothing in between, and the fill on its ref failed
    /// `stale_ref` while the replica held the box again. On the placeholder
    /// app's rebuilt board, whose fields the app lays out again once they
    /// hold text, each in a new node at the same place, a get, a click and a
    /// fill on the ref a field had before act on its twin: the field with an
    /// identifier by its identifier and role, the other by its name, role
    /// and bounds. Each reply's `resolved` names the stale ref and the
    /// twin's, which the click and the fill name as their `ref` and the
    /// click's hit as the session's; the session acts on the twin's ref from
    /// then on, and on the stale one, which stands for the field's latest
    /// node once the fill has emptied and filled it.
    #[test]
    fn a_stale_ref_acts_on_its_twin_by_identifier_and_role_else_by_name_role_and_bounds() {
        for platform in ["ios", "android"] {
            let driver = FakeDriver::parse(platform).unwrap();
            let rebuilt = ["PLACEHOLDER_SCREEN=rebuilt"];
            let (mut target, _) = recorded(&driver, PLACEHOLDER, &rebuilt);
            let first = ref_named(&mut target, "placeholder-textbox-1");
            let second = ref_named(&mut target, "placeholder-textbox-2");
            // The text puts each field in a new node, its ref left stale.
            fill_in(&mut target, "#placeholder-id-1", "standup").unwrap();
            fill_in(&mut target, &second, "later").unwrap();
            for (stale, held) in [(&first, "standup"), (&second, "later")] {
                let read = value_of(&mut target, stale)
                    .unwrap_or_else(|failure| panic!("{platform}: get {stale}: {failure:?}"));
                assert_eq!(read.data["value"], held, "{platform}: {}", read.data);
                let (named, twin) = resolved(&read);
                assert_eq!(&named, stale, "{platform}: {}", read.data);
                assert!(
                    twin.starts_with("@e") && &twin != stale,
                    "{platform}: {}",
                    read.data
                );
                let clicked = click_on(&mut target, stale)
                    .unwrap_or_else(|failure| panic!("{platform}: click {stale}: {failure:?}"));
                let data = &clicked.data;
                assert_eq!(
                    resolved(&clicked),
                    (stale.clone(), twin.clone()),
                    "{platform}: {data}"
                );
                assert_eq!(data["ref"], twin.as_str(), "{platform}: {data}");
                assert_eq!(data["hit"]["ref"], twin.as_str(), "{platform}: {data}");
                assert!(
                    clicked.warning.is_none(),
                    "{platform}: {:?}",
                    clicked.warning
                );
                let held_by_twin = value_of(&mut target, &twin)
                    .unwrap_or_else(|failure| panic!("{platform}: get {twin}: {failure:?}"));
                assert_eq!(held_by_twin.data["value"], held, "{platform}");
                assert!(
                    held_by_twin.data.get("resolved").is_none(),
                    "{platform}: {}",
                    held_by_twin.data
                );
                let filled = fill_in(&mut target, stale, "again")
                    .unwrap_or_else(|failure| panic!("{platform}: fill {stale}: {failure:?}"));
                let data = &filled.data;
                assert_eq!(
                    resolved(&filled),
                    (stale.clone(), twin.clone()),
                    "{platform}: {data}"
                );
                assert_eq!(data["ref"], twin.as_str(), "{platform}: {data}");
                assert_eq!(data["verified"], "text", "{platform}: {data}");
                // Emptied first, then filled, the field took new nodes again.
                let again = value_of(&mut target, stale)
                    .unwrap_or_else(|failure| panic!("{platform}: get {stale}: {failure:?}"));
                assert_eq!(again.data["value"], "again", "{platform}: {}", again.data);
                assert_eq!(&resolved(&again).0, stale, "{platform}: {}", again.data);
            }
        }
    }

    /// Design D3 of action-verdicts: a ref whose element is gone without a
    /// twin in the replica, no element of its role and identifier nor of its
    /// role, name and bounds, fails `stale_ref` with its advice, as the
    /// rebuilt board's fields do once the placeholder app was relaunched on
    /// its form.
    #[test]
    fn a_stale_ref_without_a_twin_fails_stale_ref() {
        for platform in ["ios", "android"] {
            let driver = FakeDriver::parse(platform).unwrap();
            let rebuilt = ["PLACEHOLDER_SCREEN=rebuilt"];
            let (mut target, _) = recorded(&driver, PLACEHOLDER, &rebuilt);
            let first = ref_named(&mut target, "placeholder-textbox-1");
            let second = ref_named(&mut target, "placeholder-textbox-2");
            let open = json!({"action": "open", "app": PLACEHOLDER, "relaunch": true}).to_string();
            super::super::run_on(&mut target, "default", &request(&open)).unwrap();
            for failed in [
                click_on(&mut target, &first),
                fill_in(&mut target, &second, "later"),
                value_of(&mut target, &first),
            ] {
                let failure = failed.unwrap_err();
                assert_eq!(failure.code, "stale_ref", "{platform}: {}", failure.message);
                assert!(
                    failure.message.contains("Take a new snapshot"),
                    "{platform}: {}",
                    failure.message
                );
            }
        }
    }

    /// Design D3 of action-verdicts, scenario "A rebuilt field", as the
    /// Simulator showed it: a fill on a map app's search field, by the ref
    /// the snapshot right after `open` gave it, failed `stale_ref`, the tap
    /// that gave the field the focus having raised its card and put the
    /// field in a new node before the text went in. On the placeholder
    /// app's raised board a fill by the field's ref, and one by its
    /// identifier, enter the text into the field where the raised card
    /// holds it and read it back. On iOS, where the fill taps the field
    /// before its text, the text goes into the field's twin, by identifier
    /// and role, whose ref the reply gives, and the fill by the ref names
    /// both refs in `resolved`; on Android the text goes in with the focus,
    /// before the field moves.
    #[test]
    fn a_fill_whose_focus_tap_puts_the_field_in_a_new_node_enters_the_text_into_its_twin() {
        let raised = ["PLACEHOLDER_SCREEN=raised"];
        for platform in ["ios", "android"] {
            for by_ref in [true, false] {
                let driver = FakeDriver::parse(platform).unwrap();
                let (mut target, _) = recorded(&driver, PLACEHOLDER, &raised);
                let field = ref_named(&mut target, "placeholder-textbox-1");
                let aimed = if by_ref {
                    field.clone()
                } else {
                    "#placeholder-id-1".to_owned()
                };
                let filled = fill_in(&mut target, &aimed, "coffee")
                    .unwrap_or_else(|failure| panic!("{platform}: fill {aimed}: {failure:?}"));
                let data = &filled.data;
                assert_eq!(data["verified"], "text", "{platform} {aimed}: {data}");
                assert_eq!(
                    held(&mut target, "#placeholder-id-1"),
                    "coffee",
                    "{platform} {aimed}"
                );
                if platform == "android" {
                    assert!(data.get("resolved").is_none(), "{platform}: {data}");
                    continue;
                }
                let twin = ref_named(&mut target, "placeholder-textbox-1");
                assert_ne!(twin, field, "{platform}: the field took a new node");
                assert_eq!(data["ref"], twin.as_str(), "{platform} {aimed}: {data}");
                match by_ref {
                    true => assert_eq!(
                        resolved(&filled),
                        (field.clone(), twin.clone()),
                        "{platform}: {data}"
                    ),
                    false => assert!(data.get("resolved").is_none(), "{platform}: {data}"),
                }
            }
        }
    }
}
