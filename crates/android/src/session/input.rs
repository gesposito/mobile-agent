//! The input sink: points in device points to the agent's input and text
//! requests in physical pixels, or to the controller's touches.

use std::time::Duration;

use mirror::backend::{Input, InputAck};
use protocol::message::{Call, Reply};

use crate::emulator::TouchPoint;

/// The agent request for `input`, at `scale` physical pixels per point.
pub fn agent_call(input: &Input, scale: f64) -> Call {
    let _ = (input, scale);
    todo!("task 5.3")
}

/// The acknowledgement an agent reply carries.
pub fn input_ack(reply: Reply) -> Result<InputAck, protocol::Error> {
    let _ = reply;
    todo!("task 5.3")
}

/// The controller touches of a swipe from `from` to `to` over `duration`,
/// each with its offset from the start: contact 0 down at the start, a move
/// every `step` along the line, and up at the end, as the agent's swipe
/// does.
pub fn swipe_touches(
    from: (i32, i32),
    to: (i32, i32),
    duration: Duration,
    step: Duration,
) -> Vec<(Duration, TouchPoint)> {
    let _ = (from, to, duration, step);
    todo!("task 5.3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirror::Point;
    use mirror::backend::NodeHandle;
    use protocol::code;
    use protocol::message::{
        InputAck as AgentAck, KeyParams, SwipeParams, TapParams, TextAck, TextMethod, TextParams,
    };

    const SCALE: f64 = 2.625;

    #[test]
    fn taps_and_long_presses_go_out_in_pixels() {
        let tap = Input::Tap {
            at: Point::new(100.0, 200.0),
            hold: None,
        };
        assert_eq!(
            agent_call(&tap, SCALE),
            Call::Tap(TapParams {
                x: 263,
                y: 525,
                duration_ms: None
            })
        );
        let press = Input::Tap {
            at: Point::new(10.4, 0.2),
            hold: Some(Duration::from_millis(800)),
        };
        assert_eq!(
            agent_call(&press, SCALE),
            Call::Tap(TapParams {
                x: 27,
                y: 1,
                duration_ms: Some(800)
            })
        );
    }

    #[test]
    fn swipes_keep_their_duration_and_keys_their_code() {
        let swipe = Input::Swipe {
            from: Point::new(200.0, 800.0),
            to: Point::new(200.0, 200.0),
            duration: Duration::from_millis(300),
        };
        assert_eq!(
            agent_call(&swipe, SCALE),
            Call::Swipe(SwipeParams {
                x1: 525,
                y1: 2100,
                x2: 525,
                y2: 525,
                duration_ms: 300
            })
        );
        let key = Input::Key {
            code: 66,
            meta: Some(1),
        };
        assert_eq!(
            agent_call(&key, SCALE),
            Call::Key(KeyParams {
                code: 66,
                meta: Some(1)
            })
        );
    }

    #[test]
    fn text_names_the_node_of_the_latest_read_and_its_method() {
        let text = Input::Text {
            text: "hunter2".into(),
            node: Some(NodeHandle {
                window: 3,
                node: 42,
            }),
            method: TextMethod::DIRECT,
        };
        assert_eq!(
            agent_call(&text, SCALE),
            Call::TextSet(TextParams {
                text: "hunter2".into(),
                node: Some(42),
                mode: Some(TextMethod::DIRECT)
            })
        );
        let focused = Input::Text {
            text: "a".into(),
            node: None,
            method: TextMethod::AUTO,
        };
        assert_eq!(
            agent_call(&focused, SCALE),
            Call::TextSet(TextParams {
                text: "a".into(),
                node: None,
                mode: None
            }),
            "auto leaves the choice to the agent"
        );
    }

    #[test]
    fn acknowledgements_carry_the_platforms_verdict_and_the_text_method() {
        assert_eq!(
            input_ack(Reply::Input(AgentAck { accepted: false })).unwrap(),
            InputAck {
                accepted: false,
                method: None
            }
        );
        assert_eq!(
            input_ack(Reply::Text(TextAck {
                method: TextMethod::KEYS
            }))
            .unwrap(),
            InputAck {
                accepted: true,
                method: Some(TextMethod::KEYS)
            }
        );
        let error = input_ack(Reply::Empty).unwrap_err();
        assert_eq!(error.code, code::PROTOCOL_ERROR);
    }

    #[test]
    fn a_swipe_on_the_controller_is_down_moves_and_up_along_the_line() {
        let touches = swipe_touches(
            (100, 1000),
            (100, 200),
            Duration::from_millis(40),
            Duration::from_millis(8),
        );
        let first = touches.first().unwrap();
        assert_eq!(first, &(Duration::ZERO, TouchPoint::down(100, 1000)));
        let last = touches.last().unwrap();
        assert_eq!(last, &(Duration::from_millis(40), TouchPoint::up(100, 200)));
        let moves = &touches[1..touches.len() - 1];
        assert_eq!(moves.len(), 5, "{touches:?}");
        assert_eq!(moves.last().unwrap().1, TouchPoint::down(100, 200));
        let mut previous = (Duration::ZERO, 1000);
        for (at, point) in moves {
            assert!(*at > previous.0 && point.y < previous.1, "{touches:?}");
            assert_eq!(point.pressure, crate::emulator::DOWN_PRESSURE);
            previous = (*at, point.y);
        }
    }

    #[test]
    fn a_swipe_without_duration_still_moves_once() {
        let touches = swipe_touches((0, 0), (50, 0), Duration::ZERO, Duration::from_millis(8));
        assert_eq!(
            touches,
            vec![
                (Duration::ZERO, TouchPoint::down(0, 0)),
                (Duration::ZERO, TouchPoint::down(50, 0)),
                (Duration::ZERO, TouchPoint::up(50, 0)),
            ]
        );
    }
}
