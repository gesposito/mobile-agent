//! Input for one target, whatever transport carries it: a gesture's timed
//! touch steps, buttons and keys, handed to the target's transport one event
//! at a time, with one request at a time per target.

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::gesture::{Screen, TouchStep};
use crate::macos::coresim::Device;
use crate::model::{Button, Dispatch, Key, TransportKind};

/// The Left Shift, Control, Option and Command usages, held around a key
/// whose modifiers ask for them.
const LEFT_SHIFT: u32 = 0xE1;
const LEFT_CONTROL: u32 = 0xE0;
const LEFT_OPTION: u32 = 0xE2;
const LEFT_COMMAND: u32 = 0xE3;

/// Something that puts input into a Simulator.
pub trait Transport: Send + Sync {
    fn kind(&self) -> TransportKind;

    /// Hands one touch event to the transport.
    fn touch(&self, step: &TouchStep) -> Result<()>;

    /// Presses (`down`) or releases a hardware button.
    fn button(&self, button: Button, down: bool) -> Result<()>;

    /// Presses or releases a key of the USB HID keyboard page.
    fn key(&self, usage: u32, down: bool) -> Result<()>;

    /// Returns once every event handed over so far has been sent.
    fn flush(&self) -> Result<()>;
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

    /// The target's transport, opened with `open` the first time.
    pub fn transport(
        &self,
        open: impl FnOnce(&Device) -> Result<Arc<dyn Transport>>,
    ) -> Result<Arc<dyn Transport>> {
        let mut transport = lock(&self.transport);
        if let Some(transport) = transport.as_ref() {
            return Ok(transport.clone());
        }
        let opened = open(&self.device)?;
        *transport = Some(opened.clone());
        Ok(opened)
    }

    /// Sends a gesture's steps, each at its time after the start.
    pub fn touch(&self, transport: &dyn Transport, plan: &[TouchStep]) -> Result<Dispatch> {
        let _serial = lock(&self.serial);
        let start = Instant::now();
        let mut dispatched = None;
        for step in plan {
            sleep_until(start + step.at);
            transport.touch(step)?;
            dispatched.get_or_insert_with(|| start.elapsed());
        }
        transport.flush()?;
        Ok(report(transport, start, dispatched, plan.len()))
    }

    /// A short press of a hardware button.
    pub fn button(&self, transport: &dyn Transport, button: Button) -> Result<Dispatch> {
        let _serial = lock(&self.serial);
        let start = Instant::now();
        transport.button(button, true)?;
        let dispatched = start.elapsed();
        transport.button(button, false)?;
        transport.flush()?;
        Ok(report(transport, start, Some(dispatched), 2))
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
        let start = Instant::now();
        let mut dispatched = None;
        for &usage in modifiers.iter().chain([&key.usage]) {
            transport.key(usage, true)?;
            dispatched.get_or_insert_with(|| start.elapsed());
        }
        for &usage in [&key.usage].into_iter().chain(modifiers.iter().rev()) {
            transport.key(usage, false)?;
        }
        transport.flush()?;
        Ok(report(
            transport,
            start,
            dispatched,
            2 * (modifiers.len() + 1),
        ))
    }
}

fn report(
    transport: &dyn Transport,
    start: Instant,
    dispatched: Option<Duration>,
    events: usize,
) -> Dispatch {
    Dispatch {
        transport: transport.kind(),
        activation_wait: None,
        dispatched: dispatched.unwrap_or_default(),
        completed: start.elapsed(),
        events,
    }
}

fn sleep_until(when: Instant) {
    if let Some(wait) = when.checked_duration_since(Instant::now()) {
        thread::sleep(wait);
    }
}
