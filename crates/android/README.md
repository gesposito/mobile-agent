# android

Host-side Android driver (design D11). So far it holds:

- `adb`: the driver's own client for the adb server protocol (design D3, task 5.1).

Agent deployment and the agent session (tasks 5.2 and 5.3) come later.

## adb client

`adb::AdbServer` talks to the local adb server (127.0.0.1, port 5037 or
`ANDROID_ADB_SERVER_PORT`) with one connection per request. It never spawns the
`adb` binary. Each connection has a 2 s connect deadline and a 10 s read and
write deadline, which `AdbServer::with_io_timeout` changes. The client is
blocking, so async callers run it on a blocking thread.

| Operation | Protocol |
|---|---|
| `devices`, `targets` | `host:devices-l`, keeping every state; `targets` adds `getprop` facts for online targets and advice for the others |
| `Device::shell`, `shell_stream` | `shell,v2,raw:` with exit status and separate stderr; `shell` closes stdin at once |
| `Device::push`, `push_bytes` | sync `SEND` in 64 KiB chunks, each header written with its data, then `DONE` and adbd's status |
| `Device::forward`, `forward_tcp`, `remove_forward`, `forwards` | `host-serial:<serial>:forward:...`, reading the second status and the port bound for `tcp:0` |
| `Device::reverse`, `remove_reverse`, `reverses` | `reverse:forward:...` on the device transport, reading adbd's own status |
| `Device::open_service`, `open_abstract` | `host:transport:<serial>`, then the service, with no rule; a missing listener fails with `closed` |

The integration tests confirmed three behaviours on the API 36 emulator:

- adbd copies the owner's permission bits of a pushed file to group and other,
  so 0o644 arrives as 0666 and 0o755 as 0777. The `adb` binary's pushes do the
  same.
- Dropping a `ShellStream` ends its command, because adbd sends it SIGHUP.
- A reverse rule makes adbd listen on its device side. The integration tests
  use this to get a live abstract socket with no process on the device.

## Tests

Unit tests need no device:

```sh
cargo test -p android
```

The integration tests in `tests/` run when `MOBILE_AGENT_ANDROID_SERIAL` names a
device on the local adb server. Without it, each test prints
`skipped <test>: ...` and passes, which is what CI does.

```sh
eval "$(mise env)"   # ANDROID_HOME and the SDK tools
MOBILE_AGENT_ANDROID_SERIAL=emulator-5554 cargo test -p android -- --nocapture
```

What the adb integration tests do on the device:

- They push files named `/data/local/tmp/mobile-agent-it-*` and delete them.
- They install forward and reverse rules on sockets named `mobile-agent-it-*`.
  Forward rules are removed first. The reverse rule is removed only after every
  connection through it has closed, because the adb server aborts when a
  device-side connection arrives for a reverse rule it has already dropped
  (spike S6).
- They start `sleep 60` in a shell stream and check that it ends when the
  stream closes.

`shell_round_trip` prints the p50, p95 and maximum of 100 `echo ok` round trips.

## Spec scenarios and tests

Requirement **Target discovery** (android-driver spec):

| Scenario or clause | Tests |
|---|---|
| Scenario "Unauthorized device" | `adb::tests::an_unauthorized_device_is_listed_with_instructions` (scripted adb server) |
| Unauthorized and offline targets are listed with that state | `adb::tests::devices_keep_every_state`, `adb::devices::tests::keeps_unauthorized_offline_and_authorizing_targets`, `adb::devices::tests::keeps_the_no_permissions_explanation` |
| An actionable message for each unusable state | `adb::devices::tests::unauthorized_advice_says_to_accept_the_prompt_on_the_device`, `adb::devices::tests::every_unusable_state_has_advice_and_an_online_device_has_none` |
| Serial, model, API level and emulator flag | `adb::devices::tests::an_online_target_takes_its_level_model_and_emulator_flag_from_getprop`, `adb::tests::a_target_whose_properties_cannot_be_read_stays_listed`, integration `targets_report_the_api_level_and_the_emulator_flag` |
| Agent state (absent, running, incompatible) | task 5.2 |

Task 5.1's own checks against the emulator, in `tests/adb_device.rs`:
`listing_shows_the_device`, `shell_v2_keeps_the_exit_status_and_stderr_apart`,
`a_command_reading_stdin_sees_end_of_file`,
`closing_a_shell_stream_ends_the_command`, `push_writes_the_bytes_with_the_mode`,
`push_copies_a_local_file`, `a_push_adbd_refuses_reports_its_message`,
`a_forwarded_port_connects`, `a_service_stream_reaches_a_live_abstract_socket`,
`a_service_stream_to_a_missing_socket_fails_with_closed`,
`a_reverse_adbd_refuses_reports_its_error` and
`a_missing_serial_fails_with_the_servers_message`.
