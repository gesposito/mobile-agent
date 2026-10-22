//! The reply line: agent-browser's envelope ([`Reply`]) and one struct of
//! data per action, under the names the daemon's handlers write, with the
//! details failures carry ([`Details`]). Every struct refuses fields it does
//! not name, so `crates/daemon/tests/replies.rs`, which reads each action's
//! reply on a fake target, fails when a handler adds a field until its type
//! gains it.
//!
//! A reply whose shape depends on the request, such as `get`'s or `wait`'s,
//! has one struct with a field for each shape, absent in the others. The
//! handlers build their replies themselves (`crates/daemon/src/handlers`);
//! these types read them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One reply line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply<T = Value> {
    /// The request's id, echoed.
    pub id: Value,
    pub success: bool,
    /// The action's data; for a failure its details, null without any.
    pub data: Option<T>,
    /// What failed and what to do about it; null on success.
    pub error: Option<String>,
    /// A failure's snake_case code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Why a success is not to be taken at face value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// `status`: the daemon, its targets and its sessions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub daemon: Daemon,
    pub targets: Vec<StatusTarget>,
    pub sessions: Vec<Session>,
}

/// The daemon, as `status` reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Daemon {
    pub pid: u32,
    /// The release and the executable's build, which the CLI compares with
    /// the daemon it would start.
    pub version: String,
    pub socket: String,
    pub uptime_ms: u64,
    /// How long the daemon waits without requests before it exits; null
    /// when it never does.
    pub idle_timeout_ms: Option<u64>,
    pub drivers: Vec<String>,
    /// `auto` or `off`.
    pub emulator_controller: String,
}

/// A target, as `status` reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusTarget {
    pub id: String,
    /// `ios` or `android`.
    pub platform: String,
    /// Such as `simulator`, `android-emulator` or `ios-device`.
    pub class: String,
    pub name: String,
    /// The state its platform reports, such as `booted` or `device`.
    pub state: String,
    pub attached: bool,
    /// How many times the daemon attached the target.
    pub attaches: u32,
    /// Whether the attached target has a frame source; null before it is
    /// attached.
    pub frame_source: Option<bool>,
    /// The app a session opened last, until it is closed.
    pub app: Option<String>,
    /// The session that holds the target.
    pub claimed_by: Option<String>,
    /// Why the last attach failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// While the target is attached, what its driver reports: `lines` of
    /// text and the driver's own fields (crates/daemon/README.md, Drivers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<Map<String, Value>>,
}

/// A session, as `status` reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Session {
    pub name: String,
    /// The targets it holds.
    pub targets: Vec<String>,
    /// The default timeout it set for its waiting commands.
    pub default_timeout_ms: Option<u64>,
}

/// `devices`: the connected targets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Devices {
    pub devices: Vec<Device>,
}

/// A target, as `devices` lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Device {
    /// A Simulator UDID or an adb serial; `device` takes it.
    pub id: String,
    pub platform: String,
    pub class: String,
    pub name: String,
    pub state: String,
    pub model: Option<String>,
    /// The OS name and version, such as `iOS 26.5`.
    pub os: Option<String>,
    /// What makes the target usable, for one its state keeps from being
    /// automated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advice: Option<String>,
    pub claimed_by: Option<String>,
}

/// `close`: the app stopped, or left running, and the target released.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Close {
    pub closed: bool,
    /// The app the session had opened, which was stopped, or with `keep`
    /// left running.
    pub app: Option<String>,
    pub target: String,
    /// With `keep`, true: the app was left running.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub kept: bool,
}

/// `cancel`: the session's command it ended, if one was in flight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Cancel {
    /// Whether a command of the session was in flight, which the cancel
    /// ended.
    pub cancelled: bool,
    /// That command's action; null when none was in flight.
    pub action: Option<String>,
    /// The target it ran on; null when none was in flight.
    pub target: Option<String>,
}

/// `set_timeout`: the session's default timeout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetTimeout {
    pub session: String,
    pub default_timeout_ms: u64,
}

/// `doctor`: the daemon's identity, drivers and compatibility table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Doctor {
    pub pid: u32,
    pub version: String,
    pub drivers: Vec<String>,
    pub emulator_controller: String,
    pub compatibility: Compatibility,
    /// The physical iPhone driver's check of its runner bundle, when that
    /// driver is registered (crates/daemon/README.md, The physical iPhone
    /// driver).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ios_runner: Option<Value>,
}

/// The rows the drivers contribute to the compatibility table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    pub rows: Vec<CompatibilityRow>,
    /// Why the table is empty, when it is.
    pub note: Option<String>,
}

/// One row of the compatibility table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct CompatibilityRow {
    /// What was validated or found, such as an Xcode release.
    pub subject: String,
    /// The call site or path.
    pub feature: String,
    /// `validated`, `supported`, `limited` or `unsupported`.
    pub status: String,
    pub note: Option<String>,
}

/// `open`: the launched app and how long its first screen took.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Open {
    pub opened: String,
    pub target: String,
    /// Whether the app's screen settled.
    pub settled: bool,
    /// From the launch request until the screen settled.
    pub elapsed_ms: f64,
    /// Until the replica showed the app; null when it did not.
    pub first_tree_ms: Option<f64>,
    /// A dialog of system UI in front of the app, as a permission alert,
    /// named as an action refused under it names it, such as `alert "Allow
    /// …"`; absent when none stands in front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<String>,
}

/// `snapshot`: the listing, or its delta, and what reading it cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    /// The listing's text; with `delta`, agent-browser's delta object.
    pub snapshot: Listing,
    /// Without `delta`, each listed ref's role and name, by the ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<BTreeMap<String, RefNode>>,
    /// Without `delta`, the session's refs that left the screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_refs: Option<Vec<String>>,
    /// The app in front, or the system UI that owns the foreground.
    pub origin: String,
    /// The target's worker's time to answer, device reads included.
    pub read_ms: f64,
    /// Whether it answered without reading the device.
    pub served_from_replica: bool,
    /// The mirror's replica revision.
    pub replica_revision: u64,
    pub target: String,
    pub verdict: Verdict,
}

/// A snapshot's listing: its text, or a delta.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Listing {
    Text(String),
    Delta(DeltaSnapshot),
}

/// A listed ref's role and name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct RefNode {
    pub name: String,
    pub role: String,
}

/// Agent-browser's delta snapshot, as the mirror writes it: the whole
/// listing (`full`), nothing new (`unchanged`) or the changes (`delta`)
/// against the session's last delta snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeltaSnapshot {
    /// `full`, `unchanged` or `delta`.
    pub kind: String,
    pub revision: u64,
    /// The revision compared with, but for `full`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u64>,
    /// With `full`, each listed ref's role and name, by the ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<BTreeMap<String, RefNode>>,
    /// With `full`, the listing's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    /// With `delta`, the refs added, removed and renamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<Vec<RefChange>>,
    /// With `delta`, the lines of the listing replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_change: Option<TreeChange>,
}

/// A change to the refs in a delta.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct RefChange {
    /// `add`, `remove` or `replace`.
    pub op: String,
    #[serde(rename = "ref")]
    pub reference: String,
    /// With `add`, the ref's role and name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<RefNode>,
    /// With `replace`, `role` or `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// With `replace`, the field's new value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// The lines of a listing a delta replaces.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TreeChange {
    pub start_line: u64,
    pub delete_count: u64,
    pub lines: Vec<String>,
}

/// How far a snapshot can be trusted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    /// The backend that read the tree.
    pub backend: String,
    /// Whether that backend is a fallback.
    pub fallback: bool,
    pub degraded: bool,
    pub reasons: Vec<Reason>,
}

/// Why a snapshot is degraded, and what to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Reason {
    pub code: String,
    pub advice: String,
}

/// `diff_snapshot`: the screen against the session's last snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiffSnapshot {
    pub changed: bool,
    /// The diff as text.
    pub diff: String,
    pub additions: u64,
    pub removals: u64,
    pub modifications: u64,
    pub unchanged: u64,
    pub added: Vec<Element>,
    pub removed: Vec<Element>,
    /// The changed elements, each with the `fields` that changed.
    pub modified: Vec<Element>,
    pub base_replica_revision: u64,
    pub replica_revision: u64,
    pub target: String,
}

/// An element, as replies describe it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Element {
    /// Its ref, such as `@e3`; null for an element without one.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub role: String,
    pub name: String,
    /// Its identifier.
    pub id: Option<String>,
    pub value: Option<String>,
    pub enabled: bool,
    /// Left, top, right and bottom, in device points.
    pub bounds: [f64; 4],
    /// In a diff's `modified`, the fields that changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

/// `get`: an element's text, value or bounds, by what the request asked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Get {
    /// For `text`, the element's value, else its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// For `value`, the field's value, null when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// For `value`, a switch's or a checkbox's state, as the listing's
    /// `checked` has it; absent for an element that is neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// For `box`, the bounds in device points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Whether a tap reaches part of it.
    pub in_view: bool,
    /// The twin a stale ref the request named stood for; absent when the
    /// request named none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
}

/// A stale ref that stood for its twin, the element an app rebuilt in its
/// place: by its role and identifier, else by its role, name and bounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Resolved {
    /// The ref the request named, such as `@e221`, whose element is gone.
    pub stale: String,
    /// The twin's ref, such as `@e240`, which the session acts on from then
    /// on.
    #[serde(rename = "ref")]
    pub reference: String,
}

/// `wait`: what was waited for and how it ended, by the kind of wait.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Wait {
    /// `time`, `settle`, `change`, `selector` or `text`.
    pub waited: String,
    /// For `time`, the milliseconds waited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    /// For `settle`, whether the screen settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled: Option<bool>,
    /// For `settle`, the signal that changed last: `tree`, `frame`, `both`
    /// or `none`; or `timeout`. For `change`, what moved since the
    /// session's last snapshot: `tree`, `frame` or `both`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<f64>,
    /// For `settle`, what kept changing: `tree`, `frame`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub still_changing: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_changes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_changes: Option<u64>,
    /// For `change`, the delta against the session's last delta snapshot,
    /// as `snapshot` with `delta` gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<DeltaSnapshot>,
    /// For an element or a text, the element; null once it is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<Element>,
    /// Whether a tap reaches part of the element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_view: Option<bool>,
    /// For an element, the element as the request gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    /// For a text, the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// `screenshot`: the image written, or with `inline` returned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Screenshot {
    /// Where the image was written; null with `inline` and no path, which
    /// writes none.
    pub path: Option<String>,
    /// `jpeg` or `png`.
    pub format: String,
    /// The image's size in pixels.
    pub width: u32,
    pub height: u32,
    /// The screen's size in points, which a click takes, as the image shows
    /// it scaled to `width` by `height` pixels.
    pub points: Points,
    pub quality: u8,
    /// The frame's number, for the mirror's latest frame.
    pub seq: Option<u64>,
    pub age_ms: f64,
    /// `capture` when the driver took it now, `frame` for the mirror's
    /// latest frame.
    pub source: String,
    /// With `inline`, the image's bytes, base64-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// With `inline`, the image's type: `image/jpeg` or `image/png`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// A size in points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Points {
    pub width: f64,
    pub height: f64,
}

/// `click`: where the tap went and what it set off, from its dispatch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Click {
    /// The element or point as the request gave it.
    pub clicked: String,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    /// What the tap landed on as the replica held it at dispatch; null when
    /// no element with a ref lay under the point.
    pub hit: Option<Hit>,
    pub x: f64,
    pub y: f64,
    pub accepted: bool,
    /// The time the input sink took to accept the tap.
    pub dispatch_ms: f64,
    /// To the first frame whose hash changed, when one came.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_change_ms: Option<f64>,
    /// To the mirror's first tree delta, when one came.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_delta_ms: Option<f64>,
    /// How long the effects were watched.
    pub watched_ms: f64,
    /// The twin a stale ref the request named stood for; absent when the
    /// request named none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
}

/// The element a click's tap landed on: the top-most with a ref that a tap
/// reaches at the point, or the keyboard where it draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Hit {
    pub role: String,
    pub name: String,
    /// The ref the session's snapshots listed it under, such as `@e3`; null
    /// when they did not.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
}

/// `fill`: how the text went in and what the read-back found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Fill {
    pub filled: String,
    /// The text as the request gave it.
    pub text: String,
    /// The field's ref; null for a node holding no field.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    /// `keys` or `direct`.
    pub method: Option<String>,
    /// `text` or `length` for how the field showed the text, `false` when it
    /// showed nothing to read back, null when it was not read back.
    pub verified: Option<Verified>,
    /// Where the read-back took the text from: `value`, `descendants` or
    /// `name`.
    pub read_from: Option<String>,
    /// `no_value_to_read` when `verified` is false.
    pub reason: Option<String>,
    pub read_back_ms: Option<f64>,
    /// Whether the text went in again in one piece.
    pub retried: bool,
    /// The twin a stale ref the request named stood for; absent when the
    /// request named none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
}

/// How a fill's read-back found the text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Verified {
    /// `text` or `length`.
    Shown(String),
    /// `false`: the field showed nothing to read back.
    NotShown(bool),
}

/// `type`: the text typed and what the read-back of the field found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeText {
    /// The text as the request gave it.
    pub typed: String,
    /// The field's ref; null for the focused field.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    /// `keys` or `direct`.
    pub method: Option<String>,
    /// What the field holds once the keys went in, as the read-back read
    /// it; null when it showed nothing or was not read back.
    pub text: Option<String>,
    /// `text` or `length` for how the field showed the text added to what it
    /// held, `false` when it showed other text or nothing, null when it was
    /// not read back.
    pub verified: Option<Verified>,
    /// Where the read-back took what the field holds from: `value`,
    /// `descendants` or `name`.
    pub read_from: Option<String>,
    /// `text_mismatch` or `no_value_to_read` when `verified` is false;
    /// `no_focused_field` when no field in the tree had the focus.
    pub reason: Option<String>,
    pub read_back_ms: Option<f64>,
    /// The twin a stale ref the request named stood for; absent when the
    /// request named none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
}

/// `press`: the key pressed, in the platform's codes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Press {
    pub pressed: String,
    pub code: u32,
    pub meta: Option<u32>,
}

/// `swipe`: the stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Swipe {
    /// The direction, or `true` for a stroke between two points.
    pub swiped: Swiped,
    pub from: [f64; 2],
    pub to: [f64; 2],
}

/// What a swipe was.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Swiped {
    Direction(String),
    Points(bool),
}

/// `scroll`: the strokes, and with `until` the element brought into view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Scroll {
    pub scrolled: bool,
    pub direction: String,
    /// How far the content moved; with `until`, the length of the strokes
    /// that look for the element.
    pub amount: f64,
    pub strokes: u32,
    /// Where the last stroke came down, null when there was none.
    pub from: Option<[f64; 2]>,
    /// Where the last stroke lifted, null when there was none.
    pub to: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// With `until`, the share of the element's bounds in view, from 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<Element>,
    /// The twin a stale ref the request named stood for; absent when the
    /// request named none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
}

/// `shutdown`: the daemon stopped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Shutdown {
    pub stopped: bool,
    /// The targets that had not stopped within the wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_stopped: Option<Vec<String>>,
}

/// The details a failure carries in its data, each field present for the
/// failures that give it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Details {
    /// `no_target`, `target_not_found` and `ambiguous_target`: the targets
    /// to choose from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<Candidate>>,
    /// `target_claimed`: the session that holds the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    /// `target_claimed`: the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// `stroke_too_short`: the stroke's length, in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<f64>,
    /// `stroke_too_short`: the longest stroke the platform takes for a tap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap_slop: Option<f64>,
    /// A scroll's strokes before it stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strokes: Option<u32>,
    /// A scroll's `until`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// The share of the element's bounds in view, from 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<f64>,
    /// Where a scroll's last stroke came down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<[f64; 2]>,
    /// Where a scroll's last stroke lifted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<[f64; 2]>,
    /// `element_offscreen` and `text_mismatch`: the element's ref.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// `text_mismatch`: what the field holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// `text_mismatch`: whether the field shows mask characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masked: Option<bool>,
}

/// A target to choose from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema), schemars(transform = crate::param::soft_wraps))]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    pub platform: String,
    pub class: String,
    pub name: String,
}
