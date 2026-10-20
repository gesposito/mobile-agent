# Design

## Context

The demo pass (2026-10-20; notes, transcripts and videos kept privately) ran nine tasks through `mobile-agent-mcp` with Claude Code 2.1.304 on `claude-opus-5-5`, one fresh `claude -p` per task: iOS 116 turns, 104 server calls, 9 failed; Android 58, 50, 0. The host: macOS 27.0.1, Xcode 27.1 (27A9275), the iOS 26.5 runtime (23F77), the only one installed; the README's quick start, which lists Settings' General row as `@e11`, ran under Xcode 27.0 (27A266a). The acceptance of `mcp-server` (`reports/12.1-mcp-acceptance.md`) ran the fixture app on the same host with no visibility problem: the fixture's screens are SwiftUI lists and forms; Settings' root is a UIKit collection view, Calendar's sheet and Maps' search field live in sheets over the map. The listing's visibility comes from `crates/mirror`'s visibility module: what a tap reaches (`reach`, the 8-point floor), what a view shows through its scroll containers' clipping (`Sight::shows`), the covers (bars, keyboards, dialogs, pinned headers), as the archived `rn-app-fixes` change built them (its design D31 to D57) against React Native screens.

## Goals / Non-Goals

**Goals:**
- Settings', Calendar's and Maps' on-screen elements listed and reachable as the fixture's are, with a fake screen of the same geometry holding the fix.
- Replies that spare a model a snapshot or a screenshot: `type`'s read-back, usable refs in a wait's summary, a switch's value, the found element, the alert in front, the screen's size in points.
- The server's bound a cap, never a default above the daemon's.
- A second pass on the same tasks with fewer turns and no failed call from the tool's side, counted.

**Non-Goals:**
- Safari's web content in the iOS tree (deferred in `docs/overview.md`; Android's Chrome lists its webview).
- A batch tool or parallel calls in the server.
- The Android alarm row's name (" Alarm" without the time), which is the app's accessibility label.

## Decisions

### D1. Spike S14 before the mirror's fix
The visibility fault is reproduced with the CLI on the Simulator's Settings root, Calendar's New Event sheet and Maps' search field: the full and interactive listings, `get box` of the missing elements, the windows and scroll containers the replica holds with their frames and content sizes, and the visibility module's inputs for one missing row (which container clips it, which cover, which floor), against the fixture's List tab on the same host, which lists. The spike names the cause: a container whose frame or content size the clipping misreads under this runtime, a window the replica misplaces, a cover that is not one, or something else; and whether Xcode 27.1 or the iOS 26.5 runtime changed what the accessibility bridge reports (the fixture's and the quick start's listings under 27.0 are the comparison; an older runtime is not downloaded for this). It also measures the switch: a plain tap and holds of 40, 80 and 150 ms on Settings' Bold Text switch and on the fixture's Animate toggle, ten each, to see which flips a UIKit switch. Report `docs/spikes/s14-settings-visibility.md`; go when the cause is in the mirror's computation (task 2 fixes it), no-go when it is in what the bridge reports (then task 2 gates a reading on the case, as `rn-app-fixes` gated the RN cadence).

### D2. The fix is held by geometry, not by the app
Task 2 adds the failing case to the fake driver's screens or the mirror's fixtures with the geometry the spike recorded (the window, the container's frame and content size, the row's bounds), writes the listing test red, then changes the computation; the verification on the real apps is task 5's. No per-app rule, no "Settings" in the code: lean core.

### D3. `type` reads back, and does not retry
`type` enters text into the focused field with no target; after the entry, the daemon reads the focused element's value as `fill`'s read-back does (`read_back` in `handlers/act.rs`, the same hold and word diff), and the reply carries `text`, `verified`, `readFrom` and `method`, the CLI's line `✓ Done (read back "example.com", typed)` as `fill`'s. A mismatch is a warning naming what the field holds, not a retry: `type` appends, so entering again would double the text; the warning says to `fill` the field instead. No focused field in the tree: `not read back` with a warning saying so.

### D4. A wait's summary issues its refs
The screen summary a `wait_timeout` carries is a listing; the handler that builds it registers its refs with the session as a snapshot does (`remember`), so a model may act on what it saw. Alternative, a summary without refs (rejected: the summary is there to be acted on).

### D5. Replies name what the screen holds
`get value` on a switch returns the accessibility value the tree carries (`on` or `off` as the platform's value reads, or the raw value), and `no value` as a line when the element has none; `wait --text` answers with the element found, `✓ Found text "Welcome, agent" @e12`, as `wait <selector>` names its element; `open` returns as soon as a system alert window is in front of the app, `✓ Opened <app>; an alert "..." is in front` with a warning naming the alert, since the next action is the model's (the dialog covers are the mirror's already); the screenshot reply carries `points` (the screen's size in points) beside the image's pixels and the scale, the CLI's line and the server's text say `460x1000 of a 402x874-point screen`, and `invalid_argument` for coordinates off the screen says the screen's size in points and that a screenshot's pixels are not points.

### D6. The switch press, gated
If S14 finds a plain tap does not flip a UIKit switch while a hold does, the Simulator driver's tap on an element whose role is a switch holds for the shortest duration the spike found reliable, gated on that role and platform; the fixture's SwiftUI toggle and Android's switches, which flip on a tap, keep the tap. If S14 finds the tap lands off the switch's thumb or the row takes it, the fix is where the finding points.

### D7. The server's bound caps
The server passes `--call-timeout` less the margin as a command's `timeout` only when the call gives a larger one; a call that gives none keeps the daemon's default (its session's, else 10 s), so a `click` on a missing element fails at 10 s with `element_not_found` and the model asks for longer with `timeout` where it means to wait. `wait` keeps the same rule: the card and the tool's description say to pass `timeout` for a wait past 10 s. This reverses `mcp-server`'s D7 insertion, whose aim, the daemon's wait ending before the server's bound, the cap meets. The screenshot result's text states the sizes (D5). The card says that a click which reports no change may precede the app's reaction, and that `wait` with `change` collects it, which the Photos task showed (a push 428 ms after the tap reported none).

## Risks / Trade-offs

- [The cause is in the bridge, not the mirror] → D1's no-go path gates a reading on the case, measured; the fake screen still holds the behavior.
- [`type`'s read-back on a field that is not focused reads the wrong element] → read only the element the tree marks focused; otherwise `not read back` and the warning.
- [A 10 s default makes a slow app's waits fail where 53 s passed] → the card and descriptions say to pass `timeout`; the acceptance's tasks passed with the daemon's defaults before `mcp-server` raised them.
