//! The dtuhidd XPC transport (design D6, spike S3), the default from
//! CoreSimulator 1155.4: one connection to the guest's digitizer service per
//! target, carrying touches with one or two contacts and the edge flag,
//! keyboard usages and Consumer-page buttons.
//!
//! Opening sends the activation probe, a barrier `IndigoKeyboardButtonEvent`
//! with usage 0, and returns at once. Input submitted before the reply is
//! held by the gate and flushed by the reply handler the moment the reply
//! arrives: S3 lost nothing sent before activation, warm or cold, and the
//! 200 ms wait after the reply that idb and iosef keep only delayed the
//! first action. A silent daemon is retried as idb does, 4 s per attempt,
//! five attempts 4 s apart. A peer that goes away closes the session, and the
//! target opens a new one on its next request.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread;
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};

use crate::error::{Error, Result};
use crate::gate::{Gate, State, Submitted};
use crate::gesture::{Phase, Ratio, TouchStep};
use crate::macos::coresim::Device;
use crate::macos::input::Transport;
use crate::macos::xpc::{self, RawConnection, Sim4Symbols, Value};
use crate::model::{Button, Edge, TransportKind};

/// The digitizer feature dtuhidd vends, and every message's
/// `featureIdentifier`.
pub const DIGITIZER: &str = "com.apple.coredevice.feature.remote.hid.digitizer";

/// The waits around the connection, from idb's `DTUHIDTiming`.
mod timing {
    use std::time::Duration;

    /// Deadline for one activation reply; the first message launches the
    /// daemon, so it pays a cold start.
    pub const ACTIVATION: Duration = Duration::from_secs(4);
    /// Between attempts: dtuhidd declares a 10 s minimum runtime, so launchd
    /// throttles respawning one that aborted early.
    pub const RETRY_BACKOFF: Duration = Duration::from_secs(4);
    pub const ATTEMPTS: u32 = 5;
    /// How long a request waits for activation before giving up; covers
    /// every attempt and backoff.
    pub const ACTIVATION_TOTAL: Duration = Duration::from_secs(45);
    /// Time for sent events to reach the guest before the connection goes.
    pub const DRAIN: Duration = Duration::from_millis(80);
    /// Deadline for one send barrier.
    pub const BARRIER: Duration = Duration::from_secs(2);
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unavailable(reason: impl Into<String>) -> Error {
    Error::InputUnavailable {
        transport: TransportKind::Dtuhid,
        reason: reason.into(),
    }
}

/// A dtuhidd session: the gate, the current connection and the attempts
/// that made it.
pub struct Dtuhid {
    inner: Arc<Inner>,
}

struct Inner {
    device: Device,
    symbols: Sim4Symbols,
    gate: Gate<Value>,
    /// The serial queue the connection's events and replies run on.
    queue: DispatchRetained<DispatchQueue>,
    connection: Mutex<Option<Arc<RawConnection>>>,
    /// The attempt whose connection is current; replies and events of
    /// earlier attempts are ignored.
    attempt: AtomicU32,
    connections: AtomicU32,
    last_failure: Mutex<String>,
    changed: Condvar,
    waiting: Mutex<()>,
}

impl Dtuhid {
    /// Connects to the device's digitizer service and sends the activation
    /// probe; returns without waiting for its reply.
    pub fn open(device: &Device) -> Result<Dtuhid> {
        let symbols = Sim4Symbols::resolve().map_err(unavailable)?;
        let inner = Arc::new(Inner {
            device: device.clone(),
            symbols,
            gate: Gate::new(),
            queue: DispatchQueue::new(
                "dev.mobileagent.simulator.dtuhid",
                DispatchQueueAttr::SERIAL,
            ),
            connection: Mutex::new(None),
            attempt: AtomicU32::new(0),
            connections: AtomicU32::new(0),
            last_failure: Mutex::new("no reply yet".into()),
            changed: Condvar::new(),
            waiting: Mutex::new(()),
        });
        inner.connect(1)?;
        let watched = Arc::downgrade(&inner);
        thread::Builder::new()
            .name("simulator-dtuhid-activation".into())
            .spawn(move || watch_activation(&watched))
            .map_err(|error| {
                unavailable(format!("the activation watchdog did not start: {error}"))
            })?;
        Ok(Dtuhid { inner })
    }
}

/// Retries a daemon that does not answer the probe, and closes the gate
/// after the last attempt.
fn watch_activation(inner: &Weak<Inner>) {
    for attempt in 1..=timing::ATTEMPTS {
        let Some(session) = inner.upgrade() else {
            return;
        };
        if session.wait_settled(timing::ACTIVATION) != State::Pending {
            return;
        }
        if attempt == timing::ATTEMPTS {
            let reason = lock(&session.last_failure).clone();
            session.gate.close(format!(
                "dtuhidd did not answer the activation probe in {} attempts: {reason}",
                timing::ATTEMPTS
            ));
            session.notify();
            return;
        }
        if session.wait_settled(timing::RETRY_BACKOFF) != State::Pending {
            return;
        }
        if let Err(error) = session.connect(attempt + 1) {
            session.gate.close(error.to_string());
            session.notify();
            return;
        }
    }
}

impl Inner {
    /// Opens attempt `attempt`'s connection and sends its probe, replacing
    /// the connection of the attempt before.
    fn connect(self: &Arc<Inner>, attempt: u32) -> Result<()> {
        let port = self.device.lookup(DIGITIZER).map_err(|error| {
            unavailable(format!("the guest does not vend {DIGITIZER}: {error}"))
        })?;
        let connection =
            Arc::new(self.symbols.connect(port).ok_or_else(|| {
                unavailable("xpc_connection_create_from_endpoint returned nothing")
            })?);
        connection.set_target_queue(&self.queue);
        let events = {
            let session = Arc::downgrade(self);
            RcBlock::new(move |event: xpc::Object| {
                if let Some(session) = session.upgrade() {
                    // SAFETY: XPC hands the handler a live object.
                    session.on_event(attempt, unsafe { Value::retaining(event) });
                }
            })
        };
        connection.set_event_handler(&events);
        connection.resume();
        self.connections.fetch_add(1, Ordering::SeqCst);
        // The attempt is current before its probe can be answered, and the
        // previous connection's late callbacks are then ignored.
        self.attempt.store(attempt, Ordering::SeqCst);
        let previous = lock(&self.connection).replace(connection.clone());
        if let Some(previous) = previous {
            previous.cancel();
        }
        let session = Arc::downgrade(self);
        connection.send_with_reply(&activation_probe(), &self.queue, move |reply| {
            if let Some(session) = session.upgrade() {
                // SAFETY: XPC hands the handler a live object.
                session.on_reply(attempt, unsafe { Value::retaining(reply) });
            }
        });
        Ok(())
    }

    fn current(&self) -> Option<Arc<RawConnection>> {
        lock(&self.connection).clone()
    }

    /// The probe's reply: a daemon is behind the connection and has
    /// activated its services, so the held input goes out now. An XPC error
    /// reply means XPC answered for a peer that is not there.
    fn on_reply(&self, attempt: u32, reply: Option<Value>) {
        if self.attempt.load(Ordering::SeqCst) != attempt {
            return;
        }
        match reply {
            Some(reply) if !reply.is_error() => {
                if let Some(connection) = self.current() {
                    let _ = self.gate.activate(|message| connection.send(&message));
                }
                self.notify();
            }
            Some(reply) => *lock(&self.last_failure) = reply.error_reason(),
            None => *lock(&self.last_failure) = "a nil reply".into(),
        }
    }

    /// An error on the connection: once active, the peer went away and the
    /// session is over; before, the watchdog retries.
    fn on_event(&self, attempt: u32, event: Option<Value>) {
        let Some(event) = event.filter(Value::is_error) else {
            return;
        };
        if self.attempt.load(Ordering::SeqCst) != attempt {
            return;
        }
        let reason = event.error_reason();
        if self.gate.state() == State::Active {
            self.gate.close(format!("the peer went away: {reason}"));
            self.notify();
        } else {
            *lock(&self.last_failure) = reason;
        }
    }

    fn notify(&self) {
        let _waiting = lock(&self.waiting);
        self.changed.notify_all();
    }

    /// Waits up to `timeout` for the gate to leave Pending.
    fn wait_settled(&self, timeout: Duration) -> State {
        let deadline = Instant::now() + timeout;
        let mut waiting = lock(&self.waiting);
        loop {
            let state = self.gate.state();
            if state != State::Pending {
                return state;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return state;
            };
            waiting = self
                .changed
                .wait_timeout(waiting, remaining.min(Duration::from_millis(50)))
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    fn submit(&self, message: Value) -> Result<Submitted> {
        self.gate
            .submit(message, |message| {
                if let Some(connection) = self.current() {
                    connection.send(&message);
                }
            })
            .map_err(|closed| unavailable(closed.0))
    }
}

impl Transport for Dtuhid {
    fn kind(&self) -> TransportKind {
        TransportKind::Dtuhid
    }

    fn touch(&self, step: &TouchStep) -> Result<Submitted> {
        self.inner.submit(digitizer_event(step))
    }

    fn button(&self, button: Button, down: bool) -> Result<Submitted> {
        self.inner.submit(button_event(button, down))
    }

    fn key(&self, usage: u32, down: bool) -> Result<Submitted> {
        self.inner.submit(keyboard_event(usage, down))
    }

    /// Waits for XPC to have sent everything handed over; says nothing
    /// about the daemon having read it.
    fn flush(&self) -> Result<()> {
        let Some(connection) = self.inner.current() else {
            return Ok(());
        };
        let (tx, rx) = mpsc::channel();
        connection.send_barrier(move || {
            let _ = tx.send(());
        });
        rx.recv_timeout(timing::BARRIER)
            .map_err(|_| unavailable(format!("XPC did not send within {:?}", timing::BARRIER)))
    }

    fn wait_active(&self) -> Result<()> {
        match self.inner.wait_settled(timing::ACTIVATION_TOTAL) {
            State::Active => Ok(()),
            State::Closed => Err(unavailable(self.inner.gate.closed().unwrap_or_default())),
            State::Pending => Err(unavailable(format!(
                "no activation within {:?}",
                timing::ACTIVATION_TOTAL
            ))),
        }
    }

    fn usable(&self) -> bool {
        self.inner.gate.state() != State::Closed
    }

    fn activation(&self) -> Option<Duration> {
        self.inner.gate.activation()
    }

    fn connections(&self) -> u32 {
        self.inner.connections.load(Ordering::SeqCst)
    }
}

impl Drop for Dtuhid {
    /// Lets the last events reach the guest, then closes the connection:
    /// dtuhidd resets its services when its peer disconnects (S3).
    fn drop(&mut self) {
        if self.inner.gate.state() == State::Active {
            thread::sleep(timing::DRAIN);
        }
        self.inner.gate.close("the session was closed");
        if let Some(connection) = lock(&self.inner.connection).take() {
            connection.cancel();
        }
        self.inner.notify();
    }
}

/// The `{messageType, isBarrier, featureIdentifier, payload}` envelope.
fn envelope(message_type: &str, payload: &Value, barrier: bool) -> Value {
    let message = Value::dictionary();
    message.set_string("messageType", message_type);
    message.set_bool("isBarrier", barrier);
    message.set_string("featureIdentifier", DIGITIZER);
    message.set_value("payload", payload);
    message
}

fn contact(ratio: Ratio) -> Value {
    let contact = Value::dictionary();
    contact.set_double("x", ratio.x);
    contact.set_double("y", ratio.y);
    contact
}

/// One or two contacts as an `IndigoDigitizerEvent`. Integers travel as
/// `uint64`, which dtuhidd's decoder requires; a single contact omits
/// `pointTwo`, as idb's optional field does.
fn digitizer_event(step: &TouchStep) -> Value {
    let payload = Value::dictionary();
    if let Some(first) = step.contacts.first() {
        payload.set_value("pointOne", &contact(*first));
    }
    if let Some(second) = step.contacts.get(1) {
        payload.set_value("pointTwo", &contact(*second));
    }
    payload.set_uint64(
        "eventType",
        match step.phase {
            Phase::Down => 0,
            Phase::Move => 1,
            Phase::Up => 2,
        },
    );
    payload.set_uint64(
        "edge",
        match step.edge {
            Edge::None => 0,
            Edge::Top => 1,
            Edge::Left => 2,
            Edge::Bottom => 3,
            Edge::Right => 4,
        },
    );
    // 0 is the main screen.
    payload.set_uint64("target", 0);
    envelope("IndigoDigitizerEvent", &payload, false)
}

/// `HIDButtonState`: 1 down, 2 up; 0 fails dtuhidd's decode.
fn state(down: bool) -> u64 {
    if down { 1 } else { 2 }
}

fn keyboard_event(usage: u32, down: bool) -> Value {
    let payload = Value::dictionary();
    payload.set_uint64("usageCode", u64::from(usage));
    payload.set_uint64("state", state(down));
    envelope("IndigoKeyboardButtonEvent", &payload, false)
}

/// A hardware button by its Consumer page (0x0C) usage, as idb's
/// `SimulatorHIDButtonIdentity` maps them for this transport.
fn button_event(button: Button, down: bool) -> Value {
    let usage = match button {
        Button::Home => 0x40,
        Button::Lock => 0x30,
        Button::VolumeUp => 0xE9,
        Button::VolumeDown => 0xEA,
        Button::Siri => 0xCF,
    };
    let payload = Value::dictionary();
    payload.set_uint64("usagePage", 0x0C);
    payload.set_uint64("usageCode", usage);
    payload.set_uint64("state", state(down));
    envelope("IndigoButtonEvent", &payload, false)
}

/// The probe: a barrier key-up with usage 0, "no event", which the daemon
/// answers without the guest seeing a key.
fn activation_probe() -> Value {
    let payload = Value::dictionary();
    payload.set_uint64("usageCode", 0);
    payload.set_uint64("state", 2);
    envelope("IndigoKeyboardButtonEvent", &payload, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(message: &Value) -> Value {
        message.value("payload").expect("a payload")
    }

    #[test]
    fn a_one_finger_touch_has_one_point_and_uint64_fields() {
        let message = digitizer_event(&TouchStep {
            at: Duration::ZERO,
            phase: Phase::Down,
            contacts: vec![Ratio { x: 0.25, y: 0.75 }],
            edge: Edge::None,
        });
        assert_eq!(
            message.string("messageType").as_deref(),
            Some("IndigoDigitizerEvent")
        );
        assert!(!message.bool("isBarrier"));
        assert_eq!(
            message.string("featureIdentifier").as_deref(),
            Some(DIGITIZER)
        );
        let payload = payload(&message);
        assert_eq!(payload.count(), 4, "no pointTwo");
        let point = payload.value("pointOne").expect("pointOne");
        assert_eq!(
            (point.double("x"), point.double("y")),
            (Some(0.25), Some(0.75))
        );
        assert_eq!(payload.uint64("eventType"), Some(0));
        assert_eq!(payload.uint64("edge"), Some(0));
        assert_eq!(payload.uint64("target"), Some(0));
    }

    #[test]
    fn phases_edges_and_two_contacts_reach_the_wire() {
        for (phase, wire) in [(Phase::Down, 0), (Phase::Move, 1), (Phase::Up, 2)] {
            let message = digitizer_event(&TouchStep {
                at: Duration::ZERO,
                phase,
                contacts: vec![Ratio { x: 0.3, y: 0.5 }, Ratio { x: 0.7, y: 0.5 }],
                edge: Edge::Bottom,
            });
            let payload = payload(&message);
            assert_eq!(payload.uint64("eventType"), Some(wire));
            assert_eq!(payload.uint64("edge"), Some(3));
            let two = payload.value("pointTwo").expect("pointTwo");
            assert_eq!(two.double("x"), Some(0.7));
        }
    }

    #[test]
    fn keys_and_buttons_are_usages_with_a_one_based_state() {
        let down = payload(&keyboard_event(0x04, true));
        assert_eq!(down.uint64("usageCode"), Some(0x04));
        assert_eq!(down.uint64("state"), Some(1));
        assert_eq!(
            payload(&keyboard_event(0x04, false)).uint64("state"),
            Some(2)
        );
        let home = button_event(Button::Home, true);
        assert_eq!(
            home.string("messageType").as_deref(),
            Some("IndigoButtonEvent")
        );
        let home = payload(&home);
        assert_eq!(
            (home.uint64("usagePage"), home.uint64("usageCode")),
            (Some(0x0C), Some(0x40))
        );
        assert_eq!(
            payload(&button_event(Button::Lock, false)).uint64("usageCode"),
            Some(0x30)
        );
    }

    #[test]
    fn the_probe_is_a_barrier_key_up_with_usage_zero() {
        let probe = activation_probe();
        assert!(probe.bool("isBarrier"));
        assert_eq!(
            probe.string("messageType").as_deref(),
            Some("IndigoKeyboardButtonEvent")
        );
        let payload = payload(&probe);
        assert_eq!(payload.uint64("usageCode"), Some(0));
        assert_eq!(payload.uint64("state"), Some(2));
    }
}
