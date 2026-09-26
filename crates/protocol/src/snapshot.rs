//! The snapshot payload: the result of a `snapshot` request.
//!
//! Class names, package names and resource-id prefixes live once in the
//! snapshot's string table and nodes refer to them by index; enums are
//! integers and node states a bitmask (design D4).

/// How deeply nodes may nest below a window root. Decoding a deeper tree
/// fails instead of exhausting the stack; agents cap their walk below it.
pub const MAX_NODE_DEPTH: usize = 256;

/// One tree read across the interactive windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// The string table that [`Node::class`], [`Node::res_pkg`] and
    /// [`Window::package`] index into.
    pub strings: Vec<String>,
    /// Interactive windows, top-most first.
    pub windows: Vec<Window>,
    /// Nodes in this snapshot, across all windows.
    pub node_count: u32,
    /// Which bounds cut the walk short; empty when the tree is complete.
    pub truncated: Truncation,
    /// The last tree-stream event sequence number the agent had sent when it
    /// read the tree, so a consumer knows which events the snapshot reflects.
    pub seq: u64,
}

/// One window and its node tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// Platform window id; events name windows by it.
    pub id: u32,
    pub kind: WindowType,
    /// Z-order; higher is closer to the user.
    pub layer: i32,
    /// String-table index of the owning package name.
    pub package: Option<u32>,
    pub title: Option<String>,
    pub bounds: Bounds,
    pub flags: WindowFlags,
    /// The root node; absent when the platform returned none.
    pub root: Option<Node>,
}

/// One accessibility node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Unique within the snapshot.
    pub id: u32,
    /// String-table index of the class name.
    pub class: u32,
    /// String-table index of the resource-id prefix, such as `pkg:id/`.
    pub res_pkg: Option<u32>,
    /// The resource id after its prefix.
    pub res_id: Option<String>,
    pub text: Option<String>,
    /// Content description.
    pub desc: Option<String>,
    pub bounds: Bounds,
    pub flags: NodeFlags,
    /// Child nodes in platform order; the wire omits an empty list.
    pub children: Vec<Node>,
}

/// A screen rectangle in pixels: left, top, right, bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
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
}

wire_flags! {
    /// Window states.
    pub struct WindowFlags;
    ACTIVE = 0, "active";
    FOCUSED = 1, "focused";
    ACCESSIBILITY_FOCUSED = 2, "accessibility_focused";
    PICTURE_IN_PICTURE = 3, "picture_in_picture";
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
