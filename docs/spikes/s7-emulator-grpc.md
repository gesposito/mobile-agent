# S7: emulator gRPC controller

Spike for task 2.10 of `mobile-automation-foundation`. It checks the android-driver requirement "Emulator fast path" and design D7: can the emulator's gRPC controller (`android.emulation.control.EmulatorController`), found through the emulator's own discovery file and token, carry taps and frames, and does that path meet the harness gate "tap-to-frame-change p95 under 50 ms on an emulator"? The code is in `spikes/s7-emulator-grpc/`, kept privately since the repository was made public (`docs/spikes/README.md`).

**In brief.** The controller works as the fast path. A tonic client generated from the proto that ships with the installed emulator connected through the discovery file's port and bearer token. sendTouch is acknowledged in 0.49 ms at p50 and 0.74 ms at p95. Frames arrive only when the guest posts one, at up to 46 fps under a continuous drag at 540 px wide, and 15 to 30 ms after the emulator timestamps them. The tap-to-frame-change gate is not met on this AVD: p95 is 147 ms at 540 px and 155 ms at native size. About 80 % of each trial passes before the emulator has the changed frame. The guest renders in software (`gpu auto` resolved to lavapipe and SwiftShader), so the time is spent in the guest, not on the control path. An emulator started without `-grpc` wrote no discovery file and served no controller, and the client fell back with the launch-flag warning. The console's `grpc <port>` command can start a token-protected endpoint on such an emulator at run time. Verdict: go for the fast path, no-go for the gate on this configuration.

## Setup

| Item | Value |
|---|---|
| Host | Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2 (25G83) |
| Emulator | 37.1.11.0 (build 15917651), `qemu-system-aarch64-headless` on HVF |
| AVD | `pixel_api36`: Pixel 8 profile, `google_apis` arm64-v8a API 36 (`BE2A.250530.026.F3`), 1080x2400 at 420 dpi and 60 Hz, 4 vCPUs, 2,560 MB, `hw.keyboard=no`, gesture navigation |
| Guest GPU | `hw.gpu.mode=auto`, which resolved to software rendering: `hardware-qemu.ini` says `lavapipe`, SurfaceFlinger reports ANGLE on SwiftShader Vulkan |
| Launch | `-avd pixel_api36 -no-window -no-audio -no-boot-anim -grpc 8554 -grpc-use-token` |
| Proto | `$ANDROID_HOME/emulator/lib/emulator_controller.proto`, 1,865 lines, SHA-256 `1d62c6bcad5f06621f90ec2bf26c661ba769ccd0f1416b5314d25a68e04eee5f`, importing only `google/protobuf/empty.proto`; the AOSP prebuilts copy has 1,397 lines (protocol research note) |
| Client | Rust 1.95.0; tonic and tonic-prost 0.14.6, prost 0.14.4, tonic-prost-build 0.14.6 running libprotoc 31.1 from protoc-bin-vendored 3.2.0; hyper 1.11.1, h2 0.4.19, tokio 1.53.1; xxhash-rust 0.8.18 |
| Spike code | `787e113` (client and discovery) and `bcb6f1b` (measurements). Every kept result comes from this Rust code; the touch run used an earlier revision of `measure.sh` with the same command line. `negative.sh` runs the negative path. |

To rerun, in the private copy: `cd spikes/s7-emulator-grpc && eval "$(mise env)" && ./measure.sh emulator-5554`, then `./negative.sh`. `build.rs` compiles the proto found under `$ANDROID_HOME`, or the one named by `S7_EMULATOR_PROTO`. `results/` holds the output of every kept run, the raw samples as CSV, and `host.txt`, which logs each run's starting and ending load, repeats included.

**Method.**

- Channel: TCP_NODELAY; HTTP/2 stream window 32 MiB, connection window 64 MiB and frames up to 16 MiB (`--h2 tuned`, the default; one comparison run uses hyper's 2 MiB and 5 MiB defaults). The message limit is 64 MiB, because a native RGBA8888 frame is 10.4 MB and tonic's default is 4 MB.
- Timings are host wall-clock times with nearest-rank percentiles. Warm-up is excluded: 10 taps for `touch`, 3 trials for `tap-to-frame`, 3 calls for `screenshot`.
- `touch`: sendTouch with pressure 512, then sendTouch with pressure 0, each awaited, 40 ms between taps, at (300, 60) on the status bar, where a tap changes nothing.
- `tap-to-frame`:
  - Setup: the Settings homepage is opened in a fresh task, streamScreenshot is subscribed, and the screen settles (no content change for 400 ms).
  - Start: t0 is taken just before sendTouch down on the "Network & internet" row at (540, 740); sendTouch up follows once down returns.
  - Change detection: each frame's key is its dimensions plus the xxh3 hash of its pixels, leaving out the top 6 % of rows (the status bar clock). The first frame whose key differs from the settled pre-tap frame ends the trial, and its changed region must contain the tap point.
  - Split: the emulator's `timestampUs` divides each trial into production (tap to the frame's timestamp) and delivery (timestamp to arrival: scaling, copying and transfer).
  - Reset: wait for the opened page to settle, press Back with `adb shell input keyevent 4`, and wait for the screen to settle back on the pre-tap key.
- `stream`: 10 s windows, once on a static screen and once while a 60 Hz sendTouch drag scrolls the Settings list 1,000 px up and down over a 2 s period, so that the guest renders continuously. It reports fps between the first and last frame, MB/s over the window, the inter-frame interval, the lag from `timestampUs` to arrival, emulator CPU from `ps` cputime, and the drag's sendTouch round trip.
- Hygiene:
  - The host and the emulator were shared. Other agents booted iOS Simulators and ran builds, and the 1-minute load reached 100 to 300 several times; spikes S1 and S6 also used the emulator.
  - Each run waits up to 10 minutes for no running xcodebuild, kotlinc or gradle build and a 1-minute load under `LOAD_MAX`, and runs again if the load passes twice that during the run. `LOAD_MAX` was 4 for the touch run and 6 for the rest, because the baseline without builds had settled between 4 and 7. The kept runs started at loads of 4.5 to 9.2, as read by each tool at its start, and each result file records its own.
  - Before each run that drives the screen, the suite waits until no device process looks like a UI automation user, then holds a marker process (`sh -c '…; : app_process s7-lock'`) until the run ends. S1's owner check matches it, so S1 waits instead of taking the screen.
  - Runs that spanned the shared adb server's restart at 23:13:19 local time were discarded. Every kept tap and stream run started after it; the touch run finished before it.

## Measurements

### Discovery and authentication

- On macOS the emulator writes `pid_<qemu pid>.ini` into `$HOME/Library/Caches/TemporaryItems/avd/running/`, mode 0600. The console commands `avd discoverypath` and `avd grpc` return the file's path and the port, so a driver that can reach the console needs no per-OS directory table. Third-party tools look in `$XDG_RUNTIME_DIR/avd/running` (then `/run/user/<uid>/avd/running`) on Linux and `%LOCALAPPDATA%\Temp\avd\running` on Windows ([t3code #12313](https://github.com/pingdotgg/t3code/issues/12313), [expo-device-hub #131](https://github.com/expo/expo-device-hub/pull/131), [auto-mobile #6407](https://github.com/kaeawc/auto-mobile/issues/6407)).
- Keys written by 37.1.11: `emulator.build`, `emulator.version`, `avd.id`, `avd.name`, `avd.dir`, `port.serial`, `port.adb`, `cmdline`, `grpc.port`, `grpc.token`, `grpc.allowlist`, `launcher.dir`. `port.serial` is the console port, so the file for `emulator-5554` is the one with `port.serial=5554`. The token is 88 base64 characters ending in `=` padding, so a line splits at its first `=` only.
- With `-grpc-use-token` every call carries `authorization: Bearer <token>`. Without the header, getStatus fails with UNAUTHENTICATED: "Missing the 'authorization' header with security credentials."
- Connect took 0.8 ms, getStatus 2 to 3 ms, getVmState 0.4 to 1 ms (state RUNNING, `booted` true).

### sendTouch round trip

200 taps, 400 sends, 10 warm-up taps excluded, load 4.5. The call returns once the emulator has scheduled the event on its main looper, so it acknowledges acceptance by the emulator, not delivery to the app.

| Calls | n | p50 ms | p95 ms | max ms |
|---|---|---|---|---|
| All sends | 400 | 0.49 | 0.74 | 1.47 |
| Down (pressure 512) | 200 | 0.56 | 0.75 | 1.47 |
| Up (pressure 0) | 200 | 0.40 | 0.68 | 0.88 |
| Tap, down + up | 200 | 0.96 | 1.35 | 2.12 |

The round trip grows while frames stream. The drag's moves in the stream table below took 0.55 to 3.0 ms at p50 and 1.4 to 15 ms at p95, and the most bytes per second gave the worst p95.

### Tap to frame change

| Input | Frames | Trials | Tap to arrival p50 / p95 ms | Production p50 / p95 ms | Delivery p50 / p95 ms | Load before, after |
|---|---|---|---|---|---|---|
| sendTouch | RGB888 1080x2400 | 40 | 133.6 / 155.5 | 101.6 / 128.0 | 29.6 / 37.9 | 5.7, 9.0 |
| sendTouch | RGB888 540x1200 | 40 | **107.0 / 146.5** | 86.9 / 130.5 | 16.2 / 29.6 | 5.8, 10.9 |
| sendTouch | PNG 540x1200 | 20 | 120.5 / 155.0 | 100.1 / 133.9 | 21.2 / 28.4 | 4.8, 7.5 |
| `adb shell input tap` | RGB888 540x1200 | 20 | 161.9 / 181.9 | 141.5 / 166.6 | 14.7 / 20.9 | 4.9, 7.9 |

- The two sendTouch calls took 1.2 to 3.2 ms at p50 during the trials (4.1 to 17.6 ms at p95), and hashing a frame added 0.02 to 0.32 ms at p50. The control path's share of a trial is input plus delivery, about 17 ms at 540 px and 32 ms at native size (p50). The rest is the guest: input dispatch, a software-rendered frame and composition.
- Every changed region was the ripple's first frame around the tap point, 2 to 4 px across at native size and up to 55 px at 540 px. Every trial opened the page and came back to the baseline after Back, except two PNG trials that never settled back onto the pre-tap key (18 of 20).
- `adb shell input tap` is 55 ms slower than sendTouch at p50 (161.9 against 107.0 ms). Almost all of the difference is in production, which for adb includes starting the `adb` and `input` processes.

### streamScreenshot

Static screen, RGB888 at 1080x2400: 10 frames in 10 s, all within 0.47 s of subscribing while Settings finished drawing, with one content change among them; emulator CPU 0.33 cores; the first frame came 50 ms after subscribing. The stream pushes a frame whenever the guest posts one, identical or not, and nothing while the screen is still.

While a 60 Hz drag scrolls the list, 10 s windows:

| Format | Size | fps | MB/s | Inter-frame p50 / p95 / max ms | Timestamp to arrival p50 / p95 ms | Emulator cores | Drag sendTouch p50 / p95 ms |
|---|---|---|---|---|---|---|---|
| RGB888 | 1080x2400 | 36.8 | 284.5 | 26.3 / 38.8 / 62.2 | 28.7 / 34.2 | 7.2 | 3.03 / 12.99 |
| RGB888 | 540x1200 | 44.7 | 86.9 | 21.5 / 32.5 / 71.5 | 14.6 / 21.0 | 7.4 | 0.96 / 6.59 |
| RGBA8888 | 1080x2400 | 35.9 | 371.1 | 26.2 / 42.7 / 80.6 | 26.8 / 37.0 | 7.0 | 1.95 / 15.10 |
| RGBA8888 | 540x1200 | 46.1 | 118.7 | 21.2 / 31.8 / 41.8 | 15.2 / 20.9 | 7.5 | 1.14 / 5.10 |
| PNG | 1080x2400 | 15.3 | 3.9 | 65.1 / 78.1 / 99.5 | 65.4 / 78.3 | 7.9 | 0.59 / 2.61 |
| PNG | 540x1200 | 44.5 | 5.0 | 21.2 / 31.6 / 48.1 | 21.4 / 29.5 | 7.9 | 0.55 / 1.37 |
| RGB888, hyper's default windows | 1080x2400 | 36.7 | 283.8 | 26.5 / 37.3 / 46.5 | 27.6 / 35.9 | 7.1 | 2.44 / 6.94 |
| RGB888, drag on a second connection | 1080x2400 | 36.0 | 279.1 | 26.8 / 40.5 / 75.0 | 28.9 / 34.8 | 7.0 | 2.21 / 7.45 |

Loads were 5.0 to 6.3 before each window and 5.9 to 8.0 after it.

- The guest produced at most about 46 frames per second under the drag, below the display's 60 Hz: the emulator spent about 7 cores rendering in software.
- `seq` counts the frames the emulator produced for the stream. The stream skipped at most 7 raw frames in a window, but at native size production itself fell to about 36 per second while each frame took about 28 ms to copy and send. PNG at native size needs 65 ms per frame, so the stream skipped 290 of 441 produced frames and kept only the latest, as the lossy frame class in the protocol note assumes.
- The window sizes made no difference on loopback: hyper's defaults moved the same 284 MB/s.
- A second connection for input cut the drag's p95 from 13.0 to 7.5 ms at native size. At 540 px on the same connection it was 6.6 ms.

### getScreenshot

30 calls each after 3 warm-up calls, on the static Settings homepage, loads 5.3 to 9.2.

| Format | Size | Bytes | p50 ms | p95 ms |
|---|---|---|---|---|
| PNG | 1080x2400 | 255,390 | 56.6 | 71.2 |
| PNG | 540x1200 | 114,025 | 16.1 | 31.2 |
| RGB888 | 1080x2400 | 7,776,000 | 21.0 | 50.0 |
| RGB888 | 540x1200 | 1,944,000 | 10.8 | 23.5 |
| RGBA8888 | 1080x2400 | 10,368,000 | 17.9 | 28.3 |
| RGBA8888 | 540x1200 | 2,592,000 | 10.0 | 23.2 |

No call failed: the documented FAILED_PRECONDITION for raw formats before the guest posts a frame never occurred on a booted device.

### Negative path: an emulator without `-grpc`

A second, read-only instance of `pixel_api36` could not start: "Another emulator instance is running. Please close it or run all emulators with -read-only flag." emulator-5554 runs without `-read-only` and was not to be restarted. So `negative.sh` created a throwaway AVD, `s7-negative`, from the same system image and Pixel 8 profile with `avdmanager`, and started it with `-no-window -no-audio -no-boot-anim -no-snapshot -port 5556` and no `-grpc`. It then stopped the instance with `adb -s emulator-5556 emu kill` and deleted the AVD. Load was 20.7 at the start, just after an iOS Simulator boot, and memory was 78 % free. `results/negative.txt` has the full run.

| Check | Result |
|---|---|
| Discovery file | None in 120 s, although the guest booted (getStatus later reported `booted`). `avd discoverypath` still names the path the file would have. |
| Console `avd grpc` | `KO: No active gRPC service` |
| Port 8556 (console port + 3000) | Connection refused |
| Other listening ports | Console 5556, adb 5557 and three ephemeral ports. None answered gRPC: two refused IPv4 connections, and every connection they accepted, over IPv4 or IPv6, was reset during the HTTP/2 handshake or stayed silent past the 5 s deadline. |
| Client | `discover` exited 2 and `status` exited 1, both with "emulator control endpoint unavailable for emulator-5556: taps and screenshots fall back to the agent; start the emulator with `-grpc 8556 -grpc-use-token` to enable the fast path". |
| Console `grpc 8556` | "OK: gRPC endpoint available at port 8556". The emulator then wrote `pid_<pid>.ini` with `grpc.port=8556`, a `grpc.token`, and `grpc.jwks` and `grpc.jwk_active` for JWT clients. getStatus took 2.1 ms with the token and failed UNAUTHENTICATED without it. |
| Shutdown | Exited 2 s after `emu kill`; the discovery file was removed; the shared adb server listed only emulator-5554 throughout. |

To keep the throwaway instance off the shared adb server, the script pointed its `ANDROID_ADB_SERVER_PORT` at an unused port. The emulator's own boot-time adb call (`adb shell settings put system screen_off_timeout …`) then started a second adb server on that port, and an adb server looks for every running emulator when it starts. The script's watcher stopped it 28 times during the run, and emulator-5554 stayed online. Tools that start throwaway emulators should not copy that setting.

### What the proto does not say

- A width alone does not scale. getScreenshot and streamScreenshot treat a zero in either `width` or `height` as "display size", so a 540-wide request came back 1080x2400 until the height (1200, from the display's aspect ratio) was set too.
- Raw frames arrive top-down. The proto says RGB888 rows run "bottom up", but a raw frame read top-down matched the PNG of the same screen (RMSE 0.013; flipped, 0.130), and every tap's changed region sat at the tap point only when read top-down.
- `EmulatorStatus.platformConfig`, which replaces the deprecated `hardwareConfig`, is empty on 37.1.11. The display size (`hw.lcd.width`, `hw.lcd.height`) is only in `hardwareConfig`. `vmConfig.numberOfCpuCores` reports 16 while the guest has 4.
- sendKey returns OK and does nothing when the guest has no keyboard device. This AVD has `hw.keyboard=no`, so the guest's only key device is `gpio-keys` with KEY_POWER. Neither `GoBack` nor evdev code 158 (KEY_BACK) produced an input event (checked with `getevent`), while `adb shell input keyevent BACK` worked.
- streamScreenshot pushes frames whose content did not change, so a consumer that wants change events must hash frames itself. Its first frame arrived 22 to 85 ms after subscribing in the kept runs, and up to 146 ms under heavy load.

## Verdict

**Emulator fast path (android-driver requirement): go, with two amendments.**

- The endpoint is found and authenticated through the emulator's own discovery file, and it covers the fast path:
  - sendTouch acknowledges in 0.49 ms at p50 and 0.74 ms at p95, well inside the 50 ms tap-acknowledgement scenario, although the acknowledgement means scheduled, not delivered.
  - Frames arrive only when the guest posts one, 15 ms (540 px) to 29 ms (native) after the emulator timestamps them.
  - Raw getScreenshot takes 10 to 21 ms at p50.
- The disabled case is unambiguous: no discovery file, or none with `grpc.port`, and `avd grpc` answering "No active gRPC service". The client fell back and printed the launch-flag warning the spec asks for.
- Amendment 1, keys. sendKey silently drops keys when the AVD has no keyboard device (`hw.keyboard=no`, common for command-line AVDs). Taps, gestures and frames use the endpoint; key presses and text go through the agent unless `hardwareConfig` reports `hw.keyboard=yes`.
- Amendment 2, enabling. The console's `grpc <port>` turns the endpoint on for a running emulator, with the same token security; the console itself needs `$HOME/.emulator_console_auth_token`. The driver can offer this instead of only asking for a restart with `-grpc`.

**Harness gate, tap-to-frame-change p95 under 50 ms on an emulator: no-go on this AVD.**

- Measured p95: 146.5 ms with 540 px RGB888 frames, 155.5 ms at native size, 155.0 ms with PNG, and 181.9 ms with `adb shell input tap`.
- The control path is not the bottleneck. Its share is input acknowledgement plus delivery: about 17 ms at 540 px and 32 ms at native size (p50).
- The guest's production of the changed frame takes 87 to 102 ms at p50 and 128 to 134 ms at p95: input dispatch, a software-rendered frame (`gpu auto` resolved to lavapipe and SwiftShader, about 7 cores while scrolling) and composition, on a shared host at load 5 to 11. Production alone exceeds 50 ms at p95, so no client-side choice meets the gate here.
- For task 2.11: keep the fast path. Before the harness enforces the gate (task 10.1), measure tap-to-frame again on an AVD with host GPU rendering (`-gpu host`) on a quiet host; S7 could not restart the shared emulator to do so. The harness should also record the GPU mode and split production from delivery with `timestampUs`, as this spike does. If host rendering still misses 50 ms, restate the gate per GPU mode or on the path's own share.

**D7: stream raw frames scaled on the emulator, encode on the host.**

- Request RGB888 at the mirror's working width, with both width and height set. At 540x1200 it gave 45 fps (this guest's limit), 15 ms from timestamp to arrival and 87 MB/s over loopback, and kept the drag's sendTouch p95 at 6.6 ms.
- Native raw frames doubled the lag (27 to 29 ms), capped the stream near 36 fps, moved 284 to 371 MB/s, and raised input p95 to 13 to 15 ms on the same connection.
- RGBA8888 is as fast as RGB888 and 33 % larger. Use PNG only at reduced size: at native size it drops to 15 fps with 65 ms of lag.
- Full-resolution screenshots on demand: use raw getScreenshot (18 to 21 ms at p50) plus a host encoder, not PNG (57 ms).
- Hash on the host for the settle signal:
  - The stream repeats unchanged frames.
  - xxh3 over a native frame costs 0.3 ms at p50.
  - The status bar has to be left out of the hash.
  - The stream keeps only the latest frame (PNG at native size skipped 290 of 441), which matches the lossy frame class in the protocol note.
- One HTTP/2 connection per emulator is enough at 540 px; give input its own connection if native frames stream. hyper's default flow-control windows are fine on loopback; raise tonic's 4 MB decode limit.

**For task 5.4 and later.**

- Generate the client from the installed emulator's proto.
- Resolve the target by `port.serial`, or through the console's `avd discoverypath` and `avd grpc`, and send the bearer token.
- Set a deadline on every call, since an emulator port can accept TCP and never answer.
- Request width and height together, read raw rows top-down, take the display size from `hardwareConfig`, and route keys by `hw.keyboard`.
- `doctor` (task 8.7) should report whether the endpoint is up, the GPU mode and `hw.keyboard`.
