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
                timestamps, note_window_change, cancel_on, visibility, covered_by, hit, holds, twin,
                scroll_viewport, scroll_container_in, uncovered, keys_hidden, edit_menu, field_within,
                fields_within
  timestamps.rs when the latest frame, frame change, tree change, read and input happened
  replica.rs    windows and node trees, identity matching between reads, deltas, sequence numbers, resync
  view.rs       roles, names, visibility, what modals, views drawn over others, hosts, bars and
                pinned headers cover, tabs by their row, scroll hints
  visibility.rs how much of an element is in view: its visible bounds, the part a tap reaches,
                its share and cut edge; the visible bounds of the scroll container under a
                point, or of the one showing the most of an area; what of an area the system
                and keyboard windows above it leave a stroke
  roles.rs      roles from Android widget classes, XCUIElementType names and UIKit classes
  render.rs     listings and agent-browser's snapshot lines, what the keyboard covers of them
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
benches/        microbenchmarks of pump, snapshot, visibility, scroll_viewport, uncovered,
                delta_snapshot, diff, wait and settle over grid, list, bluesky-typeahead-android
                and grid-rn on the fake target (design D14 of the microbenchmarks change), each
                printing beside criterion's timing the allocations and bytes of a call, which
                three verified calls must agree on; run them with
                `cargo bench -p mirror -- --noplot [filter]` (the library sets bench = false, so
                its built-in bench harness never sees criterion's options), or with every suite
                against the committed baseline through `just micro` (reports/micro/README.md)
tests/          one file per task: replica, refs, interactive, settle, waits, diff, resync, frames,
                timestamps, editables, controls, visibility, cancel, and bench_inputs for the
                fixtures the benchmarks read
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
| `Input` | `Tap { at, hold }`, `Swipe { from, to, duration, release }`, `Key { code, meta }`, `Text { text, node: Option<NodeHandle>, method }` |
| `Release` | `Moving` (the finger lifts at the end of its path, still moving, and the platform flings the content on), `Resting` (the finger comes to rest before it lifts, and the content stops where it left it; how the platform is made to see it stop is the driver's) |
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
  Simulator drivers and the physical iPhone driver set it for every window. Without it the mirror infers
  it: a window holding a node of any class under `androidx.compose.` sends
  none (spike S1), which covers the
  `androidx.compose.ui.platform.AndroidComposeView` spike S1 saw and the
  `androidx.compose.ui.platform.ComposeView` the API 36 emulator shows.
  `owner` names the system process that owns a window, `modal` marks a
  window that cuts the application windows below it. The `modal` node flag
  marks a view the platform marks modal inside its window, which the
  Simulator's reader finds for a view an iOS app marks
  `accessibilityViewIsModal`, and the `on_top` node flag a node drawn after
  every sibling its bounds overlap, which the Android agent tells from the
  order a parent draws its children in (task 10.28). An iOS driver needs
  no such flag: XCTest lists a view's subviews in the order UIKit draws
  them, which the mirror reads as the drawing order in a read whose
  windows are rooted at `XCUIElementType` classes, from a backend that is
  not a fallback (rn-app-fixes, design D31), so the driver keeps the
  children in XCTest's order.
  An input-method window's bounds are what the keyboard covers, as the
  Android agent gives them, the pieces of the window's touchable region
  that hold what it shows (task 10.25), and as the Simulator's reader gives
  the keyboard's window, which spans the screen (task 10.21): none for a
  window that draws nothing, as while a hardware keyboard serves the field
  in focus and iOS keeps the keyboard's keys below the screen, or the strip
  it keeps for a keyboard toolbar (rn-app-fixes, design D52). The reader
  bounds the keyboard's window by its host, which also holds the place of
  the input accessory view of the field in focus, and the app draws that
  view in its own window, as a browser draws its address bar above the
  keyboard: the mirror takes from a keyboard window's bounds the band
  across their top that a child of the host, the node whose bounds the
  window's are, holds while it holds nothing, no children, name or value,
  across the host from its top (real-app-fixes, task 5.6). With the
  address bar in focus on an iPhone 17 the host reached from 498 points
  down and the place it kept from 498 to 566, the keyboard drawing from
  566, so the field, from 510 to 558, read as under the keyboard; with a
  text field in focus the host starts at the prediction bar, 539 points
  down, and holds no such place. The text
  effects window, which holds iOS's edit menu and spans the screen too,
  stays in the application's tree after the application's window, and the
  Simulator's reader gives its node's bounds as what it draws, the menu's
  list or nothing, so a later sibling's cover holds the menu's strip alone
  (design D55).
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
| input the mirror injected, for windows without events | those windows, when the tree is next needed; on a target whose frames come slowly (`slow_frames`), once the first frame that changed after the input arrived, at most `first_frame_wait` (500 ms) after the input | `action` |
| frame hash changed since such a window's last read | it, 200 ms after that read ended at the soonest, and once more after frames stop | `frames` |
| text the mirror pasted or set into a field of such a window, not typed as keys, until a read shows the field's text changed, the next input, or 1 s after the text | the field's window, as soon as a frame changes after its last read began; without frames, as soon as that read ends (task 10.29) | `frames`; `action` without frames |
| 1 s since such a window's last read began; on a target whose frames come slowly (`slow_frames`) and that has a frame source, `still_poll_interval` (10 s), since frames that change re-read the window as the row above says | it | `poll` |
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
arrived, perform due reads, never wait, but for the re-read after input on
a target whose frames come slowly, which first waits for the input's first
changed frame) or `run_until(deadline)`, and
`next_wakeup` says when the next poll, frame-driven re-read or re-read after
text falls due.
`drain` applies what arrived without reading the tree, even when a read is
due, and returns how many items it applied; the reads those items call for
happen at the next `pump`, snapshot, wait or ref.

`cancel_on(flag)` gives the mirror a flag that ends its waits (design D7 of
mcp-server): `run_until`, `wait` and `settle` look at it before they block
on the event stream and at least every `CANCEL_SLICE` (50 ms) while they
do, and fail with `cancelled` while it is up. A read or an input in flight
runs to its end first, and a wait with nothing to wait for, as a settle on
a screen already quiet, ends as it would without the flag. Without a flag
the waits block until their next item or wake-up; with one lowered they
block in slices, with the same reads and results. The daemon raises a
target's flag for its session's `cancel` and lowers it once the command in
flight is over.

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
  order `checked=`, `selected`, `disabled`, `focused`, `visible=`, `id=`,
  `ref=`; a value follows `: ` when it differs from the name.
- An element's name is its own label, else the labels of its non-actionable
  descendants, which are then not listed again. A tapped container takes the
  control role a child carries (Material in Compose), and a node repeating its
  parent's role, name and identifier is folded into it (XCTest's nested alert
  buttons). So is the only child of a text or a heading that repeats its
  name, as text or in its role, without an identifier of its own: XCTest
  reports React Native's text as a text whose only child is its own text,
  a heading likewise without the heading's identifier, and UIKit's scroll
  bars as elements whose only child repeats them, which the full listing
  then shows once (rn-app-fixes, design D35). A field with no label of its
  own takes the label of the nearest labelled container it sits in, when
  that container is neither actionable nor a window's root and holds no
  other field, as React Native lays out a field whose wrapper carries the
  label; the container is then not listed again.
- React Native says that a view is a checkbox, and whether it is checked,
  only in its value on iOS, where no trait says so (`checkbox, checked`,
  `checkbox, unchecked`, in English): such a node lists as a checkbox with
  `checked=`, as a switch does, its value not shown (D35).
- Refs go to actionable elements; to named headings, cells and list items;
  and to labelled generic nodes whose children, if any, are static text,
  other than a window's root and the nodes an actionable element holds,
  which name it or stay its content. React Native exposes a view marked
  accessible without a role this way, as Bluesky's sheet options and
  language rows. On iOS it hides a field inside such a view, so the view is
  all the read has of the field, and a tap by its ref lands on its center,
  over the field. A selected one gets a ref whatever it holds: React Native
  exposes a tab this way on iOS, with no trait but `selected`, and the
  production-app smoke test's selected tab, which also showed its label under its icon,
  got none (D35). The other tabs of such a bar have neither the trait nor
  anything else to tell them by (the production-app run of 2026-10-12 found an
  unselected tab holding its icon's glyph as a text whose only child is its
  own text, and no ref for it), so the labelled generic nodes laid out in
  one row with a selected one among its siblings, each overlapping it over
  at least half the height of the shorter, list as tabs with refs, the
  selected one with its state; a row without a selected node, or inside an
  actionable element, changes nothing (rn-app-fixes, design D40).
  An interactive listing (`SnapshotOptions::interactive()`) keeps only
  elements with refs, as agent-browser's `-i`. A full listing leaves out
  window roots and unnamed, unidentified generic containers, texts and images,
  listing their children in their place.
- Application windows come first, then system bars; a keyboard window is one
  `- keyboard` line, but for one that draws nothing, which counts as no
  keyboard (rn-app-fixes, design D52): with the Simulator's hardware
  keyboard connected iOS draws no keys, and the fifth production-app run's
  listings counted every control below 274 points as under a keyboard the
  screen did not show. An interactive listing leaves out the elements that
  lie wholly under a keyboard window, bounded by what the keyboard draws
  (tasks 10.21 and 10.25), and counts them in the keyboard's place, `(2 more
  actionable items under the keyboard, close it to reveal)`; an element
  partly under one says how much of its bounds shows, `visible=35%`, a whole
  percent and at least 1, in either listing (rn-app-fixes, design D38): in
  the production-app run of 2026-10-12 the elements under the keyboard stayed
  listed, and clicks on a submit button 1 to 35% in view tapped its centre
  on the keyboard. An interactive listing likewise leaves out an element a
  tap reaches none of (`visibility`, below), whatever covers it, a header, a
  view drawn over it or the edge of what its list shows, and counts it in
  its scroll container's hint, and marks one a tap reaches only part of with
  that share, `visible=50%`, the less of it and the share beside the
  keyboard (design D45): the third production-app run's listings gave no sign of
  a heading half under a footer button or of a card 70% under a header, and
  listed a card scrolled above the list's top and rows behind a header. Most
  elements lie within what their containers show, with no view over part of
  them, and only the rest are measured. The full listing marks what a tap
  reaches part of likewise and keeps what it reaches none of, at
  `visible=0%` (design D50): the fourth production-app run's full listings
  marked the keyboard's share alone. It keeps there too an element with a
  ref a view drawn over it covers whole, which the interactive listing
  leaves out, with what it holds, but in a scroll container the view
  covers whole, whose hints none count (design D57): the sixth production-app
  run's full listing left out two controls the creation tab's footer held
  while the interactive hint counted them. The top-most modal, a modal window or
  an alert or dialog node, cuts everything it covers. A view marked modal
  (the `modal` node flag) cuts what its siblings hold within its bounds
  (task 10.22): assistive technology, and XCTest's tree, leave out every
  sibling, but a sibling outside the view is in sight and a tap reaches it,
  as Bluesky's header is beside its typeahead, a sibling only because React
  Native flattens the views between them. Each node is judged by its own
  bounds, and acting on a covered one fails with `element_covered`, naming
  the view by its role and name, or `modal view` when it has neither. On
  Android a node drawn over the siblings it overlaps (the `on_top` node
  flag) cuts what they hold within its bounds when it shows something, a
  node on screen with a ref or a name in it or below it (task 10.28):
  Android reports no occlusion inside a window and lists a view group's
  children by where they lie, so Bluesky's typeahead and the Explore page
  under it, both with the same bounds, read as siblings with the typeahead
  first. A view drawn over others that shows nothing, such as the decoration
  Bluesky draws over a video, may be transparent and hides nothing, and only
  a node with an area is judged, since Android gives a node scrolled out of
  its list bounds clipped to nothing: those rows keep counting in their
  list's hint under the tab bar drawn over the list's bottom, which hides
  only what lies wholly under it.
- On iOS the order is what tells (rn-app-fixes, design D31): XCTest lists a
  view's subviews in the order UIKit draws them and reports no occlusion,
  so in a tree from XCTest's snapshot (a window rooted at an
  `XCUIElementType` class, from a backend that is not a fallback, since the
  translator flattens the tree into a list in another order) a node listed
  after a sibling it overlaps cuts what the sibling holds within its
  bounds, under the conditions of `on_top`: it shows something, only nodes
  with an area are judged, and the walk enters no node outside its bounds,
  so what a scroll container clips stays in the container's hint. The
  shapes verified are those of the production-app smoke test of
  2026-10-09, on an iOS 26 Simulator, as synthetic fixtures: a consent
  screen's footer over the heading scrolled inside its Accept and continue
  button, a create screen's floating button over a list's last row, a
  feed's sticky header over the post scrolled under it; and Bluesky's
  typeahead, listed after the Explore page it is drawn over, which its
  modal flag covers, the order alone covering the page where the
  typeahead's rows lie, since the typeahead holds the whole page as a host
  does (below). A node reported disabled counts as
  showing something, since UIKit hit-tests a disabled control as an enabled
  one and the touch goes nowhere (rn-app-fixes, design D38): in the
  production-app run a control mostly under a disabled button was tapped on the
  button. Bars are spared from the siblings listed after them, since XCTest
  lists a navigation bar before the content its controller draws it over,
  which runs under it (the SwiftUI fixture's List tab); instead a bar by its
  class (`XCUIElementTypeNavigationBar`, `XCUIElementTypeTabBar`,
  `XCUIElementTypeToolbar`) covers what its later siblings hold within its
  bounds, and a header the app pins across a scrolling sibling's top, a
  view listed before it that does not scroll, spans its width and reaches
  from its top or above to its top or below, short of its bottom, covers
  what that sibling holds within its bounds, whatever the order, the
  sibling covering none of it (D38): React Native lists its header before
  the list it is drawn over, and the rows the list scrolled up past its top
  lie in the header's frame, where the production-app run found a 44-point row
  at 2 points still listed. A tab bar listed after a list covers the rows
  wholly under it, as on Android. React Native's accessibility lists a view
  that draws something, as a footer's background, as a node that holds
  nothing, what the view lays out listed after it as its siblings, top to
  bottom, and lists no node for a view that draws nothing (rn-app-fixes,
  design D48): a node that holds nothing and names nothing, without a ref
  or an identifier, listed after a sibling that holds or is a scroll
  container it lies over part of, covers what it lies over, as a later
  sibling that shows something does, and one that holds all of the
  container, as a background listed after a list would, covers none of it.
  On the fourth production-app run's creation tab the scroll view runs on to
  the bottom of the screen under the footer, whose view held nothing, and
  a tap on a control's strip under it opened nothing (`create-rn-ios`). An alert's static text is spared too
  (design D26 of physical-ios): UIKit's alert controller
  draws nothing over its title and message, while XCTest lists the scroll
  indicators of the scroll view that holds them after them, their
  accessibility frames 30-point hit areas along the view's edges, which
  UIKit draws as bars a few points thick, the horizontal one wholly over
  the message on an iPhone 17 Simulator and on the SE (`alert-message-ios`,
  `alert-message-se`); XCTest lists each indicator twice with one frame,
  and for the same reason an indicator (`_UIScrollViewScrollIndicator`)
  covers nothing but that copy: over part of the alert's OK button its
  frame left a tap 61% of it, which the interactive listing would mark, as
  it would every row along a list's edge (rn-app-fixes task 4.3). A node
  listed after a sibling that its frame holds whole, that shows nothing
  itself and is no bar, is a host (real-app-fixes, design D2): it cuts what
  the sibling holds within the bounds of what shows in it, not within its
  frame, each bar by its frame, each scroll container by its children that
  show something, its cells, and each node that shows something itself by
  its bounds, a scroll indicator and a node that shows nothing drawing
  nothing, and a refusal or a hint names the part that lies over the
  element. XCTest gives such a node its whole frame while it draws only what
  it holds: spike S14 found, on the Simulator's iOS 26.5 runtime, the
  container iOS 26 keeps a navigation controller's floating bottom toolbar
  in spanning the window, listed after the content, while it draws only the
  toolbar, 86 points at the screen's bottom, which hid every row of a
  system app's list, and a sheet's list, in a view of the sheet's content
  frame listed after the search bar's view, with that frame while its first
  cell starts 73 points lower, under the search bar, which hid the search
  field (`floating-toolbar-ios`, `search-sheet-ios`). A node listed after a
  sibling it overlaps only in part, a footer, a header or a floating button
  laid out over one edge of what it is drawn over, covers its frame, as the
  archived shapes need: the consent screen's footer, the feed's header listed
  after its list, the creation tab's footer, whose band above its tab bar
  covers the strip of the controls under it, the controls behind a header,
  and the edit menu's window as the reader bounds it. The risk is 10.28's,
  in two forms nothing in the tree tells apart: such a node transparent
  where it overlaps its sibling hides that part as if drawn, and a host
  whose background is drawn, which the tree does not show, leaves listed
  what that background hides. The text effects window, listed after the
  application's window, covers what lies under iOS's edit menu as such a
  later sibling, its node bounded by the menu's list (design D55): with the
  window's frame across the screen the sixth production-app run's listing
  held the menu's cells alone. A window across the screen is a host, and
  covers what its menu draws.
  A view that covers part of an element leaves it listed, and a tap on it
  goes to the part a tap reaches (`visibility`, below). On either platform,
  acting on what a view drawn over it covers, all of it or so much that no
  part a tap reaches is left, names the cover as the hint counting the
  element does, by one rule (`View::cover_name`, design D57): the `header`
  or the `footer` of the scroll container the element counts in, or of
  the window for one outside any, by the hints' rule below, else the node a
  tap at the element's centre would land on, the innermost one of the view
  that holds that point and shows something itself, else the view, by its
  role and name, as an `overlay` when it is a plain container or as a
  `disabled control` when it is a disabled one, else a bar there, else the
  `keyboard` when its window holds the point, and advises bringing the
  element out from under it rather than acting on the cover: the smoke
  test's tap on that heading accepted the consent, and the sixth production-app
  run's click on a control the creation tab's footer held was refused
  naming the `overlay` while the hint said `under the footer`.
- Scroll hints come from actionable nodes outside a container's viewport when
  the read has them, and from the container's scroll semantics otherwise
  (Compose reports only the nodes in the viewport): a range with a position
  counts the items beyond it (`(184 more items below, ...)`), and a range in
  other units or none at all gives the ways the container can scroll
  (`(more items below, ...)`); back reads as above or left, as in a
  left-to-right layout. Content above or left is hinted before the
  container's children, below or right after them. The interactive listing
  also counts there what it leaves out for a cover: an element a view drawn
  over it covers whole or one a tap reaches none of, both of which the
  full listing keeps at `visible=0%`, inside the container's bounds on the
  side of the container's middle it lies on along the axis the container
  scrolls (rn-app-fixes, design D45), and only one on the screen, which a
  listing could show; so its counts of covered elements are the full
  listing's `visible=0%` elements, by construction, which a test checks on
  every fixture (designs D54 and D57). The full listing's hints count only
  what it leaves out, elements off the screen, none of what it shows at
  `visible=0%` (design D57): the sixth production-app run's full listing
  repeated the interactive listing's count while it showed those elements.
  Beside what is scrolled away a hint names the cover of each element it
  counts so, as a refused action names it: the view over its centre, else,
  for one beyond what its container shows above or below, the container's
  header or footer there, a view across that edge listed before or after
  it; `behind the header` for a view listed before the container across
  its top, whatever it holds, or a plain view across its top short of its
  bottom, `under the footer` for a plain one across its bottom short of its
  top, a window over the application's own neither (design D55), `under
  the keyboard`, else what a tap at its centre would land on, as in `(6
  more actionable items above, 3 of them behind the header, scroll up to
  reveal)` (designs D50, D54 and D57): the fifth production-app run's header
  was a named view, and the rows its list scrolled above its frame, on the
  screen above the header, counted as scrolled away though the full
  listing showed them at `visible=0%`. Such an element outside any scroll
  container is counted on a line after the view over it, `(2 more
  actionable items behind the header)`. A modal or a view marked modal
  cuts what it covers with no count, no scroll bringing it out, and the
  keyboard keeps its own line.
- Empty listings read `(no interactive elements)` and `(empty screen)`.
- Refs are `eN`, parsed from `@eN`, `ref=eN` or `eN`; numbers only grow. A
  ref names one control: its element, and for an element without an
  identifier, also its accessible name, or else its role and place. An
  element keeps its ref while it survives, and without an identifier, while
  it keeps its name, or its role and its position among its parent's
  children, as a field whose label changes once it holds text does
  (rn-app-fixes, design D39); renamed and moved, it is another control,
  which the next snapshot gives a fresh ref, and the old ref is refused as
  stale from the moment the change is read. `removed_refs` lists the refs
  the previous snapshot listed and this one does not.

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
| `DEFAULT_STILL_POLL_INTERVAL` | 10 s | design D26 of physical-ios: the poll of a window without events on a target whose frames come slowly and that has a frame source, where frames that change call for their own re-read; a physical iPhone's 450 to 650 ms reads on the 1 s poll kept its worker reading when 47 to 60% of idle snapshots arrived, which waited up to 733 ms (`reports/11.1-ios-device.md`, finding 2) |
| `DEFAULT_MIN_REREAD_INTERVAL` | 200 ms | design D5: frame-driven re-reads bounded by a minimum interval, but for the window of a field text was just pasted or set into (task 10.29) |
| `DEFAULT_PROBE_INTERVAL` | 50 ms | task 10.11: four probes to a quiet window, so a settle without frames ends at most 50 ms after the quiet window that follows the last move |
| `DEFAULT_PROBE_ANCHORS` | 4 | task 10.11: nodes per probe, one round trip to the app each |
| `DEFAULT_OVERSCROLL` | 500 ms | task 10.14: on the API 36 emulator the Expo list pulled against its end drew its stretch for 333 to 383 ms after the finger lifted; 117 ms beyond the longest |
| `slow_frames` | none | design D23 of physical-ios: `SlowFrames { capture_interval, first_frame_wait, still_poll_interval }`, which the driver's attachment sets for frames that come slowly, a physical iPhone's polled screenshots, about 19 a second (a 52.6 ms interval) |
| `DEFAULT_FIRST_FRAME_WAIT` | 500 ms | design D23 of physical-ios: how long the re-read after input, and settle after it, hold for the input's first changed frame on such a target; spike S11 saw a tap's first changed frame 222 to 439 ms after its acknowledgement at p95 |
| `DEFAULT_MAX_DIMENSION` | 1000 px | spike S2's measured output size |
| `DEFAULT_QUALITY` | 80 | spikes S1 and S2 |
| `CANCEL_SLICE` | 50 ms | design D7 of mcp-server: the longest a wait blocks between two looks at its cancel flag, under the 100 ms within which a cancel ends a wait |

- `settle` ends once neither tree changes (events as they arrive, window
  changes noted to the mirror, deltas of re-reads) nor frame hash changes
  have happened for the quiet window, counted from the last change or
  injected input, and otherwise at the
  maximum with `settled: false` and the signals that kept changing. Frame
  callbacks without pixel changes do not count (spike S2). Without a frame
  source, windows without events are re-read every minimum re-read interval
  while waiting. `Signals::TreeOnly` ignores frames, for video.
- On a target whose frames come slowly (`slow_frames`, design D23 of
  physical-ios), the quiet window is at least two capture intervals,
  whatever shorter one the wait is given: a change can take a capture
  interval to show. After injected input the wait, when it watches frames,
  does not settle until the first frame that changed after the input came,
  or `first_frame_wait` from the input has passed, then waits its quiet
  window after the last change: a physical iPhone's polled screenshots
  showed a tap's first change after the default quiet window (spike S11:
  274 and 439 ms at p50 and p95 for a row that opens a page), so a settle a
  quiet window after the tap would end before the change. A tap that
  changes nothing settles once the wait is over. Without a frame source the
  setting changes nothing: the wait takes the probe-less re-read path of
  any target without frames.
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
  them leaves nothing. They are clipped too by what each of those
  containers shows, as `scroll_viewport` tells it, which loses the side of
  a view drawn across the container that the container does not fit in
  (rn-app-fixes, design D45): on the third production-app run a paste control's
  lower 10 points lay below a floating button but under a footer drawn
  over its list, a row whose frame ends at the footer's top holding the
  control, so that no view in the tree lay over them, and a tap there
  opened nothing. What a tap reaches, `reachable`, is the largest
  rectangle of the element's bounds inside them less every view drawn over
  part of it, at least 8 points on each side (`REACH_FLOOR`), or as long as
  the element on a side where it is shorter, and a tap on the element goes
  to its centre; `None` when no such rectangle is left (rn-app-fixes,
  design D37): in the production-app run of 2026-10-12 a tap at the centre of a
  card 70% under a sticky header pressed the header, and a tap on a heading
  half under a footer's button pressed the button. A view over part of the
  element and across it also cuts the visible bounds, on the side the
  element fits in with the least movement, which `scroll --until` aims a
  stroke for, of the sides its scroll containers can carry it to, as their
  semantics say: up or left when one scrolls forward, down or right when
  one scrolls back (design D46). On the third production-app run strokes aimed
  below a floating button pushed a list standing at its top, which moved
  nothing. `fraction` is the share of the element's bounds a tap
  reaches. Android reports bounds clipped to every ancestor, so an
  element its list cuts off reads as the part that shows (a row of 48
  points read as 1.2 points tall at the Expo list's edge on the API 36
  emulator), while iOS reports whole frames: an element whose bounds end,
  within a pixel, at an edge of its scroll containers' bounds where one of
  them can scroll further, as its scroll semantics say, is `cut` there,
  and how much of it lies beyond is unknown. A container without scroll
  semantics cannot tell, and cuts nothing.
- `covered_by` tells, the same way, what covers an element: the top-most
  modal, a view marked modal or a view drawn over all of it, or what a tap
  at its centre would land on when the views over part of it leave nothing
  a tap reaches, named as an action on its ref names it when it fails with
  `element_covered`. The keyboard is named `keyboard` (`KEYBOARD`); the
  daemon reads the windows again before it refuses a tap for it, since the
  keyboard moves a layout without an event. A covered element has no part
  a tap reaches.
- `hit` tells, the same way, what a tap at a point lands on (action-verdicts,
  design D1): the top-most element with a ref that a tap reaches there, as
  `visibility` judges what a tap reaches of an element, inside its visible
  bounds before the views drawn over part of it and under none of them, the
  last of the top-most window that holds one, the innermost or the one
  drawn over the others, or the keyboard where it draws, with the ref the
  mirror issued it while that ref names it; `holds` tells whether one
  element is another or holds it. The daemon names in a click's reply what
  its tap landed on, and warns when that is neither the element the click
  named, nor part of it, nor what holds it.
- `twin` tells, the same way, what stands for the element a stale ref
  named (action-verdicts, design D3): an app that rebuilds a control puts
  a new node in its place, which the replica cannot pair with the old one,
  so the ref is stale while the control is back. The twin is the one
  element bearing a ref, outside a keyboard, of the role and identifier
  the ref was issued for, which the ref table keeps beside its role, name,
  place and bounds in pixels as a listing last gave them, else the one of
  its role and name whose bounds lie, edge by edge, within a physical pixel
  of those, as the reach computation takes an edge within a pixel of
  another to end there; none when no element or more than one matches so.
  It carries a ref of its own, issued then if it had none; the stale ref
  stays stale. The daemon acts on the twin and names both refs.
- `scroll_viewport` tells, the same way, the visible bounds of the scroll
  container a stroke through a point moves (task 10.16): the innermost
  node on screen and not covered that scrolls along the stroke's axis, or
  along an axis its platform does not name, and holds the point, in the
  top-most window that has one, with the visible bounds an element inside
  it gets. The daemon bounds each stroke of `scroll --until` that looks
  for its element to three quarters of their extent along the axis, so
  that no element passes through them between two looks.
- `scroll_container_in` tells, the same way, what shows of the scroll
  container that shows the most of an area (rn-app-fixes, design D41):
  among the nodes on screen and not covered that scroll along the axis, in
  the top-most window that has one, the one whose visible bounds share the
  most of it. With the keyboard up the daemon strokes a scroll only through
  that part of what `uncovered` leaves, below a pinned header and above the
  keyboard: in the production-app run of 2026-10-12 `scroll up 300` came down on
  the header at 119.5 points and moved nothing.
- `field_within` tells the one field a node holds, with a ref for it, when
  it holds exactly one (D39): `fill` and `type` given an identifier on a
  node that takes no text enter the text into that field. `fields_within`
  tells how many it holds: on one that holds none and has no ref, `fill`
  and `type` tap it and enter the text where the keyboard that comes up
  types (design D44).
- `uncovered` tells, the same way, what of an area a stroke may go
  through (task 10.32): the area less the system and input-method windows
  above the window of an element, or for the screen above the window in
  front (the active window that is not a keyboard's, which Android flags
  active while a finger touches it, else the top-most application
  window), each cutting it as a bar cuts visible bounds, and whether a
  keyboard cut it. A stroke that comes down on one of those windows goes
  to it, and a keyboard takes it for typing (Gboard glide-typed a word
  into the focused field at every stroke of a scroll on a Nexus 5X), so
  the daemon strokes a scroll through what `uncovered` leaves.
- `keys_hidden` tells, the same way, whether a keyboard window above the
  window in front draws nothing (rn-app-fixes, design D52): iOS keeps the
  keyboard's window while an input has the focus and a hardware keyboard
  serves it, its keys below the screen, and the input takes keys at once,
  so the daemon's keyboard waits take such a window for the keyboard, with
  the field's focus for a field.
- `edit_menu` tells where iOS's edit menu is while one is up: the bounds of
  the list it draws its cells in (`_UIEditMenuListView`), on the screen
  (rn-app-fixes, design D55). A tap on the field in focus opens the menu
  and a paste that lands nothing leaves it up, so the daemon's `fill`
  closes it after its paste's read-back.
- `wait` evaluates after every item the target reports and every re-read,
  sleeping on the event stream in between; selectors match text in the name
  or value, or the whole name or value when `exact`, the identifier, the
  role and a ref, among elements on screen and not covered, or, for
  `Attached` and `Detached`, among every element the replica holds, in view
  or not, a keyboard's keys aside, as a query looks for one (rn-app-fixes,
  design D51), the first on screen and not covered given before the rest, as
  an action's selector resolves through it (design D53). A timeout fails
  with `wait_timeout` and a summary of an interactive snapshot. Its first
  look takes in what arrived and leaves a read that is due for later while
  the replica is current by the mirror's own rules: no read of every window
  due (before the first read, after a resync or a window change), no window
  an event named or input went to since its read. A condition that holds
  then answers at once, without the poll or the re-read after frames or text
  that is due, which the next pump takes (rn-app-fixes, design D36): in the
  production-app smoke test a wait paid 43 to 473 ms for that read, 380 ms at the median.
  Otherwise, and once it does not hold, the wait reads first as before; it
  never answers from a replica that has not been read, where nothing would
  match and every disappearance would hold, nor from the screen before a
  tap.
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
| `element_covered` | no | a modal, a view drawn over all of it, or views, bars or the keyboard over so much of it that less than 8 points a side are left inside what its scroll containers show, cover the ref's element; the message names what a tap at its centre would land on |
| `element_offscreen` | no | the ref's element is out of view: none of it shows, or too little for a tap with nothing named over its centre |
| `wait_timeout` | yes | the condition did not hold in time; the error carries the screen |
| `no_frame` | yes | no frame source, or no frame yet |
| `disconnected` | no | the target is gone |
| `cancelled` | yes | the cancel flag was raised while the mirror waited (`cancel_on`) |

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
| `grid-rn` | Expo Big tab on the API 36 emulator, 1,049 nodes as spike S1 counted them: the status bar's 28 and the app's 1,021, its 500 cells in a 20-column grid, each a clickable view group holding a text view with its number, all on screen |
| `login-ios` | SwiftUI login read by the Simulator's in-guest reader |
| `alert` | the SwiftUI login under its in-app alert, with the covered screen still in the read |
| `alert-message-ios` | the SwiftUI System tab under its in-app alert, read by the Simulator's in-guest reader on an iPhone 17 (physical-ios task 9.5): the title and message in a scroll view followed by its scroll indicators, each listed twice, their 30-point frames along the view's edges, the horizontal one over the message |
| `alert-message-se` | the same alert as the runner read it on an iPhone SE (3rd generation), the alert's view an `XCUIElementTypeAlert`, where the message fits and each indicator reads 1 page; the glass views and the app window's wrappers trimmed |
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
| `bluesky-typeahead` | Bluesky's search screen with the keyboard up and its typeahead flagged `modal` over the Explore page, the header a sibling outside it (names replaced) |
| `bluesky-typeahead-android` | the same screen on Android: the typeahead an `AbsListView` flagged `on_top` over the Explore page, both with the same bounds, the header and the tab bar flagged too, rows scrolled out of the page clipped to nothing (names replaced) |
| `consent-ios` | a React Native consent screen on iOS in XCTest's classes, no flags (rn-app-fixes, design D31): a footer listed after the scroll view, holding Accept and continue over the heading the content is scrolled to (placeholder names) |
| `create-ios` | a React Native create screen on iOS: a floating button listed after the list, over its last row and partly over the row before (placeholder names) |
| `feed-ios` | a React Native feed on iOS: a sticky header listed after the list, over the post scrolled under it, the first post above the screen (placeholder names) |
| `tabbar-ios` | a React Native tab bar on iOS (rn-app-fixes, design D35 and D40): three tabs, labelled `Other`s, the middle one selected and also showing its label, a text whose only child is its own text, the other two holding their icon's glyph likewise (placeholder names) |
| `members-ios` | a React Native members screen on iOS: checkboxes React Native announces only in their value (`checkbox, checked`), a heading and a text each repeated by their only child, and the list's two scroll bars likewise (placeholder names) |
| `consent-footer-ios` | a React Native consent screen on iOS in the structure of the production-app run of 2026-10-12 (rn-app-fixes, design D37): two footer buttons listed after the scroll view, a 22-point heading with its lower half under the secondary one, a 12-point text with its upper 9 points under the primary one (placeholder names) |
| `create-disabled-ios` | a React Native creation tab on iOS (D37, D38, D45, D46): its disabled 56-point primary button listed after the scroll view, over 38 of the 48 points of the paste control and the button beside it, and a footer listed after it, from the button's bottom down, over the list's lower part, holding a tab bar whose first tab holds its icon's glyph; the two controls sit in a row whose frame ends at the footer's top, their lower 10 points under the footer (placeholder names) |
| `create-rn-ios` | a React Native creation tab on iOS (D48, D50), the raw read of a throwaway screen built to the fourth production-app run's boxes of the tab, which it gives to the point: a scroll view, an `XCUIElementTypeScrollView` inside `RCTScrollViewComponentView`, running on under the footer to the bottom of the screen, a disabled primary button over 38 of the 48 points of the paste control and the button beside it, and the footer's view, which holds nothing, its tabs listed after it, over their lower 10 points (placeholder names) |
| `feed-sticky-ios` | a React Native feed on iOS (D37, D45): the sticky header listed before the list, a card 70% above the list's top in the header's frame, a card above it whose lower 27.67 points lie on screen above the header, twelve cards above the screen (placeholder names) |
| `header-top-ios` | a React Native list on iOS with the keyboard up (D38, D41): a plain header listed before the list, its frame down to the list's top, a 44-point row at 2 points in it, a field with the focus and a button partly under the keyboard (placeholder names) |
| `form-keyboard-ios` | a React Native sign-in form on iOS with the keyboard up (D37, D38): the submit button 3.67 of its 48 points above the keyboard's top, two links under the keyboard (placeholder names) |
| `sheet-ios` | a React Native paste sheet on iOS (D39): a title field without an identifier in an unidentified view among four of one class, which gains a counter at the focus, the field's name changing with its text (placeholder names) |
| `keyboard-hardware-ios` | the SwiftUI fixture's Login tab with its username field in focus while the Simulator's hardware keyboard is connected, read by the in-guest reader on an iPhone 17 (D52): iOS draws no keys, the keyboard's host view lies below the screen, keys trimmed to four, and a wrapper of the prediction bar reaches 600 points above it onto the screen; the window's bounds as the reader gives them, none |
| `header-named-ios` | a React Native organizations list on iOS (D54): a header the app names listed before the list, three rows the list scrolled above its frame lying on the screen above the header, three above the screen (placeholder names) |
| `edit-menu-ios` | the SwiftUI fixture's Login tab with iOS's edit menu up over its username field, which holds "agent", the keyboard up, read by the in-guest reader on an iPhone 17 (D55): the menu (Paste, Select, Select All, AutoFill) in the text effects window, the application's second window, whose node the reader bounds by what it draws, the menu's list, 121 to 165 points down, over the password field; keys trimmed to four |
| `floating-toolbar-ios` | a UIKit list screen on iOS 26 in the shape spike S14 recorded on the Simulator (real-app-fixes, design D2): a layout container listing a navigation bar, a transition view holding a collection view whose rows, 370 by 52 points, lie on screen from 293.33 points down, and a plain container spanning the window, listed after the transition view, holding a plain host of the same frame that holds the floating bottom toolbar, 86 points at the screen's bottom, with its search field (placeholder names) |
| `search-sheet-ios` | a UIKit map screen on iOS 26 with its sheet at half height, in the shape spike S14 recorded (D2): the map and the overlay holding its buttons in one transition view, the sheet in a second one listed after it, both spanning the window; the sheet's content view lists the search bar's view, 73 points tall, then a plain view holding a named list of the content view's frame whose first cell starts 73 points lower, under the search bar's band (placeholder names) |
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
  and design D5's frame-driven re-reads catch later changes. After text
  that goes in in one piece, pasted on a Simulator or set directly on
  Android, the field's window does not wait out the minimum interval: on
  the iOS 26.5 Simulator a paste showed 43 to 93 ms after Command-V, never
  in the read that followed it, so that window is read again as soon as a
  frame changes, until a read shows the field's text changed, the next
  input or a poll interval after the text (task 10.29). A `fill` then took
  a paste of 36 characters 69 to 124 ms after the entry, against 248 to
  295 ms with the interval. Text typed as keys keeps the interval: `fill` holds it 200 ms,
  which a read at its first frame cannot end, and that read would put the
  next one 200 ms after it. Every other window and input keeps it too,
  where an animation would otherwise make reads back to back. On a target
  whose frames come slowly, as a physical iPhone's screenshots, the re-read
  after input waits for the input's first changed frame, at most 500 ms
  (design D23 of physical-ios): read at once, it would show the screen
  before the change, and the frame-driven re-read that catches the change
  comes a minimum interval after a read that costs about half a second
  there. The spec's target whose frames are a change signal and whose
  reads cost more than a frame interval is one the driver sets
  `slow_frames` for and that has a frame source: there the poll waits
  `still_poll_interval`, 10 s, since frames that change call for a re-read
  of their own (design D26 of physical-ios), so a read happens once the
  frames changed since the read before, or once the bound has passed.
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
  read of Ignite's sign-in screen (`reports/10.4-rn-smoke.md`) shows each field as such a node with
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
  sibling in the next read, and siblings of a class and identifier that
  neither their content nor an identified descendant tells apart by their
  position among their parent's children, so an element can survive a
  screen transition as another control: Bluesky's Forgot password? became
  the forgot-password form's Next, and its ref with it (second run, finding
  6). A ref's identity therefore includes the accessible name of an
  element without an identifier, or else its role and that position: a
  control renamed in place, such as a field whose label changes once it
  holds text (rn-app-fixes, design D39) or a counter's label, keeps its
  ref, while Next, one position before where Forgot password? sat, gets a
  fresh one; an identified element keeps its ref under any name. Element
  identities, and with them deltas and structural diffs, still follow the
  element.
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
| | Idle event-less window with still frames (physical-ios task 9.1, design D26) | `replica::with_slow_frames_a_still_screen_is_polled_once_every_still_poll_interval`, `replica::with_slow_frames_a_frame_change_since_the_last_read_brings_the_poll_back`, `replica::without_slow_frames_or_without_a_frame_source_the_poll_stays_at_one_second`; through the daemon's fake iPhone, `worker::tests::an_idle_fake_iphone_with_still_frames_is_not_polled_where_others_are` and `worker::tests::a_fake_iphones_windows_are_re_read_on_frame_changes_and_polled_when_still` |
| Snapshot content | Interactive snapshot of a scrolling list | `interactive::an_interactive_list_snapshot_lists_visible_rows_and_hints_three_below`, `interactive::scroll_semantics_give_the_hint_when_offscreen_rows_are_absent`, `interactive::scroll_semantics_without_an_item_position_say_which_ways_more_lies`, `interactive::a_modal_window_cuts_the_application_windows_below_it`, `interactive::an_actionable_container_takes_its_name_from_non_actionable_descendants`, `interactive::every_interactive_line_carries_a_ref`, `interactive::the_full_listing_follows_agent_browser_lines`, `interactive::an_empty_listing_says_so_as_agent_browser_does`, `interactive::an_alert_cuts_the_content_it_covers`, `interactive::a_system_alert_reports_system_ui_as_the_foreground`, `interactive::the_keyboard_window_is_listed_as_one_line` |
| | (task 7.11: fields in labelled containers) | `editables::a_field_in_a_labelled_container_takes_the_containers_label_on_android`, `editables::a_field_in_a_labelled_container_takes_the_containers_label_on_ios`, `editables::a_container_names_only_its_single_unlabelled_field`, `editables::a_labelled_container_whose_field_the_platform_hides_gets_a_ref`, `editables::only_labelled_containers_with_nothing_but_static_text_below_them_get_such_refs`, `editables::fields_below_the_fold_are_hinted_once_each`, `editables::a_named_field_under_an_alert_is_covered_and_its_ref_says_which_field`, `editables::fields_in_identically_labelled_containers_collide` |
| | (task 7.12: labelled controls without traits) | `controls::labelled_controls_without_traits_on_the_hosting_provider_sheet_get_refs`, `controls::language_rows_get_refs_and_those_below_the_fold_are_hinted`, `controls::a_labelled_control_under_an_alert_is_covered`, `controls::inside_a_clickable_ancestor_such_controls_name_it_and_get_no_ref`, `controls::identical_labelled_controls_collide`, `controls::nodes_with_more_than_static_text_below_them_or_no_label_are_no_such_controls`, `editables::only_labelled_containers_with_nothing_but_static_text_below_them_get_such_refs` |
| | Heading under a fixed footer on iOS | `interactive::a_footer_drawn_after_a_heading_on_ios_hides_it`, `interactive::acting_on_what_a_view_drawn_after_it_covers_on_ios_fails_naming_the_view`, `visibility::what_a_view_drawn_after_it_covers_on_ios_is_covered_not_out_of_view` |
| | Card under a sticky header | `interactive::a_sticky_header_drawn_after_a_list_on_ios_hides_the_post_under_it`, `visibility::what_a_view_drawn_after_it_covers_on_ios_is_covered_not_out_of_view` |
| | (rn-app-fixes 1.1: a floating button over a list's last row; bars spared, a tab bar covering what lies wholly under it; overlapping siblings in any order) | `interactive::a_floating_button_drawn_after_a_list_on_ios_hides_its_last_row`, `interactive::acting_on_what_a_view_drawn_after_it_covers_on_ios_fails_naming_the_view`, `visibility::bars_over_a_list_cut_its_visible_bounds_and_a_whole_frame_shows_its_share`, `visibility::the_bars_over_a_list_are_left_out_of_its_visible_bounds`, and the unit tests `view::tests::*` |
| | (physical-ios task 9.5, design D26: an alert's message under its scroll indicators' frames) | `interactive::an_alerts_message_is_listed_once_under_it_on_the_simulator_and_the_phone` |
| | Selected tab keeps its ref | `controls::a_tab_gets_a_ref_whether_selected_or_not_and_the_selected_one_says_so`, `controls::a_row_without_a_selected_node_lists_no_tabs` (rn-app-fixes 3.4, design D40) |
| | Control under a disabled button | `interactive::a_disabled_view_drawn_after_a_control_on_ios_hides_it_whatever_it_shows`; partly under it, `interactive::a_tap_on_a_control_under_a_disabled_button_lands_on_the_part_below_it` (rn-app-fixes 3.1, design D38), on the tab as the second production-app run read it, its list ending above the tab bar |
| | Control behind a header listed first | `interactive::a_header_listed_before_its_list_on_ios_hides_the_rows_scrolled_up_behind_it`, `interactive::a_navigation_bar_listed_before_the_content_on_ios_hides_what_lies_in_it`, `interactive::acting_on_a_row_behind_a_header_listed_before_its_list_fails_naming_the_header` (D38) |
| | Elements under the keyboard | `interactive::what_lies_under_the_keyboard_is_left_out_and_what_lies_partly_under_it_says_how_much_shows`, `interactive::a_tap_on_a_button_partly_under_the_keyboard_lands_on_what_shows_or_is_refused_naming_it` (rn-app-fixes 3.2, design D38) |
| | Ref across a label change | `controls::a_field_keeps_its_ref_when_its_name_and_its_view_change_in_place`, `controls::a_control_named_by_its_text_keeps_its_ref_when_the_text_changes_in_place_on_android`; the fill by that ref in the daemon, `handlers::act::tests::a_fill_by_ref_of_a_field_whose_label_changes_reads_it_back` (rn-app-fixes 3.3, design D39) |
| | (rn-app-fixes 1.5: a checkbox with its state; a text, a scroll bar and a heading listed once) | `controls::a_checkbox_react_native_announces_in_its_value_lists_as_a_checkbox_with_its_state`, `interactive::a_text_a_scroll_bar_and_a_heading_that_repeat_themselves_are_listed_once`, and the unit test `roles::tests::react_native_s_checkbox_value_says_whether_it_is_checked` |
| Stable refs | Element survives between snapshots | `refs::a_surviving_button_keeps_its_ref_when_the_screen_changes_elsewhere`, `refs::typing_into_a_field_keeps_its_ref`, `refs::refs_only_increase_and_are_never_reused`, `refs::identical_siblings_are_reported_as_a_degraded_sibling_collision` |
| | Stale ref is refused (and its twin, action-verdicts design D3: `refs::a_stale_ref_s_twin_is_the_element_of_its_role_and_identifier_else_of_its_name_and_bounds`, `refs::a_twin_without_an_identifier_lies_within_a_pixel_of_where_the_ref_s_element_was`, `refs::no_twin_stands_for_a_stale_ref_that_two_elements_match_alike`) | `refs::acting_on_a_removed_element_fails_with_a_stale_ref_error_and_guidance`, `refs::a_stale_ref_never_resolves_to_another_element`, `interactive::a_stale_ref_names_the_element_it_was_issued_for`, `refs::a_ref_that_was_never_issued_is_unknown`, `refs::refs_parse_in_every_form_agent_browser_accepts`, `interactive::acting_on_an_element_an_alert_covers_fails_with_its_code`, `interactive::acting_on_a_row_scrolled_out_of_view_fails_with_its_code` |
| | (task 7.12: a ref names one control) | `controls::a_stale_ref_to_a_control_a_transition_renamed_is_refused_on_ios`, `controls::the_control_that_took_its_place_gets_a_fresh_ref_and_the_old_one_stays_stale`, `controls::a_wait_on_the_old_ref_sees_the_renamed_control_gone`, `controls::an_identified_control_keeps_its_ref_when_its_name_changes`, `controls::typing_into_a_field_without_an_identifier_keeps_its_ref` |
| Single coordinate contract | Tapping reported bounds hits the element | `refs::tapping_the_center_of_reported_bounds_hits_the_element`, `replica::bounds_are_in_device_points_with_the_scale_factor_beside_them`, `refs::text_into_a_ref_names_the_node_of_the_latest_read` |
| | Card partly under a sticky header | `interactive::a_tap_on_a_card_partly_under_a_sticky_header_lands_on_the_part_in_view`, `visibility::visibility_gives_the_part_of_an_element_a_tap_reaches_beside_what_covers_it`; the click's warning in the daemon, `handlers::act::tests::a_click_on_a_card_partly_above_its_list_taps_the_part_in_view_with_a_warning` (rn-app-fixes 3.1, design D37) |
| | Heading half under a footer button | `interactive::a_tap_on_a_heading_half_under_a_footer_button_lands_on_the_half_that_shows`; by ref, identifier and text selector in the daemon, `handlers::act::tests::clicks_on_what_footer_buttons_cover_in_part_tap_what_shows_or_are_refused_naming_the_button`, and `scroll --until` judging arrival by the same part, `handlers::act::tests::scroll_until_does_not_call_a_control_mostly_under_a_disabled_button_visible` (D37) |
| | Submit button moved by the keyboard | in the daemon, which reads the windows again (`note_window_change`, `pump`) before it taps an element none of which `visibility` finds in view: `handlers::act::tests::a_click_on_a_button_the_keyboard_moved_taps_it_where_it_is_now`, `handlers::act::tests::a_click_on_a_button_the_keyboard_still_covers_fails_naming_it_and_taps_nothing`, and the rest of its row in `crates/daemon/README.md` |
| | Strip clipped by the scroll view | `visibility::what_a_tap_reaches_is_clipped_by_what_its_scroll_view_shows`, `interactive::a_control_whose_strip_below_a_floating_button_lies_under_a_footer_is_left_out_and_refused`; the click in the daemon, `handlers::act::tests::a_click_on_a_control_whose_strip_below_a_disabled_button_lies_under_a_footer_is_refused` (rn-app-fixes 4.3, design D45) |
| | Partly covered elements marked | `interactive::partly_covered_elements_say_how_much_shows_and_those_a_tap_cannot_reach_are_counted`, `interactive::a_floating_button_drawn_after_a_list_on_ios_hides_its_last_row` (D45) |
| | (rn-app-fixes 4.4, design D46: the side a stroke carries an element to; a scroll that stops names its last stroke) | `visibility::a_view_across_an_element_keeps_the_side_its_list_can_carry_it_to`; in the daemon, `handlers::act::tests::scroll_until_on_the_creation_tab_strokes_the_way_a_plain_scroll_moves_the_list`, `handlers::act::tests::scroll_until_strokes_toward_a_side_the_list_can_carry_the_control_to`, `handlers::act::tests::a_scroll_until_that_stops_names_its_last_stroke_and_what_moved` |
| Settle predicate | Screen settles | `settle::a_transition_settles_shortly_after_it_ends_and_reports_the_elapsed_time`, `settle::the_quiet_window_defaults_to_200_ms_and_the_maximum_to_3_s`, `settle::frame_callbacks_without_pixel_changes_do_not_hold_the_wait`, `settle::a_screen_that_has_been_quiet_settles_at_once`, `settle::the_default_window_does_not_end_inside_a_scroll_while_150_ms_would`, `settle::without_frames_an_event_less_window_is_re_read_while_waiting`, `settle::a_launch_noted_to_the_mirror_holds_settle_until_the_app_has_drawn`, `settle::a_launch_counts_as_a_change_whether_noted_or_reported_as_a_window_event` |
| Windows without change events; Frames and screenshots from the host, spec `ios-device-driver` (physical-ios task 5.2, design D23) | (a target whose frames come slowly: the re-read after input waits for the first changed frame; settle's floor of two captures and its hold after input; without frames, the probe-less path) | `replica::with_slow_frames_a_re_read_after_input_waits_for_the_first_changed_frame`, `replica::with_slow_frames_a_re_read_after_input_that_changes_no_frame_waits_out_the_wait`, `replica::with_slow_frames_but_no_frame_source_a_re_read_after_input_reads_at_once`, `settle::with_slow_frames_settle_waits_at_least_two_capture_intervals_without_a_change`, `settle::with_slow_frames_settle_after_input_holds_for_its_first_changed_frame`, `settle::without_frames_a_slow_frame_target_settles_on_rereads_every_minimum_interval`; through the daemon's fake iPhone, its `worker::tests` and `fake::realtime::tests::a_fake_iphones_frames_come_at_the_devices_rate` |
| | Continuous animation | `settle::a_continuous_animation_is_not_settled_after_the_maximum_and_names_frame_changes`, `settle::an_animation_that_also_sends_tree_events_names_both_signals`, `settle::tree_only_settling_ignores_frames` |
| Wait predicates from the event stream | Element appears | `waits::a_wait_for_text_resolves_within_one_processing_cycle_of_its_appearance`, `waits::a_disappear_wait_resolves_when_the_element_leaves`, `waits::an_enabled_wait_resolves_when_the_element_becomes_enabled`, `waits::a_wait_on_a_window_without_change_events_resolves_from_its_re_reads`, `waits::a_wait_that_already_holds_returns_at_once`, `waits::selectors_match_text_identifier_role_and_ref_on_screen_only` |
| | Wait times out | `waits::a_wait_that_never_matches_fails_at_the_timeout_with_the_last_snapshot_summary` |
| | Condition already holds | `waits::a_wait_that_holds_in_the_replica_answers_before_a_poll_that_is_due`, `waits::a_wait_does_not_answer_from_a_replica_behind_a_tap_or_an_event` |
| | React Native scroll view | `interactive::a_react_native_scroll_view_is_told_by_its_element_type`, `interactive::a_row_under_a_footer_react_native_lists_holding_nothing_is_left_out_and_refused`, `visibility::a_footer_react_native_lists_holding_nothing_cuts_what_its_scroll_view_shows`, `visibility::a_view_react_native_lists_holding_nothing_covers_only_part_of_a_scroll_view` (rn-app-fixes 5.1, design D48); in the daemon, `handlers::act::tests::a_click_on_a_control_whose_strip_lies_under_a_footer_react_native_lists_empty_is_refused` and `handlers::act::tests::scroll_until_strokes_inside_what_a_scroll_view_shows_above_a_footer` |
| | Hints agree with the listing | `interactive::the_hints_count_what_a_cover_leaves_nothing_of_and_name_the_cover`, `interactive::the_full_listing_marks_what_a_tap_reaches_part_or_none_of_and_keeps_it` (rn-app-fixes 5.3, design D50) |
| | Hardware keyboard connected | `interactive::a_keyboard_window_that_draws_nothing_covers_nothing_and_counts_as_no_keyboard`, and in the Simulator's reader `elements::tests::a_keyboard_window_whose_keys_lie_below_the_screen_covers_nothing`, `elements::tests::a_keyboard_window_under_a_hardware_keyboard_covers_the_strip_it_keeps_for_a_toolbar`; the waits in the daemon, `handlers::act::tests::a_fill_on_ios_with_a_hardware_keyboard_types_once_the_field_has_the_focus`, `handlers::act::tests::fill_by_the_identifier_of_a_node_that_brings_up_a_keyboard_drawing_nothing_types_at_once` (rn-app-fixes 6.1, design D52) |
| | Counts agree by construction | `interactive::hints_call_a_view_listed_before_the_list_across_its_top_the_header_and_count_what_the_listing_drops`, `interactive::the_card_above_the_feed_is_counted_behind_the_header`, `interactive::on_every_fixture_the_hints_count_what_the_interactive_listing_leaves_out` (rn-app-fixes 6.3, design D54); a control a view covers whole kept in the full listing at `visible=0%`, its hints counting only what it leaves out, `interactive::a_control_a_view_covers_whole_stays_in_the_full_listing_at_visible_0`, and a refusal naming a cover as the hint does, `interactive::a_refusal_names_a_cover_as_the_hint_counting_the_element_does` (rn-app-fixes 7.3, design D57) |
| | Edit menu up | `interactive::the_edit_menu_covers_the_strip_its_cells_draw_on_and_the_screen_stays_listed`, `interactive::a_control_wholly_under_the_edit_menu_is_left_out_and_refused_naming_the_menu`, `interactive::a_hint_never_calls_a_window_over_the_screen_a_header`, and in the Simulator's reader `elements::tests::the_edit_menus_window_covers_the_strip_its_cells_draw_on`, `elements::tests::a_system_window_that_draws_nothing_covers_nothing`; `fill` closing the menu in the daemon, `handlers::act::tests::a_fill_whose_paste_lands_nothing_dismisses_the_edit_menu_it_left_up` (rn-app-fixes 7.1, design D55) |
| In-view marking follows the screen | A collection view's rows | `interactive::a_container_spanning_the_window_that_draws_only_its_toolbar_hides_no_row_on_ios`; a sheet's search field above its list's inset, `interactive::a_list_whose_frame_runs_under_a_search_bar_hides_none_of_the_bar_on_ios`; a window across the screen as a host, `interactive::a_hint_never_calls_a_window_over_the_screen_a_header` (real-app-fixes 2.1, design D2) |
| | (real-app-fixes 5.6: a field drawn in the place the keyboard's host keeps for its input accessory view) | in the daemon, `handlers::act::tests::a_field_drawn_where_the_keyboard_keeps_its_accessory_view_s_place_is_listed_and_clicked`, on its fake's accessory board |
| Structural diff | Diff after a form submit | `diff::a_form_submit_diff_lists_removed_fields_and_added_confirmation`, `diff::a_diff_against_an_unchanged_screen_is_empty`, `diff::changed_elements_list_the_fields_that_changed`, and for agent-browser's delta snapshots `diff::delta_snapshots_follow_agent_browser_kinds_and_revisions`, `diff::delta_ref_changes_follow_agent_browser_ops`, `diff::a_delta_of_70_percent_or_more_of_the_listing_falls_back_to_full`, `diff::changing_the_listing_options_starts_again_with_the_whole_listing` |
| Quality verdict on every snapshot | Degraded read is flagged | `resync::a_root_only_read_of_a_populated_screen_is_degraded_with_advice`, `resync::truncated_reads_are_degraded_naming_the_limit`, `resync::agent_warnings_and_disconnection_reach_the_verdict`, `refs::identical_siblings_are_reported_as_a_degraded_sibling_collision` |
| Frame access | Scaled frame | `frames::a_scaled_frame_fits_the_requested_long_edge_upright_with_its_timestamp`, `frames::a_rotated_frame_is_encoded_upright`, `frames::small_frames_are_not_enlarged_and_defaults_apply`, `frames::quality_defaults_to_80_and_is_clamped`, `frames::a_frame_older_than_the_last_tree_read_says_so`, `frames::no_frame_is_an_error_with_its_code` |
| (design D7 of mcp-server, the cancel flag) | (a raised flag ends the waits; one that stays down changes nothing) | `cancel::a_raised_flag_ends_run_until_wait_and_settle_with_cancelled_before_they_block`, `cancel::a_flag_raised_while_a_wait_blocks_ends_it_within_a_slice`, `cancel::a_flag_that_stays_down_changes_nothing_a_wait_does`; the daemon's `cancel::*` end to end |
| Replica maintained from device events | (task 7.10: timing and draining for the daemon) | `timestamps::nothing_has_happened_before_the_first_read_and_the_first_read_gives_its_span`, `timestamps::the_last_frame_and_frame_change_come_from_the_frames_the_mirror_took_in`, `timestamps::only_the_latest_1024_frame_changes_are_kept`, `timestamps::the_last_tree_change_is_an_event_arrival_or_the_end_of_a_read_that_found_a_delta`, `timestamps::the_last_action_is_when_the_mirror_injected_input`, `timestamps::drain_takes_in_what_arrived_without_reading_the_tree` |
| Resynchronization | Backend fallback | `resync::a_backend_fallback_rebuilds_the_replica_stales_old_refs_and_names_the_fallback`, `resync::a_read_from_another_backend_rebuilds_the_replica_even_unannounced`, `resync::a_sequence_gap_rebuilds_the_replica_and_stales_earlier_refs`, `resync::an_out_of_sync_report_from_the_driver_also_rebuilds_the_replica`, `resync::events_a_read_already_reflects_are_skipped_but_a_gap_is_not`, `resync::a_reconnection_rebuilds_the_replica_and_restarts_sequence_numbers`, `resync::events_on_streams_this_version_does_not_know_are_ignored_even_with_gaps` |
| (what covers an element) | (named as `element_covered` names it; the part a tap reaches beside the covers) | `visibility::what_covers_an_element_is_named_as_an_action_on_its_ref_names_it`, `visibility::visibility_gives_the_part_of_an_element_a_tap_reaches_beside_what_covers_it`, and the unit tests `visibility::tests::*` |
| (task 10.15, how much of an element is in view) | (the share in view, the cut edge, the bars) | `visibility::a_row_inside_its_list_is_wholly_in_view_within_the_list_s_bounds`, `visibility::a_row_android_clips_at_the_bottom_edge_of_a_list_that_scrolls_on_is_cut_there`, `visibility::a_row_android_clips_at_the_top_edge_of_a_list_that_scrolls_back_is_cut_there`, `visibility::a_row_at_the_edge_of_a_list_that_cannot_scroll_that_way_is_whole`, `visibility::without_scroll_semantics_an_edge_is_no_cut`, `visibility::bars_over_a_list_cut_its_visible_bounds_and_a_whole_frame_shows_its_share`, `visibility::system_bars_and_the_keyboard_above_the_app_cut_its_visible_bounds`, `visibility::an_element_outside_any_scroll_container_is_measured_against_its_window`, and the unit tests `visibility::tests::*` |
| (task 10.16, the scroll container a stroke moves) | (its visible bounds, the bars, the axis) | `visibility::a_stroke_through_a_list_moves_the_list_and_its_visible_bounds_are_its_own`, `visibility::the_bars_over_a_list_are_left_out_of_its_visible_bounds`, `visibility::a_stroke_moves_the_innermost_container_that_scrolls_along_its_axis`, `visibility::a_point_outside_every_scroll_container_has_none` |
| (task 10.21, the keyboard over an iOS app) | (only what its input views cover is cut) | `visibility::the_ios_keyboard_cuts_from_its_prediction_bar_down`, and in the Simulator's reader `elements::tests::the_keyboard_window_covers_the_input_views_it_holds_not_the_screen`, `elements::tests::a_keyboard_sliding_in_covers_only_its_part_on_screen`, `elements::tests::a_keyboard_window_that_shows_nothing_covers_nothing` |
| (task 10.22, a view marked modal inside its window) | (what it covers is cut, what lies outside it stays) | `interactive::a_view_marked_modal_hides_what_it_covers_and_keeps_what_lies_outside_it`, `interactive::acting_on_an_element_a_view_marked_modal_covers_fails_with_its_code`, and in the Simulator's reader `elements::tests::a_read_honouring_modal_views_that_left_children_out_was_pruned`, `elements::tests::the_view_a_pruned_read_kept_is_marked_modal_in_the_whole_read`, `elements::tests::a_kept_view_that_moved_between_the_reads_is_found_by_what_it_holds`, `elements::tests::nothing_is_marked_when_the_reads_disagree` |
| (task 10.25, the keyboard over an Android app) | (a floating toolbar cuts no list it does not span) | `visibility::gboard_s_floating_toolbar_leaves_the_rows_beside_and_below_it_in_view`, and in the Android agent the checks of `agent-android/unit-test/KeyboardAreaTest.kt` |
| (task 10.28, a view drawn over its siblings on Android) | (what it covers is cut when it shows something; a node without an area is not judged) | `interactive::a_view_drawn_over_its_siblings_on_android_hides_what_it_covers`, `interactive::acting_on_an_element_a_view_drawn_over_it_covers_fails_with_its_code`, `interactive::a_bar_drawn_over_a_page_hides_what_lies_wholly_under_it_and_leaves_the_page_its_hint`, `interactive::a_view_drawn_over_its_siblings_that_shows_nothing_hides_nothing`, and in the Android agent the checks of `agent-android/unit-test/StackingTest.kt` |
| (task 10.32, where a stroke may come down) | (the area less the system and keyboard windows above its window, or above the window in front) | `visibility::a_stroke_goes_through_what_the_system_bars_and_the_keyboard_leave_of_its_area`, `visibility::the_screen_loses_what_the_windows_above_the_one_in_front_cover` |
