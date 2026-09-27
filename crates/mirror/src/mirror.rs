//! The mirror: one target's replica and the questions it answers.

use std::time::{Duration, Instant};

use protocol::message::TextMethod;
use protocol::snapshot::{NodeFlags, WindowFlags, WindowType};

use crate::backend::{Backend, Input, Owner};
use crate::error::MirrorError;
use crate::geometry::{Point, Rect};
use crate::replica::Replica;
use crate::snapshot::{
    Element, RefreshMode, Snapshot, SnapshotOptions, TreeDelta, WindowSource, WindowSummary,
};
use crate::view::View;

/// How often a window that sends no change events is re-read when nothing
/// else re-reads it.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The shortest time from the end of a read of a window that sends no change
/// events to a re-read that frame changes trigger (design D5: re-read after
/// frames change and once more when they stop, bounded by a minimum
/// interval).
pub const DEFAULT_MIN_REREAD_INTERVAL: Duration = Duration::from_millis(200);

/// Tuning for one mirror.
#[derive(Clone, Debug, PartialEq)]
pub struct MirrorConfig {
    pub poll_interval: Duration,
    pub min_reread_interval: Duration,
}

impl Default for MirrorConfig {
    fn default() -> MirrorConfig {
        MirrorConfig {
            poll_interval: DEFAULT_POLL_INTERVAL,
            min_reread_interval: DEFAULT_MIN_REREAD_INTERVAL,
        }
    }
}

/// Where an action lands.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A position in device points.
    Point(Point),
}

/// An action the mirror injects through the backend's input sink.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A tap, or a long press when `hold` is present.
    Tap {
        target: Target,
        hold: Option<Duration>,
    },
    Swipe {
        from: Point,
        to: Point,
        duration: Duration,
    },
    Key {
        code: u32,
        meta: Option<u32>,
    },
    /// Text for the focused field.
    Text {
        text: String,
        method: TextMethod,
    },
}

/// What happened to an action.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionOutcome {
    /// Whether the platform accepted the input.
    pub accepted: bool,
    /// Where a tap landed, in device points.
    pub at: Option<Point>,
    /// For text, the way it went in.
    pub method: Option<TextMethod>,
}

/// One target's live replica.
pub struct Mirror {
    backend: Backend,
    config: MirrorConfig,
    replica: Replica,
}

impl Mirror {
    /// A mirror over `backend`. Nothing is read until the replica is first
    /// needed.
    pub fn new(backend: Backend, config: MirrorConfig) -> Mirror {
        Mirror {
            backend,
            config,
            replica: Replica::new(),
        }
    }

    pub fn config(&self) -> &MirrorConfig {
        &self.config
    }

    fn now(&self) -> Instant {
        self.backend.clock.now()
    }

    /// Applies what the target reported so far and performs the reads that
    /// are due, without waiting.
    pub fn pump(&mut self) -> Result<(), MirrorError> {
        self.drain();
        self.refresh()
    }

    /// Applies every item already queued.
    fn drain(&mut self) {
        let now = self.now();
        while let Some(item) = self.backend.events.next(now) {
            self.replica.apply(item);
        }
    }

    /// Performs the reads that are due.
    fn refresh(&mut self) -> Result<(), MirrorError> {
        // A read can make another one due at once, such as a full read
        // after a scoped one found another backend; a few rounds suffice.
        for _ in 0..4 {
            let started = self.now();
            let Some(request) = self.replica.due_read(started, &self.config) else {
                return Ok(());
            };
            let read = self.backend.tree.read(&request)?;
            let finished = self.now();
            self.replica.integrate(read, &request, started, finished);
        }
        Ok(())
    }

    /// Keeps the replica current until `deadline`: applies items as they
    /// arrive and performs reads as they fall due.
    pub fn run_until(&mut self, deadline: Instant) -> Result<(), MirrorError> {
        loop {
            self.pump()?;
            let now = self.now();
            if now >= deadline {
                return Ok(());
            }
            let wake = self
                .replica
                .next_due(&self.config)
                .map_or(deadline, |due| due.min(deadline))
                .max(now);
            if let Some(item) = self.backend.events.next(wake) {
                self.replica.apply(item);
            }
        }
    }

    /// When a read of a window that sends no change events falls due next:
    /// its poll, or a re-read after frames changed.
    pub fn next_wakeup(&self) -> Option<Instant> {
        self.replica.next_due(&self.config)
    }

    /// A snapshot of the replica, acquiring it first when none exists.
    pub fn snapshot(&mut self, options: &SnapshotOptions) -> Result<Snapshot, MirrorError> {
        self.pump()?;
        let now = self.now();
        let view = View::build(&self.replica);
        let mut elements = Vec::new();
        for (w, window) in self.replica.windows.iter().enumerate() {
            for (i, node) in window.nodes.iter().enumerate() {
                let facts = &view.windows[w][i];
                if !facts.visible {
                    continue;
                }
                elements.push(Element {
                    id: node.element,
                    role: facts.role,
                    name: facts.name.clone(),
                    identifier: node.identifier.clone(),
                    value: facts.value.clone(),
                    bounds: facts.rect,
                    enabled: node.flags.contains(NodeFlags::ENABLED),
                    focused: node.flags.contains(NodeFlags::FOCUSED),
                    selected: node.flags.contains(NodeFlags::SELECTED),
                    checked: facts.checked,
                    actionable: facts.actionable,
                    window: window.id,
                    depth: node.depth,
                    class: node.class.to_string(),
                });
            }
        }
        Ok(Snapshot {
            revision: self.replica.revision,
            taken_at: now,
            options: *options,
            scale: self.replica.scale,
            foreground: self.foreground(),
            windows: self.summaries(now),
            elements,
        })
    }

    /// Who owns the foreground: the owner of the active window, else of the
    /// top-most application window.
    fn foreground(&self) -> Owner {
        let windows = &self.replica.windows;
        windows
            .iter()
            .find(|w| w.flags.contains(WindowFlags::ACTIVE))
            .or_else(|| windows.iter().find(|w| w.kind == WindowType::APPLICATION))
            .map_or(Owner::Target, |w| w.owner.clone())
    }

    fn summaries(&self, now: Instant) -> Vec<WindowSummary> {
        let scale = self.replica.scale;
        self.replica
            .windows
            .iter()
            .map(|w| WindowSummary {
                id: w.id,
                kind: w.kind,
                layer: w.layer,
                package: w.package.clone(),
                title: w.title.clone(),
                owner: w.owner.clone(),
                bounds: Rect::from_pixels(w.bounds, scale),
                modal: w.modal,
                source: WindowSource {
                    mode: if w.reread() {
                        RefreshMode::Reread
                    } else {
                        RefreshMode::Events
                    },
                    reason: w.reason,
                    read_cost: w.read_finished.saturating_duration_since(w.read_started),
                    age: now.saturating_duration_since(w.read_finished),
                },
            })
            .collect()
    }

    /// Injects `action` and marks the windows that send no change events
    /// for a re-read, which happens when the tree is next needed or at the
    /// next poll.
    pub fn act(&mut self, action: &Action) -> Result<ActionOutcome, MirrorError> {
        let (input, at) = match action {
            Action::Tap { target, hold } => {
                let at = match target {
                    Target::Point(point) => *point,
                };
                (Input::Tap { at, hold: *hold }, Some(at))
            }
            Action::Swipe { from, to, duration } => (
                Input::Swipe {
                    from: *from,
                    to: *to,
                    duration: *duration,
                },
                None,
            ),
            Action::Key { code, meta } => (
                Input::Key {
                    code: *code,
                    meta: *meta,
                },
                None,
            ),
            Action::Text { text, method } => (
                Input::Text {
                    text: text.clone(),
                    node: None,
                    method: *method,
                },
                None,
            ),
        };
        let ack = self.backend.input.perform(&input)?;
        let now = self.now();
        self.replica.note_action(now);
        Ok(ActionOutcome {
            accepted: ack.accepted,
            at,
            method: ack.method,
        })
    }

    /// The replica's revision: it grows whenever a read changes the replica.
    pub fn revision(&self) -> u64 {
        self.replica.revision
    }

    /// The most recent deltas, oldest first.
    pub fn deltas(&self) -> &[TreeDelta] {
        &self.replica.deltas
    }

    /// The replica as text, one line per window and node, for debugging and
    /// for comparing replicas.
    pub fn dump(&self) -> String {
        self.replica.dump()
    }
}
