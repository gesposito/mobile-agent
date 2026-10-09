//! Touches as `mainTouchscreen` reports (design D22): a point in points
//! becomes the touchscreen's 16-bit coordinates over the screen, and a tap,
//! a press or a swipe a timed series of contact reports ending with a
//! release, as spike S11 sent them through `dtuhidd`.
//!
//! A swipe that rests follows the Android `rest_ms` semantics, the finger
//! still for 60 ms at its end, then moves on as the Simulator driver's does:
//! UIKit takes a drag's velocity from its last moves, and on the iPhone SE a
//! rest followed by one pixel on still flung the list 917 px for a 400 px
//! stroke, where three moves of a point 16 ms apart stopped it after 323 px
//! (task 3.8's device check).

use std::time::Duration;

use crate::model::{Point, Release, Screen, Swipe};

/// How long a tap's contact lasts before the release: S11's tap, which
/// every one of 40 taps saw reach the screen.
pub const TAP_HOLD: Duration = Duration::from_millis(50);
/// How far apart a moving finger's reports come: one per display frame at
/// 60 Hz, as S11's drags sent them.
pub const REPORT_EVERY: Duration = Duration::from_millis(16);
/// How long a resting finger stays still at the end of its stroke before it
/// moves one pixel on and lifts: the Android `rest_ms`.
pub const REST: Duration = Duration::from_millis(60);

/// How many moves a resting finger creeps on before it lifts, and how far
/// each goes in points: the Simulator driver's, for the same UIKit.
pub const CREEP_MOVES: u32 = 3;
pub const CREEP_STEP: f64 = 1.0;

/// Whether a report's finger touches the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contact {
    Touching,
    Lifted,
}

/// One `mainTouchscreen` report, `at` after the gesture starts, at the
/// touchscreen's 16-bit coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchReport {
    pub at: Duration,
    pub contact: Contact,
    pub x: u16,
    pub y: u16,
}

/// The touchscreen's coordinates of `point`: 0 to 65535 across the screen
/// in points on each axis. A point off the screen is refused.
pub fn normalize(screen: &Screen, point: Point) -> Result<(u16, u16), String> {
    let (width, height) = screen.points();
    let inside = |value: f64, limit: f64| (0.0..=limit).contains(&value);
    if !inside(point.x, width) || !inside(point.y, height) {
        return Err(format!(
            "({}, {}) is off the {width}x{height} point screen",
            point.x, point.y
        ));
    }
    let scaled = |value: f64, limit: f64| (value / limit * 65535.0).round() as u16;
    Ok((scaled(point.x, width), scaled(point.y, height)))
}

/// A contact at `point` held for `hold`, then the release there: a tap
/// with [`TAP_HOLD`].
pub fn press(screen: &Screen, point: Point, hold: Duration) -> Result<Vec<TouchReport>, String> {
    let (x, y) = normalize(screen, point)?;
    let report = |at, contact| TouchReport { at, contact, x, y };
    Ok(vec![
        report(Duration::ZERO, Contact::Touching),
        report(hold, Contact::Lifted),
    ])
}

/// A stroke: contacts every [`REPORT_EVERY`] along the line from `from` to
/// `to`, the last at `to` after `duration`. A moving finger lifts there; a
/// resting one stays at `to`, reporting, for [`REST`], creeps on along its
/// line [`CREEP_MOVES`] moves of [`CREEP_STEP`] points [`REPORT_EVERY`]
/// apart, and lifts at the last.
pub fn swipe(screen: &Screen, swipe: &Swipe) -> Result<Vec<TouchReport>, String> {
    let steps = if swipe.duration.is_zero() {
        1
    } else {
        u32::try_from(swipe.duration.as_nanos().div_ceil(REPORT_EVERY.as_nanos()))
            .unwrap_or(u32::MAX)
            .max(1)
    };
    let along =
        |from: f64, to: f64, step: u32| from + (to - from) * f64::from(step) / f64::from(steps);
    let mut reports = Vec::with_capacity(steps as usize + 8);
    for step in 0..=steps {
        let point = Point::new(
            along(swipe.from.x, swipe.to.x, step),
            along(swipe.from.y, swipe.to.y, step),
        );
        let (x, y) = normalize(screen, point)?;
        reports.push(TouchReport {
            at: swipe.duration * step / steps,
            contact: Contact::Touching,
            x,
            y,
        });
    }
    let (x, y) = normalize(screen, swipe.to)?;
    let mut lift = TouchReport {
        at: swipe.duration,
        contact: Contact::Lifted,
        x,
        y,
    };
    if swipe.release == Release::Resting {
        // The finger stays at the end, reporting as a held finger does.
        let mut still = REPORT_EVERY;
        while still < REST {
            reports.push(TouchReport {
                at: swipe.duration + still,
                contact: Contact::Touching,
                x,
                y,
            });
            still += REPORT_EVERY;
        }
        // Then it creeps on along its line, a point a move, and lifts at
        // the last: UIKit takes a drag's velocity from its last moves.
        let (dx, dy) = (swipe.to.x - swipe.from.x, swipe.to.y - swipe.from.y);
        let length = dx.hypot(dy);
        let (width, height) = screen.points();
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
            let (x, y) = normalize(screen, crept)?;
            let at = swipe.duration + REST + REPORT_EVERY * (k - 1);
            reports.push(TouchReport {
                at,
                contact: Contact::Touching,
                x,
                y,
            });
            lift = TouchReport {
                at,
                contact: Contact::Lifted,
                x,
                y,
            };
        }
    }
    reports.push(lift);
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The iPhone SE (3rd generation): 750x1334 pixels at 2x.
    const SE: Screen = Screen {
        width: 750,
        height: 1334,
        scale: 2.0,
    };

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn points_map_to_the_touchscreen_space_and_off_screen_points_are_refused() {
        assert_eq!(normalize(&SE, Point::new(0.0, 0.0)), Ok((0, 0)));
        assert_eq!(normalize(&SE, Point::new(375.0, 667.0)), Ok((65535, 65535)));
        assert_eq!(normalize(&SE, Point::new(187.5, 333.5)), Ok((32768, 32768)));
        for off in [
            Point::new(-1.0, 10.0),
            Point::new(10.0, 668.0),
            Point::new(376.0, 0.0),
        ] {
            let refused = normalize(&SE, off).expect_err("off the screen");
            assert!(refused.contains("375x667"), "{refused}");
        }
    }

    #[test]
    fn a_tap_is_a_contact_held_50_ms_then_a_release_at_the_same_point() {
        let at = normalize(&SE, Point::new(187.0, 550.0)).expect("on screen");
        assert_eq!(
            press(&SE, Point::new(187.0, 550.0), TAP_HOLD),
            Ok(vec![
                TouchReport {
                    at: Duration::ZERO,
                    contact: Contact::Touching,
                    x: at.0,
                    y: at.1
                },
                TouchReport {
                    at: TAP_HOLD,
                    contact: Contact::Lifted,
                    x: at.0,
                    y: at.1
                },
            ])
        );
        assert_eq!(TAP_HOLD, ms(50));
        let long = press(&SE, Point::new(10.0, 10.0), ms(800)).expect("a press");
        assert_eq!(long.len(), 2);
        assert_eq!(long[1].at, ms(800));
        assert_eq!(long[1].contact, Contact::Lifted);
    }

    #[test]
    fn a_moving_swipe_reports_every_16_ms_along_its_line_and_lifts_at_its_end() {
        let stroke = Swipe::new(Point::new(187.0, 500.0), Point::new(187.0, 260.0), ms(160));
        let reports = swipe(&SE, &stroke).expect("a swipe");
        let start = normalize(&SE, stroke.from).expect("on screen");
        let end = normalize(&SE, stroke.to).expect("on screen");
        assert_eq!((reports[0].x, reports[0].y), start);
        assert_eq!(reports[0].at, Duration::ZERO);
        let (lift, moves) = reports.split_last().expect("reports");
        assert_eq!(lift.contact, Contact::Lifted);
        assert_eq!((lift.x, lift.y), end);
        assert_eq!(lift.at, ms(160));
        assert!(
            moves
                .iter()
                .all(|report| report.contact == Contact::Touching)
        );
        assert_eq!(moves.len(), 11, "every 16 ms over 160 ms, both ends");
        assert!(
            moves
                .windows(2)
                .all(|pair| pair[1].at - pair[0].at <= REPORT_EVERY && pair[1].y < pair[0].y),
            "{moves:?}"
        );
        assert_eq!(moves.last().map(|report| (report.x, report.y)), Some(end));
    }

    #[test]
    fn a_resting_swipe_stays_still_60_ms_then_creeps_on_three_points_and_lifts_there() {
        let stroke =
            Swipe::new(Point::new(187.0, 500.0), Point::new(187.0, 260.0), ms(160)).resting();
        let reports = swipe(&SE, &stroke).expect("a swipe");
        let end = normalize(&SE, stroke.to).expect("on screen");
        let arrived = reports
            .iter()
            .position(|report| (report.x, report.y) == end)
            .expect("the finger reaches the end");
        assert_eq!(reports[arrived].at, ms(160));
        let (lift, rest) = reports[arrived..].split_last().expect("reports");
        let creep = rest.len() - CREEP_MOVES as usize;
        let (still, crept) = rest.split_at(creep);
        assert!(
            still
                .iter()
                .all(|report| (report.x, report.y) == end && report.contact == Contact::Touching),
            "the finger stays at the end while it rests: {still:?}"
        );
        assert!(
            still.len() >= 4,
            "it keeps reporting while it rests: {still:?}"
        );
        // A point a move up the screen, the stroke's way, 16 ms apart.
        for (k, report) in crept.iter().enumerate() {
            let on = normalize(&SE, Point::new(187.0, 260.0 - (k + 1) as f64 * CREEP_STEP))
                .expect("on screen");
            assert_eq!((report.x, report.y), on, "move {k}");
            assert_eq!(report.at, ms(160) + REST + REPORT_EVERY * k as u32);
            assert_eq!(report.contact, Contact::Touching);
        }
        let last = crept.last().expect("the creep");
        assert_eq!(lift.contact, Contact::Lifted);
        assert_eq!((lift.x, lift.y), (last.x, last.y));
        assert_eq!(lift.at, last.at);
        assert_eq!(REST, ms(60));
    }

    #[test]
    fn a_swipe_that_leaves_the_screen_is_refused() {
        let stroke = Swipe::new(Point::new(10.0, 10.0), Point::new(10.0, 700.0), ms(100));
        assert!(swipe(&SE, &stroke).is_err());
    }
}
