//! Actions: taps, text, keys and gestures, injected through the mirror so
//! it knows to re-read windows that send no change events.

use std::time::{Duration, Instant};

use mirror::backend::Release;
use mirror::{
    Action, DEFAULT_QUIET_WINDOW, MirrorError, Point, Rect, Ref, SettleOptions, Target, Timestamps,
    Visibility,
};
use protocol::message::TextMethod;
use serde_json::{Map, Value, json};

use super::aim::{self, Aim};
use super::view::element_json;
use super::{millis, timeout};
use crate::driver::Platform;
use crate::keys;
use crate::wire::code::{self, INPUT_REJECTED, STROKE_TOO_SHORT, TEXT_MISMATCH};
use crate::wire::{Failure, Request, Success};
use crate::worker::{Attached, Outcome};

/// How long a swipe takes when the command names no duration.
const SWIPE_DURATION: Duration = Duration::from_millis(300);

/// How far `swipe` and `scroll` move when the command names no distance,
/// in points, as agent-browser's default of 300 pixels.
const DEFAULT_DISTANCE: f64 = 300.0;

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
    /// Why the watch stopped early, if it failed.
    problem: Option<String>,
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
            problem = Some(error.to_string());
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

pub(super) fn click(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let hold = request.u64("hold")?.map(Duration::from_millis);
    let (aimed, reference, clicked, found) = match (request.f64("x")?, request.f64("y")?) {
        (Some(x), Some(y)) => (
            Target::Point(Point::new(x, y)),
            None,
            format!("{x} {y}"),
            None,
        ),
        _ => {
            let given = request.required_str("target")?;
            let aim = Aim::parse(given)?;
            let (aimed, reference, found) =
                aim::action_target(target, session, &aim, timeout(request)?)?;
            (aimed, reference, given.to_owned(), found)
        }
    };
    // The replica first takes in what arrived before the tap, so nothing
    // the tap did not cause is counted as its effect.
    target.mirror.pump()?;
    // How much of the element is in view where the tap goes in; a ref the
    // mirror cannot resolve fails the tap below.
    let seen = found
        .or_else(|| {
            reference
                .and_then(|reference| target.mirror.resolve(reference).ok())
                .map(|element| element.id)
        })
        .and_then(|element| target.mirror.visibility(element));
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
    let fresh = target
        .input_notes
        .as_ref()
        .and_then(|notes| notes.activation_wait())
        .is_some();
    let spans = watch(target, started, revision, dispatch, fresh);
    let point = outcome.at.unwrap_or_default();
    let mut data = Map::new();
    data.insert("clicked".into(), clicked.as_str().into());
    data.insert("ref".into(), reference_json(reference));
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
    let mut warnings = Vec::new();
    if let Some(seen) = seen {
        // A cut edge names the axis; otherwise lists scroll up and down.
        let vertical = !cut_along(&seen, false);
        if !mostly_in_view(&seen, vertical) {
            warnings.push(format!(
                "{clicked} was not mostly in view ({}), so the tap at {point} may have missed it; scroll --until {clicked} brings it into view first",
                in_view(&seen)
            ));
        }
    }
    if let Some(problem) = spans.problem {
        warnings.push(format!(
            "the tap went in, but watching its effects failed: {problem}"
        ));
    }
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    Ok(Success::new(Value::Object(data)).warn(warning))
}

fn text_param(request: &Request) -> Result<String, Failure> {
    request.str("text").map(str::to_owned).ok_or_else(|| {
        Failure::new(
            code::INVALID_ARGUMENT,
            format!("{} needs a \"text\" string", request.action),
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
/// Command-V (task 10.23), and the mirror reads a field that sends no
/// change events again no sooner than its minimum interval after its last
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

/// What a read-back found in the field.
#[derive(Debug, PartialEq)]
enum ReadBack {
    /// It shows the text, which the read-back took `after` the entry.
    Shown { shown: Shown, after: Duration },
    /// It showed other text, `None` for none, until the read-back's bound.
    Other(Option<String>),
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

/// Reads back the field `reference` names after an entry by `method` that
/// ended at `entered`: until the field shows `text` and has held it since
/// the hold after that method ends ([`hold_after`]), which after text that
/// went in in one piece takes the first read that shows it, or until it
/// has shown other text up to [`READ_BACK_MAX`] after the entry, the mirror
/// reading it again as reads fall due meanwhile. What the field shows at
/// the bound stands.
fn read_back(
    target: &mut Attached,
    reference: Ref,
    text: &str,
    entered: Instant,
    method: Option<TextMethod>,
) -> Result<ReadBack, Failure> {
    let hold = entered + hold_after(method);
    let deadline = entered + READ_BACK_MAX;
    loop {
        let element = match target.mirror.resolve(reference) {
            Ok(element) => element,
            Err(MirrorError::StaleRef { .. }) => return Ok(ReadBack::Gone),
            Err(error) => return Err(error.into()),
        };
        let now = Instant::now();
        let rereads = target.mirror.next_wakeup().is_some();
        let stamps = target.mirror.timestamps();
        match compare(element.value.as_deref(), &element.name, text) {
            Some(shown) if now >= deadline || held(&stamps, hold, now, rereads) => {
                return Ok(ReadBack::Shown {
                    shown,
                    after: now.saturating_duration_since(entered),
                });
            }
            None if now >= deadline => return Ok(ReadBack::Other(element.value)),
            _ => {}
        }
        let until = if now < hold { hold } else { deadline };
        target
            .mirror
            .run_until((now + READ_BACK_SLICE).min(until))?;
    }
}

/// Enters `text` into the field `reference` names as `method` asks; says
/// the method the target used.
fn enter(
    target: &mut Attached,
    reference: Ref,
    text: &str,
    method: TextMethod,
) -> Result<Option<TextMethod>, Failure> {
    let outcome = target.mirror.act(&Action::Text {
        text: text.to_owned(),
        into: Some(reference),
        method,
    })?;
    if !outcome.accepted {
        return Err(rejected("text; fill takes a text field"));
    }
    Ok(outcome.method)
}

/// How `fill` entered its text by `method` on `platform`, as messages say.
fn entered_by(platform: Platform, method: Option<TextMethod>) -> &'static str {
    if method == Some(TextMethod::KEYS) {
        return "typed the text";
    }
    match platform {
        Platform::Ios => "pasted the text",
        Platform::Android => "set the text directly",
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

/// A fill's reply: the field as `given`, its ref, the method the text the
/// field holds went in by, how the read-back found it and how long after
/// the entry it took it (`None` when the field was not read back), and
/// whether `fill` entered the text again.
fn filled(
    given: &str,
    reference: Ref,
    method: Option<TextMethod>,
    read: Option<(Shown, Duration)>,
    retried: bool,
) -> Success {
    Success::new(json!({
        "filled": given,
        "ref": format!("@{reference}"),
        "method": method_name(method),
        "verified": read.map(|(shown, _)| shown.name()),
        "readBackMs": read.map(|(_, after)| millis(after)),
        "retried": retried,
    }))
}

/// The failure of a fill whose field `given` holds `value` after `fill`
/// entered the text as `tries` say.
fn mismatch(given: &str, reference: Ref, value: Option<&str>, tries: &str) -> Failure {
    let masked =
        value.is_some_and(|value| !value.is_empty() && value.chars().all(|c| MASKS.contains(&c)));
    Failure::new(
        TEXT_MISMATCH,
        format!(
            "{given} holds {} after fill {tries}; a field that limits or formats its text shows it so: take a snapshot to see the field, then fill it again or go on if it holds what you meant",
            described(value)
        ),
    )
    .with_data(json!({"ref": format!("@{reference}"), "value": value, "masked": masked}))
}

/// Fills the field the request names: enters the text, typed or pasted as
/// the target chooses, and reads the field back, holding typed text
/// [`READ_BACK_HOLD`] and taking text that went in in one piece from the
/// first read that shows it. A field that shows other text gets the text
/// again in one piece, pasted on a Simulator and set directly on Android,
/// and is read back again; a field that still shows other text fails the
/// fill with `text_mismatch`. On Android an empty field reads as its hint,
/// so an empty fill there is not read back.
pub(super) fn fill(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let text = text_param(request)?;
    let given = request.required_str("target")?;
    let aim = Aim::parse(given)?;
    let reference = aim::text_ref(target, session, &aim, timeout(request)?)?;
    let platform = target.info.platform;
    let first = enter(target, reference, &text, TextMethod::AUTO)?;
    let entered = Instant::now();
    if text.is_empty() && platform == Platform::Android {
        return Ok(filled(given, reference, first, None, false));
    }
    let gone = |by: &str| {
        Some(format!(
            "{given} left the screen after fill {by}, so it could not be read back"
        ))
    };
    let value = match read_back(target, reference, &text, entered, first)? {
        ReadBack::Shown { shown, after } => {
            return Ok(filled(given, reference, first, Some((shown, after)), false));
        }
        ReadBack::Gone => {
            return Ok(filled(given, reference, first, None, false)
                .warn(gone(entered_by(platform, first))));
        }
        ReadBack::Other(value) => value,
    };
    let first_by = entered_by(platform, first);
    let second = match enter(target, reference, &text, TextMethod::DIRECT) {
        Ok(second) => second,
        Err(failure) => {
            let tries = format!(
                "{first_by}, and entering it again in one piece failed: {}",
                failure.message
            );
            return Err(mismatch(given, reference, value.as_deref(), &tries));
        }
    };
    let entered = Instant::now();
    let second_by = entered_by(platform, second);
    let again = || {
        let it = second_by.replacen("the text", "it", 1);
        format!(
            "{given} showed {} after fill {first_by}, so fill {it} instead",
            described(value.as_deref())
        )
    };
    match read_back(target, reference, &text, entered, second)? {
        ReadBack::Shown { shown, after } => {
            Ok(filled(given, reference, second, Some((shown, after)), true).warn(Some(again())))
        }
        ReadBack::Gone => Ok(
            filled(given, reference, second, None, true).warn(Some(format!(
                "{}; {}",
                again(),
                gone(second_by).unwrap_or_default()
            ))),
        ),
        ReadBack::Other(still) => {
            let tries = if first_by == second_by {
                format!("{first_by} twice")
            } else {
                format!(
                    "{first_by}, then {}",
                    second_by.replacen("the text", "it", 1)
                )
            };
            Err(mismatch(given, reference, still.as_deref(), &tries))
        }
    }
}

pub(super) fn type_text(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let text = text_param(request)?;
    let into = match request.str("target") {
        Some(given) => {
            let aim = Aim::parse(given)?;
            Some(aim::text_ref(target, session, &aim, timeout(request)?)?)
        }
        None => None,
    };
    let outcome = target.mirror.act(&Action::Text {
        text: text.clone(),
        into,
        method: TextMethod::KEYS,
    })?;
    if !outcome.accepted {
        return Err(rejected(
            "text; type takes a text field or the focused field",
        ));
    }
    Ok(Success::new(json!({
        "typed": text,
        "ref": reference_json(into),
        "method": method_name(outcome.method),
    })))
}

pub(super) fn press(target: &mut Attached, _session: &str, request: &Request) -> Outcome {
    let key = request.required_str("key")?;
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

fn point_param(request: &Request, key: &str) -> Result<Option<Point>, Failure> {
    match request.params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(pair)) if pair.len() == 2 => match (pair[0].as_f64(), pair[1].as_f64()) {
            (Some(x), Some(y)) => Ok(Some(Point::new(x, y))),
            _ => Err(Failure::new(
                code::INVALID_ARGUMENT,
                format!("\"{key}\" must be [x, y] in points"),
            )),
        },
        Some(_) => Err(Failure::new(
            code::INVALID_ARGUMENT,
            format!("\"{key}\" must be [x, y] in points"),
        )),
    }
}

pub(super) fn swipe(target: &mut Attached, _session: &str, request: &Request) -> Outcome {
    let duration = request
        .u64("duration")?
        .map_or(SWIPE_DURATION, Duration::from_millis);
    let (from, to, swiped) = match (point_param(request, "from")?, point_param(request, "to")?) {
        (Some(from), Some(to)) => (from, to, Value::Bool(true)),
        _ => {
            let direction = Direction::parse(request.str("direction").unwrap_or("up"))?;
            let distance = request.f64("distance")?.unwrap_or(DEFAULT_DISTANCE);
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

pub(super) fn scroll(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let direction = Direction::parse(request.str("direction").unwrap_or("down"))?;
    let amount = request.f64("amount")?.unwrap_or(DEFAULT_DISTANCE);
    let area = match request.str("target") {
        Some(given) => {
            let aim = Aim::parse(given)?;
            aim::element(target, session, &aim, timeout(request)?)?.bounds
        }
        None => screen(target)?,
    };
    let extent = if direction.vertical() {
        area.height()
    } else {
        area.width()
    };
    // The content moves in the direction, so the finger moves the other way;
    // a long scroll takes several strokes, none of them short enough to be a
    // tap.
    let longest = (extent * MAX_STROKE).max(1.0);
    let platform = target.info.platform;
    if let Some(until) = request.str("until") {
        let step = amount.max(1.0).min(longest);
        check_stroke(platform, step, amount, longest)?;
        return scroll_until(target, session, request, (area, direction, step), until);
    }
    let (count, step) = split(amount.max(0.0), longest);
    check_stroke(platform, step, amount, longest)?;
    let mut strokes = 0u32;
    while strokes < count {
        let (from, to) = stroke(area, direction.opposite(), step);
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
    Ok(Success::new(json!({
        "scrolled": true,
        "direction": direction.name(),
        "amount": amount,
        "strokes": strokes,
    })))
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
    let extent = |rect: &Rect| {
        let (start, end) = along(rect, vertical);
        end - start
    };
    let shown = viewport
        .map(|viewport| extent(&viewport))
        .filter(|&shown| shown > 0.0)
        .unwrap_or_else(|| extent(&area));
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

/// Scrolls `direction` inside `area` in strokes of at most `step` points
/// until the element `until` names is on screen, not covered and mostly in
/// view (see [`mostly_in_view`]), letting the screen settle after each
/// stroke so a check never races a moving list. Every stroke comes to rest
/// before the finger lifts, so the content goes no farther than the finger,
/// and a stroke that looks for the element carries the content at most
/// [`REACH`] of what the scroll container it moves shows (see
/// [`search_stroke`]), so the element cannot pass through the container
/// between two looks. Once the element is on screen, each further stroke
/// brings it in rather than going on by the step (see [`decide`]). Fails
/// with `element_not_found` once the content stops moving or no stroke
/// along the axis brings more of the element in, after [`UNTIL_STROKES`]
/// strokes, or at the command's timeout; an element on screen then gives
/// its share in view as `data.visible`.
fn scroll_until(
    target: &mut Attached,
    session: &str,
    request: &Request,
    (area, direction, step): (Rect, Direction, f64),
    until: &str,
) -> Outcome {
    let aim = Aim::parse(until)?;
    let deadline = Instant::now() + timeout(request)?;
    let vertical = direction.vertical();
    let slop = tap_slop(target.info.platform);
    let mut strokes = 0u32;
    let mut still = 0u32;
    loop {
        target.catch_up()?;
        let found = aim::on_screen(target, session, &aim)?;
        let seen = found
            .as_ref()
            .and_then(|element| target.mirror.visibility(element.id));
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
            return Ok(Success::new(json!({
                "scrolled": strokes > 0,
                "direction": direction.name(),
                "amount": reach,
                "strokes": strokes,
                "until": until,
                "visible": share(seen.fraction),
                "element": element_json(element),
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
                timeout(request)?.as_millis()
            ))
        } else {
            None
        };
        if let Some(reason) = reason {
            let (message, data) = match seen {
                Some(seen) => (
                    format!(
                        "{until} was on screen but not mostly in view ({}) after {strokes} strokes of {} points {}: {reason}; take a snapshot to see where the content is",
                        in_view(&seen),
                        points(reach),
                        direction.name()
                    ),
                    json!({"strokes": strokes, "until": until, "visible": share(seen.fraction)}),
                ),
                None => (
                    format!(
                        "{until} was not on screen after {strokes} strokes of {} points {}: {reason}; take a snapshot to see where the content is",
                        points(reach),
                        direction.name()
                    ),
                    json!({"strokes": strokes, "until": until}),
                ),
            };
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
        target.mirror.settle(&SettleOptions {
            max: UNTIL_SETTLE.min(deadline.saturating_duration_since(Instant::now())),
            ..SettleOptions::default()
        })?;
        target.catch_up()?;
        // The replica shows whether the stroke moved the content once the
        // reads it called for have run, such as the full read after a
        // window event.
        target.mirror.pump()?;
        if target.mirror.revision() == revision {
            still += 1;
        } else {
            still = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::driver::{AttachOptions, Driver, DriverError, Screen, Size};
    use crate::fake::{FakeDriver, Typing};

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
        super::super::open(&mut target, "default", &request(&open)).unwrap();
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
        super::fill(target, "default", &request(&fill))
    }

    /// What the field `field` names holds, as `get value` reads it.
    fn held(target: &mut Attached, field: &str) -> Value {
        let get = json!({"action": "get", "what": "value", "target": field}).to_string();
        super::super::view::get(target, "default", &request(&get))
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
        let warning = success.warning.unwrap_or_default();
        assert!(
            warning.contains("\"noelb\"") && warning.contains("pasted"),
            "the warning names what the keys left and the paste: {warning}"
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
        Request::parse(line).unwrap()
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
        let listing = super::super::view::snapshot(
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
        let data = super::click(&mut warm, "default", &click).unwrap().data;
        assert!(
            data.get("frameChangeMs").is_none(),
            "a warm watch ends 200 ms after a tap that changed nothing yet: {data}"
        );

        let (mut fresh, fixture) = slow_ios(Some(Arc::new(Fresh)));
        let click = request(&format!(r#"{{"action": "click", "target": "{fixture}"}}"#));
        let data = super::click(&mut fresh, "default", &click).unwrap().data;
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
        let upright = super::swipe(&mut target, "default", &swipe).unwrap().data;
        assert_eq!(upright["from"][0], 200.0, "{upright}");
        turned.store(true, Ordering::SeqCst);
        let rotated = super::swipe(&mut target, "default", &swipe).unwrap().data;
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
}
