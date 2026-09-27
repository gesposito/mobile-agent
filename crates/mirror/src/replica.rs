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
use std::time::Instant;

use protocol::event::{EventPayload, LifecycleKind, Stream};
use protocol::snapshot::{Bounds, Node, NodeFlags, Truncation, Window, WindowFlags, WindowType};

use crate::backend::{
    BackendInfo, FrameMeta, Owner, ReadReason, ReadRequest, StreamItem, Timed, TreeRead, WindowMeta,
};
use crate::geometry::Rect;
use crate::mirror::MirrorConfig;
use crate::snapshot::{ElementId, TreeDelta};

/// The view class that hosts Jetpack Compose content, which sends no change
/// events to a UiAutomation client (spike S1).
pub(crate) const COMPOSE_HOST: &str = "androidx.compose.ui.platform.AndroidComposeView";

/// How many deltas the replica keeps.
const DELTA_LOG: usize = 256;

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
    /// A lossless stream skipped numbers.
    Gap,
}

/// One target's replica.
pub(crate) struct Replica {
    pub windows: Vec<Win>,
    /// Grows with every read that changes the replica.
    pub revision: u64,
    pub deltas: Vec<TreeDelta>,
    /// When a tree event arrived or a read found a delta, whichever is later.
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
    next_element: u64,
    /// The last sequence number accounted for, per lossless stream.
    seqs: HashMap<Stream, u64>,
    pub frame: Option<FrameMeta>,
    pub last_frame_change: Option<Instant>,
    pub frame_changes: u64,
    pub last_action: Option<Instant>,
    pub tree_events: u64,
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
            next_element: 0,
            seqs: HashMap::new(),
            frame: None,
            last_frame_change: None,
            frame_changes: 0,
            last_action: None,
            tree_events: 0,
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
            StreamItem::Event(event) => match self.check_seq(event.stream(), event.seq) {
                Seq::Skip => {}
                Seq::Apply | Seq::Gap => self.apply_event(at, &event.payload),
            },
            StreamItem::Frame(meta) => self.apply_frame(meta),
            _ => {}
        }
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
            *last = seq;
            Seq::Gap
        }
    }

    fn apply_event(&mut self, at: Instant, payload: &EventPayload) {
        match payload {
            EventPayload::Tree(change) => {
                self.tree_events += 1;
                self.last_tree_change = self.last_tree_change.max(Some(at));
                match self.windows.iter_mut().find(|w| w.id == change.window) {
                    Some(window) => window.dirty = true,
                    None => self.request_full(ReadReason::Events),
                }
            }
            EventPayload::Window(_) => {
                self.tree_events += 1;
                self.last_tree_change = self.last_tree_change.max(Some(at));
                self.request_full(ReadReason::Events);
            }
            EventPayload::Lifecycle(lifecycle) if lifecycle.change == LifecycleKind::READY => {
                self.request_full(ReadReason::Events);
            }
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
        }
        self.frame = Some(meta);
    }

    fn frames_changed_since(&self, t: Instant) -> bool {
        self.last_frame_change.is_some_and(|c| c > t)
    }

    /// Marks the windows that send no change events for a re-read after
    /// input the mirror injected at `at`.
    pub fn note_action(&mut self, at: Instant) {
        self.last_action = self.last_action.max(Some(at));
        for window in &mut self.windows {
            if window.reread() {
                window.action_pending = true;
            }
        }
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

    /// When the next poll or frame-driven re-read falls due, if ever.
    pub fn next_due(&self, config: &MirrorConfig) -> Option<Instant> {
        self.windows
            .iter()
            .filter(|w| w.reread())
            .map(|w| {
                let poll = w.read_started + config.poll_interval;
                if self.frames_changed_since(w.read_started) {
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
            windows: metas,
            backend,
            scale,
            ..
        } = read;
        let full = request.windows.is_none();
        let first = self.revision == 0;
        let fresh = full && (self.fresh || first);
        let mut metas: HashMap<u32, WindowMeta> =
            metas.into_iter().map(|m| (m.window, m)).collect();
        let mut old: HashMap<u32, Win> = self.windows.drain(..).map(|w| (w.id, w)).collect();
        let mut next_windows = Vec::with_capacity(snapshot.windows.len());
        let mut changes: Vec<(u32, Changes)> = Vec::new();
        for wire in &snapshot.windows {
            let meta = metas
                .remove(&wire.id)
                .unwrap_or_else(|| WindowMeta::default_for(wire.id));
            let mut win = self.convert(
                wire,
                &snapshot.strings,
                meta,
                (started, finished, request.reason),
            );
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
        self.scale = scale;
        self.backend = Some(backend);
        self.truncated = snapshot.truncated;
        self.updated_at = Some(finished);
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

    fn convert(
        &mut self,
        wire: &Window,
        strings: &[String],
        meta: WindowMeta,
        (started, finished, reason): (Instant, Instant, ReadReason),
    ) -> Win {
        let nodes = match &wire.root {
            Some(root) => self.convert_nodes(root, strings),
            None => Vec::new(),
        };
        let emits_events = meta
            .emits_events
            .unwrap_or_else(|| !nodes.iter().any(|n| &*n.class == COMPOSE_HOST));
        Win {
            id: wire.id,
            kind: wire.kind,
            layer: wire.layer,
            package: wire.package.and_then(|i| strings.get(i as usize)).cloned(),
            title: wire.title.clone(),
            bounds: wire.bounds,
            flags: wire.flags,
            emits_events,
            owner: meta.owner,
            modal: meta.modal,
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
