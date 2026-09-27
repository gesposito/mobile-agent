# android

Host-side Android driver (design D11). So far it holds:

- `adb`: the driver's own client for the adb server protocol (design D3, task 5.1).
- `agent`: deploying the resident agent, reusing or replacing a running one,
  and the connection to it (design D2 and D3, task 5.2).
- `emulator`: the client for the emulator's gRPC controller, the emulator fast
  path (design D7, task 5.4).
- `session`: an attached agent, and the controller when there is one, behind
  the mirror's backend traits (design D5 and D7, task 5.3).

Path selection and the status report (task 5.5) come later.

## adb client

`adb::AdbServer` talks to the local adb server (127.0.0.1, port 5037 or
`ANDROID_ADB_SERVER_PORT`) with one connection per request. It never spawns the
`adb` binary. Each connection has a 2 s connect deadline and a 10 s read and
write deadline, which `AdbServer::with_io_timeout` changes. The client is
blocking, so async callers run it on a blocking thread.

| Operation | Protocol |
|---|---|
| `devices`, `targets` | `host:devices-l`, keeping every state; `targets` adds `getprop` facts for online targets and advice for the others |
| `Device::shell`, `shell_stream`, `shell_stream_with` | `shell,v2,raw:` with exit status and separate stderr, or `shell,v2,pty:` for a pseudo-terminal; `shell` closes stdin at once |
| `Device::push`, `push_bytes` | sync `SEND` in 64 KiB chunks, each header written with its data, then `DONE` and adbd's status |
| `Device::forward`, `forward_tcp`, `remove_forward`, `forwards` | `host-serial:<serial>:forward:...`, reading the second status and the port bound for `tcp:0` |
| `Device::reverse`, `remove_reverse`, `reverses` | `reverse:forward:...` on the device transport, reading adbd's own status |
| `Device::open_service`, `open_abstract` | `host:transport:<serial>`, then the service, with no rule; a missing listener fails with `closed` |

The integration tests confirmed four behaviours on the API 36 emulator:

- adbd copies the owner's permission bits of a pushed file to group and other,
  so 0o644 arrives as 0666 and 0o755 as 0777. The `adb` binary's pushes do the
  same.
- Dropping a `ShellStream` ends its command, because adbd sends it SIGHUP.
- On a pipe, C programs such as `getevent` buffer their output in blocks; on a
  pseudo-terminal (`Stdio::Pty`) they flush every line.
- A reverse rule makes adbd listen on its device side. The integration tests
  use this to get a live abstract socket with no process on the device.

## Agent deployment

`agent::Deployer` deploys one agent build on one device. `AgentJar` is the jar
`agent-android/build.sh` writes (`agent-android/dist/mobile-agent-android.jar`,
or the path in `MOBILE_AGENT_JAR`), expected to report the version in
`agent-android/VERSION`, which the crate compiles in. `Deployer::attach` lists
the agent processes with `ps -A -o PID,ARGS`, where the agent shows as
`mobile-agent <socket>`, then:

1. **Reuse.** When `Records` hold an agent for the serial and it still runs
   with the recorded pid and socket, the deployer connects with the recorded
   token and completes the handshake. If the host accepts the hello and it
   names `mobile-agent-android <VERSION>`, the connection is returned as
   `Outcome::Reused` and no process starts. The agent's previous connection,
   if any, is replaced by this one.
2. **Replacement.** A recorded agent that is not compatible is stopped before
   a new one starts, and the launch reports it in `Launched::replaced` with
   the reason. An agent speaking no protocol version the host speaks is
   refused in the welcome with `version_mismatch` and exits by itself. Another
   build is asked to `shutdown`. One that does not answer as recorded is
   killed. Whatever is still there after `exit_timeout` (3 s) is killed by
   pid, after checking that `/proc/<pid>/cmdline` still names its socket.
3. **Conflict.** Any other agent process serves another host, so it is a
   `Conflict` naming it (`mobile-agent`, its pid and socket), reported at once
   without starting anything. With `DeployOptions::force` the launch passes
   `--force`, and the new agent stops it along with any other known
   automation client.
4. **Launch.** Otherwise the jar is pushed and started:
   - The jar goes to `/data/local/tmp/<socket>.jar` with mode 0o444. adbd
     copies the owner's permission bits to group and other (0o644 arrives as
     0666), so read-only for everyone is the one mode that arrives unwidened
     and never writable.
   - The socket name is per launch and says which build listens on it:
     `mobile-agent-<version>-p<protocol>-<12 hex digits>`. The token is 16
     random bytes as hex.
   - The launch runs `read -r t && (trap '' HUP PIPE; MOBILE_AGENT_TOKEN="$t"
     CLASSPATH=<jar> exec app_process / dev.mobileagent.agent.Main serve
     --socket <socket> --idle-exit-ms <ms> </dev/null &)` on a shell v2
     stream and writes the token to its standard input, so the token is on
     no command line.
   - The agent runs in a subshell's background, so it is reparented to init.
     Dropping the stream makes adbd send SIGHUP to the shell it started,
     which has exited, never to the agent, and the agent's later writes to
     the closed stdout fail with EPIPE: ART blocks SIGPIPE in every thread
     (`SigBlk` 0x80001204 on the API 36 emulator), and the command ignores
     SIGHUP and SIGPIPE as well, in case an adbd signals the whole process
     group. `the_agent_outlives_its_launch_stream_and_a_closed_stdout`
     checks the parent (pid 1), the ignored signals (`SigIgn` 0x1001) and
     that a refused root peer, which makes the agent print, leaves it
     serving.
   - The stream is read until the ready line, which gives the pid and the
     holder `--force` stopped, or the conflict line, which gives the likely
     holder and the agent's message. The stream is then dropped, the jar is
     deleted (the running agent keeps the dex it loaded) and the agent's
     socket is opened as an adb service stream with the token.
   - A hello that names another build than the jar should report means a
     stale jar: that agent is stopped and the error says to rebuild.

`Records` keep socket, pid, token and versions per serial, in memory, and in a
directory when the daemon names one (`Records::in_dir`, one file per serial,
mode 0600), so a restarted daemon reuses the agent its predecessor left
running. `Attached::stop` sends `shutdown`, waits for the exit, kills the
agent past the timeout and forgets the record; `Deployer::stop_recorded` does
the same without a connection. `Deployer::state` reads the agent state from
the process list alone, `Absent`, `Running` or `Incompatible`, from the
version and protocol in the socket name: connecting would take the agent's
connection away from its host.

`agent::Connection` drives the protocol crate's `HostSession` over the socket
on a reader thread. It answers pings, acts on deadlines, hands each response
to the caller waiting for it, and passes every other event to an `EventSink`;
events that arrive before the sink is set are replayed to it.

Measured on the API 36 emulator through `tests/agent_deploy.rs`, shared with
other work: a first attach took 215.8 ms (process listing 14.3 ms, push
3.5 ms, launch stream to ready line 185.6 ms, connection and handshake
3.0 ms), a reuse 17.0 ms (listing 13.9 ms, handshake 2.0 ms), and replacing
an older build 265.3 ms, of which stopping it took 28.2 ms.

## Agent session

`session::Session::start` takes an `Attached` agent and, optionally, a
`ControllerLink` (a connected controller on a two-thread Tokio runtime of its
own). It reads the display density with `wm density` (420 dpi on the API 36
emulator, a scale of 2.625 pixels per point, the override density first when
one is set). `Session::backend` hands a `mirror::Backend` over, once:

- **Tree.** Every read is one `snapshot` request with a 10 s deadline; the
  agent reads every interactive window, so a read may return more windows
  than the mirror asked for. Reads after an attach, an action or a resync take
  the agent's idle bound (500 ms, ending 200 ms after the last event or
  action), and reads the target's own changes trigger (events, polls, frames)
  do not wait, since the mirror's settle replaces the platform's idle wait
  (design D8). The snapshot passes through as the agent sent it: its window
  metadata (whether each window's toolkit sends change events, `false` for
  Compose content; the system process that owns it; whether it is modal)
  and its scroll semantics are what the mirror reads.
- **Events.** The connection's events reach the stream as they arrive, with
  the host time: protocol events, `OutOfSync` and `Resynced` as themselves,
  and the close as `Disconnected` with the protocol crate's reconnect hint. A
  killed agent closes its socket, so the stream ends at once; a silent one is
  declared dead after 3 s by the protocol crate's liveness. A resync the host
  session could not complete is retried on the next read.
- **Input.** Taps (a long press with a hold), swipes, keys and text go to the
  agent in physical pixels with a 5 s deadline plus the gesture's duration;
  text names the node of the latest read, and `auto` leaves the method to
  the agent. With a controller, taps and swipes go through it instead (a
  swipe moves every 8 ms along its line, as the agent's does); keys and text
  stay with the agent. Input the agent did not inject leaves its cache of
  Compose windows stale and does not start its idle wait, so the next read
  waits until the input is 200 ms old and, when the last read showed a window
  that sends no change events, first sends a tree `resync`, which empties
  the agent's node cache. A controller
  that fails before a touch lands hands that input to the agent; after that,
  all input goes through the agent.
- **Frames**, with a controller only. A task on the link's runtime subscribes
  to 540-pixel-wide RGB888 frames and puts each on the event stream as a
  `Frame` whose hash leaves the top 6 % of rows out (the status bar, spike
  S7), and whose capture time is its arrival minus the lag the emulator's
  timestamp shows, at most a second. The last four frames are kept; the frame
  source turns and scales one and encodes it with jpeg-encoder. The stream is
  subscribed to again when it fails.

`Session::stop` sends `shutdown` and kills the agent past the timeout;
`Session::screenshot` asks the agent. `Session::reads`, `last_read`,
`inputs` and `frames_received` count what went where.

Measured through `tests/agent_session.rs` on the API 36 emulator, a debug
build, shared with other work: the Compose Big screen (573 nodes, every one
in the replica, all 500 cells with refs) took 494.6 ms for the first
snapshot, of which the agent's cold read with its idle wait 490.5 ms; warm
snapshots from the replica took 3.15 ms at p50 and 3.33 ms at p95 with no
device read; a tap through the mirror on the Login tab was acknowledged in
11.3 ms and the snapshot after it showed the Login screen 425.8 ms after the
tap (the agent's idle wait, which ends no sooner than 200 ms after its input,
then a cold read of the Compose window); a killed agent was reported disconnected after 29.2 ms.
With a controller, a tab switch brought 37 frames in 3 s, 34 of them with
distinct hashes.

## Emulator controller client

An emulator started with `-grpc <port> -grpc-use-token` writes the port and a
bearer token into its discovery file, `pid_<pid>.ini`. On macOS the file is in
`~/Library/Caches/TemporaryItems/avd/running`; `emulator::candidate_dirs` lists
the directories for each OS. `emulator::resolve(serial)` matches the file's
`port.serial` against the serial's console port, and the newest file wins.
`Controller::connect` opens two HTTP/2 connections, one for status, screenshots
and frames and one for touches, and reads the hardware through `getStatus`.

It handles the quirks spike S7 found:

- Scaling needs both dimensions. `scaled_size` sets both, the height following
  the display's aspect ratio (540x1200 on a 1080x2400 display).
- Raw rows arrive top-down, although the proto says bottom-up. `Frame::row(0)`
  is the top row.
- The display size is only in the deprecated `hardwareConfig`.
  `Hardware::from_status` reads it there unless `platformConfig` has it.
- Some emulator ports accept TCP and never answer. Every unary call carries a
  deadline, both as `grpc-timeout` and on the client side, and so do the
  connections and the opening of the stream. The stream then waits as long as
  `FrameStream::next` is told to, since a still screen sends nothing.
- `sendKey` accepts keys and drops them when the AVD has no keyboard. The
  client sends no keys. `Controller::keys_supported` reports `hw.keyboard` so
  the driver can route keys and text to the agent.
- The stream repeats unchanged frames, and the status bar clock changes on its
  own. `ChangeDetector` compares xxh3 hashes that leave out the top 6 % of rows.

When the endpoint cannot be used, `connect` and `resolve` return `Unavailable`.
Its `warning()` names the launch flag, `-grpc <console port + 3000>
-grpc-use-token`, and the console command that enables the endpoint without a
restart, `grpc <port>`, sent for example with `adb -s <serial> emu grpc <port>`.

### The controller proto and CI

`build.rs` generates the client from the proto that the installed emulator
ships, because the AOSP copy is older. It looks for the proto in this order:

1. `MOBILE_AGENT_EMULATOR_PROTO`, the path of a proto file;
2. `$ANDROID_HOME/emulator/lib/emulator_controller.proto`;
3. the same under `$ANDROID_SDK_ROOT`;
4. the same under the SDK's default location (`~/Library/Android/sdk` on macOS,
   `~/Android/Sdk` on Linux, `%LOCALAPPDATA%\Android\Sdk` on Windows).

`protoc` comes from protoc-bin-vendored. The crate was built and tested against
Emulator 37.1.11.0, whose proto has 1,865 lines and SHA-256
`1d62c6bcad5f06621f90ec2bf26c661ba769ccd0f1416b5314d25a68e04eee5f`.
`emulator::PROTO_SOURCE` records the path a build used.

CI needs the SDK's copy too, since a build without it fails and says how to
provide one. The actions/runner-images readmes list Android Emulator 37.1.11
on the macOS 26 image, under `$ANDROID_HOME`. The Ubuntu 24.04 image has the
SDK but not the emulator package, so the Linux job must install it before
building, with `sdkmanager emulator`, or point `MOBILE_AGENT_EMULATOR_PROTO` at
a copy of the SDK's proto.

### Vendored emulator proto

`proto/emulator_controller.proto` is a copy of the SDK's
`emulator/lib/emulator_controller.proto` from emulator 37.1.11 (SHA-256
`1d62c6bcad5f06621f90ec2bf26c661ba769ccd0f1416b5314d25a68e04eee5f`). The build
prefers the installed copy and falls back to this one with a warning, so hosts
without the emulator package build. Refresh it when the emulator updates: copy
the SDK file over it and record the new SHA here.

## Tests

Unit tests need no device:

```sh
cargo test -p android
```

The integration tests in `tests/` run when `MOBILE_AGENT_ANDROID_SERIAL` names a
device on the local adb server. Without it, each test prints
`skipped <test>: ...` and passes, which is what CI does. The emulator tests
need an emulator started with `-grpc <port> -grpc-use-token`.

```sh
eval "$(mise env)"   # ANDROID_HOME and the SDK tools
MOBILE_AGENT_ANDROID_SERIAL=emulator-5554 cargo test -p android -- --nocapture
```

What the adb integration tests do on the device:

- They push files named `/data/local/tmp/mobile-agent-it-*` and delete them,
  retrying the deletion if the emulator's adb transport reconnects meanwhile.
- They install forward and reverse rules on sockets named `mobile-agent-it-*`.
  Forward rules are removed first. The reverse rule is removed only after every
  connection through it has closed, because the adb server aborts when a
  device-side connection arrives for a reverse rule it has already dropped
  (spike S6).
- They start `sleep 60` and `tty` in shell streams and check that `sleep` ends
  when its stream closes.

What the agent deployment tests (`tests/agent_deploy.rs`) do on the device:

- They start agents from the jar `agent-android/build.sh` wrote, with a 10 s
  idle bound, and stop each one at the end; a test that fails first kills its
  agents by pid, checking `/proc/<pid>/cmdline` against their sockets.
- The replacement test starts the current dex with every `mobile-agent-android
  0.1.0` rewritten to `0.0.9` and the dex checksum recomputed, pushed as a bare
  dex, which `app_process` loads like a jar. The agent has no option to report
  another version.
- The conflict test runs `uiautomator events`, which holds UI automation for
  as long as it runs, and lets the forced attach stop it.
- The detach test connects to the agent as root through `su 0`, which the
  agent refuses and logs on its closed stdout; without `su` it skips that
  part.
- The tests hold UI automation for a few seconds each and run one at a time.

What the agent session tests (`tests/agent_session.rs`) do on the device:

- They start the Compose fixture (`am start -S`) on a tab, wait until it is
  in front, and force-stop it at the end.
- The full-stack test taps the Login tab through the mirror and kills its
  agent with SIGKILL; the others stop their agents.
- The controller test subscribes to frames and taps the List tab.

The device may be shared. Before a test the agent and session tests wait,
up to a minute, until no agent runs and nothing holds UI automation. A
session test that another user disturbs, by starting an agent or taking UI
automation (another agent's `--force` kills ours) or by moving the fixture,
runs again once the device is quiet, three times at most; a failure while
nobody else is on the device fails the test.

What the emulator integration tests do:

- They subscribe to frames and take raw screenshots, and they run `screencap`
  once to compare row order.
- They tap twice on the status bar, where a tap changes nothing: at (250, 50)
  and (300, 70) on a 1080-pixel-wide display. Two different points are needed
  because evdev drops a position equal to the previous contact's.
- `getevent -l` runs on a pseudo-terminal and watches the taps arrive. It
  receives SIGHUP when its stream closes, and the test checks that it is gone.

Measurements printed with `--nocapture`: `shell_round_trip` prints the p50, p95
and maximum of 100 `echo ok` round trips. `frames_arrive_scaled_to_the_working_width`
prints the first frame's latency and the frames of the next 3 s, which are none
when the screen is still. `a_touch_lands_on_the_touchscreen` prints the
`sendTouch` round trips. `raw_screenshots_are_full_resolution_and_top_down`
prints the p50 and p95 of 10 raw screenshots.

## Spec scenarios and tests

Requirement **Target discovery** (android-driver spec):

| Scenario or clause | Tests |
|---|---|
| Scenario "Unauthorized device" | `adb::tests::an_unauthorized_device_is_listed_with_instructions` (scripted adb server) |
| Unauthorized and offline targets are listed with that state | `adb::tests::devices_keep_every_state`, `adb::devices::tests::keeps_unauthorized_offline_and_authorizing_targets`, `adb::devices::tests::keeps_the_no_permissions_explanation` |
| An actionable message for each unusable state | `adb::devices::tests::unauthorized_advice_says_to_accept_the_prompt_on_the_device`, `adb::devices::tests::every_unusable_state_has_advice_and_an_online_device_has_none` |
| Serial, model, API level and emulator flag | `adb::devices::tests::an_online_target_takes_its_level_model_and_emulator_flag_from_getprop`, `adb::tests::a_target_whose_properties_cannot_be_read_stays_listed`, integration `targets_report_the_api_level_and_the_emulator_flag` |
| Agent state (absent, running, incompatible) | `agent::deploy::tests::no_agent_process_is_absent`, `agent::deploy::tests::an_agent_named_for_this_build_is_running`, `agent::deploy::tests::another_version_protocol_or_an_unknown_name_is_incompatible`, `agent::deploy::tests::a_compatible_agent_wins_over_an_incompatible_one`, `agent::launch::tests::agents_are_found_in_the_process_list_by_name_or_main_class`, integration `a_first_attach_starts_the_agent_and_installs_nothing` (absent, running, absent after stop) and `an_incompatible_agent_is_replaced_by_the_current_build` (incompatible) |

Requirement **Agent deployment without installation**:

| Scenario or clause | Tests |
|---|---|
| Scenario "First attach" | integration `a_first_attach_starts_the_agent_and_installs_nothing` |
| Scenario "Compatible agent reused" | integration `a_compatible_agent_is_reused_without_starting_a_process`; `agent::records::tests::records_in_a_directory_outlive_the_process_that_wrote_them` |
| Scenario "Incompatible agent replaced" | integration `an_incompatible_agent_is_replaced_by_the_current_build` (another build); `agent::deploy::tests::an_agent_on_an_older_protocol_is_incompatible` and `agent::connection::tests::an_agent_speaking_only_an_older_protocol_is_refused_with_version_mismatch` (an older protocol, refused so that the agent exits) |
| Started as the shell user, no package, no instrumentation, no prompt | integration `a_first_attach_starts_the_agent_and_installs_nothing` (uid 2000, package list unchanged, jar deleted); `agent::launch::tests::the_launch_command_keeps_the_token_off_the_command_line_and_detaches`, integration `the_jar_is_pushed_read_only` and `the_agent_outlives_its_launch_stream_and_a_closed_stdout` |
| Reuse only of an agent proven by its token | `agent::connection::tests::a_hello_with_another_token_is_refused`, `agent::deploy::tests::an_agent_that_does_not_answer_as_recorded_is_unreachable` |

Requirement **Single ownership of UI automation**:

| Scenario or clause | Tests |
|---|---|
| Scenario "Another tool holds automation" | integration `another_tool_holding_automation_is_a_conflict_naming_it_until_forced`; `agent::deploy::tests::a_conflict_names_the_holder_and_the_force_option`, `agent::launch::tests::a_conflict_line_names_the_holder_and_keeps_the_whole_message` |
| An agent another host started is a holder too | integration `an_agent_another_host_started_is_a_conflict_until_forced`; `agent::deploy::tests::an_agent_this_host_did_not_start_is_a_conflict_naming_it` |
| No takeover without the force option | integration `another_tool_holding_automation_is_a_conflict_naming_it_until_forced` and `an_agent_another_host_started_is_a_conflict_until_forced` (conflict first, takeover only with force, `Launched::took_over` naming the holder); `agent::launch::tests::force_and_extra_arguments_are_passed_quoted` |

Requirement **Agent lifecycle**:

| Scenario or clause | Tests |
|---|---|
| Scenario "Daemon shutdown" | integration `a_first_attach_starts_the_agent_and_installs_nothing` (`Attached::stop`: no agent process, no record) |
| Scenario "App restarted" | integration `the_same_agent_serves_the_mirror_after_the_app_is_force_stopped_and_relaunched` (session tests); agent side `lifecycle::the_same_agent_serves_snapshots_after_the_app_is_force_stopped_and_relaunched` (agent-android host tests) |
| A killed agent reported disconnected (device-protocol "Liveness detection") | integration `the_mirror_serves_compose_big_follows_a_tap_and_reports_a_killed_agent`; `session::tests::the_agent_going_away_is_reported_disconnected_with_a_reconnect_hint`, `session::events::tests::the_close_is_a_disconnection_with_a_reconnect_hint` |

The scenarios below are served by the agent, whose checks are the
agent-android host tests (`agent-android/README.md`); the driver passes them
through, and these tests check that it does.

Requirement **Tree acquisition across windows**:

| Scenario or clause | Tests |
|---|---|
| Scenario "Keyboard is included" | agent side `tree::the_keyboard_window_appears_with_its_keys_when_a_field_is_focused`; driver `session::tests::a_read_passes_the_agents_snapshot_through_with_the_scale_and_the_backend` (every window of the read, the keyboard's included, reaches the mirror as the agent sent it) |
| Scenario "Node limit reached" | agent side `tree::a_lower_node_cap_truncates_and_still_reports_the_full_count`; driver `session::tree::tests::reads_after_an_attach_action_or_resync_wait_for_idle_and_others_do_not` (the node and depth caps reach the request), `session::tests::a_read_passes_the_agents_snapshot_through_with_the_scale_and_the_backend` (node count and serialized nodes kept apart) |
| Every interactive window, with the attributes of each node and the window metadata | integration `the_mirror_serves_compose_big_follows_a_tap_and_reports_a_killed_agent` (573 nodes in the replica, every cell with a ref); agent side `metadata::a_permission_prompt_is_a_modal_window_owned_by_the_permission_controller`, `metadata::an_in_app_alert_is_a_modal_window_of_the_app_that_sends_no_change_events`; driver `session::tests::a_read_passes_the_agents_snapshot_through_with_the_scale_and_the_backend`, `session::tree::tests::event_less_windows_come_from_the_agents_metadata`, `session::tree::tests::the_scale_is_the_density_over_160_with_the_override_first` |

Requirement **Bounded idle waiting**:

| Scenario or clause | Tests |
|---|---|
| Scenario "Continuous animation" | agent side `idle::a_snapshot_during_an_endless_animation_returns_within_the_default_bound_plus_read_time`; driver `session::tree::tests::reads_after_an_attach_action_or_resync_wait_for_idle_and_others_do_not` (the agent's bound after an attach or action, none for reads the target's own changes trigger) |

Requirement **Change events**:

| Scenario or clause | Tests |
|---|---|
| Scenario "Single field changes" | agent side `events::editing_one_react_native_field_emits_events_naming_that_field_only`; driver `session::tests::events_reach_the_backends_stream_in_order`, `session::events::tests::protocol_events_and_sequence_notices_pass_through` (the event reaches the mirror, which reads only that window) |
| Sequence gaps and resynchronization | `session::tests::a_sequence_gap_reaches_the_mirror_and_the_stream_is_resynchronized` |
| Compose windows, which send none, read again after an action | integration `the_mirror_serves_compose_big_follows_a_tap_and_reports_a_killed_agent`; `session::tests::a_mirror_on_the_session_reads_a_compose_window_again_after_a_tap`, `session::tests::after_input_the_agent_did_not_inject_a_compose_read_empties_its_cache_first`, `session::tests::without_compose_foreign_input_only_delays_the_read` |

Requirement **Input injection**:

| Scenario or clause | Tests |
|---|---|
| Scenario "Tap acknowledged" | agent side `input::a_tap_is_acknowledged_as_accepted_within_50_ms`; integration `the_mirror_serves_compose_big_follows_a_tap_and_reports_a_killed_agent` (accepted through the mirror, 11.3 ms); `session::tests::input_goes_to_the_agent_in_pixels_and_returns_its_verdict`, `session::input::tests::acknowledgements_carry_the_platforms_verdict_and_the_text_method` |
| Taps, long presses, swipes with duration, keys, in pixels | `session::input::tests::taps_and_long_presses_go_out_in_pixels`, `session::input::tests::swipes_keep_their_duration_and_keys_their_code`, and on the controller `session::input::tests::a_swipe_on_the_controller_is_down_moves_and_up_along_the_line`, `session::input::tests::a_swipe_without_duration_still_moves_once` |

Requirement **Direct text setting with fallback**:

| Scenario or clause | Tests |
|---|---|
| Scenario "React Native text input" | agent side `text::a_focused_react_native_input_is_filled_directly_in_one_operation`; driver `session::input::tests::text_names_the_node_of_the_latest_read_and_its_method`, `session::tests::input_goes_to_the_agent_in_pixels_and_returns_its_verdict` (the direct method comes back) |
| Scenario "Element rejects direct setting" | agent side `text::a_focused_element_without_direct_setting_gets_key_events_and_the_fallback_is_named`; driver `session::input::tests::acknowledgements_carry_the_platforms_verdict_and_the_text_method` (the key method comes back) |

Requirement **Screenshot capture**:

| Scenario or clause | Tests |
|---|---|
| Scenario "Landscape screenshot" | agent side `screenshot::a_landscape_screenshot_has_landscape_dimensions_and_the_display_orientation`; driver `session::tests::screenshots_go_through_the_agent` |

Requirement **Emulator fast path** (android-driver spec):

| Scenario or clause | Tests |
|---|---|
| Scenario "Control endpoint enabled": taps and frames are served through it | integration `frames_arrive_scaled_to_the_working_width` and `a_touch_lands_on_the_touchscreen`; the status naming the fast path is task 5.5 |
| Scenario "Control endpoint disabled": a warning with the launch flag | integration `a_closed_port_falls_back_with_the_launch_flag_and_console_command`; `emulator::controller::tests::a_closed_port_is_unavailable_with_the_fallback_warning`, `emulator::endpoint::tests::no_discovery_file_is_unavailable`, `emulator::endpoint::tests::a_discovery_file_without_a_grpc_port_is_unavailable`; serving taps and screenshots through the agent is tasks 5.3 and 5.5 |
| The console can enable the endpoint at run time without a restart | `emulator::endpoint::tests::the_warning_names_the_launch_flag_and_the_console_command` |
| Keys and text go through the agent unless the emulator declares a keyboard | `emulator::hardware::tests::keys_go_through_the_controller_only_with_a_declared_keyboard`, integration `the_display_size_and_keyboard_match_the_avd` |
| The driver reports which path is active | task 5.5 |

Task 5.4's own checks:

| Check | Tests |
|---|---|
| Token discovery | `emulator::discovery::tests::*`, `emulator::endpoint::tests::a_discovery_file_with_a_grpc_port_resolves_to_an_endpoint`, integration `discovery_finds_the_endpoint_and_its_token` and `a_call_without_the_token_is_refused` |
| A second connection for input | `emulator::controller::tests::input_travels_on_a_second_connection` |
| RGB888 scaled with both width and height set | `emulator::hardware::tests::scaling_sets_both_width_and_height`, `emulator::hardware::tests::the_height_keeps_the_aspect_ratio_rounded_up`, `emulator::hardware::tests::no_scaling_requests_the_display_size_as_zeros`, integration `frames_arrive_scaled_to_the_working_width` |
| Raw `getScreenshot` | integration `raw_screenshots_are_full_resolution_and_top_down` |
| A deadline on every call | `emulator::controller::tests::a_silent_endpoint_fails_at_the_deadline` |
| Top-down rows | `emulator::frame::tests::rows_are_top_down`, `emulator::frame::tests::the_status_bar_is_at_the_start_of_the_buffer`, integration `raw_screenshots_are_full_resolution_and_top_down` |
| The display size from `hardwareConfig` | `emulator::hardware::tests::the_display_size_comes_from_the_lcd_keys`, `emulator::hardware::tests::the_status_falls_back_to_the_deprecated_hardware_config`, integration `the_display_size_and_keyboard_match_the_avd` |
| The frame-hash detector without the status bar | `emulator::frame::tests::a_change_in_the_status_bar_is_unchanged`, `emulator::frame::tests::one_changed_pixel_is_a_change_and_becomes_the_new_reference`, `emulator::frame::tests::a_dimension_change_is_a_change` |

Task 5.3's own checks: the frames in `session::controller::tests::*`
(status-bar-free hashes, capture time, clockwise rotation, area-average
scaling, JPEG size, the kept frames), integration
`controller_frames_arrive_on_the_event_stream_with_status_bar_free_hashes`,
and `session::tests::a_failed_read_keeps_the_agents_code`,
`session::tests::stop_asks_the_agent_to_shut_down`,
`session::tests::the_backend_is_taken_once`.

Task 5.2's own checks: the connection's reader thread in
`agent::connection::tests::*` (replies, concurrent callers, events before the
sink, pings while idle, the end of the stream), the launch parsers in
`agent::launch::tests::*` and the records in `agent::records::tests::*`.

Task 5.1's own checks against the emulator, in `tests/adb_device.rs`:
`listing_shows_the_device`, `shell_v2_keeps_the_exit_status_and_stderr_apart`,
`a_command_reading_stdin_sees_end_of_file`,
`closing_a_shell_stream_ends_the_command`,
`a_pty_stream_runs_the_command_on_a_terminal`,
`push_writes_the_bytes_with_the_mode`, `push_copies_a_local_file`,
`a_push_adbd_refuses_reports_its_message`, `a_forwarded_port_connects`,
`a_service_stream_reaches_a_live_abstract_socket`,
`a_service_stream_to_a_missing_socket_fails_with_closed`,
`a_reverse_adbd_refuses_reports_its_error` and
`a_missing_serial_fails_with_the_servers_message`.
