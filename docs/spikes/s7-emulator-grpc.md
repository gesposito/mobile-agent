# S7: emulator gRPC controller

Spike for task 2.10 of `mobile-automation-foundation`. It checks the android-driver requirement "Emulator fast path" and design D7: can the emulator's gRPC controller (`android.emulation.control.EmulatorController`), found through the emulator's own discovery file and token, carry taps and frames, and does that path meet the harness gate "tap-to-frame-change p95 under 50 ms on an emulator"? The code is in `spikes/s7-emulator-grpc/`.

**In brief.** PLACEHOLDER_BRIEF

## Setup

| Item | Value |
|---|---|
| Host | Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2 (25G83) |
| Emulator | 37.1.11.0 (build 15917651), `qemu-system-aarch64-headless` on HVF |
| AVD | `pixel_api36`: Pixel 8 profile, `google_apis` arm64-v8a API 36 (`BE2A.250530.026.F3`), 1080x2400 at 420 dpi and 60 Hz, 4 vCPUs, 2,560 MB, `hw.keyboard=no`, gesture navigation |
| Guest GPU | `hw.gpu.mode=auto`, which resolved to software rendering: `hardware-qemu.ini` says `lavapipe`, SurfaceFlinger reports ANGLE on SwiftShader Vulkan |
| Launch | `-avd pixel_api36 -no-window -no-audio -no-boot-anim -grpc 8554 -grpc-use-token` |
| Proto | `$ANDROID_HOME/emulator/lib/emulator_controller.proto`, 1,865 lines, SHA-256 `1d62c6bcad5f0662…ba769ccd0f1416b5314d25a68e04eee5f`, importing only `google/protobuf/empty.proto`; the AOSP prebuilts copy has 1,397 lines (research note) |
| Client | Rust 1.95.0; tonic and tonic-prost 0.14.6, prost 0.14.4, tonic-prost-build 0.14.6 running libprotoc 31.1 from protoc-bin-vendored 3.2.0; hyper 1.11.1, h2 0.4.19, tokio 1.53.1; xxhash-rust 0.8.18 |
| Spike code | `787e113` client and discovery, `bcb6f1b` measurements |

To rerun: `cd spikes/s7-emulator-grpc && eval "$(mise env)" && ./measure.sh emulator-5554`, then `./negative.sh`. `results/` holds the output of every kept run, the raw samples as CSV, and `host.txt` with the load before and after each run.

**Method.**

- Channel: TCP_NODELAY; HTTP/2 stream window 32 MiB, connection window 64 MiB and frames up to 16 MiB (`--h2 tuned`, the default; one comparison run uses hyper's 2 MiB and 5 MiB defaults); message limit 64 MiB, because a native RGBA8888 frame is 10.4 MB and tonic's default is 4 MB.
- Timings are host wall-clock times with nearest-rank percentiles. Warm-up is excluded: 10 taps for `touch`, 3 trials for `tap-to-frame`, 3 calls for `screenshot`.
- `touch`: sendTouch with pressure 512, then sendTouch with pressure 0, each awaited, 40 ms between taps, at (300, 60) on the status bar, where a tap changes nothing.
- `tap-to-frame`: the Settings homepage is opened in a fresh task, streamScreenshot is subscribed and the screen settles (no content change for 400 ms). t0 is taken just before sendTouch down on the "Network & internet" row at (540, 740); sendTouch up follows once down returns. Each frame's key is its dimensions plus the xxh3 hash of its pixels without the top 6 % of rows (the status bar clock). The first frame whose key differs from the settled pre-tap frame ends the trial, and its changed region must contain the tap point. The emulator's `timestampUs` splits each trial into production (tap to the frame's timestamp) and delivery (timestamp to arrival: scaling, copying and transfer). The reset waits for the opened page to settle, presses Back with `adb shell input keyevent 4`, and waits for the screen to settle back on the pre-tap key.
- `stream`: 10 s windows, once on a static screen and once while a 60 Hz sendTouch drag scrolls the Settings list 1,000 px up and down over a 2 s period, so that the guest renders continuously. It reports fps between the first and last frame, MB/s over the window, the inter-frame interval, the lag from `timestampUs` to arrival, emulator CPU from `ps` cputime, and the drag's sendTouch round trip.
- Hygiene. Other agents shared the host (iOS Simulator boots, builds; the 1-minute load reached 100 to 300 several times) and the emulator (spikes S1 and S6). Each run waits up to 10 minutes for no running xcodebuild, kotlinc or gradle build and a 1-minute load under `LOAD_MAX`, and runs again if the load passes twice that during the run. `LOAD_MAX` was 4 for the touch run and 6 for the rest, because the baseline without builds had settled between 4 and 7. Before each run that drives the screen, the suite waits until no device process looks like a UI automation user, then holds a marker process (`sh -c '…; : app_process s7-lock'`) until the run ends. S1's owner check matches it, so S1 waits instead of taking the screen. Runs that spanned the shared adb server's restart at 23:13:19 local time were discarded.

## Measurements

### Discovery and authentication

- On macOS the emulator writes `pid_<qemu pid>.ini` into `~/Library/Caches/TemporaryItems/avd/running/`, mode 0600. The console commands `avd discoverypath` and `avd grpc` return the file's path and the port, so a driver that can reach the console needs no per-OS directory table. Third-party tools look in `$XDG_RUNTIME_DIR/avd/running` (then `/run/user/<uid>/avd/running`) on Linux and `%LOCALAPPDATA%\Temp\avd\running` on Windows.
- Keys written by 37.1.11: `emulator.build`, `emulator.version`, `avd.id`, `avd.name`, `avd.dir`, `port.serial`, `port.adb`, `cmdline`, `grpc.port`, `grpc.token`, `grpc.allowlist`, `launcher.dir`. `port.serial` is the console port, so the file for `emulator-5554` is the one with `port.serial=5554`. The token is 88 base64 characters ending in `=` padding, so a line splits at its first `=` only.
- With `-grpc-use-token` every call carries `authorization: Bearer <token>`. Without the header, getStatus fails with UNAUTHENTICATED: "Missing the 'authorization' header with security credentials."
- Connect took 0.8 ms, getStatus 2 to 3 ms, getVmState 0.4 to 1 ms (state RUNNING, `booted` true).

PLACEHOLDER_TABLES

## Verdict

PLACEHOLDER_VERDICT
