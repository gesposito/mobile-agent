# android

Host-side Android driver (design D11). So far it holds:

- `adb`: the driver's own client for the adb server protocol (design D3, task 5.1).
- `emulator`: the client for the emulator's gRPC controller, the emulator fast
  path (design D7, task 5.4).

Agent deployment, the agent session and path selection (tasks 5.2, 5.3 and
5.5) come later.

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
| Agent state (absent, running, incompatible) | task 5.2 |

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
\n\n## Vendored emulator proto\n\n`proto/emulator_controller.proto` is a copy of the SDK's `emulator/lib/emulator_controller.proto` from emulator 37.1.11 (SHA-256 `1d62c6bcad5f06621f90ec2bf26c661ba769ccd0f1416b5314d25a68e04eee5f`). The build prefers the installed copy and falls back to this one with a warning, so hosts without the emulator package build. Refresh it when the emulator updates: copy the SDK file over it and record the new SHA here.\n