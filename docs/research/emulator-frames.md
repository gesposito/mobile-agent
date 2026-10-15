# Emulator frames: what tap-to-frame-change measures

Research note for task 10.13 of the `mobile-automation-foundation` change. The 10.1 run ([`reports/10.1-emulator.md`](../../reports/10.1-emulator.md#tap-to-frame-change)) measured tap-to-frame-change on an emulator rendering on the host GPU at p95 52.5 ms on the Expo fixture and 55.1 ms on the Compose one, against a 50 ms gate. This note splits that span into its parts, measures the cheap levers on the host side, and says what the gate can hold the tool to. Snapshot date: 2026-09-28.

Versions read: Android Emulator 37.1.11 (build 15917651) and the controller proto it ships, vendored as `crates/android/proto/emulator_controller.proto` [4]. The emulator's own sources for 37.x are not public. The newest public branch of its gRPC service is `emu-master-dev` at `ae9d18d` (2025-03-27) [1], whose `streamScreenshot` and `getScreenshot` match those of `emu-36-1-release` [2]. The renderer, gfxstream, was read at `main` `ebe5b42` (2026-09-17) [3] [9], the emulator's composer HAL at `goldfish-opengl` `main` `5cd042c` [8], and React Native at 0.86.3, the Expo fixture's version [7]. Bracketed numbers refer to the Sources list. Statements marked *(inference)* are reasoned from code, not observed.

## Summary

- **The span ends inside the guest's composition.** The emulator stamps a frame before reading it back, and a tap's changed frame is stamped while SurfaceFlinger is still composing it: 0.5 ms (Compose) and 1.4 ms (Expo) before its `composite` ends, at p50. In 650 probe taps the changed frame was the frame event right after the settled one (`seq` step 1), and in all but 4 the first frame after the tap. The stream never made a changed frame wait.
- **The stream keeps up.** Under host rendering it delivers every frame the guest posts, 60 per second at 270, 540 and 1080 px wide, in RGBA8888 and through `MMAP`, with no frame event coalesced. The frame's size moves only the delivery after the stamp: 2.3, 4.6 and 14 ms from timestamp to arrival at p50. The 45 fps S7 measured was the software-rendered guest's rate, not the stream's.
- **The tool's share is the dispatch.** Through the daemon, `dispatchMs` was 2.4 to 2.8 ms at p50 and at most 3.4 ms at p95. The host hashes a 540 px frame in 58 µs, after the stamp; `resize_rgb` is not on the path.
- **The rest is the guest.** The input's delivery to the app, the wait for the app's next VSync, and the app's pipeline to SurfaceFlinger's composition. From the up event to SurfaceFlinger's commit: 2.4 frames of the 60 Hz display at p50 and 3.0 at p95 for the Compose fixture, whose debug build's frame misses SurfaceFlinger's deadline in more than half the taps on an idle host, and 2.1 and 2.8 for the React Native fixture, which hands input to JavaScript one frame late.
- **Levers.** Streaming at 270 px or at the display size instead of 540 px, and sending a tap's down and up in one `streamInputEvent` call, changed nothing within the measurements' spread (bootstrap 95 % intervals of the differences include 0). Hashing before or instead of the resize is moot. None was kept.
- **The host's CPU state matters more than any lever.** One busy host core cut the Compose fixture's p50 from 53 to 57 ms to 25 to 35 ms and its p95 from 59 to 61 ms to 29 to 39 ms; the React Native fixture's p95 moved from 53.5 to 52 ms. The 10.1 run's Compose p50 of 31.7 ms on a busier host fits this.
- **The gate.** At 60 Hz the guest alone takes about three frames at p95, which is the 50 ms gate. The gate is restated for host rendering at p95 under 67 ms, four frames; software rendering, 146.5 ms at p95 in S7, is not gated.

## 1. What the gated span covers

The harness's tap-to-frame-change is the `frameChangeMs` of the daemon's `click` reply.

- **Start.** `click` takes `started` just before `Mirror::act` dispatches the tap (`crates/daemon/src/handlers/act.rs`). The span therefore includes the mirror's aim and both `sendTouch` calls, which `dispatchMs` reports on its own.
- **End.** The capture time of the first frame at or after `started` whose hash differs from the frame before it (`Mirror::frame_changes_since`). The session sets a frame's capture time to its arrival minus the lag the emulator's timestamp shows, `arrival - (now - timestampUs)`, capped at a second (`frame_meta` in `crates/android/src/session/controller.rs`).
- **The emulator's timestamp.** `getScreenshot` sets `timestampUs` from the Unix clock before it reads the frame back, scales, converts and copies it [1]. The proto says the same: "The timestamp is before the actual frame is copied and transformed" [4]. `streamScreenshot` calls `getScreenshot` once per frame event [1].

So the span ends when the emulator's stream loop starts on the changed frame. Three things the 10.1 analysis counted are outside it:

- **Delivery.** The lag from the timestamp to the frame's arrival: the readback, the scaling, the gRPC transfer and the decode.
- **The host's per-frame work.** The session hashes each frame (xxh3 over the rows below the status bar) after its arrival, then sends its metadata to the mirror over a channel; neither moves the capture time.
- **`resize_rgb`.** It runs only inside `encode_jpeg`, which `ControllerFrames::encode` calls when `Mirror::frame` encodes the latest frame. The daemon's `screenshot` asks the driver's own capture first, and the Android driver always has one, the agent's screenshot (`AgentScreenshots` in `crates/daemon/src/devices/android.rs`). The Android daemon never scales a stream frame. Hashing before or instead of the resize is therefore not a lever: there is no resize on the path. For scale, `measure_the_rgb_resize` times it at 4.85 ms at p50 from 1080x2400 to 450x1000.

What the span can include from the stream is the wait between the frame event and the loop's `getScreenshot` call: nothing when the loop is idle, and the rest of the previous frame's readback and write when it is busy. [Section 4](#4-decomposition) finds that it coalesced no frame event at any tap, and that in all but 4 of 650 taps no frame came between the tap and the changed one.

## 2. How a changed frame reaches the stream

### 2.1 In the guest

The display pipeline is paced by VSync: the app handles pending input and renders at one VSync, SurfaceFlinger composites that buffer at a later one, and "the latency is at least two frames" without phase offsets [6]. The AVD's display runs at 60 Hz, 16.7 ms per frame. Its SurfaceFlinger (`dumpsys SurfaceFlinger`) schedules the app 16.67 ms of work plus 15.67 ms of readiness before a frame's presentation, and itself 15.67 ms: it composes a frame about one display frame after the app's VSync for it. In the traces below its `commit` began 13 to 19 ms after the `doFrame` of a frame whose buffer was ready in time. A tap arrives at a random phase of the display's cycle, so the wait for the app's next VSync is uniform between 0 and 16.7 ms.

- **Compose.** A click's handler runs when the up event is dispatched, and the state it changes recomposes and draws at the next frame. If that frame's buffer is queued after SurfaceFlinger's next wake-up, it waits one more frame.
- **React Native.** Fabric's event dispatcher on Android hands batched events to JavaScript from a `ReactChoreographer` frame callback, not when the touch arrives [7]. The traces show that frame, then JavaScript's commit mounted and drawn on the following one: one frame more than Compose.

SurfaceFlinger then presents through the emulator's composer HAL, which composes on the host: the AVD reports `ro.hardware.gralloc=ranchu` and device composition for its layers. On this path the HAL sends the layers with `rcComposeAsync`, or `rcCompose` on a host without asynchronous frame commands, and returns a fence the host signals when it is done [8]. The host composes and then posts [3] [9]. The guest's present is therefore a hand-off, and the host's composition follows it.

### 2.2 In the emulator

- **Compose and post.** gfxstream runs the composition as a command on its post worker thread and waits for it. With `-no-window` it then posts the target color buffer, and `post()` marks that the guest posted a frame, which fires a `FrameBufferChangeEvent` to its listeners [3].
- **The stream loop.** `streamScreenshot` registers an `EventWaiter` for those events and loops on `frameEvent->next(125 ms)`. When events have arrived, it adds their count to the frame number that becomes `Image.seq`, calls `getScreenshot`, and writes the image with the synchronous gRPC writer [1]. Events that arrive while one frame is read back and written are counted into the next wake-up, so they show as a `seq` jump, and only the latest content is sent.
- **The readback.** `getScreenshot` stamps `timestampUs`, then reads the last posted color buffer through gfxstream's `getScreenshot`, which queues a screenshot command on the same post worker and waits for it: `readToBytesScaled` scales on the GPU and reads the pixels back in the requested format [1] [3]. A readback in progress therefore delays the next guest composition as well *(inference)*.
- **The first frame.** A new subscription gets the current screen at once, then a frame per post [1].

### 2.3 What the controller allows

- **Rate.** `ImageFormat` has no rate or interval field. The stream sends a frame per guest post, or per sensor change, and nothing while the screen is still [4]. The delivered rate can only rise by cutting the loop's per-frame cost: the size, the format, or the transport. Under host rendering it already delivers every post (section 4.1).
- **Transport.** `ImageTransport` with channel `MMAP` and a `file:///` handle makes the emulator write the pixels into a file the client created and mapped, and leave `Image.image` empty. The proto says it "can significantly reduce gRPC overhead for pixel data" and warns that the mapping "can result in tearing" [4]. The emulator opens `file://` handles with `open` and `mmap` on POSIX hosts, macOS included [5]. It saves the message copy and the HTTP/2 transfer, not the readback.
- **Format.** RGB888, RGBA8888 or PNG. S7 measured RGBA8888 as fast as RGB888 at 540 px and PNG as much slower at the native size (`docs/spikes/s7-emulator-grpc.md`).
- **Input.** `sendTouch` is acknowledged once the emulator has scheduled the event on its main looper [4]. `streamInputEvent` takes a stream of events in order, without a reply per event [4].

## 3. Method

### 3.1 Setup

- **Host.** Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2. The endpoint security extension the 10.1 report names was at times the busiest process, and each series records the 1-minute load average.
- **Emulator.** 37.1.11 as `emulator -avd pixel_api36 -no-window -no-audio -no-boot-anim -gpu host -grpc 8554 -grpc-use-token`: `pixel_api36`, API 36 userdebug, 1080x2400 at 420 dpi and 60 Hz, gfxstream on Metal. Started as a background job of zsh, whose `BG_NICE` option is on by default, it runs at nice 5; the lever series and the traced taps ran so. For the host-state series it was relaunched at nice 0 (`setopt NO_BG_NICE`), which made no difference on an idle host (section 4.4).
- **Fixtures.** The APKs of the 10.1 run: the Compose debug APK `bfc1d3b7…` and the Expo release APK `ac3770d8…`.
- **Tools.** Release builds of `cli` and `daemon` at main `08fdde7` with the stream-width setting of section 3.3, embedding agent 0.3.0, and the measurements in `crates/android/tests/frame_timing.rs`, run with `cargo test --release`. No other automation ran on the emulator.

### 3.2 Decomposition

Each tap is split into stages on two clocks, the host's and the guest's trace clock.

- **Host side.** `measure_tap_to_frame` in `crates/android/tests/frame_timing.rs` finds `row-1` and `detail-back` on a fixture's List tab through the agent, stops the agent, then taps `row-1` with `sendTouch` down then up on the controller's input connection, as the session does. It reads the frame stream on a connection of its own and hashes each frame as the session does. For every tap it records the host epoch before the tap (t0) and after each `sendTouch`, every frame between the tap and the first changed one, the changed frame and the frame before it. It then waits for the detail to settle (no hash change for 400 ms), taps `detail-back`, and waits for the list's hash to come back and settle. Its `timestampUs - t0` is the span `frameChangeMs` covers, less the mirror's aim and without the agent attached.
- **Guest side.** `atrace --async_start -c -b 16384 -a <both fixtures> input view gfx` runs during the probe's taps; `atrace --async_stop` returns the text trace, 253,467 and 439,857 events with none lost. Per tap it gives the app's `deliverInputEvent` for the down and the up, the `Choreographer#doFrame` whose vsync id the RenderThread's `DrawFrames` around the first `queueBuffer` after the input carries, and SurfaceFlinger's next `commit` and `composite`.
- **The clocks.** Before and after the taps, the host writes 30 numbered markers into `/sys/kernel/tracing/trace_marker` through one persistent `adb shell`, noting the host epoch before each write and after the shell's acknowledgement. The marker with the shortest round trip, 1.62 and 1.87 ms, maps the trace clock onto the host's epoch within half that round trip; the other markers' estimates lie within -0.5 to +2.1 ms (Compose) and -0.1 to +4.3 ms (Expo) of it.
- **The stream.** `measure_the_frame_stream` subscribes for 10 s windows while the Compose fixture's Anim tab spins, twice per layout in two rounds, the second reversed: RGB888 at 270, 540 and 1080 px wide over gRPC, RGBA8888 at 540 px, and RGB888 at 540 px through `MMAP`.
- **The host's CPU state.** The probe's taps in blocks of 25, alternating an idle host with one where `yes > /dev/null` keeps one core busy.
- **The daemon's detection and reply.** The session sends a frame's metadata to the mirror as it arrives, and `Mirror::run_until` blocks on that channel, so the replica takes the frame in as it arrives; `click`'s watch looks at the replica every 5 ms and replies once the tree delta has come too. `treeDeltaMs` and the command's wall time come from the lever series.

### 3.3 Levers

- **Stream width.** A daemon setting, `MOBILE_AGENT_ANDROID_FRAME_WIDTH`, read at attach, so one release binary measured 540 px (the default), 270 px and the display size. Each series ran the harness's tap-to-frame-change procedure through the CLI with `--json` on each fixture's List screen: `open --relaunch --env initial_tab=list`, `wait` for `row-1`, `wait --settle`; then per tap `click` on `row-1`, which opens its detail, `click` on `detail-back`, and `wait --settle`. A block was 3 warm-up taps and 10 measured ones under a daemon of its own, started with the configuration's environment and shut down after the block. Blocks rotated the three configurations, five blocks each, so each had 50 measured taps and drift in the host's load reached all three.
- **Input in one call.** `MOBILE_AGENT_FRAME_TIMING_INPUTS=touch,stream` alternates, tap by tap, the session's two `sendTouch` calls with one `streamInputEvent` call carrying the down and the up, 50 taps each.
- **Statistics.** Nearest-rank percentiles, as the bench computes them, and bootstrap 95 % intervals (2,000 resamples) of the p50 and p95 differences.

## 4. Decomposition

### 4.1 The stream

Two 10 s windows per layout while the spinner turned at 60 frames per second, 598 to 599 frames each; 1-minute load 2.1 to 3.9.

| Layout | Delivered | Frame events | Coalesced | Timestamp to arrival, p50 / p95 ms | Hash, p50 µs |
|---|---|---|---|---|---|
| RGB888 270x600, gRPC | 60.0/s | 60.0/s | 0 | 2.3 to 2.5 / 4.5 to 4.7 | 18 |
| RGB888 540x1200, gRPC (the session's) | 60.0/s | 60.0/s | 0 | 4.5 to 4.8 / 7.0 to 7.8 | 58 |
| RGB888 1080x2400, gRPC | 60.0/s | 60.0/s | 0 | 13.8 to 14.2 / 16.3 to 16.9 | 220 |
| RGBA8888 540x1200, gRPC | 60.0/s | 60.0/s | 0 | 7.3 to 7.9 / 8.7 | 73 |
| RGB888 540x1200, `MMAP` | 60.0/s | 60.0/s | 0 | 1.7 / 3.7 to 4.1 | 310, with the file read |

Inter-arrival was 16.6 to 16.8 ms at p50 and 19.3 to 23.4 ms at p95 in every window. The loop never fell a frame behind: no frame event was coalesced at any layout.

### 4.2 A tap, stage by stage

The probe's 50 measured taps per fixture, with the trace; milliseconds from the tap, or between the stages' ends. Percentiles of stages do not add up to the percentiles of the whole.

| Stage | Compose p50 / p95 | Expo p50 / p95 |
|---|---|---|
| `sendTouch` down acknowledged (host) | 1.6 / 3.5 | 1.6 / 10.4 |
| `sendTouch` up acknowledged (host) | 2.3 / 4.8 | 2.8 / 18.2 |
| Tap to the up event's dispatch in the app | 8.4 / 13.0 | 6.1 / 24.4 |
| Up event to the start of the frame that draws the change | 10.8 / 18.3 | 18.1 / 30.1 |
| That frame's start to its queued buffer | 16.1 / 22.0 | 7.8 / 11.6 |
| Queued buffer to SurfaceFlinger's `commit` | 11.3 / 17.3 | 8.7 / 12.5 |
| `commit` to the stream's timestamp | 1.2 / 2.2 | 1.2 / 2.1 |
| Stream's timestamp minus the end of `composite` | -0.5 / -0.3 | -1.4 / -0.8 |
| **Tap to the timestamp (`frameChangeMs`)** | **52.2 / 60.4** (max 65.7) | **42.9 / 57.1** (max 60.8) |
| Timestamp to arrival, outside the span | 5.8 / 8.8 | 5.6 / 9.8 |
| Up event to SurfaceFlinger's `commit`, in 60 Hz frames | 2.4 / 3.0 | 2.1 / 2.8 |
| Unchanged frames between the tap and the change | 0 in 49 taps, 1 in 1 | 0 in 50 |
| `seq` step to the changed frame | 1 in 50 | 1 in 50 |

The 1-minute load went from 2.41 to 1.74 over the Compose taps and from 3.28 to 5.44 over the Expo ones, when the endpoint security extension was busy; the up event's acknowledgement shows it at p95. Without the trace the same Compose series measured 52.0 / 63.1 ms, so the trace cost nothing visible.

Both fixtures' taps fall into two groups:

- **Compose.** In 19 taps (24 to 44 ms) the drawing frame took 8.4 to 14.8 ms from its `doFrame` to its queued buffer, and SurfaceFlinger's `commit` followed 4.3 ms later at p50. In 31 (43 to 66 ms) it took 10.5 to 26.0 ms, 17.9 ms at p50: the buffer missed SurfaceFlinger's first wake-up and waited 14.7 ms at p50 for the next. One such tap: the up dispatched at 7.0 ms, the app's VSync at 18.4 ms, `doFrame` from 18.9 to 32.5 ms, `DrawFrames` from 32.3 to 34.3 ms, SurfaceFlinger's wake-up for that frame already past, `commit` at 49.0 ms, the stamp at 50.0 ms. The fastest tap had the same steps with an 8.8 ms `doFrame` right after the up: 23.9 ms.
- **Expo.** In 29 taps (36 to 46 ms) the up reached the app 4.9 ms after the tap at p50, in 21 (50 to 61 ms) 16.3 ms after: those taps met the host's load. In 45 taps the first `doFrame` after the up, at most 3 ms, handed the events to JavaScript and the next VSync's `doFrame`, 1.4 to 8.5 ms, mounted and drew the change; in 4 the first one drew it, and in 1 a later one. SurfaceFlinger's `commit` began 13 to 18 ms after the drawing `doFrame` did, in every tap.

The emulator's share after the guest's hand-off is within the clock mapping's error: the stamp precedes the end of SurfaceFlinger's `composite`, so the host composes, posts and wakes the stream loop while SurfaceFlinger finishes. The tool's share is the dispatch, the two acknowledgements above; through the daemon, `dispatchMs` was 2.4 to 2.8 ms at p50 and 3.1 to 3.4 ms at p95 (section 5).

### 4.3 The host's work and the reply

- The hash of a 540x1200 frame took 58 µs at p50 in the stream windows, 76 µs for the tap's changed frame; it follows the arrival and does not move the stamp.
- `resize_rgb` is not on the path (section 1).
- The frame reaches the mirror 5.6 to 5.8 ms after its timestamp at p50. The click's reply waits for the tree delta, which the session holds until the input is 200 ms old: `treeDeltaMs` was 226 to 246 ms at p50 and the command 240 to 272 ms, neither part of the span.

### 4.4 The host's CPU state

The probe's taps, 25 per block, blocks alternating; the load is the 1-minute average at each block's start.

| Emulator | Fixture | Host | First block, p50 / p95 ms (load) | Second block, p50 / p95 ms (load) |
|---|---|---|---|---|
| nice 5 | Compose | idle | 57.0 / 60.6 (5.74) | 56.9 / 59.5 (3.75) |
| nice 5 | Compose | one busy core | 24.7 / 28.7 (4.29) | 24.8 / 29.1 (2.88) |
| nice 0 | Compose | idle | 52.8 / 59.3 (1.83) | 55.0 / 60.7 (2.06) |
| nice 0 | Compose | one busy core | 34.8 / 38.2 (1.46) | 28.4 / 38.9 (1.74) |
| nice 0 | Expo | idle | 45.0 / 53.5 (2.51) | 49.0 / 53.6 (2.75) |
| nice 0 | Expo | one busy core | 42.9 / 52.3 (2.52) | 41.3 / 51.9 (2.60) |

On an idle host the Compose fixture's frame, 9 to 14 ms of `doFrame` in its debug build, misses SurfaceFlinger's deadline more than half the time; with one host core kept busy it mostly does not *(inference: the host's performance cores run faster when not idle)*. The React Native fixture gains little, since its extra frame is structural. The 1-minute load does not show this: a busy core adds about one to it. The 10.1 run's Compose samples, 44 of 50 between 22.5 and 39 ms at loads of 1.5 to 7.9, match the busy-host rows.

## 5. Levers

| Lever | Fixture | Before, p50 / p95 ms | After, p50 / p95 ms | 95 % interval of the difference, p50 / p95 ms | 1-minute load | Kept |
|---|---|---|---|---|---|---|
| Stream 270 px wide instead of 540 (daemon) | Compose | 45.2 / 59.1 | 36.0 / 55.1 | -15.6 to +10.5 / -7.6 to +2.8 | 1.65 to 3.18 | no |
| Stream at the display size instead of 540 (daemon) | Compose | 45.2 / 59.1 | 40.9 / 56.1 | -12.3 to +10.5 / -7.5 to 0.0 | 1.65 to 3.18 | no |
| Stream 270 px wide instead of 540 (daemon) | Expo | 45.5 / 53.9 | 46.3 / 53.4 | -3.8 to +3.5 / -12.2 to +1.3 | 3.31 to 4.36 | no |
| Stream at the display size instead of 540 (daemon) | Expo | 45.5 / 53.9 | 45.2 / 52.9 | -3.1 to +3.6 / -12.8 to +0.8 | 3.31 to 4.36 | no |
| Down and up in one `streamInputEvent` call (probe) | Expo | 42.9 / 55.8 | 42.8 / 56.7 | -3.6 to +4.8 / -1.7 to +3.9 | 3.08 to 3.91 | no |
| Down and up in one `streamInputEvent` call (probe) | Compose | 55.8 / 75.8 | 57.2 / 72.6 | -1.5 to +13.0 / -14.0 to +10.9 | 4.08 to 3.18 | no |
| Hash before or instead of the resize | both | no resize on the path (section 1) | | | | no |
| RGBA8888 or `MMAP` frames | both | move only the delivery, after the stamp (section 4.1) | | | | no |

- **Stream width.** Every interval includes 0: the width moves the delivery and the emulator's readback, both after the stamp, as sections 1 and 4.1 predict. `dispatchMs` stayed at 2.4 to 2.8 ms at p50 and 3.1 to 3.4 ms at p95 in every series. The setting was not kept; the stream stays at 540 px.
- **Input in one call.** It acknowledged the up 1.4 to 2.0 ms earlier at p50, but the change still waited for the same VSyncs. The second Compose pair ran on a contended host (the up acknowledged at 14.6 ms at p95 against 4.8 ms quiet), which is why both of its p95s are above the other series'.
- **A busy host core** is not a lever of the tool, but it moved the Compose fixture more than anything above (section 4.4).

## 6. The gate

The decomposition puts tap-to-frame-change on the emulator at:

- the tool's dispatch, at most 3.4 ms at p95;
- the guest's input delivery, a few milliseconds on a quiet host and more under load;
- the wait for the app's next VSync, up to one frame;
- the app's pipeline to SurfaceFlinger's composition: one frame for Compose, plus one when its frame misses SurfaceFlinger's deadline, and two for React Native;
- SurfaceFlinger's composition, about 1.2 ms, during which the emulator stamps the frame.

At 60 Hz the guest's part from the up event to SurfaceFlinger's commit is 2.8 to 3.0 frames at p95, 47 to 50 ms, before the input's delivery and the dispatch. A p95 under 50 ms is therefore the guest's own floor, with nothing left for the tool, and no host-side lever moved it.

The benchmark-harness spec now gates tap-to-frame-change on an emulator rendering on the host GPU at p95 under 67 ms: four frames of the AVD's 60 Hz display, the guest's three plus one for the input's delivery, SurfaceFlinger's composition and the tool's dispatch. The limit is defined for host rendering; the harness cannot tell the GPU mode, and under software rendering, where S7 measured 146.5 ms at p95 (130.5 ms of it before the emulator's timestamp), the span is reported and not gated. The 10.1 run's 52.5 and 55.1 ms and this task's daemon series, 52.9 to 59.1 ms at p95, pass it. A tool regression of more than about 8 ms, such as a stream that made the changed frame wait or a slower dispatch, would fail it on the Compose fixture; `crates/android/tests/frame_timing.rs` measures the parts again.

## Sources

1. Android Emulator gRPC service, `EmulatorService.cpp` at `emu-master-dev` `ae9d18d` (2025-03-27), GitHub mirror of AOSP `platform/external/qemu`: `streamScreenshot` from line 695, `getScreenshot` and `set_timestampus` at line 908. https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-grpc/services/emulator-controller/server/src/android/emulation/control/EmulatorService.cpp
2. The same file at `emu-36-1-release` `9f0811e`: the same loop and timestamp, at lines 678 and 891. https://github.com/aosp-mirror/platform_external_qemu/blob/9f0811e72acfc46edc39d3d0baedd796f7d03309/android/android-grpc/services/emulator-controller/server/src/android/emulation/control/EmulatorService.cpp
3. gfxstream `host/frame_buffer.cpp` at `main` `ebe5b42` (2026-09-17): `setGuestPostedAFrame` (line 812), `post` (2567), `getScreenshot` (2836), `compose` and its `-no-window` post (3056). https://github.com/google/gfxstream/blob/ebe5b424e15c0816f64b97c9691060333bb43c7e/host/frame_buffer.cpp
4. `emulator_controller.proto` as Emulator 37.1.11 ships it, vendored as `crates/android/proto/emulator_controller.proto`: `sendTouch`, `streamInputEvent`, `streamScreenshot`, `ImageTransport`, `ImageFormat`, `Image.timestampUs`.
5. `SharedMemory_posix.cpp` at `emu-36-1-release` `9f0811e`: `file://` handles opened and mapped. https://github.com/aosp-mirror/platform_external_qemu/blob/9f0811e72acfc46edc39d3d0baedd796f7d03309/android/emu/base/memory/src/android/base/memory/SharedMemory_posix.cpp
6. AOSP, "VSync", last updated 2025-10-20. https://source.android.com/docs/core/graphics/implement-vsync
7. React Native 0.86.3, `FabricEventDispatcher.kt`: events are handed on from a `ReactChoreographer` frame callback. https://github.com/facebook/react-native/blob/v0.86.3/packages/react-native/ReactAndroid/src/main/java/com/facebook/react/uimanager/events/FabricEventDispatcher.kt
8. The emulator's composer HAL, `HostFrameComposer::presentDisplay` in `device/generic/goldfish-opengl` at `main` `5cd042c`: `rcComposeAsync` or `rcCompose`, then a goldfish sync fence as the retire fence (from line 655). https://android.googlesource.com/device/generic/goldfish-opengl/+/5cd042c8586a38b408f390fcef979d58ace3da23/system/hwc3/HostFrameComposer.cpp
9. gfxstream `host/render_control.cpp` at `main` `ebe5b42`: `rcCompose` and `rcComposeAsync` both call `FrameBuffer::compose` with a post (lines 1244 and 1537). https://github.com/google/gfxstream/blob/ebe5b424e15c0816f64b97c9691060333bb43c7e/host/render_control.cpp
