# Design

## Context

The demo passes (notes, transcripts and videos kept privately) ran Claude Code against the server on real apps. The replies that verify, the project's reason to exist, left four cases unverified, all of the same shape: the daemon knew more than it said. At a click's dispatch the replica holds the element under the point, and the click's watch (the 200 ms window that reports the first frame change and tree delta) sees the change but not its subject; a settle (`crates/mirror`'s settle predicate, a quiet window of 200 ms bounded by 3 s) resolves at once when the quiet began before the input's effect; a stale ref (the element gone from the replica) fails although the replica holds the same control again, rebuilt by the app, as task 5.5 of `real-app-fixes` found when `fill`'s read-back learned to find the field again; `open` waits for the app's screen to show and settle but does not check that the app's window is the one in front when the app was already running. The delta object the CLI prints with `--delta` is agent-browser's, byte for byte in `--json` (`crates/cli/README.md`), and the human output prints the same object pretty for a kind other than `full` and `unchanged`.

## Goals / Non-Goals

**Goals:**
- Every reply of the four cases says what happened: the element hit, the effect waited for, the twin resolved, the window in front.
- The human output of a delta reads like the listings it comes from.
- No new always-on cost on the click's dispatch path beyond a lookup in the replica the daemon holds.

**Non-Goals:**
- Hit-testing against the app itself (the replica is the model; what the app does with a tap is what the next listing shows).
- A batch tool or parallel calls in the server (one call per turn stays).
- The Android alarm row's name (the app's accessibility label).

## Decisions

### D1. The click names what it hit
At dispatch, the daemon takes from the replica the element under the tap point, the top-most that a tap reaches (the covers and the reach computation of `crates/mirror` as the listing uses them), and the reply carries `hit` (role, name, ref when the session holds one) beside `clicked`; the line reads `✓ Done (hit button "Add alarm", dispatch 0.2 ms, first tree delta 351 ms)`. When the hit element is not the target the call named (the layout moved between the snapshot and the tap, or a view came over it), the reply warns naming both, and the tap still goes in: the daemon checked the target's reach before dispatch, so a mismatch is the replica's change in between, which the warning makes visible. A click by coordinates names what it hit the same way. Alternative, refusing the tap on a mismatch (rejected: the replica may lag the app by a frame; the tap is what the model asked for, the warning is what it needs).

### D2. A settle after an input waits for the effect
`wait --settle` called within 1 s of the session's last input (a click, a fill, a type, a press, a swipe or a scroll; the session records the time) first waits for the first change of the tree or the frames since that input, up to 1 s, then counts the quiet window as today; the reply says `Settled after 612 ms: tree changed last` as today, or `nothing changed within 1 s of the press` when no effect came, which is the fact a model needs. A settle with no input in the last second behaves as today. `open`'s own settle, which follows its launch, waits the same way for the app's first screen. The mirror's settle predicate does not change (ui-mirror's requirement stands); the daemon composes the first-change wait (the change wait of `real-app-fixes`, D8 of `mcp-server`) with the settle. Alternative, a longer quiet window (rejected: it slows every settle for the few that start too early).

### D3. A stale ref resolves to its twin
When a ref's element is gone from the replica, the daemon looks for its twin in the replica as it is: an element of the same role and identifier, else one of the same role, name and bounds within the slack the reach computation uses; it acts on the twin, the reply's `resolved` naming the new ref and the line saying `(@e221 now @e240)`, and the session's refs gain the twin. Without a twin the failure stays `stale_ref` with its advice. Name, role and bounds together keep a list's rows apart (a row that scrolled into the same bounds has another name); identifier and role alone suffice where the app gives them, as task 5.5's read-back found.

### D4. `open` leaves the app in front
After its launch and settle, `open` checks the replica's foreground owner (the `origin` the listing reports) is the app's package; when it is not and the app is running (the case after `press home`), the driver activates it (on the Simulator a second launch of a running app brings it to the front where the first did not; the task measures which call does) and `open` checks again; when the app still is not in front, `open` fails with `not_in_front` naming what is, instead of answering opened. The Android driver's launch of a running activity brings it to the front already; the check costs one read of the replica.

### D5. Delta lines
The human output of a delta whose kind is `delta` prints a header line, `delta (revision 3 from 2): lines 2 to 4 replaced by`, then the delta's lines as the listing prints them, and the `removedRefs` as the full snapshot prints them; `--json` keeps agent-browser's object byte for byte. The server's result text follows, since it is the CLI's output.

## Risks / Trade-offs

- [The hit element from the replica differs from the app's hit-test] → the warning says the replica's view; the next listing is the truth; no refusal on a mismatch.
- [A twin resolves to the wrong element] → identifier and role first; name, role and bounds together otherwise; the reply names the resolution so a model can see it.
- [The settle's first-change wait adds up to 1 s where nothing was going to change] → only within 1 s of an input, and the reply says nothing came, which ends the model's guessing.
