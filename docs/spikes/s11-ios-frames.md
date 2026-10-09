# S11: iOS frames

Spike S11 (task 2.3 of `physical-ios`) looks for a host-side frame stream of a physical iPhone that gives the change signal design D23 needs: 30 frames a second under a quarter of a core, and a tap's first changed frame within 100 ms. It also checks the host-side input of design D22, the developer disk image's `dtuhidd` over RSD, on the same phone. The research note `docs/research/ios-device.md` named the three frame paths and the spike's additions. The code is `spikes/s11-ios-frames/`, and every table in Measurements is generated from its `results/` by `s11-ios-frames report`. Snapshot date: 2026-10-09.

**In brief.** No-go for D23's gate on this phone today; go for D22's host input.

- **Two of the three frame paths cannot run here.** The phone refuses the CoreDevice display stream in every form: "Remote control requires iOS 27.0 or later on this device". AVFoundation found the phone but delivered no frame without the macOS camera permission, which the spike asked for and no one answered.
- **Screenshots run.** They are lossless, so a two-level tolerance below the status bar counts no frame of a still screen as a change, and they cost at most 22.9% of a core. But the phone serves only about 19 a second, against the gate's 30, and a tap's first changed frame arrives 222 to 439 ms after the tap's acknowledgement at p95, against 100.
- **The phone was busy.** `contactsd` held one of its cores through the run, and every device-bound number is several times S9's.
- **Host input needs no stream on iOS 26.6.2.** `dtuhidd`'s input from the host landed without any stream: taps, drags, the home button and two keyboards. A tap was acknowledged in 53.7 ms at p95.

D23 is rewritten around screenshots as the iOS 26 frame source, with AVFoundation still to measure behind the camera permission and the display stream from iOS 27. D22's host input loses its stream requirement on iOS 26.

## Setup

**Host.** Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2 (25G83). Other sessions used it throughout: an Android emulator, Simulators, and build and Java processes (see Conditions).

**Apple tools.** Xcode 27.0 (27A266a); CoreDevice and `devicectl` 642.16; the developer disk image of Xcode 27.0, which the phone has mounted (spike S9).

**Device.** iPhone SE (3rd generation), `iPhone14,6` (`D49AP`), iOS 26.6.2 (23G90), a 750×1334 screen at 2x (375×667 points). It was on USB, paired with this Mac and trusted, with Developer Mode on, Auto-Lock set to Never and the battery at 100% on USB power. Every series read lockdown's `PasswordProtected` before starting, and none found the phone locked. AssistiveTouch is on, so its button floats at the top centre of every picture. The phone's identifier stays in `MOBILE_AGENT_IOS_UDID`. No result or line of this report names it, and none names the phone's name or the owner's content.

**Crates.** The spike uses the following crates, through `sfw cargo add`; the lock file holds 169.

- `idevice` 0.1.68, pinned with `=`, with the features `usbmuxd`, `core_device_proxy`, `tunnel_tcp_stack`, `rsd`, `xpc`, `dvt`, `core_device` and `display_stream` (which carries the HID clients), plus `device_info` and `sysmontap` for the phone's load. Its default TLS is rustls 0.23.45 on aws-lc-rs 1.18.1; jktcp 0.1.7 comes through it.
- tokio 1.53.2.
- objc2 0.6.5, with `objc2-av-foundation` (its `objc2-core-media` feature on), `objc2-core-media`, `objc2-core-video`, `objc2-video-toolbox`, `objc2-core-media-io`, `objc2-core-foundation` and `objc2-foundation` 0.3.2, block2 0.6.2 and dispatch2 0.3.1.
- png 0.18.1, plist 1.10.1, serde_json 1.0.151, uuid 1.27.0, libc 0.2.190 and anyhow 1.0.104.
- The frame pipeline is the Simulator driver's: `scale_bgra`, `pixels_hash`, `content_hash`, `fit_within`, `status_bar_rows` and `FrameView` from this repository's `crates/simulator`, a path dependency built without its default features.

Rust 1.98.1, release profile.

**Code.** The timed series ran on this spike's measurement code as committed in `6cb976a`, with three differences. The table generator came after the series. So did the last candidate tolerances: the tables recount every tolerance from the saved frames, and the `changes_per_tolerance` field in the series files predates the final list. The phone-load probe was written after the series, then run. Earlier exploratory runs (`src/explore.rs`) went to the scratch directory and are not reported, except where Measurements says so.

**Privileges.** One unprivileged process; no `sudo` anywhere. Nothing was installed on the phone, and no setting changed. The spike copies S9's usbmuxd, lockdown and tunnel setup, the quiet-host guard and the percentiles (`spikes/s11-ios-frames/README.md`).

**Screens.** Settings' top level, relaunched through CoreDevice's app service before each series. Bluetooth's page, opened from its row at (187, 550) points and closed with its back button at (38, 42); no switch lies under either point. Settings' list, scrolled by drags along its labels at x = 187 points. Safari's address field, for the keyboards. The home screen, for the home button.

**Method.** Every timed series first waits for a quiet host, S9's guard: a 1-minute load under 4, and no cargo, rustc, gradle, xcodebuild, kotlinc or java process above 5% of a core, checked every 30 s within a 90-minute budget. After that budget, a series may run under a load of 6, marked as such in its tables. Each series then reads the lock state, relaunches Settings, starts its frame source, and discards the first 2 s of frames. Input series discard their first two inputs and keep 20. Every result file holds the samples, the load before and after, and the frames it judged.

- **Frames.** Each frame becomes a BGRA picture: a decoded PNG for the screenshot source, a pixel buffer for AVFoundation and for the display stream through VideoToolbox. It is downscaled with `scale_bgra` to the 1000-pixel long edge (562×1000 for this screen) and hashed with `pixels_hash` and `content_hash`. Below the status bar, it is compared with the frame before it. The comparisons are the mean absolute byte difference, the share of pixels whose largest channel moved by more than 2, 4, 8, 16 or 32 levels, the largest difference, and the largest difference between the two 32-pixel content thumbnails that `content_hash` hashes. The status bar is the top 7.5% of rows, `status_bar_rows` as `content_hash` leaves it out.
- **Screenshots as a frame source.** A poller requests DVT's `takeScreenshot`, or CoreDevice's `capturescreenshot` in PNG, back to back on a connection of its own. A decode thread turns each PNG into BGRA, stripping 16-bit pictures to 8 bits. The next capture goes out while the previous picture is decoded, so the rate is the capture's. Two or four pollers run side by side on one tunnel in the parallel series.
- **CPU.** This process's user and system time from `getrusage` over the window, every thread included, as a share of one core. The helpers that may carry part of the work are sampled from `ps` beside it: usbmuxd, which relays the tunnel, the VideoToolbox decoder service and the screen-capture assistant.
- **Input.** Taps and drags go through `dtuhidd` as `mainTouchscreen` reports, the shape of `idevice`'s `hid` tool. A tap is a contact, 50 ms, and a release; it is acknowledged when the release report is written. A drag moves 240 points in 12 reports 16 ms apart and rests 150 ms before the lift, so the list does not fling. The latency is from a report's write to the analysis of the first frame whose change passes the tolerance. The series also keeps the last frame before each input as a reference, to tell which frame first differed at all.
- **The checks without timing** are one try each: every display-stream start form, AVFoundation's discovery and a 5 s capture, the touch surfaces, the home button, Indigo's keyboard event, the virtual keyboard, and a relaunched Settings judged by its picture.

**Rerun**, with the phone on USB, unlocked and trusted, and `MOBILE_AGENT_IOS_UDID` exported:

```sh
cd spikes/s11-ios-frames
cargo test                                       # unit tests: assembler, pipeline comparisons, PNG decode, guard
cargo build --release
./target/release/s11-ios-frames device           # results/device.json: phone and host versions
./target/release/s11-ios-frames launch-render    # results/launch-render.json: does a relaunched Settings draw
./target/release/s11-ios-frames phone-load       # results/phone-load.json: the phone's own CPU over 10 s
./target/release/s11-ios-frames display-stream   # results/display-stream.json: every start form
./target/release/s11-ios-frames measured         # every timed series behind the guard, then results/tables.md
./target/release/s11-ios-frames input-checks     # results/input-checks.json: home button, keyboards (types into Safari's field, then restores it)
./target/release/s11-ios-frames avfoundation     # results/avfoundation.json: discovery, permission, 5 s of capture
./target/release/s11-ios-frames report           # regenerate results/tables.md
```

`all` runs everything in that order and adds the AVFoundation series (`avf-still`, `avf-taps`, `avf-drags`, `avf-scroll`) once the camera is authorized. Each series also runs alone. `S11_RESULTS` and `S11_SCRATCH` move the results and the scratch directory for screenshots. `S11_DRY_RUN=1` runs each series briefly without the guard or the lock check, to try the code paths; its numbers are not reported. `S11_CAMERA_WAIT_S` makes `avfoundation` ask for the camera and wait that long for the answer.

### Conditions

The guard made 243 checks between 00:23:40 and 02:34:11, one row each in `results/hygiene.tsv`; the full blocks are in the git-ignored `results/hygiene.log`. 231 checks found the host busy. Another session's Android emulator ran throughout: it was above 5% of a core at 229 of the busy checks, up to 8.3 cores. The 1-minute load at the busy checks ranged from 3.5 to 16.1. At 47 checks a watched process was above 5% as well: java, and 5 times rustc, this spike's own compiles of its table generator while the guard waited. Every series started at a load under 4, so none used the fallback. The load rose during several series, to 6.1 at most. The two-poller drag series waited 20 minutes without a quiet check and was stopped, so it is not reported. Earlier runs with `S11_DRY_RUN=1` went to the scratch directory and are not reported either.

| Series | Checks that waited | Started | Finished | 1-minute load before / after |
|---|---:|---|---|---|
| transport | 46 | 2026-10-09 00:46:44 | 2026-10-09 00:47:06 | 3.23 / 2.99 |
| shot-latency-dvt | 0 | 2026-10-09 00:47:06 | 2026-10-09 00:47:13 | 2.99 / 2.77 |
| shot-latency-coredevice | 0 | 2026-10-09 00:47:13 | 2026-10-09 00:47:21 | 2.77 / 2.55 |
| shots-still | 0 | 2026-10-09 00:47:21 | 2026-10-09 00:47:56 | 2.55 / 3.38 |
| shots-taps | 5 | 2026-10-09 00:50:27 | 2026-10-09 00:51:43 | 3.49 / 6.14 |
| shots-drags | 54 | 2026-10-09 01:18:48 | 2026-10-09 01:20:09 | 3.91 / 5.57 |
| shots-scroll | 28 | 2026-10-09 01:34:11 | 2026-10-09 01:34:27 | 3.39 / 4.17 |
| shots-parallel-still | 4 | 2026-10-09 01:36:27 | 2026-10-09 01:37:03 | 3.96 / 5.05 |
| shots-parallel-scroll | 1 | 2026-10-09 01:37:33 | 2026-10-09 01:37:49 | 3.91 / 3.67 |
| shots-quad-still | 0 | 2026-10-09 01:37:49 | 2026-10-09 01:38:25 | 3.67 / 4.35 |
| shots-quad-scroll | 24 | 2026-10-09 01:50:27 | 2026-10-09 01:50:43 | 3.90 / 4.76 |
| shots-parallel-taps | 40 | 2026-10-09 02:18:53 | 2026-10-09 02:20:08 | 3.35 / 5.05 |

## Measurements

### The CoreDevice display stream: refused before iOS 27

The display service answers on iOS 26.6.2 but starts no stream. `getmediasupportinfo` reports `supportedFeatures` 0, "No supported features are available", with AVConference framework 2215.5.1, and `getmediastreamserverstatus` reports no session. Each of six start forms got the same refusal from the phone, CoreDevice error 9021 (`results/display-stream.json`):

| Form | Device's answer |
|---|---|
| idevice's tools: an audio stream, then video on the same client session | refused: audio startmediastream refused: com.apple.dt.CoreDeviceError 9021: Remote control requires iOS 27.0 or later on this device. |
| video only, as pymobiledevice3 starts it | refused: video startmediastream refused: com.apple.dt.CoreDeviceError 9021: Remote control requires iOS 27.0 or later on this device. |
| video with the action identifier and protocol version 2, as pymobiledevice3 invokes it | refused: com.apple.dt.CoreDeviceError 9021: Remote control requires iOS 27.0 or later on this device. |
| the same with the host's CoreDevice version | refused: com.apple.dt.CoreDeviceError 9021: Remote control requires iOS 27.0 or later on this device. |
| startvideooutput, the feature idevice's start_video_output invokes | refused: CoreDevice.ActionError 2: Action 'com.apple.coredevice.feature.startvideooutput' is not implemented. |
| idevice's tools' form with the Camera app in front | refused: audio startmediastream refused: com.apple.dt.CoreDeviceError 9021: Remote control requires iOS 27.0 or later on this device. |

- **Every form.** The refusal does not depend on how the request is made: an audio stream first, as `idevice`'s tools and Device Hub start the session; video alone, as pymobiledevice3 does; with the action identifier and protocol version 2; and with the host's CoreDevice version, 642.16, in place of the crate's 629.3. `startvideooutput`, the other feature `idevice` offers, is not implemented on the phone.
- **The camera.** With the Camera app in front and holding the camera, the answer is the same 9021, not pymobiledevice3's media-in-use code 9022: the phone checks its iOS version first. So whether the stream locks out the camera on this phone could not be seen.
- **Not exercised.** The receive loop and the decoder never ran on this phone: the RTP access-unit assembler with keyframe requests and receiver reports (`src/stream.rs`), and the VideoToolbox HEVC decoder to BGRA (`src/vt.rs`, `src/display.rs`). Nothing was measured of the stream's rate, CPU, loss, first frame, behaviour on a still screen, or effect on the status bar.

### AVFoundation: not measured, for want of the camera permission

- **The device.** CoreMediaIO's screen-capture allowance, `kCMIOHardwarePropertyAllowScreenCaptureDevices`, reads 0 in a fresh process on macOS 26.6.2; the spike sets it to 1. In three exploratory runs early in the session (`s11-ios-frames avf-probe`, which writes no result), the phone then appeared as one external capture device of media type muxed, model "iOS Device", within 0.2 s. Its unique identifier was neither the UDID nor contained it, so the spike matched it by the phone's name read from lockdown, which matched. The active format's media subtype was `isr ` (0x69737220), with its dimensions reported as 0x0 before capture.
- **The permission.** The camera authorization for this process read 0, not determined. macOS attributes the request to the terminal application that started the process, whose `Info.plist` carries a camera usage description and whose signature carries the camera entitlement. The spike's request (`requestAccessForMediaType:` for video) was announced to the maintainer and then waited 10 minutes without an answer. The authorization still read 0 afterwards.
- **A session without it.** `startRunning` returned after 258 ms and the session delivered no sample buffer in 5 s.
- **Later.** The phone no longer appeared at all, not within 60 s and in no muxed or external device, and the screen-capture assistant's launchd job (`com.apple.cmio.iOSScreenCaptureAssistant`) was not running (`results/avfoundation.json`).
- **Not measured.** The AVFoundation stream's rate, CPU, pixel format, noise floor, latency and status bar. The spike's `avf-still`, `avf-taps`, `avf-drags` and `avf-scroll` series measure them once the camera is allowed.

### One screenshot at a time, on a busy phone

| Step | n | This run | S9 | 1-minute load before / after |
|---|---:|---:|---:|---|
| lockdown session with the pairing record | 20 | 189.61 / 441.44 | 22.13 / 30.73 | 3.23 / 2.99 |
| tunnel: `CoreDeviceProxy` | 20 | 150.55 / 502.12 | 42.21 / 52.28 | 3.23 / 2.99 |
| tunnel: RSD handshake | 20 | 77.93 / 89.12 | 15.86 / 20.67 | 3.23 / 2.99 |
| tunnel and RSD, total | 20 | 244.92 / 928.33 | 59.90 / 69.98 | 3.23 / 2.99 |
| first DVT connection | 20 | 70.93 / 86.05 | 36.95 / 38.50 | 3.23 / 2.99 |
| DVT `takeScreenshot`, Settings | 20 | 105.49 / 184.40 | 42.33 / 43.54 | 2.99 / 2.77 |
| CoreDevice `capturescreenshot` (PNG), Settings | 20 | 131.08 / 306.56 | 42.49 / 43.38 | 2.77 / 2.55 |
| DVT `takeScreenshot`, the home screen (16-bit PNG of 3.0 MB) | 10 | 1118.80 / 3310.56 | - | - |

- **The phone was slower than during S9.** With the host quiet (a 1-minute load of 3.2 to 2.5), each step took several times S9's: a lockdown session 190 ms at p50 against 22, the tunnel with RSD's handshake 245 against 60, a DVT screenshot 105 against 42 and a CoreDevice one 131 against 42. The first capture on a new connection took 790 to 850 ms against 95 to 114. The phone was not idle. DVT's sysmontap read a total load of 103 to 113% of a core over 10 s, and `contactsd`, the Contacts daemon, held one core at 100% in 9 of the 10 samples (`results/phone-load.json`). S9's numbers are the phone's at rest; every device-bound number in this report is a busy phone's, the pessimistic case.
- **The picture's size follows the screen.** Settings' top level is an 8-bit RGB PNG of 141 KB. The home screen's wallpaper is a 16-bit RGB PNG of 3.0 MB with an ICC profile, which takes 1.1 s at p50 and 3.3 s at p95 through DVT.

### Screenshots as a frame source

Rate is frames analysed a second; CPU is this process's user and system time over the window as a share of one core, decoding included, and usbmuxd's beside it.

| Series | Pollers | Window s | Frames | fps | Interval p50 / p95 ms | Capture p50 / p95 ms | Identical PNGs | CPU % (this process) | usbmuxd % | Decode / scale / hash / diff p50 ms | 1-minute load before / after |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| shots-still | 1 | 30.0 | 233 | 7.8 | 105.5 / 237.2 | 105.9 / 230.0 | 246 of 247 | 6.5 | 3.2 | 3.06 / 0.87 / 0.41 / 1.42 | 2.55 / 3.38 |
| shots-parallel-still | 2 | 30.0 | 505 | 16.8 | 57.0 / 96.6 | 113.5 / 186.6; 112.8 / 182.2 | 523 of 527 | 12.0 | 6.1 | 3.03 / 0.86 / 0.41 / 1.41 | 3.96 / 5.05 |
| shots-quad-still | 4 | 30.0 | 545 | 18.2 | 49.9 / 106.7 | 156.1 / 274.4; 155.3 / 272.6; 153.7 / 272.0; 511.8 / 775.8 | 569 of 576 | 13.7 | 6.5 | 3.02 / 0.86 / 0.41 / 1.41 | 3.67 / 4.35 |
| shots-scroll | 1 | 10.0 | 95 | 9.5 | 98.9 / 141.8 | 101.1 / 158.2 | 13 of 108 | 7.2 | 4.8 | 3.26 / 0.86 / 0.41 / 1.41 | 3.39 / 4.17 |
| shots-parallel-scroll | 2 | 10.2 | 154 | 15.1 | 59.3 / 129.2 | 115.8 / 243.0; 121.8 / 291.3 | 26 of 180 | 15.5 | 7.4 | 5.05 / 1.31 / 0.62 / 2.10 | 3.91 / 3.67 |
| shots-quad-scroll | 4 | 10.1 | 197 | 19.4 | 45.5 / 116.3 | 171.3 / 334.2; 201.6 / 505.1; 161.4 / 396.7; 166.8 / 311.8 | 29 of 230 | 15.2 | 7.2 | 3.30 / 0.86 / 0.41 / 1.41 | 3.90 / 4.76 |

- **One poller.** 7.8 frames a second on a still screen and 9.5 while scrolling: each frame waits for its capture, so the interval between frames is the capture's time. The rest costs this process about 6 ms a frame at p50: the PNG's decode 3.0 ms, the downscale 0.9, both hashes 0.4 and the comparisons 1.4.
- **Two pollers**, DVT and CoreDevice side by side, roughly double the rate: 16.8 frames a second on a still screen and 15.1 while scrolling. Each capture slows to 113 to 122 ms.
- **Four pollers** add little: 18.2 and 19.4 frames a second. Each capture takes 150 to 200 ms, and one of the four connections got only 6 captures in 30 s. The phone serves about 18 to 19 captures a second: that is the ceiling of this path on this phone in this state.
- **CPU.** This process used 6.5 to 15.5% of a core, and usbmuxd, which relays the PNGs, 3.2 to 7.4%: together at most 22.9%, under the gate's 25% in every configuration. No VideoToolbox or screen-capture helper worked, as this path uses neither.
- **Identical pictures.** On a still screen, 246 of 247 consecutive PNGs from one poller were the same bytes. A daemon can therefore compare the PNG's bytes before decoding it, and decode only the pictures that differ.

### The noise floor and the tolerance

Consecutive frames of a still screen, compared below the status bar:

| Series | Pairs | Mean abs. difference, max | Pixels > 2 levels, max % | Pixels > 16 levels, max % | Largest difference, max | Thumbnail difference, max | `content_hash` changes | `pixels_hash` changes |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| shots-still | 233 | 0.000 | 0.000 | 0.000 | 0 | 0 | 0 | 0 |
| shots-parallel-still | 505 | 0.000 | 0.000 | 0.000 | 1 | 0 | 0 | 2 |
| shots-quad-still | 545 | 0.000 | 0.000 | 0.000 | 1 | 0 | 0 | 3 |

How many frames each candidate tolerance counts as a change:

| Tolerance | shots-still | shots-parallel-still | shots-quad-still | shots-scroll | shots-parallel-scroll | shots-quad-scroll |
|---|---:|---:|---:|---:|---:|---:|
| pixels_hash differs (exact) | 0 of 233 | 2 of 505 | 3 of 545 | 93 of 95 | 150 of 154 | 191 of 197 |
| any pixel below the status bar differs | 0 of 233 | 2 of 505 | 3 of 545 | 93 of 95 | 150 of 154 | 191 of 197 |
| a pixel below the status bar differs by more than 1 level | 0 of 233 | 0 of 505 | 0 of 545 | 93 of 95 | 150 of 154 | 191 of 197 |
| a pixel below the status bar differs by more than 2 levels | 0 of 233 | 0 of 505 | 0 of 545 | 93 of 95 | 150 of 154 | 191 of 197 |
| a pixel below the status bar differs by more than 4 levels | 0 of 233 | 0 of 505 | 0 of 545 | 93 of 95 | 150 of 154 | 191 of 197 |
| content_hash differs (32-pixel thumbnail) | 0 of 233 | 0 of 505 | 0 of 545 | 93 of 95 | 150 of 154 | 191 of 197 |
| content thumbnail differs by more than 1 level | 0 of 233 | 0 of 505 | 0 of 545 | 92 of 95 | 149 of 154 | 191 of 197 |
| content thumbnail differs by more than 2 levels | 0 of 233 | 0 of 505 | 0 of 545 | 92 of 95 | 148 of 154 | 190 of 197 |
| content thumbnail differs by more than 4 levels | 0 of 233 | 0 of 505 | 0 of 545 | 92 of 95 | 148 of 154 | 190 of 197 |
| content thumbnail differs by more than 8 levels | 0 of 233 | 0 of 505 | 0 of 545 | 91 of 95 | 147 of 154 | 190 of 197 |
| more than 0% of content pixels differ by more than 16 levels | 0 of 233 | 0 of 505 | 0 of 545 | 93 of 95 | 150 of 154 | 191 of 197 |
| more than 0.05% of content pixels differ by more than 16 levels | 0 of 233 | 0 of 505 | 0 of 545 | 92 of 95 | 148 of 154 | 190 of 197 |
| more than 0.2% of content pixels differ by more than 16 levels | 0 of 233 | 0 of 505 | 0 of 545 | 91 of 95 | 148 of 154 | 189 of 197 |
| more than 0.05% of content pixels differ by more than 32 levels | 0 of 233 | 0 of 505 | 0 of 545 | 92 of 95 | 148 of 154 | 190 of 197 |

- **One poller.** Over 233 pairs, no pixel differed, below the status bar or above it (no minute changed on the clock).
- **Two and four pollers.** Each saw one moment in 30 s when some pixel below the status bar moved by exactly one level and back. Each poller's own PNGs changed once, so the screen itself changed; with two sources interleaved, the moment shows as two or three changed frames. Over every pair, the mean absolute difference stayed 0.000 and no pixel moved by more than one level. The source is not identified (an inference: iOS 26's glass highlights, which follow the device's motion).
- **The tolerance.** A frame counts as a change when a pixel below the status bar moved by more than 2 levels from the frame before it (`Tolerance::ContentAbove(2)` in the spike). It counts none of the 1,283 still-screen frames. Every tap's and drag's first crossing was the first frame that differed at all from the frame before the input: 20 of 20 for each kind.
- **`content_hash` misses the small changes.** The 32-pixel thumbnail averages a change like a highlighted back button away. On this 667-point screen, most of the back button lies inside the band that `status_bar_rows` leaves out: 7.5% of the height, 50 points, scaled from an iPhone 17's 59 of 874, while the SE's status bar is 20 points. So the button's highlight touched 0.0012% of the content's pixels. The thumbnail saw the back tap on its first changed frame 12 times in 20, and 67 ms later at p50.

### Input to the first changed frame

The chosen tolerance's rows (every tolerance is in `results/tables.md`):

`shots-taps` and `shots-drags`:

| Tolerance | Input | n | Missed | From the start | From the acknowledgement | On the first changed frame |
|---|---|---:|---:|---:|---:|---:|
| a pixel below the status bar differs by more than 2 levels | tap, open | 20 | 0 | 325.5 / 492.1 | 273.8 / 439.4 | 20 of 20 |
| a pixel below the status bar differs by more than 2 levels | tap, back | 20 | 0 | 185.7 / 303.0 | 133.8 / 251.1 | 20 of 20 |
| a pixel below the status bar differs by more than 2 levels | drag | 20 | 0 | 272.0 / 506.1 | - | 20 of 20 |
| content_hash differs (32-pixel thumbnail) | tap, open | 20 | 0 | 325.5 / 492.1 | 273.8 / 439.4 | 20 of 20 |
| content_hash differs (32-pixel thumbnail) | tap, back | 20 | 0 | 252.8 / 403.5 | 200.5 / 352.6 | 12 of 20 |
| content_hash differs (32-pixel thumbnail) | drag | 20 | 0 | 272.0 / 506.1 | - | 20 of 20 |

Tap dispatch through `dtuhidd`, ms p50 / p95: contact written 0.08 / 0.71 (open) and 0.09 / 0.61 (back); release written, the tap's acknowledgement with its 50 ms hold, 52.20 / 53.71 (open) and 52.38 / 53.44 (back).

Drag dispatch, ms p50 / p95: first report written 0.07 / 0.88; lift written 380.23 / 398.10 (12 reports 16 ms apart and a 150 ms rest).

`shots-parallel-taps`:

| Tolerance | Input | n | Missed | From the start | From the acknowledgement | On the first changed frame |
|---|---|---:|---:|---:|---:|---:|
| a pixel below the status bar differs by more than 2 levels | tap, open | 20 | 0 | 292.1 / 360.6 | 240.6 / 309.2 | 20 of 20 |
| a pixel below the status bar differs by more than 2 levels | tap, back | 20 | 0 | 119.6 / 273.2 | 68.1 / 221.5 | 20 of 20 |
| content_hash differs (32-pixel thumbnail) | tap, open | 20 | 0 | 292.1 / 360.6 | 240.6 / 309.2 | 20 of 20 |
| content_hash differs (32-pixel thumbnail) | tap, back | 20 | 0 | 264.2 / 600.1 | 212.6 / 546.9 | 5 of 20 |

Tap dispatch through `dtuhidd`, ms p50 / p95: contact written 0.05 / 0.07 (open) and 0.06 / 0.47 (back); release written, the tap's acknowledgement with its 50 ms hold, 51.54 / 52.55 (open) and 51.73 / 53.51 (back).

- **A row that opens a page.** Bluetooth's row responds after the release. Its first changed frame came 274 ms after the tap's acknowledgement at p50 and 439 at p95. At the median, the capture that showed the change was requested 154 ms after the release and took 110 ms; the capture before it, requested 42 ms after the release, still showed nothing.
- **A back button.** It highlights on contact, so its change was in the capture already in flight at the release: at the median, that capture was requested 8 ms before the release and took 150 ms. The first changed frame came 134 ms after the acknowledgement at p50 and 251 at p95.
- **A drag.** The first changed frame came 272 ms after the first report at p50 and 506 at p95. At the median, the last capture that showed the list still was requested 1 ms after the first report (79 ms at most). The first that showed it moving was requested 145 ms after it and took 135 ms. Once, a capture requested 126 ms before the drag already showed it moving, so a capture's moment can fall late in its request.
- **Two pollers.** Halving the interval between frames brings each change in sooner, still over the gate: 68 and 222 ms after the acknowledgement for the back button, 241 and 309 ms for the row (`shots-parallel-taps`).
- **Where the time goes.** A capture shows the screen at some moment between its request and its arrival, which comes 100 to 150 ms after the request; 6 ms of decoding and comparison follow. Most of each latency is the capture, on top of the app's own response.

### `dtuhidd` input without a stream

On iOS 26.6.2 the phone takes `dtuhidd`'s input with no display stream running; no stream can run there. That is the opposite of what pymobiledevice3 and `idevice` found on iOS 27, where the surfaces stay unauthenticated without a stream. `universalhidservice` lists four surfaces: the touchscreen (service 257, usage page 13, usage 4), a keyboard (512), the main screen's buttons (1026, usage page 11) and a vendor surface (1280). The first two taps of the session, on the dock's Settings icon and on Settings' Bluetooth row, opened what they touched, and every tap and drag of the series changed the screen. One try each of the rest (`results/input-checks.json`), judged by the screenshot poller and by looking at the screenshots, which stay in the scratch directory:

| Check | Sent | What the screenshot poller saw |
|---|---|---|
| Home button (Indigo button event, consumer page 0x0C usage 0x40) | true | 1 of 7 frames changed, the first 988 ms after the input; the next screenshot is a 16-bit PNG of 3046136 bytes |
| Tap on Safari's address field | yes | 8 of 9 frames changed, the first 339 ms after the input |
| One key through Indigo's keyboard event (usage 0x14, q) | true | 4 of 11 frames changed, the first 56 ms after the input |
| "example" through the virtual keyboard (`mainKeyboard`) | 7 keys, keyboard removed: true | 11 of 23 frames changed, the first 294 ms after the input |
| Clean-up: 10 backspaces and escape through the virtual keyboard | 11 keys, keyboard removed: true | screenshot after: 282923 bytes |

The HID surfaces `universalhidservice` lists: CoreDevice touchscreen(nil) (257, usage page 13 usage 4); CoreDevice keyboard (512, usage page 0 usage 0); CoreDevice mainScreenButtons (1026, usage page 11 usage 1); CoreDevice avpCustom (1280, usage page 65377 usage 91).

- **The home button.** Indigo's button event (consumer page 0x0C, usage 0x40, held 80 ms) left Settings for the home screen. The poller saw it in its next frame, 988 ms after the press, since the home screen's picture is a 3 MB PNG. On this phone the physical home button does not work.
- **The keyboards.** A tap on Safari's address field focused it and selected the address. One key through Indigo's keyboard event (usage 0x14) replaced the selection with "q", although pymobiledevice3 reports `dtuhidd` ignoring that event on iOS 27. "example" through the virtual keyboard, which the host registers as Device Hub's `mainKeyboard` and removes afterwards, followed: the field read "qexample". The software keyboard stayed on screen.
- **The clean-up.** Ten backspaces and an escape through the virtual keyboard left the field, and Safari showed its tab's address as before.

### Settings relaunched through DVT draws nothing on this phone

During the session, Settings relaunched through DVT process control with `KillExisting` came up with its window black, and stayed black after 25 s and after a tap. Clock relaunched the same way drew, and so did Settings opened from its icon or relaunched through CoreDevice's app service. A check of three relaunches each way, judged by a screenshot 3 s later (`results/launch-render.json`):

| Launcher | Reply | Reply ms | PNG bytes 3 s later | Content mean (B, G, R) | Drew |
|---|---|---:|---:|---:|---|
| Dvt | pid | 168.0 | 28182 | 0.40 | false |
| AppService | pid | 133.6 | 141286 | 26.18 | true |
| Dvt | pid | 157.8 | 28182 | 0.40 | false |
| AppService | pid | 134.2 | 141286 | 26.18 | true |
| Dvt | pid | 140.5 | 28251 | 0.40 | false |
| AppService | pid | 134.7 | 141351 | 26.18 | true |

DVT's reply carried a pid every time, but nothing was drawn. The series therefore relaunch Settings through the app service. S9's DVT launches of Settings drew, about four hours earlier on the same phone (its screenshots were the same 141 KB picture), so this is a state the phone got into rather than a rule. It is the case S9 left to S11: a launch's reply says the process runs, not that the app is on screen, and here it never was.

### What was not measured

- **The display stream**, every number of it, since the phone refuses it before iOS 27 (above). So also whether it sends frames on a still screen, its first frame after the start, its losses and keyframe requests, the camera it holds, and the status bar under it.
- **AVFoundation**, every number of it, for want of the camera permission (above). So also its pixel format, its noise floor, and whether capture switches the status bar to demo mode. The one session that ran without the permission delivered nothing, and the status bar showed the real time before and after it.
- **The fixtures' list screen.** Installing the fixtures needs a signing identity, and this host has none yet (as in S9). Settings stood in for it.
- **A phone call during a stream**, excluded by the brief.
- **The phone at rest.** `contactsd` kept a core busy through the run. A rerun once the phone is idle would show how much of the capture time is that.

## Verdict

**No-go for D23 as written; go for D22's host input.** Neither stream runs on this phone today. Screenshots run, exact enough, and fail the rate and latency gates:

| Criterion | Source | Measured | Verdict |
|---|---|---|---|
| 30 fps sustained while scrolling | shots-scroll | 9.5 fps | fail |
| under 25% of a core, decoding included | shots-scroll | 7.2% here, 12.0% with usbmuxd | pass |
| 30 fps sustained while scrolling | shots-parallel-scroll | 15.1 fps | fail |
| under 25% of a core, decoding included | shots-parallel-scroll | 15.5% here, 22.9% with usbmuxd | pass |
| 30 fps sustained while scrolling | shots-quad-scroll | 19.4 fps | fail |
| under 25% of a core, decoding included | shots-quad-scroll | 15.2% here, 22.5% with usbmuxd | pass |
| 30 fps sustained while scrolling | avf-scroll | not measured | - |
| A tap's first changed frame within 100 ms of its acknowledgement at p95 | shots-taps, open | 439.4 ms at p95 | fail |
| the same, from the tap's contact report | shots-taps, open | 492.1 ms at p95 | fail |
| A tap's first changed frame within 100 ms of its acknowledgement at p95 | shots-taps, back | 251.1 ms at p95 | fail |
| A tap's first changed frame within 100 ms of its acknowledgement at p95 | shots-parallel-taps, open | 309.2 ms at p95 | fail |
| the same, from the tap's contact report | shots-parallel-taps, open | 360.6 ms at p95 | fail |
| A tap's first changed frame within 100 ms of its acknowledgement at p95 | shots-parallel-taps, back | 221.5 ms at p95 | fail |
| A tap's first changed frame within 100 ms of its acknowledgement at p95 | avf-taps | not measured | - |
| No still-screen frame counted as a change | shots-still | 0 of 233 frames | pass |
| No still-screen frame counted as a change | shots-parallel-still | 0 of 505 frames | pass |
| No still-screen frame counted as a change | shots-quad-still | 0 of 545 frames | pass |
| D22: tap dispatch under 150 ms at p95 (release written, 50 ms hold included) | dtuhidd, shots-taps | 53.7 ms at p95 | pass |
| D22: the tap reaches the screen | dtuhidd, shots-taps | 40 of 40 taps changed the screen | pass |

**D23 changes** (rewritten in design.md):

- The display stream is refused before iOS 27, so it is not the frame source on iOS 26. The driver asks the display service's `getmediasupportinfo` and uses the stream only on a device that lists features.
- AVFoundation stays the stream to measure on iOS 26, behind the macOS camera permission of the app that starts the daemon. The spike's `avf-*` series measure it once the maintainer allows the camera.
- Screenshots are the frame source until then: two pollers, DVT and CoreDevice, about 15 to 17 frames a second on this busy phone. The PNG's bytes are compared first, and a picture is decoded only when they differ.
- The change signal is a pixel below the status bar moving by more than 2 levels. The status bar's band is the device's (20 points on the SE), not `status_bar_rows`' 7.5%.
- The device is a target with slow frames. Settle needs two capture intervals without a change, and a re-read after input waits up to 500 ms for the first changed frame. The `ios-device` gates on frames have to move in task 2.4.

**D22's host input** (rewritten in design.md) needs no stream on iOS 26.6.2. Touches, the home button and keys through Indigo and through the virtual keyboard all landed, and a tap's dispatch met the gate at 53.7 ms at p95. The display stream is opened only where the device requires it (iOS 27, by the sources). S10 decides between this path and the runner's synthesis.

**For the tasks:**

- **3.5, frames and screenshots.** Implement the screenshot poller (`src/shots.rs`): a decode thread fed by connections of their own, a byte comparison before the decode, `scale_bgra` to the 1000-pixel edge, and the comparison below the device's status bar with the two-level tolerance. Behind the same `FrameSource`, add AVFoundation, gated on the camera authorization (`src/avf.rs`; the phone is found by name, and the allowance must be set first), and the display stream, gated on `getmediasupportinfo` (`src/stream.rs`, `src/vt.rs`). The test's "30 fps under the CPU gate for a minute" cannot pass with screenshots on this phone: 19 frames a second at most.
- **4.5, input.** On iOS 26.6.2, `dtuhidd` from the host is a working path with no runner and no stream: taps, drags with a rest before the lift, the home button, Indigo's keys and the virtual keyboard. S10's numbers for the runner's synthesis decide the default. A registered virtual keyboard must be removed afterwards, as the spike does, since Xcode's notes say a device can stay in hardware-keyboard mode for 2 minutes.
- **5.2, the mirror on a device.** Windows without events settle on screenshot frames at 15 to 19 a second. The first change comes 70 to 310 ms after an input's acknowledgement with two pollers, and 130 to 440 ms with one (p50 to p95). So the re-read after input has to wait for it rather than settle on the first quiet interval. The tests take a fake frame source with a slow rate and identical frames on a still screen.
- **For the spike review (2.4), outside S11's decisions.** Settings relaunched through DVT process control drew nothing on this phone, while the app service's relaunch drew (`results/launch-render.json`). D19's launch reply does not show that the app is on screen, and a frame check after `open` catches it. The design's Context and Risks still say that `dtuhidd`'s reports need a display stream; on iOS 26.6.2 they do not.
