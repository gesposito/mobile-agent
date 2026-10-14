//! The mirror: one target's replica and the questions it answers.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use protocol::message::TextMethod;
use protocol::snapshot::{Bounds, NodeFlags, Truncation, WindowFlags, WindowType};

use crate::backend::{
    Backend, BackendInfo, EncodeRequest, Input, NodeHandle, Owner, ReadReason, ReadRequest, Release,
};
use crate::delta::{self, DeltaContext, DeltaSnapshot, DeltaState, RefEntry};
use crate::diff::Diff;
use crate::error::{MirrorError, ResyncCause, StaleCause};
use crate::frame::{self, FrameImage, FrameRequest, TreeSkew};
use crate::geometry::{Point, Rect};
use crate::refs::{self, Lookup, Ref, RefTable};
use crate::render::{self, Line};
use crate::replica::{RNode, Replica, Win};
use crate::settle::{Settle, SettleOptions, Signal};
use crate::snapshot::{
    Element, ElementId, RefreshMode, Role, Snapshot, SnapshotOptions, TreeDelta, WindowSource,
    WindowSummary,
};
use crate::timestamps::Timestamps;
use crate::verdict::{Degradation, Verdict};
use crate::view::{self, Facts, Hidden, View};
use crate::visibility::{self, Uncovered, Visibility};
use crate::wait::{Condition, Selector, SnapshotSummary, WaitOutcome};

/// How often a window that sends no change events is re-read when nothing
/// else re-reads it.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How often a window that sends no change events is re-read when nothing
/// else re-reads it, on a target whose frames come slowly ([`SlowFrames`])
/// and that has a frame source, while its frames hold still (design D26 of
/// physical-ios). A read there costs more than a frame interval, 450 to
/// 650 ms for a physical iPhone's 500-cell screen, so the 1 s poll kept the
/// target's worker reading half the time, and the frames already tell a
/// still screen: the poll re-reads once the frames changed since the read
/// before, or once this long has passed since it.
pub const DEFAULT_STILL_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// The shortest time from the end of a read of a window that sends no change
/// events to a re-read that frame changes trigger (design D5: re-read after
/// frames change and once more when they stop, bounded by a minimum
/// interval), but for the window of a field the mirror has just entered
/// text into in one piece, which frame changes re-read at once (see
/// [`Mirror::act`]).
pub const DEFAULT_MIN_REREAD_INTERVAL: Duration = Duration::from_millis(200);

/// How often a settle wait without frames reads its anchor nodes' bounds
/// again (task 10.11): four probes to a default quiet window, so the wait
/// ends at most this long after the quiet window that follows the last
/// move.
pub const DEFAULT_PROBE_INTERVAL: Duration = Duration::from_millis(50);

/// How many anchor nodes a probe reads, spread over the elements with refs
/// from the first to the last, so that a move of the whole screen or of the
/// part holding either end shows.
pub const DEFAULT_PROBE_ANCHORS: usize = 4;

/// How long a settle wait without frames holds after a stroke that leaves
/// the content under it at an end of its range (task 10.14), counted from
/// the stroke or the tree's last change since, whichever is later. From
/// Android 12 a scroll view that a stroke pushes against an end, or that a
/// fling carries into one, draws a stretch that springs back on its own
/// (`EdgeEffect`: natural frequency 24.657, damping ratio 0.98, a release
/// or fling velocity taken in at 13 times). The stretch moves no view and
/// sends no event, so without frames nothing shows it, and until it rests
/// `ScrollView` takes a touch that comes down as the start of a drag that
/// catches the stretch: a tap on a row then opens nothing. On the API 36
/// emulator the Expo fixture's list, pulled against its end by a 600-point
/// stroke, drew the stretch for 333 to 383 ms after the frame that took
/// the finger's lift, and a tap within about 350 ms of the stroke's end
/// opened nothing; the hold leaves 117 ms beyond the longest stretch.
pub const DEFAULT_OVERSCROLL: Duration = Duration::from_millis(500);

/// How long a re-read after input, and a settle wait after it, hold for the
/// first frame that changed after the input on a target whose frames come
/// slowly (design D23 of physical-ios): a physical iPhone's polled
/// screenshots showed a tap's first change 222 to 439 ms after its
/// acknowledgement at p95 (spike S11).
pub const DEFAULT_FIRST_FRAME_WAIT: Duration = Duration::from_millis(500);

/// Frames that come slowly: screenshots polled one at a time rather than a
/// stream, as a physical iPhone's on iOS 26, about 19 a second over two
/// connections (design D23 of physical-ios). A change can take a capture
/// interval to show, and a tap's first change comes hundreds of
/// milliseconds after it, so settle waits at least two capture intervals
/// without a change and holds after input for the first changed frame, and
/// the re-read after input waits for that frame rather than read the screen
/// the input has not changed yet. The poll of a window without events waits
/// for the frames to change, or for the still-poll interval. Without a frame
/// source the setting does nothing: windows without events are re-read
/// every minimum interval, as on any target without frames, and polled
/// every poll interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlowFrames {
    /// The time between two captures: 53 ms at 19 a second.
    pub capture_interval: Duration,
    /// How long the re-read after input, and settle after it, hold for the
    /// first changed frame.
    pub first_frame_wait: Duration,
    /// How long a window without events goes unpolled while the frames hold
    /// still since its last read began.
    pub still_poll_interval: Duration,
}

impl SlowFrames {
    /// Frames captured every `capture_interval`, the first changed frame
    /// after input awaited [`DEFAULT_FIRST_FRAME_WAIT`] and a still screen
    /// polled every [`DEFAULT_STILL_POLL_INTERVAL`].
    pub fn every(capture_interval: Duration) -> SlowFrames {
        SlowFrames {
            capture_interval,
            first_frame_wait: DEFAULT_FIRST_FRAME_WAIT,
            still_poll_interval: DEFAULT_STILL_POLL_INTERVAL,
        }
    }

    /// The shortest quiet window settle takes: two capture intervals.
    pub fn quiet(&self) -> Duration {
        self.capture_interval * 2
    }
}

/// Tuning for one mirror.
#[derive(Clone, Debug, PartialEq)]
pub struct MirrorConfig {
    pub poll_interval: Duration,
    pub min_reread_interval: Duration,
    /// How often a settle wait without frames probes its anchors' bounds.
    pub probe_interval: Duration,
    /// How many anchor nodes a probe reads; 0 probes none.
    pub probe_anchors: usize,
    /// How long a settle wait without frames holds after a stroke that
    /// leaves the content under it at an end of its range, for the
    /// overscroll effect the platform draws there; zero holds none.
    pub overscroll: Duration,
    /// How the target's frames come when they come slowly, as a physical
    /// iPhone's screenshots; `None` for a stream, the default. The driver's
    /// attachment sets it.
    pub slow_frames: Option<SlowFrames>,
}

impl Default for MirrorConfig {
    fn default() -> MirrorConfig {
        MirrorConfig {
            poll_interval: DEFAULT_POLL_INTERVAL,
            min_reread_interval: DEFAULT_MIN_REREAD_INTERVAL,
            probe_interval: DEFAULT_PROBE_INTERVAL,
            probe_anchors: DEFAULT_PROBE_ANCHORS,
            overscroll: DEFAULT_OVERSCROLL,
            slow_frames: None,
        }
    }
}

/// A node a settle wait probes: its element, where the latest read has it,
/// and the bounds that read found.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    element: ElementId,
    handle: NodeHandle,
    bounds: Bounds,
}

/// A settle wait's bounds probes (task 10.11).
#[derive(Default)]
struct Probes {
    /// When the latest probe began.
    at: Option<Instant>,
    /// What the latest probe found of each anchor: its bounds, or `None`
    /// for one that was gone.
    last: HashMap<ElementId, Option<Bounds>>,
    /// When the latest probe began that found every anchor where the
    /// replica has it; `None` while the latest one did not.
    agreed: Option<Instant>,
    /// The replica's generation and the end of its latest read when the
    /// anchors were chosen.
    chosen_for: Option<(u64, Option<Instant>)>,
    /// The anchors chosen then.
    anchors: Vec<Anchor>,
}

impl Probes {
    /// The anchors of `replica` as its latest read has them, chosen once
    /// per read.
    fn anchors(&mut self, replica: &Replica, count: usize) -> Vec<Anchor> {
        let key = (replica.generation, replica.updated_at);
        if self.chosen_for != Some(key) {
            self.anchors = anchors(replica, count);
            self.chosen_for = Some(key);
        }
        self.anchors.clone()
    }

    /// Whether a probe that began at or after `change`, the latest change,
    /// found every anchor where the replica has it.
    fn agrees_since(&self, change: Option<Instant>) -> bool {
        self.agreed
            .is_some_and(|at| change.is_none_or(|change| at >= change))
    }

    /// When the next probe falls due, once one was made.
    fn next(&self, every: Duration) -> Option<Instant> {
        self.at.map(|at| at + every)
    }
}

/// Up to `count` nodes to probe: elements with refs that are on screen and
/// not covered, in application windows, top-most window first, spread
/// evenly from the first to the last so that both ends of the screen are
/// among them. While none is on screen, as when a screen sliding in was
/// read before it entered, those off it stand in.
fn anchors(replica: &Replica, count: usize) -> Vec<Anchor> {
    let view = View::build(replica);
    let (mut shown, mut off) = (Vec::new(), Vec::new());
    for (w, window) in replica.windows.iter().enumerate() {
        if window.kind != WindowType::APPLICATION {
            continue;
        }
        for (i, node) in window.nodes.iter().enumerate() {
            let facts = &view.windows[w][i];
            if !facts.bears_ref || facts.absorbed || facts.hidden != Hidden::No {
                continue;
            }
            let anchor = Anchor {
                element: node.element,
                handle: NodeHandle {
                    window: window.id,
                    node: node.node_id,
                },
                bounds: node.bounds,
            };
            if facts.visible {
                shown.push(anchor);
            } else {
                off.push(anchor);
            }
        }
    }
    spread(if shown.is_empty() { off } else { shown }, count)
}

/// `count` of `items`, evenly spread from the first to the last.
fn spread<T: Copy>(items: Vec<T>, count: usize) -> Vec<T> {
    match count {
        0 => Vec::new(),
        _ if items.len() <= count => items,
        1 => vec![items[0]],
        _ => (0..count)
            .map(|k| items[k * (items.len() - 1) / (count - 1)])
            .collect(),
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
    /// A stroke from `from` to `to` over `duration`, its finger leaving the
    /// screen as `release` says.
    Swipe {
        from: Point,
        to: Point,
        duration: Duration,
        release: Release,
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
    /// needed. Frames that come slowly and a frame source together make the
    /// still poll the windows' poll ([`SlowFrames::still_poll_interval`]).
    pub fn new(backend: Backend, config: MirrorConfig) -> Mirror {
        let mut replica = Replica::new();
        replica.still_poll = config
            .slow_frames
            .filter(|_| backend.frames.is_some())
            .map(|slow| slow.still_poll_interval);
        Mirror {
            backend,
            config,
            replica,
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
    /// are due, without waiting, but for a re-read after input on a target
    /// whose frames come slowly ([`SlowFrames`]), which first waits for the
    /// first frame that changed after the input, at most the first-frame
    /// wait from it.
    pub fn pump(&mut self) -> Result<(), MirrorError> {
        self.drain();
        self.refresh()
    }

    /// Applies what the target reported so far without reading the tree,
    /// even when a read is due, and says how many items it applied. The
    /// reads the items call for happen at the next `pump`, snapshot, wait
    /// or ref.
    pub fn drain(&mut self) -> usize {
        let now = self.now();
        let mut applied = 0;
        while let Some(item) = self.backend.events.next(now) {
            self.replica.apply(item);
            applied += 1;
        }
        applied
    }

    /// Records that the window in front changed outside the mirror's input,
    /// as launching or stopping an app does, so that the mirror treats it as
    /// a window event arriving now: every window is read again when the
    /// tree is next needed, and settle counts it as a tree change, so a
    /// settle that starts right after a launch waits for the launched app
    /// even when its first signals arrive later.
    pub fn note_window_change(&mut self) {
        let now = self.now();
        self.replica.note_window_change(now);
    }

    /// When the latest frame, frame change, tree change, read and input
    /// happened, as the mirror saw them; items still queued in the event
    /// stream count once drained.
    pub fn timestamps(&self) -> Timestamps {
        Timestamps {
            last_frame: self.replica.frame,
            last_frame_change: self.replica.last_frame_change,
            last_tree_change: self.replica.last_tree_change,
            last_read: self.replica.last_read,
            last_action: self.replica.last_action,
        }
    }

    /// The capture times of the frame changes at or after `since`, oldest
    /// first, among the latest 1024 the mirror keeps.
    pub fn frame_changes_since(&self, since: Instant) -> Vec<Instant> {
        self.replica
            .frame_change_log
            .iter()
            .copied()
            .filter(|&at| at >= since)
            .collect()
    }

    /// Performs the reads that are due. A read from another backend than
    /// the replica's is set aside and the replica rebuilt from a full read.
    fn refresh(&mut self) -> Result<(), MirrorError> {
        if self.replica.disconnected.is_some() {
            return Ok(());
        }
        self.await_first_frame();
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

    /// When a target whose frames come slowly stops waiting for the first
    /// frame that changed after the latest input: `None` without such
    /// frames, input or wait, once that frame came, or once the
    /// first-frame wait from the input has passed.
    fn first_frame_due(&self, now: Instant) -> Option<Instant> {
        let slow = self.config.slow_frames?;
        self.backend.frames.as_ref()?;
        let input = self.replica.last_action?;
        if self
            .replica
            .last_frame_change
            .is_some_and(|change| change > input)
        {
            return None;
        }
        let until = input + slow.first_frame_wait;
        (now < until).then_some(until)
    }

    /// Holds a re-read after input on a target whose frames come slowly
    /// until the first frame that changed after the input arrives, at most
    /// the first-frame wait from the input (design D23 of physical-ios):
    /// read at once, as on a stream, it would show the screen before the
    /// input's effect, a tap's first change coming hundreds of milliseconds
    /// later on a physical iPhone, and the frame-driven re-read that would
    /// catch the change comes a minimum interval after a read that costs
    /// about half a second there.
    fn await_first_frame(&mut self) {
        if !self.replica.action_pending() {
            return;
        }
        while let Some(until) = self.first_frame_due(self.now()) {
            if self.replica.disconnected.is_some() {
                return;
            }
            if let Some(item) = self.backend.events.next(until) {
                self.replica.apply(item);
            }
        }
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
                .next_due(now, &self.config)
                .map_or(deadline, |due| due.min(deadline))
                .max(now);
            if let Some(item) = self.backend.events.next(wake) {
                self.replica.apply(item);
            }
        }
    }

    /// When a read of a window that sends no change events falls due next:
    /// its poll, a re-read after frames changed, or, after text for one of
    /// its fields, the re-read that text calls for.
    pub fn next_wakeup(&self) -> Option<Instant> {
        self.replica.next_due(self.now(), &self.config)
    }

    /// A snapshot of the replica, acquiring it first when none exists.
    /// Listed elements that are actionable, content with a name, or labelled
    /// controls without traits get refs. Both listings mark an element a tap
    /// reaches only part of with that share; an interactive snapshot leaves
    /// out one a tap reaches none of, counting it in its scroll container's
    /// hint with the view drawn over it named, while a full one keeps it at
    /// 0% (rn-app-fixes, designs D45 and D50).
    pub fn snapshot(&mut self, options: &SnapshotOptions) -> Result<Snapshot, MirrorError> {
        self.pump()?;
        let now = self.now();
        let view = View::build(&self.replica);
        let reach = visibility::listed(&self.replica, &view);
        let found = view::hints(&self.replica, &view, options.interactive.then_some(&reach));
        let lines = render::listing(&self.replica, &view, &found, options, &reach);
        let mut elements = Vec::new();
        let mut text = Vec::with_capacity(lines.len());
        let mut listed = Vec::new();
        for line in lines {
            match line {
                Line::Element {
                    w,
                    i,
                    depth,
                    visible,
                } => {
                    let window = &self.replica.windows[w];
                    let node = &window.nodes[i];
                    let facts = &view.windows[w][i];
                    let generation = self.replica.generation;
                    let reference = facts.bears_ref.then(|| {
                        self.refs.assign(
                            node.element,
                            (facts.role, &facts.name, place(node)),
                            node.identifier.is_some(),
                            generation,
                        )
                    });
                    if let Some(reference) = reference {
                        listed.push((facts.signature, reference));
                    }
                    let mut listed_element = element(window, node, facts, reference);
                    listed_element.depth = depth;
                    listed_element.visible = visible;
                    text.push(render::element_line(&listed_element));
                    elements.push(listed_element);
                }
                Line::Keyboard { w } => {
                    let keyboard = keyboard(&self.replica.windows[w], self.replica.scale);
                    text.push(render::element_line(&keyboard));
                    elements.push(keyboard);
                }
                Line::Hint { index, depth } => {
                    text.push(render::hint_line(&found.scroll[index].2, depth));
                }
                Line::Covered { index, depth } => {
                    let (_, _, by, count) = &found.covered[index];
                    text.push(render::covered_line(*count, by, depth));
                }
                Line::UnderKeyboard { count } => {
                    text.push(render::under_keyboard_line(count));
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
            hints: found.scroll.into_iter().map(|(_, _, hint)| hint).collect(),
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

    /// The one field `holder` holds, as the replica has it now, with a ref
    /// for it: a node below the element that takes text, when the element
    /// holds exactly one. An identifier `fill` or `type` is given can name a
    /// node that takes no text around the field that does, as a React Native
    /// code input's view around a field without an area (rn-app-fixes,
    /// design D39). `None` when the element holds none or several, or the
    /// replica no longer holds it.
    pub fn field_within(&mut self, holder: ElementId) -> Option<Element> {
        let (w, i) = self.replica.locate(holder)?;
        let view = View::build(&self.replica);
        let window = &self.replica.windows[w];
        let facts = &view.windows[w];
        let [d] = fields_below(window, facts, i)[..] else {
            return None;
        };
        let (node, facts) = (&window.nodes[d], &facts[d]);
        let reference = self.refs.assign(
            node.element,
            (facts.role, &facts.name, place(node)),
            node.identifier.is_some(),
            self.replica.generation,
        );
        Some(element(window, node, facts, Some(reference)))
    }

    /// How many fields `holder` holds, as the replica has it now: the nodes
    /// below the element that take text, as [`Mirror::field_within`] looks
    /// for them. An identifier `fill` or `type` is given can name a node
    /// that holds none, the app's input outside the tree, as a React Native
    /// code input's view on the acceptance run's second visit to the code
    /// step (rn-app-fixes, design D44). `None` when the replica no longer
    /// holds the element.
    pub fn fields_within(&self, holder: ElementId) -> Option<usize> {
        let (w, i) = self.replica.locate(holder)?;
        let view = View::build(&self.replica);
        Some(fields_below(&self.replica.windows[w], &view.windows[w], i).len())
    }

    /// How much of `element` is in view as the replica has it now, without
    /// reading the target: the share of its bounds a tap reaches, inside
    /// the visible bounds of what holds it and under no view drawn over it,
    /// the largest part of it a tap goes to, and the edge its bounds may
    /// have been clipped at (rn-app-fixes, design D37). `None` when the
    /// replica no longer holds the element.
    pub fn visibility(&self, element: ElementId) -> Option<Visibility> {
        let (w, i) = self.replica.locate(element)?;
        let view = View::build(&self.replica);
        Some(visibility::of(&self.replica, &view, w, i))
    }

    /// What covers `element` as the replica has it now, without reading the
    /// target, named as an action on its ref names it when it fails with
    /// `element_covered`: the top-most modal, a view marked modal or a view
    /// drawn over it that covers all of it, or what a tap at its centre
    /// would land on when the views over part of it leave no part of it in
    /// view that a tap reaches, at least 8 points on each side (design
    /// D37). `None` when it is not covered so, or the replica no longer
    /// holds it.
    pub fn covered_by(&self, element: ElementId) -> Option<String> {
        let (w, i) = self.replica.locate(element)?;
        let view = View::build(&self.replica);
        visibility::reach(&self.replica, &view, w, i)
            .1
            .map(|cover| cover.by)
    }

    /// The visible bounds of the scroll container that a stroke along the
    /// vertical axis, or the horizontal one, through `at` moves, as the
    /// replica has it now, without reading the target: the innermost node on
    /// screen and not covered that scrolls along that axis and holds `at`,
    /// its bounds clipped by its window and the scroll containers around
    /// it, less the bars drawn over them, as an element inside it sees them
    /// (see [`Visibility::viewport`]). `None` when no such node holds `at`.
    pub fn scroll_viewport(&self, at: Point, vertical: bool) -> Option<Rect> {
        let view = View::build(&self.replica);
        visibility::scroller_at(&self.replica, &view, at, vertical)
    }

    /// What shows of the scroll container that shows the most of `area`,
    /// in device points, as the replica has it now, without reading the
    /// target: among the nodes on screen and not covered that scroll along
    /// the vertical axis, or the horizontal one, in the top-most window that
    /// has one, the one whose visible bounds (see
    /// [`Mirror::scroll_viewport`]) share the most of `area` (rn-app-fixes,
    /// design D41). `None` when none shares any of it.
    pub fn scroll_container_in(&self, area: Rect, vertical: bool) -> Option<Rect> {
        let view = View::build(&self.replica);
        visibility::scroller_in(&self.replica, &view, area, vertical)
    }

    /// What of `area`, in device points, a stroke may go through, as the
    /// replica has it now, without reading the target (task 10.32): `area`
    /// less the system and input-method windows above the window of
    /// `within`, or, when `within` is `None` or gone, above the window in
    /// front (the active window that is not a keyboard's, else the top-most
    /// application window), each cutting it as a bar cuts visible bounds
    /// (see [`Mirror::visibility`]).
    pub fn uncovered(&self, area: Rect, within: Option<ElementId>) -> Uncovered {
        let w = within
            .and_then(|element| self.replica.locate(element))
            .map(|(w, _)| w);
        visibility::uncovered(&self.replica, w, area)
    }

    /// Where the element `reference` names is in the replica. The element
    /// must still be the control the ref was issued for: without an
    /// identifier, under the same accessible name.
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
                let gone = MirrorError::StaleRef {
                    reference,
                    cause: StaleCause::Removed,
                    was,
                };
                let Some((w, i)) = self.replica.locate(issued.element) else {
                    return Err(gone);
                };
                let window = &self.replica.windows[w];
                let node = &window.nodes[i];
                let facts = view::facts_of(&self.replica, w, i);
                let named = (facts.role, facts.name.as_str(), place(node));
                let still =
                    refs::names(issued, named.0, named.1, named.2, node.identifier.is_some());
                if issued.retired || !still {
                    return Err(gone);
                }
                Ok((w, i))
            }
        }
    }

    /// The point an action aims at, in device points: for a ref, the centre
    /// of the largest part of its element a tap reaches, which must be at
    /// least 8 points on each side, else the action fails covered or out of
    /// view as [`Mirror::covered_by`] and [`Mirror::visibility`] tell
    /// (rn-app-fixes, design D37).
    fn aim(&mut self, target: &Target) -> Result<Point, MirrorError> {
        let reference = match target {
            Target::Point(point) => return Ok(*point),
            Target::Ref(reference) => *reference,
        };
        self.pump()?;
        let (w, i) = self.locate_ref(reference)?;
        let view = View::build(&self.replica);
        let (seen, covered) = visibility::reach(&self.replica, &view, w, i);
        if let Some(cover) = covered {
            return Err(MirrorError::Covered {
                reference,
                by: cover.by,
                drawn_over: cover.drawn_over,
            });
        }
        match seen.reachable {
            Some(part) => Ok(part.center()),
            None => Err(MirrorError::Offscreen { reference }),
        }
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
    /// next poll. After text the platform took in one piece for a field of
    /// such a window, pasted on a Simulator or set directly on Android, that
    /// window is read again as soon as frames change after its latest read
    /// began, without waiting out the minimum re-read interval, or, on a
    /// target without frames, as soon as that read ends: until a read shows
    /// the field's text changed, the next input, or a poll interval after
    /// the text (task 10.29). Text typed as keys keeps the interval: a
    /// reader that holds typed text, as `fill` holds it 200 ms, gains
    /// nothing from a read at its first frame, which would put the next read
    /// 200 ms after it.
    pub fn act(&mut self, action: &Action) -> Result<ActionOutcome, MirrorError> {
        self.drain();
        if let Some(reason) = &self.replica.disconnected {
            return Err(MirrorError::Disconnected(reason.clone()));
        }
        // The element of the field text goes into.
        let mut field = None;
        let (input, at) = match action {
            Action::Tap { target, hold } => {
                let at = self.aim(target)?;
                (Input::Tap { at, hold: *hold }, Some(at))
            }
            Action::Swipe {
                from,
                to,
                duration,
                release,
            } => (
                Input::Swipe {
                    from: *from,
                    to: *to,
                    duration: *duration,
                    release: *release,
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
                        let node = &window.nodes[i];
                        field = Some(node.element);
                        Some(NodeHandle {
                            window: window.id,
                            node: node.node_id,
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
        let stroke = match action {
            Action::Swipe { from, to, .. } => Some((*from, *to)),
            _ => None,
        };
        self.replica.note_action(now, stroke);
        let in_one_piece = ack.accepted && ack.method != Some(TextMethod::KEYS);
        if let (Some(element), true) = (field, in_one_piece) {
            let frames = self.backend.frames.is_some();
            self.replica.note_edit(element, now, frames);
        }
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
    ///
    /// Without frames, nodes that move without a change event, as views an
    /// app animates may, change nothing the tree reports (task 10.11). When
    /// the tree source can read bounds again, a few anchor nodes' bounds are
    /// probed every probe interval: a move between probes is a tree change,
    /// anchors that hold still where the replica does not have them bring a
    /// read of their windows with the cache cleared, and the wait settles
    /// only once a probe after the last change finds every anchor where the
    /// replica has it. Nor does it settle, without frames, while the tree
    /// has an application window drawn away from where its layout puts it,
    /// as the platform draws a window it animates in or out: reads and
    /// probes then report one frame of that animation, unchanged until it
    /// ends, so a maximum that passes meanwhile names the tree. After a
    /// stroke that leaves the content under it at an end of its range, it
    /// does not settle, without frames, until the overscroll hold has
    /// passed since the stroke and since the tree's last change (task
    /// 10.14): the platform's overscroll effect there shows in no read and
    /// no probe, so a maximum that passes meanwhile names the tree too.
    ///
    /// On a target whose frames come slowly ([`SlowFrames`]), the quiet
    /// window is at least two capture intervals, and after input the wait
    /// does not settle, when it watches frames, until the first frame that
    /// changed after the input came or the first-frame wait from the input
    /// has passed.
    pub fn settle(&mut self, options: &SettleOptions) -> Result<Settle, MirrorError> {
        let start = self.now();
        let deadline = start + options.max;
        let tree_before = self.replica.tree_changes;
        let frames_before = self.replica.frame_changes;
        let blind = self.backend.frames.is_none();
        let every = self.config.min_reread_interval;
        let slow = self.config.slow_frames.filter(|_| !blind);
        let quiet = slow.map_or(options.quiet, |slow| options.quiet.max(slow.quiet()));
        let mut probes = (blind
            && options.signals.tree()
            && self.config.probe_anchors > 0
            && self.backend.tree.can_probe())
        .then(Probes::default);
        self.drain();
        loop {
            if blind {
                self.reread_quietly(every)?;
            }
            if let Some(state) = probes.as_mut()
                && !self.probe_bounds(state)?
            {
                // The target is gone or cannot be probed: what the tree
                // reports decides.
                probes = None;
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
            let quiet_until = last.map(|at| at + quiet);
            // Slow frames: the input's first changed frame may still come.
            let first_frame = if options.signals.frames() && slow.is_some() {
                self.first_frame_due(now)
            } else {
                None
            };
            let still = probes.as_ref().is_none_or(|state| state.agrees_since(last));
            let transformed = blind && options.signals.tree() && self.replica.drawn_transformed();
            let overscroll = if blind && options.signals.tree() {
                self.overscroll_ends()
            } else {
                None
            };
            let overscrolling = overscroll.is_some_and(|until| now < until);
            let settled = quiet_until.is_none_or(|until| now >= until)
                && still
                && !transformed
                && !overscrolling
                && first_frame.is_none();
            if settled || now >= deadline {
                let since = now.checked_sub(quiet).unwrap_or(start);
                let mut still_changing = Vec::new();
                if !settled {
                    if transformed || overscrolling || tree.is_some_and(|at| at > since) {
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
            let mut wake = match quiet_until {
                Some(until) if until > now => until.min(deadline),
                _ => deadline,
            };
            if blind && let Some(due) = self.replica.next_reread(every) {
                wake = wake.min(due);
            }
            if let Some(due) = probes
                .as_ref()
                .and_then(|state| state.next(self.config.probe_interval))
            {
                wake = wake.min(due);
            }
            if let Some(until) = overscroll.filter(|&until| until > now) {
                wake = wake.min(until);
            }
            if let Some(until) = first_frame {
                wake = wake.min(until);
            }
            if let Some(item) = self.backend.events.next(wake.max(now)) {
                self.replica.apply(item);
            }
        }
    }

    /// Probes the anchors' bounds when a probe is due (task 10.11). A move
    /// since the last probe, or a first probe that finds the replica
    /// stale, is a tree change; anchors that held still since the last
    /// probe where the replica does not have them bring a read of their
    /// windows with the cache cleared. Says whether probing goes on: not
    /// once the target is gone or fails a probe in a way a retry cannot fix.
    fn probe_bounds(&mut self, probes: &mut Probes) -> Result<bool, MirrorError> {
        if self.replica.disconnected.is_some() {
            return Ok(false);
        }
        let started = self.now();
        if self.replica.need_full.is_some()
            || probes
                .at
                .is_some_and(|at| started < at + self.config.probe_interval)
        {
            return Ok(true);
        }
        probes.at = Some(started);
        let anchors = probes.anchors(&self.replica, self.config.probe_anchors);
        if anchors.is_empty() {
            probes.agreed = Some(started);
            return Ok(true);
        }
        let handles: Vec<NodeHandle> = anchors.iter().map(|a| a.handle).collect();
        let found = match self.backend.tree.probe(&handles) {
            Ok(found) => found,
            Err(error) if error.retryable => return Ok(true),
            Err(_) => return Ok(false),
        };
        let finished = self.now();
        let probed: HashMap<NodeHandle, Bounds> =
            found.into_iter().map(|p| (p.node, p.bounds)).collect();
        let current: HashMap<ElementId, Option<Bounds>> = anchors
            .iter()
            .map(|a| (a.element, probed.get(&a.handle).copied()))
            .collect();
        let first = probes.last.is_empty();
        let moved = probes
            .last
            .iter()
            .any(|(element, was)| current.get(element).is_some_and(|now| now != was));
        let mut stale: Vec<u32> = Vec::new();
        for anchor in &anchors {
            let now = current.get(&anchor.element).copied().flatten();
            if now != Some(anchor.bounds) && !stale.contains(&anchor.handle.window) {
                stale.push(anchor.handle.window);
            }
        }
        probes.last = current;
        if stale.is_empty() {
            probes.agreed = Some(started);
            return Ok(true);
        }
        probes.agreed = None;
        if first || moved {
            self.replica.note_moved(finished);
        } else {
            let request = ReadRequest {
                windows: Some(stale),
                reason: ReadReason::Moved,
            };
            self.read(&request, finished)?;
        }
        Ok(true)
    }

    /// When the overscroll effect the latest input may have left has come
    /// to rest, at the latest (task 10.14): the overscroll hold after that
    /// input or after the tree's last change since, whichever is later,
    /// when the input was a stroke and content under it sits at an end of
    /// its range as the replica has it. The tree's last change stands for a
    /// fling that carried the content into that end after the finger
    /// lifted: the scroll event that reports the fling's last move comes no
    /// sooner than the move.
    fn overscroll_ends(&self) -> Option<Instant> {
        let stroke = self.replica.last_stroke?;
        if self.config.overscroll.is_zero() || !self.replica.stroke_reached_an_end(&stroke) {
            return None;
        }
        let from = self
            .replica
            .last_tree_change
            .map_or(stroke.at, |change| change.max(stroke.at));
        Some(from + self.config.overscroll)
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
    /// Only elements on screen and not covered match, but for `Attached`
    /// and `Detached`, which ask whether the replica holds a match, in view
    /// or not (rn-app-fixes, design D51), as a query does. A condition that
    /// holds in the replica answers at once, without a read that is due,
    /// while the replica is current by the mirror's own rules: read since
    /// the last window change, event naming a window and injected input.
    pub fn wait(
        &mut self,
        selector: &Selector,
        condition: Condition,
        timeout: Duration,
    ) -> Result<WaitOutcome, MirrorError> {
        let start = self.now();
        let deadline = start + timeout;
        self.drain();
        // Whether the next look takes the reads that are due first: the
        // first one does not when a read is due of a replica that is
        // current, so a condition that holds in it answers without that
        // read (rn-app-fixes, design D36), which the next pump takes. With
        // nothing due, reading first reads nothing.
        let now = self.now();
        let mut read = !(self.replica.current()
            && self
                .replica
                .next_due(now, &self.config)
                .is_some_and(|due| due <= now));
        loop {
            if read {
                self.pump()?;
            }
            let now = self.now();
            let found = self.first_match(selector, condition);
            let holds = match condition {
                Condition::Disappear | Condition::Detached => found.is_none(),
                Condition::Appear | Condition::Enabled | Condition::Attached => found.is_some(),
            };
            if holds {
                return Ok(WaitOutcome {
                    elapsed: now - start,
                    element: found,
                });
            }
            if !read {
                read = true;
                continue;
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
                .next_due(now, &self.config)
                .map_or(deadline, |due| due.min(deadline))
                .max(now);
            if let Some(item) = self.backend.events.next(wake) {
                self.replica.apply(item);
            }
        }
    }

    /// The first element that matches `selector` as `condition` looks for
    /// it: on screen, not covered, and enabled for `Enabled`; any element
    /// the replica holds for `Attached` and `Detached`, a keyboard's keys
    /// aside.
    fn first_match(&self, selector: &Selector, condition: Condition) -> Option<Element> {
        let view = View::build(&self.replica);
        let held = matches!(condition, Condition::Attached | Condition::Detached);
        for (w, window) in self.replica.windows.iter().enumerate() {
            for (i, node) in window.nodes.iter().enumerate() {
                let facts = &view.windows[w][i];
                let listed = if held {
                    facts.hidden != Hidden::Keyboard
                } else {
                    facts.visible && facts.hidden == Hidden::No
                };
                if !listed {
                    continue;
                }
                if condition == Condition::Enabled && !node.flags.contains(NodeFlags::ENABLED) {
                    continue;
                }
                let reference = self.refs.of(
                    node.element,
                    (facts.role, &facts.name, place(node)),
                    node.identifier.is_some(),
                );
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

    /// The latest frame, encoded no larger than the requested long edge,
    /// upright, with its timestamp and age.
    ///
    /// The mirror picks the size and quality; the frame source encodes. The
    /// capture's age counts from now, its skew from the last tree read.
    pub fn frame(&mut self, request: &FrameRequest) -> Result<FrameImage, MirrorError> {
        self.drain();
        let frames = self.backend.frames.as_mut().ok_or(MirrorError::NoFrame)?;
        let latest = frames.latest().ok_or(MirrorError::NoFrame)?;
        let (width, height) = frame::fit(
            latest.width,
            latest.height,
            latest.rotation,
            request.max_dimension(),
        );
        let quality = request.quality();
        let data = frames.encode(&EncodeRequest {
            seq: latest.seq,
            width,
            height,
            quality,
            rotation: latest.rotation,
        })?;
        let now = self.now();
        Ok(FrameImage {
            seq: latest.seq,
            data,
            width,
            height,
            quality,
            captured_at: latest.at,
            age: now.saturating_duration_since(latest.at),
            tree_skew: TreeSkew::between(latest.at, self.replica.updated_at),
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

/// Where `node` sits, for the refs: its position among its parent's
/// children. Its parent is the same element as long as it survives, since
/// the replica pairs only the children of nodes it paired.
fn place(node: &RNode) -> u32 {
    node.position
}

/// The nodes below node `i` of `window` that take text and are not folded
/// into another, by index, `facts` being the window's.
fn fields_below(window: &Win, facts: &[Facts], i: usize) -> Vec<usize> {
    let mut fields = Vec::new();
    let mut stack: Vec<usize> = window.nodes[i].children.clone();
    while let Some(d) = stack.pop() {
        if view::field(&window.nodes[d], facts[d].role) && !facts[d].absorbed {
            fields.push(d);
        }
        stack.extend(window.nodes[d].children.iter().copied());
    }
    fields
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
        visible: None,
        actionable: facts.actionable,
        window: window.id,
        depth: node.depth as usize,
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
        visible: None,
        actionable: false,
        window: window.id,
        depth: 0,
        class: root.map_or_else(String::new, |n| n.class.to_string()),
    }
}
