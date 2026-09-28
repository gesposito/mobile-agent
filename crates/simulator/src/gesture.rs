//! Gestures as timed touch steps, whatever transport carries them. A point
//! becomes a ratio of the native portrait framebuffer, the unit dtuhidd and
//! Indigo both take: `point * scale / pixels`, as in idb and spike S3.

use std::time::Duration;

use crate::error::{Error, Result};
use crate::model::{Edge, Gesture, Point, Swipe};

/// The fingers dtuhidd's digitizer event carries.
const MAX_FINGERS: usize = 2;

/// A contact as a ratio of the native portrait framebuffer, from the top
/// left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ratio {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
}

/// One touch event of a gesture, `at` after the gesture starts.
#[derive(Clone, Debug, PartialEq)]
pub struct TouchStep {
    pub at: Duration,
    pub phase: Phase,
    /// One contact, or two for a two-finger gesture.
    pub contacts: Vec<Ratio>,
    pub edge: Edge,
}

/// A device's main screen: its native portrait size in pixels and the scale
/// from points to pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub pixels: (f64, f64),
    pub scale: f64,
}

impl Screen {
    /// The size in points.
    pub fn points(&self) -> (f64, f64) {
        (self.pixels.0 / self.scale, self.pixels.1 / self.scale)
    }

    /// The ratio `point` lands on; points off the screen are refused.
    pub fn ratio(&self, point: Point) -> Result<Ratio> {
        let (width, height) = self.points();
        let inside = |value: f64, limit: f64| (0.0..=limit).contains(&value);
        if !inside(point.x, width) || !inside(point.y, height) {
            return Err(Error::InvalidArgument(format!(
                "({}, {}) is off the {width}x{height} point screen",
                point.x, point.y
            )));
        }
        Ok(Ratio {
            x: point.x * self.scale / self.pixels.0,
            y: point.y * self.scale / self.pixels.1,
        })
    }
}

/// Touch down at `point`, then up after `hold`.
pub fn press(screen: &Screen, point: Point, hold: Duration) -> Result<Vec<TouchStep>> {
    let contact = screen.ratio(point)?;
    let step = |at, phase| TouchStep {
        at,
        phase,
        contacts: vec![contact],
        edge: Edge::None,
    };
    Ok(vec![
        step(Duration::ZERO, Phase::Down),
        step(hold, Phase::Up),
    ])
}

/// How many moves a finger that rests creeps on before it lifts.
const CREEP_MOVES: u32 = 3;

/// How far each of those moves goes, in points: on the Simulator, moves of
/// a pixel left the drag its momentum at one start point in sixteen, and
/// moves of a point at none.
const CREEP_STEP: f64 = 1.0;

/// How far apart those moves come, about one per display frame at 60 Hz.
const CREEP_EVERY: Duration = Duration::from_millis(16);

/// Touch down at the start, `steps` positions spread evenly until
/// `duration`, the last position once more, and up at `duration`.
/// Repeating the last position keeps an arm64 guest from adding inertia to
/// the drag's end, as idb does; the finger still lifts moving, and UIKit
/// glides the content on at the drag's speed.
///
/// With a `rest`, the finger comes to rest instead (task 10.16): it is at
/// the end at `duration`, sends nothing for the rest, then creeps on along
/// its line, [`CREEP_MOVES`] moves of [`CREEP_STEP`] points
/// [`CREEP_EVERY`] apart, and lifts at the last. UIKit takes a drag's
/// velocity from its last moves, which then crawl, so the content stops
/// where the finger left it; a rest with no move after it, or with one move
/// of up to 3 points, still ended in the drag's full momentum on the
/// Simulator.
pub fn swipe(screen: &Screen, swipe: &Swipe) -> Result<Vec<TouchStep>> {
    let mut plan = walk(
        screen,
        &[vec![swipe.from, swipe.to]],
        swipe.duration,
        swipe.steps,
        swipe.edge,
    )?;
    let Some(rest) = swipe.rest else {
        return Ok(plan);
    };
    plan.pop();
    let end = screen.ratio(swipe.to)?;
    let step = |at, phase, contact| TouchStep {
        at,
        phase,
        contacts: vec![contact],
        edge: swipe.edge,
    };
    plan.push(step(swipe.duration, Phase::Move, end));
    let (dx, dy) = (swipe.to.x - swipe.from.x, swipe.to.y - swipe.from.y);
    let length = dx.hypot(dy);
    let (width, height) = screen.points();
    let mut last = (swipe.duration, end);
    for k in 1..=CREEP_MOVES {
        let crept = if length > 0.0 {
            let on = CREEP_STEP * f64::from(k) / length;
            Point::new(
                (swipe.to.x + dx * on).clamp(0.0, width),
                (swipe.to.y + dy * on).clamp(0.0, height),
            )
        } else {
            swipe.to
        };
        let at = swipe.duration + rest + CREEP_EVERY * (k - 1);
        last = (at, screen.ratio(crept)?);
        plan.push(step(last.0, Phase::Move, last.1));
    }
    plan.push(step(last.0, Phase::Up, last.1));
    Ok(plan)
}

/// Every finger's path walked at an even pace, by length, over the same
/// steps; both contacts ride in each event.
pub fn gesture(screen: &Screen, gesture: &Gesture) -> Result<Vec<TouchStep>> {
    walk(
        screen,
        &gesture.fingers,
        gesture.duration,
        gesture.steps,
        Edge::None,
    )
}

/// Down at every path's start, `steps` positions at even intervals until
/// `duration`, the last once more, and up at `duration`.
fn walk(
    screen: &Screen,
    fingers: &[Vec<Point>],
    duration: Duration,
    steps: u32,
    edge: Edge,
) -> Result<Vec<TouchStep>> {
    if fingers.is_empty() || fingers.len() > MAX_FINGERS {
        return Err(Error::InvalidArgument(format!(
            "a gesture has one or two fingers, not {}",
            fingers.len()
        )));
    }
    if fingers.iter().any(Vec::is_empty) {
        return Err(Error::InvalidArgument(
            "every finger needs a path of at least one point".into(),
        ));
    }
    let steps = steps.max(1);
    let contacts = |fraction: f64| -> Result<Vec<Ratio>> {
        fingers
            .iter()
            .map(|path| screen.ratio(along(path, fraction)))
            .collect()
    };
    let interval = duration / (steps + 1);
    let mut plan = Vec::with_capacity(steps as usize + 3);
    let event = |at, phase, contacts| TouchStep {
        at,
        phase,
        contacts,
        edge,
    };
    plan.push(event(Duration::ZERO, Phase::Down, contacts(0.0)?));
    for index in 1..=steps {
        let fraction = f64::from(index) / f64::from(steps);
        plan.push(event(interval * index, Phase::Move, contacts(fraction)?));
    }
    let end = contacts(1.0)?;
    plan.push(event(interval * steps, Phase::Move, end.clone()));
    plan.push(event(duration, Phase::Up, end));
    Ok(plan)
}

/// The point `fraction` of the way along a path, by length.
fn along(path: &[Point], fraction: f64) -> Point {
    let lengths: Vec<f64> = path
        .windows(2)
        .map(|leg| (leg[1].x - leg[0].x).hypot(leg[1].y - leg[0].y))
        .collect();
    let total: f64 = lengths.iter().sum();
    if total == 0.0 {
        return path[0];
    }
    let mut remaining = fraction.clamp(0.0, 1.0) * total;
    for (leg, length) in path.windows(2).zip(&lengths) {
        if remaining <= *length && *length > 0.0 {
            let t = remaining / length;
            return Point::new(
                leg[0].x + (leg[1].x - leg[0].x) * t,
                leg[0].y + (leg[1].y - leg[0].y) * t,
            );
        }
        remaining -= length;
    }
    *path.last().expect("a non-empty path")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    /// iPhone 17: 1206x2622 pixels at 3x, 402x874 points.
    const IPHONE_17: Screen = Screen {
        pixels: (1206.0, 2622.0),
        scale: 3.0,
    };

    fn close(a: Ratio, b: (f64, f64)) -> bool {
        (a.x - b.0).abs() < 1e-9 && (a.y - b.1).abs() < 1e-9
    }

    #[test]
    fn a_point_scales_to_a_ratio_of_the_native_framebuffer() {
        assert_eq!(IPHONE_17.points(), (402.0, 874.0));
        let ratio = |x, y| IPHONE_17.ratio(Point::new(x, y)).expect("on screen");
        assert!(close(ratio(201.0, 437.0), (0.5, 0.5)));
        assert!(close(ratio(0.0, 0.0), (0.0, 0.0)));
        assert!(close(ratio(402.0, 874.0), (1.0, 1.0)));
        assert!(close(ratio(120.6, 812.0), (0.3, 812.0 / 874.0)));
        let two_x = Screen {
            pixels: (750.0, 1334.0),
            scale: 2.0,
        };
        let centre = two_x.ratio(Point::new(187.5, 333.5)).expect("on screen");
        assert!(close(centre, (0.5, 0.5)));
    }

    #[test]
    fn a_point_off_the_screen_is_refused() {
        for point in [
            Point::new(-1.0, 10.0),
            Point::new(10.0, 875.0),
            Point::new(f64::NAN, 1.0),
        ] {
            assert!(matches!(
                IPHONE_17.ratio(point),
                Err(Error::InvalidArgument(_))
            ));
        }
    }

    #[test]
    fn a_press_is_down_then_up_after_its_hold() {
        let steps = press(
            &IPHONE_17,
            Point::new(201.0, 437.0),
            Duration::from_millis(40),
        )
        .expect("a press");
        assert_eq!(steps.len(), 2);
        assert_eq!((steps[0].phase, steps[0].at), (Phase::Down, Duration::ZERO));
        assert_eq!(
            (steps[1].phase, steps[1].at),
            (Phase::Up, Duration::from_millis(40))
        );
        for step in &steps {
            assert_eq!(step.contacts.len(), 1);
            assert!(close(step.contacts[0], (0.5, 0.5)));
            assert_eq!(step.edge, Edge::None);
        }
    }

    #[test]
    fn a_swipe_spreads_its_positions_and_repeats_the_last() {
        let mut spec = Swipe::new(
            Point::new(201.0, 700.0),
            Point::new(201.0, 250.0),
            Duration::from_millis(420),
        );
        spec.steps = 20;
        spec.edge = Edge::Bottom;
        let steps = swipe(&IPHONE_17, &spec).expect("a swipe");
        // Down, 20 positions, the last again, up.
        assert_eq!(steps.len(), 23);
        assert_eq!(steps[0].phase, Phase::Down);
        assert_eq!(steps[0].at, Duration::ZERO);
        assert!(close(steps[0].contacts[0], (0.5, 700.0 / 874.0)));
        let step = Duration::from_millis(20);
        for (index, event) in steps[1..=20].iter().enumerate() {
            assert_eq!(event.phase, Phase::Move);
            assert_eq!(event.at, step * (index as u32 + 1));
        }
        assert!(close(steps[20].contacts[0], (0.5, 250.0 / 874.0)));
        assert_eq!(steps[21].phase, Phase::Move);
        assert_eq!(steps[21].contacts, steps[20].contacts);
        assert_eq!(steps[21].at, steps[20].at);
        assert_eq!(steps[22].phase, Phase::Up);
        assert_eq!(steps[22].at, Duration::from_millis(420));
        assert!(steps.iter().all(|event| event.edge == Edge::Bottom));
        let ys: Vec<f64> = steps.iter().map(|event| event.contacts[0].y).collect();
        assert!(ys.windows(2).all(|pair| pair[1] <= pair[0]), "{ys:?}");
    }

    /// Task 10.16: a swipe that rests before the finger lifts reaches its
    /// end at its duration, sends nothing for the rest, then creeps a point
    /// at a time on along its line, three moves 16 ms apart, and lifts at
    /// the last, so UIKit, which takes a drag's velocity from its last
    /// moves, ends the drag without momentum.
    #[test]
    fn a_swipe_that_rests_reaches_its_end_then_creeps_a_point_at_a_time_and_lifts() {
        let mut spec = Swipe::new(
            Point::new(201.0, 700.0),
            Point::new(201.0, 250.0),
            Duration::from_millis(420),
        );
        spec.rest = Some(Duration::from_millis(60));
        let steps = swipe(&IPHONE_17, &spec).expect("a swipe");
        // Down, 20 positions, the last again, the end at the duration, three
        // creeping moves, up.
        assert_eq!(steps.len(), 27, "{steps:?}");
        let end = (0.5, 250.0 / 874.0);
        assert_eq!(
            (steps[22].phase, steps[22].at),
            (Phase::Move, Duration::from_millis(420))
        );
        assert!(close(steps[22].contacts[0], end));
        for (k, event) in steps[23..26].iter().enumerate() {
            let k = k as u32;
            assert_eq!(event.phase, Phase::Move);
            assert_eq!(event.at, Duration::from_millis(480 + 16 * u64::from(k)));
            let y = 250.0 - f64::from(k + 1);
            assert!(close(event.contacts[0], (0.5, y / 874.0)), "{event:?}");
        }
        let up = &steps[26];
        assert_eq!((up.phase, up.at), (Phase::Up, Duration::from_millis(512)));
        assert_eq!(up.contacts, steps[25].contacts);

        // At the screen's edge the creep stops there.
        let mut edge = Swipe::new(
            Point::new(201.0, 400.0),
            Point::new(201.0, 1.0),
            Duration::from_millis(300),
        );
        edge.rest = Some(Duration::from_millis(60));
        let steps = swipe(&IPHONE_17, &edge).expect("a swipe to the edge");
        assert!(close(steps.last().unwrap().contacts[0], (0.5, 0.0)));
    }

    #[test]
    fn a_two_finger_gesture_carries_both_contacts_in_every_event() {
        let pinch = Gesture {
            fingers: vec![
                vec![Point::new(160.0, 437.0), Point::new(60.0, 437.0)],
                vec![Point::new(242.0, 437.0), Point::new(342.0, 437.0)],
            ],
            duration: Duration::from_millis(300),
            steps: 10,
        };
        let steps = gesture(&IPHONE_17, &pinch).expect("a pinch");
        assert_eq!(steps.len(), 13);
        assert!(steps.iter().all(|event| event.contacts.len() == 2));
        let first = &steps[0].contacts;
        let last = &steps[12].contacts;
        assert!(close(first[0], (160.0 / 402.0, 0.5)));
        assert!(close(last[1], (342.0 / 402.0, 0.5)));
        assert_eq!(steps[12].phase, Phase::Up);
    }

    #[test]
    fn a_path_is_walked_at_an_even_pace_by_length() {
        // An L: 100 points right, then 300 down; half way is 100 points down
        // the second leg.
        let walk = Gesture {
            fingers: vec![vec![
                Point::new(50.0, 100.0),
                Point::new(150.0, 100.0),
                Point::new(150.0, 400.0),
            ]],
            duration: Duration::from_millis(100),
            steps: 2,
        };
        let steps = gesture(&IPHONE_17, &walk).expect("a walk");
        let middle = steps[1].contacts[0];
        assert!(close(middle, (150.0 / 402.0, 200.0 / 874.0)), "{middle:?}");
    }

    #[test]
    fn unsupported_gestures_are_refused() {
        let three = Gesture {
            fingers: vec![vec![Point::new(10.0, 10.0)]; 3],
            duration: Duration::from_millis(100),
            steps: 5,
        };
        assert!(matches!(
            gesture(&IPHONE_17, &three),
            Err(Error::InvalidArgument(_))
        ));
        let empty = Gesture {
            fingers: vec![Vec::new()],
            duration: Duration::from_millis(100),
            steps: 5,
        };
        assert!(matches!(
            gesture(&IPHONE_17, &empty),
            Err(Error::InvalidArgument(_))
        ));
    }
}
