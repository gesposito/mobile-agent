//! A scripted target for tests: a [`FakeTarget`] plays a timeline of screen
//! changes, events and frames against a fake clock, so the mirror can be
//! exercised without a device and without sleeping.
//!
//! Time moves only when the mirror waits on the event stream (the clock jumps
//! to the next scripted step or to the deadline), when a read or an input
//! costs time ([`FakeTarget::read_cost`], [`FakeTarget::input_cost`]), or when
//! a test calls [`FakeTarget::advance`]. A step scheduled at or before the
//! current time has happened: its tree is what a read returns, its event
//! waits in the stream.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use protocol::event::{Event, EventPayload, Stream};
use protocol::message::{Rotation, TextMethod};
use protocol::snapshot::{Node, NodeFlags, Window};

use crate::backend::{
    Backend, BackendError, Clock, EncodeRequest, EventStream, FrameMeta, FrameSource, Input,
    InputAck, InputSink, ReadRequest, StreamItem, Timed, TreeRead, TreeSource,
};

/// Something that happens on the scripted target at a given time.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// From then on the target shows this tree; nothing is reported.
    Show(TreeRead),
    /// An event with its stream's next sequence number.
    Event(EventPayload),
    /// An event with an explicit sequence number, to script repeats; later
    /// automatic numbers continue after it.
    EventSeq(u64, EventPayload),
    /// An event the target sent and the host never received: its stream's
    /// next number is used up without anything arriving.
    Drop(Stream),
    /// A frame with this hash.
    Frame(u64),
    /// Any other stream item, delivered as it is.
    Item(StreamItem),
}

/// A read the mirror made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadRecord {
    /// When the read started, from the start of the script.
    pub at: Duration,
    pub request: ReadRequest,
}

/// An input the mirror injected.
#[derive(Clone, Debug, PartialEq)]
pub struct InputRecord {
    /// When it was injected, from the start of the script.
    pub at: Duration,
    pub input: Input,
    /// For a tap, the identifier of the innermost identified node under it.
    pub hit: Option<String>,
}

/// A scripted target. Clones share the same script and clock.
#[derive(Clone)]
pub struct FakeTarget {
    state: Arc<Mutex<State>>,
}

struct Scheduled {
    at: Instant,
    order: u64,
    step: Step,
}

struct State {
    start: Instant,
    now: Instant,
    screen: TreeRead,
    timeline: Vec<Scheduled>,
    order: u64,
    outbox: VecDeque<Timed<StreamItem>>,
    next_seq: HashMap<Stream, u64>,
    sent_tree_seq: u64,
    frame_seq: u64,
    frame_size: (u32, u32, Rotation),
    frames: Vec<FrameMeta>,
    read_cost: Duration,
    input_cost: Duration,
    delivery_delay: Duration,
    whole_reads: bool,
    read_failures: VecDeque<BackendError>,
    reads: Vec<ReadRecord>,
    inputs: Vec<InputRecord>,
    encodes: Vec<EncodeRequest>,
    reactions: Vec<(String, Vec<(Duration, Step)>)>,
}

impl FakeTarget {
    /// A target showing `screen`, its clock at the start of the script.
    pub fn new(screen: TreeRead) -> FakeTarget {
        let start = Instant::now();
        FakeTarget {
            state: Arc::new(Mutex::new(State {
                start,
                now: start,
                screen,
                timeline: Vec::new(),
                order: 0,
                outbox: VecDeque::new(),
                next_seq: HashMap::new(),
                sent_tree_seq: 0,
                frame_seq: 0,
                frame_size: (1206, 2622, Rotation::R0),
                frames: Vec::new(),
                read_cost: Duration::ZERO,
                input_cost: Duration::ZERO,
                delivery_delay: Duration::ZERO,
                whole_reads: false,
                read_failures: VecDeque::new(),
                reads: Vec::new(),
                inputs: Vec::new(),
                encodes: Vec::new(),
                reactions: Vec::new(),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("the fake target's state is never poisoned")
    }

    /// The backend the mirror drives, with this target's fake clock.
    pub fn backend(&self) -> Backend {
        Backend {
            tree: Box::new(self.clone()),
            events: Box::new(self.clone()),
            frames: Some(Box::new(self.clone())),
            input: Box::new(self.clone()),
            clock: Box::new(self.clock()),
        }
    }

    /// The same backend without a frame source, as on an Android device.
    pub fn backend_without_frames(&self) -> Backend {
        Backend {
            frames: None,
            ..self.backend()
        }
    }

    /// The target's clock.
    pub fn clock(&self) -> FakeClock {
        FakeClock {
            state: Arc::clone(&self.state),
        }
    }

    /// The instant the script starts at.
    pub fn start(&self) -> Instant {
        self.lock().start
    }

    /// The time on the fake clock, from the start of the script.
    pub fn elapsed(&self) -> Duration {
        let s = self.lock();
        s.now - s.start
    }

    /// The instant `offset` after the start of the script.
    pub fn instant(&self, offset: Duration) -> Instant {
        self.lock().start + offset
    }

    /// Schedules `step` at `offset` from the start of the script.
    pub fn at(&self, offset: Duration, step: Step) {
        let mut s = self.lock();
        let at = s.start + offset;
        s.schedule(at, step);
    }

    /// Schedules `step` `delay` after the current time.
    pub fn after(&self, delay: Duration, step: Step) {
        let mut s = self.lock();
        let at = s.now + delay;
        s.schedule(at, step);
    }

    /// When a tap's innermost identified node is `identifier` (the node
    /// itself or an unidentified node inside it), the target plays `steps`,
    /// each at its delay after the tap.
    pub fn on_tap(&self, identifier: &str, steps: Vec<(Duration, Step)>) {
        self.lock().reactions.push((identifier.to_owned(), steps));
    }

    /// Moves the clock forward without anything waiting on it.
    pub fn advance(&self, by: Duration) {
        let mut s = self.lock();
        let to = s.now + by;
        s.now = to;
        s.happen(to);
    }

    /// How long each read takes.
    pub fn read_cost(&self, cost: Duration) {
        self.lock().read_cost = cost;
    }

    /// How long each input takes to be acknowledged.
    pub fn input_cost(&self, cost: Duration) {
        self.lock().input_cost = cost;
    }

    /// How long an item takes from the target to the host: a read made in
    /// between already reflects it, as a snapshot response can overtake the
    /// events queued before it.
    pub fn delivery_delay(&self, delay: Duration) {
        self.lock().delivery_delay = delay;
    }

    /// Whether reads return every window, as the Android agent does, rather
    /// than only the requested ones.
    pub fn whole_reads(&self, whole: bool) {
        self.lock().whole_reads = whole;
    }

    /// The size and rotation of the frames the target captures.
    pub fn frame_size(&self, width: u32, height: u32, rotation: Rotation) {
        self.lock().frame_size = (width, height, rotation);
    }

    /// Makes the next read fail with `error`.
    pub fn fail_next_read(&self, error: BackendError) {
        self.lock().read_failures.push_back(error);
    }

    /// The tree the target shows now.
    pub fn screen(&self) -> TreeRead {
        let mut s = self.lock();
        let now = s.now;
        s.happen(now);
        s.screen.clone()
    }

    /// Every read so far, oldest first.
    pub fn reads(&self) -> Vec<ReadRecord> {
        self.lock().reads.clone()
    }

    /// Every input so far, oldest first.
    pub fn inputs(&self) -> Vec<InputRecord> {
        self.lock().inputs.clone()
    }

    /// Every encode request so far, oldest first.
    pub fn encodes(&self) -> Vec<EncodeRequest> {
        self.lock().encodes.clone()
    }
}

impl State {
    fn schedule(&mut self, at: Instant, step: Step) {
        self.order += 1;
        let order = self.order;
        let index = self
            .timeline
            .partition_point(|s| (s.at, s.order) <= (at, order));
        self.timeline.insert(index, Scheduled { at, order, step });
    }

    /// Plays every step due at or before `t`.
    fn happen(&mut self, t: Instant) {
        while self.timeline.first().is_some_and(|s| s.at <= t) {
            let Scheduled { at, step, .. } = self.timeline.remove(0);
            self.play(at, step);
        }
    }

    fn play(&mut self, at: Instant, step: Step) {
        match step {
            Step::Show(tree) => self.screen = tree,
            Step::Event(payload) => {
                let stream = payload.stream();
                let seq = self.take_seq(stream);
                self.send_event(at, seq, payload);
            }
            Step::EventSeq(seq, payload) => {
                let stream = payload.stream();
                let next = self.next_seq.entry(stream).or_insert(1);
                *next = (*next).max(seq + 1);
                self.send_event(at, seq, payload);
            }
            Step::Drop(stream) => {
                let seq = self.take_seq(stream);
                if stream == Stream::TREE {
                    self.sent_tree_seq = self.sent_tree_seq.max(seq);
                }
            }
            Step::Frame(hash) => {
                self.frame_seq += 1;
                let (width, height, rotation) = self.frame_size;
                let meta = FrameMeta {
                    seq: self.frame_seq,
                    at,
                    hash,
                    width,
                    height,
                    rotation,
                };
                self.frames.push(meta);
                self.deliver(at, StreamItem::Frame(meta));
            }
            Step::Item(item) => {
                if item == StreamItem::Reconnected {
                    // A new connection numbers its events from 1 again.
                    self.next_seq.clear();
                    self.sent_tree_seq = 0;
                }
                self.deliver(at, item);
            }
        }
    }

    /// Queues `item`, sent at `at`, to arrive after the delivery delay.
    fn deliver(&mut self, at: Instant, item: StreamItem) {
        let at = at + self.delivery_delay;
        self.outbox.push_back(Timed { at, item });
    }

    fn take_seq(&mut self, stream: Stream) -> u64 {
        let next = self.next_seq.entry(stream).or_insert(1);
        let seq = *next;
        *next += 1;
        seq
    }

    fn send_event(&mut self, at: Instant, seq: u64, payload: EventPayload) {
        if payload.stream() == Stream::TREE {
            self.sent_tree_seq = self.sent_tree_seq.max(seq);
        }
        let time_us = u64::try_from((at - self.start).as_micros()).unwrap_or(u64::MAX);
        self.deliver(
            at,
            StreamItem::Event(Event {
                seq,
                time_us,
                payload,
            }),
        );
    }

    fn hit(&self, x: f64, y: f64) -> Option<String> {
        let scale = if self.screen.scale > 0.0 {
            self.screen.scale
        } else {
            1.0
        };
        let (px, py) = ((x * scale).round() as i32, (y * scale).round() as i32);
        for window in &self.screen.snapshot.windows {
            let b = window.bounds;
            if px < b.left || px >= b.right || py < b.top || py >= b.bottom {
                continue;
            }
            if let Some(root) = &window.root
                && let Some(found) = hit_node(root, px, py, None)
            {
                return found;
            }
        }
        None
    }
}

/// The innermost visible node under the point, as the identifier of the
/// innermost identified node on its path; `None` when nothing is hit.
fn hit_node(node: &Node, x: i32, y: i32, identified: Option<&str>) -> Option<Option<String>> {
    let b = node.bounds;
    let inside = x >= b.left && x < b.right && y >= b.top && y < b.bottom;
    if !inside || !node.flags.contains(NodeFlags::VISIBLE) {
        return None;
    }
    let identified = node.res_id.as_deref().or(identified);
    for child in node.children.iter().rev() {
        if let Some(found) = hit_node(child, x, y, identified) {
            return Some(found);
        }
    }
    Some(identified.map(str::to_owned))
}

impl TreeSource for FakeTarget {
    fn read(&mut self, request: &ReadRequest) -> Result<TreeRead, BackendError> {
        let mut s = self.lock();
        let now = s.now;
        s.happen(now);
        let at = now - s.start;
        s.reads.push(ReadRecord {
            at,
            request: request.clone(),
        });
        let cost = s.read_cost;
        s.now += cost;
        if let Some(error) = s.read_failures.pop_front() {
            return Err(error);
        }
        let mut read = s.screen.clone();
        read.snapshot.seq = s.sent_tree_seq;
        if let (Some(ids), false) = (&request.windows, s.whole_reads) {
            read.snapshot.windows.retain(|w| ids.contains(&w.id));
            read.windows.retain(|m| ids.contains(&m.window));
            read.scroll.retain(|i| ids.contains(&i.window));
            read.snapshot.node_count = read.snapshot.windows.iter().map(count_window).sum();
        }
        Ok(read)
    }
}

fn count_window(window: &Window) -> u32 {
    fn count(node: &Node) -> u32 {
        1 + node.children.iter().map(count).sum::<u32>()
    }
    window.root.as_ref().map_or(0, count)
}

impl EventStream for FakeTarget {
    fn next(&mut self, deadline: Instant) -> Option<Timed<StreamItem>> {
        let mut s = self.lock();
        loop {
            let now = s.now;
            s.happen(now);
            if s.outbox.front().is_some_and(|item| item.at <= now) {
                return s.outbox.pop_front();
            }
            let step = s.timeline.first().map(|step| step.at);
            let arrival = s.outbox.front().map(|item| item.at);
            match [step, arrival].into_iter().flatten().min() {
                Some(upcoming) if upcoming <= deadline => {
                    s.now = s.now.max(upcoming);
                }
                _ => {
                    s.now = s.now.max(deadline);
                    let now = s.now;
                    s.happen(now);
                    return if s.outbox.front().is_some_and(|item| item.at <= now) {
                        s.outbox.pop_front()
                    } else {
                        None
                    };
                }
            }
        }
    }
}

impl FrameSource for FakeTarget {
    fn latest(&self) -> Option<FrameMeta> {
        let mut s = self.lock();
        let now = s.now;
        s.happen(now);
        s.frames.last().copied()
    }

    fn encode(&mut self, request: &EncodeRequest) -> Result<Vec<u8>, BackendError> {
        let mut s = self.lock();
        s.encodes.push(*request);
        Ok(format!(
            "jpeg {}x{} q{} frame {}",
            request.width, request.height, request.quality, request.seq
        )
        .into_bytes())
    }
}

impl InputSink for FakeTarget {
    fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError> {
        let mut s = self.lock();
        let now = s.now;
        s.happen(now);
        let hit = match input {
            Input::Tap { at, .. } => s.hit(at.x, at.y),
            _ => None,
        };
        if let Some(target) = &hit {
            let steps: Vec<(Duration, Step)> = s
                .reactions
                .iter()
                .filter(|(id, _)| id == target)
                .flat_map(|(_, steps)| steps.iter().cloned())
                .collect();
            for (delay, step) in steps {
                s.schedule(now + delay, step);
            }
        }
        let at = now - s.start;
        s.inputs.push(InputRecord {
            at,
            input: input.clone(),
            hit,
        });
        let cost = s.input_cost;
        s.now += cost;
        let method = match input {
            Input::Text { method, .. } if *method == TextMethod::AUTO => Some(TextMethod::DIRECT),
            Input::Text { method, .. } => Some(*method),
            _ => None,
        };
        Ok(InputAck {
            accepted: true,
            method,
        })
    }
}

/// The fake target's clock, shared with the target.
#[derive(Clone)]
pub struct FakeClock {
    state: Arc<Mutex<State>>,
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.state
            .lock()
            .expect("the fake target's state is never poisoned")
            .now
    }
}
