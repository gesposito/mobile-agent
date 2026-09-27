//! The activation gate (spike S3): dtuhidd drops events addressed to a
//! service it has not activated, so a session sends one barrier probe and
//! holds every event until the reply, then flushes them in order the moment
//! it arrives. Sending happens under the gate's lock, both for a flush and
//! for an event submitted after activation, so a later event can never
//! overtake a held one. A pure state machine over a mutex, with no XPC in it.

use std::fmt;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Where the gate is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The probe is out; events are held.
    Pending,
    /// The reply came; events go straight out.
    Active,
    /// The probe went unanswered, or the peer went away.
    Closed,
}

/// Why a submitted event went nowhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closed(pub String);

impl fmt::Display for Closed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the input session is closed: {}", self.0)
    }
}

/// What became of a submitted event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submitted {
    /// Handed to the sender at once.
    Sent,
    /// Held until activation.
    Held,
}

/// Holds events until the transport's activation reply arrives.
pub struct Gate<M> {
    inner: Mutex<Inner<M>>,
}

struct Inner<M> {
    state: State,
    held: Vec<M>,
    reason: String,
    opened: Instant,
    activation: Option<Duration>,
}

impl<M> Gate<M> {
    /// A gate that holds from now, when the probe goes out.
    pub fn new() -> Gate<M> {
        Gate {
            inner: Mutex::new(Inner {
                state: State::Pending,
                held: Vec::new(),
                reason: String::new(),
                opened: Instant::now(),
                activation: None,
            }),
        }
    }

    pub fn state(&self) -> State {
        self.lock().state
    }

    /// How long activation took, once it has.
    pub fn activation(&self) -> Option<Duration> {
        self.lock().activation
    }

    /// How many events are held.
    #[cfg(test)]
    pub fn held(&self) -> usize {
        self.lock().held.len()
    }

    /// Why the gate closed, once it has.
    pub fn closed(&self) -> Option<String> {
        let inner = self.lock();
        (inner.state == State::Closed).then(|| inner.reason.clone())
    }

    /// Offers `message`: sent through `send` under the lock once active,
    /// held while the probe is out, refused once closed.
    pub fn submit(&self, message: M, send: impl FnOnce(M)) -> Result<Submitted, Closed> {
        let mut inner = self.lock();
        match inner.state {
            State::Active => {
                send(message);
                Ok(Submitted::Sent)
            }
            State::Pending => {
                inner.held.push(message);
                Ok(Submitted::Held)
            }
            State::Closed => Err(Closed(inner.reason.clone())),
        }
    }

    /// The reply arrived: every held event goes through `send`, in order,
    /// under the lock. Activating twice sends nothing more.
    pub fn activate(&self, mut send: impl FnMut(M)) -> Result<usize, Closed> {
        let mut inner = self.lock();
        match inner.state {
            State::Closed => Err(Closed(inner.reason.clone())),
            State::Active => Ok(0),
            State::Pending => {
                inner.state = State::Active;
                inner.activation = Some(inner.opened.elapsed());
                let held = std::mem::take(&mut inner.held);
                let count = held.len();
                for message in held {
                    send(message);
                }
                Ok(count)
            }
        }
    }

    /// No reply before the deadline, or the peer went away: drops what is
    /// held. Returns whether this call closed it; the first reason stands.
    pub fn close(&self, reason: impl Into<String>) -> bool {
        let mut inner = self.lock();
        if inner.state == State::Closed {
            return false;
        }
        inner.state = State::Closed;
        inner.reason = reason.into();
        inner.held.clear();
        true
    }

    fn lock(&self) -> MutexGuard<'_, Inner<M>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<M> Default for Gate<M> {
    fn default() -> Gate<M> {
        Gate::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    fn collect<M>(gate: &Gate<M>) -> (Result<usize, Closed>, Vec<M>) {
        let mut sent = Vec::new();
        let result = gate.activate(|message| sent.push(message));
        (result, sent)
    }

    #[test]
    fn events_are_held_while_the_probe_is_out() {
        let gate = Gate::<&str>::new();
        assert_eq!(gate.state(), State::Pending);
        let mut sent = Vec::new();
        assert_eq!(gate.submit("down", |m| sent.push(m)), Ok(Submitted::Held));
        assert_eq!(gate.submit("up", |m| sent.push(m)), Ok(Submitted::Held));
        assert!(sent.is_empty());
        assert_eq!(gate.held(), 2);
        assert_eq!(gate.activation(), None);
    }

    #[test]
    fn activation_flushes_the_held_events_in_order_then_sends_directly() {
        let gate = Gate::<&str>::new();
        for event in ["down", "move", "up"] {
            gate.submit(event, |_| panic!("held")).expect("held");
        }
        let (result, sent) = collect(&gate);
        assert_eq!(result, Ok(3));
        assert_eq!(sent, vec!["down", "move", "up"]);
        assert_eq!(gate.state(), State::Active);
        assert_eq!(gate.held(), 0);
        assert!(gate.activation().is_some());

        let mut direct = Vec::new();
        assert_eq!(gate.submit("next", |m| direct.push(m)), Ok(Submitted::Sent));
        assert_eq!(direct, vec!["next"]);
    }

    #[test]
    fn activating_twice_sends_nothing_more() {
        let gate = Gate::<&str>::new();
        gate.submit("down", |_| {}).expect("held");
        assert_eq!(collect(&gate).0, Ok(1));
        let first = gate.activation().expect("recorded");
        assert_eq!(collect(&gate), (Ok(0), Vec::new()));
        assert_eq!(gate.activation(), Some(first), "the first wait stands");
    }

    #[test]
    fn closing_drops_what_is_held_and_refuses_the_rest() {
        let gate = Gate::<&str>::new();
        gate.submit("down", |_| {}).expect("held");
        assert_eq!(gate.closed(), None);
        assert!(gate.close("no reply within 4s"));
        assert!(!gate.close("the peer went away"), "the first reason stands");
        assert_eq!(gate.closed().as_deref(), Some("no reply within 4s"));
        assert_eq!(gate.state(), State::Closed);
        assert_eq!(gate.held(), 0);
        let reason = Closed("no reply within 4s".into());
        assert_eq!(gate.submit("up", |_| {}), Err(reason.clone()));
        assert_eq!(
            collect(&gate).0,
            Err(reason),
            "a late reply cannot reopen it"
        );
    }

    #[test]
    fn an_active_gate_closes_when_the_peer_goes_away() {
        let gate = Gate::<&str>::new();
        gate.activate(|_| {}).expect("activated");
        assert!(gate.close("the peer went away"));
        assert!(gate.submit("tap", |_| {}).is_err());
    }

    #[test]
    fn a_flush_and_a_concurrent_sender_never_reorder_or_lose_events() {
        let gate = Arc::new(Gate::<usize>::new());
        let log = Arc::new(Mutex::new(Vec::new()));
        let sender = {
            let (gate, log) = (gate.clone(), log.clone());
            thread::spawn(move || {
                for i in 0..500 {
                    gate.submit(i, |m| log.lock().expect("log").push(m))
                        .expect("open");
                }
            })
        };
        thread::sleep(Duration::from_millis(1));
        gate.activate(|m| log.lock().expect("log").push(m))
            .expect("activated");
        sender.join().expect("the sender finished");
        let log = log.lock().expect("log");
        assert_eq!(
            *log,
            (0..500).collect::<Vec<_>>(),
            "every event once, in order"
        );
    }
}
