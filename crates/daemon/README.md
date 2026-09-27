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
command. The library holds the server, the target registry, the driver
contract and the fake driver every test runs on.

## Layout

```
src/
  main.rs       the mobile-agent-daemon binary: flags, environment, exit status
  server.rs     startup and replacement, sidecar files, connections, idle exit
  paths.rs      the socket directory, its files, the build's version string
  wire.rs       the request and reply envelope, the error codes
  registry.rs   discovery, target selection, claims, session settings, workers
  worker.rs     one thread per target owning its mirror; per-session state
  handlers/     what each action does: mod (status, devices, open, close, set_timeout,
                doctor), view (snapshot, diff, get, wait, screenshot), act (click,
                fill, type, press, swipe, scroll), aim (refs and selectors)
  keys.rs       key names to Android key codes and iOS HID usages
  driver.rs     the Driver factory contract and the registration point
  fake/         the fake driver: target list, device model, real-time adapter, trees
tests/          claims, idle and lifecycle, over the socket against the binary
```

## Running

```sh
mobile-agent-daemon [--socket-dir <dir>] [--idle-timeout <time>] [--fake <targets>]
```

| Flag | Environment | Default |
|---|---|---|
| `--socket-dir` | `MOBILE_AGENT_SOCKET_DIR` | `$XDG_RUNTIME_DIR/mobile-agent`, else `~/.mobile-agent`, else `mobile-agent` in the temporary directory |
| `--idle-timeout` | `MOBILE_AGENT_IDLE_TIMEOUT_MS` | `1h`; raw milliseconds or a whole number with `s`, `m` or `h`; `0` never exits |
| `--fake` | `MOBILE_AGENT_FAKE_TARGETS` | none |

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
| `daemon.lock` | serializes startups |
| `daemon.log` | stderr of daemons the CLI started, appended |

The directory is created with mode 0700. Startups take the lock. Under it
a new daemon connects to the socket: a daemon whose version file matches
keeps the socket and the new one leaves; one of another version is sent
`shutdown`, then SIGTERM and SIGKILL if it has not finished within 5 s, and
replaced; a socket nobody answers on is stale and removed. The new daemon
writes its pid and version files, then binds the socket, so a client that
reaches the socket finds the version of the daemon behind it. On exit a
daemon removes the socket only if its inode is still the one it bound, and
the version and pid files only if the pid file still names it, so a
successor's files survive.

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

`session` defaults to `default`. `force: true` takes a claimed target over.
`device` and `platform` select the target. `code` and `warning` appear only
when set; `data` carries details on some failures (`holder`, `candidates`).

| Action | Parameters | Data |
|---|---|---|
| `status` | | `daemon {pid, version, socket, uptimeMs, idleTimeoutMs, drivers}`, `targets [{id, platform, class, name, attached, app, claimedBy}]`, `sessions [{name, targets, defaultTimeoutMs}]` |
| `devices` | `platform` | `devices [{id, platform, class, name, model, os, claimedBy}]` |
| `open` | `app`, `relaunch`, `env ["K=V"]`, `timeout` | `opened`, `target`, `settled`, `elapsedMs` |
| `close` | | `closed`, `app`, `target`; the claim is released |
| `snapshot` | `interactive`, `maxDepth`, `delta`, `full` | `snapshot` (text, or with `delta` agent-browser's delta object), `refs`, `removedRefs` (without `delta`), `origin`, `replicaRevision`, `target`, `verdict {backend, fallback, degraded, reasons [{code, advice}]}` |
| `diff_snapshot` | `interactive`, `maxDepth` (without a baseline) | `changed`, `diff`, `additions`, `removals`, `modifications`, `unchanged`, `added`, `removed`, `modified` (elements, with `fields`), `baseReplicaRevision`, `replicaRevision`, `target` |
| `click` | `target` or `x`, `y`; `hold`, `timeout` | `clicked`, `ref`, `x`, `y`, `accepted`, `dispatchMs` |
| `fill` | `target`, `text`, `timeout` | `filled`, `ref`, `method` |
| `type` | `text`; `target`, `timeout` | `typed`, `ref`, `method` |
| `press` | `key` | `pressed`, `code`, `meta` |
| `swipe` | `direction`, `distance`, or `from`, `to` as `[x, y]`; `duration` | `swiped`, `from`, `to` |
| `scroll` | `direction`, `amount`, `target`, `timeout` | `scrolled`, `direction`, `amount`, `strokes` |
| `screenshot` | `path`, `quality`, `maxDimension` | `path`, `format`, `width`, `height`, `quality`, `seq`, `ageMs` |
| `wait` | `ms`; or `settle`, `quiet`, `treeOnly`, `timeout`; or `target` or `text`, `state`, `timeout` | `waited` (`time`, `settle`, `selector`, `text`); settle: `settled`, `elapsedMs`, `stillChanging`, `treeChanges`, `frameChanges`; elements: `elapsedMs`, `element {ref, role, name, id, value, enabled, bounds}` |
| `get` | `what` (`text`, `value`, `box`), `target`, `timeout` | `text`, or `value`, or `x`, `y`, `width`, `height` |
| `set_timeout` | `ms` | `session`, `defaultTimeoutMs` |
| `doctor` | | `pid`, `version`, `drivers`, `compatibility {rows [{subject, feature, status, note}], note}` |
| `shutdown` | | `stopped`; the daemon exits after the reply |

Every target action but `set_timeout` claims its target. Points are device
points, as the mirror reports bounds. `dispatchMs` is the time the input
sink took to accept the tap, the span of the benchmark's `tap-dispatch`.

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
unless given `timeout`. Timeouts are at most 600000 ms.

The idle timer runs from the end of the last request and stands still
while any request is in flight, so a long wait outlasts a short idle
timeout.

## Drivers

`Driver` is the factory the registry takes backends from:

```rust
pub trait Driver: Send + Sync {
    fn name(&self) -> &str;
    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError>;
    fn attach(&self, target: &TargetInfo) -> Result<Attachment, DriverError>;
    fn compatibility(&self) -> Vec<CompatibilityRow> { Vec::new() }
}

pub struct Attachment {
    pub backend: mirror::backend::Backend,
    pub apps: Box<dyn AppControl>,   // launch and terminate apps
    pub display: Size,               // points, for gestures no element anchors
    pub config: MirrorConfig,
}
```

The registry spawns a worker thread per target on its first command; the
worker attaches, owns the mirror, runs commands in order, pumps the mirror
between them, and detaches when the daemon stops. `driver::registered` is
the registration point: the Android driver (task 5.3) and the Simulator
driver (tasks 6.1 to 6.9) join the list there, the latter also filling
`doctor`'s Xcode compatibility table through `Driver::compatibility`. Until
then the list holds the fake driver alone, when fake targets are named.

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
  `FIXTURE_INITIAL_TAB` honored at launch;
- a settings app with a Wi-Fi switch.

Android fakes use widget classes, a status bar, a keyboard window and
dialog windows, and send window events; iOS fakes use `XCUIElementType`
classes, alert nodes and SpringBoard's window, and send no events, so the
mirror re-reads them as it re-reads a Simulator. Screenshots are
placeholder bytes, not an image.

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
| `unknown_key` | the platform has no code for the key |
| `io_error` | a file could not be written |
| `internal` | a command failed unexpectedly; the worker detaches the target |

The mirror's codes pass through: `stale_ref`, `unknown_ref`,
`element_covered`, `element_offscreen`, `wait_timeout`, `no_frame`,
`disconnected`, and drivers' own codes.

## Interpretations of the spec

- *Sessions* isolate refs: besides claims, a session may use only the refs
  its own snapshots listed on the target.
- *Target selection*: a session that holds one of several connected targets
  keeps addressing it without a flag, as agent-browser's sessions stay on
  their browser; a session that holds none fails listing the candidates.
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
- The backend has no app lifecycle, screen size or on-demand screenshot;
  the driver contract adds `AppControl` and `display`, and targets without
  a frame source answer `screenshot` with `no_frame` until a driver adds a
  capture path.
- `testing::FakeTarget` runs on a fake clock and its tap reactions are
  fixed per identifier; the fake driver adds a real-time adapter and a
  stateful device model. Its `inputs()` copies the whole history, so the
  adapter hit-tests taps itself.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Sessions | Claimed target | `claims::a_claimed_target_refuses_another_session_naming_the_holder`, `claims::force_takes_the_target_over_and_the_previous_holder_is_refused`, `claims::every_target_command_claims_an_unclaimed_target`, `claims::a_released_target_serves_another_session_without_force`, `claims::the_holder_keeps_using_its_target`, `claims::status_names_the_holder_of_each_target`, `claims::a_request_without_a_session_uses_the_default_session` |
| Daemon lifecycle | (idle exit, configurable) | `idle::the_daemon_exits_after_its_idle_timeout_and_removes_its_files`, `idle::requests_postpone_the_idle_exit`, `idle::an_idle_timeout_of_zero_disables_the_exit`, `idle::the_idle_timeout_defaults_to_one_hour`, `idle::idle_timeouts_take_the_units_agent_browser_takes` |
| Daemon lifecycle | (version file, replacement, socket) | `lifecycle::the_daemon_writes_its_pid_and_version_files`, `lifecycle::a_daemon_of_another_version_is_shut_down_and_replaced`, `lifecycle::a_second_daemon_of_the_same_version_leaves_the_first_serving`, `lifecycle::files_left_by_a_killed_daemon_do_not_block_a_new_one`, `lifecycle::shutdown_stops_the_daemon_and_removes_its_files`, `lifecycle::the_socket_lives_under_the_runtime_directory`, `lifecycle::an_overlong_socket_path_is_refused_with_a_hint` |
| Output formats | (envelope and codes) | `lifecycle::every_reply_follows_the_agent_browser_envelope`, `lifecycle::malformed_and_http_looking_requests_are_refused_with_a_code`, `wire::tests::replies_follow_agent_browser_field_order` |
| Command grammar | Interactive snapshot output | `lifecycle::a_snapshot_of_the_fake_launcher_lists_a_heading_and_two_buttons`, `lifecycle::devices_lists_every_fake_target_with_its_platform_and_class`, `lifecycle::open_launches_an_installed_app_and_refuses_an_unknown_one` |
| Command grammar | (keys, element arguments) | `keys::tests::*`, `handlers::aim::tests::arguments_are_refs_identifiers_texts_or_roles` |

The CLI's tests (`crates/cli/README.md`) cover every scenario end to end
through this daemon.

```sh
cargo test -p daemon
```
