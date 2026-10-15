# S3: Simulator input over dtuhidd and the legacy Indigo client

Spike S3 (task 2.6) checks the input half of D6 from Rust: a host process opens the dtuhidd XPC transport on a booted iOS 26.5 Simulator under Xcode 27, pays the activation probe once, and delivers taps, long presses, swipes, a two-finger pinch, typing, special keys and hardware buttons, measured against the spec's 20 ms warm-tap gate. As a one-shot fact-finding step it also sends taps through SimulatorKit's legacy Indigo client on a fresh boot, before and after dtuhidd is opened on the same device. dtuhidd works for every kind of input tried and dispatches a tap in 0.03 ms at the median: go. Indigo touch also worked on this host, before and after dtuhidd was activated, which contradicts the research note: go as the fallback it is meant to be, with nothing on this host needing it.

## Setup

- Host: Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2 (25G83). Other agents ran an Android emulator, benchmarks and Simulator boots throughout, pushing the 1-minute load as high as 268. Every timed step waited until the 1-minute load was under 10 and logs the three load averages it started under; `pgrep` found no Gradle, xcodebuild or kotlinc process before the final dtuhidd run.
- Xcode 27.0 (27A266a). CoreSimulator 1171.7 and CoreSimDeviceIO 1171.7 from `/Library/Developer/PrivateFrameworks`; the `SimDevice` class is vended by that same CoreSimulator, which is at or past 1155.4, the first version that injects dtuhidd. SimulatorKit 1005.2 from `Xcode.app/Contents/SharedFrameworks`, loaded only for the Indigo step. Runtime iOS 26.5 (23F77).
- Devices, all iPhone 17 (1206x2622 pixels at 3x, so 402x874 points), headless, DeviceHub.app and Simulator.app never running:
  - `mobile-agent-spike` carried every dtuhidd measurement, with the Expo fixture (`dev.mobileagent.fixture.expo`) already installed. It was rebooted once, at 23:27 on 2026-09-26. For the pinch, one screenshot was added to its Photos library with `simctl addmedia` and Photos was opened, its welcome sheet dismissed and its notification prompt declined; the photo was deleted at the end and sits in Recently Deleted. To read dtuhidd's log, the guest's log level for `com.apple.dt.coredevice` was raised to debug; it survived the reboot and was reset at the end.
  - `mobile-agent-s3-indigo` was created fresh for the Indigo step, twice (the first copy was deleted during an interruption), and deleted at the end.
- App geometry, in points: tab bar centre line y 812, Login tab x 40.2, List tab x 120.6, username field (201, 113), password field (201, 168), Sign in (201, 218).
- Rust 1.95.0. Crates, all added with `sfw cargo add`: `objc2` 0.6.4 with the `exception` feature, `objc2-foundation` 0.3.2, `objc2-core-foundation` 0.3.2, `objc2-io-surface` 0.3.2, `block2` 0.6.2, `dispatch2` 0.3.1, `libc` 0.2.189. No XPC crate: libxpc is bound by hand, 25 functions and 2 data symbols in one `extern "C"` block. Both profiles set `panic = "unwind"`.
- Reused from S2 unchanged: the `dlopen` loader, the exception guard, the forwarded-send helper for ROCKit proxies, the display registration and the frame pipeline. `main_display` takes `-[SimDevice io]` instead of the device, and the hash gained a size parameter and a crop (below).
- Code: `spikes/s3-simulator-hid`, kept privately since the repository was made public (`docs/spikes/README.md`). Rerun with `cargo build --release`, then `./measure.sh dtuhidd <udid> <out-dir>`, `./measure.sh indigo <FixtureExpo.app> <out-dir>` (creates, uses and deletes its own device) and `./measure.sh unlock <udid> <out-dir>`. `cargo test` runs the unit tests; two of them load the host's CoreSimulator and one checks where SimulatorKit is.

Private symbols, resolved at run time:

| Symbol | Kind | Found in |
|---|---|---|
| `xpc_endpoint_create_mach_port_4sim(mach_port_t, uint64_t, uint64_t)` | C, `dlsym` | `/usr/lib/system/libxpc.dylib` |
| `xpc_connection_enable_sim2host_4sim(xpc_connection_t)` | C, `dlsym` | `/usr/lib/system/libxpc.dylib` |
| `xpc_connection_create_from_endpoint(xpc_endpoint_t)` | C, `dlsym` (public in the SDK, resolved like the other two, as idb does) | `/usr/lib/system/libxpc.dylib` |
| `-[SimDevice lookup:error:]`, returning `mach_port_t` | Objective-C, `msg_send!` | CoreSimulator |
| `-[SimDevice deviceType]`, `-[SimDeviceType mainScreenSize]`, `-mainScreenScale` | Objective-C | CoreSimulator |
| `_TtC12SimulatorKit24SimDeviceLegacyHIDClient`: `+alloc`, `-initWithDevice:error:`, `-sendWithMessage:freeWhenDone:completionQueue:completion:` | Objective-C class found by its mangled name | SimulatorKit |
| `IndigoHIDMessageForMouseNSEvent(CGPoint *, CGPoint *, IndigoHIDTarget, NSEventType, NSSize, IndigoHIDEdge)` | C, `dlsym` | SimulatorKit |

`-lookup:error:` goes through plain `msg_send!` with a `u32` return and an `NSError **` out parameter; the debug build's signature check accepts it. None of these call sites needed the Objective-C shim.

### How input is sent

- Connection, as in idb and iosef: `-lookup:error:` for `com.apple.coredevice.feature.remote.hid.digitizer`, `xpc_endpoint_create_mach_port_4sim(port, 0, 0)`, `xpc_connection_create_from_endpoint`, `xpc_connection_enable_sim2host_4sim`, a serial target queue, an event handler, resume.
- Messages are dictionaries `{messageType, isBarrier, featureIdentifier, payload}`. `IndigoDigitizerEvent {pointOne {x, y}, pointTwo?, eventType, edge, target}` carries one or two contacts; `IndigoKeyboardButtonEvent {usageCode, state}` a USB HID keyboard usage; `IndigoButtonEvent {usagePage, usageCode, state}` a Consumer-page button (home 0x40, lock 0x30). Integers are `uint64`, coordinates are `double` ratios of the portrait framebuffer from the top left, `state` is 1 for down and 2 for up. A point in iOS points becomes a ratio as `point × scale / pixels`, idb's formula.
- Activation: the first message is a barrier `IndigoKeyboardButtonEvent {usageCode 0, state 2}` sent with a reply handler, a 4 s deadline and up to five attempts 4 s apart. Until the reply arrives, the gate holds input; on the reply the session sleeps a 200 ms tail, as idb and iosef do, then flushes what it held in order. `activate --early-tap` makes the pre-activation path measurable, and `--tail` and `--no-gate` vary it.
- Gestures: a tap is down, hold, up (40 ms hold unless stated); a swipe is a down, 20 interpolated positions spread over the duration, the last position repeated and the up (idb repeats it to avoid inertial scrolling on arm64 Simulators; here the list still coasted in proportion to the speed); a pinch sends both contacts in every event; typing sends down and up per character, with Left Shift around shifted ones.
- Timing: send completion is the request until `xpc_connection_send_message` returns for the touch-down, which is when XPC has taken the message. The send barrier is the request until `xpc_connection_send_barrier` fires, which is when XPC has written everything sent before it. Tap to frame is the request until the first display-surface frame whose hash differs from the settled screen's.
- Frames: S2's SimScreen callbacks. A worker hashes every presented frame below its top 7.5 percent, which leaves out the status bar clock the 32-pixel hash would otherwise see tick, and keeps the frames in a history so a late caller still finds the first changed one. Before each tap the screen must go 150 ms without a present. Typing uses a 512-pixel thumbnail, because one glyph does not move the 32-pixel one.

## Measurements

Timings are wall-clock milliseconds from `Instant`, p50 and p95 by nearest rank. Load is the 1, 5 and 15 minute averages when the step started. Unless stated, the numbers are from the final run, 01:14 to 01:34 on 2026-09-27 at commit `50bbac0`.

### Tests, red then green

- `02ed5e9` added the tests with the event builders, the activation gate and the key map stubbed: 24 of 54 failed on `not implemented` (10 event and scaling tests, 7 gate tests, 7 key map tests); the 30 passing ones covered the libxpc value wrapper, the pipeline and the loader.
- `0fc98fa` made all of them pass and added the CoreSimulator version gate (1155.10 sorts above 1155.4). The suite now has 61 tests.

### Activation

| | Lookup | Connect | Probe reply | Tail | Total |
|---|---|---|---|---|---|
| Warm daemon, 20 sessions in a row (load 9.68 17.14 28.17) | 1.77 / 2.20 | 0.01 / 0.01 | 8.61 / 9.96 | 200 | 214.40 / 215.73 |
| First session of a boot, `mobile-agent-spike` (load 5.20) | 2.55 | 0.01 | 333.48 | 200 | 539.52 |
| First session after its reboot (load above 160) | 1.38 | 0.01 | 296.63 | 200 | 507.53 |
| First session of boot 1 of a new device, early tap sent ahead of the probe (load 8.84) | 2.88 | 0.01 | 285.28 | 200 | 489.91 |
| First session of its boot 2, early tap held by the gate (load 7.36) | 1.56 | 0.01 | 267.80 | 200 | 472.28 |
| First session of its boot 3, early tap held, no tail (load 9.53) | 0.84 | 0.01 | 267.15 | 0 | 268.02 |

Warm rows are p50 / p95; the probe reply ranged from 6.41 to 10.02 ms. The first session of a boot pays for launchd starting dtuhidd: 267 to 333 ms for the reply over five boots. Every session answered on its first attempt. Once started, dtuhidd stays up: an hour after its last peer left, its job still read `runs = 1`, despite `EnablePressuredExit`, and only a reboot reset it to `runs = 0`. So the cold activation is paid once per boot.

What activation does in the guest, from dtuhidd's and backboardd's logs: on each new peer connection dtuhidd creates four virtual HID services, `mainTouchscreen` (0x101), `mainKeyboard` (0x200), `mainScreenButtons` (0x402) and `avpCustom` (0x500), and opens their devices within about 30 ms, logging "Device opened, dispatched 0 pending events"; backboardd logs "digitizer attached" and "acquired main display digitizer". On disconnect dtuhidd resets each service's gesture state and cancels all four. Right after the cancel, on each of the three disconnects logged, one more touchscreen event, most likely that reset, is logged "No active service, dropping event"; both events of each logged tap had been logged "Handling IndigoDigitizerEvent" before it, so no input was lost to it.

### Input before activation

`activate --early-tap 120.6 812` submits a zero-hold tap on the List tab right after the probe goes out, with the app on its Login tab. Warm daemon, 5 runs each (load 6.40 to 9.38):

| Early tap | Landed | Probe reply | First changed frame after the tap was submitted | After the flush |
|---|---|---|---|---|
| Sent ahead of the probe, no gate | 5 of 5 | 8.45 to 19.07 | 109 to 135 | |
| Held by the gate, no tail | 5 of 5 | 6.75 to 10.18 | 114 to 129 | 105 to 118 |
| Held by the gate, 200 ms tail | 5 of 5 | 8.18 to 11.12 | 259 to 277 | 47 to 61 |
| Cold daemon, sent ahead of the probe, no gate (boot 1) | 1 of 1 | 285.28 | 334 | |
| Cold daemon, held by the gate, 200 ms tail (boot 2) | 1 of 1 | 267.80 | 527 | 56 |
| Cold daemon, held by the gate, no tail (boot 3) | 1 of 1 | 267.15 | 324 | 57 |

Nothing sent before activation was dropped, warm or cold, with or without the gate: 18 of 18 early taps landed. On a cold daemon the messages wait in the connection while launchd starts dtuhidd, and are handled once it takes the peer. A tap on a connection a few milliseconds old takes about 110 ms to show, against about 56 ms once the connection is a few hundred milliseconds old and 36 ms in a settled session: the new peer's services are still being created and opened, and dtuhidd holds events for a device that is not open yet. The tail therefore does not rescue the first action, it delays it: warm, the tap shows about 265 ms after it was requested instead of about 115; cold, 527 ms instead of 324.

### Taps

200 taps alternating between the List and Login tab bar items, each replacing the whole screen, in one session:

| | Hold 40 ms | No hold |
|---|---|---|
| Load at start | 8.92 16.48 27.68 | 6.33 14.74 26.46 (38.88 at the end) |
| Taps that changed the frame | 200 of 200 | 200 of 200 |
| Send completion, p50 / p95 / max | 0.03 / 0.05 / 0.06 | 0.03 / 0.16 / 9.70 |
| Send barrier, p50 / p95 / max | 0.07 / 0.10 / 1.12 | 0.08 / 4.22 / 31.44 |
| Whole gesture, p50 / p95 | 43.59 / 45.12 | 0.11 / 7.66 |
| Tap to first changed frame, p50 / p95 / max | 73.68 / 97.37 / 109.19 | 36.22 / 61.72 / 110.63 |

The no-hold run met a load spike from another spike's Simulator boot, the likely source of its send barrier tail and the 9.70 ms send. An earlier pair of runs (23:40, load 4.27 to 6.71, before the frame-history fix, which does not affect a caller already waiting) agrees: send completion p50 0.03 and 0.04 ms, tap to frame p50 74.08 / p95 98.27 ms with the hold and 37.47 / 61.03 ms without. React Native fires `onPress` on the touch-up, so the hold is in the tap-to-frame figure: taking the 40 ms off leaves about 34 ms, close to the no-hold 36, which is the app, its render, the next 60 Hz present and the read.

### Typing and keys

| | Keys | Send, p50 / p95 | First changed frame, p50 / p95 / max |
|---|---|---|---|
| `hunter2` into the password field (load 6.82) | 7 | 0.06 / 0.08 | 18.00 / 48.34 / 48.34 |
| `abcdefghijklmnopqrstuvwxyz0123456789` into the username field (load 6.52) | 36 | 0.07 / 0.21 | 17.34 / 27.40 / 32.21 |

Send is down, up and the send barrier. The benchmark taps the field, waits for its first frame, sleeps 1.2 s for the keyboard's slide-in and waits for 400 ms without a present, 1.8 to 1.9 s in all; without that wait the first keys are credited with the keyboard's own frames (699 ms in an early run). Every character arrived in order: after `hunter2` and Sign in the fixture read "Welcome, " (the username was empty), and the 36 characters appeared exactly. `key delete` three times removed "789", and `key return` dismissed the keyboard.

### Swipes, pinch, long press, buttons

- Swipe with velocity: 450 points up the List screen in 20 steps. Over 967 ms (465 points/s) the list came to rest with row 15 at the top; over 239 ms (1,879 points/s), row 30; over 132 ms (3,410 points/s), row 52. The touch-down was handed to XPC 0.03 ms after the request.
- Two-finger pinch: both contacts in one `IndigoDigitizerEvent` with `pointOne` and `pointTwo`. In Photos' library grid, a pinch out from a 40 to a 170 point radius over 435 ms went from three columns to one, and a pinch in from 170 to 30 went to five.
- Long press: 900 ms on the Contacts icon opened its context menu.
- Buttons: `IndigoButtonEvent` home (0x0C/0x40) went to the Home Screen and lock (0x0C/0x30) to the Lock Screen, each sent in 0.04 to 0.10 ms.
- Edge flag: a bottom-edge swipe (edge 3, about 450 points in about 200 ms) dismissed the Lock Screen, three times.

### Indigo

`measure.sh indigo` created a new `mobile-agent-s3-indigo`, booted it, installed and launched the fixture, and read dtuhidd's launchd job and the `com.apple.coredevice.dtuhidd.active` notify flag in the guest between steps. Every tap had to change pixels if it was delivered: the fixture was restarted on its Login tab before each process's first tap, which went to List, and within a process each tap went to whichever tab the last verified tap had left unselected. Load 7.4 to 11.5.

| Step | dtuhidd before the step | Indigo taps delivered | Tap to first changed frame |
|---|---|---|---|
| Boot 1, before anything opened dtuhidd, three processes | not running, `runs = 0`, flag 0 | 3 of 3 | 87.91, 111.87, 97.50 |
| Boot 1, after the first dtuhidd session had closed | running, `runs = 1`, flag 1 | 1 of 1 | 101.33 |
| Boot 1, a dtuhidd session open in the same process, alternating with dtuhidd taps | flag 1 | 3 of 3 | 76.44, 65.64, 82.04 |
| Boot 2, after a reboot | not running, `runs = 0`, flag 0 | 1 of 1 | 86.10 |

The dtuhidd taps between them, in the same session: 3 of 3 delivered, 62.79, 96.20 and 72.71 ms. With the 40 ms hold, Indigo's tap to frame (p50 86.10 ms, p95 111.87 ms over 8) sits beside dtuhidd's (73.68 / 97.37 over 200).

- Client: `_TtC12SimulatorKit24SimDeviceLegacyHIDClient` is registered once SimulatorKit is loaded, `-initWithDevice:error:` returns a client, and every `-sendWithMessage:freeWhenDone:completionQueue:completion:` completed without an error.
- Messages: `IndigoHIDMessageForMouseNSEvent` with the six-parameter Xcode 27 prototype (target 0x32, `NSEventType` 1 and 2 for down and up, `NSSize {1, 1}`, edge 0) returned a message every time. It was re-enveloped as idb's single-touch message, 0x140 bytes with event type 0x02 and the payload duplicated, with the ratios written over the builder's.
- Cost: a tap's two sends and their completions took 0.5 to 5.3 ms beyond the 40 ms hold.
- The legacy client does not start dtuhidd: after three Indigo taps its job still read `runs = 0`.
- Neither transport locked the other out. Indigo sent first on boots 1 and 2, and dtuhidd still took every tap it was given, cold or warm.
- An earlier session on the first copy of this device agrees: 17 of 17 correctly aimed Indigo taps were delivered, 16 of them after dtuhidd had been activated. Its first version tapped one point three times, so the second and third taps hit the tab the first had selected and looked like misses; that version also ran while other agents had the load above 200. The protocol above is the fix.

Not tested: Indigo keys and buttons (the note reports keys dropped and buttons working from 1155.4), the builder's 16 ms window for moves, and hosts below 1155.4, which this host does not have.

### An unexplained freeze

On the boot `mobile-agent-spike` had when S3 started, after a lock-button press, a bottom-edge unlock, a fixture launch and a two-finger pinch, each from its own short-lived process, the display stopped presenting: screenshots returned a frame minutes old, S3's and S2's frame streams saw no callbacks, and taps were logged by dtuhidd but changed nothing. The lock button still woke the device. A reboot cleared it. None of the steps reproduced it on its own afterwards: lock and a bottom-edge unlock with an 80 ms and a 2 s drain, then a Home Screen pinch with the 80 ms drain, each left the device responsive, 4 of 4 taps landing. The device also stayed awake through more than an hour of idle, so auto-lock is not the cause. The same-tab artefact described under Indigo is ruled out: the taps went to tabs that were not showing, and screenshots confirmed nothing changed.

## The 20 ms gate

The spec's requirement is that on a warm session a tap is dispatched within 20 ms of the request, and its scenario says delivered to the Simulator within 20 ms.

- Dispatch passes by three orders of magnitude: the touch-down reaches XPC 0.03 ms after the request at the median and 0.05 to 0.16 ms at p95 over 400 taps, 9.70 ms at worst under a load spike. XPC has written it 0.07 to 0.08 ms after the request at the median.
- Delivery to the Simulator was not timed on its own. dtuhidd logs each event on arrival, 45 ms apart for a 40 ms hold, and a warm tap changes the screen 36 ms after the request at the median, so the daemon's own share cannot be large.
- A visible change cannot meet 20 ms for this app on any transport: the first changed frame follows a zero-hold tap by 36 ms at the median and 62 ms at p95, almost all of it React Native's handling and render and the wait for the next 60 Hz present. Keys, whose effect is a single glyph, show in 17 to 18 ms at the median.
- The first action of a session is outside the gate and costs the activation: about 9 ms for a warm daemon or 267 to 333 ms for the first session of a boot, plus the new connection's 50 to 110 ms before its first tap shows.

The gate should stay a dispatch gate, the driver should report tap-to-frame alongside it, and the "Input before activation" scenario should report the activation wait, as it does.

## Verdict

**Go for the dtuhidd XPC transport, as the default from CoreSimulator 1155.4.**
- Every piece of the connection works from Rust with hand-written libxpc bindings, three `dlsym` lookups and one `msg_send!`.
- One connection on the digitizer service carries touches with one or two contacts and the edge flag, keyboard usages and Consumer-page buttons. The other dtuhidd Mach services were not needed.
- Warm tap dispatch is 0.03 ms at the median. 400 of 400 benchmark taps landed, as did every functional check outside the one freeze described above.
- Activation costs about 9 ms warm and 267 to 333 ms for the first session of a boot. Input sent before the reply was never lost, warm or cold.

**Go for the legacy Indigo client as the fallback for touch, and nothing on this host needs it.**
- The Xcode 27 six-parameter `IndigoHIDMessageForMouseNSEvent` with a unit `NSSize`, re-enveloped as idb's single-touch message and sent through `SimDeviceLegacyHIDClient`, delivered taps on a fresh boot before dtuhidd had ever run.
- It went on delivering them after dtuhidd was activated, interleaved with dtuhidd taps.
- Keys and buttons over Indigo, the 16 ms builder window for moves, and hosts below 1155.4 were not tested; this host has no older CoreSimulator.

What this means for the gated work:
- 6.4 can port `xpc.rs`, `dtuhid.rs`, `events.rs`, `gate.rs` and `keymap.rs` as they are, and the gestures in `input.rs`. The session should be held open for its target's lifetime, which pays the activation once, and should flush held input as soon as the reply arrives: the 200 ms tail delayed the first action on both warm and cold daemons and never saved one.
- 6.3 can run its tests on this host, because Indigo touch works here, taking `indigo.rs`'s builder binding and single-touch envelope. Its tests must aim every tap at something that will change, or a tap on the tab already showing reads as a miss.
- 6.5 keys the choice on the CoreSimulator that vends `SimDevice`, as `device.rs` does; this host has only one, so the two-runtime scenario gets one transport.
- 6.7: typing through dtuhidd was exact at a 60 ms gap between characters, and the fastest safe gap was not measured. At 60 ms a 500-character string takes about 30 s, which is the case for paste.
- The driver has to wake a device whose display is not presenting (lock button, then a bottom-edge swipe) and should detect that state from the frame callbacks going silent.

## Proposed replacement for the input sentences of design.md D6

This spike does not edit design.md. Proposed text, replacing the sentences from "The input transport is selected" to "all HID calls run on one serial queue":

> The input transport is selected from the host's CoreSimulator version, as idb and iosef do. From 1155.4 it is the dtuhidd XPC service, `com.apple.coredevice.feature.remote.hid.digitizer`, reached through `-[SimDevice lookup:error:]` and hand-written bindings to the `_4sim` libxpc functions resolved with dlsym; its one connection carries touches with one or two contacts and the edge flag that system gestures need, keyboard usages and Consumer-page buttons. Before 1155.4 it is the Indigo Mach messages through SimulatorKit's legacy HID client. The session sends dtuhidd's activation probe, holds input until the reply, flushes it at once and keeps the one connection for the target's lifetime. Spike S3 (`docs/spikes/s3-simulator-hid.md`) measured this on Xcode 27 with CoreSimulator 1171.7 and iOS 26.5: a tap's touch-down reaches XPC 0.03 ms after the request at the median and 0.05 to 0.16 ms at p95, against the 20 ms dispatch gate, and the first changed frame follows 36 ms after a zero-hold tap, almost all of it the app's own rendering; activation takes about 9 ms once dtuhidd is running and 270 to 330 ms for the first session of a boot, while launchd starts the daemon; input submitted before the reply was never lost, warm or cold, and it is flushed as soon as the reply arrives, because a fixed wait after the reply only delayed it. The driver does not probe the legacy transport, because other sources report that activating dtuhidd disables it, although on this host Indigo touch kept working afterwards. The Indigo message builders keep unsynchronized global state, so Indigo calls run on one serial queue.

## What the research note got wrong or left open

Proposed corrections and additions to `docs/research/simulator.md`. This spike does not edit that note.

1. The verdict and the "Input transports by Xcode" table call Indigo touch a boot race that is dead once dtuhidd is active on 1155.4 and later with iOS 26.x guests, and D6 forbids probing because "opening dtuhidd disables the Indigo services for the rest of the boot". On this host, iOS 26.5 on CoreSimulator 1171.7, Indigo touch worked on a fresh boot and kept working after dtuhidd was activated and `com.apple.coredevice.dtuhidd.active` read 1, interleaved with dtuhidd taps, with a dtuhidd session open in the same process: 8 of 8 Indigo taps in the final protocol, 4 of them with dtuhidd active, and 17 of 17 in an earlier session. Neither did idb's "both claim `mainTouchscreen` and whichever sends first on a boot keeps it": Indigo sent first on two boots and dtuhidd took every tap afterwards. So the S3 risk that the Indigo half is order-dependent did not materialize either. The sources' observations may hold on other guests, on iOS 27, or with DeviceHub.app attached; they did not hold here.
2. "Events sent before activation are dropped [axe#71]" did not hold on this host: 18 of 18 early taps landed, 15 with a warm daemon and the first tap of each of three fresh boots, sent ahead of the probe or held until its reply. AXe's report is from an iOS 27.0 guest, which this host cannot run.
3. The 200 ms reply tail, which the note carries over from idb and iosef, delays the first action instead of protecting it: an early tap showed about 265 ms after the request with the tail and about 115 without it on a warm daemon, and 527 against 324 ms on a cold one.
4. "No source publishes a warm dtuhidd tap latency" and "no source reports a home-indicator swipe through [the edge field]" are now answered: 0.03 ms dispatch, and a bottom-edge swipe dismissed the Lock Screen.
5. The Mach service list suggests separate button and keyboard services. One digitizer connection carries keyboard and button events too, and dtuhidd instantiates `mainTouchscreen`, `mainKeyboard`, `mainScreenButtons` and `avpCustom` for each peer connection.
6. The note describes dtuhidd as demand-launched with a 10 s minimum runtime. After its first launch it stayed up for more than an hour with no peer, so the cold activation is paid once per boot, not once per idle period.
7. The note does not give `-[SimDevice lookup:error:]`'s return type. It is `mach_port_t`, and a plain `msg_send!` with a `u32` return and an `NSError **` works, with the debug build's signature check.
8. Missing risk: a device whose display stops presenting. Frame callbacks go silent while dtuhidd still accepts input, and the input changes nothing; a reboot fixed the one occurrence, and the lock button wakes a locked device.

## Addendum: the pasteboard (task 10.31)

Task 10.31 measured the ways to put a paste's text on the Simulator's pasteboard, on `mobile-agent-spike` (iPhone 17, iOS 26.5) under Xcode 27.0 with CoreSimulator 1171.7, on 2026-10-07. A throwaway harness, not committed, opened each fixture on its Login tab through the driver, focused `login-username`, and ran 20 trials per way and fixture with the ways' order rotated each trial: empty the field (Command-A, delete), then set a 36-character text unique to the trial, send Command-V over dtuhidd, and read the tree until the field showed the text, for at most 3 s. "Set" is the call that put the text on the pasteboard, "Command-V to shown" ends with the read that first showed it. The fresh-boot `pbcopy` series ran alone, right after a reboot, before anything had started `dtpasteboardd`, the 1-minute load at 45.02 when it began; the rest ran from 12:47:05 to 12:51:04 UTC at a 1-minute load of 3.98 to 24.23, median 16.28, raised by another session's builds.

| Way | Fixture | Set: min / p50 / p95 / max (ms) | Command-V to shown, p50 / p95 | Set to shown, p50 / p95 | Shown |
|---|---|---|---|---|---|
| `devicectl device pasteboard copy`, a process per paste | SwiftUI | 118.1 / 167.4 / 233.9 / 240.7 | 81.9 / 100.8 | 253.3 / 321.0 | 20 of 20 |
| | Expo | 124.7 / 200.7 / 247.0 / 249.7 | 76.3 / 93.6 | 277.1 / 327.2 | 20 of 20 |
| `xcrun simctl pbcopy`, `dtpasteboardd` running | SwiftUI | 312.4 / 366.1 / 406.9 / 425.8 | | | 0 of 20 |
| | Expo | 310.0 / 353.6 / 404.0 / 423.9 | | | 0 of 20 |
| `xcrun simctl pbcopy`, a fresh boot | SwiftUI | 298.8 / 348.4 / 398.5 / 468.5 | 78.9 / 101.7 | 426.1 / 549.5 | 20 of 20 |
| `SET` to `dtpasteboardd`, a connection per paste | SwiftUI | 8.3 / 16.6 / 32.6 / 34.5 | 82.5 / 93.5 | 103.5 / 128.1 | 20 of 20 |
| | Expo | 10.2 / 15.7 / 34.3 / 35.1 | 76.1 / 96.9 | 89.6 / 119.6 | 20 of 20 |
| `SET` to `dtpasteboardd`, one connection kept | SwiftUI | 13.7 / 16.5 / 31.7 / 32.2 | 89.4 / 115.7 | 107.0 / 137.8 | 20 of 20 |
| | Expo | 7.5 / 14.1 / 21.4 / 29.4 | 78.5 / 102.3 | 92.1 / 114.7 | 20 of 20 |

- **`pbcopy` once `dtpasteboardd` runs.** Every Command-V pasted the previous trial's text: `pbcopy` exits 0, and `CoreSimulatorBridge`, its guest end, logs "Ignoring snapshot for com.apple.UIKit.pboard.general: the CoreDevice pasteboard stack is in use" (`docs/research/simulator.md`, The pasteboard). The host pasteboard's sync through Simulator.app or DeviceHub.app was not measured: it needs one of them attached to the device, DeviceHub.app attaches to every booted Simulator, other sessions' included, and the sync writes the host's own pasteboard.
- **The `SET`.** It is `devicectl`'s own message, sent from the measuring process: `-[SimDevice lookup:error:]` for `com.apple.coredevice.feature.pasteboard` took 1.6 ms, and in a probe's eight `SET`s the reply came 3.0 to 7.6 ms after the send, at a load of 4 to 6. Keeping the connection saved nothing measurable, so the driver opens one per paste and keeps no state. No Command-V after a `SET_REPLY` pasted older text, in 80 trials.
- **What a paste needs besides.** Without `kTCCServicePasteboard` the fixture showed "“FixtureSwiftUI” would like to paste from “dtpasteboardd”" with Don't Allow Paste and Allow Paste, and the field stayed empty; with it, the text arrived. That is what the driver's grant is for. `-[SimDevice installedAppsWithError:]`, which finds the applications to grant, took 10.8 / 12.5 / 21.8 ms (min / p50 / max) over 20 calls for the device's 39 applications, and a grant 2.9 / 6.8 / 20.5 ms over 15. A reinstall gave the application a new bundle directory and changed the modification time of `Containers/Bundle/Application`, and kept its grant.
