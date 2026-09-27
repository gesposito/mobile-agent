//! Input for one target, whatever transport carries it: a gesture's timed
//! touch steps, buttons and keys, handed to the target's transport one event
//! at a time, with one request at a time per target.

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::gate::Submitted;
use crate::gesture::{Screen, TouchStep};
use crate::macos::coresim::Device;
use crate::model::{Button, Dispatch, Key, TransportKind};

/// The Left Shift, Control, Option and Command usages, held around a key
/// whose modifiers ask for them.
const LEFT_SHIFT: u32 = 0xE1;
const LEFT_CONTROL: u32 = 0xE0;
const LEFT_OPTION: u32 = 0xE2;
const LEFT_COMMAND: u32 = 0xE3;

/// Something that puts input into a Simulator. A transport that must
/// activate first holds what it is handed until then.
pub trait Transport: Send + Sync {
    fn kind(&self) -> TransportKind;

    /// Hands one touch event to the transport.
    fn touch(&self, step: &TouchStep) -> Result<Submitted>;

    /// Presses (`down`) or releases a hardware button.
    fn button(&self, button: Button, down: bool) -> Result<Submitted>;

    /// Presses or releases a key of the USB HID keyboard page.
    fn key(&self, usage: u32, down: bool) -> Result<Submitted>;

    /// Returns once every event handed over so far has been sent.
    fn flush(&self) -> Result<()>;

    /// Returns once the transport is active, having sent what it held.
    fn wait_active(&self) -> Result<()>;

    /// False once the session is over; the target then opens another.
    fn usable(&self) -> bool;

    /// How long activation took, once done; `None` for a transport without.
    fn activation(&self) -> Option<Duration>;

    /// Connections opened for this session.
    fn connections(&self) -> u32;
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A booted device the driver has sent requests to: its handle, its screen
/// geometry and its input session, opened on first use and kept.
pub struct Target {
    pub device: Device,
    pub screen: Screen,
    transport: Mutex<Option<Arc<dyn Transport>>>,
    /// Held for the length of each input request, so that one target's
    /// gestures never interleave.
    serial: Mutex<()>,
}

impl Target {
    pub fn new(device: Device, screen: Screen) -> Target {
        Target {
            device,
            screen,
            transport: Mutex::new(None),
            serial: Mutex::new(()),
        }
    }

    /// The target's transport, opened with `open` the first time and again
    /// after a session ends.
    pub fn transport(
        &self,
        open: impl FnOnce(&Device) -> Result<Arc<dyn Transport>>,
    ) -> Result<Arc<dyn Transport>> {
        let mut transport = lock(&self.transport);
        if let Some(transport) = transport.as_ref().filter(|t| t.usable()) {
            return Ok(transport.clone());
        }
        // A finished session is dropped before its successor connects.
        transport.take();
        let opened = open(&self.device)?;
        *transport = Some(opened.clone());
        Ok(opened)
    }

    /// The transport already open, if any.
    pub fn current_transport(&self) -> Option<Arc<dyn Transport>> {
        lock(&self.transport).clone()
    }

    /// Sends a gesture's steps, each at its time after the start. An event
    /// the transport held for its activation waits for it, and the rest of
    /// the gesture keeps its timing from then on.
    pub fn touch(&self, transport: &dyn Transport, plan: &[TouchStep]) -> Result<Dispatch> {
        let _serial = lock(&self.serial);
        let mut request = Request::start(transport);
        for step in plan {
            request.sleep_until(step.at);
            let submitted = transport.touch(step)?;
            request.submitted(submitted, step.at)?;
        }
        request.finish(plan.len())
    }

    /// A short press of a hardware button.
    pub fn button(&self, transport: &dyn Transport, button: Button) -> Result<Dispatch> {
        let _serial = lock(&self.serial);
        let mut request = Request::start(transport);
        for down in [true, false] {
            let submitted = transport.button(button, down)?;
            request.submitted(submitted, Duration::ZERO)?;
        }
        request.finish(2)
    }

    /// A key press with its modifiers held around it.
    pub fn key(&self, transport: &dyn Transport, key: Key) -> Result<Dispatch> {
        let _serial = lock(&self.serial);
        let modifiers: Vec<u32> = [
            (key.modifiers.command, LEFT_COMMAND),
            (key.modifiers.control, LEFT_CONTROL),
            (key.modifiers.option, LEFT_OPTION),
            (key.modifiers.shift, LEFT_SHIFT),
        ]
        .into_iter()
        .filter_map(|(held, usage)| held.then_some(usage))
        .collect();
        let presses = modifiers
            .iter()
            .chain([&key.usage])
            .map(|&usage| (usage, true));
        let releases = [&key.usage]
            .into_iter()
            .chain(modifiers.iter().rev())
            .map(|&usage| (usage, false));
        let mut request = Request::start(transport);
        for (usage, down) in presses.chain(releases) {
            let submitted = transport.key(usage, down)?;
            request.submitted(submitted, Duration::ZERO)?;
        }
        request.finish(2 * (modifiers.len() + 1))
    }
}

/// The timing of one input request.
struct Request<'a> {
    transport: &'a dyn Transport,
    requested: Instant,
    /// What event times count from: the request, or the activation that
    /// released held input.
    base: Instant,
    dispatched: Option<Duration>,
    activation_wait: Option<Duration>,
}

impl<'a> Request<'a> {
    fn start(transport: &'a dyn Transport) -> Request<'a> {
        let now = Instant::now();
        Request {
            transport,
            requested: now,
            base: now,
            dispatched: None,
            activation_wait: None,
        }
    }

    fn sleep_until(&self, at: Duration) {
        if let Some(wait) = (self.base + at).checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }

    /// Records an event handed over at `at`. One the transport held waits
    /// for the activation that sends it.
    fn submitted(&mut self, submitted: Submitted, at: Duration) -> Result<()> {
        if submitted == Submitted::Held {
            self.transport.wait_active()?;
            let now = Instant::now();
            self.activation_wait = Some(now - self.requested);
            self.base = now.checked_sub(at).unwrap_or(now);
        }
        let requested = self.requested;
        self.dispatched.get_or_insert_with(|| requested.elapsed());
        Ok(())
    }

    fn finish(self, events: usize) -> Result<Dispatch> {
        self.transport.flush()?;
        Ok(Dispatch {
            transport: self.transport.kind(),
            activation_wait: self.activation_wait,
            dispatched: self.dispatched.unwrap_or_default(),
            completed: self.requested.elapsed(),
            events,
        })
    }
}
