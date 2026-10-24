# agent-cli Specification

## Purpose
Gives agents and humans one command surface for devices whose grammar, ref syntax and JSON output follow agent-browser, so that skills and prompts written for the web tool transfer to devices unchanged where the concepts overlap.

## Requirements

### Requirement: Command grammar
The CLI SHALL provide at least these commands: `devices`, `open`, `close`, `snapshot` with an interactive flag, `click` by ref or coordinates, `fill`, `type`, `press`, `swipe`, `scroll`, `screenshot`, `wait` by selector or settle, `get text`, `diff snapshot`, `batch`, `status`, and `doctor`. Snapshot output SHALL list one element per line with role, name and ref.

#### Scenario: Interactive snapshot output
- **WHEN** a client runs an interactive snapshot on a screen with a heading and two buttons
- **THEN** the output lists a heading line and two button lines, each ending with a ref

### Requirement: Ref syntax
Elements SHALL be addressed as `@e` followed by a number as printed by snapshot. Acting on a stale ref SHALL fail with an error that says to re-snapshot.

#### Scenario: Stale ref
- **WHEN** a client clicks a ref from a snapshot taken before navigation
- **THEN** the command fails with a stale-ref error and guidance

### Requirement: Output formats
The CLI SHALL print human-readable output by default and structured JSON with a stable schema when the JSON flag is given. Errors SHALL go to standard error with a non-zero exit code and SHALL include a machine-readable code in JSON mode.

#### Scenario: JSON error
- **WHEN** a command fails in JSON mode
- **THEN** standard output carries a JSON object with an error code and message and the exit status is non-zero

### Requirement: Daemon lifecycle
The first command SHALL start the daemon when none is running. CLI process overhead, measured as time from process start to daemon request, SHALL stay under 20 ms. The daemon SHALL exit after a configurable idle period and `status` SHALL report the daemon and its targets.

#### Scenario: Cold first command
- **WHEN** no daemon is running and a client runs `devices`
- **THEN** a daemon starts, the command succeeds, and a second command reuses the daemon

### Requirement: Target selection
The CLI SHALL accept platform and device flags. With one connected target it SHALL be selected by default; with several and no flag the command SHALL fail listing the candidates.

#### Scenario: Ambiguous target
- **WHEN** two targets are connected and no device flag is given
- **THEN** the command fails and lists both targets with their identifiers

### Requirement: Sessions
A session flag SHALL isolate target claims and refs between concurrent clients. A target claimed by one session SHALL refuse commands from another session unless a force flag is given.

#### Scenario: Claimed target
- **WHEN** session A holds a target and session B sends a command to it without force
- **THEN** session B's command fails naming session A as the holder

### Requirement: Batch execution
The CLI SHALL run several commands from one invocation in order, SHALL support stopping at the first error, and SHALL return one aggregate result in JSON mode.

#### Scenario: Batch with failure
- **WHEN** a batch of three commands is run with stop-on-error and the second fails
- **THEN** the third is not executed and the aggregate result marks the first as succeeded and the second as failed

### Requirement: Configurable timeouts
Every waiting command SHALL accept a timeout, defaults SHALL be documented in help output, and a session-level default SHALL be settable.

#### Scenario: Per-command timeout
- **WHEN** a wait is run with a 2 s timeout on an element that never appears
- **THEN** the command fails after 2 s, not after the session default

### Requirement: Agent-facing help
Each command SHALL provide help with examples, and a dedicated help command SHALL print a compact guide suitable for inclusion in an agent prompt.

#### Scenario: Prompt guide
- **WHEN** a client asks for the agent guide
- **THEN** the output fits in under 120 lines and covers the snapshot-act-resnapshot loop, refs and settle

### Requirement: Text entry verification
`fill` SHALL read the field back after the entry and report whether the text was verified; when the field holds its value in descendants rather than in its own value, as a composite code field does, it SHALL verify through them, and when it holds it in its own accessible name, through the part of the name that changed with the entry, separators left out, never the whole name; when no value can be read back it SHALL report the entry as unverified and succeed rather than fail with `text_mismatch`, which is reserved for a value read back that differs from the text entered; when `fill` or `type` resolves an identifier to a node that takes no text but holds exactly one editable descendant, that descendant SHALL be the target, named in the reply; when the node holds none and a tap on it raises the keyboard within a bound, the text SHALL go to the focused input, the reply naming the node, and otherwise the action SHALL fail with `invalid_argument`; the reading the read-back compares with SHALL be taken after the field is emptied, when `fill` empties a field that held text before typing; the read-back's source SHALL be the value only when it changed with the entry, else the descendants or the name, so that a placeholder left by the emptying is never compared; the reply's `method` SHALL say `keys` for typed text and `direct` for text entered in one piece, which on iOS is a paste through the pasteboard, documented with its side effect on the pasteboard.

#### Scenario: Code field that names its digits
- **WHEN** `fill` enters a six-digit code into a field whose accessible name spells the digits it holds and shows no value
- **THEN** the reply reports the entry verified through the name, and a wrong code read back fails with `text_mismatch`

#### Scenario: Identifier on the field's wrapper
- **WHEN** `fill` is given an identifier that resolves to a generic node holding one editable field
- **THEN** the text goes into that field and the reply names it

#### Scenario: Name with a fixed suffix
- **WHEN** `fill` enters a six-digit code into a field whose name reads "–, –, –, –, –, –, Verification code, required" before and "1, 2, 3, 4, 5, 6, Verification code, required" after
- **THEN** the reply reports the entry verified through the name, comparing the digits alone

#### Scenario: Node without a field that raises the keyboard
- **WHEN** `fill` is given an identifier that resolves to a generic node with no children and a tap on it raises the keyboard
- **THEN** the text goes to the focused input, the reply names the node and says the keyboard carried the text, and the read-back runs through the node's name

#### Scenario: Composite code field
- **WHEN** `fill` enters a six-digit code into a field whose six boxes hold one digit each
- **THEN** the digits land, the reply reports the entry verified through the boxes, and no `text_mismatch` is raised

#### Scenario: Value that cannot be read back
- **WHEN** `fill` enters text into a field the tree reports without a value
- **THEN** the reply succeeds with `verified: false` and says why

#### Scenario: Mismatch
- **WHEN** the field reads back a value that differs from the text entered after the retry
- **THEN** the reply fails with `text_mismatch` naming both values

#### Scenario: Refill of the same text
- **WHEN** `fill` types into a field the same six-digit code it already holds, the field's name spelling the digits
- **THEN** the field is emptied first, the reply reports the entry verified through the name, never `no_value_to_read`

#### Scenario: Refill with the keyboard up
- **WHEN** `fill` types into the six-box code field the same code it holds, with the keyboard up, in either keyboard state
- **THEN** the reply reports the entry verified through the name and the code is right, never a `text_mismatch` against a placeholder

### Requirement: Queries reach elements out of view
`get` and a `wait` for an element's presence or absence in the tree (`--state attached` and `detached`) SHALL resolve any element the replica holds, in view or not, within one read, and SHALL say in the reply whether the element is in view; only an element the tree does not hold SHALL fail with `element_not_found`, naming what is listed.

#### Scenario: Box of a row scrolled above
- **WHEN** `get box` names a row the list has scrolled above the screen and the tree still holds
- **THEN** the reply gives its bounds and says it is out of view, within one read

### Requirement: Selectors resolve covered elements
An action by text selector or identifier SHALL resolve any element the replica holds within one read and then take the same path as an action by ref, failing with `element_covered` naming the cover or `element_offscreen` as a ref would, within the time a ref takes; the selector wait SHALL apply only to an element the tree does not hold yet.

#### Scenario: Click by text on a covered element
- **WHEN** a click names by text an element one view covers whole
- **THEN** it fails with `element_covered` naming the view within one read, never `element_not_found` after the selector timeout

### Requirement: Change waits, claim release and cancel
`wait --change` SHALL return when the session's listing, in the options of its last snapshot, differs from the one that snapshot showed, at once when it changed before the call, with the delta `snapshot --delta` gives and what moved, or `wait_timeout` at its bound, a change the listing does not show leaving it waiting; with `--frames` a frame change SHALL end it too; `close --keep` SHALL release the session's claim without stopping the app; `cancel` SHALL end the session's command in flight, which then fails with `cancelled`, so a target is not held by a wait a client gave up on, a device call in flight running to its end first. Requests and replies SHALL be built from the shared types whose schemas the CLI, the harness and the MCP server use.

#### Scenario: Wait for the next change
- **WHEN** `wait --change` is called while a tap's effect is still arriving
- **THEN** it returns at the first change with the delta and what moved, and on a still screen it returns `wait_timeout` at its bound

#### Scenario: Change before the wait
- **WHEN** the screen changed after the session's last snapshot and before `wait --change`
- **THEN** it returns at once with the delta

#### Scenario: A change outside the listing
- **WHEN** the replica changes outside the session's interactive listing, or only the frames move, during `wait --change`
- **THEN** it keeps waiting and returns `wait_timeout` at its bound

#### Scenario: Frames on request
- **WHEN** only the frames move during `wait --change --frames`
- **THEN** it returns with `endedBy` `frame` and the delta `unchanged`

#### Scenario: Close and keep
- **WHEN** a session closes with `--keep` while its app runs
- **THEN** the claim is released, the app keeps running, and another session can attach

#### Scenario: Cancel a wait
- **WHEN** a session's wait runs and the session's `cancel` arrives on another connection
- **THEN** the wait fails with `cancelled` within about 100 ms, `cancel` replies that it ended it, and the target serves the next command at once

#### Scenario: Cancel during a tap
- **WHEN** `cancel` arrives while a click's tap is being delivered
- **THEN** the tap goes in and the click completes, its watch cut short with a warning

### Requirement: Typed text is read back
`type` SHALL read the focused field back after entering the text, as `fill` does, SHALL say what it read in its line, and SHALL warn naming what the field holds when it differs, without entering the text again; with no focused field it SHALL say the text was not read back.

#### Scenario: Keys dropped
- **WHEN** `type example.com` reaches the field as `exampl`
- **THEN** the reply says it read back `exampl` and warns that the field differs from the text, advising `fill`

### Requirement: Replies name what the screen holds
A wait's timeout summary SHALL issue the refs it prints to the session; `get value` on a switch SHALL return its state and SHALL say `no value` for an element without one; `wait --text` SHALL name the element it found; `open` SHALL return when a system alert comes in front of the app, naming the alert; the screenshot reply SHALL carry the screen's size in points beside the image's pixels, and a click on coordinates off the screen SHALL name the screen's size in points and say that a screenshot's pixels are not points.

#### Scenario: An alert in front of the opened app
- **WHEN** `open` launches an app whose first screen is covered by a permission alert
- **THEN** `open` returns at once with the app opened and a warning naming the alert, and the next snapshot lists the alert

#### Scenario: Pixels taken for points
- **WHEN** a click names coordinates beyond the screen's points
- **THEN** the failure says the screen's size in points and that the screenshot's pixels are scaled

### Requirement: Replies verify the action
A click SHALL name the element the tap landed on as the replica held it at dispatch, and SHALL warn when it is not the target named; a settle requested within a second of the session's last input SHALL wait for that input's first effect before it counts quiet, and SHALL say when none came; a settle on a target whose windows send no change events SHALL read the tree its frames changed before it ends, and SHALL count a change that read finds; a stale ref whose element has a twin in the replica, by identifier and role, else by name, role and bounds, SHALL resolve to the twin and say so, and SHALL fail `stale_ref` without one; `open` SHALL leave the app in front, activating a running one that stayed behind, and SHALL fail `not_in_front` naming what is in front when it cannot; a plain scroll SHALL answer once the screen has come to rest after its last stroke, within a bound, and SHALL say how long the content moved on; a delta snapshot's changes SHALL print as listing lines in the human output while `--json` keeps the delta object.

#### Scenario: A tap that lands elsewhere
- **WHEN** the layout moves between the snapshot and the tap so that another element lies under the point
- **THEN** the click's reply names the element hit and warns that it is not the target

#### Scenario: A settle right after a press
- **WHEN** `wait --settle` follows a press whose effect comes 300 ms later
- **THEN** it settles after the effect and names what changed last, and with no effect within a second it says so

#### Scenario: A menu's items after a pause
- **WHEN** `wait --settle` follows a tap whose menu takes its items with a frame after a pause longer than the quiet window, on a target whose windows send no change events
- **THEN** the settle ends once the tree holds the items, and the listing right after names them

#### Scenario: A rebuilt field
- **WHEN** the app rebuilds a field's node after the snapshot that listed it
- **THEN** a fill on its old ref acts on the rebuilt field and the reply names the new ref

#### Scenario: A click right after a scroll
- **WHEN** a click follows a plain scroll whose list glides on after the finger lifts
- **THEN** the scroll answers once the list has come to rest, saying how long it moved on, and the click lands on the row the listing then shows

#### Scenario: Open behind the home screen
- **WHEN** `open` names an app that runs behind the home screen
- **THEN** the app comes to the front, or `open` fails naming what is in front

### Requirement: Hits are named without a ref
A click's reply SHALL name the element the tap landed on whether or not the session holds a ref for it, the ref null and the line saying so when none, so that a tap by coordinates or on a plain text says what it landed on.

#### Scenario: A tap on a plain text
- **WHEN** a click by coordinates lands on a text the interactive listing does not give a ref
- **THEN** the reply names the text with `ref` null and the line says `(no ref)`
