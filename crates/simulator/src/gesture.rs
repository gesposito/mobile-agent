//! Gestures as timed touch steps, whatever transport carries them. A point
//! becomes a ratio of the native portrait framebuffer, the unit dtuhidd and
//! Indigo both take: `point * scale / pixels`, as in idb and spike S3.

use std::time::Duration;

use crate::error::Result;
use crate::model::{Edge, Gesture, Point, Swipe};

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
        unimplemented!("task 6.3: gesture planning")
    }

    /// The ratio `point` lands on; points off the screen are refused.
    pub fn ratio(&self, point: Point) -> Result<Ratio> {
        let _ = point;
        unimplemented!("task 6.3: gesture planning")
    }
}

/// Touch down at `point`, then up after `hold`.
pub fn press(screen: &Screen, point: Point, hold: Duration) -> Result<Vec<TouchStep>> {
    let _ = (screen, point, hold);
    unimplemented!("task 6.3: gesture planning")
}

/// Touch down at the start, `steps` positions spread evenly until
/// `duration`, the last position once more, and up at `duration`. Repeating
/// the last position keeps an arm64 guest from adding inertia to the drag's
/// end, as idb does.
pub fn swipe(screen: &Screen, swipe: &Swipe) -> Result<Vec<TouchStep>> {
    let _ = (screen, swipe);
    unimplemented!("task 6.3: gesture planning")
}

/// Every finger's path walked at an even pace, by length, over the same
/// steps; both contacts ride in each event.
pub fn gesture(screen: &Screen, gesture: &Gesture) -> Result<Vec<TouchStep>> {
    let _ = (screen, gesture);
    unimplemented!("task 6.3: gesture planning")
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
