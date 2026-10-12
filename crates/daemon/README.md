# daemon

The host daemon of design D1 and spec `agent-cli`: one process per user
holds every device connection, keeps a mirror (`crates/mirror`) per attached
target, and serves the `mobile-agent` CLI (`crates/cli`) on a Unix socket
with agent-browser's newline-delimited JSON. One daemon owns every target,
because Android permits one UI automation owner per device, so sessions are
claims inside it rather than daemons of their own (research note
`docs/research/protocol.md`, "Differ"). Commands for different targets run
concurrently.

The binary is `mobile-agent-daemon`; the CLI starts it on its first
command. It carries the Android agent: `build.rs` runs
`agent-android/build.sh` on a copy of `agent-android` in its `OUT_DIR`
(so builds of other profiles never share the script's `out/` and `dist/`,
and the source tree is left alone), embeds the jar and records its digest,
the build `AgentJar::build` names, so an installed daemon deploys the agent
with neither the source tree nor `MOBILE_AGENT_JAR`. The build fails, with
the script's output, when the jar cannot be built; `MOBILE_AGENT_JAR` at
build time embeds a prebuilt jar instead, and `SKIP_AGENT_BUILD=1` embeds
none (the CI jobs without the Android SDK set it), leaving a daemon that
deploys an agent only from the jar `MOBILE_AGENT_JAR` names at run time.
`MOBILE_AGENT_JAR` at run time takes precedence over the embedded jar, for
development. The library holds the server, the target registry, the driver
contract, the device drivers (the Simulator driver of `crates/simulator`,
the physical iPhone driver of `crates/ios` and the Android driver of
`crates/android` behind the mirror's backend traits) and the fake driver
every test without a device runs on.

## Layout

```
src/
  main.rs       the mobile-agent-daemon binary: flags, environment, exit status
  server.rs     startup and replacement, sidecar files, connections, shutdown, idle exit
  paths.rs      the socket directory, its files, the build's version string
  wire.rs       the request and reply envelope, the error codes
  registry.rs   discovery, target selection, claims, session settings, workers
  worker.rs     one thread per target owning its mirror; per-session state
  handlers/     what each action does: mod (status, devices, open, close, set_timeout,
                doctor), view (snapshot, diff, get, wait, screenshot), act (click,
                fill, type, press, swipe, scroll), aim (refs and selectors)
  keys.rs       key names to Android key codes and iOS HID usages
  signals.rs    the signal a settle ended on, from the changes the mirror timed
  driver.rs     the Driver factory contract and the registration point
  devices/      the device drivers: simulator (Simulator driver), ios (physical iPhones),
                android (Android driver)
  fake/         the fake driver: target list, device model, real-time adapter, trees
tests/          claims, controller, idle, ios_device and lifecycle, over the socket against
                the binary; bench_inputs, the same for the screen the benches time; devices,
                the CLI against a real Simulator, emulator and iPhone
benches/        daemon.rs, the microbenchmarks of design D14 of the microbenchmarks change:
                Request::parse of the line the CLI sends for snapshot -i, reply_line of the
                reply to it on the fake fixture's Big tab (500 cells), with their
                allocations, and that snapshot through the socket of a daemon on a fake
                emulator, one request per iteration
```

`cargo bench -p daemon -- --noplot [filter]` runs the benchmarks with
criterion's default settings, about 10 s each, after a setup that starts a
daemon on a fake target, brings it to the Big tab and stops it once the
socket benchmark is done. The library and the binary set `bench = false`, so
their test harnesses never see criterion's flags. A run on a busy host is no
measurement; numbers for a report come from a quiet one (design D15), which
`just micro` waits for before it compares every suite with
`reports/micro/baseline.json`.

## Running

```sh
mobile-agent-daemon [--socket-dir <dir>] [--idle-timeout <time>] [--drivers <names>]
                    [--emulator-controller <auto|off>]
                    [--fake <targets>] [--fake-latency <time>] [--fake-frame-delay <time>]
```

| Flag | Environment | Default |
|---|---|---|
| `--socket-dir` | `MOBILE_AGENT_SOCKET_DIR` | `$XDG_RUNTIME_DIR/mobile-agent`, else `~/.mobile-agent`, else `mobile-agent` in the temporary directory |
| `--idle-timeout` | `MOBILE_AGENT_IDLE_TIMEOUT_MS` | `1h`; raw milliseconds or a whole number with `s`, `m` or `h`; `0` never exits |
| `--drivers` | `MOBILE_AGENT_DRIVERS` | the device drivers the host runs by default: `simulator` (macOS only) and `android`; none when fake targets are named. Comma-separated names, `simulator`, `ios` (physical iPhones, macOS only, registered only when named) and `android`, or `none` |
| `--emulator-controller` | `MOBILE_AGENT_EMULATOR_CONTROLLER` | `auto`: taps, swipes and frames on an Android emulator go through its gRPC controller when it answers. `off` sends them through the agent on every emulator, as on a device, with no frame stream; fake emulators then have no frame source either. Any other value is a usage error |
| `--fake` | `MOBILE_AGENT_FAKE_TARGETS` | none |
| `--fake-latency` | `MOBILE_AGENT_FAKE_LATENCY_MS` | `0`; how long fake targets take to show a change, in the same forms |
| `--fake-frame-delay` | `MOBILE_AGENT_FAKE_FRAME_DELAY_MS` | `0`; how long after the attach fake targets' frame streams start, in the same forms |
| `--fake-splash` | `MOBILE_AGENT_FAKE_SPLASH_MS` | `0`; how long fake targets' fixture apps show their launch splash when their process starts |
| `--fake-activation` | `MOBILE_AGENT_FAKE_ACTIVATION_MS` | `0`; how long after it starts a fake target's input session activates |
| `--fake-stop` | `MOBILE_AGENT_FAKE_STOP_MS` | `0`; how long detaching a fake target takes, as stopping the Android agent takes |
| `--fake-fling` | `MOBILE_AGENT_FAKE_FLING` (`1`, `true`, `yes` or `on`) | off; fake targets' lists glide on after a stroke whose finger lifts moving, as far as their platform's fling carries them |
| `--fake-typing` | `MOBILE_AGENT_FAKE_TYPING` | `whole`; how fake targets' fields take text whose method the target may choose, as `fill` sends it: `whole`, set directly as the Android agent sets it; `keys`, typed key by key up to 32 characters and pasted beyond, as a Simulator enters it; `reorder`, as `keys` with each key from the fourth on landing before the field's last character; `max:<n>`, whole, the fields keeping their first `n` characters however text reaches them. Any other value is a usage error |

Flags beat the environment. A daemon exits 0 after `shutdown`, after its
idle timeout, or when a daemon of the same build already serves the socket;
it exits 1 when it cannot start, printing why on stderr (the CLI appends
that to `daemon.log` and reports its end).

## Socket directory

| File | Content |
|---|---|
| `daemon.sock` | the Unix socket; its path must fit in 103 bytes |
| `daemon.pid` | the serving daemon's pid |
| `daemon.version` | the release, `+`, then the executable's size and modification time in nanoseconds, in hex: `0.1.0+47f518.18d90f0fdb09d75c` |
| `daemon.lock` | serializes startups; removed on exit when no startup holds it |
| `daemon.log` | stderr of daemons the CLI started, appended |
| `android-agents/` | a record per serial of the Android agents this daemon started (socket, pid, token, versions, build), mode 0600, so the next daemon reuses an agent this one left running |

The directory is created with mode 0700. Startups take the lock. Under it
a new daemon connects to the socket: a daemon whose version file matches
keeps the socket and the new one leaves; one of another version is sent
`shutdown`, then SIGTERM and SIGKILL if it has not finished within 5 s, and
replaced; a socket nobody answers on is stale and removed. The new daemon
writes its pid and version files, then binds the socket, so a client that
reaches the socket finds the version of the daemon behind it. On exit a
daemon removes the socket only if its inode is still the one it bound, and
the version and pid files only if the pid file still names it, so a
successor's files survive; then the lock file, unless a startup holds the
lock, and the directory when nothing else is left in it, so a daemon on a
directory of its own, such as a test's under the temporary directory,
leaves nothing behind (Shutdown gives the order). A startup that locked a
lock file an exiting daemon then removed takes the lock again on the file
at the path, so no two startups hold locks on different files. Shutting
down and the idle exit wake the accept loop through a pipe it waits on
beside the socket, not by connecting to the socket's path, so a daemon
whose socket file was removed still stops.

Because the version names the build, the CLI replaces a daemon started from
an older build of the same release, not only from another release.

## Protocol

One JSON object per line, one reply per request; a connection may carry
several requests. Lines that start like an HTTP request are refused.

```
{"id": "r123456", "action": "snapshot", "session": "default", "interactive": true}
{"id": "r123456", "success": true, "data": {...}, "error": null}
{"id": "r123457", "success": false, "data": null, "error": "e2 is stale: ...", "code": "stale_ref"}
```

`session` defaults to `default`. `force: true` takes a claimed target over,
and when the command attaches the target, what keeps it from automation:
another holder of Android's UI automation, which the new agent stops.
`device` and `platform` select the target. `code` and `warning` appear only
when set; `data` carries details on some failures (`holder`, `candidates`).

| Action | Parameters | Data |
|---|---|---|
| `status` | | `daemon {pid, version, socket, uptimeMs, idleTimeoutMs, drivers, emulatorController}`, `targets [{id, platform, class, name, state, attached, attaches, frameSource, app, claimedBy, driver}]` (`attaches` how many times the daemon attached the target; `frameSource` whether the attached target has a frame source, null before it is attached; `driver` while attached: see Drivers), `sessions [{name, targets, defaultTimeoutMs}]` |
| `devices` | `platform` | `devices [{id, platform, class, name, state, model, os, advice, claimedBy}]` (`advice` only for a target its state keeps from automation) |
| `open` | `app`, `relaunch`, `env ["K=V"]`, `timeout` | `opened`, `target`, `settled`, `elapsedMs` (from the launch request until the screen settled), `firstTreeMs` (until the replica showed the app, null when it did not) |
| `close` | | `closed`, `app`, `target`; the claim is released |
| `snapshot` | `interactive`, `maxDepth`, `delta`, `full` | `snapshot` (text, or with `delta` agent-browser's delta object), `refs`, `removedRefs` (without `delta`), `origin`, `readMs` (the target's worker's time to answer, device reads included), `servedFromReplica` (answered without reading the device), `replicaRevision`, `target`, `verdict {backend, fallback, degraded, reasons [{code, advice}]}` |
| `diff_snapshot` | `interactive`, `maxDepth` (without a baseline) | `changed`, `diff`, `additions`, `removals`, `modifications`, `unchanged`, `added`, `removed`, `modified` (elements, with `fields`), `baseReplicaRevision`, `replicaRevision`, `target` |
| `click` | `target` or `x`, `y`; `hold`, `timeout` | `clicked`, `ref`, `x`, `y`, `accepted`, `dispatchMs`, `frameChangeMs` and `treeDeltaMs` when they came, `watchedMs` |
| `fill` | `target`, `text`, `timeout` | `filled`, `ref` (the field's, also when an identifier named the node around it; null for a node holding no field, whose keyboard took the text), `method` (how the text the field holds went in: `keys` or `direct`, which is a paste on iOS and a set on Android), `verified` (`text` when the field showed the text, `length` when it showed as many mask characters, a secure field's; `false` when it showed nothing to read back; null when it was not read back), `readFrom` (where the read-back took the text from: `value`; `descendants` for a field without a value of its own; `name` for one with nothing below it whose accessible name the entry changed, compared by the part that changed; null unless verified), `reason` (`no_value_to_read` when `verified` is false, else null), `readBackMs` (from the end of the entry to the read-back taking the field's text, or to its giving up on a field with nothing to read; null when not read back), `retried` (whether the text went in again in one piece) |
| `type` | `text`; `target`, `timeout` | `typed`, `ref`, `method` |
| `press` | `key` | `pressed`, `code`, `meta` |
| `swipe` | `direction`, `distance`, or `from`, `to` as `[x, y]`; `duration` | `swiped`, `from`, `to` |
| `scroll` | `direction`, `amount`, `target`, `until`, `timeout` | `scrolled`, `direction`, `amount`, `strokes`, `from` and `to` (where the last stroke came down and lifted, as `[x, y]`: every stroke's without `until`, null when it made none); with `until`, `amount` is the length of the strokes that look for the element, and `until`, `visible` (the share of the element's bounds in view, from 0 to 1) and `element` follow; a scroll with `until` that stops gives its last stroke's `from` and `to` in its failure's data |
| `screenshot` | `path`, `quality`, `maxDimension` | `path`, `format`, `width`, `height`, `quality`, `seq`, `ageMs`, `source` (`capture` when the driver took it now, with `seq` null; `frame` for the mirror's latest frame) |
| `wait` | `ms`; or `settle`, `quiet`, `treeOnly`, `timeout`; or `target` or `text`, `state`, `timeout` | `waited` (`time`, `settle`, `selector`, `text`); settle: `settled`, `endedBy` (`tree`, `frame`, `both`, `none`, `timeout`), `elapsedMs`, `stillChanging` (`tree`, `frame`), `treeChanges`, `frameChanges`; elements: `elapsedMs`, `element {ref, role, name, id, value, enabled, bounds}` |
| `get` | `what` (`text`, `value`, `box`), `target`, `timeout` | `text`, or `value`, or `x`, `y`, `width`, `height` |
| `set_timeout` | `ms` | `session`, `defaultTimeoutMs` |
| `doctor` | | `pid`, `version`, `drivers`, `emulatorController`, `compatibility {rows [{subject, feature, status, note}], note}` |
| `shutdown` | | `stopped`; `notStopped` when a target had not stopped within 5 s. The reply comes once the daemon has stopped its targets and removed its files (see Shutdown), and the daemon exits after it |

Every target action but `set_timeout` claims its target. Points are device
points, as the mirror reports bounds. `dispatchMs` is the time the input
sink took to accept the tap, the span of the benchmark's `tap-dispatch`.

Element arguments (`target`, `until`) are a ref (`@e3`, `ref=e3`, `e3`), or
a selector: `#id` or `id=<id>`, `text=<text>` or bare text for an element
whose name or value contains the text, `text==<text>` for one whose whole
name or value is the text, `role=<role>`. The mirror's selectors match text
by containment, so an exact-text selector looks through the full listing,
where every element on screen and not covered is listed with its computed
name and value, every 10 ms while the mirror keeps the replica current.

`scroll` moves the content `amount` points in as few strokes as fit in 80%
of the scrolled area, all of one length, so no short last stroke is left
over. A stroke no longer than the platform's tap threshold is a tap to the
app (`scroll down 2` opened what lay under the Simulator's center), so
`scroll` refuses one with `stroke_too_short` before it touches the screen,
naming the threshold in the message and as `data.tapSlop`: 8 points on
Android, whose touch slop (`config_viewConfigurationTouchSlop`) the API 36
emulator's framework declares as 8 dp, and 10 points on iOS, where UIKit's
scroll views pan only past 10 points. On the fixtures' lists, strokes that
started on a row opened its detail up to 8 points on the emulator (the
Compose fixture) and 10 points on the Simulator (the Expo fixture), and
scrolled from 9 and 11 points.

`scroll` strokes through what the system bars and the keyboard leave of
the element it scrolls, or of the screen (`Mirror::uncovered`): its bounds
less the system and input-method windows above its window, or for the
screen above the window in front, each cutting them as a bar cuts an
element's visible bounds, where it spans them from one side to the other
and reaches one of their edges, and while the keyboard cuts them, through
what shows of the scroll container that shows the most of what is left
(`Mirror::scroll_container_in`, design D41), so below a header pinned over
the list and above the keyboard: in the acceptance run of 2026-10-12
`scroll up 300` with the keyboard up came down at 119.5 points on a fixed
header and moved nothing. A stroke that comes down on the keyboard
goes to the keyboard, which takes it for typing (task 10.32: on a Nexus 5X
with Gboard up, `scroll --until` stroked across the keyboard 50 times and
glide typing entered "by" into the focused field 49 times; with Gboard
docked on the API 36 emulator, `scroll down 600` came down at 757 points,
179 below the keyboard's top, and typed "by"). The longest stroke, 80% of
the extent, comes from that part, and with `until` the part is taken again
before every stroke, since a stroke can bring the keyboard up or send it
away. A keyboard window that spans the area neither across nor down, as
Gboard's stylus toolbar or its floating keyboard, cuts nothing, as it cuts
no visible bounds. When the keyboard leaves too little of the area for a
stroke longer than a tap, where the area alone takes one, or less than 120
points along the scroll's axis (`STROKE_ROOM`), where the area alone has
that much, which holds no stroke that moves the list's content, `scroll`
fails with `covered_by_keyboard` before it touches the screen, naming what the
keyboard covers and how to close it: `press Back` on Android, and on iOS
the field's return key, as in most apps.

`scroll` with `until` scrolls in strokes of at most `amount` points (at
most 80% of the scrolled area) and lets the screen settle after each, at
most 1.5 s, so a check never races a moving list; it stops once the element
is on screen, not covered and mostly in view, and fails with
`element_not_found` when two strokes in a row change nothing (the content
stopped moving), when no stroke along its axis brings more of an element on
screen in, after 50 strokes, or at its timeout, naming its last stroke's
start and end, in the message and as `data.from` and `data.to`, and what
moved after it: the element by how many points which way, else the screen,
else nothing (design D46: the third acceptance run's scroll gave up after
strokes that went the other way, its reply naming only the step). A stroke
that brings an element in aims at the side of a view across it that the
element's list can carry it to, as `Visibility::viewport` keeps it, where
that run's strokes aimed below a floating button pushed a list standing at
its top, which a plain scroll then moved. It looks for the element
only between strokes, and a stroke whose finger lifts moving flings the
content on: a 600-point stroke over 300 ms carried the fixtures' lists
1,103 to 1,319 points on the API 36 emulator, where they show about 780,
and the Expo fixture's 1,522 to 1,608 on the iOS 26.5 Simulator, where it
shows about 730, so a row could pass through the list between two looks
(task 10.16: `scroll down 600 --until row-100` failed in 3 of 4 runs on
each of the emulator's fixtures, and on the Simulator `row-25`, which the
first stroke carried past, in all 4 runs). Every stroke it makes therefore
comes to rest before the finger lifts (`Release::Resting`, which the
drivers carry out), so the content goes no farther than the finger, and a
stroke that looks for the element carries the content at most three
quarters of what the scroll container under the stroke shows along the
scroll's axis (`Mirror::scroll_viewport`; the scrolled area stands for a
container the replica does not have), plus the platform's tap threshold,
since the content moves only once the finger has gone that far. At one look
at least the element then shows whole, or a quarter of what the container
shows of it when it is longer. Mostly in view means its centre inside the
visible bounds the mirror gives it (`Mirror::visibility`: its window,
clipped by the scroll containers around it, less the system and keyboard
windows above it and the tab bars and toolbars drawn over them) and at
least half of its bounds inside them, its bounds cut off at no edge where a
container around it can still scroll, as its scroll semantics say, since
Android reports an element its list cuts off as the part that shows (task
10.15: a frame-path run of the harness's list-scroll flow stopped with
`row-200` 1.2 points tall at the Expo list's bottom edge, all of the row
Android reported); an element longer than the visible bounds along the
axis, or cut off along it, is mostly in view once it fills them. Once the
element is on screen but not mostly in view, a stroke of the step could
carry it past the far edge, so the next stroke only brings it in, whichever
way that takes: one that sticks out of the visible bounds just far enough
to carry it 8 points inside, and one cut off at an edge, whose length is
unknown, or longer than the visible bounds, far enough to carry the middle
of its bounds to theirs; the platform's tap threshold is added, since the
content moves only once the finger has gone that far, the stroke is at most
`amount` long, and the finger moves at most 400 points a second, so that a
fling, were the platform still to take the finger as moving, stays short.
`visible` rounds the share to a hundredth, and an `element_not_found` for
an element on screen carries it as `data.visible`; for an element Android
cut off it is the share of what Android reports, which can be 1.

`fill` reads the field back after the entry (task 10.23: on Bluesky's
search field a fill of "nobel", typed as keys 15 ms apart, left "noelb" and
replied success). Text it typed as keys it takes once the field shows it
and has held it since 200 ms after the entry, the settle's quiet window,
as an app that owns its field's value can set it again between the keys
and after them: shown by a read that began that late, or by an earlier one
after which neither the tree nor, on a target whose windows are read again
as frames change, a frame changed. Text that went in in one piece, pasted
on a Simulator, set through the runner on a physical iPhone or set directly
on Android, leaves such an app no keys to
interleave with, so `fill` takes it at the first read that shows it: one
that began after the entry, or an earlier one after which nothing changed
(task 10.30: a hold after such text was 200 of the 224 ms a text entry took
at p50 on the emulator's Expo fixture, whose fields report a change at
once). A field that shows other text is read again as reads fall due, up
to 1 s after the entry, since a paste showed 84 to 462 ms after Command-V
in the fixtures' fields on the iOS 26.5 Simulator. A field
counts as showing the text when it shows the text itself, as many mask
characters (`•` or `●`), some perhaps showing the text's own, as a secure
field does, or nothing while it is named as the text, since the
Simulator's reader leaves out a value that repeats the placeholder. Other
text at the bound gets the text again in one piece, `direct`: pasted on a
Simulator, the field's value set through the runner on a physical iPhone,
which leaves the pasteboard alone, set with `ACTION_SET_TEXT` on Android,
as Playwright's and agent-browser's single `Input.insertText` and
UiAutomator2's set text are, rather than typed again into a field that can
reorder keys again; the reply
then has `retried` and a warning naming what the field showed. Still other
text fails with `text_mismatch`, naming what the field holds and the text
entered (by its length where the field shows nothing or mask characters).
On Android an empty field reads as its hint, so an empty fill there is not
read back. A field that leaves the screen before it shows the text ends the
read-back with a warning.

What a field shows is its own value when it has one, or when it is a text
field (a textbox or a search field), whose empty value the platform
reports as none (design D33). A field without a value of its own is read
back through its descendants: their values, else the names of the texts
and untraited nodes with nothing listed below them, joined in the order of
the full listing, as a React Native code input built of six boxes keeps
its digits in the boxes' texts (in the smoke test of 2026-10-09 `fill`
typed such a code and failed with `text_mismatch`, reading the field
alone); the reply's `readFrom` says `descendants`. A field with nothing
below it whose accessible name the entry changed is read back through the
name (design D39), as a React Native code input on iOS spells the digits
it holds in its name (in the acceptance run of 2026-10-12 all 7 fills of a
code replied `verified: false` while the name spelled the digits, and a
wrong code went unreported): the part of the name the entry changed, what
lies between the words the names before and after the entry share at
their start and at their end, the names cut into words at spaces, commas,
dashes and underscores, compared with the text, both without those
separators (design D43), as a code field names its six boxes, each its
digit or a dash, before its label: "–, –, –, –, –, –, Verification code,
required" becomes "1, 2, 3, 4, 5, 6, Verification code, required", where
the third acceptance run compared the whole name and failed every right
code with `text_mismatch`. The part takes in the shared words next to it
when they spell the text with it, as a code entered over one it starts as
does; a part that shows only bullets tells nothing but the text's length
and is other text, which `text_mismatch` names by its count. The reply's
`readFrom` says `name`, and a mismatch names the part that changed. A name
that did not change is a label, never compared. A
field that shows none of these, as iOS reads a React Native field inside a
view marked accessible (a labelled node with nothing below it), cannot be
read back: once the
screen has held still from a settle's quiet window after the entry, the
fill succeeds with `verified: false`, `reason: no_value_to_read` and a
warning, without entering the text again, which could enter it twice.
`type` reads nothing back.

On iOS, `fill` into a field without the focus while no keyboard is up taps
the field itself first, then enters the text once the replica shows the
keyboard's window (an input-method window that cuts the screen, as
`Mirror::uncovered` has it), at most 1 s after the tap (design D34): in the
React Native smoke test the first keys typed into a just-focused field were
lost in 3 of 8 runs while the Simulator's keyboard slid in, and spike S3
measured the slide-in at about 1.2 s, the keyboard's window showing in the
tree before the animation ends. A paste is Command-V, a key the keyboard
takes like the others, so the wait holds whichever way the text goes in. A
field with the focus, or a keyboard already up, pays nothing, and the tap
obeys the click's rule for an element out of view. The driver's own tap,
which follows, finds the field focused. Android sets text without typing
and does not wait.

An identifier that names a node that takes no text but holds exactly one
field (`Mirror::field_within`, design D39) sends `fill` and `type` into
that field, with a warning naming it and its ref: in the acceptance run of
2026-10-12 `fill` and `type` by the identifier of a React Native code
input's view failed `invalid_argument` on the step's second visit (4 of
4), while a click on the identifier and a `type` worked. A field with an
area takes a tap of its own, as any field does; one without, which a tap
cannot reach, takes the tap on the node around it unless it has the focus,
and the text as the field in focus. An identifier on a node that takes no
text, holds no field (`Mirror::fields_within`) and has no ref sends `fill`
and `type` to the input a tap on it focuses (design D44): on the third
acceptance run's second visit the code input's view held nothing, the
app's input not in the tree, and `fill` and `type` by its identifier
failed `invalid_argument` while a click on it raised the keyboard and an
untargeted `type` landed the code. The node is tapped where a tap reaches
it, the keyboard's window awaited up to 1 s on either platform, and the
text goes untargeted to the input in focus, `fill` reading it back
through the node found again by its identifier, its value, descendants or
name; the reply's `ref` is null and a warning names the node and says the
keyboard it brought up carried the text. No keyboard after the second, or
a field the tree shows with the focus after the tap, which the keyboard
would serve instead, fails `invalid_argument` as before; a node holding
several fields fails at once. A field whose label
changes once it holds text keeps its ref across the change, and its view
its identity when it gains a counter at the focus (`crates/mirror/README.md`,
Snapshots), so a `fill` by that ref reads the field back, where in the
acceptance run it failed `stale_ref` before anything was typed.

`open` waits for the launched app's first tree before it settles: until the
replica shows the app in front with more than its root and more than a
launch splash, read after the launch returned when the app was relaunched,
since a read before that can still show the process the relaunch stopped. A
splash lists nothing interactive in the app's windows, and in the full
listing nothing at all (the Expo fixture's window holds an empty React
Native root view for a few hundred ms after its first read), a single image
(Bluesky's listed only `image [id=SplashScreenLegacy]` for seconds), or a
single element whose identifier or class says splash; the full listing is
taken only when the interactive one lists nothing in the app's windows.
When only a splash showed within the app's budget, `open` warns that it
did. Whatever the driver, the launch is noted to the mirror as a change of
the window in front (`Mirror::note_window_change`), so every window is read
again and the settle counts the launch as a tree change: it waits for an
app that draws within its quiet window of the launch, even one already in
front. `close` notes the stop the same way. The app gets the waiting
commands' default timeout (the session's, else 10000 ms) to show it, as the
Compose fixture's 500-cell screen needs on an emulator, where the launch
takes 1 to 2 s and the cold read 1.5 s; the screen then gets 3000 ms to
settle (design D8). An explicit `timeout` bounds both together. Between the
two, on a target with a frame source, it waits for the frame stream's first
frame, at most 2 s from its start: the first frame is no change, so a click
whose effects came before it would report no frame change, and an
emulator's stream delivered its first frame up to 500 ms after the attach
(a Simulator's comes with the subscription). It starts the target's input
session, when its driver has one, before the launch, and lets it activate,
at most 2 s from its start, so the first click does not wait for it. It
reports `settled` false, with a warning, when the app did not show in time
or its screen did not settle, and warns when no frame came.

## Tap spans and settle signals

The mirror times what happens on a target (task 7.10): `Mirror::timestamps`
gives the last frame change (the capture time of the latest frame whose
hash differs from the frame before), the last tree change (a tree or window
event's arrival, a noted window change, or the end of a read that found a
delta), the last read and the last input it injected, and
`Mirror::frame_changes_since` the frame changes since an instant. Items
still queued in the event stream count once drained, so the daemon calls
`Mirror::drain`, which applies them without reading the tree, before it
looks. The mirror's tree deltas (`Mirror::deltas`) supply the rest.

A click brings the replica current, notes the replica's revision, and
dispatches the tap; `dispatchMs` runs until the input is accepted. It then
runs the mirror in 5 ms slices, so windows are re-read as they would be for
a snapshot, draining before each look, until the first frame change after
the dispatch (on targets with a frame source) and the first delta of a
later revision have both arrived, until nothing has changed for the 200 ms
quiet window since the tap or the last change, or for 1 s. When the
driver's input notes say the tap waited for its input transport to
activate, the first tap of a new input session, the watch waits up to 1.5 s
for the first change and lasts up to 2.5 s: on the spike Simulator the
first tap after dtuhidd activated changed the first frame about 202 ms and
the tree about 458 ms after its dispatch, just past a warm watch's 200 ms.
`frameChangeMs` and `treeDeltaMs` run from the dispatch to the frame's
capture and to the delta's read, and are omitted when the signal never
came; `watchedMs` says how long the watch lasted. A watch that fails after
the tap went in becomes a warning. So does a tap on an element that was not
mostly in view as `scroll --until` has it (task 10.15), naming the point
the tap went to and saying to bring the element in first. The tap goes to
the centre of the part of the element a tap reaches (`Visibility::reachable`,
design D37): its bounds within what shows of them, and within what each
scroll container around it shows (design D45: on the third acceptance run
a tap on a control's strip under a footer drawn over its list opened
nothing), less every view drawn over part of it, the largest rectangle left that is at least 8 points on
each side, or as long as the element where it is shorter. The centre of
its bounds may lie on what covers it: on iOS, whose frames are whole, the
centre of a row the tab bar covers lies on the tab bar, and in the
acceptance run of 2026-10-12 a tap at the centre of a card 70% under a
sticky header pressed the header. A ref, an identifier and a text selector
take this one path: the mirror aims a ref's tap itself, and a selector's
element without a ref is aimed at the same point here, as is the tap with
which `fill` focuses its field. An element nothing of which a tap reaches
in view is not tapped at those bounds (design D32): in the React Native smoke test the keyboard moved a submit button up
without an event, the replica kept the button's old bounds, under the
keyboard, before a frame-driven re-read fell due, and the tap went to the
number pad there. The click reads the windows again once (the mirror
reads a window on demand only after text, so every window, as each read of
a Simulator's reader does anyway) and taps the element where that read
has it, or, when nothing of it a tap reaches is in view still, fails with
`element_offscreen` before any tap, naming the element as given with its
role and name, what keeps it out of view (the keyboard, its list, its
window) and how to bring it in (close the keyboard, `scroll --until`);
`data.ref` and `data.visible`, 0. `click` makes no stroke of its own: a
row the keyboard covers inside a list is refused the same way, and `scroll
--until` brings it in. A covered element is no matter of bounds out of
date: an element under a modal, under a view drawn over all of it as the
mirror tells from the tree's order on iOS (design D31), such as a button
wholly under a fixed footer and so also 0% in view, or with less than 8
points a side left from under the views, bars and disabled controls drawn
over part of it (design D38), fails with `element_covered` as the mirror
names what a tap at its centre would land on (`Mirror::covered_by`),
before any read again and any tap. The keyboard covers only once that read
still has it over the element, since its coming up moves a layout without
an event. `scroll --until` judges arrival by the same part: an element
mostly under a view drawn over it is not mostly in view, where in the
acceptance run it called a control under a disabled button visible 1.0.

A settle wait (`wait --settle`) ends one quiet window after the latest of
the last tree change (event, noted window change or delta), the last frame
change and the last input the mirror injected, as the mirror's timestamps
give them. `endedBy` names the signal whose change that was: `tree`,
`frame`, or `both` when their last changes came within 20 ms; `none` when
the input came later than any change or the quiet window had already passed
when the wait began; `timeout` when the wait reached its maximum, with
`stillChanging` naming what changed in the last quiet window. `--tree-only`
leaves frames out, as it does for the mirror. On a target without frames
whose tree source can probe, an Android agent from 0.3.0 on, the mirror's
settle also reads a few anchor nodes' bounds again every 50 ms (task
10.11, `crates/mirror/README.md`), and does not settle while the tree has
an application window the platform draws away from where its layout puts
it (the agent's `transformed` window flag), as it draws a window it
animates in or out. So `open` and `wait --settle` wait out a screen that
moves without a change event, and an app's window as the platform scales
or slides it in, and end with the replica holding where the screen came to
rest; a move a probe finds is a tree change. Without frames, a settle
after a `scroll` or `swipe` stroke that leaves the list under it at an end
of its range also holds for 500 ms after the stroke and after the tree's
last change (task 10.14): from Android 12 the list draws a stretch there
that springs back with no event, and until it rests it takes a tap on a
row for the start of a drag, so a click sent right after a settle that
ended sooner opens nothing.

## Reading the tree after the display changed

On a window that sends no change events (every Simulator window, Compose on
Android) the mirror re-reads after input, when frames change and on its
poll, and a frame-driven re-read comes no sooner than its minimum interval
(200 ms) after the previous read ended. A click's watch lets the mirror run
the re-read the tap calls for at once, which on the Simulator caught the
SwiftUI fixture mid-switch (the Big tab's cells gone, the Login screen not
there yet); the frames that changed next then call for a read that falls
due after the click has replied. So `snapshot`, `diff snapshot` and every
ref or selector argument first bring the replica up to the display
(`Attached::catch_up`): when a frame changed after the latest tree read
began, as the mirror's timestamps show once what arrived is drained, they
wait for that re-read, at most the minimum interval plus 100 ms. Targets
without frames, and targets whose windows all send change events, never
wait; a warm snapshot of a still screen is served from the replica. A
screen that keeps changing its frames, such as a focused field's blinking
cursor, can make a tree read wait for one re-read, since the worker's own
pumps keep re-reading such a screen between commands.

A physical iPhone's frames come slowly, screenshots polled about 19 times a
second, and its tree read costs about half a second, so its attachment sets
the mirror's `slow_frames` (design D23): the re-read a tap calls for waits
in the mirror for the tap's first changed frame, at most 500 ms, rather than
read the screen before the change and leave the change to a frame-driven
re-read a minimum interval after it; settle waits at least two captures
without a change and, after input, for that first changed frame too. With
its frames subscribed, a still screen is polled every 10 s rather than every
second (design D26), since frames that change call for a re-read of their
own: polled every second, the half-second reads kept the target's worker
reading when 47 to 60% of an idle phone's snapshots arrived
(`reports/11.1-ios-device.md`, finding 2).

## Sessions and claims

A target command claims its target for the request's session when nobody
holds it. Another session is refused with `target_claimed`, naming the
holder in the message and as `data.holder`, unless it passes `force`, which
moves the claim. `close` stops the app the session's `open` launched and
releases the claim. Claims last until then or until the daemon exits.

Refs are isolated between sessions: a session acts only on refs its own
snapshots (and waits and diffs) listed on the target; any other ref fails
with `unknown_ref`. A session's first `snapshot --delta` on a target starts
again in full when another session took the mirror's last delta snapshot.
Each session keeps its last snapshot per target as the baseline of `diff
snapshot`, and its default timeout.

## Target selection

`device` is an id as `devices` lists it, else a name one target alone has,
compared without case; `platform` is `ios` or `android`. The daemon picks
the target they name; else the only connected target of the platform; else
the one of those targets the session already holds. Several without a
choice fail with `ambiguous_target`, a `device` naming none with
`target_not_found`, and no target with `no_target`; the first two list the
candidates' ids, platforms and names in the message and as
`data.candidates`. A target the daemon holds attached is resolved from its
attachment, with no driver asked, when `device` names it by its id, or when
no `device` is given and it is the one target of the platform the session
holds (design D26 of physical-ios): an iPhone's listing probes lockdown and
the keychain, so a command after a pause paid 150 to 220 ms for it with the
target attached all along. Otherwise target listings are reused for 2 s,
and an unknown `device` asks the drivers again; `devices` always does,
each driver first forgetting what it keeps between listings (`Driver::forget`),
as the iPhone driver forgets the signing identity it checked.

## Timeouts and idle exit

A waiting command (a wait for an element or text, or a click, fill, type,
get or scroll on a selector) waits for its own `timeout`, else its
session's default (`set_timeout`), else the client's `defaultTimeout`, else
10000 ms. `wait` with `settle` and `open` wait at most 3000 ms (design D8)
unless given `timeout`; `open` first waits for the launched app's first
tree up to the waiting default. Timeouts are at most 600000 ms.

The idle timer runs from the end of the last request and stands still
while any request is in flight, so a long wait outlasts a short idle
timeout.

## Shutdown

`shutdown` answers once the daemon has let go of everything it held. The
daemon accepts no more connections, stops every target it attached, all at
once, which stops an Android target's agent and a Simulator's reader, and
waits for them up to 5 s; then it removes its socket, closes the listener,
removes its version, pid and lock files, and only then replies and exits.
A client that has the reply finds neither the daemon's socket nor its
agents, so the next command starts a fresh daemon, whose attach finds the
device's UI automation free. While the targets stop, the listener stays
open without accepting: a client that connects meanwhile waits, then is
turned away once the socket is gone, rather than finding no daemon and
starting another beside targets still held. A target command that reaches
the target registry once the targets are stopped is refused with
`daemon_stopping`, so the daemon attaches nothing on its way out. The idle
exit stops in the same order, with no reply to send.

A target still stopping after 5 s is left to finish on its own: the daemon
exits all the same, logs it, and the reply names it in `notStopped` and in
a warning (`notStopped` is absent when every target stopped):

```json
{"id":"r1","success":true,"data":{"notStopped":["emulator-5554"],"stopped":true},"error":null,"warning":"emulator-5554 had not stopped within 5 s, and the daemon stopped waiting; an agent it started there may still run"}
```

## Drivers

`Driver` is the factory the registry takes backends from:

```rust
pub trait Driver: Send + Sync {
    fn name(&self) -> &str;
    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError>;
    fn attach(&self, target: &TargetInfo, options: &AttachOptions)
        -> Result<Attachment, DriverError>;   // options.force: the command's --force
    fn compatibility(&self) -> Vec<CompatibilityRow> { Vec::new() }
}

pub struct Attachment {
    pub backend: mirror::backend::Backend,
    pub apps: Box<dyn AppControl>,           // launch and terminate apps
    pub display: Box<dyn Screen>,            // its size in points now, for gestures no element anchors
    pub config: MirrorConfig,
    pub capture: Option<Box<dyn Capture>>,   // screenshots the driver takes now
    pub report: Option<Arc<dyn Report>>,     // what status shows from the driver
    pub input_notes: Option<Arc<dyn InputNotes>>,  // whether the last input waited for an activation
    pub input_session: Option<Box<dyn InputSession>>,  // starts the input session ahead of the first input
}
```

A `TargetInfo` carries the target's `state` as its platform names it and,
for a state that keeps it from automation, `advice`. The registry spawns
a worker thread per target on its first command; the worker attaches,
owns the mirror, runs commands in order, pumps the mirror between them,
and detaches when the daemon stops: dropping the `Attachment` detaches, so
a driver stops what it started for the target when its parts are dropped.
It also detaches a target whose connection is gone, once a command fails
`disconnected`, after its reply, or a pump between commands does, so that
the next command attaches again, which starts over (design D26 of
physical-ios): the physical iPhone's driver reports so of a runner whose
connection fell silent and whose restart failed, which a worker that kept
the attachment left failing `disconnected` until the daemon stopped, with
the silent runner still on the phone (`reports/11.1-ios-device.md`,
finding 5). The new attach ends a runner left behind first. Every
driver's `disconnected` detaches, a locked iPhone's too, which then
attaches again once unlocked; `status` counts each target's attaches
(`attaches`).
`status` asks each attached target's `Report` outside the registry's
lock; `screenshot` takes the driver's `Capture` when there is one and
the mirror's latest frame otherwise.

`driver::registered` is the registration point. A daemon registers the
device drivers this host runs by default, the Simulator driver on macOS and
the Android driver everywhere, unless fake targets are named, which stand
in for them (`--drivers` names drivers to register beside fakes, or
`none`). The physical iPhone driver registers on macOS only when
`--drivers` names `ios` (`devices::OPT_IN`): with the team exported, its
listing ran `security` for the runner's signing identity on every listing,
45 ms at p50 and 53 ms at p95 with no phone attached (84 ms the first
time), and probes each phone through lockdown, 163 ms at p50 with the SE
attached, while a phone whose lockdown never answers holds the listing for
the probe's 10 s timeout, the target registry waiting on it (2026-10-10,
task 5.1, a throwaway program against a fake usbmuxd socket and the real
one, load about 4). The identity is checked once per daemon and kept while
it is found, checked again after a check that failed or when `devices`
asks, each `security` command ended after 5 s (design D26). Discovery merges every driver's targets,
device drivers first; the Simulator and iPhone drivers fill `doctor`'s
Xcode compatibility table through `Driver::compatibility`, and a driver adds
fields of its own to `doctor`'s reply through `Driver::doctor`, as the
iPhone driver's `iosRunner`.

### The Simulator driver

`devices/simulator.rs` opens CoreSimulator on first use (about 330 ms,
which a daemon that never lists a Simulator does not pay) and lists the
booted Simulators of the default device set, state `booted`. Attaching
subscribes to the display, reads the tree once (which spawns the reader and
gives the screen's size in points) and puts the driver behind the backend
traits:

- tree: every read covers the foreground, through the reader spawned into
  the Simulator or the host translator, whose verdict names the fallback;
  the failures a launching app causes (`no_foreground`, `snapshot_failed`,
  `foreground_changed`, `busy`) are retried for 5 s. Every window sends no
  change events, so the mirror re-reads after input, on frames and on its
  poll.
- events: the display's frames, their hash the thumbnail below the status
  bar. A launch or a stop through `open` or `close` is noted to the mirror
  by the daemon, as for every driver. The subscription only hashes:
  encoded frames and screenshots are read from the display at the size
  asked for.
- input: taps, long presses and swipes in points through the host's
  transport (dtuhidd, else Indigo); iOS key codes (`keys.rs`) are keyboard
  usages with the HID modifier byte, or consumer-page buttons (Home, Power,
  VolumeUp, VolumeDown, Siri). Text for a field taps it, waits up to 1.5 s
  for an editable node to take the focus and, unless keys were asked for
  (`type`), selects what the field holds with Command-A so the text
  replaces it, and for an empty text deletes it with the delete key, as
  it does for typed text whose first letter is a capital, which then waits
  up to 1 s for a read that shows the field empty: typed over the
  selection, its first word was rewritten by autocorrection at the first
  space, the capital lost (task 4.5 of `rn-app-fixes`).
  Direct text is pasted, text left to the driver typed up to 32 characters
  and pasted beyond; a paste reports the method `direct`, typing `keys`. The
  input session starts during `open` (`InputSession`): the driver's
  `open_input` opens it without any input, which sends dtuhidd its
  activation probe, and `open` lets it activate, at most 2 s from its
  start, so the first click does not wait for dtuhidd (Bluesky's first
  click took 957 ms to dispatch while it activated). That click is still
  noted as the session's first, with no activation wait, so its watch
  waits longer for its effects.
- apps: launched with the launch environment (`--env K=V` reaches the app
  as `K`, as `SIMCTL_CHILD_K` does through `simctl`), a running app brought
  to the front unless `--relaunch`; `app_not_found` when `simctl` finds no
  container for the bundle.
- status: the input transport and why, its activation time and
  connections, or the host's plan before the first input; the tree backend
  and the reader with its pid, path, state and readiness time; the frames
  received.

Dropping the attachment resets the target in the driver, which stops its
reader and closes its input session; the frame subscription ends with the
mirror.

`status` for a Simulator after a few taps:

```
input: dtuhidd (CoreSimulator 1171.7 injects dtuhidd, as every CoreSimulator from 1155.4 does; Indigo stands in only if dtuhidd cannot be reached), activated in 22.9 ms, 1 connection
tree: simulator-reader: simulator-reader runs in the Simulator
reader: simulator-reader 0.1.0, pid 26561, ready, ready 283.4 ms after its spawn, 1 spawn, from ~/Library/Caches/dev.mobileagent.simulator/simulator-reader-f95422e26c0f566b
frames: 194 received from the display
```

are its lines; the JSON fields beside them are `input`, `inputReason`,
`activationMs`, `connections`, `inputPlan`, `tree`, `treeReason`, `reader
{name, pid, path, readyMs, state, spawns}`, `framesReceived`, `frameError`
and `unavailable {call, failure, host}`.

### The physical iPhone driver

`devices/ios.rs` puts the driver of `crates/ios` (design D18 to D25) behind
the contract, as that crate's README maps its traits. It opens the driver
on first use and lists every iPhone attached over USB, in the `ios-device`
class, its state the readiness code: `ready`, or the first state it has not
passed (`not_trusted`, `developer_mode_off`, `disk_image_unmounted`,
`device_locked`, `no_signing_identity`, `unsupported_ios`), the state's
message from `crates/ios` as the advice, which names the step. Its `os` is
`iOS <version>` and its `model` the `ProductType`, such as `iPhone14,6`.
Attaching connects the device (its readiness, the developer disk image,
the CoreDevice tunnel; a phone that is not ready fails with its state's
code), starts the runner `agent-ios` builds, of the workspace's
`agent-ios/VERSION`, through the test manager, reads the screen's pixels
and scale from lockdown, and subscribes to the screen:

- tree: every read is the runner's snapshot of the foreground, in pixels,
  put into points by the screen's scale (2 on the SE); `window_changed`, a
  foreground that changed during the read, is retried for 5 s. Every
  window sends no change events, so the mirror re-reads after input, when
  frames change and on its poll, every 10 s while the frames hold still.
  The verdict's backend is `ios-runner`.
- events: frames from screenshots polled on DVT and CoreDevice, about 19 a
  second, a frame each time a pixel below the status bar moves more than
  two levels (design D23), its time the capture's arrival; encoded frames
  are those the stream holds, scaled. A poller whose captures fail opens its
  connection again once and, failing again, is left out, the other polling
  alone (design D26 of physical-ios). The mirror's `slow_frames` is set for
  a capture every 52.6 ms (Reading the tree after the display changed);
  without a subscription the mirror re-reads every minimum interval, as on
  any target without frames.
- input: taps, long presses, swipes (a resting release creeps on as UIKit
  needs), hardware buttons and keys through the disk image's `dtuhidd`
  from the host where the device's iOS takes its reports (iOS 18 up to
  27), else through the runner's synthesis; `MOBILE_AGENT_IOS_INPUT` set to
  `dtuhidd` or `runner` takes one way alone, `auto` (the default) by the
  iOS. Key codes are keyboard usages with the HID modifier byte or the
  consumer page's Home, Power, VolumeUp and VolumeDown. Text goes through
  the runner into the field with keyboard focus: a field a ref names is
  tapped first, unless the latest read shows it focused, and the text waits
  up to 1.5 s for an editable node to take the focus; `keys` types it,
  `direct` sets the field's value, and text whose method is left to the
  driver is typed into an empty field and set into one that holds text, so
  it replaces what the field held, the runner setting it where typing
  fails. The reply names the method the runner used.
- apps: launched through DVT's process control with the launch environment
  (`--env K=V` reaches the app as `K`); `--relaunch` ends the running
  instance first; a running app otherwise keeps its process.
  `app_not_found` for an app that is not installed.
- screenshots come from DVT's screenshot channel, or from CoreDevice's
  capture once DVT's failed where CoreDevice's served, at the long edge
  asked for, as JPEG.
- status: the input path (`dtuhidd` or `runner`) and why, the last input's
  acknowledgement, text through the runner; the frames, `screenshots` with
  the connections still polled, their capture rate and the count received,
  a poller that reopened its connection or was left out named with why, or
  why the screen could not be subscribed to; the runner, its agent, state,
  restarts, how long its
  hello took and the runners of a lost session its start ended, and in
  `lastDeath` why it was last started again, `it stalled: ...` for a
  runner that stopped answering while its connection lived; the tunnel;
  the disk image; the runner bundle; the battery as the attach's probe
  read it from lockdown, which the harness records in a run's provenance.
- doctor: `iosRunner`, the runner bundle an attach installs (`bundle`) and
  the version its hello must name (`version`); the compatibility table's
  device rows, the supported range (iOS 17.4 up to but not including 27,
  validated on 26.6.2 under Xcode 27.0) then every call site of
  `ios::call_sites` with the iOS and Xcode it was validated on, `validated`,
  or `supported` where it was not exercised.

The runner bundle an attach installs, when the phone's runner is missing or
says hello with another version, is `agent-ios/dist/MobileAgentRunner-Runner.app`
in the workspace this daemon was built in, as the Android driver finds its
jar, or the one `MOBILE_AGENT_IOS_RUNNER_APP` names, a relative path taken
from the workspace's root; a failure about the runner while no bundle is
built there names `just agent-ios-build`. Dropping the attachment releases
the runner, which ends its process through DVT's process control, a
stalled runner's included, asking it nothing, and closes what the driver
opened for the device.

`status` for the SE after an `open` (2026-10-10, the identifier and the home
directory cut):

```
input: dtuhidd (iOS 26.6.2 takes dtuhidd's reports from the host without a display stream); text through the runner, typed or set
frames: screenshots polled on DVT and CoreDevice, 19.8 captures a second, 11 received
runner: mobile-agent-ios-runner 0.2.0, ready, 0 restarts, hello 765 ms after its start
tunnel: 82 RSD services, none missing, opened in 65.5 ms, 1 open
disk image: Personalized at /System/Developer
runner bundle: ~/<workspace>/agent-ios/dist/MobileAgentRunner-Runner.app
```

are its lines; the JSON fields beside them are `input`, `inputReason`,
`lastInputMs`, `frames`, `captureRate`, `pollers [{connection, captures,
reopens, leftOut}]` (`dvt`, then `coreDevice`), `framesReceived`, `frameError`,
`runner {agent, state, restarts, handshakeRetries, replaced, leftovers,
startedMs, lastDeath}`, `tunnel {opens, openedMs, services, missing}`,
`diskImage {type, path}`, `runnerBundle` and `battery {levelPct, charging,
externalPower}`, null when the probe read none; with one, a line such as
`battery: 100%, not charging, on external power, when attached`, the SE's
on 2026-10-10, follows the runner bundle's. The fake iPhone reports 76%,
charging, on external power.

### The Android driver

`devices/android.rs` lists every target the local adb server knows, in
every state (`device`, `offline`, `unauthorized`, `no permissions`...),
with the driver's advice for an unusable one; without an adb server there
are no Android targets, and doctor reports the server. Attaching refuses a
target whose state is not `device` (`target_not_ready`, with the advice),
deploys the agent jar (`MOBILE_AGENT_JAR`, else the one `agent-android/build.sh`
wrote), reusing the agent this daemon or its predecessor started when it
is compatible (records in `android-agents/`), replacing an incompatible
one, and refusing another holder of UI automation (`automation_conflict`);
selects the emulator's controller when it answers; and starts the agent
session, whose backend the mirror runs on. With `--emulator-controller
off` it never looks for the controller: taps, swipes and frames take the
agent's path on every emulator, as on a device (task 10.12). Emulator
37.1.11 serves its token-protected controller even when started without
`-grpc`, and only `-ports` leaves it off (`reports/10.1-emulator.md`,
finding 10), so the setting is how a daemon measures or uses the agent's
path on an emulator that serves the controller. The display's size is read
whenever a gesture no element anchors needs it, from the `cur=` size of
`dumpsys window displays`, which follows the current rotation (`wm size`
reports the natural orientation's, the fallback).

- apps: `open` launches by package through its launcher activity, as the
  launcher does (`am start -W -a MAIN -c LAUNCHER -n <activity>`), each
  `--env K=V` a string extra `--es K V` of the same name, so `--env
  initial_tab=big` opens the fixture on its Big tab; `--relaunch` adds
  `-S`. `app_not_found` for a package that is not installed or has no
  launcher activity. `close` force-stops it.
- screenshots come from the agent, upright in every display orientation.
- status: the session's report, the agent and the path of each kind of
  input and output, and where the jar came from (`agentJar`: `embedded`, or
  the path `MOBILE_AGENT_JAR` named) with its build (`agentBuild`). An
  emulator's report carries the daemon's `emulatorController` setting, and
  when it is off, the line `emulator controller: off, so taps, swipes and
  frames take the agent's path, as on a device (...)`.

Dropping the attachment stops the agent (`shutdown`, then a kill past
2 s) and forgets its record, so `shutdown`, an idle exit and a detach
after a failed command leave no agent behind. An agent whose session
cannot start after the attach, such as when the display density cannot be
read, is stopped from its record the same way before the attach fails. A daemon killed before it
could stop its agent leaves the record, and its successor reuses the agent
if it still runs.

`status` data for an emulator started with `-grpc 8554 -grpc-use-token`:

```json
{"agent": "mobile-agent-android 0.1.0", "controllerError": null, "disconnected": null,
 "endpoint": "http://127.0.0.1:8554", "frameError": null, "frames": "emulator controller", "framesReceived": 67,
 "input": "emulator controller", "inputs": {"agent": 0, "controller": 1}, "keyboard": false, "keys": "agent",
 "lines": ["emulator-5554: mobile-agent-android 0.1.0 over protocol 1, pid 12499, socket mobile-agent-0.1.0-p1-d34aa73c8d57-8de2e87454f6, launched",
           "input: emulator controller at http://127.0.0.1:8554 (taps, long presses and swipes)",
           "frames: emulator controller, 67 received", "screenshots: agent",
           "keys and text: agent; the AVD declares no keyboard, so the controller would drop keys"],
 "pid": 12499, "protocol": 1, "reused": false, "screenshots": "agent",
 "socket": "mobile-agent-0.1.0-p1-d34aa73c8d57-8de2e87454f6", "warning": null}
```

### The fake driver

`MOBILE_AGENT_FAKE_TARGETS` (or `--fake`) lists `platform[:id]` entries:
`android` alone becomes `emulator-5554`, then `emulator-5556`; `ios` alone
`FAKE-IOS-1`, then `FAKE-IOS-2`; `ios-device` alone, a physical iPhone's
stand-in, `FAKE-IOS-DEVICE-1`, then `FAKE-IOS-DEVICE-2`; an Android id not
starting `emulator-` is a device. Each target is the mirror's scripted `FakeTarget` behind a
real-time adapter, whose clock follows the system clock so waits, polls and
settle windows take real time, and a device model:

- a launcher (`com.android.launcher3` or `com.apple.springboard`): the
  heading "Home" and the buttons Fixture and Settings;
- the fixture app of `fixtures/CONTRACT.md` under `dev.mobileagent.fixture.expo`
  and the platform's native id: login (with `hunter2`), a 200-row list with
  details and scrolling, 500 cells, an endless animation that keeps frames
  changing, an in-app alert and a location prompt owned by system UI, with
  the initial tab honored at launch as each platform's fixtures read it:
  `--env FIXTURE_INITIAL_TAB=<tab>` on iOS, `--env initial_tab=<tab>` on
  Android;
- a settings app with a Wi-Fi switch;
- the placeholder app `dev.mobileagent.placeholder`, whose screen
  `--env PLACEHOLDER_SCREEN=<name>` picks at launch, under placeholder
  names: `consent`, a heading half under the secondary one of two footer
  buttons listed after the content and a 12-point text under the primary
  one; `creation`, a list under a header listed before it, which a stroke
  scrolls and which says which ways it scrolls, a disabled button drawn
  after it over most of a control and a footer drawn after that over the
  list's lower part and the rest of the control, and `creation-open`, the
  same without the footer;
  `feed`, a card partly above its list's top, where the header listed
  before the list is drawn; `list`, a header listed before a list down to
  122 points and a field in the list that brings the keyboard up, and
  `list-low`, the header down to 430 points, which leaves 109 points of
  the list above the keyboard; `code`, a code field whose name spells the
  digits it holds, and `code-again`, the field without an area inside an
  identified node, as on the step's second visit; `code-named`, the code
  field named by its boxes, each its digit or a dash, before its label,
  `code-masked`, a bullet for each digit, and `code-bare`, its node holding
  no field, a tap on it focusing an input outside the tree, as the third
  acceptance run met the step on its two visits (design D43, D44); `sheet`, a field without
  an identifier whose view gains a counter at the focus and whose name
  changes with its text, in the structure of the acceptance run of
  2026-10-12 (design D37 to D41); and by default the form with the shapes
  a React Native app showed in the smoke test of 2026-10-09: a heading; `placeholder-code`, a code field built of
  six boxes, a labelled node without traits holding a text per character;
  `placeholder-secret`, a field the tree shows as a labelled node without
  traits, no value and nothing below it, as iOS reads a React Native field
  inside a view marked accessible, which takes text and never shows it;
  `placeholder-status`; `placeholder-hint`, a button the keyboard, once up,
  covers all but its top 14 points of; `placeholder-submit`, low on the
  screen, which the layout moves above the keyboard 50 ms after the
  keyboard came up, without an event and in no frame, so that the replica
  keeps its old bounds until its next read (design D32);
  `placeholder-help`, under the keyboard; `placeholder-terms` at the
  bottom; and, once the hint is tapped, `placeholder-footer`, a toolbar
  holding `placeholder-accept`, listed after the terms button and drawn
  over it, as a fixed footer is. A tap on the keyboard enters a 5 into the
  field in focus, as its number pad does.

Android fakes use widget classes, a status bar, a keyboard window (from
600 points down, while a field has focus) and dialog windows, and
send window events; a stroke that comes down on the keyboard goes to the
keyboard and moves nothing, its glide typing entering "by" into the field
in focus, as Gboard did on a Nexus 5X (task 10.32); the fixture's list
reports a row it cuts off with bounds clipped to the list, as Android
does, and its scroll semantics as the agent reads a RecyclerView's
(vertical, back and forward as far as it scrolls, its rows as items). iOS
fakes use `XCUIElementType` classes and send no events, so the mirror
re-reads them as it re-reads a Simulator, and show what the Simulator's
reader reports (task 6.10):
SpringBoard's window, system UI owned by the process name `SpringBoard`,
holds the home screen and the location prompt, so `origin` reads
`SpringBoard` there; an in-app alert sits, with the view presenting it, in
a modal window of its own above the app's (the app window's id with bit 30
set, titled by the alert, active and focused in the app window's stead),
which the mirror names `dialog "In-app alert"` in an `element_covered`
error; while a field has focus the keyboard's window sits above the app's
as the reader gives it (the app window's id with bit 31 set, the app's
package, bounded by what the keyboard draws: from its prediction bar at
539 points down, task 10.21), and a stroke that comes down on it types as
on Android. The keyboard comes up with the focus; the tests' fakes can
have it come up some time after the focus, an iOS field losing the text
that reaches it sooner, typed or pasted, or never (`Slide`). Their states
are `device`
and `booted`; they have neither a capture nor a report. A fake iPhone
(`ios-device`) is an iOS fake in the `ios-device` class, on iOS 26.6.2,
`ready`, whose frames are the screenshots a physical iPhone's are and whose
`status` is the physical iPhone driver's report of a phone whose runner
said hello: its input through `dtuhidd`, its frames `screenshots` at 19 a
second with the count delivered, the runner `mobile-agent-ios-runner` of
`agent-ios/VERSION` with no restart. Its frames show only at its captures,
every 52.6 ms from the attach, a change and each frame of a transition or
of the Anim tab's spinner at the capture that follows it, and its
attachment sets the mirror's `slow_frames` as the physical driver's does;
`FakeDriver::with_device_frames(false)` takes its frame source away, as a
phone whose screenshots could not be subscribed to, its `status` saying
so. `FakeDriver::with_readiness` lists one
in a readiness state with its advice, `crates/ios`'s message, and an
attach to it then fails with the state's code, as the driver's does. Each
attach of a fake iPhone starts a runner process of its own, ending first
the ones a lost session left on the phone, which its `status` names among
the runner's leftovers (`FakeDriver::runners_on` lists the processes);
`FakeDriver::with_lost_runner(after)` loses the first attach's runner
connection that long after the attach, its restart failing, as a physical
iPhone's fell silent in the measured run: that attachment's tree reads and
input then fail `disconnected` with the physical driver's message, its
`status` names the runner failed with no restart, and its process stays
on the phone until the next attach ends it. `FakeDriver::stall_runner(id)`
stalls the runner of the phone's attachment, as a physical iPhone's
stalled on the 43rd emptying fill of every session in physical-ios task
9.6: the next tree read or text entry, which go through the runner, fails
as the physical driver fails the call that finds a runner stalled,
`runner_stalled` at the first stall since the attach, its `status` then
counting a restart with the stall as `lastDeath`, and `disconnected` from
the second, the session failed, whose release ends the stalled runner;
taps, swipes and keys go through `dtuhidd` and meet no stall.
Emulators, Simulators and fake iPhones have a frame source; Android devices
have none, as design D7 has it, so `screenshot` answers them with
`no_frame`. Every change shows
after `--fake-latency`, together with its window event and frame; a move to
another screen (another app, tab, detail, alert or prompt) animates, its
frames changing every 16 ms for 96 ms after the tree changed, then settling
on the screen's final frame. A relaunch takes the running app off the
screen at once and shows the new one after the latency, as a relaunched
app's stopped process leaves before the new one draws. Frames are produced
as the backend is called, not on a timer. With `--fake-frame-delay` the
frame stream starts that long after the attach, as an emulator's does once
its gRPC subscription is up: frames captured before it are never delivered,
and its first frame shows the screen as it is then. With `--fake-splash` a
fixture app whose process starts (a first launch or a relaunch) shows its
launch splash, one image `SplashScreenLegacy` in its window, that long
before its first screen. With `--fake-activation` the target has an input
session that input waits for until it has been started that long, started
by the first input or by `open`, and input notes that name the session's
first input, as a Simulator's. With `--fake-stop` detaching a target takes
that long on its worker, as stopping the Android agent does, so `shutdown`
waits that long for it. With `--fake-fling` a fixture's list glides on
after a stroke whose finger lifts moving, as far as its platform's fling
carries it at the finger's speed: Android's `OverScroller` spline with
`ViewConfiguration`'s scroll friction, from 50 points a second, and UIKit's
normal deceleration rate; a 600-point stroke over 300 ms glides 647 points
on Android and 998 on iOS, which puts the lists where the emulator's and
the Simulator's went (1,103 to 1,319 and 1,522 to 1,608 points). A stroke
that comes to rest before the finger lifts, and every stroke without the
flag, moves the list as far as the finger. With `--fake-typing keys` the
fixture's fields take a `fill` of up to 32 characters key by key and report
the method `keys`, as a Simulator does, and longer text whole as a paste,
`direct`; with `reorder` they take those keys out of order, each from the
fourth on landing before the field's last character, so `fill nobel` leaves
"noelb", as Bluesky's search field did on the Simulator
(`reports/10.4-rn-smoke.md`, third run, finding 3); with `max:<n>` a field
keeps its first `n` characters, as a field with a maximum length does.
Direct text always goes in whole, and `type` key by key after what the
field holds. Screenshots are placeholder
bytes, not an image. An Android fake reports its paths in `status` as the
Android driver names them: a fake emulator's input and frames go through
the `emulator controller`, a fake device's input through the `agent` with
no frames. With `--emulator-controller off` a fake emulator has no frame
source and reports the agent's paths, as a real emulator does.

## Errors

| Code | When |
|---|---|
| `invalid_request` | the line is not a JSON object, or looks like HTTP |
| `invalid_argument` | a parameter is missing, of the wrong type or out of range |
| `unknown_action` | the daemon has no such action |
| `no_target` | no target (of the platform) is connected |
| `ambiguous_target` | several targets and no choice; `data.candidates` |
| `target_not_found` | `device` names no connected target; `data.candidates` |
| `target_claimed` | another session holds the target; `data.holder` |
| `app_not_found` | the target has no such app |
| `element_not_found` | no element matched a selector within the timeout, or `scroll` with `until` stopped without the element mostly in view; `data.strokes`, `data.until`, `data.from` and `data.to` for the last stroke, and `data.visible` for an element on screen |
| `input_rejected` | the target refused the input, such as text for a non-field |
| `stroke_too_short` | a scroll's strokes would be no longer than the platform's tap threshold (8 points on Android, 10 on iOS); `data.stroke`, `data.tapSlop` |
| `covered_by_keyboard` | the keyboard covers what a scroll strokes, but for too little for a stroke longer than the tap threshold or less than 120 points along the scroll's axis, and a stroke that came down on it would type; the message names what it covers and how to close the keyboard; `data.strokes` |
| `text_mismatch` | `fill` left other text in the field than it entered, typed or pasted and then entered again in one piece, as its value or its descendants show it; the message names what the field holds (quoted, as a number of masked characters, or nothing) and the text entered (quoted, or by its length when the field shows nothing or mask characters); `data.ref`, `data.value` (null for nothing), `data.masked` |
| `unknown_key` | the platform has no code for the key |
| `io_error` | a file could not be written |
| `daemon_stopping` | the daemon is shutting down, and a target command reached it after it stopped its targets; run the command again to start a new daemon |
| `internal` | a command failed unexpectedly; the worker detaches the target |

A command that fails `disconnected`, a code the mirror and the drivers
give, detaches its target too, after the reply (see Drivers).

The mirror's codes pass through: `stale_ref`, `unknown_ref`,
`element_covered`, `element_offscreen`, `wait_timeout`, `no_frame`,
`disconnected`, and drivers' own codes. `click` also fails with
`element_offscreen` itself, for an element nothing of which a tap reaches
is in view after its window was read again (see Tap spans and settle signals). The Simulator driver's keep its
names (`unsupported_xcode`, `target_unavailable`, `no_foreground`,
`no_input_transport`, `framework_error` and the rest in
`crates/simulator/src/error.rs`). The physical iPhone driver's keep theirs
too (`crates/ios/src/error.rs`): a readiness state's code for a phone that
is not ready (`device_locked`, `developer_mode_off`, `disk_image_unmounted`,
`not_trusted`, `no_signing_identity`, `unsupported_ios`,
`automation_not_enabled`), `disconnected` with the cause, `runner_start_failed`,
`runner_version_mismatch`, `invalid_bundle` and the rest, an app that is
not installed being `app_not_found` and a runner that died and could not be
started again (the driver's `runner_died`) `disconnected`, its message
kept, so that the worker detaches the target; a call the driver failed
because the runner stalled, its connection alive (two calls in a row
answered `busy` or `deadline_exceeded`, or none answered for 30 s), keeps
its `runner_stalled`, which can be repeated: the runner is started again,
a tree read retries it and waits for the restart, and the target stays
attached, while a second stall in the session is `runner_died`, so
`disconnected`; the adapter adds `invalid_argument` for a
`MOBILE_AGENT_IOS_INPUT` other than `auto`, `dtuhidd` or `runner`. The
Android adapter's are:

| Code | When |
|---|---|
| `target_not_ready` | the target's state is not `device`; the message carries the advice |
| `automation_conflict` | another process holds the device's UI automation, likely the one named; the command with `--force` takes it over |
| `agent_failed` | the agent jar could not be read, or the agent did not start or complete its handshake |
| `agent_mismatch` | the jar reports another version than this daemon deploys; rebuild it |
| `adb_failed` | the adb server or the device failed a request |
| `launch_failed` | `am start` could not open the app |

## Interpretations of the spec

- *Sessions* isolate refs: besides claims, a session may use only the refs
  its own snapshots listed on the target.
- *Target selection*: a session that holds one of several connected targets
  keeps addressing it without a flag, as agent-browser's sessions stay on
  their browser; a session that holds none fails listing the candidates.
  Connected targets are the booted Simulators and every target the adb
  server lists; a Simulator that is shut down is no target, and an adb
  target in another state than `device` is listed with its state and
  refused when a command needs it.
- *Daemon lifecycle*: the version file names the build as well as the
  release, so a rebuilt daemon is replaced as a new release is.
- *Configurable timeouts*: settle keeps the 3 s bound of design D8 rather
  than the session default, and timeouts are capped at 10 minutes so the
  CLI can size its reply wait.

## Gaps in the mirror's API

Worked around here without changing `crates/mirror`:

- `Mirror::delta_snapshot` returns neither the verdict nor the foreground,
  so a delta reply's `origin` and `verdict` come from a plain snapshot taken
  just before it: one more listing per delta request.
- Refs are issued only by `Mirror::snapshot`; `Mirror::wait` returns an
  element without a ref when no snapshot listed it, so text entry by
  selector takes an interactive snapshot to obtain one.
- `Mirror::snapshot` computes `removed_refs` against the last snapshot
  anyone took, and `Mirror::wait` takes one on timeout for its summary, so
  `removedRefs` is per target, not per session.
- The backend has no app lifecycle, screen size, on-demand screenshot or
  status; the driver contract adds `AppControl`, `display`, `Capture` and
  `Report`. Fake targets without a frame source answer `screenshot` with
  `no_frame`.
- The mirror schedules a frame-driven re-read no sooner than its minimum
  interval after the previous read and serves tree reads from the replica
  meanwhile, so the worker waits for that re-read before a tree read
  (`Attached::catch_up`), timing it with the mirror's timestamps.
- `testing::FakeTarget` runs on a fake clock and its tap reactions are
  fixed per identifier; the fake driver adds a real-time adapter and a
  stateful device model. Its `inputs()` copies the whole history, so the
  adapter hit-tests taps itself.
- The mirror reads one window on demand only after text it entered, so a
  click on an element nothing of which a tap reaches is in view reads its
  window again with
  `note_window_change` and `pump`, which reads every window and counts as a
  tree change before the tap (the click's spans count only what follows
  the tap); a Simulator's reader reads every window at each read anyway.
- `Element` carries no children, so `fill`'s read-back through a field's
  descendants takes them from a full listing, by depth: one listing per
  look while the field shows no value of its own, which issues refs as any
  snapshot does, and misses descendants the listing folds into a name.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Sessions | Claimed target | `claims::a_claimed_target_refuses_another_session_naming_the_holder`, `claims::force_takes_the_target_over_and_the_previous_holder_is_refused`, `claims::every_target_command_claims_an_unclaimed_target`, `claims::a_released_target_serves_another_session_without_force`, `claims::the_holder_keeps_using_its_target`, `claims::status_names_the_holder_of_each_target`, `claims::a_request_without_a_session_uses_the_default_session` |
| Daemon lifecycle | (idle exit, configurable) | `idle::the_daemon_exits_after_its_idle_timeout_and_removes_its_files`, `idle::requests_postpone_the_idle_exit`, `idle::an_idle_timeout_of_zero_disables_the_exit`, `idle::the_idle_timeout_defaults_to_one_hour`, `idle::idle_timeouts_take_the_units_agent_browser_takes`, `idle::a_daemon_whose_socket_file_was_removed_still_exits_when_idle` (task 8.14) |
| Daemon lifecycle | (version file, replacement, socket) | `lifecycle::the_daemon_writes_its_pid_and_version_files`, `lifecycle::a_daemon_of_another_version_is_shut_down_and_replaced`, `lifecycle::a_second_daemon_of_the_same_version_leaves_the_first_serving`, `lifecycle::files_left_by_a_killed_daemon_do_not_block_a_new_one`, `lifecycle::shutdown_stops_the_daemon_and_removes_its_files`, `lifecycle::the_socket_lives_under_the_runtime_directory`, `lifecycle::an_overlong_socket_path_is_refused_with_a_hint`; task 8.14: `lifecycle::shutdown_completes_after_the_socket_file_was_removed_and_leaves_no_lock_directory`, `server::tests::an_exiting_daemon_removes_the_lock_unless_a_startup_holds_it`, `server::tests::a_startup_takes_the_lock_again_after_an_exit_removed_its_file` |
| Output formats | (envelope and codes) | `lifecycle::every_reply_follows_the_agent_browser_envelope`, `lifecycle::malformed_and_http_looking_requests_are_refused_with_a_code`, `wire::tests::replies_follow_agent_browser_field_order` |
| Command grammar | Interactive snapshot output | `lifecycle::a_snapshot_of_the_fake_launcher_lists_a_heading_and_two_buttons`, `lifecycle::devices_lists_every_fake_target_with_its_platform_and_class`, `lifecycle::open_launches_an_installed_app_and_refuses_an_unknown_one` |
| Command grammar | (keys, element arguments) | `keys::tests::*`, `handlers::aim::tests::arguments_are_refs_identifiers_texts_or_roles` |
| Command grammar | (task 8.10: CLI gaps the harness found) | the CLI's `commands::an_exact_text_selector_matches_the_whole_name_or_value_only`, `commands::scroll_until_moves_the_content_until_the_element_is_on_screen`, `commands::open_relaunch_reports_settled_only_once_the_relaunched_app_shows_its_screen`, `commands::status_names_whether_each_attached_target_has_a_frame_source`, `snapshot::a_snapshot_says_how_long_the_daemon_took_and_whether_the_replica_served_it`; `handlers::aim::tests::text_double_equals_is_an_exact_text_selector` |
| Command grammar | (task 8.8: the device drivers end to end) | `devices::devices_lists_the_simulator_and_the_emulator_with_platform_and_state`, `devices::the_swiftui_fixture_is_driven_through_the_cli_on_a_simulator`, `devices::the_compose_fixture_is_driven_through_the_cli_on_an_emulator`, `devices::doctor_shows_the_xcode_compatibility_table_the_simulator_driver_fills` |
| (task 8.8, registration) | (device drivers, fakes) | `driver::tests::every_device_driver_the_host_runs_registers_unless_fake_targets_are_named`, `driver::tests::named_drivers_register_alone_or_beside_fake_targets`, `driver::tests::registration_puts_device_drivers_before_the_fake_one`, `server::tests::the_drivers_flag_names_the_device_drivers_to_register` |
| (task 8.8, Simulator adapter) | (listing, keys, errors, doctor table, status, display, text) | `devices::simulator::tests::*` |
| (task 8.8, Android adapter) | (listing, readiness, display, launch, status, errors) | `devices::android::tests::*` |
| Device discovery and readiness, spec `ios-device-driver` (task 5.1 of physical-ios) | Ready device listed; Device not ready | `devices::ios::tests::a_ready_iphone_is_listed_with_its_name_model_and_ios_version`, `devices::ios::tests::an_unready_iphone_is_listed_in_its_state_with_the_step_as_advice`, `fake::tests::ios_device_fakes_take_default_ids_and_the_ios_device_class`, `fake::tests::an_unready_fake_iphone_is_listed_in_its_state_and_refuses_an_attach`, `ios_device::devices_lists_a_fake_iphone_in_the_ios_device_class`; the CLI's `doctor::each_iphone_is_listed_in_its_readiness_state_with_the_daemons_step`; on the phone, `devices::the_swiftui_fixture_is_driven_through_the_cli_on_an_iphone` |
| One resident runner per session, spec `ios-device-driver` (task 5.1) | Runner stays up (the status names the runner's agent and restarts) | `devices::ios::tests::the_status_names_the_input_path_the_frames_with_their_rate_and_the_runner`, `fake::tests::a_fake_iphone_reports_dtuhidd_screenshots_and_its_runner`, `ios_device::status_names_a_fake_iphones_input_frames_and_runner`; the battery the attach read, for the harness's provenance (task 6.2): `devices::ios::tests::the_status_names_the_battery_the_attach_read` |
| Compatibility statement, spec `ios-device-driver` (task 5.1) | Table lists the device paths | `devices::ios::tests::the_compatibility_rows_are_the_supported_range_then_every_call_site`, `ios_device::a_daemon_with_the_ios_driver_names_its_runner_bundle_and_the_device_rows`; the CLI's `doctor::the_iphone_checks_need_macos_and_the_device_rows_fill_the_table` |
| Frames and screenshots from the host; Foreground tree read (windows without events), spec `ios-device-driver` (tasks 5.2 and 9.1 of physical-ios, designs D23 and D26) | Frame change after a tap (settle on slow frames, with and without them; the re-reads of windows without events and their still poll) | `worker::tests::on_a_fake_iphone_a_snapshot_right_after_a_tap_shows_what_the_tap_did`, `worker::tests::a_fake_iphones_windows_are_re_read_on_frame_changes_and_polled_when_still`, `worker::tests::an_idle_fake_iphone_with_still_frames_is_not_polled_where_others_are`, `worker::tests::settle_on_a_fake_iphone_holds_for_the_first_changed_frame_after_a_tap`, `worker::tests::settle_on_a_fake_iphone_without_frames_re_reads_every_minimum_interval`, `fake::realtime::tests::a_fake_iphones_frames_come_at_the_devices_rate`; the mirror's `replica::with_slow_frames_*` and `settle::with_slow_frames_*`; on the phone, `devices::the_swiftui_fixture_is_driven_through_the_cli_on_an_iphone`; a screenshot poller left out or reopened named in `status` (task 9.8): `devices::ios::tests::the_status_names_a_poller_left_out_of_the_frames_beside_the_capture_rate` |
| (task 5.1, the iPhone driver's registration and adapter) | (`ios` registered only when named, the runner bundle, the input variable, errors, keys) | `driver::tests::the_ios_driver_registers_only_when_named`, `driver::tests::registration_puts_device_drivers_before_the_fake_one`, `devices::ios::tests::the_runner_bundle_is_the_built_one_or_the_one_the_variable_names`, `devices::ios::tests::the_input_variable_takes_auto_dtuhidd_or_runner`, `devices::ios::tests::driver_errors_keep_their_code_and_retryable_flag`, `devices::ios::tests::keyboard_codes_become_keys_and_consumer_codes_buttons`; the driver's screen in `crates/ios`, `macos::driver::tests::the_screen_is_the_devices_pixels_and_scale_from_lockdown` |
| Replica maintained from device events | (task 8.8: a tree read after frames changed) | `worker::tests::a_tree_read_waits_for_the_re_read_that_changed_frames_call_for`, `worker::tests::a_tree_read_does_not_wait_when_no_frame_changed_since_the_last_read`, `worker::tests::a_target_without_frames_never_waits` |
| (task 8.9, harness spans) | (tap spans, settle signals) | `signals::tests::a_wait_that_did_not_settle_timed_out`, `signals::tests::the_signal_that_changed_last_ended_the_wait`, `signals::tests::changes_within_20_ms_of_each_other_ended_it_together`, `signals::tests::a_change_at_the_action_counts_but_older_ones_do_not`, `signals::tests::a_screen_quiet_before_the_wait_began_ended_on_nothing`; end to end in the CLI's `spans::*` |
| (task 8.13, tap-length strokes and the first frame) | (scroll refusals, `open` priming the frame stream) | the CLI's `commands::scroll_refuses_strokes_the_platform_takes_for_a_tap`, `spans::the_first_click_after_open_reports_its_first_frame_change`; `handlers::act::tests::a_scroll_splits_into_equal_strokes_that_fit`, `handlers::act::tests::strokes_the_platform_takes_for_taps_are_refused`, `fake::realtime::tests::a_late_frame_stream_delivers_nothing_before_it_starts_then_the_screen_shown`, `server::tests::flags_override_the_environment_and_zero_disables_the_idle_exit`; against the emulator, `devices::the_compose_fixture_is_driven_through_the_cli_on_an_emulator`'s first click |
| (task 8.15, launch splash and input session) | (`open` waits past a splash, activates the input session) | the CLI's `commands::open_waits_past_a_launch_splash_for_the_app_s_first_screen`, `spans::the_first_click_after_open_does_not_wait_for_the_input_session`; `fake::realtime::tests::a_launch_shows_the_splash_before_the_first_screen`, `fake::realtime::tests::input_waits_for_the_session_to_activate_unless_it_started_ahead`; against the Simulator, `devices::the_expo_fixture_shows_its_screen_not_its_splash_right_after_open` and `devices::the_swiftui_fixture_is_driven_through_the_cli_on_a_simulator`'s first click |
| (task 8.12, the mirror's timing) | (spans, settle signals and catch-up from `Mirror::timestamps`, `frame_changes_since` and `drain`; launches and stops noted with `note_window_change`) | the CLI's `commands::open_and_close_are_window_changes_that_a_settle_waits_out`; with the outputs of the task 8.9 rows, the catch-up rows above and the CLI's `spans::*` and `commands::*` unchanged |
| (task 10.11, settle without frames) | (`open` waits out a screen moving without change events and a window the platform animates in) | the mirror's `settle::without_frames_settle_waits_until_probed_bounds_hold_still_and_the_replica_has_them` and `settle::without_frames_settle_waits_while_a_window_is_drawn_transformed`, and the Android session's `session::tests::a_settle_without_frames_probes_the_agent_and_reads_the_moved_window_evicted`; against an emulator on the agent path, `devices::open_on_the_agent_path_waits_for_the_expo_screen_to_come_to_rest` |
| Failure containment, spec `ios-device-driver` (physical-ios task 9.3, design D26) | Runner connection lost (on the fake iPhone) | `worker::tests::a_lost_runner_connection_detaches_the_target_and_the_next_command_attaches_again`, `worker::tests::a_lost_runner_connection_a_poll_finds_detaches_the_target_before_the_next_command`, `devices::ios::tests::a_runner_that_died_and_could_not_start_again_is_disconnected`, `ios_device::status_names_a_fake_iphones_input_frames_and_runner` (`attaches`); the driver's side in `crates/ios`, `macos::containment::a_call_in_flight_when_the_runner_falls_silent_fails_the_session_until_the_next_attach` and `macos::runner::tests::a_runner_that_falls_silent_is_ended_and_started_again_once` |
| Failure containment, spec `ios-device-driver` (physical-ios task 9.7, design D26) | (a runner that stalls, its connection alive, on the fake iPhone: kept attached at the first stall, detached at the second, ended by the release and the stop) | `worker::tests::a_fake_iphones_stalled_runner_is_started_again_and_a_second_stall_detaches_the_target`, `devices::ios::tests::a_call_the_stalled_runner_failed_can_be_repeated_and_a_second_stall_is_disconnected`; the driver's side in `crates/ios`'s README (task 9.7's row) |
| (physical-ios task 9.2, design D26: an attached target resolved without a listing; `devices` lists afresh) | (a command on an attached target asks no driver; a target not attached is listed; `devices` tells the drivers to forget, then lists) | `registry::tests::an_attached_target_is_resolved_without_a_listing_and_devices_lists_afresh`; the iPhone driver's cached signing identity in `crates/ios`, `macos::driver::tests::the_signing_identity_is_checked_once_and_again_after_a_failure_or_when_forgotten` |
| (task 8.16, shutdown waits for the targets) | (`shutdown` answers once the targets have stopped and the files are gone, bounded by 5 s) | `lifecycle::shutdown_answers_once_the_targets_have_stopped_and_the_socket_is_gone`, `lifecycle::a_target_still_stopping_after_5_s_is_named_in_the_shutdown_reply`, `registry::tests::stopping_names_the_targets_still_detaching_when_the_wait_is_over`, `registry::tests::a_stopped_registry_starts_no_worker`, `server::tests::flags_override_the_environment_and_zero_disables_the_idle_exit`; the CLI's `lifecycle::shutdown_returns_once_the_daemon_has_stopped_its_targets_and_removed_its_socket` and `lifecycle::a_command_right_after_shutdown_starts_a_fresh_daemon_without_meeting_the_old_socket` |
| (task 10.12, the emulator controller switch) | (the setting leaves emulators' controllers unused; `status` and `doctor` report it) | `controller::the_switch_puts_an_emulators_input_and_frames_on_the_agent_path`, `controller::without_the_switch_an_emulator_takes_the_controllers_path`, `controller::the_flag_beats_the_variable_and_a_value_other_than_auto_or_off_stops_the_start`, `server::tests::the_emulator_controller_flag_takes_auto_or_off`, `driver::tests::the_emulator_controller_setting_is_auto_or_off`, `devices::android::tests::the_controller_setting_off_puts_an_emulator_on_the_agent_path`, `fake::tests::the_controller_setting_off_puts_fake_emulators_on_the_agent_path`; against an emulator that serves the controller, `devices::the_controller_switch_leaves_a_served_controller_unused` |
| (task 10.14, overscroll without frames) | (a click right after the settle that follows a stroke to the end of a list opens the row) | the mirror's `settle::without_frames_a_stroke_that_pulls_a_list_at_its_end_holds_the_wait_for_the_overscroll`, `settle::without_frames_a_stroke_that_leaves_a_list_at_its_end_holds_the_wait_for_the_overscroll`, `settle::without_frames_a_fling_into_a_list_s_end_holds_the_wait_from_its_last_change` and their controls; against an emulator on the agent path, `devices::the_bench_s_list_scroll_flow_opens_the_last_row_on_the_agent_path` |
| (task 10.15, `scroll --until` stops once the element is mostly in view) | (a sliver of the element on screen does not stop the scroll; `visible` in the reply) | `handlers::act::tests::a_scroll_that_looks_for_an_element_stops_once_it_is_mostly_in_view`, `handlers::act::tests::a_partly_shown_element_is_brought_in_by_one_stroke_no_longer_than_the_step`, `handlers::act::tests::no_stroke_brings_in_an_element_the_scroll_s_axis_does_not_cut_off`, `handlers::act::tests::a_stroke_that_brings_an_element_in_moves_the_finger_at_most_400_points_a_second`; the mirror's `visibility::*`; the CLI's `commands::scroll_until_stops_only_once_the_element_is_mostly_in_view`, `commands::scroll_until_fails_when_its_axis_cannot_bring_a_partly_shown_element_in` and `commands::a_click_on_an_element_mostly_out_of_view_warns`; against an emulator on both of its paths, `devices::scroll_until_stops_only_once_the_row_is_mostly_in_view_on_an_emulator` |
| (task 10.16, `scroll --until` does not carry its element past unseen) | (every stroke comes to rest before the finger lifts; a stroke that looks for the element goes at most three quarters of what the list shows; fake lists that fling) | `handlers::act::tests::a_stroke_that_looks_for_an_element_goes_at_most_three_quarters_of_what_its_list_shows`, `fake::app::tests::a_list_glides_on_as_far_as_its_platform_flings_it`, `fake::app::tests::a_swipe_moves_the_list_by_the_finger_and_the_glide`, `server::tests::flags_override_the_environment_and_zero_disables_the_idle_exit`; the mirror's `visibility::*` and `settle::a_stroke_reaches_the_target_released_as_the_action_says`; the CLI's `commands::scroll_until_finds_a_row_that_a_flinging_stroke_would_carry_through_the_list` and `commands::scroll_until_strokes_come_to_rest_so_a_row_stops_where_they_aim`; against devices, `devices::scroll_until_finds_a_mid_list_row_on_an_emulator` and `devices::scroll_until_finds_a_mid_list_row_on_a_simulator` |
| (task 10.23, `fill` reads the field back) | (keys a field takes out of order are pasted over; a field that keeps other text fails with `text_mismatch`; secure fields by length; text that shows late; empty fills; the Simulator's empty fill and direct text; fake fields that take text as a Simulator types it) | `handlers::act::tests::a_fill_whose_keys_a_field_takes_out_of_order_pastes_the_text`, `handlers::act::tests::a_fill_that_a_field_cuts_short_fails_naming_what_the_field_holds`, `handlers::act::tests::a_secure_field_is_read_back_by_the_number_of_its_characters`, `handlers::act::tests::a_fill_replies_once_the_field_shows_its_text`, `handlers::act::tests::an_empty_fill_reads_the_field_back_empty_except_on_android`, `handlers::act::tests::a_field_shows_the_text_exactly_as_its_mask_or_as_an_empty_placeholder`, `devices::simulator::tests::an_empty_text_deletes_what_the_field_held`, `devices::simulator::tests::keys_are_typed_direct_text_is_pasted_and_a_paste_reads_as_direct_setting`, `fake::app::tests::fields_take_text_as_their_typing_mode_says`, `fake::realtime::tests::text_is_typed_as_the_simulator_types_it_only_under_a_typing_mode_that_types_keys`, `fake::tests::typing_modes_are_named_whole_keys_reorder_or_max_with_a_length`; the CLI's `commands::fill_reads_the_field_back_and_pastes_text_whose_keys_went_in_out_of_order`, `errors::a_fill_whose_field_keeps_other_text_fails_with_text_mismatch` |
| (task 10.30, the hold only after keys) | (text set or pasted in one piece taken at the first read that shows it, typed text after the 200 ms hold, on either platform; the text entered again after other text not held) | `handlers::act::tests::a_fill_whose_text_went_in_in_one_piece_takes_the_first_read_that_shows_it`, `handlers::act::tests::a_fill_typed_as_keys_takes_the_text_once_the_field_has_held_it_200_ms`, `handlers::act::tests::a_fill_that_enters_its_text_again_in_one_piece_takes_the_first_read_that_shows_it` |
| (task 10.32, scroll strokes keep off the keyboard) | (strokes through what the system bars and the keyboard leave of the screen and of a list partly under it; `covered_by_keyboard` before any stroke; the fake's keyboard takes a stroke for glide typing) | `handlers::act::tests::a_scroll_of_the_screen_strokes_above_the_keyboard`, `handlers::act::tests::a_scroll_inside_a_list_partly_under_the_keyboard_strokes_through_what_shows_of_it`, `handlers::act::tests::a_scroll_inside_an_element_the_keyboard_covers_fails_naming_the_keyboard`, `fake::app::tests::a_stroke_that_comes_down_on_the_keyboard_types_and_moves_nothing`; the mirror's `visibility::a_stroke_goes_through_what_the_system_bars_and_the_keyboard_leave_of_its_area` and `visibility::the_screen_loses_what_the_windows_above_the_one_in_front_cover` |
| Single coordinate contract (design D32) | Submit button moved by the keyboard | `handlers::act::tests::a_click_on_a_button_the_keyboard_moved_taps_it_where_it_is_now`, `handlers::act::tests::a_click_on_a_button_the_keyboard_still_covers_fails_naming_it_and_taps_nothing`, `handlers::act::tests::a_click_on_a_button_partly_in_view_taps_its_centre_with_a_warning`, `handlers::act::tests::a_row_under_the_keyboard_is_refused_by_click_and_brought_in_by_scroll_until`, `handlers::act::tests::a_click_on_a_button_a_footer_is_drawn_over_fails_covered_without_a_read_or_a_tap` |
| Text entry verification (design D33) | Composite code field; Value that cannot be read back; Mismatch | `handlers::act::tests::a_fill_of_a_code_into_a_field_of_boxes_is_verified_through_their_texts`, `handlers::act::tests::a_fill_into_a_field_with_no_value_to_read_succeeds_unverified`, `handlers::act::tests::a_fill_whose_field_reads_back_other_text_after_the_retry_fails_naming_both` |
| Single coordinate contract (design D37, D38) | Card partly under a sticky header; Heading half under a footer button (the tap on the part a tap reaches with its warning, by ref, identifier and text selector; `scroll --until` judging arrival by that part) | `handlers::act::tests::a_click_on_a_card_partly_above_its_list_taps_the_part_in_view_with_a_warning`, `handlers::act::tests::clicks_on_what_footer_buttons_cover_in_part_tap_what_shows_or_are_refused_naming_the_button`, `handlers::act::tests::scroll_until_does_not_call_a_control_mostly_under_a_disabled_button_visible`, `handlers::act::tests::a_click_on_a_button_partly_in_view_taps_the_part_in_view_with_a_warning`; the mirror's rows of these scenarios |
| (design D41, strokes inside the reachable container) | (with the keyboard up a scroll strokes below a pinned header and above the keyboard; under 120 points it fails `covered_by_keyboard`) | `handlers::act::tests::a_scroll_with_the_keyboard_up_strokes_inside_the_list_below_its_header`, `handlers::act::tests::a_scroll_of_a_list_the_keyboard_leaves_less_than_120_points_of_fails_naming_it` |
| Text entry verification (design D39) | Code field that names its digits; Identifier on the field's wrapper; Ref across a label change | `handlers::act::tests::a_fill_of_a_code_into_a_field_whose_name_spells_it_is_verified_through_the_name`, `handlers::act::tests::fill_and_type_by_the_identifier_of_a_fields_wrapper_go_into_the_field`, `handlers::act::tests::a_fill_by_ref_of_a_field_whose_label_changes_reads_it_back` |
| Text entry verification (design D43, D44) | Name with a fixed suffix; Node without a field that raises the keyboard | `handlers::act::tests::a_name_shows_the_text_in_the_part_the_entry_changed`, `handlers::act::tests::a_fill_of_a_code_into_a_field_named_by_its_boxes_compares_what_the_entry_changed`, `handlers::act::tests::fill_and_type_by_the_identifier_of_a_node_that_raises_the_keyboard_go_to_the_input_it_focuses`, `handlers::act::tests::an_identifier_on_a_node_whose_tap_raises_no_keyboard_still_takes_no_text` |
| Single coordinate contract (design D45, D46) | Strip clipped by the scroll view (the click by identifier refused naming the disabled button); a scroll that stops naming its last stroke | `handlers::act::tests::a_click_on_a_control_whose_strip_below_a_disabled_button_lies_under_a_footer_is_refused`, `handlers::act::tests::scroll_until_on_the_creation_tab_strokes_the_way_a_plain_scroll_moves_the_list`, `handlers::act::tests::scroll_until_strokes_toward_a_side_the_list_can_carry_the_control_to`, `handlers::act::tests::a_scroll_until_that_stops_names_its_last_stroke_and_what_moved`; the mirror's rows of these scenarios |
| Text entry, spec `ios-device-driver` (task 5.3 of physical-ios) | (fill names text set through a physical iPhone's runner, not a paste) | `handlers::act::tests::a_fill_on_an_iphone_says_it_set_the_text_rather_than_pasted_it`; the CLI's `help::the_agent_guide_says_where_a_physical_iphone_differs` |
| Text entry, spec `simulator-driver` (design D34) | First keystroke lands | `handlers::act::tests::a_fill_on_ios_types_into_a_field_it_focused_once_the_keyboard_is_up`, `handlers::act::tests::a_fill_on_ios_pastes_into_a_field_it_focused_once_the_keyboard_is_up`, `handlers::act::tests::a_fill_on_ios_waits_at_most_1_s_for_a_keyboard_that_never_comes`, `handlers::act::tests::a_fill_on_ios_into_a_field_with_the_focus_and_the_keyboard_up_does_not_wait`, `handlers::act::tests::a_fill_on_android_neither_taps_first_nor_waits_for_the_keyboard`, `fake::app::tests::an_ios_field_loses_text_while_its_keyboard_slides_in`; against a Simulator, ten fills by hand (task 1.4 of rn-app-fixes) |
| Host microbenchmarks (spec `benchmark-harness`) | (the input of `benches/daemon.rs`: a warm interactive snapshot of the fixture's Big tab lists its 500 cells and five tabs, each with a ref, and is served from the replica) | `bench_inputs::a_warm_snapshot_of_the_big_tab_lists_its_500_cells_and_five_tabs_with_refs` |

The CLI's tests (`crates/cli/README.md`) cover every scenario end to end
through this daemon, on fake targets with the device drivers off
(`MOBILE_AGENT_DRIVERS=none`).

```sh
cargo test -p daemon
```

`tests/devices.rs` drives real targets through the built `mobile-agent`
binary, a daemon of its own per test on a private socket directory, one
test at a time. The Simulator tests run when `MOBILE_AGENT_SIM_UDID` names
a booted Simulator with the SwiftUI and Expo fixtures installed, the
Android test when `MOBILE_AGENT_ANDROID_SERIAL` names an emulator (or
device) with the Compose fixture installed and the agent jar built; without
them each test prints why it skipped and passes. The Simulator is left
with the Expo fixture on its Login tab and the SwiftUI fixture stopped; the
Compose fixture is stopped, and the Android test first waits up to a minute
for the shared device to have no agent and nothing holding UI automation.
Each flow prints what it measured.

```sh
eval "$(mise env)"   # the Android SDK
MOBILE_AGENT_SIM_UDID=<udid> MOBILE_AGENT_ANDROID_SERIAL=emulator-5554 \
  cargo test --release -p daemon --test devices -- --nocapture
```
