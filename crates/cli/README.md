# cli

`mobile-agent`, the command line of spec `agent-cli` (design D1 and D9): a
thin client of the daemon in `crates/daemon` that speaks agent-browser's
grammar, ref syntax and JSON envelope, so skills and prompts written for the
web tool carry over where devices have the same concepts. It links nothing
of the daemon; each command is one JSON request over the daemon's Unix
socket, and the first command starts the daemon.

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
  request.rs         a parsed command to its daemon request
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
| `open <app> [--relaunch] [--env K=V]...` | launches an app, claims its target, waits for the screen to settle |
| `close` | stops the app this session opened and releases the target |
| `snapshot [-i] [-d N] [--delta [--full]]` | lists elements, one per line with role, name and ref |
| `click <target>` or `click <x> <y>` (alias `tap`) `[--hold MS]` | taps a ref, a selector or a point in device points |
| `fill <target> <text>` | replaces a field's text |
| `type [<target>] <text>` | types key events into a field, or into the focused one |
| `press <key>` (alias `key`) | presses a key: `Enter`, `Back`, `Home`, `Control+a`, a letter or digit |
| `swipe <direction> [distance]` or `swipe <x1> <y1> <x2> <y2>` | moves the finger |
| `scroll [direction] [amount] [-s <target>] [--until <target>]` | moves the content, in strokes longer than a tap (more than 8 points on Android, 10 on iOS, else `stroke_too_short`); with `--until`, stroke by stroke until the element is mostly in view, each stroke at rest before the finger lifts and short enough that the element cannot pass unseen, `visible` giving the share of it in view |
| `screenshot [path] [--quality N] [--max-dimension PX]` | saves the latest frame as a JPEG |
| `wait <target>`, `wait <ms>`, `wait --text <t>`, `wait --settle` | waits for an element (`--state visible\|hidden\|detached\|enabled`), time, text or a settled screen |
| `get text\|value\|box <target>` | reads an element |
| `diff snapshot` | compares the screen with this session's last snapshot |
| `batch [--bail] [command]...` | runs commands in order |
| `status` | reports the daemon, its targets with what each attached target's driver reports, and its sessions |
| `doctor` | checks toolchains, the adb server, emulators and the daemon |
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
short. The reply's `visible` is the share of the element's bounds in view,
from 0 to 1, rounded to a hundredth; an `element_not_found` for an element
on screen but not mostly in view carries it too, as `data.visible`. A
`click` on an element that is not mostly in view still taps the centre of
its bounds, and warns.

A target is a ref as snapshots print it (`@e3`; `ref=e3` and `e3` work too)
or a selector: `#id` or `id=...` for a test identifier, `text=...` or bare
text for an element whose name or value contains it, `text==...` for one
whose whole name or value is the text, `role=...`. Refs never wait;
selectors wait for a match up to the command's timeout.

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
agent-browser does, which a click follows with its spans) on stdout and
errors (`✗`) and warnings (`⚠`) on stderr. `--json` prints one line in agent-browser's envelope, fields in
its order, `code` and `warning` only when set:

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
`firstTreeMs`;
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

Every failure exits with status 1 and carries a snake_case code. The CLI's
own are `invalid_argument` (usage), `unknown_command`, `invalid_config`,
`daemon_unavailable`, `daemon_timeout` and `doctor_failed`; the daemon's,
such as `stale_ref`, `target_claimed` or `ambiguous_target`, arrive in its
reply.

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
| `settle` | `wait --settle` | `elapsedMs`, `endedBy`, `stillChanging` | from the start of the wait until neither signal has changed for the quiet window (200 ms), at most `--timeout` (3000 ms) |

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
| `none` | nothing changed since the last action, or the screen was already quiet when the wait began (`elapsedMs` about 0) |
| `timeout` | the screen did not settle in time; `settled` is false and `stillChanging` names the signals, `tree` and `frame`, that changed in the last quiet window |

With a fake target that shows every change 120 ms late
(`MOBILE_AGENT_FAKE_LATENCY_MS=120`, see `crates/daemon/README.md`):

```json
{"success":true,"data":{"accepted":true,"clicked":"@e2","dispatchMs":0.314,"frameChangeMs":120.078,"ref":"@e2","treeDeltaMs":120.688,"watchedMs":123.28,"x":204.0,"y":120.0},"error":null}
{"success":true,"data":{"accepted":true,"clicked":"@e2","dispatchMs":0.303,"ref":"@e2","treeDeltaMs":121.268,"watchedMs":123.903,"x":204.0,"y":120.0},"error":null}
{"success":true,"data":{"accepted":true,"clicked":"204 600","dispatchMs":0.026,"ref":null,"watchedMs":304.642,"x":204.0,"y":600.0},"error":null}
{"success":true,"data":{"elapsedMs":297.58,"endedBy":"frame","frameChanges":6,"settled":true,"stillChanging":[],"treeChanges":0,"waited":"settle"},"error":null}
```

The lines are a click on an emulator, the same click on a device without
frames, a tap on empty space (whose watch ran past 200 ms because the
attach's own delayed change arrived during it), and a settle after a move
to another screen, whose 96 ms transition ended it. In human mode:

```
✓ Done (dispatch 0.3 ms, first frame change 120.1 ms, first tree delta 120.7 ms)
✓ Done (dispatch 0.0 ms, no change in 305 ms)
✓ Settled after 297.58 ms: frames changed last
✓ Stopped waiting after 3000.36 ms: frames kept changing
```

## Daemon lifecycle

Before each request the CLI reads `daemon.version` and compares it with the
version of the daemon executable it would start (the release plus the
executable's size and modification time). When they differ, or when no
daemon answers, it starts that executable in its own process group, in
`/`, with stderr appended to `daemon.log`, and waits up to 10 s until a
daemon of that version serves the socket; the new daemon replaces one of
another build, and concurrent first commands share one daemon. A daemon
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

Waiting commands take `--timeout <ms>`: `wait` for an element or text, and
`click`, `fill`, `type`, `get` and `scroll` on a selector. Without one,
the session's default applies (`set timeout`), else `--default-timeout`,
`MOBILE_AGENT_DEFAULT_TIMEOUT` or the config's `defaultTimeout`, else 10000
ms. `wait --settle` waits at most 3000 ms unless given `--timeout`; `open`
waits for the launched app's first tree up to that default, then at most
3000 ms for its screen to settle, and `--timeout` bounds both. Timeouts
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
| `android-sdk` | `ANDROID_HOME`, `ANDROID_SDK_ROOT` or the default location, and `adb version` |
| `adb-server` | `host:version` on `ANDROID_ADB_SERVER_PORT` or 5037 |
| `emulator-control` | each running emulator's gRPC control endpoint in its discovery file; the `-grpc`, `-grpc-use-token` and `emu grpc` hint when it has none; a warning, with how to turn it back on, when the daemon's emulator controller setting is off |
| `sfw`, `just` | on `PATH`, with the install commands through Socket Firewall |
| `xcode-compatibility` | the table's rows, which the Simulator driver supplies through `Driver::compatibility` |

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

## Help

Every command's help ends with examples and states its defaults; `help
<command>` prints the same as `<command> --help`, and `help get text`
reaches a subcommand. `help agent` prints `src/agent-guide.txt`, 85 lines
for an agent's prompt: the snapshot, act and snapshot-again loop, the
listing format, refs and `stale_ref`, selectors, acting, waiting and
settle, reading, output and codes, targets and sessions, and batch.

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
| Daemon lifecycle | Cold first command | `lifecycle::a_cold_devices_starts_the_daemon_and_the_next_command_reuses_it`, `lifecycle::a_daemon_of_another_version_is_replaced_by_the_next_command`, `lifecycle::files_left_by_a_killed_daemon_do_not_block_the_next_command`, `lifecycle::concurrent_first_commands_share_one_daemon`, `lifecycle::the_idle_timeout_flag_reaches_the_daemon_the_cli_starts`, `lifecycle::a_daemon_that_fails_to_start_is_reported_with_its_error`, `overhead::status_overhead_against_a_running_daemon_is_under_20_ms`, `commands::status_reports_the_daemon_and_its_targets`; idle exit in the daemon's `idle::*`; task 8.16: `lifecycle::shutdown_returns_once_the_daemon_has_stopped_its_targets_and_removed_its_socket`, `lifecycle::a_command_right_after_shutdown_starts_a_fresh_daemon_without_meeting_the_old_socket` |
| Target selection | Ambiguous target | `selection::two_targets_without_a_device_flag_fail_listing_both`, `selection::one_connected_target_is_selected_by_default`, `selection::the_device_flag_selects_a_target_by_id_or_name`, `selection::the_platform_flag_selects_the_only_target_of_its_platform`, `selection::a_platform_with_several_targets_is_still_ambiguous_among_them`, `selection::a_device_flag_naming_no_target_fails_listing_the_connected_ones`, `selection::the_device_and_platform_flags_have_environment_twins`, `selection::a_session_that_holds_one_target_keeps_using_it_without_a_flag`, `selection::no_connected_target_is_reported_with_a_code` |
| Sessions | Claimed target | `sessions::a_claimed_target_refuses_another_session_naming_the_holder_until_forced`, `sessions::refs_are_isolated_between_sessions`, `sessions::the_session_and_json_flags_have_environment_twins`, `selection::claims_are_per_target`; the daemon's `claims::*` |
| Batch execution | Batch with failure | `batch::a_bailing_batch_stops_after_the_failing_second_command`, `batch::a_batch_without_bail_runs_every_command_and_still_fails`, `batch::a_batch_reads_a_json_array_of_argv_arrays_from_stdin`, `batch::quoted_arguments_keep_their_spaces`, `batch::a_human_batch_prints_each_result_with_a_blank_line_between`, `batch::a_command_that_does_not_parse_is_a_failed_entry_with_a_code`, `batch::a_batch_applies_its_global_flags_to_every_command`, `batch::an_empty_batch_prints_an_empty_array`, `batch::tests::words_split_as_a_shell_splits_them` |
| Configurable timeouts | Per-command timeout | `timeouts::a_2_s_wait_fails_at_2_s_not_after_the_session_default`, `timeouts::the_session_default_applies_to_waits_that_name_no_timeout`, `timeouts::session_defaults_belong_to_their_session`, `timeouts::the_environment_default_applies_when_the_session_sets_none`, `timeouts::timeouts_are_bounded_and_must_be_numbers`, `timeouts::a_long_wait_outlasts_a_short_idle_timeout`, `help::waiting_commands_document_their_default_timeouts`, `client::tests::waits_get_their_own_timeout_plus_a_margin_and_other_commands_30_s` |
| Agent-facing help | Prompt guide | `help::the_agent_guide_fits_in_120_lines_and_covers_the_loop_refs_and_settle`, `help::every_command_has_help_with_examples`, `help::the_top_level_help_points_to_the_agent_guide`, `help::help_for_an_unknown_topic_fails_with_a_code` |

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

Doctor (task 8.7) has no spec scenario of its own; `doctor::*` tests each
check against scripted command outputs and
`doctor::doctor_lists_every_check_with_a_status_in_both_outputs` runs the
command.

The tests start their daemons with fake targets and
`MOBILE_AGENT_DRIVERS=none`, so a Simulator or emulator running on the
host never shows up in their listings.

```sh
cargo test -p cli     # builds mobile-agent-daemon through cargo when it is missing
```
