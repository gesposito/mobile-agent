//! The snapshot payload: the result of a `snapshot` request.
//!
//! Class names, package names and resource-id prefixes live once in the
//! snapshot's string table and nodes refer to them by index; enums are
//! integers and node states a bitmask (design D4). Snapshots, windows, nodes
//! and scroll semantics are maps keyed by field index; bounds, scroll ranges
//! and scroll positions are positional arrays.

use minicbor::decode::{Decoder, Error};
use minicbor::{Decode, Encode};

use crate::cbor::{self, Elements};

/// How deeply nodes may nest below a window root; agents cap their walk at
/// this depth and decoders reject deeper trees. The decoder keeps its own
/// stack of open nodes on the heap, so a deep tree never exhausts the thread
/// stack.
pub const MAX_NODE_DEPTH: usize = 256;

/// One tree read across the interactive windows.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Snapshot {
    /// The string table that [`Node::class`], [`Node::res_pkg`] and
    /// [`Window::package`] index into.
    #[cbor(n(0), decode_with = "crate::cbor::decode_vec")]
    pub strings: Vec<String>,
    /// Interactive windows, top-most first.
    #[cbor(n(1), decode_with = "crate::cbor::decode_vec")]
    pub windows: Vec<Window>,
    /// Nodes in this snapshot, across all windows.
    #[n(2)]
    pub node_count: u32,
    /// Which bounds cut the walk short; empty when the tree is complete.
    #[n(3)]
    pub truncated: Truncation,
    /// The last tree-stream event sequence number the agent had sent when it
    /// read the tree, so a consumer knows which events the snapshot reflects.
    #[n(4)]
    pub seq: u64,
}

/// One window and its node tree.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Window {
    /// Platform window id; events name windows by it.
    #[n(0)]
    pub id: u32,
    #[n(1)]
    pub kind: WindowType,
    /// Z-order; higher is closer to the user.
    #[n(2)]
    pub layer: i32,
    /// String-table index of the owning package name.
    #[n(3)]
    pub package: Option<u32>,
    #[n(4)]
    pub title: Option<String>,
    #[n(5)]
    pub bounds: Bounds,
    #[n(6)]
    pub flags: WindowFlags,
    /// The root node; absent when the platform returned none.
    #[n(7)]
    pub root: Option<Node>,
    /// Whether the window's toolkit sends change events for its content.
    /// `Some(false)` marks a window whose changes are only seen by reading
    /// it again (Jetpack Compose sends no events to a UI automation client);
    /// `None` when the agent cannot tell.
    #[n(8)]
    pub emits_events: Option<bool>,
    /// String-table index of the system process that owns the window, such
    /// as `com.android.systemui` for the status bar or the permission
    /// controller for a permission prompt; absent when the application under
    /// automation owns it.
    #[n(9)]
    pub owner: Option<u32>,
    /// The window is a modal, such as a dialog, that keeps the user from the
    /// windows below it until it is dismissed; the wire omits it when false.
    #[cbor(n(10), skip_if = "is_false")]
    pub modal: bool,
}

/// One accessibility node.
///
/// Decoded without recursion (see [`MAX_NODE_DEPTH`]): the derived decoder
/// took about 20 KiB of stack per level in a debug build.
#[derive(Clone, Debug, PartialEq, Eq, Encode)]
#[cbor(map)]
pub struct Node {
    /// Unique within the snapshot.
    #[n(0)]
    pub id: u32,
    /// String-table index of the class name.
    #[n(1)]
    pub class: u32,
    /// String-table index of the resource-id prefix, such as `pkg:id/`.
    #[n(2)]
    pub res_pkg: Option<u32>,
    /// The resource id after its prefix.
    #[n(3)]
    pub res_id: Option<String>,
    #[n(4)]
    pub text: Option<String>,
    /// Content description.
    #[n(5)]
    pub desc: Option<String>,
    #[n(6)]
    pub bounds: Bounds,
    #[n(7)]
    pub flags: NodeFlags,
    /// Child nodes in platform order; the wire omits an empty list.
    #[cbor(n(8), skip_if = "Vec::is_empty")]
    pub children: Vec<Node>,
    /// How the node scrolls; present on nodes that scroll.
    #[n(9)]
    pub scroll: Option<Scroll>,
}

/// Scroll semantics of a node that scrolls: the axis, whether it can scroll
/// back toward the start of its content and forward toward its end from
/// where it is, and, where the platform reports them, the range its content
/// spans and the part of that range inside its viewport, both in the
/// range's unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode)]
#[cbor(map)]
pub struct Scroll {
    #[n(0)]
    pub axis: ScrollAxis,
    #[n(1)]
    pub flags: ScrollFlags,
    /// The range the content spans, when the platform reports one.
    #[n(2)]
    pub range: Option<ScrollRange>,
    /// The part of `range` inside the viewport, when known; never present
    /// without a range.
    #[n(3)]
    pub position: Option<ScrollPosition>,
}

/// The range a scrolling node's content spans, on the wire `[unit, min, max]`,
/// both ends inclusive: `[items, 0, 199]` for a list of 200 items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode)]
#[cbor(array)]
pub struct ScrollRange {
    #[n(0)]
    pub unit: ScrollUnit,
    #[n(1)]
    pub min: i32,
    #[n(2)]
    pub max: i32,
}

/// The part of a [`ScrollRange`] inside the viewport, on the wire
/// `[first, last]`, both inclusive: `[0, 15]` while the first 16 items of a
/// list are in view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Encode, Decode)]
#[cbor(array)]
pub struct ScrollPosition {
    #[n(0)]
    pub first: i32,
    #[n(1)]
    pub last: i32,
}

/// A node whose map is being decoded: the fields read so far and, while
/// its child list is being read, the cursor into that list.
struct OpenNode {
    at: usize,
    entries: Elements,
    children: Option<Elements>,
    id: Option<u32>,
    class: Option<u32>,
    res_pkg: Option<u32>,
    res_id: Option<String>,
    text: Option<String>,
    desc: Option<String>,
    bounds: Option<Bounds>,
    flags: Option<NodeFlags>,
    child_nodes: Vec<Node>,
    scroll: Option<Scroll>,
}

impl OpenNode {
    fn begin(d: &mut Decoder<'_>) -> Result<OpenNode, Error> {
        Ok(OpenNode {
            at: d.position(),
            entries: Elements::begin_map(d)?,
            children: None,
            id: None,
            class: None,
            res_pkg: None,
            res_id: None,
            text: None,
            desc: None,
            bounds: None,
            flags: None,
            child_nodes: Vec::new(),
            scroll: None,
        })
    }

    /// Reads the value of `key`, except the child list, whose elements the
    /// caller reads one node at a time.
    fn read<'b, C>(&mut self, key: i64, d: &mut Decoder<'b>, ctx: &mut C) -> Result<(), Error> {
        match key {
            0 => self.id = Some(d.u32()?),
            1 => self.class = Some(d.u32()?),
            2 => self.res_pkg = Option::<u32>::decode(d, ctx)?,
            3 => self.res_id = Option::<String>::decode(d, ctx)?,
            4 => self.text = Option::<String>::decode(d, ctx)?,
            5 => self.desc = Option::<String>::decode(d, ctx)?,
            6 => self.bounds = Some(Bounds::decode(d, ctx)?),
            7 => self.flags = Some(NodeFlags::decode(d, ctx)?),
            8 => {
                let children = Elements::begin(d)?;
                self.child_nodes = Vec::with_capacity(cbor::reserve(children.len(), d));
                self.children = Some(children);
            }
            9 => self.scroll = Option::<Scroll>::decode(d, ctx)?,
            _ => d.skip()?,
        }
        Ok(())
    }

    fn finish(self) -> Result<Node, Error> {
        let at = self.at;
        Ok(Node {
            id: self.id.ok_or_else(|| cbor::missing(0, "id").at(at))?,
            class: self.class.ok_or_else(|| cbor::missing(1, "class").at(at))?,
            res_pkg: self.res_pkg,
            res_id: self.res_id,
            text: self.text,
            desc: self.desc,
            bounds: self
                .bounds
                .ok_or_else(|| cbor::missing(6, "bounds").at(at))?,
            flags: self.flags.ok_or_else(|| cbor::missing(7, "flags").at(at))?,
            children: self.child_nodes,
            scroll: self.scroll,
        })
    }
}

impl<'b, C> Decode<'b, C> for Node {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Node, Error> {
        let mut open = vec![OpenNode::begin(d)?];
        loop {
            let depth = open.len();
            let top = open
                .last_mut()
                .expect("a node is open until the root finishes");
            let child_next = match &mut top.children {
                Some(children) => {
                    let more = children.next(d)?;
                    if !more {
                        top.children = None;
                    }
                    more
                }
                None => false,
            };
            if child_next {
                if depth > MAX_NODE_DEPTH {
                    return Err(Error::message(format!(
                        "node tree deeper than {MAX_NODE_DEPTH} levels"
                    ))
                    .at(d.position()));
                }
                open.push(OpenNode::begin(d)?);
            } else if top.entries.next(d)? {
                let key = d.i64()?;
                top.read(key, d, ctx)?;
            } else {
                let done = open.pop().expect("checked above").finish()?;
                match open.last_mut() {
                    Some(parent) => parent.child_nodes.push(done),
                    None => return Ok(done),
                }
            }
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A screen rectangle in pixels, on the wire `[left, top, right, bottom]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Encode, Decode)]
#[cbor(array)]
pub struct Bounds {
    #[n(0)]
    pub left: i32,
    #[n(1)]
    pub top: i32,
    #[n(2)]
    pub right: i32,
    #[n(3)]
    pub bottom: i32,
}

impl Bounds {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Bounds {
        Bounds {
            left,
            top,
            right,
            bottom,
        }
    }
}

wire_enum! {
    /// The kind of a window, with Android's `AccessibilityWindowInfo.TYPE_*` values.
    pub struct WindowType(u8);
    /// Any window this version does not classify.
    UNKNOWN = 0, "unknown";
    APPLICATION = 1, "application";
    INPUT_METHOD = 2, "input_method";
    SYSTEM = 3, "system";
    ACCESSIBILITY_OVERLAY = 4, "accessibility_overlay";
    SPLIT_SCREEN_DIVIDER = 5, "split_screen_divider";
    MAGNIFICATION_OVERLAY = 6, "magnification_overlay";
}

wire_flags! {
    /// Node states.
    pub struct NodeFlags;
    CLICKABLE = 0, "clickable";
    LONG_CLICKABLE = 1, "long_clickable";
    FOCUSABLE = 2, "focusable";
    FOCUSED = 3, "focused";
    ENABLED = 4, "enabled";
    CHECKABLE = 5, "checkable";
    CHECKED = 6, "checked";
    SCROLLABLE = 7, "scrollable";
    SELECTED = 8, "selected";
    EDITABLE = 9, "editable";
    PASSWORD = 10, "password";
    /// Visible to the user, as the platform reports it.
    VISIBLE = 11, "visible";
    ACCESSIBILITY_FOCUSED = 12, "accessibility_focused";
    /// A view the platform marks modal, as iOS does with
    /// `accessibilityViewIsModal`: assistive technology leaves out its
    /// siblings, and what they hold, while it is on screen.
    MODAL = 13, "modal";
    /// Drawn after every sibling its bounds overlap, as Android reports the
    /// order a parent draws its children in (`getDrawingOrder`): where they
    /// overlap, it is what shows.
    ON_TOP = 14, "on_top";
}

wire_flags! {
    /// Window states.
    pub struct WindowFlags;
    ACTIVE = 0, "active";
    FOCUSED = 1, "focused";
    ACCESSIBILITY_FOCUSED = 2, "accessibility_focused";
    PICTURE_IN_PICTURE = 3, "picture_in_picture";
    /// Drawn away from where its layout puts it, as the platform draws a
    /// window while it animates the window in or out: partly off the
    /// display, or at another size than its layout's, which agents tell on
    /// Android 14 and later. Its bounds and its nodes' are then those of a
    /// frame of that animation.
    TRANSFORMED = 4, "transformed";
}

wire_enum! {
    /// The direction a node scrolls in.
    pub struct ScrollAxis(u8);
    /// The platform names no single direction.
    UNKNOWN = 0, "unknown";
    VERTICAL = 1, "vertical";
    HORIZONTAL = 2, "horizontal";
}

wire_enum! {
    /// What a [`ScrollRange`] counts.
    pub struct ScrollUnit(u8);
    /// A collection's items along the axis (Android's collection rows or
    /// columns), numbered from 0.
    ITEMS = 0, "items";
    /// Percent of the scrollable range, 0 to 100, for a position the platform
    /// reports as a value between bounds (Android's `RangeInfo`).
    PERCENT = 1, "percent";
}

wire_flags! {
    /// Which ways a node can scroll from where it is.
    pub struct ScrollFlags;
    /// Back, toward the start: up, or left in a left-to-right layout.
    BACK = 0, "back";
    /// Forward, toward the end: down, or right in a left-to-right layout.
    FORWARD = 1, "forward";
}

wire_flags! {
    /// Which request bounds cut a snapshot short.
    pub struct Truncation;
    /// `max_depth` was reached.
    DEPTH = 0, "depth";
    /// `max_nodes` was reached.
    NODES = 1, "nodes";
    /// A window could not be read.
    WINDOWS = 2, "windows";
}
