//! Actions: taps, text, keys and gestures, injected through the mirror so
//! it knows to re-read windows that send no change events.

use std::time::{Duration, Instant};

use mirror::{Action, Point, Rect, Target};
use protocol::message::TextMethod;
use serde_json::{Value, json};

use super::aim::{self, Aim};
use super::{millis, timeout};
use crate::keys;
use crate::wire::{Failure, Request, Success, code};
use crate::worker::{Attached, Outcome};

/// The target refused the input.
pub(crate) const INPUT_REJECTED: &str = "input_rejected";

/// How long a swipe takes when the command names no duration.
const SWIPE_DURATION: Duration = Duration::from_millis(300);

/// How far `swipe` and `scroll` move when the command names no distance,
/// in points, as agent-browser's default of 300 pixels.
const DEFAULT_DISTANCE: f64 = 300.0;

/// The longest stroke of a scroll, as a share of the scrolled area.
const MAX_STROKE: f64 = 0.8;

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
    Ok(Success::new(json!({
        "clicked": clicked,
        "ref": reference_json(reference),
        "x": point.x,
        "y": point.y,
        "accepted": outcome.accepted,
        "dispatchMs": millis(dispatch),
    })))
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
    let outcome = target.mirror.act(&Action::Text {
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

fn screen(target: &Attached) -> Rect {
    Rect::new(0.0, 0.0, target.display.width, target.display.height)
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
            let (from, to) = stroke(screen(target), direction, distance);
            (from, to, direction.name().into())
        }
    };
    let outcome = target.mirror.act(&Action::Swipe { from, to, duration })?;
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
        None => screen(target),
    };
    let extent = if direction.vertical() {
        area.height()
    } else {
        area.width()
    };
    // The content moves in the direction, so the finger moves the other way;
    // a long scroll takes several strokes.
    let longest = (extent * MAX_STROKE).max(1.0);
    let mut left = amount.max(0.0);
    let mut strokes = 0u32;
    while left > 0.5 && strokes < 20 {
        let step = left.min(longest);
        let (from, to) = stroke(area, direction.opposite(), step);
        let outcome = target.mirror.act(&Action::Swipe {
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
