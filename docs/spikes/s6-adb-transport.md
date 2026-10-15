# S6: ADB transport

Spike S6 (tasks.md 2.9) measures the host-side ADB transport for design decision D3. The same operations are timed through two Rust client crates for the adb server protocol, through the `adb` binary, and through a small reference client written for the spike. Code: `spikes/s6-adb-transport/`, kept privately since the repository was made public (`docs/spikes/README.md`). Snapshot date: 2026-09-26.

## Setup

- **Host.** Apple M1 Max, 10 cores, 64 GB, macOS 26.6.2. Other work shared the host and the emulator throughout, running its own spikes, builds and Simulators (see Conditions).
- **Android.** platform-tools 37.0.1 (adb 1.0.41), one adb server on 127.0.0.1:5037 shared by every agent; Android Emulator 37.1.11.0; build-tools 37.0.0 for d8; platform android-36.
- **Target.** `emulator-5554`, AVD `pixel_api36` started with `-no-window -no-audio -no-boot-anim -grpc 8554 -grpc-use-token`. System image `google/sdk_gphone64_arm64/emu64a:16/BE2A.250530.026.F3/13894323:userdebug/dev-keys` (API 36, Android 16, security patch 2025-07-05), 4 vCPUs, 2.5 GB RAM, SELinux enforcing, adbd running as `shell`.
- **Columns.**
  - `adb_client`: the adb_client crate 3.2.3 with `default-features = false`, which drops the framebuffer feature and its `image` dependency.
  - `mozdevice`: Mozilla's mozdevice crate 0.5.5.
  - `adb binary`: `adb -s emulator-5554 ...` spawned once per operation; pushes also run as `adb binary -Z`, without compression.
  - `reference`: not a candidate. About 410 lines written for the spike from the protocol (`src/adapters/raw.rs`, `src/wire.rs`, and the listing parser in `src/devices.rs`), standard library only. It shows what the driver's own client would cost.
- **Toolchains.** Rust 1.95.0, release profile; Kotlin 2.4.20 on Temurin 21.0.12.
- **Payloads.** The agent jar that `agent-android/build.sh` builds at this commit, 663,251 bytes (one `classes.dex` of 2,277,628 bytes, build-tools 37.0.0). 10 MiB means 10,485,760 bytes of xorshift64* output, which adb's lz4, zstd and brotli cannot compress.
- **Echo server.** `kotlin/src/placeholder/s6/Main.kt`, built by `kotlin/build.sh` with the agent-android recipe (kotlinc against android-36, d8 with the stdlib, a jar holding one dex of 2,285,408 bytes; the jar is 666,480 bytes), started with `app_process` as the shell user. `--listen NAME` accepts on the abstract socket NAME. `--dial localabstract:NAME` or `--dial tcp:PORT` connects out, redials after every session, and polls every 200 µs while nothing listens. Every connection gets a hello frame first, then frames (a u32 big-endian length and the payload) are echoed unchanged.
- **Tests.** 38 unit tests for the percentiles, the `adb devices -l` parser, the frame echo protocol and the smart-socket helpers were committed first and failed (`0 passed; 38 failed`, commit `5db57e8`), then passed. Three more were each seen red, then green: two cover the host-quiet filter, and one covers the reverse-listing matcher. `cargo test` runs 41.
- **Spike commits.** All measurements come from `153cc42`, which the harness recorded in both runs below. `c44cb13` then changes only how reverse rules are torn down, outside the timed sections (see the adb server abort under Forward versus reverse).
- **Method.** Each candidate runs 10 warm-up iterations per operation, which are discarded. Then 200 iterations are timed for the listing, the shell and the 64-byte echoes, and 100 for everything else. Candidates alternate round robin inside each iteration, so load drift reaches every column alike. After each push operation, `sha256sum` on the device checks every candidate's file. Before every 10 iterations a gate checks for an external burst: 1-minute load under 10, the emulator's qemu process under 300% CPU, and no gradle, xcodebuild or kotlinc process. It pauses until all three hold, for at most 20 minutes per operation. The thresholds sit above what the harness causes itself (qemu up to 223% during the 10 MiB pushes) and below the bursts other work caused (load up to 660, qemu up to 760%).
- **Rerun**, with the Android SDK on PATH and the emulator booted:

```sh
cd spikes/s6-adb-transport          # in the private copy
kotlin/build.sh                   # echo server jar into target/kotlin/
../../agent-android/build.sh      # the agent jar used as a push payload
cargo test                        # 41 unit tests
cargo build --release
./target/release/s6-adb-transport run       # summary.md and samples.csv in target/s6-results/<time>/
./target/release/s6-adb-transport probe     # forward and reverse checks, error propagation, SELinux
./target/release/s6-adb-transport listing   # odd device listings from a fake adb server
```

`cargo build --features usb` also compiles the direct-USB adapter, which never ran (see Direct USB).

### Conditions

`uptime` and `pgrep -fl "gradle|xcodebuild|kotlinc"` were checked before every run. That pattern also matches other work's `zsh -c` wrappers whose text only mentions a build, including their own wait loops. The gate therefore ignores `sh`, `bash` and `zsh -c` wrappers and continuation lines; a real build still matches through its own xcodebuild, kotlinc or Gradle java process. Earlier timing attempts were stopped or discarded:

- three were stopped while waiting for a calm host, with other work pushing the load up to 180 and qemu up to 750%;
- one completed but saw load 116 to 660 during its later operations, so it is not reported;
- one ended when the emulator's adb transport dropped.

These two runs are reported:

| Run | Start | Gate pauses | 1-minute load | Emulator qemu CPU | Transport |
|---|---|---|---|---|---|
| 7, the tables below | 23:11 | none | 4.3 to 5.7 | 19% to 223% | `transport_id:5` throughout |
| 6, the stability check | 22:56 | one, 490 s, before the first operation | 5.4 to 9.8 | 4% to 92% | reconnected during `forward-echo`, whose samples were discarded |

The emulator's adb transport reconnected four times during the run (`transport_id` 1 to 5), at 21:53, 22:53, 23:01 and 23:05. Each time, the adb server logged `emulator-5554: connection terminated: read failed`. For the last two, whose device log was still available, adbd logged `connection terminated: write failed`. The 23:01 reconnect happened while this spike's harness was paused and sent nothing. The other three coincided with activity from this spike and from other work, and their cause is not known. The harness now records a failed operation, discards its partial samples, waits for the device and goes on.

This spike also aborted the shared adb server once, at 23:13:19, during a `probe` run (see "The adb server can abort on reverse" below). The next adb command started a new server. Every client of the old server, in every agent, lost its connections and its forward rules at that moment.

## Measurements

All times are milliseconds from run 7. The Stability section compares medians with run 6.

### Device listing, `host:devices-l`, 200 samples

| | adb_client | mozdevice | adb binary | reference |
|---|---:|---:|---:|---:|
| p50 | 16.66 | 0.47 | 16.73 | 0.51 |
| p95 | 18.73 | 0.78 | 18.68 | 0.79 |
| max | 29.81 | 1.18 | 39.87 | 1.18 |

adb_client lists through `ADBServer`. Before every connection to a loopback server, `ADBServer::connect` runs `adb start-server` (`server/adb_server.rs`), so a listing costs as much as the binary.

### Shell round trip, `echo ok`, 200 samples

| | adb_client | mozdevice | adb binary | reference |
|---|---:|---:|---:|---:|
| p50 | 6.78 | 5.85 | 24.27 | 5.71 |
| p95 | 9.87 | 9.28 | 34.43 | 9.16 |
| max | 18.49 | 16.93 | 57.61 | 13.93 |

adb_client opens two server connections per command: one for `host:features`, then one for `shell,v2`. mozdevice uses shell v1, which returns no exit status and merges stderr into stdout. The reference client sends one `shell,v2,raw:` request.

### Push of the agent jar, 663,251 bytes, 100 samples

| | adb_client | mozdevice | adb binary | adb binary -Z | reference |
|---|---:|---:|---:|---:|---:|
| p50 | 10.44 | 19.41 | 31.80 | 31.97 | 9.88 |
| p95 | 13.86 | 23.80 | 37.11 | 37.80 | 12.66 |
| max | 53.35 | 70.37 | 102.88 | 96.54 | 20.11 |
| MiB/s at p50 | 60.6 | 32.6 | 19.9 | 19.8 | 64.0 |

### Push of 10 MiB of random bytes, 100 samples

| | adb_client | mozdevice | adb binary | adb binary -Z | reference |
|---|---:|---:|---:|---:|---:|
| p50 | 38.32 | 50.80 | 62.56 | 63.30 | 35.14 |
| p95 | 65.45 | 66.02 | 75.67 | 84.12 | 68.38 |
| max | 146.87 | 115.25 | 132.52 | 152.21 | 128.97 |
| MiB/s at p50 | 261.0 | 196.8 | 159.8 | 158.0 | 284.6 |

adb_client and the reference client both send sync v1 `SEND` in 64 KiB chunks. Before every push, mozdevice runs `ls` on the destination's parent directory through the shell. It then sends 32 KiB chunks, each as three separate writes on a socket without `TCP_NODELAY`. The binary's default and `-Z` pushes stay within noise of each other on both payloads, so compression is not what the binary pays for. Its gap to the protocol clients matches the process cost visible in the listing.

### Forward to the agent's abstract socket, 100 samples

Forward setup plus first connect: `forward tcp:<port> localabstract:<name>`, a TCP connect to that port, and reading the agent's hello.

| | adb_client | mozdevice | adb binary | reference |
|---|---:|---:|---:|---:|
| p50 | 3.28 | 2.79 | 22.30 | 3.08 |
| p95 | 4.10 | 3.51 | 24.88 | 3.58 |
| max | 5.81 | 6.66 | 95.62 | 4.04 |

At p50, the forward setup alone takes 1.09, 0.59, 19.87 and 0.74 ms, and the connect plus hello takes 2.17, 2.16, 2.35 and 2.35 ms.

A service stream needs no forward rule: a new server connection sends `host:transport:<serial>`, then `localabstract:<name>`. Only the reference client can open one. With the hello it took p50 2.52, p95 3.06 and max 4.00 ms.

### Echo round trips on an open connection

64-byte frames, 200 samples. Each of the first four columns uses a forward set up by that candidate; the last uses a service stream.

| | adb_client's forward | mozdevice's forward | the binary's forward | reference's forward | service stream |
|---|---:|---:|---:|---:|---:|
| p50 | 1.59 | 1.58 | 1.56 | 1.59 | 1.54 |
| p95 | 2.74 | 2.66 | 2.45 | 2.57 | 2.39 |
| max | 9.18 | 5.70 | 4.63 | 3.80 | 5.15 |

64 KiB frames, 100 samples:

| | adb_client's forward | mozdevice's forward | the binary's forward | reference's forward | service stream |
|---|---:|---:|---:|---:|---:|
| p50 | 2.69 | 2.70 | 2.69 | 2.69 | 2.64 |
| p95 | 3.92 | 3.57 | 3.58 | 3.49 | 3.54 |
| max | 4.67 | 4.63 | 3.74 | 4.05 | 4.18 |

Through one reverse connection that the reference client set up, 64 B took p50 1.65, p95 2.52 and max 40.70 ms (200 samples), and 64 KiB took p50 3.46, p95 4.28 and max 5.58 ms (100 samples). Whoever installs the rule, data flows through the adb server, and the columns agree within noise.

### Reverse with the agent dialing out, 100 samples

Reverse setup, connect-out and first echo: `reverse localabstract:<name> tcp:<port>` to a fresh host listener, accepting the agent's connection, reading the hello, and one 64-byte echo. The rule is removed before the connection closes, so the agent's redial never lands in the next sample.

| | adb_client | mozdevice | adb binary | reference |
|---|---:|---:|---:|---:|
| p50 | 5.70 | 5.76 | 23.85 | 5.73 |
| p95 | 7.88 | 6.62 | 26.52 | 6.45 |
| max | 20.46 | 29.44 | 96.14 | 7.72 |

At p50 the parts were:

- setup alone: 2.47, 2.37, 21.02 and 2.32 ms;
- accept plus hello: 1.68, 1.76, 1.18 and 1.75 ms;
- first echo: 1.62, 1.63, 1.66 and 1.66 ms.

The split between the first two moves with each client: the binary returns only after its process exits, by which time the agent has often connected, and adb_client returns before adbd confirms (see Client behaviour). Compare the combined figure. The agent's 200 µs dial poll adds at most 0.2 ms.

### Stability

p50 in run 7 / run 6, in ms:

| Operation | adb_client | mozdevice | adb binary | reference |
|---|---:|---:|---:|---:|
| Device listing | 16.66 / 17.00 | 0.47 / 0.46 | 16.73 / 17.10 | 0.51 / 0.47 |
| Shell `echo ok` | 6.78 / 6.12 | 5.85 / 5.29 | 24.27 / 23.77 | 5.71 / 5.11 |
| Push, agent jar | 10.44 / 8.72 | 19.41 / 16.91 | 31.80 / 29.67 | 9.88 / 8.33 |
| Push, 10 MiB | 38.32 / 36.36 | 50.80 / 47.92 | 62.56 / 61.27 | 35.14 / 35.40 |
| Forward setup + first connect | 3.28 / 2.87 | 2.79 / 2.46 | 22.30 / 20.48 | 3.08 / 2.86 |
| Reverse setup + connect-out + echo | 5.70 / 5.48 | 5.76 / 5.35 | 23.85 / 22.49 | 5.73 / 5.43 |

For the other rows, run 7 / run 6: `adb binary -Z` pushes took 31.97 / 29.45 and 63.30 / 61.78 ms. The service stream plus hello took 2.52 / 2.28 ms. Echoes through reverse took 1.65 / 1.60 ms at 64 B and 3.46 / 2.90 ms at 64 KiB. Run 6 has no forward echoes (see Conditions).

### Forward versus reverse on API 36

The `probe` subcommand ran three times: before `153cc42` was committed, at `153cc42`, and at `c44cb13`. Quoted figures come from the run at `153cc42`. The outcomes below held in all three, except where noted.

- **Forward to an abstract socket owned by a shell-user process works.** The echo server ran as user `shell` in `u:r:shell:s0`, and its socket appeared in `/proc/net/unix` as `@s6probe_f_94591`. With SELinux enforcing, forwards from all four clients delivered the hello and a 64-byte echo. Logcat recorded no `avc:` denial mentioning adbd, shell or the spike while the probe ran. That settles risk 10 of `docs/research/android-agent.md` for API 36: adbd may connect to a shell-owned abstract socket. No API 37 (Android 17) image was available, so D3's question about Android 17's policy stays open.
- **A successful connect through a forward proves nothing.** With nothing listening on the device, a TCP connect to the forward port succeeded in 0.32 ms, and the connection closed without a byte 1.9 ms after the connect began. This is why the agent speaks first.
- **A service stream fails explicitly.** adb's `docs/dev/services.md` documents opening `localabstract:<name>` as a device service after `host:transport:<serial>`. Against the live socket this took 2.1 ms with the hello. Against a missing socket, the server answered `FAIL` with the message `closed` in 1.8 ms.
- **Reverse works as well.** With the echo server dialing `localabstract:<name>`, `reverse localabstract:<name> tcp:<port>` from all four clients delivered the hello and an echo. So did `reverse tcp:<port> tcp:<port>` through mozdevice's typed API, with the agent dialing a device TCP port.
- **Costs at p50.** Forward setup plus first connect takes about 3 ms through a protocol client. A service stream plus hello takes 2.5 ms. Reverse setup plus connect-out takes about 4.1 ms (5.7 ms with the first echo). Echoes cost the same over either: about 1.6 ms at 64 B and 2.6 to 3.5 ms at 64 KiB.
- **Lifecycle.** With reverse, the agent is the side that connects. A resident agent that later daemon sessions reuse (the spec's "Compatible agent reused") would have to keep dialing, or be woken, to reach each new daemon. With forward, or with a service stream, the daemon connects to a listening agent whenever it needs to. Forward rules live in the adb server and reverse rules in adbd, and both belong to a transport that reconnected four times in this run *(inference: the rules are dropped with their transport)*. A service stream has no rule to reinstall.

#### The adb server can abort on reverse

The server records a transport's reverse destinations when it relays `reverse:forward`. It erases one as soon as it relays `reverse:killforward`, before adbd has processed the request. When the device then opens a connection toward a host address missing from that set, `handle_packet` in `adb.cpp` does not refuse it: it calls `LOG(FATAL)`, and the server process dies (`IsReverseConfigured` and `UpdateReverseConfig` are in `transport.cpp`).

That is what happened at 23:13:19, in the second probe's check of mozdevice's typed `reverse_port`. The probe closed the agent's connection and then removed the rule with `kill_reverse_port`. The agent redialed as soon as it saw the close and reached adbd, which was still listening. The server had already dropped the destination. It logged `handle_packet disallowed connect to tcp:54324 from emulator-5554` at fatal level. 34 ms later the next adb command, very likely the probe's own `adb reverse --list`, started a new server.

adb_client's `reverse_remove` opens the same window, because it returns before adbd confirms. It did not trigger an abort in about 330 adb_client iterations of the timed reverse operation across three runs. Commit `c44cb13` keeps the agent's session open until adbd's `reverse:list-forward` no longer lists the rule. The same probe and a reverse-only run have not aborted the server since.

For the design, reverse therefore carries a hazard that forward does not. Removing or rebinding a rule while the agent may dial can take down the host's adb server and, with it, every other tool's connections. The same presumably applies when another tool runs `adb reverse --remove-all` *(inference from the same code path)*. Forward connections and service streams are opened by the host, so this check never runs for them.

### Client behaviour

adb_client 3.2.3:

- `ADBServer` runs `adb start-server` before every connection to a loopback server, so each listing spawns a process: 16.66 ms at p50, against 0.51 ms for the protocol.
- `shell_command` asks for `host:features` on a separate connection before every command: 6.78 ms at p50, against 5.71 ms.
- `reverse` reads only the server's first `OKAY`. For `reverse tcp:70000 tcp:1`, adbd answered `cannot bind listener: bad port number '70000'`, but adb_client returned success. The other three clients returned that error.
- `devices_long` fails the entire listing with `Regex parsing error: missing field` when one target is in the Linux `no permissions (...)` state. Unauthorized and offline targets list correctly. The `listing` subcommand fed the same lines from a fake server to every client.
- `forward` sends `host:forward:...` after `host:transport:<serial>` rather than `host-serial:<serial>:forward:...`. A check sent `host-usb:forward:...` after the same switch, with no USB device attached, and the forward was bound to the emulator. The server keeps the switched transport, so this is safe with several devices.
- With default features off, its dependency tree still has 84 crates, including rustls, ring, rsa and rcgen, which serve its direct device-protocol paths (RSA authentication, and TLS for wireless debugging).

mozdevice 0.5.5:

- `Device::new`, which `Host::device_or_default` also calls, runs `id`, `su 0 id` and `su -c id`. On a rooted device it then runs `setenforce permissive` and prefixes every later shell command with `su 0`. `su 0 id` returns uid 0 on this userdebug emulator. The harness therefore builds `Device` as a struct literal and never called the constructor on the shared emulator.
- `Host::devices` keeps only targets in the `device` state: 2 of the fake server's 5 targets.
- `forward_port` and `reverse_port` take TCP ports only. `localabstract:` endpoints worked through the public `Host::execute_command` and `Device::execute_host_command`, sending the same request strings as the binary; no fallback to the binary was needed.
- `kill_reverse_port` removes the rule but reports `adb server response did not contain expected hexstring length`. The one run where it reported `failed to fill whole buffer` was the run in which the server aborted mid-request.
- `push` runs `ls` on the parent directory before every transfer (19.41 ms at p50 for the jar, against 10.44 ms for adb_client). Shell is v1, and read and write timeouts default to 2 s. Its dependency tree has 29 crates.

adb binary 1.0.41: at p50 each operation costs 16 to 22 ms more than through the reference client:

- listing +16.2 ms;
- shell +18.6 ms;
- forward setup +19.1 ms;
- reverse setup +18.7 ms;
- jar push +21.9 ms.

Process creation also adds maxima near 100 ms.

No candidate needed the binary for `forward` or `reverse`.

| Client | Forward to `localabstract:` | Reverse to `localabstract:` |
|---|---|---|
| adb_client | typed API | typed API; device-side errors lost |
| mozdevice | public raw request; typed API is TCP-only | public raw request; typed API is TCP-only |
| adb binary | native | native |
| reference | `host-serial:<serial>:forward:...` | `reverse:forward:...`, reads adbd's status |

### Direct USB: not measured

No physical device was attached, and an emulator has no USB side, so the direct device-protocol path could not be timed. An adapter over adb_client's `ADBUSBDevice` compiles behind `--features usb` (`src/adapters/usb.rs`; `cargo clippy --features usb` is clean) and never ran. The emulator does expose the same device protocol over TCP on its adbd port (5555), but that path was not tried. The shared adb server already holds that transport, and a second host connection could have disconnected the other work's clients. As D3 notes, direct USB also cannot coexist with an adb server that has claimed the device.

## Verdict

**Go, with two changes to D3.** The driver speaks the adb server protocol from Rust, as D3 proposes, but through a small client of its own in `crates/android` rather than a client crate. Neither adb_client nor mozdevice is taken as a dependency. The agent is reached through a service stream rather than a forward rule. The `adb` binary stays out of the hot path.

- **The binary is rejected, now with numbers.** Every operation pays for a process: 16 to 22 ms more at p50 and maxima near 100 ms. At p50, against the protocol, a shell command is 4 times slower, a listing 33 times, a forward setup 27 times and a reverse setup 9 times.
- **The protocol path is settled; the crates are not.** Wherever the crates do the same work as the reference client, they are as fast. But each has behaviour that conflicts with the spec or with a shared device:
  - adb_client puts the binary back into listing, reports a refused `reverse` as success, and loses the whole listing when one target has no permissions; it also brings 84 crates.
  - mozdevice drops unauthorized and offline targets, which Target discovery requires listing. It can switch the device to SELinux permissive from its constructor. Its typed tunnels are TCP-only and its shell is v1.
  - The subset the driver needs took about 410 lines without dependencies: listing with every state, shell v2, sync `SEND`, forward, reverse with adbd's status, and service streams. It came within 0.3 ms of the faster crate on every operation at p50 and was faster on shell and both pushes.
- **The agent link is a service stream.** The driver opens `localabstract:<name>` through the server whenever it needs a connection. There is no forward rule and no host port, the open fails explicitly when the agent is gone, and it costs 2.5 ms with the hello. The agent keeps speaking first. `adb forward` costs the same if a rule is ever wanted.
- **Reverse is the fallback, with rules.** It works on API 36. It stays available for a build whose policy refuses adbd connections to a shell-owned socket, which is unverified for Android 17 and OEM builds. It costs 1 to 1.5 ms more per connection setup and needs an agent that dials. A race around its rules can also abort the shared adb server. If it is used:
  - the agent dials only when the daemon asks it to, with no redial loop;
  - rules are removed only after the agent's connections are closed and it has stopped dialing.
- **Direct USB stays deferred**, unmeasured because no device was attached.

For the tasks this gates: task 5.1 implements this client with the tests that S6's unit tests seed (the listing parser with every state, frames, shell v2 and sync). S1, or a later run with a physical device and an API 37 image, should repeat `probe` on those targets.

### Proposed D3 text

Replacement for D3 in design.md, recording the numbers. design.md itself is not edited here.

> ### D3. ADB through the local server protocol, spoken by the driver's own client; no `adb` binary in the hot path
> The driver speaks the adb server protocol on port 5037 with a small client in `crates/android`. It opens one server connection per request and uses:
>
> - `host:devices-l`, parsed so that every state is kept, including unauthorized, offline and `no permissions`;
> - `shell,v2,raw:` for the exit status and separate stderr;
> - sync `SEND` in 64 KiB chunks, each header written with its data;
> - `host-serial:<serial>:forward` and `reverse:forward`, reading through the second status so device-side failures surface.
>
> The driver reaches the agent by opening `localabstract:<name>` as a device service after `host:transport:<serial>`. No forward rule or host port exists to leak or to reinstall after a transport reconnect, and a missing agent fails the open with `closed` instead of yielding an empty connection. Spike S6 (`docs/spikes/s6-adb-transport.md`) measured these p50 and p95 times on the API 36 emulator, own client against the `adb` binary:
>
> - device listing: 0.51 and 0.79 ms, against 16.73 and 18.68 ms;
> - `echo ok` shell round trip: 5.71 and 9.16 ms, against 24.27 and 34.43 ms;
> - push of the 663 KB agent jar: 9.88 and 12.66 ms, against 31.80 and 37.11 ms;
> - 10 MiB push: 35.1 and 68.4 ms (285 MiB/s at p50), against 62.6 and 75.7 ms;
> - service stream plus hello: 2.52 and 3.06 ms;
> - forward setup plus first connect: 3.08 and 3.58 ms, against 22.30 and 24.88 ms;
> - reverse setup, connect-out and first echo: 5.73 and 6.45 ms, against 23.85 and 26.52 ms.
>
> However the connection was made, an echo round trip took 1.5 to 1.7 ms at 64 B and 2.6 to 3.5 ms at 64 KiB at p50.
>
> Alternatives:
>
> - the `adb` binary (rejected: a process per operation, 16 to 22 ms more at p50 with maxima near 100 ms);
> - the adb_client crate (rejected: its `ADBServer` runs `adb start-server` before every connection, its `reverse` reports success when adbd refuses, its `devices_long` fails the whole listing on one `no permissions` target, and it brings 84 crates, including rustls and RSA, for paths the driver does not use);
> - the mozdevice crate (rejected: `Device::new` runs `setenforce permissive` on rooted and userdebug targets, `devices` drops every target not in the `device` state, the typed forward and reverse API is TCP-only, and shell v1 loses the exit status);
> - direct USB via the ADB device protocol (not measured, since no device was attached; deferred because it conflicts with a running adb server over USB claims; kept as an option for hosts without an adb server).
>
> On API 36 with SELinux enforcing, adbd connects to an abstract socket owned by the shell-user agent, both through `adb forward` and as a service stream, and logs no denial. `adb reverse` with the agent dialing out also works and remains the fallback for a build that refuses the forward direction; Android 17 and OEM builds are unverified. The fallback is constrained. When a device-side connection arrives for a reverse destination the server has already dropped, the adb server aborts (`LOG(FATAL)` in `handle_packet`), which S6 triggered once by closing the agent's connection just before removing its rule, so that the agent's redial raced the removal. The agent therefore dials only when asked, and rules are removed only after its connections are closed. The agent listens on an abstract Unix socket named with a per-launch id and speaks first, and the handshake carries a per-launch token. It never opens a TCP port, unlike openatx's server on 0.0.0.0:9008.
