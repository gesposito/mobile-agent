# cli

`mobile-agent`, the command line of spec `agent-cli` (design D1 and D9): a
thin client of the daemon in `crates/daemon` that speaks agent-browser's
grammar, ref syntax and JSON envelope, so skills and prompts written for the
web tool carry over where devices have the same concepts. It links nothing
of the daemon; each command is one JSON request over the daemon's Unix
socket, built from the request types of `crates/wire`, and the first
command starts the daemon. It is one of the two agent surfaces: the MCP
server `mobile-agent-mcp` (`crates/mcp/README.md`) serves the same
actions, refs and replies to a client that calls tools, through this
crate's library, so the agent guide holds for either.

```sh
cargo build --release -p cli -p daemon    # mobile-agent and mobile-agent-daemon, side by side
target/release/mobile-agent help agent    # the guide for an agent's prompt
```

## Layout

```
src/
  main.rs            the mobile-agent binary
  lib.rs             run(): parse, settings, dispatch; help topics; doctor; the code module
  args.rs            the grammar, with each command's examples and defaults
  request.rs         a parsed command to its daemon request, built from crates/wire's request types
  client.rs          socket directory, auto-start, version check, retries, reply wait
  output.rs          human output and agent-browser's JSON envelope
  batch.rs           batch: word splitting, entries, the aggregate array
  config.rs          settings files
  doctor.rs          doctor's checks, the host probe, the report
  agent-guide.txt    what help agent prints
tests/               one file per concern: snapshot, errors, commands, sessions, lifecycle,
                     overhead, batch, selection, timeouts, help, doctor, spans
```

## Commands

| Command | Does |
|---|---|
| `devices` | lists connected targets with their state, and what makes an unusable one usable |
| `open <app> [--relaunch] [--env K=V]...` | launches an app, claims its target, waits for the screen to settle, after the launch's first effect when it comes within a second, as a settle right after an action waits for one; leaves the app in front once its screen showed and settled, launching it again when another holds the front, as the home screen took it back from a running app launched while it went home, and fails with `not_in_front` naming what is in front when the app still is not (action-verdicts, design D4); returns at once when a dialog of system UI, such as a permission alert, stands in front of the app, naming it in `alert` and a warning, as it does one that came in front while the screen settled; the launch itself waits at most until the open's start plus its timeout, so on Android `--timeout` gives a cold `am start -W` slower than the adb client's 10 s read deadline the time it needs, and a launch past it fails with `deadline_exceeded` naming the launch and the deadline (android-pass-fixes, design D1) |
| `close [--keep]` | stops the app this session opened and releases the target; `--keep` releases the target and leaves the app running, so the next session finds it as this one left it |
| `cancel` | ends this session's command in flight, such as a wait another shell or client left running: it fails with `cancelled` within about 100 ms, a tap or tree read already under way finishing first and a click whose tap went in completing with a warning; replies once the command has ended, naming it, or that nothing was in flight |
| `snapshot [-i] [-d N] [--delta [--full]]` | lists elements, one per line with role, name and ref, marking with `visible=NN%` what a tap reaches part of; `-i` leaves out what a tap reaches none of, which the full listing keeps at `visible=0%`, a view covering it whole or not, unless a dialog covers it; `-i`'s hint lines count what it leaves out for a cover beside what is scrolled away, naming the cover as a refused action does (`2 of them behind the header`, a view listed before a list across its top being its header whatever it holds), and the full listing's count only what it leaves out, so that `-i`'s counts of covered elements are the full listing's `visible=0%` elements; the keyboard covers what it draws, not the band across the top of its window that its host keeps, holding nothing, for the input accessory view of the field in focus, which the app draws in its own window, as a browser's address bar above the keys; a keyboard window that draws no keys, as under a hardware keyboard, covers nothing and gets no `- keyboard` line, and iOS's edit menu covers only the strip it draws on |
| `click <target>` or `click <x> <y>` (alias `tap`) `[--hold MS]` | taps a ref, a selector or a point in device points and names the element its tap landed on as the replica held it at dispatch, `hit`, with a warning naming both when that is not the element named (Output); a point off the screen fails with `invalid_argument`, naming the screen's size in points and saying that a screenshot's pixels are scaled; on a Simulator a switch takes a press held 40 ms on the control a labelled switch holds, unless `--hold` names another, and so does an element that holds a switch sharing its identifier or lies in one, as the table cell or the label of a settings row, whichever selector named it |
| `fill <target> <text>` | replaces a field's text and reads it back, from the field's value or, for a field without one, from what lies inside it, as a code field's boxes hold its digits, or else from the part of its name the entry changed, separators left out, as a code field that names its boxes, each its digit or a dash, before its label (`readFrom`: `value`, `descendants`, `name`); text typed as keys, as on a Simulator, into a field that holds text, or known only by its name a name that spells the text already, goes in once the field has been emptied and reads empty, and is read back against the emptied field, so a refill of the code a code field holds reads back through its name; the value of a field that is not a text field counts only when it changed with the entry and is no placeholder, as long as the entry and more than dashes, so a code field that shows a box's placeholder as its value reads back through its boxes or its name; an identifier on a view that takes no text but holds exactly one field fills that field, which a warning names, and one on such a view that holds no field and has no ref taps it, waits up to 1 s for the keyboard, or for its window drawing nothing while a hardware keyboard serves the input, and enters the text where the tap put the focus, reading it back through the view, with `ref: null` and a warning saying the keyboard carried the text, or fails with `invalid_argument` when no keyboard comes up: text typed that the field took otherwise is entered again in one piece, with a warning naming what the keys left; a field that still holds other text fails with `text_mismatch`, naming what it holds, the part of a name that changed for a name, and one that shows nothing to read back succeeds with `verified: false`, `reason: no_value_to_read` and a warning; iOS's edit menu that a paste landing nothing leaves up, opened by the tap readying the paste, is closed with a tap on the field near its left end before the reply; a field whose node the app replaces while the text goes in, as a sheet laid out again, is read back where the screen holds it now, found by its identifier and role, else by its bounds and role, and one the tap readying it puts in a new node, as a search card that tap raises, takes the text in its twin, whose ref the reply gives (action-verdicts, design D3). `method` is `keys` for typed text and `direct` for text entered in one piece: on iOS a paste, which replaces the device's pasteboard and can show iOS's paste banner, on Android a set |
| `type [<target>] <text>` | types key events into a field, or into the focused one, and reads that field back as `fill` does, the text verified once the field shows it added to what it held: `text` is what the field holds, and one that holds other text, as a field that dropped keys, is named in a warning that advises `fill`, the text not typed again, since `type` adds to what a field holds; with no field in the tree holding the focus the text is not read back (`reason: no_focused_field`), with a warning; an identifier takes the field inside a view, or the view's keyboard, as `fill`'s does |
| `press <key>` (alias `key`) | presses a key: `Enter`, `Back`, `Home`, `Control+a`, a letter or digit |
| `swipe <direction> [distance]` or `swipe <x1> <y1> <x2> <y2>` | moves the finger |
| `scroll [direction] [amount] [-s <target>] [--until <target>]` | moves the content, in strokes longer than a tap (more than 8 points on Android, 10 on iOS, else `stroke_too_short`) that keep off the keyboard and the system bars and, with the keyboard up, inside what shows of the list (`covered_by_keyboard` when the keyboard leaves less than 120 points of what it scrolls); without `--until` each stroke lifts moving, as a swipe does, so the list glides on after the last, and the reply comes once the screen has come to rest, at most 3000 ms after the last stroke, `settled` saying whether it did and `movedOnMs` how long the content moved on after the last stroke's lift, since a list still gliding takes a tap as a stop (action-verdicts, design D6); with `--until`, stroke by stroke until the element is mostly in view, each stroke at rest before the finger lifts and short enough that the element cannot pass unseen, `visible` giving the share of it in view, and a scroll that stops naming its last stroke's start and end, also as `data.from` and `data.to`, and what moved after it |
| `screenshot [path] [--quality N] [--max-dimension PX]` | saves the latest frame as a JPEG; `points` is the screen's size in points, which a click takes, beside the image's `width` and `height` in pixels |
| `wait <target>`, `wait <ms>`, `wait --text <t>`, `wait --settle`, `wait --change [--frames]` | waits for an element (`--state visible\|hidden\|enabled` on screen, `attached\|detached` held in the tree, in view or not, the reply's `inView` saying which), time, text or a settled screen, naming the element it found by its role, name and ref, `--text Lisbon` naming an element whose text is `Lisbon` alone, case and the whitespace around it aside, before `Lisbon Airport (LIS)` listed above it, and the first that contains the text when none holds it alone (pass-4-fixes, design D8), the full listing giving one that bears a ref and has none yet; a timeout's listing of the screen is the session's last snapshot, its refs the session's; `--change` returns once the listing in the options of this session's last snapshot changed from what that snapshot showed, at once for a change that came before it, a change the listing does not show leaving it waiting, and prints what moved and the delta `snapshot --delta` prints; `--frames` ends it at a frame change too, for a canvas, a map or a screen the tree does not describe; `--settle` within a second of this session's last click, fill, type, press, swipe or scroll first waits for that action's first effect, a change of the tree or the frames, until a second after the action, then for the quiet window, `input` naming the action, `endedBy` what changed last since it, and the line `nothing changed within 1 s of the click` when none came (action-verdicts, design D2); on a Simulator, once quiet, it reads the tree the frames changed and settles again from a change that read finds, so a menu that takes its items with a frame after a pause longer than the quiet window is listed whole right after it (task 3.5) |
| `get text\|value\|box <target>` | reads an element the tree holds, in view or not, at once: `inView` says which, and the human output adds a line when it is out of view; `value` gives a switch's or a checkbox's state as `checked`, printed `on` or `off`, and prints `no value` for an element without either; `text` and `value` of an element that holds a switch sharing its identifier or lies in one, as the table cell or the label of a settings row, read that switch, whose control `click` presses on a Simulator, and `box` stays the element's own; one the tree does not hold fails with `element_not_found` at the timeout, naming what the screen lists, whose refs are then the session's |
| `diff snapshot` | compares the screen with this session's last snapshot |
| `batch [--bail] [command]...` | runs commands in order |
| `status` | reports the daemon, its targets with what each attached target's driver reports, and its sessions |
| `doctor` | checks toolchains, the adb server, emulators, the iPhone's signing, runner and readiness, and the daemon |
| `set timeout <ms>` | sets this session's default timeout |
| `shutdown` | stops the daemon, returning once it has let go of its targets |
| `help [agent \| <command>]` | prints help, a command's help, or the agent guide |

`scroll --until` looks for the element between strokes, so no stroke may
carry the element through its list unseen: a stroke whose finger lifts
moving flings the list on, and a 600-point stroke over 300 ms carried the
fixtures' lists 1,103 to 1,319 points on the API 36 emulator, where they
show about 780, and the Expo fixture's 1,522 to 1,608 on the iOS 26.5
Simulator, where it shows about 730. Each stroke comes to rest before the
finger lifts, so the list moves only as far as the finger, and one that
looks for the element is the amount long, or shorter: at most three
quarters of what the scroll container under the stroke shows along the
scroll's axis, plus the tap threshold, since the list moves only once the
finger has gone that far. At one look at least the element then shows
whole, or a quarter of what the list shows of it when it is longer. The
reply's `amount` is that stroke's length.

It stops once the element is mostly in view: its centre and at least half
of it inside the visible bounds of what holds it (its window, clipped by
the scroll containers around it, less the system bars, keyboard, tab bars
and toolbars drawn over them). Android reports an element its list cuts
off as the part that shows, so one whose bounds end at the edge of a list
that scrolls on counts as partly in view whatever share its bounds give,
and an element longer than the list stops the scroll once it fills the
list. Once the element is on screen, the next stroke brings it in rather
than going on by the amount: one just long enough to carry an element
whose bounds stick out 8 points inside, and one that carries the middle of
an element cut off at the edge, or longer than the list, to the list's
middle, in either case at most the amount, coming to rest before the
finger lifts and with the finger moving at most 400 points a second, so
that a fling, were the platform still to take the finger as moving, stays
short. An element under part of a view across it, as a floating button,
is carried to the side of the view it reaches with the least movement,
of the sides its list can carry it to: in the production-app run of
2026-10-12 strokes aimed below a floating button pushed a list that stood
at its top, and the scroll gave up while a plain scroll moved the list. The reply's `visible` is the share of the element's bounds in view,
from 0 to 1, rounded to a hundredth; an `element_not_found` for an element
on screen but not mostly in view carries it too, as `data.visible`.

`click` makes no stroke of its own. The tap goes to the part of the
element a tap reaches: its bounds within what shows of them, and within
what each scroll container around it shows, less every view drawn over
part of it, the tap landing at the centre of the largest
rectangle left that is at least 8 points on each side, or as long as the
element where it is shorter (`crates/mirror/README.md`, Visibility). An
element whose reachable part is not most of it is tapped there, with a
warning naming the point: in the production-app run of 2026-10-12 a tap at
the centre of a card under a sticky header pressed the header. For an
element nothing of which a tap reaches in view the windows are read again
once, since a layout can move without an event (in the React Native smoke
test of 2026-10-09 the keyboard moved a submit button up, and the tap went
to the number pad, where the replica still had the button), and the tap
goes where that read has the element, or the click fails with
`element_offscreen` and `data.visible` 0, naming what keeps the element
out of view and how to bring it in (close the keyboard, `scroll --until`).
An element under a dialog, or with less than 8 points left from under a
view drawn over it, fails with `element_covered` without that read or a
tap, naming the cover as the hint counting the element does: the dialog,
the header or the footer of its list, or what a tap at its centre would
land on, the control of the view (rn-app-fixes, design D57); on iOS a view
listed after a sibling it overlaps is
drawn over it, since XCTest lists views in the order UIKit draws them,
when it shows something or is a control, enabled or not, and a navigation
bar, tab bar or toolbar or a header pinned over a list's top whatever the
order; on Android a view the agent flags so (`crates/mirror/README.md`,
Snapshots). The keyboard covers too, once that read still has it over the
element, by what it draws: the band across the top of its window that its
host keeps, holding nothing, for the input accessory view of the field in
focus, which the app draws in its own window, as a browser its address bar
above the keys, covers nothing, so a click on a field drawn there taps it
(real-app-fixes, task 5.6).
Such a view, as a fixed footer, a floating button or a sticky
header, would take the tap itself, so the message advises scrolling the
element out from under it rather than tapping it: in the production-app smoke test a tap
on a heading under a consent screen's footer accepted the consent. A ref,
an identifier and a text selector take this one path, as does the tap
with which `fill` focuses its field: a selector names any element the
replica holds, in view or not, one on screen and not covered first, so
that an action on one that is covered or out of view is refused at once,
as on its ref, where in the fifth production-app run of 2026-10-14 a click by
text on a text a button covered whole, a click by identifier on a control
a footer's view held and a fill by identifier on a field behind a dialog
each waited out their 10 s and failed `element_not_found` (rn-app-fixes,
design D53). `fill` and `type` refuse such an element the same way before
looking for its ref. With a hardware keyboard connected iOS draws no keys
and keeps the keyboard's window drawing nothing, which covers nothing and
counts as no keyboard: `fill` types once its field has the focus, and a
view holding no field takes the text once the window is up (design D52);
once a keyboard wait has seen such a window, and until one sees a keyboard
with keys, the waits end at the focus, as a number pad brings up no window
at all under a hardware keyboard (design D57). iOS's edit menu lives in a
window of its own spanning the screen, which covers only the strip the
menu draws on (design D55).

A target is a ref as snapshots print it (`@e3`; `ref=e3` and `e3` work too)
or a selector: `#id` or `id=...` for a test identifier, `text=...` or bare
text for an element whose name or value contains it, one whose name or
value is the text alone, case and the whitespace around it aside, before
one that only contains it (pass-4-fixes, design D8), `text==...` for one
whose whole name or value is the text, `role=...`. Refs never wait; a
selector names any element the screen's tree holds, in view or not, and
waits up to the command's timeout only while it holds none.

## Global flags

Each works before or after the command and has an environment twin; flags
beat the environment, which beats the settings files.

| Flag | Environment | Default |
|---|---|---|
| `--json` | `MOBILE_AGENT_JSON` | off |
| `--session <name>` | `MOBILE_AGENT_SESSION` | `default` |
| `--platform <ios\|android>` | `MOBILE_AGENT_PLATFORM` | any |
| `--device <id>` | `MOBILE_AGENT_DEVICE` | the only target, or the one the session holds |
| `--force` | `MOBILE_AGENT_FORCE` | off |
| `--default-timeout <ms>` | `MOBILE_AGENT_DEFAULT_TIMEOUT` | 10000 |
| `--config <path>` | `MOBILE_AGENT_CONFIG` | see below |
| `--idle-timeout <time>` | `MOBILE_AGENT_IDLE_TIMEOUT_MS` | `1h` (for a daemon this command starts) |
| `--emulator-controller <auto\|off>` | `MOBILE_AGENT_EMULATOR_CONTROLLER` | `auto` (for a daemon this command starts): taps, swipes and frames on an Android emulator go through its gRPC controller when it answers; `off` sends them through the agent, as on a device |
| `--socket-dir <dir>` | `MOBILE_AGENT_SOCKET_DIR` | `$XDG_RUNTIME_DIR/mobile-agent`, else `~/.mobile-agent` |
| `--daemon-bin <path>` (hidden) | `MOBILE_AGENT_DAEMON_BIN` | `mobile-agent-daemon` beside `mobile-agent`, else on `PATH` |

Boolean variables take `1`, `true`, `yes` or `on`. Settings files are read
as agent-browser reads its own: `--config` or `MOBILE_AGENT_CONFIG` alone,
else `~/.mobile-agent/config.json` overlaid key by key by
`./mobile-agent.json`. Their keys are `json`, `session`, `platform`,
`device`, `defaultTimeout`, `idleTimeout` and `emulatorController`; others
are ignored.

`--emulator-controller off` exists because Emulator 37.1.11 serves its
token-protected controller even when started without `-grpc`; only
`-ports` leaves it off, so the setting is how an emulator that serves it
takes the agent's path, the path every physical device takes. Like the idle
timeout it applies to a daemon the command starts: a running daemon keeps
its own, which `status` names on the daemon's line (`emulator controller
off`) and in each emulator's lines, and `doctor` in its `emulator-control`
check; `mobile-agent shutdown` lets the next command start one with the new
setting.

## Output

Human mode prints the listing or a short confirmation (`✓ Done`, as
agent-browser does) on stdout and errors (`✗`) and warnings (`⚠`) on
stderr. A click follows its confirmation with the element its tap landed
on, as the replica held it at dispatch and the listing names it, then its
spans, `✓ Done (hit button "Sign in", dispatch 0.3 ms, first tree delta
120.7 ms)`, saying `(no ref)` after an element other than the keyboard
that the session holds no ref for, as a plain text or one a click by
selector reached before a snapshot listed it, `✓ Done (hit text "Address"
(no ref), dispatch 0.0 ms, first frame change 121.3 ms, first tree delta
156.3 ms)` on Safari's address field on the Simulator (android-pass-fixes,
design D2), under a view without a label the labeled element below it, the
deepest under the point, `hit checkbox "0 minutes"` on Clock's time picker
where the minute's container, which bears an identifier alone, answered
`hit generic (no ref)` (pass-4-fixes, design D5), naming none where no
element a listing lists lies under the point, and with a warning naming both when the element hit is neither the
one the click named, nor part of it, nor what holds it, as when a view of
system UI comes down over it (action-verdicts, design D1); `fill` with what
the read-back found and how the text went in, `✓ Done (read back "agent",
typed)`, `✓ Done (read back 7 masked characters, entered in one piece)`
for a secure field, `entered in one piece` being `direct` as the JSON
names it, a paste on iOS and a set on Android, with `retried` when the
text went in again, and `✓ Done (nothing to read back, typed)` beside the
warning for a field that shows nothing to read; `type` with what the
field holds once the keys went in, `✓ Done (read back "example.com",
typed)`, also when it differs from the text, as `✓ Done (read back
"exampl", typed)` beside the warning that names it, and `✓ Done (not read
back, typed)` when no field had the focus; `scroll` with its strokes and,
without `--until`, when the content came to rest after the last, `✓ Done
(2 strokes, at rest 2341.3 ms after the last)`, `still moving` beside a
warning when it had not within 3000 ms, and with `--until` how much of
the element is in view,
`✓ Done (3 strokes; "Row 57" is 100% in view)`, the element named as the
listing names it; a wait for an element or a text with what it found,
`✓ Found button "Sign in" @e4`, or `✓ Found text "Welcome, agent"` for a
text, which bears no ref; `get value` with the value, a switch's `on` or
`off`, or `no value`; and `screenshot` with the image's pixels beside the
screen's points, `✓ Screenshot saved to /tmp/shot.jpg (460x1000 of a
402x874-point screen)`. `open` prints `✓ Opened <app> on <target>`, and
the warning names a dialog of system UI in front of the app, `⚠ alert
"Allow “FixtureSwiftUI” to use your location?" stands in front of
dev.mobileagent.fixture.swiftui; answer it before you act on the app: a
snapshot lists its buttons`. An action or a `get` on a ref whose element is
gone acts on its twin when the replica holds one, the element the app
rebuilt in its place, of the ref's role and identifier, else of its role,
name and bounds, and its line names both refs first, `✓ Done (@e2 now @e4,
hit textbox "placeholder-textbox-1", dispatch 0.1 ms, no change in 204
ms)` on the fake placeholder app's rebuilt board, a `get`'s on a line after
what it read, `(@e3 now @e5)`, as the reply's `resolved`, `{"stale": "@e3",
"ref": "@e5"}`, does; the session acts on the twin's ref from then on, and
a ref without a twin fails with `stale_ref` (action-verdicts, design D3),
whose message names the dialog of system UI in front, such as a
permission alert, and says to dismiss it first when one holds the screen
(pass-4-fixes, design D7).
`--json` prints one line in
agent-browser's envelope, fields in its order, `code` and `warning` only
when set:

```json
{"success":true,"data":{"origin":"com.android.launcher3","refs":{"e1":{"name":"Home","role":"heading"},"e2":{"name":"Fixture","role":"button"},"e3":{"name":"Settings","role":"button"}},"removedRefs":[],"replicaRevision":1,"snapshot":"- heading \"Home\" [id=launcher-title, ref=e1]\n- button \"Fixture\" [id=launcher-fixture, ref=e2]\n- button \"Settings\" [id=launcher-settings, ref=e3]","target":"emulator-5554","verdict":{"backend":"fake","degraded":false,"fallback":false,"reasons":[]}},"error":null}
{"success":false,"data":null,"error":"e2 is stale: the button \"Fixture\" it named is no longer on the screen. Take a new snapshot and use its refs","code":"stale_ref"}
```

A snapshot's data holds agent-browser's `snapshot`, `origin` (the app in
front), `refs` (`eN` to `{role, name}`) and `removedRefs` (`@eN`), with the
mirror's `replicaRevision`, the `target`, the quality `verdict`, `readMs`
(how long the daemon took to answer, device reads included) and
`servedFromReplica` (whether it answered without reading the device)
beside them; a degraded verdict also sets `warning`. `open` replies
`settled` once the launched app has shown its first tree, past any launch
splash, and the screen has settled after it, the launch counting as a
change of the window in front, with `elapsedMs` from the launch and
`firstTreeMs`, and `alert`, naming a dialog of system UI in front of the
app, when one stands there, and `retriedAfterMs` when the driver launched
the app again after the target refused the first launch, as a Simulator
refused one right after the app's termination, the line saying `✓ Opened
com.apple.Maps on <udid> (retried the refused launch after 3000 ms)`
(pass-4-fixes, design D2);
`status` says for each attached target whether it has a frame source
(`frameSource`; `frames` or `no frames` in human mode). With `--delta`, `snapshot`
becomes agent-browser's delta object, byte for byte as the mirror writes it:

```json
{"success":true,"data":{"origin":"com.android.launcher3","replicaRevision":1,"snapshot":{"kind":"full","refs":{"e1":{"name":"Home","role":"heading"}},"revision":1,"tree":"- heading \"Home\" [id=launcher-title, ref=e1]"},"target":"emulator-5554","verdict":{"backend":"fake","degraded":false,"fallback":false,"reasons":[]}},"error":null}
```

The other kinds are `{"baseRevision", "kind": "unchanged", "revision"}` and
`{"baseRevision", "changes": [{"op": "add|remove|replace", "ref", ...}],
"kind": "delta", "revision", "treeChange": {"deleteCount", "lines",
"startLine"}}`. `crates/daemon/README.md` lists every action's data.

In human mode `snapshot --delta` prints a `full` delta's listing,
`unchanged (revision N)` for `unchanged`, and for `delta` a header naming
the lines of the last delta snapshot's listing it replaces, counted from 1
where `startLine` counts from 0, then the lines that take their place as
the listing prints them, and the refs it removed, `removed refs: @e5, @e6`,
in the form of a snapshot's `removedRefs`, which the full snapshot's human
output does not print (action-verdicts, design D5); the refs it added or
renamed show in its lines. A fill and a Back on the fixture's Login tab:

```
$ mobile-agent snapshot -i --delta > /dev/null
$ mobile-agent fill '#login-username' agent
✓ Done (read back "agent", entered in one piece)
$ mobile-agent press Back
✓ Done
$ mobile-agent snapshot -i --delta
delta (revision 2 from 1): line 2 replaced by
- textbox "Username" [id=login-username, ref=e2]: agent
```

The header says `lines 2 to 4 replaced by` for several lines, `inserted
after line 4`, or `before line 1`, for lines that replace none, and `lines
3 to 4 removed` for lines left without a replacement.

`wait --change` prints what moved and after how long, then the delta as
`snapshot --delta` prints it; frames that move alone, as an animation's,
leave it waiting, and with `--frames` end it with the listing `unchanged`:

```
$ mobile-agent click '#tab-list'
✓ Done (hit button "List", dispatch 0.9 ms, first frame change 0.1 ms, first tree delta 1.0 ms)
$ mobile-agent wait --change
✓ The tree and frames changed after 0.02 ms
- button "Row 1" [id=row-1, ref=e10]
- button "Row 2" [id=row-2, ref=e11]
…
$ mobile-agent click '#tab-anim'
✓ Done (hit button "Anim", dispatch 3.2 ms, first frame change 2.9 ms, first tree delta 4.7 ms)
$ mobile-agent snapshot -i --delta > /dev/null
$ mobile-agent wait --change --frames
✓ Frames changed after 0.04 ms
unchanged (revision 5)
```

The first wait answered at once, since the List tab came in during the
click, after the session's last snapshot; the delta was the whole listing,
the List tab replacing most of the Login tab's.

With `--json` the data is `{"waited": "change", "endedBy": "tree|frame|both",
"elapsedMs", "snapshot": <the delta object>}`, `tree` naming a change of the
listing, and a listing that holds still fails with `wait_timeout` at the
wait's timeout. `close --keep` prints `✓ Released
emulator-5554; dev.mobileagent.fixture.expo keeps running` and its data
gains `"kept": true`; `cancel` prints `✓ Cancelled the wait in flight on
emulator-5554`, or `✓ Nothing in flight to cancel`, its data
`{"cancelled", "action", "target"}`.

Every failure exits with status 1 and carries a snake_case code. The CLI's
own are `invalid_argument` (usage), `unknown_command`, `invalid_config`,
`daemon_unavailable`, `daemon_timeout` and `doctor_failed`; the daemon's,
such as `stale_ref`, `target_claimed` or `ambiguous_target`, arrive in its
reply, and so does `cancelled`, for a command its session's `cancel` ended.

## Timing spans

`click` and `tap` watch what the tap sets off before they reply, and `wait
--settle` says which signal it waited out, so the benchmark harness can read
the spans only the daemon observes ("the tool's own clock" in
`crates/bench/README.md`) from each reply's `data`:

| Harness primitive | Command | Field | Measured |
|---|---|---|---|
| `tap-dispatch` | `click`, `tap` | `dispatchMs` | from the tap's dispatch, once its ref or selector is resolved, until the target accepts the input |
| `tap-to-frame-change` | `click`, `tap` | `frameChangeMs` | from the dispatch to the capture of the first frame whose hash differs from the frame before it |
| `tap-to-tree-delta` | `click`, `tap` | `treeDeltaMs` | from the dispatch to the first tree delta the mirror finds: a re-read after a change event, or for windows without events, a re-read after the action or after frames changed |
| `settle` | `wait --settle` | `elapsedMs`, `endedBy`, `stillChanging` | from the start of the wait until neither signal has changed for the quiet window (200 ms), at most `--timeout` (3000 ms) after it waited, within a second of the session's last action, for that action's first effect, until a second after the action at most (`input`) |

`open` waits for the frame stream's first frame on a target with frames,
at most 2 s: the first frame is no change, so the first click after an
attach reports `frameChangeMs` only once the stream has delivered one.

Spans are milliseconds on the daemon's monotonic clock, to the microsecond.
`frameChangeMs` and `treeDeltaMs` are absent, never 0 or null, when the
signal did not come: the target has no frame source (Android devices, per
design D7), or the tap changed nothing. The watch lasts until both first
effects have arrived (the frame change only on targets with frames), or
nothing has changed for 200 ms since the tap or the last change, or 1 s;
after the first tap of a new input session, such as a Simulator's first
after dtuhidd activated, it waits up to 1.5 s for the first change and
2.5 s in all. `watchedMs` says how long it lasted. So a tap that changes nothing replies
after about 200 ms, and on a screen that was already animating the first
frame change can be the animation's.

`endedBy` names the signal whose last change began the settle's final
quiet window:

| Value | When |
|---|---|
| `tree` | the tree changed last, as on a target without frames |
| `frame` | frames changed last, as when a transition animates after the tree changed |
| `both` | their last changes came within 20 ms of each other |
| `none` | nothing changed since the last action, or the screen was already quiet when the wait began (`elapsedMs` about 0); after an action less than a second before, which `input` names, nothing changed within a second of it, the line ending `nothing changed within 1 s of the click` |
| `timeout` | the screen did not settle in time; `settled` is false and `stillChanging` names the signals, `tree` and `frame`, that changed in the last quiet window |

With a fake target that shows every change 120 ms late
(`MOBILE_AGENT_FAKE_LATENCY_MS=120`, see `crates/daemon/README.md`):

```json
{"success":true,"data":{"accepted":true,"clicked":"@e2","dispatchMs":0.133,"frameChangeMs":120.026,"hit":{"name":"Fixture","ref":"@e2","role":"button"},"ref":"@e2","treeDeltaMs":122.274,"watchedMs":122.495,"x":204.0,"y":120.0},"error":null}
{"success":true,"data":{"accepted":true,"clicked":"@e2","dispatchMs":0.201,"hit":{"name":"Fixture","ref":"@e2","role":"button"},"ref":"@e2","treeDeltaMs":120.413,"watchedMs":123.332,"x":204.0,"y":120.0},"error":null}
{"success":true,"data":{"accepted":true,"clicked":"204 600","dispatchMs":0.013,"hit":null,"ref":null,"watchedMs":323.171,"x":204.0,"y":600.0},"error":null}
{"success":true,"data":{"elapsedMs":297.58,"endedBy":"frame","frameChanges":6,"settled":true,"stillChanging":[],"treeChanges":0,"waited":"settle"},"error":null}
```

The lines are a click on an emulator, the same click on a device without
frames, a tap on empty space (whose watch ran past 200 ms because the
attach's own delayed change arrived during it), and a settle after a move
to another screen, whose 96 ms transition ended it. In human mode:

```
✓ Done (hit button "Fixture", dispatch 0.1 ms, first frame change 120.0 ms, first tree delta 122.3 ms)
✓ Done (dispatch 0.0 ms, no change in 323 ms)
✓ Settled after 297.58 ms: frames changed last
✓ Stopped waiting after 3000.36 ms: frames kept changing
```

With the fake showing every change 300 ms late, a click's watch gives up
before the tap's effect comes, and a settle right after the click waits
for that effect before its quiet window (action-verdicts, design D2): in a
batch, `✓ Done (hit button "List", dispatch 1.7 ms, no change in 200 ms)`
then `✓ Settled after 401.71 ms: frames changed last`; after a tap on
empty space, `✓ Done (dispatch 0.1 ms, no change in 203 ms)` then `✓
Settled after 793.41 ms: nothing changed within 1 s of the click`, its data
`{"elapsedMs":789.055,"endedBy":"none","frameChanges":0,"input":"click","settled":true,"stillChanging":[],"treeChanges":0,"waited":"settle"}`.

## Daemon lifecycle

Before each request the CLI reads `daemon.version` and compares it with the
version of the daemon executable it would start (the release plus the
executable's size and modification time). When they differ, or when no
daemon answers, it starts that executable in its own process group, in
`/`, with stderr appended to `daemon.log`, and waits up to 10 s until a
daemon of that version serves the socket; the new daemon replaces one of
another build, and concurrent first commands share one daemon. A process
that sends many requests, as the MCP server does, checks the version on
its first and again only after one that found no daemon listening
(`client::call_checked`), and may read a reply's data without the refs
map of its listing or delta, skipped as the parser meets it
(`client::Data::WithoutRefs`; design D3 of `profiling`). A daemon
that fails to start is reported with the end of its log. `shutdown` never
starts one. It returns once the daemon has stopped its targets, an Android
target's agent included, and removed its socket and other files, which it
waits up to 5 s for (`crates/daemon/README.md`, Shutdown), so the next
command starts a fresh daemon that never meets the old one's agent; a
target that had not stopped within the 5 s is named in `data.notStopped`
and in a warning. With no daemon running, `shutdown` succeeds with
`{"stopped": false}` and the warning `no daemon was running`. Requests are
retried up to five times on transient errors, 200 ms times the attempt
apart, and the reply is awaited for the command's own timeout plus 10 s,
at least 30 s; a wait that names no timeout may run to its session's
default, which the CLI cannot know, so it is awaited up to the 600 s
ceiling.

## Timeouts

Waiting commands take `--timeout <ms>`: `wait` for an element, text or a
change, and `click`, `fill`, `type`, `get` and `scroll` on a selector. Without one,
the session's default applies (`set timeout`), else `--default-timeout`,
`MOBILE_AGENT_DEFAULT_TIMEOUT` or the config's `defaultTimeout`, else 10000
ms. `wait --settle` waits at most 3000 ms unless given `--timeout`, after
it waited, within a second of the session's last action, until a second
after the action at most for its first effect; `scroll` without `--until`
waits at most 3000 ms after its last stroke for the content to come to
rest; `open` launches the
app and waits for its first tree within that default from its start, the
launch included, so that an Android launch still running then fails with
`deadline_exceeded` naming it (android-pass-fixes, design D1), for the
launch's first effect within a second of the launch, then at most 3000 ms
for its screen to settle, and `--timeout` bounds them all. Timeouts
are at most 600000 ms. Each command's help states its default.

## Batch

As agent-browser's: commands as quoted strings, split into words as a
POSIX shell splits them, or a JSON array of argv arrays on stdin, run in
order. `--bail` stops at the first failure. `--json` prints one array; the
exit status is 1 when any entry failed:

```json
[{"command":["get","text","#login-status"],"error":null,"result":{"text":"Signed out"},"success":true},{"code":"unknown_ref","command":["click","@e99"],"error":"@e99 is not a ref from session \"default\"'s snapshots of emulator-5554; take a snapshot and use the refs it prints","result":null,"success":false}]
```

Entries use the batch's flags, except a `--session`, `--device`,
`--platform` or `--force` on the entry's own line. A command that does not
parse is a failed entry with its code; batch inside batch is refused.

## Doctor

`doctor` prints one line per check with ✓ (ok), ⚠ (warn), ✗ (fail) or -
(skip), a hint below where something can be done, then the Xcode
compatibility table. Only the daemon check fails doctor.

| Check | Looks at |
|---|---|
| `daemon` | the daemon answers doctor (started if needed): pid and version |
| `drivers` | the daemon's registered drivers |
| `xcode` | `xcode-select -p` and `xcodebuild -version`; macOS only |
| `simulator-runtimes` | the available iOS runtimes in `xcrun simctl list runtimes --json` |
| `ios-signing` | `MOBILE_AGENT_IOS_TEAM` and the valid Apple Development identities `security find-identity -v -p codesigning` lists, naming neither; a warning with the step (the team's certificate in Xcode, the variable exported) when either is missing; macOS only |
| `ios-runner` | the runner bundle the daemon installs on a phone, as its `doctor` names it (`iosRunner`: the bundle and the version it expects), present with that version (`plutil` reads its `CFBundleShortVersionString`); a warning with `just agent-ios-build` when it is missing or of another version |
| `ios-devices` | each iPhone the daemon's `devices` lists (class `ios-device`) in its readiness state; for a phone that is not ready, the hint carries the daemon's advice, the step `crates/ios`'s readiness states name (trust, Developer Mode, the disk image, the lock, the signing identity); a warning when no iPhone is attached |
| `android-sdk` | `ANDROID_HOME`, `ANDROID_SDK_ROOT` or the default location, and `adb version` |
| `adb-server` | `host:version` on `ANDROID_ADB_SERVER_PORT` or 5037 |
| `emulator-control` | each running emulator's gRPC control endpoint in its discovery file; the `-grpc`, `-grpc-use-token` and `emu grpc` hint when it has none; a warning, with how to turn it back on, when the daemon's emulator controller setting is off |
| `sfw`, `just` | on `PATH`, with the install commands through Socket Firewall |
| `xcode-compatibility` | the table's rows, which the Simulator driver and the physical iPhone driver supply through `Driver::compatibility`: the Simulator's components and private calls, the iPhone's supported iOS range and its call sites, each with the iOS and Xcode it was validated on |

On this machine every check reports a status, and the Simulator driver
fills the table with the host's components and every private call it
makes (the 20 call rows are cut here):

```
✓ daemon               running, pid 30134, version 0.1.0+1106558.18d9183b130b6d97
✓ drivers              simulator, android
✓ xcode                Xcode 27.0 (27A266a) at /Applications/Xcode.app/Contents/Developer
✓ simulator-runtimes   iOS 26.5
✓ android-sdk          /Users/<user>/Library/Android/sdk, adb 1.0.41 (37.0.1-15733141)
✓ adb-server           running on port 5037, protocol 41
✓ emulator-control     gRPC control: emulator-5554 on port 8554
✓ sfw                  Socket Firewall Free, version 1.15.2
✓ just                 just 1.58.0
✓ xcode-compatibility  29 validated combinations, listed below

Xcode compatibility:
  Xcode 27.0 (27A266a)  Xcode 26.0 up to but not including 28  validated  validated on Xcode 27.0; the CI matrix also builds on Xcode 26
  CoreSimulator 1171.7  1155.4 or later for dtuhidd  validated  input through dtuhidd
  input transport dtuhidd, then Indigo  dtuhidd from CoreSimulator 1155.4, Indigo before  validated  CoreSimulator 1171.7 injects dtuhidd, ...
  tree reader simulator-reader 0.1.0 at ~/Library/Caches/dev.mobileagent.simulator/simulator-reader-f95422e26c0f566b  built for aarch64-apple-ios-sim into this binary  validated  spawned into each booted Simulator, nothing installed
  ...
  Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: spike S3 and tests/dtuhid.rs  -[SimDevice lookup:error:] (CoreSimulator)  validated
  ...
```

A daemon that serves only fake targets registers no device driver, so its
`drivers` check names `fake` and the table stays empty.

The checks of a device driver that the run's `MOBILE_AGENT_DRIVERS` leaves
out ask the host nothing: both outputs keep them in place as skips whose
message says they were not consulted. They are `xcode`,
`simulator-runtimes` and `xcode-compatibility` for the Simulator's driver,
`ios-signing`, `ios-runner` and `ios-devices` for the physical iPhone's
(`xcode-compatibility` runs for it too), and `android-sdk`, `adb-server` and
`emulator-control` for Android's. doctor reads the variable as the daemon
does (a daemon the CLI starts inherits it): comma-separated names,
`simulator`, `ios` (physical iPhones) and `android`, or `none`; unset or
blank, every check runs but the iPhone's, whose driver registers only when
named (`not consulted: the physical iOS driver registers only when
MOBILE_AGENT_DRIVERS names ios`). With `MOBILE_AGENT_DRIVERS=simulator` no
adb runs and the adb server is not asked:

```
✓ daemon               running, pid 85469, version 0.1.0+6a9f20.18dc1c8451cdccf6
✓ drivers              simulator
✓ xcode                Xcode 27.0 (27A266a) at /Applications/Xcode.app/Contents/Developer
✓ simulator-runtimes   iOS 26.5
- ios-signing          not consulted: MOBILE_AGENT_DRIVERS=simulator leaves out the physical iOS driver
- ios-runner           not consulted: MOBILE_AGENT_DRIVERS=simulator leaves out the physical iOS driver
- ios-devices          not consulted: MOBILE_AGENT_DRIVERS=simulator leaves out the physical iOS driver
- android-sdk          not consulted: MOBILE_AGENT_DRIVERS=simulator leaves out the Android driver
- adb-server           not consulted: MOBILE_AGENT_DRIVERS=simulator leaves out the Android driver
- emulator-control     not consulted: MOBILE_AGENT_DRIVERS=simulator leaves out the Android driver
✓ sfw                  Socket Firewall Free, version 1.15.4
✓ just                 just 1.58.0
✓ xcode-compatibility  29 validated combinations, listed below
```

In `--json` each such check reads `{"name": "adb-server", "status": "skip",
"message": "not consulted: MOBILE_AGENT_DRIVERS=simulator leaves out the
Android driver", "hint": null}`.

With `MOBILE_AGENT_DRIVERS=ios`, the runner bundle built and the iPhone SE
(3rd generation) attached, ready (2026-10-10; the identifier stands in for
the phone's, and the 37 call rows are cut):

```
✓ daemon               running, pid 98802, version 0.1.0+e7d530.18dcfc48e4f67822
✓ drivers              ios
- xcode                not consulted: MOBILE_AGENT_DRIVERS=ios leaves out the Simulator driver
- simulator-runtimes   not consulted: MOBILE_AGENT_DRIVERS=ios leaves out the Simulator driver
✓ ios-signing          MOBILE_AGENT_IOS_TEAM is set, and the keychain holds 1 valid Apple Development identity; each iPhone's readiness below tells whether one is the team's
✓ ios-runner           <workspace>/agent-ios/dist/MobileAgentRunner-Runner.app, version 0.2.0
✓ ios-devices          <udid> (iPhone14,6, iOS 26.6.2): ready
- android-sdk          not consulted: MOBILE_AGENT_DRIVERS=ios leaves out the Android driver
- adb-server           not consulted: MOBILE_AGENT_DRIVERS=ios leaves out the Android driver
- emulator-control     not consulted: MOBILE_AGENT_DRIVERS=ios leaves out the Android driver
✓ sfw                  Socket Firewall Free, version 1.15.4
✓ just                 just 1.58.0
✓ xcode-compatibility  38 validated combinations, listed below

Xcode compatibility:
  iOS 26.6.2, Xcode 27.0  physical iPhones: iOS 17.4 up to but not including 27, validated on 26.6.2, through idevice 0.1.68  supported  host input through dtuhidd from iOS 18; another iOS is refused with unsupported_ios
  iOS 26.6.2 (iPhone SE 3rd generation), Xcode 27.0, macOS 26.6.2: spike S9 and tests/discovery.rs  ListDevices, ReadPairRecord, Connect (usbmuxd (/var/run/usbmuxd))  validated
  ...
```

A phone that is not ready turns `ios-devices` into a warning whose hint
names the step, as the daemon's `devices` gives it, such as `<udid>: it is
locked: unlock it, and set Settings > Display & Brightness > Auto-Lock to
Never for long runs`.

## Help

Every command's help ends with examples and states its defaults; `help
<command>` prints the same as `<command> --help`, and `help get text`
reaches a subcommand. `help agent` prints `src/agent-guide.txt`, 119 lines
for an agent's prompt, under the spec's 120: the snapshot, act and
snapshot-again loop, the listing format and what it leaves out, a keyboard
covering what it draws, refs, the twin an app rebuilt and `stale_ref`,
selectors, acting, with `open` leaving the app in front or failing
`not_in_front`, the element a click's reply says its tap hit, `(no ref)`
after one without a ref, `scroll` answering once the content is at rest,
what an action on an element out of sight fails with and what `fill` and
`type` read back, waiting, a settle after an action waiting for its effect
and `wait --change` after an action whose effect may still be arriving,
with `--frames` for a canvas or a map, the `--timeout` that waits and
`open` take, `open` failing `deadline_exceeded` when the launch outlasts
it, reading, with a screenshot's pixels beside the screen's points, output
and codes, targets, sessions and batch, with `close --keep` leaving the
app running and `cancel`, and where a physical iPhone differs (the
readiness codes a command on it fails with and where `devices` and
`doctor` name the step, the slower tier, a tree read of a 500-cell screen
about 0.5 s and frames 19 a second, and text through the runner, `direct`
the field's value set rather than pasted).

## Process overhead

The spec keeps CLI process overhead, from process start to the daemon
request, under 20 ms. The measurement times whole `status` runs against a
running daemon on a fake target, the daemon's reply included, so it bounds
that overhead from above:

```sh
cargo build --release -p cli -p daemon
export MOBILE_AGENT_SOCKET_DIR="$(mktemp -d)" MOBILE_AGENT_FAKE_TARGETS=android
target/release/mobile-agent status > /dev/null    # starts the daemon
hyperfine --warmup 5 --runs 300 -N --export-json overhead.json 'target/release/mobile-agent status'
target/release/mobile-agent shutdown
```

On an Apple M1 Max (10 cores) under macOS 26.6.2, with a load average of 5.6
from parallel builds:

```
Benchmark 1: target/release/mobile-agent status
  Time (mean ± σ):       7.3 ms ±   1.4 ms    [User: 1.7 ms, System: 1.4 ms]
  Range (min … max):     5.2 ms …  14.1 ms    300 runs
```

The exported samples give a median of 6.88 ms and a p95 of 10.00 ms. On the
same host `/usr/bin/true` took 4.6 ms ± 4.2 ms and `mobile-agent --version`,
which parses the command line and contacts no daemon, 5.3 ms ± 1.8 ms: process
creation is most of the cost.

The test `overhead::status_overhead_against_a_running_daemon_is_under_20_ms`
is the gate: it starts the daemon with a first `status`, then requires the
median of 30 `status` runs of the debug build to stay under 20 ms, measured
with hyperfine when it is on `PATH` and timed by the test otherwise.

## Differences from agent-browser

- One daemon serves every session, and sessions are claims inside it,
  because Android allows one UI automation owner per device; agent-browser
  runs a daemon per session. The socket is `daemon.sock`, not
  `<session>.sock`.
- In JSON mode a failure's message travels in the envelope on stdout, as
  agent-browser prints it, and not also on stderr.
- `diff snapshot` returns the mirror's element-by-element diff: `diff` is
  its text (`added:`, `removed:`, `changed:` sections), and `modifications`,
  `added`, `removed` and `modified` sit beside agent-browser's `changed`,
  `additions`, `removals` and `unchanged`; there is no `--baseline` file.
- `snapshot` has no `-c` or `-s`, `screenshot` no `--full` or `--annotate`,
  and `close` no `--all`.
- Every failure has a code, including an unknown ref, which agent-browser
  leaves without one; batch entries that do not parse have one too, and
  keep `result: null`.
- Timeouts are capped at 10 minutes and the default is 10 s, where
  agent-browser's is 25 s.
- `click` and `tap` reply once the tap's first effects have arrived, or
  after 200 ms of quiet, at most 1 s, rather than as soon as the input is
  accepted, and report the spans above.
- `click` makes no stroke: agent-browser's scrolls the element into view
  first, where an element none of which is in view after the windows are
  read again fails here with `element_offscreen`, and `scroll --until`
  brings it in. Both refuse an element something covers and name the
  cover, but where agent-browser says to act on the cover first,
  `element_covered` for a view drawn over the element says to scroll the
  element out from under it, since that view, as a fixed footer, takes the
  tap and can press its own button.
- `fill` reads the field back and says what it read, `verified` (`text`,
  `length` for a masked field, or `false` with `reason: no_value_to_read`
  for a field that shows nothing to read) and `readFrom` (`value`,
  `descendants` or `name`, the part of the name the entry changed), beside
  the `text` as the request gave it, and how the text went in, `method` (`keys`, or `direct`:
  a paste on iOS, which replaces the device's pasteboard, a set on Android)
  and `retried`; agent-browser's `fill` clears the field and types, and its
  documentation checks the result with a snapshot or `diff snapshot`.
- `wait --change`, `close --keep` and `cancel` are this tool's own: they
  serve agents that work through the MCP server (`crates/mcp/README.md`;
  the `mcp-server` change, designs D7 to D9), which waits for a change
  instead of polling snapshots, leaves the app it drove running when it
  exits and ends a wait its client gave up on; that README's tool table
  maps each tool to agent-browser's name.
- The version file names the build as well as the release.
- A config file cannot set the socket directory, and there is no
  namespace flag; `--socket-dir` covers both uses.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Command grammar | Interactive snapshot output | `snapshot::an_interactive_snapshot_lists_a_heading_and_two_buttons_each_ending_with_a_ref`, `snapshot::an_ios_target_lists_the_same_launcher`, `snapshot::a_full_snapshot_also_lists_text_and_system_bars`, `snapshot::depth_limits_the_listing`, and every command in `commands::*` (devices, open and close, click by coordinates, tap, click timing, fill and type, press, scroll and swipe, screenshot, waits, an endless animation, get, diff snapshot, selectors, shutdown) |
| Ref syntax | Stale ref | `errors::clicking_a_ref_from_before_navigation_fails_with_a_stale_ref_error_and_guidance`, `errors::refs_are_accepted_as_at_ref_equals_and_bare_forms`, `snapshot::removed_refs_name_the_refs_the_previous_snapshot_listed` |
| Command grammar | (task 8.8: states and driver status) | `output::tests::devices_list_each_target_with_its_state_and_what_makes_it_usable`, `output::tests::status_prints_what_the_driver_reports_under_each_attached_target`; against a real Simulator and emulator in the daemon's `devices::*` |
| Output formats | JSON error | `errors::a_failed_command_in_json_mode_prints_an_error_object_with_a_code_and_exits_non_zero`, `errors::a_failed_command_in_human_mode_writes_its_error_to_stderr_and_exits_1`, `errors::every_failure_carries_a_snake_case_code`, `errors::an_unreachable_daemon_is_reported_with_a_code`, `errors::a_usage_error_in_human_mode_goes_to_stderr`, `snapshot::the_json_snapshot_follows_agent_browser_shape`, `snapshot::delta_snapshots_start_full_then_report_unchanged_and_changes`, `output::tests::the_json_line_keeps_agent_browser_field_order_and_omits_unset_fields` |
| Daemon lifecycle | Cold first command | `lifecycle::a_cold_devices_starts_the_daemon_and_the_next_command_reuses_it`, `lifecycle::a_daemon_of_another_version_is_replaced_by_the_next_command`, `lifecycle::files_left_by_a_killed_daemon_do_not_block_the_next_command`, `lifecycle::concurrent_first_commands_share_one_daemon`, `lifecycle::the_idle_timeout_flag_reaches_the_daemon_the_cli_starts`, `lifecycle::a_daemon_that_fails_to_start_is_reported_with_its_error`, `client::tests::a_checked_caller_reads_the_version_once_and_again_after_a_call_finds_no_daemon`, `client::tests::a_reply_read_without_refs_is_the_whole_reply_less_the_listing_s_refs_map`, `overhead::status_overhead_against_a_running_daemon_is_under_20_ms`, `commands::status_reports_the_daemon_and_its_targets`; idle exit in the daemon's `idle::*`; task 8.16: `lifecycle::shutdown_returns_once_the_daemon_has_stopped_its_targets_and_removed_its_socket`, `lifecycle::a_command_right_after_shutdown_starts_a_fresh_daemon_without_meeting_the_old_socket` |
| Target selection | Ambiguous target | `selection::two_targets_without_a_device_flag_fail_listing_both`, `selection::one_connected_target_is_selected_by_default`, `selection::the_device_flag_selects_a_target_by_id_or_name`, `selection::the_platform_flag_selects_the_only_target_of_its_platform`, `selection::a_platform_with_several_targets_is_still_ambiguous_among_them`, `selection::a_device_flag_naming_no_target_fails_listing_the_connected_ones`, `selection::the_device_and_platform_flags_have_environment_twins`, `selection::a_session_that_holds_one_target_keeps_using_it_without_a_flag`, `selection::no_connected_target_is_reported_with_a_code` |
| Sessions | Claimed target | `sessions::a_claimed_target_refuses_another_session_naming_the_holder_until_forced`, `sessions::refs_are_isolated_between_sessions`, `sessions::the_session_and_json_flags_have_environment_twins`, `selection::claims_are_per_target`; the daemon's `claims::*` |
| Batch execution | Batch with failure | `batch::a_bailing_batch_stops_after_the_failing_second_command`, `batch::a_batch_without_bail_runs_every_command_and_still_fails`, `batch::a_batch_reads_a_json_array_of_argv_arrays_from_stdin`, `batch::quoted_arguments_keep_their_spaces`, `batch::a_human_batch_prints_each_result_with_a_blank_line_between`, `batch::a_command_that_does_not_parse_is_a_failed_entry_with_a_code`, `batch::a_batch_applies_its_global_flags_to_every_command`, `batch::an_empty_batch_prints_an_empty_array`, `batch::tests::words_split_as_a_shell_splits_them` |
| Configurable timeouts | Per-command timeout | `timeouts::a_2_s_wait_fails_at_2_s_not_after_the_session_default`, `timeouts::the_session_default_applies_to_waits_that_name_no_timeout`, `timeouts::session_defaults_belong_to_their_session`, `timeouts::the_environment_default_applies_when_the_session_sets_none`, `timeouts::timeouts_are_bounded_and_must_be_numbers`, `timeouts::a_long_wait_outlasts_a_short_idle_timeout`, `help::waiting_commands_document_their_default_timeouts`, `client::tests::waits_get_their_own_timeout_plus_a_margin_and_other_commands_30_s` |
| Change waits, claim release and cancel | Wait for the next change; A change outside the listing; Frames on request; Close and keep | `commands::wait_change_prints_what_moved_and_the_delta_and_times_out_on_a_still_screen` (frames alone running to `wait_timeout` without `--frames`, `--frames` refused without `--change`), `commands::close_keep_releases_the_target_and_leaves_the_app_running`, `commands::cancel_ends_the_session_s_wait_that_another_shell_started`, `help::every_command_has_help_with_examples` (`cancel` among them), `help::wait_help_shows_frames_for_a_change_wait`, `help::the_agent_guide_says_when_to_wait_for_a_change_and_what_close_keep_leaves`, `request::tests::every_command_sends_the_line_it_sent_before_less_the_parameters_at_their_default` over `crates/wire/tests/lines.json`'s lines for them; the daemon's `changes::*`, `cancel::*` and `claims::close_with_keep_releases_the_claim_and_leaves_the_app_running_for_another_session` |
| Agent-facing help | Prompt guide | `help::the_agent_guide_fits_in_120_lines_and_covers_the_loop_refs_and_settle`, `help::the_agent_guide_says_what_a_click_refuses_and_what_fill_reads_back`, `help::the_agent_guide_says_where_a_tap_goes_and_what_the_keyboard_hides` (rn-app-fixes, design D37 to D40), `help::the_agent_guide_says_what_fill_reads_from_a_name_and_what_a_snapshot_leaves_out` (design D43 to D45), `help::the_agent_guide_says_what_hints_count_what_snapshot_keeps_and_what_get_reads_out_of_view` (design D50 and D51), `help::the_agent_guide_says_a_hardware_keyboard_covers_nothing_and_a_selector_is_refused_as_its_ref` (design D52 and D53), `help::the_agent_guide_says_what_the_edit_menu_covers_and_when_a_code_fields_value_counts` (design D55 to D57), `help::the_agent_guide_says_where_a_physical_iphone_differs` (physical-ios task 5.3), `help::every_command_has_help_with_examples`, `help::the_top_level_help_points_to_the_agent_guide`, `help::help_for_an_unknown_topic_fails_with_a_code` |
| Text entry verification | Composite code field; Value that cannot be read back; Mismatch; Code field that names its digits; Identifier on the field's wrapper; Name with a fixed suffix; Node without a field that raises the keyboard; Refill of the same text; Refill with the keyboard up | the daemon's `handlers::act::tests::a_fill_of_a_code_into_a_field_of_boxes_is_verified_through_their_texts`, `handlers::act::tests::a_fill_into_a_field_with_no_value_to_read_succeeds_unverified` and `handlers::act::tests::a_fill_whose_field_reads_back_other_text_after_the_retry_fails_naming_both` (rn-app-fixes, design D33); through a name the entry changed, a wrapper's identifier and a field whose label changes, `handlers::act::tests::a_fill_of_a_code_into_a_field_whose_name_spells_it_is_verified_through_the_name`, `handlers::act::tests::fill_and_type_by_the_identifier_of_a_fields_wrapper_go_into_the_field` and `handlers::act::tests::a_fill_by_ref_of_a_field_whose_label_changes_reads_it_back` (design D39); through the part of a name the entry changed, `handlers::act::tests::a_name_shows_the_text_in_the_part_the_entry_changed` and `handlers::act::tests::a_fill_of_a_code_into_a_field_named_by_its_boxes_compares_what_the_entry_changed` (design D43); through a view holding no field, `handlers::act::tests::fill_and_type_by_the_identifier_of_a_node_that_raises_the_keyboard_go_to_the_input_it_focuses` and `handlers::act::tests::an_identifier_on_a_node_whose_tap_raises_no_keyboard_still_takes_no_text` (design D44); the emptying before keys and the read-back against the emptied field, `handlers::act::tests::a_refill_of_the_code_a_field_holds_empties_it_first_and_is_verified_through_the_name` and `handlers::act::tests::a_keyed_fill_into_a_field_holding_text_empties_it_and_types_once_it_reads_empty` (design D49); a code field showing a placeholder value, `handlers::act::tests::a_code_field_showing_a_placeholder_value_is_read_back_through_its_name_in_both_keyboard_states` and `handlers::act::tests::a_value_shows_the_entry_only_when_it_changed_with_it_and_is_no_placeholder` (design D56); the edit menu a lost paste leaves up, `handlers::act::tests::a_fill_whose_paste_lands_nothing_dismisses_the_edit_menu_it_left_up` (design D55); end to end, `errors::a_fill_whose_field_keeps_other_text_fails_with_text_mismatch` and `commands::fill_reads_the_field_back_and_pastes_text_whose_keys_went_in_out_of_order`; `method` and its side effect on the pasteboard in the guide, `help::the_agent_guide_says_what_a_click_refuses_and_what_fill_reads_back` |
| Selectors resolve covered elements | Click by text on a covered element | the daemon's `handlers::act::tests::a_click_by_text_on_a_text_a_button_covers_whole_is_refused_naming_it_at_once`, `handlers::act::tests::a_click_by_identifier_on_a_control_a_view_holds_whole_is_refused_at_once`, `handlers::act::tests::a_fill_by_identifier_on_a_field_behind_a_dialog_is_refused_naming_it_at_once` and `handlers::act::tests::a_selector_on_an_element_the_tree_does_not_hold_waits_its_timeout`, and the mirror's `waits::an_attached_wait_gives_the_match_on_screen_before_one_a_view_covers` (rn-app-fixes, design D53); in the guide, `help::the_agent_guide_says_a_hardware_keyboard_covers_nothing_and_a_selector_is_refused_as_its_ref` |
| Queries reach elements out of view | Box of a row scrolled above | the daemon's `handlers::act::tests::get_and_a_wait_for_presence_resolve_a_row_scrolled_above_at_once_and_say_it_is_out_of_view` and `handlers::act::tests::only_an_element_the_tree_does_not_hold_is_not_found_and_a_wait_for_one_in_view_still_waits` (rn-app-fixes, design D51); end to end, `commands::get_reads_an_element_out_of_view_and_says_so` |
| Typed text is read back (real-app-fixes, design D3) | Keys dropped | `output::tests::type_says_what_it_read_back_from_the_field`, `commands::fill_replaces_a_field_and_type_appends_to_it` (the line end to end, the focused field's too); the daemon's `handlers::act::tests::type_reads_back_the_field_it_typed_into_the_focused_one_without_a_target`, `handlers::act::tests::type_into_a_field_that_drops_keys_warns_naming_what_it_holds_and_types_once` and `handlers::act::tests::type_with_no_focused_field_in_the_tree_says_it_did_not_read_its_text_back` |
| Replies name what the screen holds (real-app-fixes, design D5) | An alert in front of the opened app; Pixels taken for points (a timeout's refs, a switch's value, the element a wait found) | `commands::the_lines_name_what_the_screen_holds`, `commands::wait_returns_when_an_element_or_text_appears_and_when_the_screen_settles`, `output::tests::get_value_prints_a_switch_s_state_and_says_when_there_is_no_value`, `output::tests::a_wait_names_the_element_it_found`, `output::tests::a_screenshot_says_its_pixels_and_the_screen_s_points`; the daemon's `screen::*` |
| Replies verify the action (action-verdicts, designs D1 to D6) | Open behind the home screen (the daemon's, on the fake's home returns); A tap that lands elsewhere (the line naming the element hit); A click right after a scroll (on fakes whose lists glide, `MOBILE_AGENT_FAKE_FLING`: the line saying when the list came to rest, and the row the click opens); A menu's items after a pause (on a fake Simulator's menu board: the listing right after the settle names the items); A settle right after a press (at a 300 ms latency: the effect waited for, nothing within 1 s of a click that changed nothing, the quiet window alone with no input in the last second, open's settle after its launch's first effect); A rebuilt field (the line naming both refs); (a delta's changes as listing lines) | `output::tests::a_settle_after_an_input_says_when_nothing_changed_within_1_s_of_it`, `spans::a_settle_right_after_a_click_waits_for_its_effect_and_names_what_changed_last`, `spans::a_settle_after_a_click_that_changes_nothing_says_nothing_changed_within_1_s_of_it`, `spans::a_settle_with_no_input_in_the_last_second_ends_at_once_on_a_quiet_screen`, `spans::open_s_settle_waits_for_its_launch_s_first_effect`; `output::tests::a_plain_scroll_says_when_the_content_came_to_rest_after_its_strokes`, `commands::a_click_right_after_a_scroll_opens_a_row_of_the_list_at_rest`; `spans::a_listing_right_after_a_settle_names_the_items_a_menu_took_after_a_pause`; `output::tests::a_click_names_what_its_tap_hit_before_its_spans`, `commands::a_click_reports_where_it_landed_and_its_dispatch_time`, `spans::a_click_prints_its_spans`; `output::tests::an_action_on_a_stale_ref_resolved_to_its_twin_names_both_refs`, `errors::a_ref_whose_field_the_app_rebuilt_acts_on_its_twin_and_names_it`, `errors::clicking_a_ref_from_before_navigation_fails_with_a_stale_ref_error_and_guidance` (no twin); `output::tests::a_delta_prints_a_header_its_lines_and_the_refs_it_removed`, `output::tests::the_json_line_keeps_a_delta_object_byte_for_byte`, `snapshot::a_delta_snapshot_prints_the_lines_that_changed_under_a_header_naming_them`; the daemon's `handlers::act::tests::open_launches_again_a_running_app_the_home_screen_took_the_front_back_from`, `handlers::act::tests::open_of_an_app_the_home_screen_keeps_behind_fails_not_in_front_naming_what_is`, `handlers::act::tests::a_click_whose_tap_lands_on_a_view_over_its_target_names_it_and_warns`, `handlers::act::tests::a_stale_ref_acts_on_its_twin_by_identifier_and_role_else_by_name_role_and_bounds`, `handlers::act::tests::a_fill_whose_focus_tap_puts_the_field_in_a_new_node_enters_the_text_into_its_twin` and `handlers::act::tests::a_stale_ref_without_a_twin_fails_stale_ref`, `handlers::act::tests::a_plain_scroll_answers_once_a_gliding_list_came_to_rest_so_a_click_right_after_opens_a_row` and `handlers::act::tests::a_plain_scroll_without_a_glide_answers_one_quiet_window_after_its_stroke`, `handlers::act::tests::a_settle_right_after_a_tap_ends_once_the_tree_holds_what_its_frames_showed`, the mirror's `refs::a_stale_ref_s_twin_is_the_element_of_its_role_and_identifier_else_of_its_name_and_bounds`, and the server's `tools::a_snapshot_after_a_change_answers_with_the_lines_that_changed` |
| Hits are named without a ref (android-pass-fixes, design D2; pass-4-fixes, design D5) | A tap on a plain text (the line saying `(no ref)` after an element other than the keyboard that the session holds no ref for); A tap through a generic container | `output::tests::a_click_whose_hit_has_no_ref_says_so`, the server's `tools::a_click_answers_with_the_cli_line_and_keeps_its_warning` (a click by selector before a snapshot); the daemon's `handlers::act::tests::a_click_by_coordinates_on_a_plain_text_names_it_without_a_ref` and `handlers::act::tests::a_click_by_coordinates_names_the_deepest_labeled_element_under_the_point` |
| A text wait prefers the whole text (pass-4-fixes, design D8) | A city beside its airport | the daemon's `handlers::act::tests::a_text_wait_prefers_the_element_whose_text_is_the_whole_text` (the wait, and a click by `text=`, which resolves its selector the same way) |
| A refused launch is retried once (pass-4-fixes, design D2) | A launch right after a termination (the line naming the wait before the second launch) | `output::tests::an_open_whose_launch_was_retried_says_after_what_wait`; the daemon's `handlers::act::tests::an_open_whose_launch_the_driver_retried_says_after_what_wait` and the simulator's `macos::driver::tests::a_launch_refused_with_esrch_is_launched_once_more_after_the_wait` |
| A stale ref resolves to its twin (pass-4-fixes, design D7) | A dialog in front (the failure's message, which the CLI prints after `✗` and the server after the code) | the daemon's `handlers::act::tests::a_stale_ref_without_a_twin_under_a_system_dialog_names_the_dialog` |

The timing spans (task 8.9) serve the benchmark harness rather than a
spec scenario: `spans::a_click_reports_the_first_frame_change_and_the_first_tree_delta`,
`spans::a_tap_reports_the_same_spans_as_a_click`,
`spans::an_ios_target_reports_the_tree_delta_its_re_reads_find`,
`spans::a_target_without_frames_reports_no_frame_change_rather_than_zero`,
`spans::a_tap_that_changes_nothing_reports_neither_span`,
`spans::a_click_prints_its_spans`,
`spans::a_settle_after_an_edit_ended_on_both_signals`,
`spans::a_settle_after_a_transition_ended_on_the_frames`,
`spans::a_settle_on_a_target_without_frames_ended_on_the_tree`,
`spans::a_settle_that_times_out_names_the_signal_still_changing`,
`spans::a_settle_on_a_screen_already_quiet_ended_on_nothing` and
`spans::a_settle_prints_the_signal_it_ended_on`, with the daemon's
`signals::tests::*` for the classification. Task 8.12 moved the daemon's
timing onto the mirror's timestamps with these outputs unchanged, and
`commands::open_and_close_are_window_changes_that_a_settle_waits_out`
checks that a settle counts a launch and a stop on every platform.
Task 8.13 adds `commands::scroll_refuses_strokes_the_platform_takes_for_a_tap`
and `spans::the_first_click_after_open_reports_its_first_frame_change`;
task 8.15 `commands::open_waits_past_a_launch_splash_for_the_app_s_first_screen`
and `spans::the_first_click_after_open_does_not_wait_for_the_input_session`.
Task 10.12's emulator controller setting has
`lifecycle::the_emulator_controller_setting_reaches_the_daemon_the_cli_starts`,
`output::tests::status_names_the_emulator_controller_when_it_is_off`,
`doctor::a_daemon_that_leaves_the_controller_unused_says_so` and
`config::tests::a_project_file_overrides_the_user_file_key_by_key`.
Task 10.15 adds `commands::scroll_until_stops_only_once_the_element_is_mostly_in_view`,
`commands::scroll_until_fails_when_its_axis_cannot_bring_a_partly_shown_element_in`
and `commands::a_click_on_an_element_mostly_out_of_view_warns`.
Task 10.16 adds `commands::scroll_until_finds_a_row_that_a_flinging_stroke_would_carry_through_the_list`
and `commands::scroll_until_strokes_come_to_rest_so_a_row_stops_where_they_aim`,
on fakes whose lists fling (`MOBILE_AGENT_FAKE_FLING`).
Task 10.23 adds `commands::fill_reads_the_field_back_and_pastes_text_whose_keys_went_in_out_of_order`
and `errors::a_fill_whose_field_keeps_other_text_fails_with_text_mismatch`,
on fakes whose fields take keys out of order or cut text short
(`MOBILE_AGENT_FAKE_TYPING`). Tasks 6.4 and 6.5 of the `mcp-server` change
add `output::tests::fill_says_what_it_read_back_and_how_the_text_went_in`
and
`output::tests::scroll_counts_its_strokes_and_says_where_until_s_element_ended`,
with `fill`'s line checked end to end in
`commands::fill_replaces_a_field_and_type_appends_to_it`,
`commands::press_enter_submits_the_login_form` and
`commands::fill_reads_the_field_back_and_pastes_text_whose_keys_went_in_out_of_order`,
and `scroll`'s in `commands::scroll_and_swipe_move_the_list` and
`commands::scroll_until_moves_the_content_until_the_element_is_on_screen`.
Task 3.4 of `action-verdicts` adds
`output::tests::a_plain_scroll_says_when_the_content_came_to_rest_after_its_strokes`
and `commands::a_click_right_after_a_scroll_opens_a_row_of_the_list_at_rest`,
on fakes whose lists glide as long as their platform's fling lasts and
stop at a tap (`MOBILE_AGENT_FAKE_FLING`).

Doctor (task 8.7) has no spec scenario of its own; `doctor::*` tests each
check against scripted command outputs and
`doctor::doctor_lists_every_check_with_a_status_in_both_outputs` runs the
command. Task 10.24 adds
`doctor::the_run_s_drivers_choose_which_checks_ask_the_host` over scripted
outputs, and
`doctor::the_android_checks_are_not_consulted_when_the_run_names_the_simulator_alone`
and `doctor::the_simulator_checks_are_not_consulted_when_the_run_names_android_alone`,
which run the command against fake tools: `xcode-select`, `xcodebuild` and
`xcrun` alone on `PATH`, an SDK whose `adb` logs its arguments, and an adb
server on a port of its own that counts its connections. Task 5.1 of
physical-ios adds the physical iPhone's checks, spec `ios-device-driver`
"Device not ready" and "Table lists the device paths", over scripted
outputs and a scripted daemon:
`doctor::the_signing_check_reads_the_team_and_the_development_identities`,
`doctor::the_runner_check_finds_the_bundle_the_daemon_installs_and_its_version`,
`doctor::each_iphone_is_listed_in_its_readiness_state_with_the_daemons_step`,
`doctor::the_iphone_checks_need_macos_and_the_device_rows_fill_the_table`,
and `ios` naming that driver rather than the Simulator's in
`doctor::the_run_s_drivers_choose_which_checks_ask_the_host`.

The tests start their daemons with fake targets and
`MOBILE_AGENT_DRIVERS=none` and give every CLI run the same, so a Simulator
or emulator running on the host never shows up in their listings, and
doctor asks neither the host's Xcode nor its adb.

```sh
cargo test -p cli     # builds mobile-agent-daemon through cargo when it is missing
```
