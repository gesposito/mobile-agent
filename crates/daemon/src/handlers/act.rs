//! Actions: taps, text, keys and gestures, injected through the mirror so
//! it knows to re-read windows that send no change events.

use std::time::{Duration, Instant};

use mirror::{Action, DEFAULT_QUIET_WINDOW, Point, Rect, SettleOptions, Target};
use protocol::message::TextMethod;
use serde_json::{Map, Value, json};

use super::aim::{self, Aim};
use super::view::element_json;
use super::{millis, timeout};
use crate::keys;
use crate::wire::code::{self, INPUT_REJECTED};
use crate::wire::{Failure, Request, Success};
use crate::worker::{Attached, Outcome};

/// How long a swipe takes when the command names no duration.
const SWIPE_DURATION: Duration = Duration::from_millis(300);

/// How far `swipe` and `scroll` move when the command names no distance,
/// in points, as agent-browser's default of 300 pixels.
const DEFAULT_DISTANCE: f64 = 300.0;

/// The longest stroke of a scroll, as a share of the scrolled area.
const MAX_STROKE: f64 = 0.8;

/// How long a tap's effects are watched for at most.
const WATCH_MAX: Duration = Duration::from_secs(1);

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
/// or for [`WATCH_MAX`]. The mirror keeps reading meanwhile, so a window
/// without change events is re-read as it would be for a snapshot.
fn watch(target: &mut Attached, start: Instant, revision: u64, dispatch: Duration) -> Spans {
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
        let frame = target.observer.first_frame_change_since(start);
        let tree = tree_delta(target);
        if tree.is_some() && (frame.is_some() || !target.frames) {
            break (frame, tree);
        }
        let now = Instant::now();
        let since_start = |at: Option<Instant>| at.filter(|&at| at >= start);
        let last_change = [
            since_start(target.observer.last_frame_change()),
            since_start(target.observer.last_tree_event()),
            target
                .mirror
                .deltas()
                .last()
                .filter(|d| d.revision > revision)
                .map(|d| d.at),
        ]
        .into_iter()
        .flatten()
        .max()
        .map_or(start, |at| at.max(start));
        let deadline = start + WATCH_MAX;
        if now >= last_change + DEFAULT_QUIET_WINDOW || now >= deadline {
            break (frame, tree);
        }
        if let Err(error) = target.mirror.run_until((now + WATCH_SLICE).min(deadline)) {
            problem = Some(error.to_string());
            break (
                target.observer.first_frame_change_since(start),
                tree_delta(target),
            );
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
    let (aimed, reference, clicked) = match (request.f64("x")?, request.f64("y")?) {
        (Some(x), Some(y)) => (Target::Point(Point::new(x, y)), None, format!("{x} {y}")),
        _ => {
            let given = request.required_str("target")?;
            let aim = Aim::parse(given)?;
            let (aimed, reference) = aim::action_target(target, session, &aim, timeout(request)?)?;
            (aimed, reference, given.to_owned())
        }
    };
    // The replica first takes in what arrived before the tap, so nothing
    // the tap did not cause is counted as its effect.
    target.mirror.pump()?;
    let revision = target.mirror.revision();
    let started = Instant::now();
    let outcome = target.act(&Action::Tap {
        target: aimed,
        hold,
    })?;
    let dispatch = started.elapsed();
    if !outcome.accepted {
        return Err(rejected("tap"));
    }
    let spans = watch(target, started, revision, dispatch);
    let point = outcome.at.unwrap_or_default();
    let mut data = Map::new();
    data.insert("clicked".into(), clicked.into());
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
    let warning = spans
        .problem
        .map(|problem| format!("the tap went in, but watching its effects failed: {problem}"));
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

pub(super) fn fill(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let text = text_param(request)?;
    let given = request.required_str("target")?;
    let aim = Aim::parse(given)?;
    let reference = aim::text_ref(target, session, &aim, timeout(request)?)?;
    let outcome = target.act(&Action::Text {
        text,
        into: Some(reference),
        method: TextMethod::AUTO,
    })?;
    if !outcome.accepted {
        return Err(rejected("text; fill takes a text field"));
    }
    Ok(Success::new(json!({
        "filled": given,
        "ref": format!("@{reference}"),
        "method": method_name(outcome.method),
    })))
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
    let outcome = target.act(&Action::Text {
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
    let outcome = target.act(&Action::Key {
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
    let outcome = target.act(&Action::Swipe { from, to, duration })?;
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
    // a long scroll takes several strokes.
    let longest = (extent * MAX_STROKE).max(1.0);
    if let Some(until) = request.str("until") {
        let step = amount.max(1.0).min(longest);
        return scroll_until(target, session, request, (area, direction, step), until);
    }
    let mut left = amount.max(0.0);
    let mut strokes = 0u32;
    while left > 0.5 && strokes < 20 {
        let step = left.min(longest);
        let (from, to) = stroke(area, direction.opposite(), step);
        let outcome = target.act(&Action::Swipe {
            from,
            to,
            duration: SWIPE_DURATION,
        })?;
        if !outcome.accepted {
            return Err(rejected("scroll"));
        }
        left -= step;
        strokes += 1;
    }
    Ok(Success::new(json!({
        "scrolled": true,
        "direction": direction.name(),
        "amount": amount,
        "strokes": strokes,
    })))
}

/// Scrolls `direction` inside `area` in strokes of `step` points until the
/// element `until` names is on screen, not covered and not scrolled away,
/// letting the screen settle after each stroke so a check never races a
/// moving list. Fails with `element_not_found` once the content stops
/// moving, after [`UNTIL_STROKES`] strokes, or at the command's timeout.
fn scroll_until(
    target: &mut Attached,
    session: &str,
    request: &Request,
    (area, direction, step): (Rect, Direction, f64),
    until: &str,
) -> Outcome {
    let aim = Aim::parse(until)?;
    let deadline = Instant::now() + timeout(request)?;
    let mut strokes = 0u32;
    let mut still = 0u32;
    loop {
        target.catch_up()?;
        if let Some(element) = aim::on_screen(target, session, &aim)? {
            if let Some(reference) = element.reference {
                target.session(session).seen.insert(reference);
            }
            return Ok(Success::new(json!({
                "scrolled": strokes > 0,
                "direction": direction.name(),
                "amount": step,
                "strokes": strokes,
                "until": until,
                "element": element_json(&element),
            })));
        }
        let now = Instant::now();
        let reason = if still >= STILL_STROKES {
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
            return Err(Failure::new(
                code::ELEMENT_NOT_FOUND,
                format!(
                    "{until} was not on screen after {strokes} strokes of {step} points {}: {reason}; take a snapshot to see where the content is",
                    direction.name()
                ),
            )
            .with_data(json!({"strokes": strokes, "until": until})));
        }
        let revision = target.mirror.revision();
        let (from, to) = stroke(area, direction.opposite(), step);
        let outcome = target.act(&Action::Swipe {
            from,
            to,
            duration: SWIPE_DURATION,
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
    use crate::fake::FakeDriver;

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
}
