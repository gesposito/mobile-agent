//! The mirror: one target's replica and the questions it answers.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use protocol::message::TextMethod;
use protocol::snapshot::{NodeFlags, Truncation, WindowFlags, WindowType};

use crate::backend::{Backend, BackendInfo, Input, NodeHandle, Owner, ReadRequest};
use crate::delta::{self, DeltaContext, DeltaSnapshot, DeltaState, RefEntry};
use crate::diff::Diff;
use crate::error::{MirrorError, ResyncCause, StaleCause};
use crate::geometry::{Point, Rect};
use crate::refs::{Lookup, Ref, RefTable};
use crate::render::{self, Line};
use crate::replica::{RNode, Replica, Win};
use crate::settle::{Settle, SettleOptions, Signal};
use crate::snapshot::{
    Element, ElementId, RefreshMode, Role, Snapshot, SnapshotOptions, TreeDelta, WindowSource,
    WindowSummary,
};
use crate::verdict::{Degradation, Verdict};
use crate::view::{self, Facts, Hidden, View};
use crate::wait::{Condition, Selector, SnapshotSummary, WaitOutcome};

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
    /// The center of the element a ref names.
    Ref(Ref),
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
    /// Text for the element `into` names, or for the focused field.
    Text {
        text: String,
        into: Option<Ref>,
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
    refs: RefTable,
    /// Refs the last snapshot listed, for its successor's removed refs.
    listed: Vec<Ref>,
    /// The last delta snapshot's listing.
    delta: Option<DeltaState>,
}

impl Mirror {
    /// A mirror over `backend`. Nothing is read until the replica is first
    /// needed.
    pub fn new(backend: Backend, config: MirrorConfig) -> Mirror {
        Mirror {
            backend,
            config,
            replica: Replica::new(),
            refs: RefTable::new(),
            listed: Vec::new(),
            delta: None,
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

    /// Performs the reads that are due. A read from another backend than
    /// the replica's is set aside and the replica rebuilt from a full read.
    fn refresh(&mut self) -> Result<(), MirrorError> {
        if self.replica.disconnected.is_some() {
            return Ok(());
        }
        // A read can make another one due at once, such as a full read
        // after a scoped one found another backend; a few rounds suffice.
        for _ in 0..4 {
            let started = self.now();
            let Some(request) = self.replica.due_read(started, &self.config) else {
                return Ok(());
            };
            self.read(&request, started)?;
        }
        Ok(())
    }

    /// Performs `request`, started at `started`, and takes it into the
    /// replica unless it comes from another backend, which starts the
    /// replica over instead.
    fn read(&mut self, request: &ReadRequest, started: Instant) -> Result<(), MirrorError> {
        let read = self.backend.tree.read(request)?;
        let finished = self.now();
        if !self.replica.fresh && self.replica.backend_differs(&read.backend) {
            self.replica
                .resync(ResyncCause::Backend(read.backend.clone()));
            return Ok(());
        }
        self.replica.integrate(read, request, started, finished);
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
    /// Listed elements that are actionable, or content with a name, get
    /// refs.
    pub fn snapshot(&mut self, options: &SnapshotOptions) -> Result<Snapshot, MirrorError> {
        self.pump()?;
        let now = self.now();
        let view = View::build(&self.replica);
        let found = view::hints(&self.replica, &view);
        let lines = render::listing(&self.replica, &view, &found, options);
        let mut elements = Vec::new();
        let mut text = Vec::with_capacity(lines.len());
        let mut listed = Vec::new();
        for line in lines {
            match line {
                Line::Element { w, i, depth } => {
                    let window = &self.replica.windows[w];
                    let node = &window.nodes[i];
                    let facts = &view.windows[w][i];
                    let generation = self.replica.generation;
                    let reference = facts.bears_ref.then(|| {
                        self.refs
                            .assign(node.element, facts.role, &facts.name, generation)
                    });
                    if let Some(reference) = reference {
                        listed.push((facts.signature, reference));
                    }
                    let mut listed_element = element(window, node, facts, reference);
                    listed_element.depth = depth;
                    text.push(render::element_line(&listed_element));
                    elements.push(listed_element);
                }
                Line::Keyboard { w } => {
                    let keyboard = keyboard(&self.replica.windows[w], self.replica.scale);
                    text.push(render::element_line(&keyboard));
                    elements.push(keyboard);
                }
                Line::Hint { index, depth } => {
                    text.push(render::hint_line(&found[index].2, depth));
                }
            }
        }
        let replica = &self.replica;
        self.refs.prune(|e| replica.locate(e).is_some());
        let reasons = self.degradations(&listed);
        let refs: Vec<Ref> = listed.iter().map(|(_, r)| *r).collect();
        let mut removed_refs: Vec<Ref> = self
            .listed
            .iter()
            .filter(|r| !refs.contains(r))
            .copied()
            .collect();
        removed_refs.sort_unstable();
        self.listed = refs;
        Ok(Snapshot {
            revision: self.replica.revision,
            taken_at: now,
            options: *options,
            scale: self.replica.scale,
            foreground: self.foreground(),
            verdict: Verdict {
                backend: self
                    .replica
                    .backend
                    .clone()
                    .unwrap_or_else(|| BackendInfo::new("none", false)),
                degraded: !reasons.is_empty(),
                reasons,
            },
            windows: self.summaries(now),
            elements,
            hints: found.into_iter().map(|(_, _, hint)| hint).collect(),
            removed_refs,
            text: if text.is_empty() {
                render::empty(options).to_owned()
            } else {
                text.join("\n")
            },
        })
    }

    /// Why the replica's data is degraded, in a fixed order: the backend,
    /// truncation, sparse application windows, sibling collisions among
    /// the `listed` refs, agent warnings, disconnection.
    fn degradations(&self, listed: &[(u64, Ref)]) -> Vec<Degradation> {
        let replica = &self.replica;
        let mut reasons = Vec::new();
        if let Some(backend) = replica.backend.as_ref().filter(|b| b.fallback) {
            reasons.push(Degradation::FallbackBackend {
                name: backend.name.clone(),
            });
        }
        let truncated = replica.truncated;
        if !truncated.is_empty() {
            reasons.push(Degradation::Truncated {
                depth: truncated.contains(Truncation::DEPTH),
                nodes: truncated.contains(Truncation::NODES),
                windows: truncated.contains(Truncation::WINDOWS),
            });
        }
        for window in &replica.windows {
            if window.kind == WindowType::APPLICATION
                && window.owner == Owner::Target
                && window.nodes.len() <= 1
            {
                reasons.push(Degradation::SparseTree { window: window.id });
            }
        }
        reasons.extend(collisions(listed));
        reasons.extend(
            replica
                .warnings
                .iter()
                .cloned()
                .map(Degradation::AgentWarning),
        );
        if let Some(reason) = &replica.disconnected {
            reasons.push(Degradation::Disconnected(reason.clone()));
        }
        reasons
    }

    /// The element `reference` names, as the replica shows it now.
    pub fn resolve(&mut self, reference: Ref) -> Result<Element, MirrorError> {
        self.pump()?;
        let (w, i) = self.locate_ref(reference)?;
        let window = &self.replica.windows[w];
        let facts = view::facts_of(&self.replica, w, i);
        Ok(element(window, &window.nodes[i], &facts, Some(reference)))
    }

    /// Where the element `reference` names is in the replica.
    fn locate_ref(&self, reference: Ref) -> Result<(usize, usize), MirrorError> {
        match self.refs.lookup(reference) {
            Lookup::Unknown => Err(MirrorError::UnknownRef(reference.to_string())),
            Lookup::Forgotten => Err(MirrorError::StaleRef {
                reference,
                cause: StaleCause::Removed,
                was: None,
            }),
            Lookup::Issued(issued) => {
                let was = Some((issued.role, issued.name.clone()));
                if issued.generation < self.replica.generation {
                    let index = usize::try_from(issued.generation).unwrap_or(usize::MAX);
                    let cause = self
                        .replica
                        .resyncs
                        .get(index)
                        .cloned()
                        .unwrap_or(ResyncCause::Reconnected);
                    return Err(MirrorError::StaleRef {
                        reference,
                        cause: StaleCause::Resync(cause),
                        was,
                    });
                }
                self.replica
                    .locate(issued.element)
                    .ok_or(MirrorError::StaleRef {
                        reference,
                        cause: StaleCause::Removed,
                        was,
                    })
            }
        }
    }

    /// The point an action aims at, in device points: a ref's element must
    /// be on screen and not covered.
    fn aim(&mut self, target: &Target) -> Result<Point, MirrorError> {
        let reference = match target {
            Target::Point(point) => return Ok(*point),
            Target::Ref(reference) => *reference,
        };
        self.pump()?;
        let (w, i) = self.locate_ref(reference)?;
        let view = View::build(&self.replica);
        let facts = &view.windows[w][i];
        if facts.hidden == Hidden::Covered {
            return Err(MirrorError::Covered {
                reference,
                by: view.modal.clone().unwrap_or_else(|| "modal".to_owned()),
            });
        }
        if !facts.visible {
            return Err(MirrorError::Offscreen { reference });
        }
        Ok(facts.rect.center())
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
        self.drain();
        if let Some(reason) = &self.replica.disconnected {
            return Err(MirrorError::Disconnected(reason.clone()));
        }
        let (input, at) = match action {
            Action::Tap { target, hold } => {
                let at = self.aim(target)?;
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
            Action::Text { text, into, method } => {
                let node = match into {
                    Some(reference) => {
                        self.pump()?;
                        let (w, i) = self.locate_ref(*reference)?;
                        let window = &self.replica.windows[w];
                        Some(NodeHandle {
                            window: window.id,
                            node: window.nodes[i].node_id,
                        })
                    }
                    None => None,
                };
                (
                    Input::Text {
                        text: text.clone(),
                        node,
                        method: *method,
                    },
                    None,
                )
            }
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

    /// Waits until neither the tree nor the frames have changed for the
    /// quiet window, or until the maximum passes. The window counts from
    /// the last change or the last injected input, whichever is later; a
    /// screen quiet for that long is settled at once. Tree changes are the
    /// change events as they arrive and the deltas of re-reads; without
    /// frames to watch, windows that send no change events are re-read every
    /// minimum re-read interval meanwhile.
    pub fn settle(&mut self, options: &SettleOptions) -> Result<Settle, MirrorError> {
        let start = self.now();
        let deadline = start + options.max;
        let tree_before = self.replica.tree_changes;
        let frames_before = self.replica.frame_changes;
        let blind = self.backend.frames.is_none();
        let every = self.config.min_reread_interval;
        self.drain();
        loop {
            if blind {
                self.reread_quietly(every)?;
            }
            let now = self.now();
            let tree = options
                .signals
                .tree()
                .then_some(self.replica.last_tree_change)
                .flatten();
            let frames = options
                .signals
                .frames()
                .then_some(self.replica.last_frame_change)
                .flatten();
            let last = [tree, frames, self.replica.last_action]
                .into_iter()
                .flatten()
                .max();
            let quiet_until = last.map(|at| at + options.quiet);
            let settled = quiet_until.is_none_or(|until| now >= until);
            if settled || now >= deadline {
                let since = now.checked_sub(options.quiet).unwrap_or(start);
                let mut still_changing = Vec::new();
                if !settled {
                    if tree.is_some_and(|at| at > since) {
                        still_changing.push(Signal::Tree);
                    }
                    if frames.is_some_and(|at| at > since) {
                        still_changing.push(Signal::Frames);
                    }
                }
                return Ok(Settle {
                    settled,
                    elapsed: now - start,
                    still_changing,
                    tree_changes: self.replica.tree_changes - tree_before,
                    frame_changes: self.replica.frame_changes - frames_before,
                });
            }
            let mut wake = quiet_until.map_or(deadline, |until| until.min(deadline));
            if blind && let Some(due) = self.replica.next_reread(every) {
                wake = wake.min(due);
            }
            if let Some(item) = self.backend.events.next(wake.max(now)) {
                self.replica.apply(item);
            }
        }
    }

    /// Re-reads the windows that send no change events when they are due
    /// every `every`, leaving other windows alone.
    fn reread_quietly(&mut self, every: Duration) -> Result<(), MirrorError> {
        if self.replica.disconnected.is_some() {
            return Ok(());
        }
        let started = self.now();
        if self.replica.need_full.is_some() {
            return self.refresh();
        }
        if let Some(request) = self.replica.due_reread(started, every) {
            self.read(&request, started)?;
        }
        Ok(())
    }

    /// Waits until an element matching `selector` meets `condition`,
    /// re-evaluating whenever the target reports something or a re-read
    /// falls due, and fails at `timeout` with a summary of the screen.
    /// Only elements on screen and not covered match.
    pub fn wait(
        &mut self,
        selector: &Selector,
        condition: Condition,
        timeout: Duration,
    ) -> Result<WaitOutcome, MirrorError> {
        let start = self.now();
        let deadline = start + timeout;
        loop {
            self.pump()?;
            let now = self.now();
            let found = self.first_match(selector, condition == Condition::Enabled);
            let holds = match condition {
                Condition::Disappear => found.is_none(),
                Condition::Appear | Condition::Enabled => found.is_some(),
            };
            if holds {
                return Ok(WaitOutcome {
                    elapsed: now - start,
                    element: found,
                });
            }
            if now >= deadline {
                let summary = self.summary()?;
                return Err(MirrorError::WaitTimeout {
                    selector: selector.clone(),
                    condition,
                    waited: now - start,
                    summary: Box::new(summary),
                });
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

    /// The first element on screen, not covered, that matches `selector`,
    /// and is enabled when `enabled` says so.
    fn first_match(&self, selector: &Selector, enabled: bool) -> Option<Element> {
        let view = View::build(&self.replica);
        for (w, window) in self.replica.windows.iter().enumerate() {
            for (i, node) in window.nodes.iter().enumerate() {
                let facts = &view.windows[w][i];
                if !facts.visible || facts.hidden != Hidden::No {
                    continue;
                }
                if enabled && !node.flags.contains(NodeFlags::ENABLED) {
                    continue;
                }
                let reference = self.refs.of(node.element);
                if selector.matches(
                    &facts.name,
                    facts.value.as_deref(),
                    node.identifier.as_deref(),
                    facts.role,
                    reference,
                ) {
                    return Some(element(window, node, facts, reference));
                }
            }
        }
        None
    }

    /// A summary of the screen as an interactive snapshot shows it.
    fn summary(&mut self) -> Result<SnapshotSummary, MirrorError> {
        const SHOWN: usize = 20;
        let snapshot = self.snapshot(&SnapshotOptions::interactive())?;
        let lines: Vec<String> = snapshot.text.lines().map(str::to_owned).collect();
        let age = self.replica.updated_at.map_or(Duration::ZERO, |read| {
            snapshot.taken_at.saturating_duration_since(read)
        });
        Ok(SnapshotSummary {
            revision: snapshot.revision,
            age,
            windows: snapshot.windows.len(),
            elements: snapshot.refs().len(),
            omitted: lines.len().saturating_sub(SHOWN),
            lines: lines.into_iter().take(SHOWN).collect(),
            verdict: snapshot.verdict,
        })
    }

    /// A new snapshot with `baseline`'s options, diffed against `baseline`.
    pub fn diff(&mut self, baseline: &Snapshot) -> Result<Diff, MirrorError> {
        let current = self.snapshot(&baseline.options)?;
        Ok(Diff::between(baseline, &current))
    }

    /// A snapshot as agent-browser's `snapshot --delta` returns it; `full`
    /// forces the whole listing, as `--full` does. The listing is compared
    /// with the previous delta snapshot's when the options, the replica's
    /// generation and the foreground owner are the same.
    pub fn delta_snapshot(
        &mut self,
        options: &SnapshotOptions,
        full: bool,
    ) -> Result<DeltaSnapshot, MirrorError> {
        let snapshot = self.snapshot(options)?;
        let mut refs: Vec<RefEntry> = snapshot
            .elements
            .iter()
            .filter_map(|e| {
                e.reference.map(|reference| RefEntry {
                    reference,
                    role: e.role,
                    name: e.name.clone(),
                })
            })
            .collect();
        refs.sort_by_key(|entry| entry.reference);
        let context = DeltaContext {
            options: *options,
            generation: self.replica.generation,
            foreground: snapshot.foreground.clone(),
        };
        let revision = self.delta.as_ref().map_or(1, |d| d.revision + 1);
        let result = match &self.delta {
            Some(previous) if !full && previous.context == context => {
                delta::against(previous, revision, &snapshot.text, &refs)
            }
            _ => DeltaSnapshot::Full {
                revision,
                tree: snapshot.text.clone(),
                refs: refs.clone(),
            },
        };
        self.delta = Some(DeltaState {
            revision,
            context,
            tree: snapshot.text,
            refs,
        });
        Ok(result)
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

/// An element as snapshots list it.
fn element(window: &Win, node: &RNode, facts: &Facts, reference: Option<Ref>) -> Element {
    Element {
        id: node.element,
        reference,
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
    }
}

/// The groups of listed refs that share a signature: identical siblings
/// that only their position tells apart.
fn collisions(listed: &[(u64, Ref)]) -> Vec<Degradation> {
    let mut groups: Vec<Vec<Ref>> = Vec::new();
    let mut by_signature: HashMap<u64, usize> = HashMap::new();
    for &(signature, reference) in listed {
        match by_signature.get(&signature) {
            Some(&group) => groups[group].push(reference),
            None => {
                by_signature.insert(signature, groups.len());
                groups.push(vec![reference]);
            }
        }
    }
    groups
        .into_iter()
        .filter(|refs| refs.len() > 1)
        .map(|refs| Degradation::SiblingCollision { refs })
        .collect()
}

/// A keyboard window as one listed element.
fn keyboard(window: &Win, scale: f64) -> Element {
    let root = window.nodes.first();
    Element {
        id: root.map_or(ElementId(0), |n| n.element),
        reference: None,
        role: Role::Keyboard,
        name: String::new(),
        identifier: None,
        value: None,
        bounds: Rect::from_pixels(window.bounds, scale),
        enabled: true,
        focused: false,
        selected: false,
        checked: None,
        actionable: false,
        window: window.id,
        depth: 0,
        class: root.map_or_else(String::new, |n| n.class.to_string()),
    }
}
