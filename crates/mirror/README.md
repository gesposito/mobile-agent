# mirror

The live UI replica (spec `ui-mirror`, design D5 and D8): one target's
windows, accessibility tree and latest frame, kept current from what the
target's driver reports, so that snapshots, waits, settle and diffs are
answered from host state instead of a device round trip and an idle wait.
The crate holds no device code: drivers implement the traits in
`src/backend.rs`, and `src/testing.rs` provides a scripted fake target that
the tests, and later the daemon's tests, drive with a fake clock.

## Layout

```
src/
  backend.rs    the driver-facing traits: tree source, event stream, frame source, input sink, clock
  mirror.rs     Mirror: pump, drain, run_until, snapshot, act, settle, wait, diff, delta_snapshot, frame,
                timestamps, note_window_change, visibility, scroll_viewport
  timestamps.rs when the latest frame, frame change, tree change, read and input happened
  replica.rs    windows and node trees, identity matching between reads, deltas, sequence numbers, resync
  view.rs       roles, names, visibility, modal cutting, scroll hints
  visibility.rs how much of an element is in view: its visible bounds, share and cut edge; the
                visible bounds of the scroll container under a point
  roles.rs      roles from Android widget classes, XCUIElementType names and UIKit classes
  render.rs     listings and agent-browser's snapshot lines
  refs.rs       eN refs: parsing, issuing, stale detection
  verdict.rs    quality verdicts and their advice
  settle.rs     settle options and results
  wait.rs       selectors, conditions, timeout summaries
  diff.rs       the structural diff
  delta.rs      agent-browser's delta snapshots and their JSON
  frame.rs      frame size, quality and age policy
  json.rs       JSON strings escaped as serde_json escapes them
  geometry.rs   device points
  testing.rs    FakeTarget: a scripted backend on a fake clock
fixtures/       synthetic trees in device points with their scale, and event timelines
tests/          one file per task: replica, refs, interactive, settle, waits, diff, resync, frames,
                timestamps, editables, controls, visibility
tests/support/  the fixture loader
```

## Driver contract

A driver builds a `Backend` for each target and hands it to `Mirror::new`.
Every call is synchronous and the mirror calls one method at a time; a
driver answers from its own threads or blocks on the device.

```rust
pub struct Backend {
    pub tree: Box<dyn TreeSource>,
    pub events: Box<dyn EventStream>,
    pub frames: Option<Box<dyn FrameSource>>,   // None on Android devices
    pub input: Box<dyn InputSink>,
    pub clock: Box<dyn Clock>,                  // SystemClock outside tests
}

pub trait TreeSource: Send {
    fn read(&mut self, request: &ReadRequest) -> Result<TreeRead, BackendError>;
    fn can_probe(&self) -> bool { false }        // true: `probe` reads bounds again
    fn probe(&mut self, nodes: &[NodeHandle]) -> Result<Vec<ProbedNode>, BackendError>;
}
pub trait EventStream: Send {
    /// The next item, waiting until `deadline` at most; with a deadline that
    /// has passed, only what is queued.
    fn next(&mut self, deadline: Instant) -> Option<Timed<StreamItem>>;
}
pub trait FrameSource: Send {
    fn latest(&self) -> Option<FrameMeta>;
    fn encode(&mut self, request: &EncodeRequest) -> Result<Vec<u8>, BackendError>;
}
pub trait InputSink: Send {
    fn perform(&mut self, input: &Input) -> Result<InputAck, BackendError>;
}
pub trait Clock: Send {
    fn now(&self) -> Instant;
}
```

`BackendError` is `protocol::Error`. The types around the traits:

| Type | Fields |
|---|---|
| `ReadRequest` | `windows: Option<Vec<u32>>` (`None` reads every window), `reason: ReadReason` (`Attach`, `Events`, `Action`, `Poll`, `Frames`, `Moved`, `Resync`) |
| `ProbedNode` | `node: NodeHandle`, `bounds` (physical pixels): a node's bounds as a probe read them again |
| `TreeRead` | `snapshot: protocol::snapshot::Snapshot` (windows with their metadata, nodes with their scroll semantics), `backend: BackendInfo`, `scale: f64` |
| `Owner` | `Target` or `System(name)`, from a window's `owner` |
| `BackendInfo` | `name: String`, `fallback: bool` |
| `Timed<StreamItem>` | `at: Instant` (host arrival), `item` |
| `StreamItem` | `Event(protocol::event::Event)`, `Frame(FrameMeta)`, `OutOfSync { stream, expected, received }`, `Resynced { stream, seq }`, `BackendChanged(BackendInfo)`, `Reconnected`, `Disconnected(String)` |
| `FrameMeta` | `seq`, `at: Instant` (capture), `hash: u64`, `width`, `height`, `rotation` (clockwise turn that makes the pixels upright) |
| `EncodeRequest` | `seq`, `width`, `height` (upright output), `quality`, `rotation` |
| `Input` | `Tap { at, hold }`, `Swipe { from, to, duration }`, `Key { code, meta }`, `Text { text, node: Option<NodeHandle>, method }` |
| `InputAck` | `accepted: bool`, `method: Option<TextMethod>` |

Conventions a driver follows:

- **Units.** Payload bounds are physical pixels, as the protocol defines
  them; `TreeRead::scale` is pixels per device point (2.625 on a 420 dpi
  Android screen, 3 on an iPhone). Input coordinates come back in points.
- **Fresh reads.** Every window a read returns is current as of the read:
  windows whose toolkit sends no change events are read with the platform
  cache cleared, and so are the windows a read for `Moved` names. A read
  may return more windows than requested; a requested window missing from
  the result is gone. `snapshot.seq` is the last tree-stream event sent
  before the read, so the mirror skips events the read already reflects.
- **Bounds probes** (task 10.11). A source whose `can_probe` says so reads
  the bounds of the nodes `probe` names, nodes of its latest read, again
  past its caches, and answers the nodes still on screen; the Android
  driver asks the agent's `bounds` (agent 0.3.0 on). The mirror probes only
  while it waits for a target without frames to settle.
- **Window metadata** (the protocol's window keys 8 to 10, task 3.6).
  `emits_events: Some(false)` puts a window on the re-read path and
  `Some(true)` keeps it off, whatever its classes; the Android agent sets
  `Some(false)` for windows hosting Compose, dialogs included, and
  Simulator drivers set it for every window. Without it the mirror infers
  it: a window holding a node of any class under `androidx.compose.` sends
  none (spike S1), which covers the
  `androidx.compose.ui.platform.AndroidComposeView` spike S1 saw and the
  `androidx.compose.ui.platform.ComposeView` the API 36 emulator shows.
  `owner` names the system process that owns a window, `modal` marks a
  window that cuts the application windows below it.
- **Classes and labels.** Android passes widget classes. The Simulator driver
  passes `XCUIElementType` names, or the UIKit class for elements typed
  `Other` so that `_UIAlertControllerPhoneTVMacView` is recognized as an
  alert. `desc` is the label, `text` the value (an Android agent can pass a
  field's hint as its description), `res_id` the test identifier.
- **Visibility.** The `visible` flag as the platform reports it; on iOS,
  whether the frame overlaps the screen.
- **Foreground.** The `active` window owns the foreground; system UI windows,
  such as SpringBoard's or the Android permission controller's for a
  permission prompt, carry an `owner`, which snapshots report as
  `Owner::System`.
- **Stream items.** The Android driver forwards the host session's
  `HostEvent::Event` as `Event`, `OutOfSync` and `Resynced` as themselves,
  and `Closed` as `Disconnected`; a new agent connection is `Reconnected`.
  The Simulator driver reports a switch from the in-guest reader to the
  translator as `BackendChanged` (a read from another backend is caught
  either way). Frames go on the same stream, so a wait wakes on them; their
  hash leaves the status bar out (spike S7).

## How the replica stays current

| Trigger | Read | Reason |
|---|---|---|
| first use | every window | `attach` |
| tree event naming a known window | that window | `events` |
| window event, lifecycle `ready`, tree event for an unknown window, `note_window_change` | every window | `events` |
| input the mirror injected, for windows without events | those windows, when the tree is next needed | `action` |
| frame hash changed since such a window's last read | it, 200 ms after that read ended at the soonest, and once more after frames stop | `frames` |
| 1 s since such a window's last read began | it | `poll` |
| while settling without frames, bounds probes found anchors held still where the replica does not have them | their windows, cache cleared | `moved` |
| sequence gap, driver's out-of-sync report, reconnection, another backend | every window, identities started over | `resync` |

Reads are matched against the previous read of the same window: nodes under
matched parents pair up by class and identifier, siblings that share both by
subtree content, then by sharing an identified descendant; unpaired nodes get
new identities, so an identity never moves to another element. Differences
become deltas (`Mirror::deltas`) and advance `Mirror::revision`. Events on a
lossless stream numbered at or below the last read's `seq` or the driver's
resync are skipped; a skipped number starts the replica over. Events on
streams this version does not know are ignored. Once disconnected, the mirror
stops reading, serves the last replica and refuses actions.

The daemon keeps a mirror current between commands with `pump` (apply what
arrived, perform due reads, never wait) or `run_until(deadline)`, and
`next_wakeup` says when the next poll or frame-driven re-read falls due.
`drain` applies what arrived without reading the tree, even when a read is
due, and returns how many items it applied; the reads those items call for
happen at the next `pump`, snapshot, wait or ref.

What the daemon needs to know about a target's timing it reads from the
mirror, not from the backend (task 7.10):

| Call | Gives |
|---|---|
| `timestamps().last_frame` | the latest frame the mirror took in: capture time, number, hash |
| `timestamps().last_frame_change` | the capture time of the latest frame whose hash differed from the one before; the first frame is no change |
| `frame_changes_since(t)` | the capture times of the frame changes at or after `t`, oldest first, among the latest 1024 |
| `timestamps().last_tree_change` | the latest tree change: a tree or window event's arrival, a noted window change, the end of a read that found a delta, or the end of a bounds probe that found nodes moved; the first acquisition is no change |
| `timestamps().last_read` | the latest read taken into the replica: `started`, `finished` and `reason` |
| `timestamps().last_action` | when the mirror last injected input, once the platform accepted it |

Items still queued in the event stream count once drained, so a caller
drains before it reads the timestamps. `note_window_change` records that the
window in front changed outside the mirror's input, as launching or stopping
an app does through the daemon: the mirror treats it as a window event
arriving at that moment, so every window is read again when the tree is next
needed and settle counts it as a tree change. A settle that starts right
after a launch then waits for the launched app even when its first frames or
events arrive later, and a driver need not queue a window event of its own
for the launch (the Simulator driver's way of reporting one, which settle
also counts).

## Snapshots

`Snapshot::text` is agent-browser's listing, one element per line:

```
- textbox "Username" [id=login-username, ref=e1]: alice
- button "Sign in" [id=login-submit, ref=e3]
- switch "Animate" [checked=true, id=anim-toggle, ref=e4]
(3 more actionable items below, scroll down to reveal)
```

- Names are normalized (no-break spaces become spaces, zero-width characters
  go) and JSON-escaped as agent-browser escapes them. Attributes come in the
  order `checked=`, `selected`, `disabled`, `focused`, `id=`, `ref=`; a value
  follows `: ` when it differs from the name.
- An element's name is its own label, else the labels of its non-actionable
  descendants, which are then not listed again. A tapped container takes the
  control role a child carries (Material in Compose), and a node repeating its
  parent's role, name and identifier is folded into it (XCTest's nested alert
  buttons). A field with no label of its own takes the label of the nearest
  labelled container it sits in, when that container is neither actionable
  nor a window's root and holds no other field, as React Native lays out a
  field whose wrapper carries the label; the container is then not listed
  again.
- Refs go to actionable elements; to named headings, cells and list items;
  and to labelled generic nodes whose children, if any, are static text,
  other than a window's root and the nodes an actionable element holds,
  which name it or stay its content. React Native exposes a view marked
  accessible without a role this way, as Bluesky's sheet options and
  language rows. On iOS it hides a field inside such a view, so the view is
  all the read has of the field, and a tap by its ref lands on its center,
  over the field.
  An interactive listing (`SnapshotOptions::interactive()`) keeps only
  elements with refs, as agent-browser's `-i`. A full listing leaves out
  window roots and unnamed, unidentified generic containers, texts and images,
  listing their children in their place.
- Application windows come first, then system bars; a keyboard window is one
  `- keyboard` line. The top-most modal, a modal window or an alert or dialog
  node, cuts everything it covers.
- Scroll hints come from actionable nodes outside a container's viewport when
  the read has them, and from the container's scroll semantics otherwise
  (Compose reports only the nodes in the viewport): a range with a position
  counts the items beyond it (`(184 more items below, ...)`), and a range in
  other units or none at all gives the ways the container can scroll
  (`(more items below, ...)`); back reads as above or left, as in a
  left-to-right layout. Content above or left is hinted before the
  container's children, below or right after them.
- Empty listings read `(no interactive elements)` and `(empty screen)`.
- Refs are `eN`, parsed from `@eN`, `ref=eN` or `eN`; numbers only grow. A
  ref names one control: its element, and for an element without an
  identifier, also its accessible name. An element keeps its ref while it
  survives, and without an identifier, while it keeps its name; renamed, it
  is another control, which the next snapshot gives a fresh ref, and the old
  ref is refused as stale from the moment the rename is read. `removed_refs`
  lists the refs the previous snapshot listed and this one does not.

Every snapshot carries a `Verdict`: the backend, whether it is degraded, and
the reasons in this order, each with advice:

| Reason | When |
|---|---|
| `fallback_backend` | the read came from a fallback backend, such as the Simulator's translator |
| `truncated` | a depth, node or window bound cut the read short |
| `sparse_tree` | an application window of the target came back as its root alone |
| `sibling_collision` | listed refs whose identity without positions (window, then class, identifier and content down the path) coincides |
| `agent_warning` | the agent reported reduced ability or lost its UI automation role |
| `disconnected` | the target is gone; the snapshot is the last screen seen |

## Settle, waits, diffs, deltas and frames

| Setting | Default | Source |
|---|---|---|
| `DEFAULT_QUIET_WINDOW` | 200 ms | spike S1: content changes every 101 to 116 ms, gaps up to 187 ms inside a scroll |
| `DEFAULT_SETTLE_MAX` | 3 s | design D8 |
| `DEFAULT_POLL_INTERVAL` | 1 s | chosen here; spike S1 measured cold Compose reads at 21 to 349 ms |
| `DEFAULT_MIN_REREAD_INTERVAL` | 200 ms | design D5: frame-driven re-reads bounded by a minimum interval |
| `DEFAULT_PROBE_INTERVAL` | 50 ms | task 10.11: four probes to a quiet window, so a settle without frames ends at most 50 ms after the quiet window that follows the last move |
| `DEFAULT_PROBE_ANCHORS` | 4 | task 10.11: nodes per probe, one round trip to the app each |
| `DEFAULT_OVERSCROLL` | 500 ms | task 10.14: on the API 36 emulator the Expo list pulled against its end drew its stretch for 333 to 383 ms after the finger lifted; 117 ms beyond the longest |
| `DEFAULT_MAX_DIMENSION` | 1000 px | spike S2's measured output size |
| `DEFAULT_QUALITY` | 80 | spikes S1 and S2 |

- `settle` ends once neither tree changes (events as they arrive, window
  changes noted to the mirror, deltas of re-reads) nor frame hash changes
  have happened for the quiet window, counted from the last change or
  injected input, and otherwise at the
  maximum with `settled: false` and the signals that kept changing. Frame
  callbacks without pixel changes do not count (spike S2). Without a frame
  source, windows without events are re-read every minimum re-read interval
  while waiting. `Signals::TreeOnly` ignores frames, for video.
- Without a frame source, a node that moves without a change event, as a
  view an app animates may, changes nothing the tree reports, so the quiet
  window could pass while the screen still moved (task 10.11). When the
  tree source can probe, the wait reads the bounds of up to
  `probe_anchors` anchors every `probe_interval`: elements with refs on
  screen and not covered, in application windows, spread from the first to
  the last (those off screen stand in while none is on it). A move between
  two probes, or a first probe that finds the replica stale, is a tree
  change; anchors that held still since the last probe where the replica
  does not have them bring a `Moved` read of their windows, the cache
  cleared, whose delta is a tree change too. The wait settles only once a
  probe that began after the last change found every anchor where the
  replica has it, so it ends with the replica holding where the screen came
  to rest. A probe the source fails with a retryable error is tried again
  at the next interval; any other failure, or a target that is gone, stops
  the probing and leaves the wait to the tree. With frames nothing is
  probed.
- Without a frame source, the wait does not settle either while an
  application window of the replica is flagged `transformed` and is not in
  picture-in-picture (task 10.11): from Android 14 the platform reports a
  window it animates in or out at one frame of that animation, to reads
  and probes alike, until the animation ends and the window's bounds
  change at once, so neither the quiet window nor the probes can see it
  move. The window change the platform then reports brings a read, and the
  wait settles a quiet window after it. A window that stays flagged holds
  the wait to the maximum, whose result names the tree. With frames the
  flag changes nothing.
- Without a frame source, after a stroke that leaves the content under it
  at an end of its range, the wait does not settle either until the
  `overscroll` hold has passed since the stroke and since the tree's last
  change (task 10.14). The content is a visible node under the point where
  the stroke came down, scrolling along the stroke's axis (or along an axis
  its platform does not name), that can scroll one way from where it is but
  not the other, as the replica has it. From Android 12 a scroll view that
  a stroke pushes against an end, or that a fling carries into one, draws a
  stretch that springs back on its own; it moves no view and sends no
  event, and until it rests the scroll view takes a touch that comes down
  as the start of a drag that catches the stretch, so a tap on a row opens
  nothing. The hold follows the latest input only while it is that stroke
  and the app in front has not changed; a maximum that passes during it
  names the tree. With frames, which show the stretch, the hold changes
  nothing.
- `visibility` tells, from the replica as it is and without reading the
  target, how much of an element is in view (task 10.15). Its visible
  bounds are its window's, clipped by every scroll container around it (a
  node that says it scrolls, a list or grid, or one with scroll
  semantics), less the bars drawn over them: system and keyboard windows
  above its window, and tab bars and toolbars of its window that neither
  hold it nor scroll with it. A bar cuts the bounds where it spans them
  from one side to the other and reaches an edge, and one over all of
  them leaves nothing. `fraction` is the share of the element's bounds
  inside them. Android reports bounds clipped to every ancestor, so an
  element its list cuts off reads as the part that shows (a row of 48
  points read as 1.2 points tall at the Expo list's edge on the API 36
  emulator), while iOS reports whole frames: an element whose bounds end,
  within a pixel, at an edge of its scroll containers' bounds where one of
  them can scroll further, as its scroll semantics say, is `cut` there,
  and how much of it lies beyond is unknown. A container without scroll
  semantics cannot tell, and cuts nothing.
- `scroll_viewport` tells, the same way, the visible bounds of the scroll
  container a stroke through a point moves (task 10.16): the innermost
  node on screen and not covered that scrolls along the stroke's axis, or
  along an axis its platform does not name, and holds the point, in the
  top-most window that has one, with the visible bounds an element inside
  it gets. The daemon bounds each stroke of `scroll --until` by their
  extent along the axis, so that no element passes through them between
  two looks.
- `wait` evaluates after every item the target reports and every re-read,
  sleeping on the event stream in between; selectors match text in the name
  or value, the identifier, the role and a ref, among elements on screen and
  not covered. A timeout fails with `wait_timeout` and a summary of an
  interactive snapshot.
- `diff` matches elements by identity and lists added, removed and changed
  elements with the changed fields; bounds are not compared.
- `delta_snapshot` follows agent-browser's `--delta`: revisions count from 1;
  `full`, `unchanged` or `delta` with ref ops (`add`, `remove`, `replace` of
  `role` or `name`) and a line splice; the whole listing again when the
  delta's JSON is 70% of the listing or more, or when the options, the
  replica generation or the foreground owner changed. `to_json` writes the
  bytes agent-browser's `serde_json` writes.
- `frame` fits the latest frame's upright size to the long edge without
  enlarging it and asks the frame source to rotate and encode; it reports
  the capture time, the age and whether the frame is newer or older than the
  last tree read.

## Errors

| Code | Retryable | Meaning |
|---|:-:|---|
| `stale_ref` | no | the ref's element is gone, or the replica was rebuilt after it was issued; take a new snapshot |
| `unknown_ref` | no | not a ref this mirror issued |
| `element_covered` | no | a modal covers the ref's element |
| `element_offscreen` | no | the ref's element is scrolled out of view |
| `wait_timeout` | yes | the condition did not hold in time; the error carries the screen |
| `no_frame` | yes | no frame source, or no frame yet |
| `disconnected` | no | the target is gone |

Backend errors keep the driver's code and retryable flag.

## Fixtures

`fixtures/*.json` are small synthetic trees in the shape of the reference
apps, in device points with the scale beside them:

| Fixture | Shape |
|---|---|
| `login` | Expo login on Android: status bar and app windows, React Native views that send events |
| `compose-anim` | Compose Anim tab: an event-less window, a tagged toggle with its label and switch on child nodes |
| `list` | Expo list: 200 rows, rows 1 to 16 visible, 17 to 19 mounted below the fold |
| `list-compose` | Compose list: 16 rows in the viewport and scroll semantics for 200 |
| `grid` | SwiftUI Big tab: a header and 500 cells in a 20-column grid |
| `login-ios` | SwiftUI login read by the Simulator's in-guest reader |
| `alert` | the SwiftUI login under its in-app alert, with the covered screen still in the read |
| `system-alert` | a location prompt in SpringBoard's tree |
| `login-translator` | the SwiftUI login as the host translator flattens it (fallback backend) |
| `sparse` | an application root alone |
| `signin-android` | Ignite's sign-in screen on Android: each field an unlabelled `EditText` inside a focusable view that carries its label |
| `signin-ios` | the same screen in XCTest's classes: each field an unlabelled text field inside a labelled `Other` |
| `signin-ios-hidden` | the same screen as the React Native smoke test read it on iOS: labelled `Other`s with their fields hidden |
| `bluesky-signin` | Bluesky's sign-in form on iOS: Forgot password? is the form's only button without an identifier |
| `bluesky-forgot` | the forgot-password form it opens: Next is the form's only button without an identifier, where Forgot password? sat |
| `bluesky-hosting` | Bluesky's hosting provider sheet: Automatic and Manual as labelled `Other`s over static text, Dismiss as a labelled leaf, Done a button |
| `bluesky-language` | Bluesky's language selector: 16 rows as labelled `Other`s over static text, 10 in the viewport |
| `events-typing` | keystrokes into the login fields, one event each |
| `events-gap` | typing with event 2 lost in transit |
| `events-resync` | the driver reporting the stream out of sync and resynchronized |

A node is `{"class", "id", "text", "desc", "bounds": [l, t, r, b], "flags",
"off", "children"}`; flags default to `enabled` and `visible`, `flags` adds
and `off` removes. `{"repeat": [first, last], "layout": {"rows" | "grid":
...}, "viewport": [...], "node": ...}` unrolls a template with `{n}`
substituted and hides what falls outside the viewport; `"bounds": "fill"`
copies the parent's. A window takes `"events"`, `"owner"` and `"modal"` for
its window metadata, and a node `"scroll": {"axis", "back", "forward",
"range": [unit, min, max], "position": [first, last]}` for its scroll
semantics, unit `"items"` or `"percent"`. A timeline names its `start`
fixture and lists steps at millisecond offsets, each with a `patch`
(`find`, which sets any field, `remove`, `insert`, `insert_before`,
`add_window`, `remove_window`, `screen`, `backend`) and an `event` (with an
optional explicit `seq`), `drop`,
`frame`, `out_of_sync`, `resynced`, `backend_changed` or `reconnected`.

```sh
cargo test -p mirror
```

## Interpretations of the spec

- *Windows without change events* asks for a re-read after every injected
  action. The mirror re-reads when the tree is next needed (a snapshot, a
  wait, a ref), so the read sees the action's effect rather than racing it,
  and design D5's frame-driven re-reads catch later changes.
- *Settle predicate*: the quiet window counts from the last observed change
  or injected input. A screen that has been quiet that long settles at once,
  and a tap that changes nothing settles one window after the tap. On a
  target without frames whose tree source can probe, moves the probes find
  are changes too, and the screen is settled only once a probe after the
  last change found its anchors where the replica has them (task 10.11): a
  still screen settles after that one probe. Nor is it settled, without
  frames, until the overscroll hold has passed since a stroke that left a
  list at an end of its range and since the tree's last change (task
  10.14).
- *Snapshot content* asks for bounds, which snapshot lines leave out as
  agent-browser's do; `Element::bounds` carries them in points. The lines add
  `id=` and `focused`, which agent-browser has no counterpart for, and hint
  lines in parentheses, as agent-browser writes its empty-page lines.
- *Snapshot content* keeps actionable elements in an interactive listing.
  The mirror counts among them a labelled generic node whose children, if
  any, are static text, outside actionable elements, which the platform
  exposes as an element without traits. The React Native smoke test's iOS
  read of Ignite's sign-in screen shows each field as such a node with
  nothing below it, labelled Email or Password, the text field hidden, and
  agent-device lists them as cells (`reports/10.4-rn-smoke.md`, finding 1);
  Bluesky's sheet options and language rows are such nodes over static
  text, which agent-device lists as `Other` and `Cell` (second run, finding
  4). Where the read keeps the field, the field is the element, named after
  its container; a container of two fields names neither.
- *Stable refs*: siblings that share class, identifier and subtree content
  cannot be told apart; their refs follow order, and the verdict reports a
  `sibling_collision` rather than merging them (design D5, Risks).
- *Stable refs* keeps a ref while its element survives. The replica pairs
  the only sibling of a class without an identifier with the only such
  sibling in the next read, so an element can survive a screen transition
  as another control: Bluesky's Forgot password? became the forgot-password
  form's Next, and its ref with it (second run, finding 6). A ref's
  identity therefore includes the accessible name of an element without an
  identifier. A control renamed in place, such as a counter's label, also
  gets a fresh ref; an identified element keeps its ref under any name.
  Element identities, and with them deltas and structural diffs, still
  follow the element.
- *Quality verdict*: a root-only application window is reported as sparse
  without inspecting the frame; the advice points to the frame.
- *Resynchronization* also covers a read that comes back from another
  backend without a `BackendChanged` item, and a reconnection.
- *Frame access* reports the frame's age from now and its offset from the
  last tree read (`TreeSkew`).
- Scroll semantics and window metadata come from the protocol's snapshot
  (task 3.6); the mirror keeps only `Owner`, the resolved form of a
  window's `owner`. A read without `emits_events` falls back to inferring
  event-less windows from any node whose class is under `androidx.compose.`:
  `androidx.compose.ui.platform.AndroidComposeView`, the class the fixtures
  name, and `androidx.compose.ui.platform.ComposeView`, which a Compose
  window shows on the API 36 emulator. A Compose dialog shows no Compose
  class at all there, which no inference from classes catches, so the
  Android agent reports `emits_events` whenever its walk can tell rather
  than leaving it to this inference, and the field, when present, decides.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Replica maintained from device events | Warm snapshot is served locally | `replica::a_warm_snapshot_is_served_without_reading_the_device` |
| | First snapshot after attach | `replica::the_first_snapshot_after_attach_acquires_the_whole_tree_then_applies_events`, `replica::recorded_event_sequences_replay_to_the_trees_a_cold_read_returns`, `replica::a_window_change_reads_every_window` |
| Windows without change events | Action on an event-less window | `replica::a_tap_on_an_event_less_window_is_reflected_by_a_full_re_read_with_its_cost`, `replica::window_metadata_that_a_window_sends_no_events_puts_it_on_the_re_read_path`, `replica::window_metadata_that_a_window_sends_events_keeps_it_off_the_re_read_path`, `replica::without_window_metadata_any_androidx_compose_class_puts_a_window_on_the_re_read_path` |
| | Idle event-less window | `replica::an_idle_event_less_window_is_re_read_once_per_poll_without_deltas_when_unchanged`, `replica::an_event_less_window_that_changed_on_its_own_yields_a_delta_at_the_next_poll`, `replica::frame_changes_re_read_event_less_windows_at_most_every_minimum_interval_and_once_after_they_stop` |
| Snapshot content | Interactive snapshot of a scrolling list | `interactive::an_interactive_list_snapshot_lists_visible_rows_and_hints_three_below`, `interactive::scroll_semantics_give_the_hint_when_offscreen_rows_are_absent`, `interactive::scroll_semantics_without_an_item_position_say_which_ways_more_lies`, `interactive::a_modal_window_cuts_the_application_windows_below_it`, `interactive::an_actionable_container_takes_its_name_from_non_actionable_descendants`, `interactive::every_interactive_line_carries_a_ref`, `interactive::the_full_listing_follows_agent_browser_lines`, `interactive::an_empty_listing_says_so_as_agent_browser_does`, `interactive::an_alert_cuts_the_content_it_covers`, `interactive::a_system_alert_reports_system_ui_as_the_foreground`, `interactive::the_keyboard_window_is_listed_as_one_line` |
| | (task 7.11: fields in labelled containers) | `editables::a_field_in_a_labelled_container_takes_the_containers_label_on_android`, `editables::a_field_in_a_labelled_container_takes_the_containers_label_on_ios`, `editables::a_container_names_only_its_single_unlabelled_field`, `editables::a_labelled_container_whose_field_the_platform_hides_gets_a_ref`, `editables::only_labelled_containers_with_nothing_but_static_text_below_them_get_such_refs`, `editables::fields_below_the_fold_are_hinted_once_each`, `editables::a_named_field_under_an_alert_is_covered_and_its_ref_says_which_field`, `editables::fields_in_identically_labelled_containers_collide` |
| | (task 7.12: labelled controls without traits) | `controls::labelled_controls_without_traits_on_the_hosting_provider_sheet_get_refs`, `controls::language_rows_get_refs_and_those_below_the_fold_are_hinted`, `controls::a_labelled_control_under_an_alert_is_covered`, `controls::inside_a_clickable_ancestor_such_controls_name_it_and_get_no_ref`, `controls::identical_labelled_controls_collide`, `controls::nodes_with_more_than_static_text_below_them_or_no_label_are_no_such_controls`, `editables::only_labelled_containers_with_nothing_but_static_text_below_them_get_such_refs` |
| Stable refs | Element survives between snapshots | `refs::a_surviving_button_keeps_its_ref_when_the_screen_changes_elsewhere`, `refs::typing_into_a_field_keeps_its_ref`, `refs::refs_only_increase_and_are_never_reused`, `refs::identical_siblings_are_reported_as_a_degraded_sibling_collision` |
| | Stale ref is refused | `refs::acting_on_a_removed_element_fails_with_a_stale_ref_error_and_guidance`, `refs::a_stale_ref_never_resolves_to_another_element`, `interactive::a_stale_ref_names_the_element_it_was_issued_for`, `refs::a_ref_that_was_never_issued_is_unknown`, `refs::refs_parse_in_every_form_agent_browser_accepts`, `interactive::acting_on_an_element_an_alert_covers_fails_with_its_code`, `interactive::acting_on_a_row_scrolled_out_of_view_fails_with_its_code` |
| | (task 7.12: a ref names one control) | `controls::a_stale_ref_to_a_control_a_transition_renamed_is_refused_on_ios`, `controls::the_control_that_took_its_place_gets_a_fresh_ref_and_the_old_one_stays_stale`, `controls::a_wait_on_the_old_ref_sees_the_renamed_control_gone`, `controls::a_control_named_by_its_text_gets_a_fresh_ref_when_the_text_changes_on_android`, `controls::an_identified_control_keeps_its_ref_when_its_name_changes`, `controls::typing_into_a_field_without_an_identifier_keeps_its_ref` |
| Single coordinate contract | Tapping reported bounds hits the element | `refs::tapping_the_center_of_reported_bounds_hits_the_element`, `replica::bounds_are_in_device_points_with_the_scale_factor_beside_them`, `refs::text_into_a_ref_names_the_node_of_the_latest_read` |
| Settle predicate | Screen settles | `settle::a_transition_settles_shortly_after_it_ends_and_reports_the_elapsed_time`, `settle::the_quiet_window_defaults_to_200_ms_and_the_maximum_to_3_s`, `settle::frame_callbacks_without_pixel_changes_do_not_hold_the_wait`, `settle::a_screen_that_has_been_quiet_settles_at_once`, `settle::the_default_window_does_not_end_inside_a_scroll_while_150_ms_would`, `settle::without_frames_an_event_less_window_is_re_read_while_waiting`, `settle::a_launch_noted_to_the_mirror_holds_settle_until_the_app_has_drawn`, `settle::a_launch_counts_as_a_change_whether_noted_or_reported_as_a_window_event` |
| | Continuous animation | `settle::a_continuous_animation_is_not_settled_after_the_maximum_and_names_frame_changes`, `settle::an_animation_that_also_sends_tree_events_names_both_signals`, `settle::tree_only_settling_ignores_frames` |
| Wait predicates from the event stream | Element appears | `waits::a_wait_for_text_resolves_within_one_processing_cycle_of_its_appearance`, `waits::a_disappear_wait_resolves_when_the_element_leaves`, `waits::an_enabled_wait_resolves_when_the_element_becomes_enabled`, `waits::a_wait_on_a_window_without_change_events_resolves_from_its_re_reads`, `waits::a_wait_that_already_holds_returns_at_once`, `waits::selectors_match_text_identifier_role_and_ref_on_screen_only` |
| | Wait times out | `waits::a_wait_that_never_matches_fails_at_the_timeout_with_the_last_snapshot_summary` |
| Structural diff | Diff after a form submit | `diff::a_form_submit_diff_lists_removed_fields_and_added_confirmation`, `diff::a_diff_against_an_unchanged_screen_is_empty`, `diff::changed_elements_list_the_fields_that_changed`, and for agent-browser's delta snapshots `diff::delta_snapshots_follow_agent_browser_kinds_and_revisions`, `diff::delta_ref_changes_follow_agent_browser_ops`, `diff::a_delta_of_70_percent_or_more_of_the_listing_falls_back_to_full`, `diff::changing_the_listing_options_starts_again_with_the_whole_listing` |
| Quality verdict on every snapshot | Degraded read is flagged | `resync::a_root_only_read_of_a_populated_screen_is_degraded_with_advice`, `resync::truncated_reads_are_degraded_naming_the_limit`, `resync::agent_warnings_and_disconnection_reach_the_verdict`, `refs::identical_siblings_are_reported_as_a_degraded_sibling_collision` |
| Frame access | Scaled frame | `frames::a_scaled_frame_fits_the_requested_long_edge_upright_with_its_timestamp`, `frames::a_rotated_frame_is_encoded_upright`, `frames::small_frames_are_not_enlarged_and_defaults_apply`, `frames::quality_defaults_to_80_and_is_clamped`, `frames::a_frame_older_than_the_last_tree_read_says_so`, `frames::no_frame_is_an_error_with_its_code` |
| Replica maintained from device events | (task 7.10: timing and draining for the daemon) | `timestamps::nothing_has_happened_before_the_first_read_and_the_first_read_gives_its_span`, `timestamps::the_last_frame_and_frame_change_come_from_the_frames_the_mirror_took_in`, `timestamps::only_the_latest_1024_frame_changes_are_kept`, `timestamps::the_last_tree_change_is_an_event_arrival_or_the_end_of_a_read_that_found_a_delta`, `timestamps::the_last_action_is_when_the_mirror_injected_input`, `timestamps::drain_takes_in_what_arrived_without_reading_the_tree` |
| Resynchronization | Backend fallback | `resync::a_backend_fallback_rebuilds_the_replica_stales_old_refs_and_names_the_fallback`, `resync::a_read_from_another_backend_rebuilds_the_replica_even_unannounced`, `resync::a_sequence_gap_rebuilds_the_replica_and_stales_earlier_refs`, `resync::an_out_of_sync_report_from_the_driver_also_rebuilds_the_replica`, `resync::events_a_read_already_reflects_are_skipped_but_a_gap_is_not`, `resync::a_reconnection_rebuilds_the_replica_and_restarts_sequence_numbers`, `resync::events_on_streams_this_version_does_not_know_are_ignored_even_with_gaps` |
| (task 10.15, how much of an element is in view) | (the share in view, the cut edge, the bars) | `visibility::a_row_inside_its_list_is_wholly_in_view_within_the_list_s_bounds`, `visibility::a_row_android_clips_at_the_bottom_edge_of_a_list_that_scrolls_on_is_cut_there`, `visibility::a_row_android_clips_at_the_top_edge_of_a_list_that_scrolls_back_is_cut_there`, `visibility::a_row_at_the_edge_of_a_list_that_cannot_scroll_that_way_is_whole`, `visibility::without_scroll_semantics_an_edge_is_no_cut`, `visibility::bars_over_a_list_cut_its_visible_bounds_and_a_whole_frame_shows_its_share`, `visibility::system_bars_and_the_keyboard_above_the_app_cut_its_visible_bounds`, `visibility::an_element_outside_any_scroll_container_is_measured_against_its_window`, and the unit tests `visibility::tests::*` |
| (task 10.16, the scroll container a stroke moves) | (its visible bounds, the bars, the axis) | `visibility::a_stroke_through_a_list_moves_the_list_and_its_visible_bounds_are_its_own`, `visibility::the_bars_over_a_list_are_left_out_of_its_visible_bounds`, `visibility::a_stroke_moves_the_innermost_container_that_scrolls_along_its_axis`, `visibility::a_point_outside_every_scroll_container_has_none` |
