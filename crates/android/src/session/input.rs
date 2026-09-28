//! The input sink: points in device points to the agent's input and text
//! requests in physical pixels, or to the controller's touches.

use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use mirror::Point;
use mirror::backend::{BackendError, Input, InputAck, InputSink};
use protocol::code;
use protocol::message::{Call, KeyParams, Reply, SwipeParams, TapParams, TextMethod, TextParams};

use super::Inner;
use crate::emulator::{self, TouchPoint};

/// How often a swipe on the controller moves its contact, as the agent's
/// swipe does.
const SWIPE_STEP: Duration = Duration::from_millis(8);

fn millis(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

/// The agent request for `input`, at `scale` physical pixels per point.
pub fn agent_call(input: &Input, scale: f64) -> Call {
    match input {
        Input::Tap { at, hold } => {
            let (x, y) = at.to_pixels(scale);
            Call::Tap(TapParams {
                x,
                y,
                duration_ms: hold.map(millis),
            })
        }
        Input::Swipe { from, to, duration } => {
            let (x1, y1) = from.to_pixels(scale);
            let (x2, y2) = to.to_pixels(scale);
            Call::Swipe(SwipeParams {
                x1,
                y1,
                x2,
                y2,
                duration_ms: millis(*duration),
                rest_ms: None,
            })
        }
        Input::Key { code, meta } => Call::Key(KeyParams {
            code: *code,
            meta: *meta,
        }),
        Input::Text { text, node, method } => Call::TextSet(TextParams {
            text: text.clone(),
            node: node.map(|handle| handle.node),
            mode: (*method != TextMethod::AUTO).then_some(*method),
        }),
    }
}

/// The acknowledgement an agent reply carries.
pub fn input_ack(reply: Reply) -> Result<InputAck, protocol::Error> {
    match reply {
        Reply::Input(ack) => Ok(InputAck {
            accepted: ack.accepted,
            method: None,
        }),
        Reply::Text(ack) => Ok(InputAck {
            accepted: true,
            method: Some(ack.method),
        }),
        _ => Err(protocol::Error::new(
            code::PROTOCOL_ERROR,
            "the agent answered an input request with another result",
        )),
    }
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
    let steps = if step.is_zero() {
        1
    } else {
        duration.as_nanos().div_ceil(step.as_nanos()).max(1)
    };
    let steps = u32::try_from(steps).unwrap_or(u32::MAX);
    let along = |from: i32, to: i32, i: u32| {
        from + ((f64::from(to - from) * f64::from(i)) / f64::from(steps)).round() as i32
    };
    let mut touches = Vec::with_capacity(steps as usize + 2);
    touches.push((Duration::ZERO, TouchPoint::down(from.0, from.1)));
    for i in 1..=steps {
        touches.push((
            duration * i / steps,
            TouchPoint::down(along(from.0, to.0, i), along(from.1, to.1, i)),
        ));
    }
    touches.push((duration, TouchPoint::up(to.0, to.1)));
    touches
}

/// The session's input sink.
pub(crate) struct SessionInput {
    pub(crate) inner: Arc<Inner>,
}

/// Whether a controller failure happened before anything reached the
/// device, so that the agent can take the same input.
enum ControllerFailure {
    BeforeContact(emulator::Error),
    AfterContact(emulator::Error),
}

impl SessionInput {
    /// Taps and swipes through the controller; `None` for input it does not
    /// take.
    fn through_controller(&self, input: &Input) -> Option<Result<InputAck, ControllerFailure>> {
        let inner = &*self.inner;
        let link = inner.controller.as_ref()?;
        if inner
            .controller_failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
        {
            return None;
        }
        let pixels = |p: &Point| p.to_pixels(inner.scale);
        let touches = match input {
            Input::Tap { at, hold } => {
                let (x, y) = pixels(at);
                let hold = hold.unwrap_or(Duration::ZERO);
                vec![
                    (Duration::ZERO, TouchPoint::down(x, y)),
                    (hold, TouchPoint::up(x, y)),
                ]
            }
            Input::Swipe { from, to, duration } => {
                swipe_touches(pixels(from), pixels(to), *duration, SWIPE_STEP)
            }
            Input::Key { .. } | Input::Text { .. } => return None,
        };
        let controller = link.controller().clone();
        let result = link.block_on(async move {
            let start = tokio::time::Instant::now();
            for (i, (at, point)) in touches.iter().enumerate() {
                tokio::time::sleep_until(start + *at).await;
                if let Err(error) = controller.touch(std::slice::from_ref(point)).await {
                    return Err(if i == 0 {
                        ControllerFailure::BeforeContact(error)
                    } else {
                        ControllerFailure::AfterContact(error)
                    });
                }
            }
            Ok(())
        });
        if !matches!(result, Err(ControllerFailure::BeforeContact(_))) {
            inner.note_foreign_input(Instant::now());
            inner.controller_inputs.fetch_add(1, Ordering::SeqCst);
        }
        Some(result.map(|()| InputAck {
            accepted: true,
            method: None,
        }))
    }
}

impl InputSink for SessionInput {
    fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError> {
        match self.through_controller(input) {
            Some(Ok(ack)) => return Ok(ack),
            Some(Err(failure)) => {
                let (error, retry) = match failure {
                    ControllerFailure::BeforeContact(error) => (error, true),
                    ControllerFailure::AfterContact(error) => (error, false),
                };
                *self
                    .inner
                    .controller_failed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(error.to_string());
                if !retry {
                    return Err(BackendError::new(
                        code::INTERNAL,
                        format!(
                            "the emulator controller failed during the gesture ({error}); \
                             later input goes through the agent"
                        ),
                    ));
                }
            }
            None => {}
        }
        let inner = &*self.inner;
        let gesture = match input {
            Input::Tap { hold, .. } => hold.unwrap_or(Duration::ZERO),
            Input::Swipe { duration, .. } => *duration,
            Input::Key { .. } | Input::Text { .. } => Duration::ZERO,
        };
        let reply = inner.connection.call(
            agent_call(input, inner.scale),
            inner.options.input_deadline + gesture,
        )?;
        inner.agent_inputs.fetch_add(1, Ordering::SeqCst);
        input_ack(reply)
    }
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
                duration_ms: 300,
                rest_ms: None
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
