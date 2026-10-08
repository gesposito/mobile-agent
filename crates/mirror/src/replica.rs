//! The replica: one target's windows and node trees, kept current from full
//! reads and from what the target reports.
//!
//! Every read of a window is matched against the previous one so that an
//! element keeps its [`ElementId`] while it survives; the differences are the
//! window's deltas. Nodes are matched under matched parents, among siblings
//! of the same class and identifier: a lone sibling on both sides matches
//! outright, the others by the content of their subtree, then by sharing an
//! identified descendant. Siblings that none of these tell apart get fresh
//! identities, so an identity never moves to another element.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use protocol::event::{EventPayload, LifecycleKind, Stream};
use protocol::snapshot::{
    Bounds, Node, NodeFlags, Scroll, ScrollAxis, ScrollFlags, Truncation, Window, WindowFlags,
    WindowType,
};

use crate::backend::{
    BackendInfo, FrameMeta, Owner, ReadReason, ReadRequest, StreamItem, Timed, TreeRead,
};
use crate::error::ResyncCause;
use crate::geometry::{Point, Rect};
use crate::mirror::MirrorConfig;
use crate::snapshot::{ElementId, TreeDelta};
use crate::timestamps::ReadSpan;

/// The package of Jetpack Compose's classes. Compose content sends no change
/// events to a UiAutomation client (spike S1), and a window hosting it holds
/// a node of one of these classes:
/// `androidx.compose.ui.platform.AndroidComposeView` in spike S1,
/// `androidx.compose.ui.platform.ComposeView` on the API 36 emulator. Only a
/// read without window metadata falls back to looking for them.
pub(crate) const COMPOSE_PACKAGE: &str = "androidx.compose.";

/// How many deltas the replica keeps.
const DELTA_LOG: usize = 256;

/// How many frame changes the replica keeps: about 17 s of a 60 fps
/// animation.
const FRAME_CHANGE_LOG: usize = 1024;

/// Not yet matched to a previous read.
const UNASSIGNED: ElementId = ElementId(0);

/// One node of a window, in the window's pre-order.
#[derive(Clone, Debug)]
pub(crate) struct RNode {
    pub element: ElementId,
    /// The node's id in the read it came from.
    pub node_id: u32,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub depth: usize,
    pub class: Arc<str>,
    pub identifier: Option<String>,
    pub text: Option<String>,
    pub desc: Option<String>,
    pub bounds: Bounds,
    pub flags: NodeFlags,
    /// How the node scrolls, as the read reported it.
    pub scroll: Option<Scroll>,
    /// A hash of the node's class, identifier, description and, unless it
    /// is editable, text, combined with its children's: equal for subtrees
    /// that read the same, whatever their bounds and states.
    pub content: u64,
}

impl RNode {
    pub fn editable(&self) -> bool {
        self.flags.contains(NodeFlags::EDITABLE)
    }
}

/// One window of the replica.
#[derive(Clone, Debug)]
pub(crate) struct Win {
    pub id: u32,
    pub kind: WindowType,
    pub layer: i32,
    pub package: Option<String>,
    pub title: Option<String>,
    pub bounds: Bounds,
    pub flags: WindowFlags,
    pub emits_events: bool,
    pub owner: Owner,
    pub modal: bool,
    /// Pre-order; the root is at index 0 when the window has one.
    pub nodes: Vec<RNode>,
    /// Change events named the window since its last read.
    pub dirty: bool,
    /// The mirror injected input since the window's last read; only kept
    /// for windows that send no change events.
    pub action_pending: bool,
    pub read_started: Instant,
    pub read_finished: Instant,
    pub reason: ReadReason,
}

impl Win {
    /// Whether the replica's copy of the window came from full reads rather
    /// than change events.
    pub fn reread(&self) -> bool {
        !self.emits_events
    }
}

/// A stroke the mirror injected: where the finger came down and where it
/// lifted, in device points, and when the platform acknowledged the stroke,
/// which is once the finger has lifted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stroke {
    pub from: Point,
    pub to: Point,
    pub at: Instant,
}

/// Text the mirror entered in one piece into a field of a window that sends
/// no change events, until a read shows the field's text changed (task
/// 10.29): the field shows the text some time after the input, often after
/// the read that follows the input, so the field's window is read again as
/// soon as frames change after its latest read began, or, on a target
/// without frames, as soon as that read ends, rather than after the minimum
/// re-read interval. Only the text counts: on a Simulator text for a field
/// taps it first, and a read can show the field focused before its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Edit {
    /// The window holding the field.
    pub window: u32,
    /// The field.
    pub element: ElementId,
    /// What the field held as the replica had it when the text went in.
    pub text: Option<String>,
    /// When the platform acknowledged the text.
    pub at: Instant,
    /// Whether the target has frames to say when the field's window changes.
    pub frames: bool,
}

#[derive(Default)]
struct Changes {
    added: Vec<ElementId>,
    removed: Vec<ElementId>,
    changed: Vec<ElementId>,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// What to do with an event, given its sequence number.
enum Seq {
    Apply,
    /// Already reflected by a read, or a repeat.
    Skip,
    /// A lossless stream skipped numbers: the next one was `expected`.
    Gap {
        expected: u64,
    },
}

/// One target's replica.
pub(crate) struct Replica {
    pub windows: Vec<Win>,
    /// Grows with every read that changes the replica.
    pub revision: u64,
    pub deltas: Vec<TreeDelta>,
    /// When a tree event arrived, a read found a delta or a bounds probe
    /// found nodes moved, whichever is latest.
    pub last_tree_change: Option<Instant>,
    /// When a read last finished.
    pub updated_at: Option<Instant>,
    pub backend: Option<BackendInfo>,
    pub scale: f64,
    pub truncated: Truncation,
    /// A read of every window is due, and why.
    pub need_full: Option<ReadReason>,
    /// The next full read starts the replica over: no identity carries.
    pub fresh: bool,
    /// How many times the replica was started over; refs from an earlier
    /// generation are stale.
    pub generation: u64,
    /// Why each generation after the first started: `resyncs[g - 1]` for
    /// generation `g`.
    pub resyncs: Vec<ResyncCause>,
    /// What the agent reported about its reduced ability.
    pub warnings: Vec<String>,
    /// Why the target is gone, once it is.
    pub disconnected: Option<String>,
    next_element: u64,
    /// The last sequence number accounted for, per lossless stream.
    seqs: HashMap<Stream, u64>,
    pub frame: Option<FrameMeta>,
    pub last_frame_change: Option<Instant>,
    pub frame_changes: u64,
    /// Capture times of the latest frame changes, oldest first.
    pub frame_change_log: VecDeque<Instant>,
    /// The latest read taken in.
    pub last_read: Option<ReadSpan>,
    pub last_action: Option<Instant>,
    /// The latest input the mirror injected, when it was a stroke and the
    /// app in front has not changed since.
    pub last_stroke: Option<Stroke>,
    /// The latest input the mirror injected, when it was text for a field of
    /// a window that sends no change events and no read has shown the
    /// field's text changed since.
    pub edit: Option<Edit>,
    /// Tree and window events applied, plus deltas found by reads and moves
    /// found by bounds probes.
    pub tree_changes: u64,
    interner: HashMap<String, Arc<str>>,
    /// Where each element is: window and node index.
    index: HashMap<ElementId, (usize, usize)>,
}

impl Replica {
    pub fn new() -> Replica {
        Replica {
            windows: Vec::new(),
            revision: 0,
            deltas: Vec::new(),
            last_tree_change: None,
            updated_at: None,
            backend: None,
            scale: 1.0,
            truncated: Truncation::EMPTY,
            need_full: Some(ReadReason::Attach),
            fresh: true,
            generation: 0,
            resyncs: Vec::new(),
            warnings: Vec::new(),
            disconnected: None,
            next_element: 0,
            seqs: HashMap::new(),
            frame: None,
            last_frame_change: None,
            frame_changes: 0,
            frame_change_log: VecDeque::new(),
            last_read: None,
            last_action: None,
            last_stroke: None,
            edit: None,
            tree_changes: 0,
            interner: HashMap::new(),
            index: HashMap::new(),
        }
    }

    /// Where `element` is now, as window and node indices.
    pub fn locate(&self, element: ElementId) -> Option<(usize, usize)> {
        self.index.get(&element).copied()
    }

    fn reindex(&mut self) {
        self.index.clear();
        for (w, window) in self.windows.iter().enumerate() {
            for (i, node) in window.nodes.iter().enumerate() {
                self.index.insert(node.element, (w, i));
            }
        }
    }

    fn alloc(&mut self) -> ElementId {
        self.next_element += 1;
        ElementId(self.next_element)
    }

    fn request_full(&mut self, reason: ReadReason) {
        self.need_full = self.need_full.max(Some(reason));
    }

    /// Applies one item the target reported.
    pub fn apply(&mut self, timed: Timed<StreamItem>) {
        let Timed { at, item } = timed;
        match item {
            StreamItem::Event(event) => {
                // An unknown stream says nothing about the tree, so its
                // numbering cannot put the replica out of sync.
                if matches!(event.payload, EventPayload::Unknown(_)) {
                    return;
                }
                let stream = event.stream();
                match self.check_seq(stream, event.seq) {
                    Seq::Skip => {}
                    Seq::Apply => self.apply_event(at, &event.payload),
                    Seq::Gap { expected } => {
                        self.resync(ResyncCause::Gap {
                            stream,
                            expected,
                            received: event.seq,
                        });
                        self.apply_event(at, &event.payload);
                    }
                }
            }
            StreamItem::Frame(meta) => self.apply_frame(meta),
            StreamItem::OutOfSync {
                stream,
                expected,
                received,
            } => self.resync(ResyncCause::Gap {
                stream,
                expected,
                received,
            }),
            StreamItem::Resynced { stream, seq } => {
                self.seqs.insert(stream, seq);
            }
            StreamItem::BackendChanged(backend) => {
                if self.backend.as_ref().is_some_and(|b| *b != backend) {
                    self.resync(ResyncCause::Backend(backend));
                }
            }
            StreamItem::Reconnected => {
                self.seqs.clear();
                self.warnings.clear();
                self.disconnected = None;
                self.resync(ResyncCause::Reconnected);
            }
            StreamItem::Disconnected(reason) => self.disconnected = Some(reason),
        }
    }

    /// Starts the replica over: the next read is a full one that carries no
    /// identity, and refs issued so far are stale.
    pub fn resync(&mut self, cause: ResyncCause) {
        self.need_full = Some(ReadReason::Resync);
        self.fresh = true;
        self.generation += 1;
        self.resyncs.push(cause);
        self.edit = None;
    }

    /// Whether a read from `backend` comes from another backend than the
    /// replica's.
    pub fn backend_differs(&self, backend: &BackendInfo) -> bool {
        self.backend.as_ref().is_some_and(|b| b != backend)
    }

    fn check_seq(&mut self, stream: Stream, seq: u64) -> Seq {
        if stream.is_lossy() {
            return Seq::Apply;
        }
        let last = self.seqs.entry(stream).or_insert(0);
        if seq <= *last {
            Seq::Skip
        } else if seq == *last + 1 {
            *last = seq;
            Seq::Apply
        } else {
            let expected = *last + 1;
            *last = seq;
            Seq::Gap { expected }
        }
    }

    fn apply_event(&mut self, at: Instant, payload: &EventPayload) {
        match payload {
            EventPayload::Tree(change) => {
                self.tree_changes += 1;
                self.last_tree_change = self.last_tree_change.max(Some(at));
                match self.windows.iter_mut().find(|w| w.id == change.window) {
                    Some(window) => window.dirty = true,
                    None => self.request_full(ReadReason::Events),
                }
            }
            EventPayload::Window(_) => {
                self.tree_changes += 1;
                self.last_tree_change = self.last_tree_change.max(Some(at));
                self.request_full(ReadReason::Events);
            }
            EventPayload::Lifecycle(lifecycle) => match lifecycle.change {
                LifecycleKind::READY => self.request_full(ReadReason::Events),
                LifecycleKind::WARNING => self.warnings.push(lifecycle.message.clone()),
                LifecycleKind::OWNERSHIP_LOST => self.warnings.push(format!(
                    "another process took the UI automation connection: {}",
                    lifecycle.message
                )),
                LifecycleKind::SHUTTING_DOWN => {
                    let reason = if lifecycle.message.is_empty() {
                        "the agent is shutting down".to_owned()
                    } else {
                        lifecycle.message.clone()
                    };
                    self.disconnected = Some(reason);
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn apply_frame(&mut self, meta: FrameMeta) {
        if self
            .frame
            .is_some_and(|previous| previous.hash != meta.hash)
        {
            self.last_frame_change = self.last_frame_change.max(Some(meta.at));
            self.frame_changes += 1;
            if self.frame_change_log.len() == FRAME_CHANGE_LOG {
                self.frame_change_log.pop_front();
            }
            self.frame_change_log.push_back(meta.at);
        }
        self.frame = Some(meta);
    }

    fn frames_changed_since(&self, t: Instant) -> bool {
        self.last_frame_change.is_some_and(|c| c > t)
    }

    /// Records that the window in front changed at `at` outside the
    /// mirror's input, as a window event arriving then would: a tree change,
    /// and a read of every window when the tree is next needed. The last
    /// stroke is forgotten: it scrolled content that is no longer in front.
    pub fn note_window_change(&mut self, at: Instant) {
        self.tree_changes += 1;
        self.last_tree_change = self.last_tree_change.max(Some(at));
        self.last_stroke = None;
        self.request_full(ReadReason::Events);
    }

    /// Records that a bounds probe ending at `at` found nodes moved that
    /// no event reported: a tree change, since their bounds are part of the
    /// tree, although the replica holds them where they were until a read
    /// brings them.
    pub fn note_moved(&mut self, at: Instant) {
        self.tree_changes += 1;
        self.last_tree_change = self.last_tree_change.max(Some(at));
    }

    /// Whether an application window is drawn away from where its layout
    /// puts it, as the platform draws a window while it animates the window
    /// in or out, other than one in picture-in-picture, which is drawn
    /// scaled at rest. Its nodes' bounds are then those of a frame of that
    /// animation.
    pub fn drawn_transformed(&self) -> bool {
        self.windows.iter().any(|w| {
            w.kind == WindowType::APPLICATION
                && w.flags.contains(WindowFlags::TRANSFORMED)
                && !w.flags.contains(WindowFlags::PICTURE_IN_PICTURE)
        })
    }

    /// Marks the windows that send no change events for a re-read after
    /// input the mirror injected at `at`, a stroke from the first point to
    /// the second when `stroke` gives them. Any input ends the edit of the
    /// input before it.
    pub fn note_action(&mut self, at: Instant, stroke: Option<(Point, Point)>) {
        self.last_action = self.last_action.max(Some(at));
        self.last_stroke = stroke.map(|(from, to)| Stroke { from, to, at });
        self.edit = None;
        for window in &mut self.windows {
            if window.reread() {
                window.action_pending = true;
            }
        }
    }

    /// Records that the input noted last was text in one piece for
    /// `element`, on a target with frames when `frames` says so: its window
    /// is read again sooner until a read shows the field's text changed (see
    /// [`Edit`]). Nothing is recorded for a field of a window that sends
    /// change events.
    pub fn note_edit(&mut self, element: ElementId, at: Instant, frames: bool) {
        let Some((w, i)) = self.locate(element) else {
            return;
        };
        let window = &self.windows[w];
        if window.reread() {
            self.edit = Some(Edit {
                window: window.id,
                element,
                text: window.nodes[i].text.clone(),
                at,
                frames,
            });
        }
    }

    /// Why `window` is due at once for the edit of a field it holds, if it
    /// is (see [`Edit`]): frames changed after its latest read began, or the
    /// target has no frames. A poll interval after the text the edit no
    /// longer calls for reads, so text that leaves its field as it was, and
    /// so shows no change, re-reads its window sooner for that long at most.
    fn edit_due(&self, window: &Win, now: Instant, config: &MirrorConfig) -> Option<ReadReason> {
        let edit = self.edit.as_ref().filter(|edit| {
            edit.window == window.id && window.reread() && now < edit.at + config.poll_interval
        })?;
        if !edit.frames {
            Some(ReadReason::Action)
        } else if self.frames_changed_since(window.read_started) {
            Some(ReadReason::Frames)
        } else {
            None
        }
    }

    /// Whether content that `stroke` scrolled sits at an end of its range:
    /// a visible node that scrolls along the stroke's axis, or along an
    /// axis its platform does not name, lies under the point the stroke
    /// came down at, and can scroll one way from where it is but not the
    /// other. Content that can scroll neither way does not scroll at all.
    pub fn stroke_reached_an_end(&self, stroke: &Stroke) -> bool {
        let (dx, dy) = (stroke.to.x - stroke.from.x, stroke.to.y - stroke.from.y);
        let axis = if dy.abs() >= dx.abs() {
            ScrollAxis::VERTICAL
        } else {
            ScrollAxis::HORIZONTAL
        };
        self.windows.iter().any(|window| {
            window.nodes.iter().any(|node| {
                let Some(scroll) = node.scroll else {
                    return false;
                };
                node.flags.contains(NodeFlags::VISIBLE)
                    && (scroll.axis == axis || scroll.axis == ScrollAxis::UNKNOWN)
                    && scroll.flags.contains(ScrollFlags::BACK)
                        != scroll.flags.contains(ScrollFlags::FORWARD)
                    && Rect::from_pixels(node.bounds, self.scale).contains(stroke.from)
            })
        })
    }

    /// The read due at `now`, if any.
    pub fn due_read(&self, now: Instant, config: &MirrorConfig) -> Option<ReadRequest> {
        if let Some(reason) = self.need_full {
            return Some(ReadRequest {
                windows: None,
                reason,
            });
        }
        let mut windows = Vec::new();
        let mut strongest = None;
        for window in &self.windows {
            let reason = if window.action_pending {
                Some(ReadReason::Action)
            } else if window.dirty {
                Some(ReadReason::Events)
            } else if !window.reread() {
                None
            } else if now >= window.read_started + config.poll_interval {
                Some(ReadReason::Poll)
            } else if let Some(reason) = self.edit_due(window, now, config) {
                Some(reason)
            } else if self.frames_changed_since(window.read_started)
                && now >= window.read_finished + config.min_reread_interval
            {
                Some(ReadReason::Frames)
            } else {
                None
            };
            if let Some(reason) = reason {
                windows.push(window.id);
                strongest = strongest.max(Some(reason));
            }
        }
        strongest.map(|reason| ReadRequest {
            windows: Some(windows),
            reason,
        })
    }

    /// The re-read due at `now` for the windows that send no change events
    /// when they are polled every `every`: after injected input at once,
    /// then once per interval. Used while waiting without frames to watch.
    pub fn due_reread(&self, now: Instant, every: Duration) -> Option<ReadRequest> {
        let mut windows = Vec::new();
        let mut strongest = None;
        for window in self.windows.iter().filter(|w| w.reread()) {
            let reason = if window.action_pending {
                Some(ReadReason::Action)
            } else if now >= window.read_started + every {
                Some(ReadReason::Poll)
            } else {
                None
            };
            if let Some(reason) = reason {
                windows.push(window.id);
                strongest = strongest.max(Some(reason));
            }
        }
        strongest.map(|reason| ReadRequest {
            windows: Some(windows),
            reason,
        })
    }

    /// When [`Replica::due_reread`] next has something to read.
    pub fn next_reread(&self, every: Duration) -> Option<Instant> {
        self.windows
            .iter()
            .filter(|w| w.reread())
            .map(|w| w.read_started + every)
            .min()
    }

    /// When the next poll or frame-driven re-read falls due, as of `now`, if
    /// ever; a window an edit calls for is due as its latest read ended.
    pub fn next_due(&self, now: Instant, config: &MirrorConfig) -> Option<Instant> {
        self.windows
            .iter()
            .filter(|w| w.reread())
            .map(|w| {
                let poll = w.read_started + config.poll_interval;
                if self.edit_due(w, now, config).is_some() {
                    w.read_finished
                } else if self.frames_changed_since(w.read_started) {
                    poll.min(w.read_finished + config.min_reread_interval)
                } else {
                    poll
                }
            })
            .min()
    }

    /// Takes a read into the replica: matches every returned window against
    /// its previous copy and records the deltas.
    pub fn integrate(
        &mut self,
        read: TreeRead,
        request: &ReadRequest,
        started: Instant,
        finished: Instant,
    ) {
        let TreeRead {
            snapshot,
            backend,
            scale,
        } = read;
        let full = request.windows.is_none();
        let first = self.revision == 0;
        let fresh = full && (self.fresh || first);
        let mut old: HashMap<u32, Win> = self.windows.drain(..).map(|w| (w.id, w)).collect();
        let mut next_windows = Vec::with_capacity(snapshot.windows.len());
        let mut changes: Vec<(u32, Changes)> = Vec::new();
        for wire in &snapshot.windows {
            let mut win =
                self.convert(wire, &snapshot.strings, (started, finished, request.reason));
            let previous = old.remove(&wire.id);
            match (&previous, fresh) {
                (Some(previous), false) => {
                    let found = reconcile(previous, &mut win, &mut self.next_element);
                    if !found.is_empty() {
                        changes.push((win.id, found));
                    }
                }
                _ => {
                    let mut added = Vec::with_capacity(win.nodes.len());
                    for node in &mut win.nodes {
                        node.element = self.alloc();
                        added.push(node.element);
                    }
                    if !fresh {
                        changes.push((
                            win.id,
                            Changes {
                                added,
                                ..Changes::default()
                            },
                        ));
                    }
                }
            }
            next_windows.push(win);
        }
        let mut rest: Vec<Win> = old.into_values().collect();
        rest.sort_by_key(|w| w.id);
        for previous in rest {
            let asked = request
                .windows
                .as_ref()
                .is_none_or(|ids| ids.contains(&previous.id));
            if !asked {
                next_windows.push(previous);
            } else if !fresh {
                changes.push((
                    previous.id,
                    Changes {
                        removed: previous.nodes.iter().map(|n| n.element).collect(),
                        ..Changes::default()
                    },
                ));
            }
        }
        next_windows.sort_by_key(|w| std::cmp::Reverse(w.layer));
        self.windows = next_windows;
        self.reindex();
        // An edit ends once a read shows its field's text changed or the
        // field gone, or starts the replica over.
        if let Some(edit) = &self.edit {
            let shown = self
                .locate(edit.element)
                .is_none_or(|(w, i)| self.windows[w].nodes[i].text != edit.text);
            if fresh || shown {
                self.edit = None;
            }
        }
        self.scale = scale;
        self.backend = Some(backend);
        self.truncated = snapshot.truncated;
        self.updated_at = Some(finished);
        self.last_read = Some(ReadSpan {
            started,
            finished,
            reason: request.reason,
        });
        if full {
            self.need_full = None;
            self.fresh = false;
            let tree = self.seqs.entry(Stream::TREE).or_insert(0);
            *tree = (*tree).max(snapshot.seq);
        }
        if fresh || !changes.is_empty() {
            self.revision += 1;
        }
        if fresh && !first {
            self.last_tree_change = self.last_tree_change.max(Some(finished));
        }
        if !changes.is_empty() {
            self.last_tree_change = self.last_tree_change.max(Some(finished));
            for (window, found) in changes {
                self.tree_changes += 1;
                self.deltas.push(TreeDelta {
                    at: finished,
                    revision: self.revision,
                    window,
                    reason: request.reason,
                    added: found.added,
                    removed: found.removed,
                    changed: found.changed,
                });
            }
            if self.deltas.len() > DELTA_LOG {
                let excess = self.deltas.len() - DELTA_LOG;
                self.deltas.drain(..excess);
            }
        }
    }

    fn intern(&mut self, class: &str) -> Arc<str> {
        if let Some(interned) = self.interner.get(class) {
            return Arc::clone(interned);
        }
        let interned: Arc<str> = Arc::from(class);
        self.interner
            .insert(class.to_owned(), Arc::clone(&interned));
        interned
    }

    /// A window of a read with its metadata: without `emits_events` a
    /// window holding a node of a class under `COMPOSE_PACKAGE` is taken
    /// to send no change events, and without `owner` the target owns it.
    fn convert(
        &mut self,
        wire: &Window,
        strings: &[String],
        (started, finished, reason): (Instant, Instant, ReadReason),
    ) -> Win {
        let nodes = match &wire.root {
            Some(root) => self.convert_nodes(root, strings),
            None => Vec::new(),
        };
        let emits_events = wire
            .emits_events
            .unwrap_or_else(|| !nodes.iter().any(|n| n.class.starts_with(COMPOSE_PACKAGE)));
        let owner = wire.owner.map_or(Owner::Target, |i| {
            Owner::System(strings.get(i as usize).cloned().unwrap_or_default())
        });
        Win {
            id: wire.id,
            kind: wire.kind,
            layer: wire.layer,
            package: wire.package.and_then(|i| strings.get(i as usize)).cloned(),
            title: wire.title.clone(),
            bounds: wire.bounds,
            flags: wire.flags,
            emits_events,
            owner,
            modal: wire.modal,
            nodes,
            dirty: false,
            action_pending: false,
            read_started: started,
            read_finished: finished,
            reason,
        }
    }

    fn convert_nodes(&mut self, root: &Node, strings: &[String]) -> Vec<RNode> {
        let mut nodes: Vec<RNode> = Vec::new();
        let mut stack: Vec<(&Node, Option<usize>, usize)> = vec![(root, None, 0)];
        while let Some((node, parent, depth)) = stack.pop() {
            let index = nodes.len();
            let class = strings.get(node.class as usize).map_or("", String::as_str);
            let class = self.intern(class);
            nodes.push(RNode {
                element: UNASSIGNED,
                node_id: node.id,
                parent,
                children: Vec::with_capacity(node.children.len()),
                depth,
                class,
                identifier: node.res_id.clone(),
                text: node.text.clone(),
                desc: node.desc.clone(),
                bounds: node.bounds,
                flags: node.flags,
                scroll: node.scroll,
                content: 0,
            });
            if let Some(parent) = parent {
                nodes[parent].children.push(index);
            }
            for child in node.children.iter().rev() {
                stack.push((child, Some(index), depth + 1));
            }
        }
        for i in (0..nodes.len()).rev() {
            let mut h = DefaultHasher::new();
            let node = &nodes[i];
            node.class.hash(&mut h);
            node.identifier.hash(&mut h);
            node.desc.hash(&mut h);
            if !node.editable() {
                node.text.hash(&mut h);
            }
            for &child in &node.children {
                nodes[child].content.hash(&mut h);
            }
            nodes[i].content = h.finish();
        }
        nodes
    }

    /// The replica as text: one line per window, then its nodes indented by
    /// depth, bounds in points.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for w in &self.windows {
            let owner = match &w.owner {
                Owner::Target => "target".to_owned(),
                Owner::System(name) => format!("system:{name}"),
            };
            let _ = write!(
                out,
                "window {} {} layer={} owner={owner} events={} modal={}",
                w.id,
                w.kind.name().unwrap_or("unknown"),
                w.layer,
                w.emits_events,
                w.modal
            );
            if let Some(package) = &w.package {
                let _ = write!(out, " package={package}");
            }
            if let Some(title) = &w.title {
                let _ = write!(out, " title={title:?}");
            }
            let _ = writeln!(out, " bounds={}", Rect::from_pixels(w.bounds, self.scale));
            for n in &w.nodes {
                let _ = write!(out, "{}{}", "  ".repeat(n.depth + 1), n.class);
                if let Some(id) = &n.identifier {
                    let _ = write!(out, " id={id}");
                }
                if let Some(text) = &n.text {
                    let _ = write!(out, " text={text:?}");
                }
                if let Some(desc) = &n.desc {
                    let _ = write!(out, " desc={desc:?}");
                }
                let _ = writeln!(
                    out,
                    " {} {}",
                    Rect::from_pixels(n.bounds, self.scale),
                    n.flags
                );
            }
        }
        out
    }
}

/// Matches `new` against `previous`, the same window's last read: matched
/// nodes keep their identity, the others get fresh ones.
fn reconcile(previous: &Win, new: &mut Win, next: &mut u64) -> Changes {
    let mut from: Vec<Option<usize>> = vec![None; new.nodes.len()];
    let mut matched_old = vec![false; previous.nodes.len()];
    let mut stack = Vec::new();
    if let (Some(o), Some(n)) = (previous.nodes.first(), new.nodes.first())
        && o.class == n.class
        && o.identifier == n.identifier
    {
        stack.push((0, 0));
    }
    while let Some((o, n)) = stack.pop() {
        from[n] = Some(o);
        matched_old[o] = true;
        new.nodes[n].element = previous.nodes[o].element;
        stack.extend(match_children(&previous.nodes, o, &new.nodes, n));
    }
    let mut changes = Changes::default();
    for (n, node) in new.nodes.iter_mut().enumerate() {
        match from[n] {
            Some(o) => {
                let old = &previous.nodes[o];
                if old.text != node.text
                    || old.desc != node.desc
                    || old.bounds != node.bounds
                    || old.flags != node.flags
                {
                    changes.changed.push(node.element);
                }
            }
            None => {
                *next += 1;
                node.element = ElementId(*next);
                changes.added.push(node.element);
            }
        }
    }
    for (o, node) in previous.nodes.iter().enumerate() {
        if !matched_old[o] {
            changes.removed.push(node.element);
        }
    }
    let window_changed = previous.title != new.title
        || previous.bounds != new.bounds
        || previous.flags != new.flags
        || previous.kind != new.kind
        || previous.layer != new.layer;
    if let (true, Some(root)) = (window_changed, new.nodes.first())
        && !changes.changed.contains(&root.element)
        && !changes.added.contains(&root.element)
    {
        changes.changed.push(root.element);
    }
    changes
}

/// Pairs the children of old node `o` with those of new node `n`.
fn match_children(old: &[RNode], o: usize, new: &[RNode], n: usize) -> Vec<(usize, usize)> {
    type Key<'a> = (&'a str, Option<&'a str>);
    let mut groups: HashMap<Key<'_>, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for &c in &old[o].children {
        let key = (&*old[c].class, old[c].identifier.as_deref());
        groups.entry(key).or_default().0.push(c);
    }
    for &c in &new[n].children {
        let key = (&*new[c].class, new[c].identifier.as_deref());
        groups.entry(key).or_default().1.push(c);
    }
    let mut pairs = Vec::new();
    for (olds, news) in groups.into_values() {
        if olds.is_empty() || news.is_empty() {
            continue;
        }
        if olds.len() == 1 && news.len() == 1 {
            pairs.push((olds[0], news[0]));
            continue;
        }
        let mut by_content: HashMap<u64, VecDeque<usize>> = HashMap::new();
        for &c in &olds {
            by_content.entry(old[c].content).or_default().push_back(c);
        }
        let mut used = HashSet::new();
        let mut left_new = Vec::new();
        for &c in &news {
            match by_content
                .get_mut(&new[c].content)
                .and_then(VecDeque::pop_front)
            {
                Some(m) => {
                    used.insert(m);
                    pairs.push((m, c));
                }
                None => left_new.push(c),
            }
        }
        let left_old: Vec<usize> = olds.into_iter().filter(|c| !used.contains(c)).collect();
        if left_old.is_empty() || left_new.is_empty() {
            continue;
        }
        let old_ids: Vec<HashSet<&str>> = left_old
            .iter()
            .map(|&c| subtree_identifiers(old, c))
            .collect();
        let mut taken = vec![false; left_old.len()];
        for &c in &left_new {
            let ids = subtree_identifiers(new, c);
            if ids.is_empty() {
                continue;
            }
            if let Some(i) =
                (0..left_old.len()).find(|&i| !taken[i] && !old_ids[i].is_disjoint(&ids))
            {
                taken[i] = true;
                pairs.push((left_old[i], c));
            }
        }
    }
    pairs
}

/// The identifiers of the nodes below `root`.
fn subtree_identifiers(nodes: &[RNode], root: usize) -> HashSet<&str> {
    let mut ids = HashSet::new();
    let mut stack: Vec<usize> = nodes[root].children.clone();
    while let Some(i) = stack.pop() {
        if let Some(id) = &nodes[i].identifier {
            ids.insert(id.as_str());
        }
        stack.extend(nodes[i].children.iter().copied());
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: &str = "android.widget.FrameLayout";
    const TEXT: &str = "android.widget.TextView";
    const BUTTON: &str = "android.widget.Button";
    const GROUP: &str = "android.view.ViewGroup";
    const IMAGE: &str = "android.widget.ImageView";
    const SWITCH: &str = "android.widget.Switch";
    const EDIT: &str = "android.widget.EditText";
    const PROGRESS: &str = "android.widget.ProgressBar";
    const CHECKBOX: &str = "android.widget.CheckBox";

    /// The string table of the reads below: their nodes' classes.
    const CLASSES: [&str; 9] = [
        FRAME, TEXT, BUTTON, GROUP, IMAGE, SWITCH, EDIT, PROGRESS, CHECKBOX,
    ];

    /// A node of `class`, one of [`CLASSES`], with the identifier and the
    /// text given, holding `children`.
    fn node(
        class: &str,
        identifier: Option<&str>,
        text: Option<&str>,
        children: Vec<Node>,
    ) -> Node {
        let class = CLASSES
            .iter()
            .position(|&c| c == class)
            .expect("a class of the table");
        Node {
            id: 0,
            class: class as u32,
            res_pkg: None,
            res_id: identifier.map(str::to_owned),
            text: text.map(str::to_owned),
            desc: None,
            bounds: Bounds::default(),
            flags: NodeFlags::EMPTY,
            children,
            scroll: None,
        }
    }

    fn identified(class: &str, identifier: &str) -> Node {
        node(class, Some(identifier), None, Vec::new())
    }

    fn text(shown: &str) -> Node {
        node(TEXT, None, Some(shown), Vec::new())
    }

    /// A list row: a view group identified as `row`, as every row is,
    /// holding a text view identified as `label` that shows `shown`.
    fn row(label: &str, shown: &str) -> Node {
        let held = node(TEXT, Some(label), Some(shown), Vec::new());
        node(GROUP, Some("row"), None, vec![held])
    }

    /// A window's nodes as the replica holds them: `children` under a root.
    fn window(children: Vec<Node>) -> Vec<RNode> {
        let strings = CLASSES.map(String::from);
        Replica::new().convert_nodes(&node(FRAME, None, None, children), &strings)
    }

    /// What node `i` shows: its text, its first child's, or its identifier.
    fn label(nodes: &[RNode], i: usize) -> &str {
        let node = &nodes[i];
        let first_child = node.children.first().map(|&c| &nodes[c]);
        node.text
            .as_deref()
            .or_else(|| first_child?.text.as_deref())
            .or(node.identifier.as_deref())
            .expect("a label")
    }

    /// The children of two matched nodes pair in the order their class and
    /// identifier first appear among the old node's children, whatever the
    /// new node's order, and so in the same order on every call; within a
    /// class and identifier, in the order the rules pair them: subtree
    /// content, then a shared identified descendant. `reconcile` walks the
    /// pairs in that order, so what its stack allocates is the same for the
    /// same two reads, as design D14's exact allocation counts need.
    #[test]
    fn children_pair_in_the_order_their_keys_first_appear_on_every_call() {
        let old = window(vec![
            text("Title"),
            identified(BUTTON, "save"),
            row("row-1-label", "One"),
            text("Subtitle"),
            identified(IMAGE, "logo"),
            row("row-2-label", "Two"),
            identified(SWITCH, "wifi"),
            identified(EDIT, "name"),
            identified(PROGRESS, "spinner"),
            identified(CHECKBOX, "terms"),
        ]);
        let new = window(vec![
            identified(CHECKBOX, "terms"),
            identified(SWITCH, "wifi"),
            row("row-2-label", "Two (done)"),
            text("Subtitle"),
            identified(EDIT, "name"),
            row("row-1-label", "One (done)"),
            text("Title"),
            identified(IMAGE, "logo"),
            identified(BUTTON, "save"),
            identified(PROGRESS, "loading"),
        ]);

        let first = match_children(&old, 0, &new, 0);
        let second = match_children(&old, 0, &new, 0);
        assert_eq!(first, second, "the same pairs in the same order");
        let named: Vec<(&str, &str)> = first
            .iter()
            .map(|&(o, n)| (label(&old, o), label(&new, n)))
            .collect();
        assert_eq!(
            named,
            [
                ("Subtitle", "Subtitle"),
                ("Title", "Title"),
                ("save", "save"),
                ("Two", "Two (done)"),
                ("One", "One (done)"),
                ("logo", "logo"),
                ("wifi", "wifi"),
                ("name", "name"),
                ("terms", "terms"),
            ]
        );
    }
}
