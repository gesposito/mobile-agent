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
contract, the device drivers (the Simulator driver of `crates/simulator`
and the Android driver of `crates/android` behind the mirror's backend
traits) and the fake driver every test without a device runs on.

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
  devices/      the device drivers: simulator (Simulator driver), android (Android driver)
  fake/         the fake driver: target list, device model, real-time adapter, trees
tests/          claims, controller, idle and lifecycle, over the socket against the binary;
                devices, the CLI against a real Simulator and emulator
```

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
| `--drivers` | `MOBILE_AGENT_DRIVERS` | every device driver the host runs: `simulator` (macOS only) and `android`; none when fake targets are named. Comma-separated names, `ios` for `simulator`, or `none` |
| `--emulator-controller` | `MOBILE_AGENT_EMULATOR_CONTROLLER` | `auto`: taps, swipes and frames on an Android emulator go through its gRPC controller when it answers. `off` sends them through the agent on every emulator, as on a device, with no frame stream; fake emulators then have no frame source either. Any other value is a usage error |
| `--fake` | `MOBILE_AGENT_FAKE_TARGETS` | none |
| `--fake-latency` | `MOBILE_AGENT_FAKE_LATENCY_MS` | `0`; how long fake targets take to show a change, in the same forms |
| `--fake-frame-delay` | `MOBILE_AGENT_FAKE_FRAME_DELAY_MS` | `0`; how long after the attach fake targets' frame streams start, in the same forms |
| `--fake-splash` | `MOBILE_AGENT_FAKE_SPLASH_MS` | `0`; how long fake targets' fixture apps show their launch splash when their process starts |
| `--fake-activation` | `MOBILE_AGENT_FAKE_ACTIVATION_MS` | `0`; how long after it starts a fake target's input session activates |
| `--fake-stop` | `MOBILE_AGENT_FAKE_STOP_MS` | `0`; how long detaching a fake target takes, as stopping the Android agent takes |

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
| `status` | | `daemon {pid, version, socket, uptimeMs, idleTimeoutMs, drivers, emulatorController}`, `targets [{id, platform, class, name, state, attached, frameSource, app, claimedBy, driver}]` (`frameSource` whether the attached target has a frame source, null before it is attached; `driver` while attached: see Drivers), `sessions [{name, targets, defaultTimeoutMs}]` |
| `devices` | `platform` | `devices [{id, platform, class, name, state, model, os, advice, claimedBy}]` (`advice` only for a target its state keeps from automation) |
| `open` | `app`, `relaunch`, `env ["K=V"]`, `timeout` | `opened`, `target`, `settled`, `elapsedMs` (from the launch request until the screen settled), `firstTreeMs` (until the replica showed the app, null when it did not) |
| `close` | | `closed`, `app`, `target`; the claim is released |
| `snapshot` | `interactive`, `maxDepth`, `delta`, `full` | `snapshot` (text, or with `delta` agent-browser's delta object), `refs`, `removedRefs` (without `delta`), `origin`, `readMs` (the target's worker's time to answer, device reads included), `servedFromReplica` (answered without reading the device), `replicaRevision`, `target`, `verdict {backend, fallback, degraded, reasons [{code, advice}]}` |
| `diff_snapshot` | `interactive`, `maxDepth` (without a baseline) | `changed`, `diff`, `additions`, `removals`, `modifications`, `unchanged`, `added`, `removed`, `modified` (elements, with `fields`), `baseReplicaRevision`, `replicaRevision`, `target` |
| `click` | `target` or `x`, `y`; `hold`, `timeout` | `clicked`, `ref`, `x`, `y`, `accepted`, `dispatchMs`, `frameChangeMs` and `treeDeltaMs` when they came, `watchedMs` |
| `fill` | `target`, `text`, `timeout` | `filled`, `ref`, `method` |
| `type` | `text`; `target`, `timeout` | `typed`, `ref`, `method` |
| `press` | `key` | `pressed`, `code`, `meta` |
| `swipe` | `direction`, `distance`, or `from`, `to` as `[x, y]`; `duration` | `swiped`, `from`, `to` |
| `scroll` | `direction`, `amount`, `target`, `until`, `timeout` | `scrolled`, `direction`, `amount`, `strokes`; with `until`, `until` and `element` |
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

`scroll` with `until` scrolls in strokes of `amount` points (at most 80% of
the scrolled area) and lets the screen settle after each, at most 1.5 s, so
a check never races a moving list; it stops once the element is on screen,
not covered and not scrolled away, and fails with `element_not_found` when
two strokes in a row change nothing (the content stopped moving), after 50
strokes, or at its timeout.

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
the tap went in becomes a warning.

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
`data.candidates`. Target listings are reused for 2 s; `devices` and an
unknown `device` ask the drivers again.

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
`status` asks each attached target's `Report` outside the registry's
lock; `screenshot` takes the driver's `Capture` when there is one and
the mirror's latest frame otherwise.

`driver::registered` is the registration point. A daemon registers the
device drivers this host runs, the Simulator driver on macOS and the
Android driver everywhere, unless fake targets are named, which stand in
for them (`--drivers` names drivers to register beside fakes, or `none`).
Discovery merges every driver's targets, device drivers first; the
Simulator driver fills `doctor`'s Xcode compatibility table through
`Driver::compatibility`.

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
  replaces it; a paste reports the method `direct`, typing `keys`. The
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
`FAKE-IOS-1`, then `FAKE-IOS-2`; an Android id not starting `emulator-` is a
device. Each target is the mirror's scripted `FakeTarget` behind a
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
- a settings app with a Wi-Fi switch.

Android fakes use widget classes, a status bar, a keyboard window and
dialog windows, and send window events; the fixture's list reports a row it
cuts off with bounds clipped to the list, as Android does, and its scroll
semantics as the agent reads a RecyclerView's (vertical, back and forward
as far as it scrolls, its rows as items). iOS fakes use `XCUIElementType`
classes and send no events, so the mirror re-reads them as it re-reads a
Simulator, and show what the Simulator's reader reports (task 6.10):
SpringBoard's window, system UI owned by the process name `SpringBoard`,
holds the home screen and the location prompt, so `origin` reads
`SpringBoard` there; an in-app alert sits, with the view presenting it, in
a modal window of its own above the app's (the app window's id with bit 30
set, titled by the alert, active and focused in the app window's stead),
which the mirror names `dialog "In-app alert"` in an `element_covered`
error. Their states are `device`
and `booted`; they have neither a capture nor a report. Emulators and
Simulators have a frame source; Android devices have none, as design D7
has it, so `screenshot` answers them with `no_frame`. Every change shows
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
waits that long for it. Screenshots are placeholder bytes, not an
image. An Android fake reports its paths in `status` as the Android driver
names them: a fake emulator's input and frames go through the `emulator
controller`, a fake device's input through the `agent` with no frames.
With `--emulator-controller off` a fake emulator has no frame source and
reports the agent's paths, as a real emulator does.

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
| `element_not_found` | no element matched a selector within the timeout |
| `input_rejected` | the target refused the input, such as text for a non-field |
| `stroke_too_short` | a scroll's strokes would be no longer than the platform's tap threshold (8 points on Android, 10 on iOS); `data.stroke`, `data.tapSlop` |
| `unknown_key` | the platform has no code for the key |
| `io_error` | a file could not be written |
| `daemon_stopping` | the daemon is shutting down, and a target command reached it after it stopped its targets; run the command again to start a new daemon |
| `internal` | a command failed unexpectedly; the worker detaches the target |

The mirror's codes pass through: `stale_ref`, `unknown_ref`,
`element_covered`, `element_offscreen`, `wait_timeout`, `no_frame`,
`disconnected`, and drivers' own codes. The Simulator driver's keep its
names (`unsupported_xcode`, `target_unavailable`, `no_foreground`,
`no_input_transport`, `framework_error` and the rest in
`crates/simulator/src/error.rs`); the Android adapter's are:

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
| Replica maintained from device events | (task 8.8: a tree read after frames changed) | `worker::tests::a_tree_read_waits_for_the_re_read_that_changed_frames_call_for`, `worker::tests::a_tree_read_does_not_wait_when_no_frame_changed_since_the_last_read`, `worker::tests::a_target_without_frames_never_waits` |
| (task 8.9, harness spans) | (tap spans, settle signals) | `signals::tests::a_wait_that_did_not_settle_timed_out`, `signals::tests::the_signal_that_changed_last_ended_the_wait`, `signals::tests::changes_within_20_ms_of_each_other_ended_it_together`, `signals::tests::a_change_at_the_action_counts_but_older_ones_do_not`, `signals::tests::a_screen_quiet_before_the_wait_began_ended_on_nothing`; end to end in the CLI's `spans::*` |
| (task 8.13, tap-length strokes and the first frame) | (scroll refusals, `open` priming the frame stream) | the CLI's `commands::scroll_refuses_strokes_the_platform_takes_for_a_tap`, `spans::the_first_click_after_open_reports_its_first_frame_change`; `handlers::act::tests::a_scroll_splits_into_equal_strokes_that_fit`, `handlers::act::tests::strokes_the_platform_takes_for_taps_are_refused`, `fake::realtime::tests::a_late_frame_stream_delivers_nothing_before_it_starts_then_the_screen_shown`, `server::tests::flags_override_the_environment_and_zero_disables_the_idle_exit`; against the emulator, `devices::the_compose_fixture_is_driven_through_the_cli_on_an_emulator`'s first click |
| (task 8.15, launch splash and input session) | (`open` waits past a splash, activates the input session) | the CLI's `commands::open_waits_past_a_launch_splash_for_the_app_s_first_screen`, `spans::the_first_click_after_open_does_not_wait_for_the_input_session`; `fake::realtime::tests::a_launch_shows_the_splash_before_the_first_screen`, `fake::realtime::tests::input_waits_for_the_session_to_activate_unless_it_started_ahead`; against the Simulator, `devices::the_expo_fixture_shows_its_screen_not_its_splash_right_after_open` and `devices::the_swiftui_fixture_is_driven_through_the_cli_on_a_simulator`'s first click |
| (task 8.12, the mirror's timing) | (spans, settle signals and catch-up from `Mirror::timestamps`, `frame_changes_since` and `drain`; launches and stops noted with `note_window_change`) | the CLI's `commands::open_and_close_are_window_changes_that_a_settle_waits_out`; with the outputs of the task 8.9 rows, the catch-up rows above and the CLI's `spans::*` and `commands::*` unchanged |
| (task 10.11, settle without frames) | (`open` waits out a screen moving without change events and a window the platform animates in) | the mirror's `settle::without_frames_settle_waits_until_probed_bounds_hold_still_and_the_replica_has_them` and `settle::without_frames_settle_waits_while_a_window_is_drawn_transformed`, and the Android session's `session::tests::a_settle_without_frames_probes_the_agent_and_reads_the_moved_window_evicted`; against an emulator on the agent path, `devices::open_on_the_agent_path_waits_for_the_expo_screen_to_come_to_rest` |
| (task 8.16, shutdown waits for the targets) | (`shutdown` answers once the targets have stopped and the files are gone, bounded by 5 s) | `lifecycle::shutdown_answers_once_the_targets_have_stopped_and_the_socket_is_gone`, `lifecycle::a_target_still_stopping_after_5_s_is_named_in_the_shutdown_reply`, `registry::tests::stopping_names_the_targets_still_detaching_when_the_wait_is_over`, `registry::tests::a_stopped_registry_starts_no_worker`, `server::tests::flags_override_the_environment_and_zero_disables_the_idle_exit`; the CLI's `lifecycle::shutdown_returns_once_the_daemon_has_stopped_its_targets_and_removed_its_socket` and `lifecycle::a_command_right_after_shutdown_starts_a_fresh_daemon_without_meeting_the_old_socket` |
| (task 10.12, the emulator controller switch) | (the setting leaves emulators' controllers unused; `status` and `doctor` report it) | `controller::the_switch_puts_an_emulators_input_and_frames_on_the_agent_path`, `controller::without_the_switch_an_emulator_takes_the_controllers_path`, `controller::the_flag_beats_the_variable_and_a_value_other_than_auto_or_off_stops_the_start`, `server::tests::the_emulator_controller_flag_takes_auto_or_off`, `driver::tests::the_emulator_controller_setting_is_auto_or_off`, `devices::android::tests::the_controller_setting_off_puts_an_emulator_on_the_agent_path`, `fake::tests::the_controller_setting_off_puts_fake_emulators_on_the_agent_path`; against an emulator that serves the controller, `devices::the_controller_switch_leaves_a_served_controller_unused` |
| (task 10.14, overscroll without frames) | (a click right after the settle that follows a stroke to the end of a list opens the row) | the mirror's `settle::without_frames_a_stroke_that_pulls_a_list_at_its_end_holds_the_wait_for_the_overscroll`, `settle::without_frames_a_stroke_that_leaves_a_list_at_its_end_holds_the_wait_for_the_overscroll`, `settle::without_frames_a_fling_into_a_list_s_end_holds_the_wait_from_its_last_change` and their controls; against an emulator on the agent path, `devices::the_bench_s_list_scroll_flow_opens_the_last_row_on_the_agent_path` |

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
