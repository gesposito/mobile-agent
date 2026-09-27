# agent-android

The resident Android agent: a Kotlin program compiled into a single `classes.dex`,
pushed to `/data/local/tmp` and started with `app_process` as the shell user, so
nothing is installed on the device (design D2). It serves the device protocol of
`crates/protocol` on an abstract Unix socket; the entry point also prints the
version banner described below.

## Layout

```
agent-android/
  VERSION                  agent version, compiled into the banner
  build.sh                 kotlinc + d8 build, no Gradle
  test.sh                  on-device smoke test
  codec-test.sh            golden cross-check of the codec against crates/protocol
  codec-test/              the Kotlin side of that cross-check
  host-tests/              Rust tests that drive the agent on a device (own Cargo workspace)
  src/dev/mobileagent/agent/
    Main.kt                entry point that app_process calls, options
    Server.kt              abstract-socket server, peer check, request dispatch
    Session.kt             one host connection: handshake, requests, deadlines, events, liveness
    Errors.kt              the structured errors the agent sends
    codec/                 the protocol codec, byte for byte the same frames as crates/protocol
      Protocol.kt          versions, limits, kinds, methods, codes, flags
      Messages.kt          message classes
      Codec.kt             encoder and decoder of every message
      CborWriter.kt        CBOR writer (preferred serialization, definite lengths)
      CborReader.kt        strict CBOR reader
      Frames.kt            32-bit length-prefixed framing
  out/   (git-ignored)     gen/BuildConfig.kt, classes/, dex/classes.dex, d8.log
  dist/  (git-ignored)     mobile-agent-android.jar
```

The codec under `codec/` is shared with `codec-test.sh`, which checks that it
writes and reads the golden frames of `crates/protocol/golden` byte for byte;
the agent uses it unchanged for every frame it sends and receives.

## Requirements

- A JDK 17 or newer with `java` and `jar` on PATH (tested with Temurin 21).
- The Kotlin compiler, `kotlinc` (tested with 2.4.20).
- An Android SDK at `ANDROID_HOME`, default `$HOME/Library/Android/sdk`, with
  `platforms/android-36` and a `build-tools` package (tested with 36.0.0), plus
  `platform-tools/adb` for the smoke test.

## Building the dex without Gradle

`./build.sh` runs four steps:

1. **Version constant.** `VERSION` must hold one semantic version. It becomes
   `const val VERSION` in the generated `out/gen/dev/mobileagent/agent/BuildConfig.kt`.
2. **Compile.**
   `kotlinc -jvm-target 17 -no-jdk -no-reflect -Werror -classpath $ANDROID_HOME/platforms/android-36/android.jar -d out/classes <sources>`.
   `-no-jdk` takes the host JDK off the compile classpath, which leaves
   `android.jar` as the only platform API. As a result, a call to a JDK-only API
   such as `java.lang.ProcessHandle` fails to compile instead of failing on the
   device. `-no-reflect` keeps kotlin-reflect out of scope because the dex does
   not include it. `-Werror` follows the workspace rule that warnings fail
   validation.
3. **Dex.**
   `d8 --release --min-api 26 --lib android.jar --output out/dex <each .class file> kotlin-stdlib.jar`.
   d8 accepts class files and archives but not directories, so each class file
   is passed on its own. The Kotlin stdlib is dexed in so that the jar needs
   nothing else on the device's class path. The build fails if d8 splits the
   output into more than one dex file.
4. **Package.** `jar --create --no-manifest` makes `classes.dex` the only member
   of `dist/mobile-agent-android.jar`. `app_process` accepts that jar on
   `CLASSPATH`.

Progress goes to stderr. The last two lines on stdout are the jar path and the
dex size:

```
/…/agent-android/dist/mobile-agent-android.jar
classes.dex 2281104 (build-tools 36.0.0; 2277628 with build-tools 37.0.0) bytes
```

Tool resolution:

- **d8.** Taken from the newest `$ANDROID_HOME/build-tools/*`, never from PATH.
  The SDK's cmdline-tools and Homebrew ship their own d8 releases.
- **kotlinc and `kotlin-stdlib.jar`.** Both come from one Kotlin home, so the
  compiler and the dexed stdlib always match. The home is `KOTLIN_HOME` if set.
  Otherwise it is the install behind `kotlinc` on PATH, with symlinks resolved;
  a mise shim is resolved with `mise which kotlinc`.

### Kotlin metadata diagnostics

The D8 in build-tools 36.0.0 (8.10.9) reads Kotlin metadata up to version 2.2.0,
but Kotlin 2.4.20 writes version 2.4.0. That D8 therefore reports diagnostics for
almost every stdlib class: about three per class, over 2,800 per build. It still
exits 0 and produces a working dex. Only kotlin-reflect reads this metadata, and
the agent does not include it. `build.sh` saves d8's full output to
`out/d8.log`, prints a one-line count of these diagnostics, and shows every other
diagnostic in full. A D8 that knows the newer metadata stops reporting them:
D8 9.4.14 reports nothing for the same input.

## Launching with app_process

```sh
adb push dist/mobile-agent-android.jar /data/local/tmp/mobile-agent-android.jar
adb shell CLASSPATH=/data/local/tmp/mobile-agent-android.jar app_process /data/local/tmp dev.mobileagent.agent.Main --version
```

`app_process` is the platform's launcher for Java programs; the device's own
`monkey` and `uiautomator` commands are scripts that exec it. Given a class name,
it starts an ART runtime with `CLASSPATH` as the system class path, loads the
class from the jar's `classes.dex`, and calls its static `main(String[])`. The
`/data/local/tmp` argument fills app_process's parent-directory slot, which it
ignores.

Started through `adb shell`, the process runs as the `shell` user (uid 2000).
It installs no package, uses no instrumentation and shows no prompt. It ends
when `main` exits. `Main` calls `exitProcess` so that the process also ends if
platform code has started non-daemon threads.

## Banner contract

- `--version`, or no arguments: prints exactly one line to stdout and exits 0.
  The line is `mobile-agent-android <version> protocol=<n>`, for example
  `mobile-agent-android 0.1.0 protocol=1`. `<version>` is the content of
  `VERSION`. `<n>` is the codec's `Protocol.VERSION`, the revision of the
  host-agent wire protocol, currently 1.
- `serve` without `--socket NAME`, or with an unknown option: prints a usage
  line to stderr and exits 2.
- Any other first argument: prints `unknown argument` to stderr, nothing to
  stdout, and exits 2.

adb's shell protocol v2, available since Android 7, returns stdout, stderr and
the exit status separately, so the host can check all three.

## Serving the protocol

```sh
adb shell MOBILE_AGENT_TOKEN=<token> CLASSPATH=/data/local/tmp/mobile-agent-android.jar \
  app_process / dev.mobileagent.agent.Main serve --socket <name>
```

- **Socket.** The agent listens on the abstract Unix socket `<name>`, which
  the host chooses per launch, and never on TCP. The host reaches it as the
  adb server service `localabstract:<name>` after `host:transport:<serial>`
  (design D3), so no forward rule exists. Once listening the agent prints
  `ready socket=<name> pid=<pid> uid=<uid> protocol=1 capabilities=0x<bits>`
  on stdout, and `ps` lists it as `mobile-agent <name>`.
- **UI automation.** Before listening the agent takes the UI automation role
  as `uiautomator` does (design D2): a reflective `UiAutomationConnection`
  handed to the hidden `UiAutomation(Looper, IUiAutomationConnection)`
  constructor and `connectWithTimeout` with
  `FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES`, so a screen reader keeps
  running. It then sets `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` and waits up to
  `--window-wait-ms` (default 2000) for an active window, since the window
  list is empty or partial for up to 55 ms after the flag is set. The ready
  line reports `ownership=held connect_ms=<ms> window_wait_ms=<ms> windows=<n>
  active=<package> forced=<name:pid or ->`. The role is released by `shutdown`
  and by any exit.
- **Conflict.** When the platform refuses the role (`already registered!`,
  which names only a binder proxy), the agent retries for 2 s, so a
  short-lived holder such as `uiautomator dump` passes. Then it infers the
  likely holder from `ps -A` (where `app_process` programs show the names they
  gave themselves: `uiautomator`, `mobile-agent <socket>`, `am instrument`,
  and the servers of Appium, openatx, Maestro and agent-device) and from the
  running instrumentations in `dumpsys activity processes`, prints
  `conflict pid=<pid> holder=<name> holder_pid=<pid> message=<text>` naming
  it, and exits with status 3 without listening. For example:
  `conflict pid=4711 holder=uiautomator holder_pid=4702 message=UI automation
  is held by another process, likely uiautomator (pid 4702: uiautomator); stop
  it, or run the agent with --force to take the role over`.
- **Force.** With `--force` the agent stops the known holders in that order,
  SIGKILL for a process the shell user may signal and `am force-stop` for an
  app or instrumentation, retrying the role after each, and reports which one
  it stopped (`forced=uiautomator:4702` and the `ready` lifecycle event). It
  never stops a process it cannot identify as an automation client; if only
  such processes remain, it reports the conflict.
- **Peer check.** Every connection's `SO_PEERCRED` uid must be 2000, the uid
  adbd and `adb shell` run as, or the agent's own uid; any other peer is
  closed before a byte is sent and logged as `refused peer uid=<uid> pid=<pid>`.
- **Token.** `MOBILE_AGENT_TOKEN` in the environment is the per-launch token.
  It stays out of the process listing and travels in the hello, where the
  host's session refuses a token other than the one it started the agent with
  (`token_mismatch`).
- **Handshake.** The agent speaks first: its hello offers versions `[1]`, its
  capabilities, a 16 MiB frame limit and `mobile-agent-android <VERSION>`.
  After the welcome it sends a `ready` lifecycle event. A welcome refusing
  the connection with `version_mismatch` makes the agent exit, so a host that
  needs another version replaces it; any other refusal only closes that
  connection.
- **Requests.** A request id already in flight closes the connection as a
  protocol error; an unknown method is answered with `unknown_method`, a
  method whose capability the hello did not advertise with
  `capability_missing`. A request whose deadline passes is answered with
  `deadline_exceeded`, its work is cancelled and its late answer dropped. A
  response over the negotiated frame size is replaced by a `frame_too_large`
  error naming the limit and the request, as is an oversized request, whose
  payload is dropped as it arrives.
- **Tree reads.** `snapshot` walks every interactive window from
  `getWindows()`, top-most first, breadth first with
  `FLAG_PREFETCH_DESCENDANTS_BREADTH_FIRST | FLAG_PREFETCH_UNINTERRUPTIBLE`
  (spike S1's cheapest strategy), and serializes class, resource id (its
  `pkg:id/` prefix in the string table), text, content description, screen
  bounds and the node flags, visibility to the user included. Before reading
  it waits for the screen to go idle, meaning no accessibility event for
  200 ms (design D8's quiet window), for at most `idle_ms`: 500 ms when the
  request names none, no wait at all for 0. An endless animation keeps
  sending events, so the wait then lasts exactly the bound; the platform's
  own `waitForIdle` is never called. Node ids follow
  the walk. `max_depth` (levels below each root, default and ceiling 256) and
  `max_nodes` (default 10,000) bound what is serialized, not what is counted:
  past either cap the walk goes on counting, up to 100,000 nodes, so
  `node_count` is the screen's full count, the omitted count is `node_count`
  minus the nodes serialized, and `truncated` names the cap that cut (bit 0
  depth, bit 1 nodes, bit 2 a window without a root). The visible count is
  the nodes whose `visible` flag is set. `seq` is the last tree event sent
  before the walk began. A read whose deadline passes stops at the next node.
  With `--trace` the agent prints `trace snapshot id=<id> windows=<n>
  nodes=<total> visible=<n> serialized=<n> truncated=0x<bits> idle_ms=<ms> read_ms=<ms>
  round_trips=<n>`, the round trips read from the accessibility client's
  interaction counter.
- **Change events.** Accessibility events become protocol events, numbered
  per stream and per connection from 1. Content changes, text, selection,
  scroll and focus events go on the tree stream with the window id and the
  child-index path from the window root to the event's source, found by
  climbing parents in the platform cache (hidden `getSourceNodeId` and
  `getChildId`); window state changes and `TYPE_WINDOWS_CHANGED` (added,
  removed, bounds, focus, state) go on the window stream. The platform cache
  has processed each event before the agent sees it, so the named subtree is
  fresh for the next read while the rest of the window stays cached.
- **Compose windows.** A window whose tree holds a class under
  `androidx.compose.` hosts an AndroidComposeView, which sends no events to
  UI automation (spike S1). While a host is connected the agent re-reads
  such windows in full every `--compose-poll-ms` (default 1000, 0 turns it
  off, never more often than four times the last re-read took), after
  evicting their cached nodes (hidden `AccessibilityCache.clearSubTree` on a
  fresh root), diffs the result against the last read the host received, and
  sends a tree event naming the deepest subtree holding every difference.
  Only windows a snapshot has shown the host are re-read. With `--trace` each
  re-read prints `trace compose windows=<ids> nodes=<n> read_ms=<ms>
  round_trips=<n> changed=<window:path or ->`.
- **Input.** `tap`, `swipe` and `key` go through the hidden
  `InputManagerGlobal.injectInputEvent(event, WAIT_FOR_RESULT)` (design D2),
  never the `UiAutomation` overloads, and each is answered with
  `{accepted}`, true only when the platform accepted every event. A tap is
  DOWN and UP with a finger at one point; with `duration_ms` it is a long
  press held that long. A swipe is a multi-step gesture: DOWN at the start,
  a MOVE every 8 ms along the line on the real clock, so the app sees the
  velocity the duration implies, and UP at the end. A key is DOWN and UP
  from the virtual keyboard with the given meta state. A gesture whose
  deadline passes is cancelled with ACTION_CANCEL, never left down. Input
  runs on its own thread, so a tap never waits behind a tree read. After an
  action the idle wait counts from it, the next snapshot evicts the Compose
  windows' cache before reading them (they report nothing the action
  changed), and the Compose poll comes forward to 200 ms after the action.
- **Text.** `text_set` targets the node the request names from the last
  snapshot, or else the node with input focus (`findFocus(FOCUS_INPUT)`),
  refreshed first. When it is editable and offers `ACTION_SET_TEXT`, the text
  is set in one action and the answer names `direct`; otherwise, or when
  `mode` asks for keys, the text is typed as key events from the virtual
  keyboard's character map, after the node gets input focus and, if it
  holds text, Ctrl+A so the keys replace it, and the answer names `keys`.
  `mode` direct on a node without the action, a character the key map
  cannot type, no focused element or a node id not in the last snapshot are
  answered with `invalid_params` naming the reason. Text counts as an
  injected action for the idle wait and the Compose re-read.
- **Resync.** `resync` answers the last sequence number sent on the stream,
  read and written under the lock events take; for the tree stream the node
  cache is emptied first, so the host's next snapshot is a full read.
- **Liveness.** The agent pings a host it has not heard from for a second and
  closes the connection after three seconds of silence, as the protocol
  crate's sessions do.
- **Reuse.** The agent outlives a connection: a later connection gets a fresh
  hello with the same token, and a new connection replaces the current one,
  so a restarted daemon reuses the running agent.
- **Stop.** `shutdown` is answered with `{}` after a `shutting_down`
  lifecycle event, and the process exits with status 0.

## Dex size baseline

| Measure | Value |
| --- | --- |
| `classes.dex` | 2,281,104 with build-tools 36.0.0, 2,277,628 with build-tools 37.0.0, which also reads Kotlin 2.4 metadata and prints no diagnostics bytes |
| Agent classes alone (stdlib on the classpath, not dexed) | 2,432 bytes |
| `mobile-agent-android.jar` (deflated) | 663,748 bytes |
| Dex contents | 1,083 classes, 12,700 method references |
| Toolchain | Kotlin 2.4.20 stdlib, D8 8.10.9 (build-tools 36.0.0), `--release --min-api 26` |

The Kotlin stdlib, dexed whole and unshrunk, accounts for almost all of the size.
The build is reproducible: repeated builds produce byte-identical dex files.

This is the baseline for the dex-size gate in spike S5 (design D4): a wire-format
candidate may add under 100 KB. The size depends on the toolchain, so compare
builds made with the same Kotlin and D8 versions. For example, D8 9.4.14 gives
2,277,544 bytes for the same input.

## Smoke test

```sh
./build.sh && ./test.sh                  # the only connected device
ANDROID_SERIAL=emulator-5554 ./test.sh   # a chosen device
```

`test.sh` checks that the jar's only member is `classes.dex` and pushes the jar
to `/data/local/tmp`. It then runs the agent through `app_process` three times:
with `--version`, with no arguments, and with an unknown argument. Each run has
a 30-second timeout on the device. The test checks:

- the banner equals `mobile-agent-android <VERSION> protocol=<n>` exactly,
  where `<n>` is read from the codec's `Protocol.VERSION`;
- the exit statuses are 0, 0 and 2;
- the unknown argument produces `unknown argument` on stderr;
- `serve` without a socket name prints a usage line and exits 2;
- no process with `dev.mobileagent.agent.Main` in its command line is left
  running.

It removes the pushed jar on exit and stops at the first mismatch with a
non-zero status and a `FAIL:` message.

## Host tests

`host-tests/` is a Cargo package outside the repository workspace (its
manifest declares its own `[workspace]`) that depends on `crates/protocol` by
path. Its tests start the agent the way the daemon will, open its socket as an
adb server service stream and drive `protocol::session::HostSession`:

```sh
./build.sh
cd host-tests
MOBILE_AGENT_ANDROID_SERIAL=emulator-5554 cargo test -- --nocapture
cargo test                       # without a device: every test prints why it skips
cargo clippy --all-targets -- -D warnings && cargo fmt --check
```

The tests run one at a time, since they share the device and its single UI
automation role. Each one starts its own agent from a jar pushed under a
unique name, and kills it by pid (after checking that `/proc/<pid>/cmdline`
still names its socket) and removes the jar when it ends. They need the
Compose and Expo fixtures installed (`fixtures/`) and put back every device
setting they change.

## Spec scenarios and their checks

Every agent-side scenario of `specs/android-driver/spec.md`, and the
agent-side behaviour of `specs/device-protocol/spec.md`, maps to a named check.
Rust tests are `host-tests/tests/<file>.rs::<name>`.

| Requirement | Scenario | Checks |
|---|---|---|
| device-protocol: Version negotiation on connect | Common version selected | `handshake::the_agent_speaks_first_with_a_hello_naming_protocol_1_and_its_token` |
| | No common version | `handshake::a_host_speaking_only_a_newer_version_is_refused_and_the_agent_exits_for_replacement` |
| device-protocol: Structured errors | Transient versus permanent failure (unknown method) | `handshake::an_unknown_method_gets_a_permanent_error_and_the_connection_stays_usable` |
| device-protocol: Liveness detection | Agent side of the pings | `handshake::the_agent_pings_a_silent_host_every_second`, `handshake::every_ping_answer_keeps_a_quiet_session_open_past_the_peer_timeout`, `handshake::the_agent_closes_a_connection_whose_host_stops_answering` |
| design D3: token and peer check | | `handshake::a_host_expecting_another_token_refuses_the_hello_and_the_agent_keeps_serving`, `handshake::a_peer_that_is_not_the_shell_user_gets_no_hello` |
| android-driver: Agent deployment without installation | Compatible agent reused | `handshake::a_later_connection_reuses_the_running_agent` |
| | Incompatible agent replaced | `handshake::a_host_speaking_only_a_newer_version_is_refused_and_the_agent_exits_for_replacement` |
| android-driver: Single ownership of UI automation | Another tool holds automation | `ownership::an_attach_while_uiautomator_events_holds_the_role_fails_naming_it`, `ownership::a_second_agent_names_the_first_as_the_holder` |
| | Force option | `ownership::force_takes_the_role_over_from_uiautomator_events` |
| | Role held with interactive windows | `ownership::the_agent_holds_ui_automation_with_interactive_windows_once_ready`, `ownership::shutdown_releases_the_role_for_the_next_owner` |
| android-driver: Tree acquisition across windows | Keyboard is included | `tree::the_keyboard_window_appears_with_its_keys_when_a_field_is_focused` |
| | Node limit reached | `tree::a_lower_node_cap_truncates_and_still_reports_the_full_count`, `tree::a_lower_depth_cap_truncates_below_it_and_still_reports_the_full_count` |
| | Attributes of every node, 500-node screens | `tree::the_compose_500_cell_screen_returns_every_cell_with_total_and_visible_counts`, `tree::the_expo_500_cell_screen_returns_every_cell_with_total_and_visible_counts` |
| android-driver: Bounded idle waiting | Continuous animation | `idle::a_snapshot_during_an_endless_animation_returns_within_the_default_bound_plus_read_time` |
| | No wait at all, a requested bound, a quiet screen | `idle::idle_zero_reads_at_once_during_an_animation`, `idle::a_requested_bound_caps_the_wait`, `idle::a_quiet_screen_is_read_without_waiting_for_the_bound` |
| android-driver: Change events | Single field changes | `events::editing_one_react_native_field_emits_events_naming_that_field_only` |
| | Window changes, sequence numbers | `events::a_keyboard_appearing_is_reported_on_the_window_stream` (and the numbering check in every events test) |
| | Compose windows re-read on a poll (design D5) | `events::a_compose_window_is_reread_on_a_poll_and_its_change_reported` |
| device-protocol: Event channel with sequence numbers | Agent side of a resync | `events::a_tree_resync_answers_the_last_sequence_and_the_next_read_is_full` |
| android-driver: Input injection | Tap acknowledged | `input::a_tap_is_acknowledged_as_accepted_within_50_ms`, `input::a_tap_the_platform_refuses_is_acknowledged_as_not_accepted` |
| | Long press, swipe with duration, multi-step gesture, keys | `input::a_long_press_holds_for_its_duration`, `input::a_swipe_with_duration_scrolls_the_list`, `input::a_slow_multi_step_drag_moves_the_list_along_its_path`, `input::key_presses_are_acknowledged_and_typed_into_the_focused_field` |
| | Compose windows read fresh after an injected action (design D5) | `input::a_tap_on_a_compose_tab_is_in_the_next_snapshot_without_a_poll` |
| android-driver: Direct text setting with fallback | React Native text input | `text::a_focused_react_native_input_is_filled_directly_in_one_operation` |
| | Element rejects direct setting | `text::a_focused_element_without_direct_setting_gets_key_events_and_the_fallback_is_named` |
| | Keys on request, a named node, refusals | `text::keys_on_request_replace_the_field_text_and_name_the_key_method`, `text::a_node_named_from_the_last_snapshot_is_set_directly`, `text::direct_text_for_a_node_without_it_or_an_unknown_node_is_refused` |
| device-protocol: Framed self-describing messages | Oversized frame is rejected explicitly (agent side) | `tree::a_snapshot_over_the_negotiated_frame_size_is_refused_naming_the_limit_and_request` |
| device-protocol: Deadlines and cancellation | Snapshot exceeds its deadline | `tree::a_snapshot_past_its_deadline_gets_a_deadline_error_and_the_next_request_is_served` |
| android-driver: Agent lifecycle | Explicit stop | `handshake::shutdown_is_answered_before_the_agent_exits` |
| Banner and arguments | | `test.sh` |
