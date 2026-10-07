//! Recorded fixtures for the mirror's tests.
//!
//! A fixture under `fixtures/` is a JSON tree in device points with the
//! scale factor beside it; [`Screen`] expands its templates (repeated rows
//! and grid cells, viewports that hide what lies outside them), applies
//! patches, and turns it into the `TreeRead` a driver would return, bounds
//! in physical pixels. A timeline fixture scripts a [`FakeTarget`]: screen
//! patches, events with their sequence numbers (gaps included), frames and
//! stream-state items at given times.
//!
//! The crate's `README.md` describes the format.

#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use mirror::backend::{BackendInfo, StreamItem, TreeRead};
use mirror::testing::{FakeTarget, Step};
use mirror::{Mirror, MirrorConfig};
use protocol::event::{
    EventPayload, Lifecycle, LifecycleKind, Stream, TreeChange, TreeChangeKind, WindowChange,
    WindowChangeKind,
};
use protocol::snapshot::{
    Bounds, Node, NodeFlags, Scroll, ScrollAxis, ScrollFlags, ScrollPosition, ScrollRange,
    ScrollUnit, Snapshot, Truncation, Window, WindowFlags, WindowType,
};
use serde_json::{Map, Value, json};

pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn read_json(name: &str) -> Value {
    let path = fixtures_dir().join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("fixture {name} is not JSON: {e}"))
}

/// The `TreeRead` of fixture `name`.
pub fn fixture(name: &str) -> TreeRead {
    Screen::load(name).read()
}

/// A mirror over a fake target showing fixture `name`, with default settings.
pub fn mirror_on(name: &str) -> (FakeTarget, Mirror) {
    let target = FakeTarget::new(fixture(name));
    let mirror = Mirror::new(target.backend(), MirrorConfig::default());
    (target, mirror)
}

/// What a fresh mirror holds after one read of `read`: the tree a cold
/// acquisition yields, for comparing a replica against.
pub fn cold_dump(read: &TreeRead) -> String {
    let target = FakeTarget::new(read.clone());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().expect("a cold read of a fixture succeeds");
    mirror.dump()
}

/// An expanded fixture: templates unrolled, every node concrete.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen(pub Value);

impl Screen {
    pub fn load(name: &str) -> Screen {
        Screen::from_json(read_json(name))
    }

    pub fn from_json(value: Value) -> Screen {
        let mut value = value;
        let windows = value["windows"]
            .as_array_mut()
            .expect("a fixture has a windows array");
        for window in windows {
            if let Some(root) = window.get_mut("root")
                && !root.is_null()
            {
                let mut nodes = expand(root, None);
                assert_eq!(nodes.len(), 1, "a window root expands to one node");
                *root = nodes.remove(0);
            }
        }
        Screen(value)
    }

    /// The read a driver would return for this screen.
    pub fn read(&self) -> TreeRead {
        to_read(&self.0)
    }

    /// Applies patch operations in order.
    pub fn patch(&mut self, ops: &Value) {
        for op in ops.as_array().expect("a patch is an array of operations") {
            self.apply(op);
        }
    }

    /// This screen with `ops` applied.
    pub fn patched(&self, ops: Value) -> Screen {
        let mut next = self.clone();
        next.patch(&ops);
        next
    }

    fn apply(&mut self, op: &Value) {
        if let Some(name) = op.get("screen") {
            let name = name.as_str().expect("screen names a fixture");
            *self = Screen::load(name);
        } else if let Some(backend) = op.get("backend") {
            self.0["backend"] = backend.clone();
        } else if let Some(id) = op.get("find") {
            let id = id.as_str().expect("find names an identifier");
            let node = self.find_mut(id).unwrap_or_else(|| panic!("no node {id}"));
            set_fields(node, op);
        } else if let Some(id) = op.get("remove") {
            let id = id.as_str().expect("remove names an identifier");
            assert!(self.remove(id), "no node {id} to remove");
        } else if let Some(parent) = op.get("insert") {
            let parent = parent
                .as_str()
                .expect("insert names the parent's identifier");
            let mut nodes = expand(&op["node"], None);
            let target = self
                .find_mut(parent)
                .unwrap_or_else(|| panic!("no parent {parent}"));
            let children = children_mut(target);
            let index = op["index"]
                .as_u64()
                .map_or(children.len(), |i| i as usize)
                .min(children.len());
            for (offset, node) in nodes.drain(..).enumerate() {
                children.insert(index + offset, node);
            }
        } else if let Some(sibling) = op.get("insert_before") {
            let sibling = sibling
                .as_str()
                .expect("insert_before names a sibling's identifier");
            let nodes = expand(&op["node"], None);
            let inserted = self.0["windows"]
                .as_array_mut()
                .expect("windows")
                .iter_mut()
                .any(|w| insert_before(&mut w["root"], sibling, &nodes));
            assert!(inserted, "no node {sibling} to insert before");
        } else if let Some(window) = op.get("add_window") {
            let mut window = window.clone();
            if let Some(root) = window.get_mut("root") {
                let mut nodes = expand(root, None);
                *root = nodes.remove(0);
            }
            self.0["windows"]
                .as_array_mut()
                .expect("windows")
                .insert(0, window);
        } else if let Some(id) = op.get("remove_window") {
            self.0["windows"]
                .as_array_mut()
                .expect("windows")
                .retain(|w| w["id"] != *id);
        } else {
            panic!("unknown patch operation {op}");
        }
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut Value> {
        for window in self.0["windows"].as_array_mut()? {
            if let Some(found) = find_node_mut(&mut window["root"], id) {
                return Some(found);
            }
        }
        None
    }

    fn remove(&mut self, id: &str) -> bool {
        let Some(windows) = self.0["windows"].as_array_mut() else {
            return false;
        };
        windows
            .iter_mut()
            .any(|window| remove_node(&mut window["root"], id))
    }

    /// Child indices from window `window`'s root to the node `id`.
    pub fn path(&self, window: u32, id: &str) -> Vec<u32> {
        let windows = self.0["windows"].as_array().expect("windows");
        let w = windows
            .iter()
            .find(|w| w["id"].as_u64() == Some(u64::from(window)))
            .unwrap_or_else(|| panic!("no window {window}"));
        let mut path = Vec::new();
        assert!(
            path_to(&w["root"], id, &mut path),
            "no node {id} in window {window}"
        );
        path
    }
}

fn children_mut(node: &mut Value) -> &mut Vec<Value> {
    if node.get("children").is_none() {
        node["children"] = json!([]);
    }
    node["children"]
        .as_array_mut()
        .expect("children is an array")
}

fn find_node_mut<'a>(node: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    if node.is_null() {
        return None;
    }
    if node["id"].as_str() == Some(id) {
        return Some(node);
    }
    let children = node.get_mut("children")?.as_array_mut()?;
    children.iter_mut().find_map(|c| find_node_mut(c, id))
}

fn remove_node(node: &mut Value, id: &str) -> bool {
    let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) else {
        return false;
    };
    if let Some(index) = children.iter().position(|c| c["id"].as_str() == Some(id)) {
        children.remove(index);
        return true;
    }
    children.iter_mut().any(|c| remove_node(c, id))
}

fn insert_before(node: &mut Value, sibling: &str, nodes: &[Value]) -> bool {
    let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) else {
        return false;
    };
    if let Some(index) = children
        .iter()
        .position(|c| c["id"].as_str() == Some(sibling))
    {
        for (offset, n) in nodes.iter().enumerate() {
            children.insert(index + offset, n.clone());
        }
        return true;
    }
    children
        .iter_mut()
        .any(|c| insert_before(c, sibling, nodes))
}

fn path_to(node: &Value, id: &str, path: &mut Vec<u32>) -> bool {
    if node["id"].as_str() == Some(id) {
        return true;
    }
    if let Some(children) = node["children"].as_array() {
        for (i, child) in children.iter().enumerate() {
            path.push(i as u32);
            if path_to(child, id, path) {
                return true;
            }
            path.pop();
        }
    }
    false
}

fn set_fields(node: &mut Value, op: &Value) {
    for (key, value) in op.as_object().expect("an object") {
        match key.as_str() {
            "find" => {}
            "flags" => {
                for flag in value.as_array().expect("flags is an array") {
                    add_flag(node, "flags", flag);
                    remove_flag(node, "off", flag);
                }
            }
            "off" => {
                for flag in value.as_array().expect("off is an array") {
                    add_flag(node, "off", flag);
                    remove_flag(node, "flags", flag);
                }
            }
            _ => {
                if value.is_null() {
                    node.as_object_mut().expect("a node").remove(key);
                } else {
                    node[key] = value.clone();
                }
            }
        }
    }
}

fn add_flag(node: &mut Value, list: &str, flag: &Value) {
    if node.get(list).is_none() {
        node[list] = json!([]);
    }
    let flags = node[list].as_array_mut().expect("a flag list");
    if !flags.contains(flag) {
        flags.push(flag.clone());
    }
}

fn remove_flag(node: &mut Value, list: &str, flag: &Value) {
    if let Some(flags) = node.get_mut(list).and_then(Value::as_array_mut) {
        flags.retain(|f| f != flag);
    }
}

/// Expands one node or template into concrete nodes. `parent` is the
/// enclosing node's bounds, for children whose bounds are `"fill"`.
fn expand(value: &Value, parent: Option<[f64; 4]>) -> Vec<Value> {
    if let Some(range) = value.get("repeat") {
        return expand_repeat(value, range, parent);
    }
    let mut node = value.clone();
    let obj = node.as_object_mut().expect("a node is an object");
    let bounds = match obj.get("bounds") {
        Some(Value::String(s)) if s == "fill" => {
            parent.expect("a filling node has a parent with bounds")
        }
        Some(b) => rect(b),
        None => parent.unwrap_or([0.0; 4]),
    };
    obj.insert("bounds".into(), json!(bounds));
    if let Some(children) = obj.remove("children") {
        let expanded: Vec<Value> = children
            .as_array()
            .expect("children is an array")
            .iter()
            .flat_map(|c| expand(c, Some(bounds)))
            .collect();
        obj.insert("children".into(), Value::Array(expanded));
    }
    vec![node]
}

fn expand_repeat(value: &Value, range: &Value, parent: Option<[f64; 4]>) -> Vec<Value> {
    let range = range.as_array().expect("repeat is [first, last]");
    let (first, last) = (
        range[0].as_u64().expect("first"),
        range[1].as_u64().expect("last"),
    );
    let layout = &value["layout"];
    let offset = layout["offset"]
        .as_array()
        .map_or([0.0, 0.0], |o| [num(&o[0]), num(&o[1])]);
    let viewport = value.get("viewport").map(rect);
    let mut out = Vec::new();
    for n in first..=last {
        let mut template = substitute(&value["node"], n);
        let mut bounds = if let Some(rows) = layout.get("rows") {
            let (top, height) = (num(&rows["top"]), num(&rows["height"]));
            let i = (n - 1) as f64;
            [
                num(&rows["left"]),
                top + i * height,
                num(&rows["right"]),
                top + (i + 1.0) * height,
            ]
        } else if let Some(grid) = layout.get("grid") {
            let columns = grid["columns"].as_u64().expect("columns");
            let (col, row) = (((n - 1) % columns) as f64, ((n - 1) / columns) as f64);
            let (left, top) = (num(&grid["left"]), num(&grid["top"]));
            let (w, h) = (num(&grid["width"]), num(&grid["height"]));
            [
                left + col * w,
                top + row * h,
                left + (col + 1.0) * w,
                top + (row + 1.0) * h,
            ]
        } else {
            template
                .get("bounds")
                .map(rect)
                .or(parent)
                .expect("a repeated node has bounds")
        };
        bounds[0] += offset[0];
        bounds[2] += offset[0];
        bounds[1] += offset[1];
        bounds[3] += offset[1];
        template["bounds"] = json!(bounds);
        let mut nodes = expand(&template, parent);
        if let Some(viewport) = viewport {
            for node in &mut nodes {
                hide_outside(node, viewport);
            }
        }
        out.extend(nodes);
    }
    out
}

/// Clears `visible` on nodes that do not overlap `viewport`.
fn hide_outside(node: &mut Value, viewport: [f64; 4]) {
    let b = rect(&node["bounds"]);
    let overlaps =
        b[0] < viewport[2] && b[2] > viewport[0] && b[1] < viewport[3] && b[3] > viewport[1];
    if !overlaps {
        add_flag(node, "off", &json!("visible"));
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            hide_outside(child, viewport);
        }
    }
}

fn substitute(value: &Value, n: u64) -> Value {
    match value {
        Value::String(s) => Value::String(s.replace("{n}", &n.to_string())),
        Value::Array(a) => Value::Array(a.iter().map(|v| substitute(v, n)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), substitute(v, n)))
                .collect::<Map<_, _>>(),
        ),
        other => other.clone(),
    }
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{v} is not a number"))
}

fn rect(v: &Value) -> [f64; 4] {
    let a = v.as_array().unwrap_or_else(|| panic!("{v} is not bounds"));
    [num(&a[0]), num(&a[1]), num(&a[2]), num(&a[3])]
}

struct Strings(Vec<String>);

impl Strings {
    fn index(&mut self, s: &str) -> u32 {
        if let Some(i) = self.0.iter().position(|x| x == s) {
            return i as u32;
        }
        self.0.push(s.to_owned());
        (self.0.len() - 1) as u32
    }
}

fn to_read(screen: &Value) -> TreeRead {
    let scale = screen["scale"].as_f64().unwrap_or(1.0);
    let mut strings = Strings(Vec::new());
    let mut next_id = 1u32;
    let mut windows = Vec::new();
    for w in screen["windows"].as_array().expect("windows") {
        let id = w["id"].as_u64().expect("a window id") as u32;
        let root = if w["root"].is_null() {
            None
        } else {
            Some(to_node(&w["root"], scale, &mut strings, &mut next_id))
        };
        windows.push(Window {
            id,
            kind: window_kind(w["kind"].as_str().unwrap_or("application")),
            layer: w["layer"].as_i64().unwrap_or(0) as i32,
            package: w["package"].as_str().map(|p| strings.index(p)),
            title: w["title"].as_str().map(str::to_owned),
            bounds: pixels(rect(&w["bounds"]), scale),
            flags: window_flags(&w["flags"]),
            root,
            emits_events: w["events"].as_bool(),
            owner: w["owner"].as_str().map(|o| strings.index(o)),
            modal: w["modal"].as_bool().unwrap_or(false),
        });
    }
    let snapshot = Snapshot {
        strings: strings.0,
        node_count: windows.iter().map(count_window).sum(),
        windows,
        truncated: truncation(&screen["truncated"]),
        seq: screen["seq"].as_u64().unwrap_or(0),
    };
    let backend = &screen["backend"];
    TreeRead {
        snapshot,
        backend: BackendInfo::new(
            backend["name"].as_str().unwrap_or("fixture"),
            backend["fallback"].as_bool().unwrap_or(false),
        ),
        scale,
    }
}

fn count_window(window: &Window) -> u32 {
    fn count(node: &Node) -> u32 {
        1 + node.children.iter().map(count).sum::<u32>()
    }
    window.root.as_ref().map_or(0, count)
}

fn pixels(b: [f64; 4], scale: f64) -> Bounds {
    Bounds::new(
        (b[0] * scale).round() as i32,
        (b[1] * scale).round() as i32,
        (b[2] * scale).round() as i32,
        (b[3] * scale).round() as i32,
    )
}

fn to_node(v: &Value, scale: f64, strings: &mut Strings, next_id: &mut u32) -> Node {
    let id = *next_id;
    *next_id += 1;
    let class = v["class"].as_str().unwrap_or("android.view.View");
    let mut flags = NodeFlags::ENABLED | NodeFlags::VISIBLE;
    for name in v["flags"].as_array().into_iter().flatten() {
        flags |= node_flag(name.as_str().expect("a flag name"));
    }
    for name in v["off"].as_array().into_iter().flatten() {
        flags = NodeFlags(flags.0 & !node_flag(name.as_str().expect("a flag name")).0);
    }
    let children = v["children"]
        .as_array()
        .map(|c| {
            c.iter()
                .map(|c| to_node(c, scale, strings, next_id))
                .collect()
        })
        .unwrap_or_default();
    Node {
        id,
        class: strings.index(class),
        res_pkg: v["res_pkg"].as_str().map(|p| strings.index(p)),
        res_id: v["id"].as_str().map(str::to_owned),
        text: v["text"].as_str().map(str::to_owned),
        desc: v["desc"].as_str().map(str::to_owned),
        bounds: pixels(rect(&v["bounds"]), scale),
        flags,
        children,
        scroll: scroll(&v["scroll"]),
    }
}

/// A node's scroll semantics: `{"axis": "vertical" | "horizontal", "back",
/// "forward", "range": [unit, min, max], "position": [first, last]}`, unit
/// `"items"` or `"percent"`.
fn scroll(v: &Value) -> Option<Scroll> {
    if v.is_null() {
        return None;
    }
    let int = |v: &Value| {
        v.as_i64()
            .unwrap_or_else(|| panic!("{v} is not an integer")) as i32
    };
    let mut flags = ScrollFlags::EMPTY;
    if v["back"].as_bool() == Some(true) {
        flags |= ScrollFlags::BACK;
    }
    if v["forward"].as_bool() == Some(true) {
        flags |= ScrollFlags::FORWARD;
    }
    Some(Scroll {
        axis: match v["axis"].as_str() {
            Some("vertical") => ScrollAxis::VERTICAL,
            Some("horizontal") => ScrollAxis::HORIZONTAL,
            _ => ScrollAxis::UNKNOWN,
        },
        flags,
        range: v["range"].as_array().map(|r| ScrollRange {
            unit: match r[0].as_str() {
                Some("items") => ScrollUnit::ITEMS,
                Some("percent") => ScrollUnit::PERCENT,
                other => panic!("unknown scroll unit {other:?}"),
            },
            min: int(&r[1]),
            max: int(&r[2]),
        }),
        position: v["position"].as_array().map(|p| ScrollPosition {
            first: int(&p[0]),
            last: int(&p[1]),
        }),
    })
}

fn node_flag(name: &str) -> NodeFlags {
    match name {
        "clickable" => NodeFlags::CLICKABLE,
        "long_clickable" => NodeFlags::LONG_CLICKABLE,
        "focusable" => NodeFlags::FOCUSABLE,
        "focused" => NodeFlags::FOCUSED,
        "enabled" => NodeFlags::ENABLED,
        "checkable" => NodeFlags::CHECKABLE,
        "checked" => NodeFlags::CHECKED,
        "scrollable" => NodeFlags::SCROLLABLE,
        "selected" => NodeFlags::SELECTED,
        "editable" => NodeFlags::EDITABLE,
        "password" => NodeFlags::PASSWORD,
        "visible" => NodeFlags::VISIBLE,
        "accessibility_focused" => NodeFlags::ACCESSIBILITY_FOCUSED,
        "modal" => NodeFlags::MODAL,
        other => panic!("unknown node flag {other}"),
    }
}

fn window_flags(v: &Value) -> WindowFlags {
    let mut flags = WindowFlags::EMPTY;
    for name in v.as_array().into_iter().flatten() {
        flags |= match name.as_str().expect("a flag name") {
            "active" => WindowFlags::ACTIVE,
            "focused" => WindowFlags::FOCUSED,
            "accessibility_focused" => WindowFlags::ACCESSIBILITY_FOCUSED,
            "picture_in_picture" => WindowFlags::PICTURE_IN_PICTURE,
            "transformed" => WindowFlags::TRANSFORMED,
            other => panic!("unknown window flag {other}"),
        };
    }
    flags
}

fn window_kind(name: &str) -> WindowType {
    match name {
        "application" => WindowType::APPLICATION,
        "input_method" => WindowType::INPUT_METHOD,
        "system" => WindowType::SYSTEM,
        "accessibility_overlay" => WindowType::ACCESSIBILITY_OVERLAY,
        other => panic!("unknown window kind {other}"),
    }
}

fn truncation(v: &Value) -> Truncation {
    let mut t = Truncation::EMPTY;
    for name in v.as_array().into_iter().flatten() {
        t |= match name.as_str().expect("a truncation name") {
            "depth" => Truncation::DEPTH,
            "nodes" => Truncation::NODES,
            "windows" => Truncation::WINDOWS,
            other => panic!("unknown truncation {other}"),
        };
    }
    t
}

fn stream(name: &str) -> Stream {
    match name {
        "tree" => Stream::TREE,
        "window" => Stream::WINDOW,
        "frame" => Stream::FRAME,
        "lifecycle" => Stream::LIFECYCLE,
        other => panic!("unknown stream {other}"),
    }
}

/// A tree event naming the node `id` in `window` of `screen`.
pub fn tree_event(screen: &Screen, window: u32, change: TreeChangeKind, id: &str) -> Step {
    Step::Event(EventPayload::Tree(TreeChange {
        change,
        window,
        path: screen.path(window, id),
    }))
}

/// A window event.
pub fn window_event(change: WindowChangeKind, window: u32) -> Step {
    Step::Event(EventPayload::Window(WindowChange { change, window }))
}

fn event_payload(screen: &Screen, e: &Value) -> EventPayload {
    match e["stream"].as_str().expect("an event stream") {
        "tree" => {
            let window = e["window"].as_u64().expect("a window") as u32;
            let path = if let Some(id) = e["node"].as_str() {
                screen.path(window, id)
            } else {
                e["path"]
                    .as_array()
                    .map(|p| {
                        p.iter()
                            .map(|i| i.as_u64().expect("an index") as u32)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let change = match e["change"].as_str().unwrap_or("content") {
                "content" => TreeChangeKind::CONTENT,
                "text" => TreeChangeKind::TEXT,
                "scroll" => TreeChangeKind::SCROLL,
                "focus" => TreeChangeKind::FOCUS,
                "selection" => TreeChangeKind::SELECTION,
                other => panic!("unknown tree change {other}"),
            };
            EventPayload::Tree(TreeChange {
                change,
                window,
                path,
            })
        }
        "window" => EventPayload::Window(WindowChange {
            change: match e["change"].as_str().unwrap_or("state") {
                "state" => WindowChangeKind::STATE,
                "added" => WindowChangeKind::ADDED,
                "removed" => WindowChangeKind::REMOVED,
                "focus" => WindowChangeKind::FOCUS,
                "bounds" => WindowChangeKind::BOUNDS,
                other => panic!("unknown window change {other}"),
            },
            window: e["window"].as_u64().expect("a window") as u32,
        }),
        "lifecycle" => EventPayload::Lifecycle(Lifecycle {
            change: match e["change"].as_str().unwrap_or("warning") {
                "ready" => LifecycleKind::READY,
                "ownership_lost" => LifecycleKind::OWNERSHIP_LOST,
                "shutting_down" => LifecycleKind::SHUTTING_DOWN,
                "warning" => LifecycleKind::WARNING,
                other => panic!("unknown lifecycle change {other}"),
            },
            message: e["message"].as_str().unwrap_or_default().to_owned(),
        }),
        other => panic!("unknown event stream {other}"),
    }
}

/// A scripted event sequence.
pub struct Timeline {
    /// The screen at the start.
    pub start: Screen,
    /// Every step at its time from the start.
    pub steps: Vec<(Duration, Step)>,
    /// The screen after the last step.
    pub finish: Screen,
    /// When the sequence is over.
    pub end: Duration,
    /// How long each read takes.
    pub read_cost: Duration,
}

impl Timeline {
    /// A fake target that plays this timeline.
    pub fn target(&self) -> FakeTarget {
        let target = FakeTarget::new(self.start.read());
        target.read_cost(self.read_cost);
        for (at, step) in &self.steps {
            target.at(*at, step.clone());
        }
        target
    }
}

/// Loads timeline fixture `name`.
pub fn timeline(name: &str) -> Timeline {
    let spec = read_json(name);
    let start = Screen::load(spec["start"].as_str().expect("a start fixture"));
    let mut screen = start.clone();
    let mut steps = Vec::new();
    for step in spec["steps"].as_array().expect("steps") {
        let at = ms(step["at"].as_u64().expect("a step time"));
        if let Some(patch) = step.get("patch") {
            screen.patch(patch);
            steps.push((at, Step::Show(screen.read())));
        }
        if let Some(e) = step.get("event") {
            let payload = event_payload(&screen, e);
            let s = match e["seq"].as_u64() {
                Some(seq) => Step::EventSeq(seq, payload),
                None => Step::Event(payload),
            };
            steps.push((at, s));
        }
        if let Some(name) = step.get("drop") {
            steps.push((at, Step::Drop(stream(name.as_str().expect("a stream")))));
        }
        if let Some(hash) = step.get("frame") {
            steps.push((at, Step::Frame(hash.as_u64().expect("a frame hash"))));
        }
        if let Some(o) = step.get("out_of_sync") {
            steps.push((
                at,
                Step::Item(StreamItem::OutOfSync {
                    stream: stream(o["stream"].as_str().expect("a stream")),
                    expected: o["expected"].as_u64().expect("expected"),
                    received: o["received"].as_u64().expect("received"),
                }),
            ));
        }
        if let Some(r) = step.get("resynced") {
            steps.push((
                at,
                Step::Item(StreamItem::Resynced {
                    stream: stream(r["stream"].as_str().expect("a stream")),
                    seq: r["seq"].as_u64().expect("seq"),
                }),
            ));
        }
        if let Some(b) = step.get("backend_changed") {
            steps.push((
                at,
                Step::Item(StreamItem::BackendChanged(BackendInfo::new(
                    b["name"].as_str().expect("a backend name"),
                    b["fallback"].as_bool().unwrap_or(false),
                ))),
            ));
        }
        if step.get("reconnected").is_some() {
            steps.push((at, Step::Item(StreamItem::Reconnected)));
        }
    }
    Timeline {
        start,
        steps,
        finish: screen,
        end: ms(spec["end"].as_u64().unwrap_or(1000)),
        read_cost: ms(spec["read_cost_ms"].as_u64().unwrap_or(0)),
    }
}
