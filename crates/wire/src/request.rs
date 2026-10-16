//! The request line: the envelope ([`Request`]) and one struct of
//! parameters per action ([`Command`]), under the names the daemon reads.
//! The parameters sit at the line's top level beside the envelope's fields,
//! as agent-browser's requests carry them.
//!
//! Every parameter's default is in its type, so a request that gives none,
//! as Codex calls a tool with `{}`, reads as the daemon reads it; a request
//! leaves a parameter at its default out of the line it serializes to, which
//! the daemon reads the same. A key the action does not take is ignored.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::param::{self, Param};

/// How long `wait` with `settle` waits for quiet by default, in
/// milliseconds: the mirror's quiet window.
pub const QUIET_MS: u64 = 200;

/// How far `swipe` and `scroll` move by default, in points, as
/// agent-browser's 300 pixels.
pub const DISTANCE: u16 = 300;

/// How long a swipe's stroke takes by default, in milliseconds.
pub const SWIPE_MS: u64 = 300;

/// A screenshot's default JPEG quality.
pub const QUALITY: u64 = 80;

/// A screenshot's default longest edge, in pixels.
pub const MAX_DIMENSION: u64 = 1000;

/// One request line: its envelope and its action with the action's
/// parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// Echoed in the reply; agent-browser's CLI sends `r` and a number.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub id: Value,
    /// The session whose claims, refs, diff baselines and default timeout
    /// the request uses.
    #[serde(
        default = "param::default_session",
        deserialize_with = "param::session"
    )]
    pub session: String,
    /// The target: a Simulator UDID or adb serial as `devices` lists it, or
    /// a target's name.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    pub device: Option<String>,
    /// The target's platform: `ios` or `android`.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    pub platform: Option<String>,
    /// Take the target over from the session that holds it.
    #[serde(
        default,
        deserialize_with = "param::flag",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub force: bool,
    /// The timeout of a waiting command that names none, in milliseconds,
    /// when its session has set none; 10000 without it.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    pub default_timeout: Param<Option<u64>>,
    #[serde(flatten)]
    pub command: Command,
}

impl Request {
    /// A request for `command` in the default session, without an id.
    pub fn new(command: Command) -> Request {
        Request {
            id: Value::Null,
            session: param::default_session(),
            device: None,
            platform: None,
            force: false,
            default_timeout: Param::default(),
            command,
        }
    }
}

/// An action with its parameters, the line's `action` naming it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    Status(Status),
    Devices(Devices),
    Close(Close),
    SetTimeout(SetTimeout),
    Doctor(Doctor),
    Open(Open),
    Snapshot(Snapshot),
    DiffSnapshot(DiffSnapshot),
    Get(Get),
    Wait(Wait),
    Screenshot(Screenshot),
    Click(Click),
    Fill(Fill),
    #[serde(rename = "type")]
    Type(TypeText),
    Press(Press),
    Swipe(Swipe),
    Scroll(Scroll),
    Shutdown(Shutdown),
}

impl Command {
    /// The action's name, as the line's `action` gives it.
    pub fn action(&self) -> &'static str {
        match self {
            Command::Status(_) => "status",
            Command::Devices(_) => "devices",
            Command::Close(_) => "close",
            Command::SetTimeout(_) => "set_timeout",
            Command::Doctor(_) => "doctor",
            Command::Open(_) => "open",
            Command::Snapshot(_) => "snapshot",
            Command::DiffSnapshot(_) => "diff_snapshot",
            Command::Get(_) => "get",
            Command::Wait(_) => "wait",
            Command::Screenshot(_) => "screenshot",
            Command::Click(_) => "click",
            Command::Fill(_) => "fill",
            Command::Type(_) => "type",
            Command::Press(_) => "press",
            Command::Swipe(_) => "swipe",
            Command::Scroll(_) => "scroll",
            Command::Shutdown(_) => "shutdown",
        }
    }
}

/// `status`: the daemon, its targets and its sessions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Status {}

/// `devices`: the connected devices, emulators and Simulators, those of the
/// request's `platform` when it names one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Devices {}

/// `close`: stops the app the session opened and releases its target.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Close {}

/// `set_timeout`: sets the session's default timeout for waiting commands.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct SetTimeout {
    /// The timeout of the session's waiting commands, in milliseconds, at
    /// most 600000.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!default, with = "u64"))]
    pub ms: Param<Option<u64>>,
}

/// `doctor`: the daemon's identity, drivers and compatibility table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Doctor {}

/// `open`: launches an app and claims its target for the session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Open {
    /// The app: an Android package or an iOS bundle id.
    #[serde(
        default,
        deserialize_with = "param::needed",
        skip_serializing_if = "String::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(!default))]
    pub app: String,
    /// Restart the app if it is running.
    #[serde(
        default,
        deserialize_with = "param::flag",
        skip_serializing_if = "std::ops::Not::not"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub relaunch: bool,
    /// Launch environment variables as `KEY=VALUE`; intent string extras on
    /// Android.
    #[serde(
        default,
        deserialize_with = "param::list",
        skip_serializing_if = "Param::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub env: Param<Vec<String>>,
    /// The longest wait for the app's first tree and its settling, in
    /// milliseconds; without it, the session's timeout for the tree, then
    /// 3000 to settle.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub timeout: Param<Option<u64>>,
}

/// `snapshot`: the screen's elements, one per line with role, name and ref.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// List only elements with refs: actionable elements and named
    /// headings, cells and list items.
    #[serde(
        default,
        deserialize_with = "param::flag",
        skip_serializing_if = "std::ops::Not::not"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub interactive: bool,
    /// The deepest nesting level to list.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub max_depth: Param<Option<u64>>,
    /// Answer with the delta against the session's last delta snapshot.
    #[serde(
        default,
        deserialize_with = "param::flag",
        skip_serializing_if = "std::ops::Not::not"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub delta: bool,
    /// With `delta`, send the whole listing.
    #[serde(
        default,
        deserialize_with = "param::flag",
        skip_serializing_if = "std::ops::Not::not"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub full: bool,
}

/// `diff_snapshot`: the screen compared with the session's last snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase")]
pub struct DiffSnapshot {
    /// Without a last snapshot, list only elements with refs.
    #[serde(
        default,
        deserialize_with = "param::flag",
        skip_serializing_if = "std::ops::Not::not"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub interactive: bool,
    /// Without a last snapshot, the deepest nesting level to list.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub max_depth: Param<Option<u64>>,
}

/// `get`: an element's text, value or bounds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Get {
    /// What to read: `text` (the element's value, else its name), `value`,
    /// or `box` (its bounds in points).
    #[serde(
        default,
        deserialize_with = "param::needed",
        skip_serializing_if = "String::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(!default))]
    pub what: String,
    /// The element: a ref such as `@e3`, or a selector (`#id`, `text=...`,
    /// `text==...`, `role=...`).
    #[serde(
        default,
        deserialize_with = "param::needed",
        skip_serializing_if = "String::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(!default))]
    pub target: String,
    /// The longest wait for a selector's element, in milliseconds; the
    /// session's default without it, else 10000.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub timeout: Param<Option<u64>>,
}

/// `wait`: waits some milliseconds, for the screen to settle, or for an
/// element or a text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase")]
pub struct Wait {
    /// Wait this many milliseconds.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub ms: Param<Option<u64>>,
    /// Wait until neither the tree nor the frames change for the quiet
    /// window.
    #[serde(
        default,
        deserialize_with = "param::flag",
        skip_serializing_if = "std::ops::Not::not"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub settle: bool,
    /// The quiet window of `settle`, in milliseconds.
    #[serde(
        default = "param::whole::<QUIET_MS>",
        deserialize_with = "param::whole_or::<_, QUIET_MS>",
        skip_serializing_if = "param::is_whole::<QUIET_MS>"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub quiet: Param<u64>,
    /// With `settle`, settle on tree changes alone, ignoring frames, as for
    /// video.
    #[serde(
        default,
        deserialize_with = "param::flag",
        skip_serializing_if = "std::ops::Not::not"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub tree_only: bool,
    /// Wait for an element whose name or value contains this text.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub text: Option<String>,
    /// Wait for this element: a ref or a selector.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub target: Option<String>,
    /// What to wait for: `visible`, `hidden` or `enabled` on screen, or
    /// `attached` or `detached` in the tree, in view or not.
    #[serde(
        default = "param::visible",
        deserialize_with = "param::visible_or",
        skip_serializing_if = "param::is_visible"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub state: String,
    /// The longest wait, in milliseconds: the session's default without it,
    /// else 10000, and 3000 with `settle`.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub timeout: Param<Option<u64>>,
}

impl Default for Wait {
    fn default() -> Self {
        Wait {
            ms: Param::default(),
            settle: false,
            quiet: param::whole::<QUIET_MS>(),
            tree_only: false,
            text: None,
            target: None,
            state: param::visible(),
            timeout: Param::default(),
        }
    }
}

/// `screenshot`: saves the screen as a JPEG.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase")]
pub struct Screenshot {
    /// Where to save the image; a new file in the temporary directory
    /// without it.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub path: Option<String>,
    /// The JPEG quality, 1 to 100.
    #[serde(
        default = "param::whole::<QUALITY>",
        deserialize_with = "param::whole_or::<_, QUALITY>",
        skip_serializing_if = "param::is_whole::<QUALITY>"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub quality: Param<u64>,
    /// The image's longest edge, in pixels.
    #[serde(
        default = "param::whole::<MAX_DIMENSION>",
        deserialize_with = "param::whole_or::<_, MAX_DIMENSION>",
        skip_serializing_if = "param::is_whole::<MAX_DIMENSION>"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub max_dimension: Param<u64>,
}

impl Default for Screenshot {
    fn default() -> Self {
        Screenshot {
            path: None,
            quality: param::whole::<QUALITY>(),
            max_dimension: param::whole::<MAX_DIMENSION>(),
        }
    }
}

/// `click`: taps an element or a point, and replies once the tap's first
/// frame change and tree delta have come.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Click {
    /// The element: a ref such as `@e3`, or a selector (`#id`, `text=...`,
    /// `text==...`, `role=...`); not given when `x` and `y` give a point.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub target: Option<String>,
    /// The point's x in device points, with `y`.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub x: Param<Option<f64>>,
    /// The point's y in device points, with `x`.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub y: Param<Option<f64>>,
    /// Hold that long, in milliseconds: a long press.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub hold: Param<Option<u64>>,
    /// The longest wait for a selector's element, in milliseconds; the
    /// session's default without it, else 10000.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub timeout: Param<Option<u64>>,
}

/// `fill`: clears a text field, enters the text and reads it back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Fill {
    /// The field: a ref or a selector.
    #[serde(
        default,
        deserialize_with = "param::needed",
        skip_serializing_if = "String::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(!default))]
    pub target: String,
    /// The text that replaces the field's content; empty clears it.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(!default, required))]
    pub text: Option<String>,
    /// The longest wait for a selector's element, in milliseconds; the
    /// session's default without it, else 10000.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub timeout: Param<Option<u64>>,
}

/// `type`: types text as key events into an element, or into the focused
/// field, keeping what the field holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct TypeText {
    /// The text to type.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(!default, required))]
    pub text: Option<String>,
    /// The field: a ref or a selector; the focused field without it.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub target: Option<String>,
    /// The longest wait for a selector's element, in milliseconds; the
    /// session's default without it, else 10000.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub timeout: Param<Option<u64>>,
}

/// `press`: presses a key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Press {
    /// A key name such as `Enter`, `Back` or `Home`, a letter or digit,
    /// modifiers joined with `+` as in `Control+a`, or a platform key code.
    #[serde(
        default,
        deserialize_with = "param::needed",
        skip_serializing_if = "String::is_empty"
    )]
    #[cfg_attr(feature = "schema", schemars(!default))]
    pub key: String,
}

/// `swipe`: moves the finger in a direction from the screen's center, or
/// between two points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Swipe {
    /// The finger's direction: `up`, `down`, `left` or `right`; `up` moves
    /// the content up and shows what is below.
    #[serde(
        default = "param::up",
        deserialize_with = "param::up_or",
        skip_serializing_if = "param::is_up"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub direction: String,
    /// How far the finger moves in `direction`, in points.
    #[serde(
        default = "param::points::<DISTANCE>",
        deserialize_with = "param::points_or::<_, DISTANCE>",
        skip_serializing_if = "param::is_points::<DISTANCE>"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub distance: Param<f64>,
    /// Where the stroke starts, `[x, y]` in points, with `to`, instead of a
    /// direction.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub from: Param<Option<[f64; 2]>>,
    /// Where the stroke ends, `[x, y]` in points, with `from`.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub to: Param<Option<[f64; 2]>>,
    /// How long the stroke takes, in milliseconds.
    #[serde(
        default = "param::whole::<SWIPE_MS>",
        deserialize_with = "param::whole_or::<_, SWIPE_MS>",
        skip_serializing_if = "param::is_whole::<SWIPE_MS>"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub duration: Param<u64>,
}

impl Default for Swipe {
    fn default() -> Self {
        Swipe {
            direction: param::up(),
            distance: param::points::<DISTANCE>(),
            from: Param::default(),
            to: Param::default(),
            duration: param::whole::<SWIPE_MS>(),
        }
    }
}

/// `scroll`: scrolls the content, or until an element is mostly in view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Scroll {
    /// The content's direction: `up`, `down`, `left` or `right`; `down`
    /// shows what is below.
    #[serde(
        default = "param::down",
        deserialize_with = "param::down_or",
        skip_serializing_if = "param::is_down"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub direction: String,
    /// How far the content moves, in points; with `until`, the longest
    /// stroke.
    #[serde(
        default = "param::points::<DISTANCE>",
        deserialize_with = "param::points_or::<_, DISTANCE>",
        skip_serializing_if = "param::is_points::<DISTANCE>"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub amount: Param<f64>,
    /// Scroll inside this element, a ref or a selector, instead of the
    /// screen.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub target: Option<String>,
    /// Scroll until this element, a ref or a selector, is mostly in view.
    #[serde(
        default,
        deserialize_with = "param::text",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub until: Option<String>,
    /// The longest wait for `target` and `until`, in milliseconds; the
    /// session's default without it, else 10000.
    #[serde(default, skip_serializing_if = "Param::is_none")]
    #[cfg_attr(feature = "schema", schemars(!skip_serializing_if))]
    pub timeout: Param<Option<u64>>,
}

impl Default for Scroll {
    fn default() -> Self {
        Scroll {
            direction: param::down(),
            amount: param::points::<DISTANCE>(),
            target: None,
            until: None,
            timeout: Param::default(),
        }
    }
}

/// `shutdown`: stops the daemon's targets, then the daemon.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
pub struct Shutdown {}
