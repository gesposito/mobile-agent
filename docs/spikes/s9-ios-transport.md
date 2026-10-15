# S9: iOS transport and lifecycle

Spike S9 (task 2.1 of `physical-ios`) measures whether the host reaches a physical iPhone's developer services from Rust without root, through the `idevice` crate, and what launching, killing and screenshotting an app cost that way, against `xcrun devicectl` for the same operations. It decides design D18 (transport) and D19 (lifecycle). The research note `docs/research/ios-device.md` named the services, the risks and the measurements, in its Spike plan with additions. The code is `spikes/s9-ios-transport/`, kept privately since the repository was made public (`docs/spikes/README.md`), and every table in Measurements is generated from its `results/` by `s9-ios-transport report`. Snapshot date: 2026-10-08.

**In brief.** Go. From an unprivileged process, `idevice` 0.1.68 opens the CoreDevice tunnel without root or a TUN device. With RSD's handshake it takes 59.9 ms at p50 and 70.0 ms at p95. Through the tunnel, DVT launches Settings in 42.5 and 46.4 ms and takes a screenshot in 42.3 and 43.5 ms, against D18's gate of 2 s and 500 ms. CoreDevice's app service and screen capture cost the same within noise. `devicectl` takes 228 to 370 ms at p50 for each of these operations, a process each, about 250 ms of which pass before its request reaches the device. The iPhone lists all four CoreDevice services the frame and input candidates need, and has the personalized developer disk image mounted. Four findings on iOS 26.6.2 change D19 and the driver:

- `idevice`'s DVT `kill_app` does not kill, and drops every DVT connection; `killPid:` with the pid archived, or `sendSignal:toPid:`, does kill.
- CoreDevice's `getlockstate` is not implemented, but lockdown's `PasswordProtected` follows the lock.
- CoreDevice's screen capture refuses JPEG.
- DVT's app-state notifications carry no foreground state.

D18 stands with its numbers; D19 now terminates through `sendSignal:toPid:` and reads the lock before a launch.

## Setup

**Host.** Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2 (25G83). Other work used it throughout: an Android emulator before the measured run, and Xcode builds and Java processes during it (see Conditions).

**Apple tools.** Xcode 27.0 (27A266a); CoreDevice and `devicectl` 642.16; the iOS developer disk image (DDI) of Xcode 27.0 (27A266a, XCTest 25228), which is the one the device has mounted.

**Device.** iPhone SE (3rd generation), `iPhone14,6` (`D49AP`), iOS 26.6.2 (23G90), on USB, paired with this Mac and trusted, with Developer Mode on. Before the measured series, the maintainer set Auto-Lock to Never and left the phone unlocked. Every series checked the lock before starting, and none found it locked. The phone was on USB power and charged from 96% to 100% over the series. The locked-phone checks below ran earlier, before the phone was unlocked. Its identifier stays in `MOBILE_AGENT_IOS_UDID`; the code reads it from there, and no result or line of this report names it. usbmuxd lists the phone twice, on USB and on the network (Wi-Fi sync is on); the spike takes the USB entry.

**Crates.** `idevice` 0.1.68, pinned with `=` as its README asks, with the features `usbmuxd`, `core_device_proxy`, `tunnel_tcp_stack`, `rsd`, `xpc`, `mobile_image_mounter`, `dvt`, `device_info`, `core_device` and `cryptexd`, and its default TLS (rustls 0.23.45 on aws-lc-rs 1.18.1); jktcp 0.1.7 through it; tokio 1.53.2; plist 1.10.1. The lock file holds 135 crates, which came in through `sfw cargo add` and `sfw cargo fetch`. Rust 1.98.1, release profile.

**Code.** The measurements ran on commit `063e36b`. Commits `efe93dd` and `de8a42b` change only the table generator, and `ffc080f` adds the results.

**Privileges.** One unprivileged process, and no `sudo` anywhere. The probe and the tunnel series record that the effective user is not root, and that the host's `utun` interfaces are the same before the tunnel opens and while it is open: jktcp carries the tunnel inside the process, with no TUN device.

**App.** Settings (`com.apple.Preferences`), a system app on every iPhone that holds no user content a kill could lose. Nothing was installed on the device. Task 2.1 asks for the SwiftUI fixture, but installing it needs a signing identity, and this host has none yet (see What was not measured).

**Method.** Every series first waits for a quiet host: a 1-minute load under 4, and no cargo, rustc, gradle, xcodebuild, kotlinc or java process above 5% of a core, checked every 30 s within a 90-minute budget. The launch and screenshot series could fall back to a load under 6 once that budget was spent; none needed it, since every series started under 4. Next, the series reads the lock state through lockdown and `devicectl`. A series that needs the device unlocked waits for an unlock instead of failing: it prints `S9-LOCKED`, polls every 15 s for up to 10 minutes, and starts over if the device locks during it. Then it discards its warm-up runs (2 or 3) and times 20 runs, and writes every sample, its summaries and the load before and after it to `results/<series>.json`.

- **Lifecycle series** launch Settings and kill it once per run. Launch is timed to the reply that carries the pid, to the pid's `Running` app-state notification, and to the pid's first appearance in DVT's process list. Kill is timed to its reply (or to the write, for a call without one), to the pid's disappearance from that list, and to its `Terminated` notification. DVT observes every path through two connections of its own: `isRunningPid:` polled back to back, and the mobile notifications channel, read on a task that stamps each message on arrival. The app gets 1.5 s to finish launching before the kill, and 1 s after the kill before the next launch.
- **Screenshot series** first put Settings on screen, so every path captures the same picture. Each also times its first capture on a new connection. The pictures stay in a scratch directory and are not committed; the results keep their sizes and dimensions.
- **`devicectl` series** time the process from spawn to exit, `xcrun`'s start included, with `--json-output` to the scratch directory and `-q`.

**Rerun**, with the device on USB, unlocked and trusted, and `MOBILE_AGENT_IOS_UDID` exported:

```sh
cd spikes/s9-ios-transport            # in the private copy
cargo test                          # 10 unit tests: percentiles, image headers, the host guard
cargo build --release
./target/release/s9-ios-transport probe          # results/probe.json: services, disk image, values, capture formats
./target/release/s9-ios-transport launch-check   # results/launch-check-{locked,unlocked}.json
./target/release/s9-ios-transport kill-check     # results/kill-check.json: the three DVT kills
./target/release/s9-ios-transport all            # every series behind the guard, then results/tables.md
./target/release/s9-ios-transport report         # regenerate results/tables.md
```

Each series also runs alone (`listing`, `lockdown`, `tunnel`, `dvt-lifecycle`, `dvt-lifecycle-killpid`, `appservice-lifecycle`, `screenshots`, `devicectl-lockstate`, `devicectl-lifecycle`, `devicectl-screenshot`). `S9_RESULTS` and `S9_SCRATCH` move the results and the scratch directory. `S9_DRY_RUN=1` runs each series three times without the guard or the lock check, to try the code paths; its numbers are not reported.

### Conditions

The guard made 102 checks between 20:01:59 and 20:46:52, one row each in `results/hygiene.tsv`; the full blocks are in the git-ignored `results/hygiene.log`. 80 checks found the host busy. At 50 of them a watched process was above 5% of a core: a concurrent `xcodebuild` at 42, a `java` process at 19, both at 11. The other 30 held the series back on the load alone. The 1-minute load at those checks ranged from 2.3 to 41.7, and no emulator process was above 5% at any check. Every series started at a 1-minute load under 4, so none used the fallback, and the 40 minutes of waiting stayed inside the 90-minute budget. Nothing was measured on a busy host. Earlier runs of the program, with `S9_DRY_RUN=1` to try its code paths, went to a scratch directory and are not reported. The load rose during three series, from other builds on the host: to 5.04 during the DVT lifecycle with `sendSignal:toPid:`, to 6.48 during the app service's, and to 8.59 during `devicectl`'s screenshots.

| Series | Waits | Ran | 1-minute load before | 1-minute load after |
|---|---:|---|---:|---:|
| usbmuxd-listing | 13 | 20:08:31 | 3.31 | 3.31 |
| lockdown | 2 | 20:09:33 to 20:09:36 | 3.83 | 4.00 |
| tunnel | 9 | 20:14:08 to 20:14:17 | 3.90 | 4.22 |
| dvt-lifecycle | 10 | 20:19:19 to 20:20:16 | 3.69 | 5.04 |
| dvt-lifecycle-killpid | 6 | 20:23:18 to 20:24:15 | 2.17 | 3.14 |
| appservice-lifecycle | 2 | 20:25:16 to 20:26:14 | 3.06 | 6.48 |
| dvt-screenshot | 9 | 20:30:45 to 20:30:49 | 3.43 | 3.31 |
| coredevice-screenshot-png | 1 | 20:31:20 to 20:31:21 | 3.67 | 3.67 |
| devicectl-lockstate | 27 | 20:44:54 to 20:45:00 | 3.27 | 3.65 |
| devicectl-lifecycle | 0 | 20:45:00 to 20:46:10 | 3.43 | 2.30 |
| devicectl-screenshot | 1 | 20:46:41 to 20:46:52 | 2.45 | 8.59 |

## Measurements

All times are milliseconds, p50 / p95 over 20 runs, nearest rank, warm-up runs excluded.

### Transport

| Step | n | p50 / p95 | 1-minute load before / after |
|---|---:|---:|---|
| usbmuxd `ListDevices`, new connection | 20 | 0.40 / 0.93 | 3.31 / 3.31 |
| lockdown: usbmuxd `Connect` to 62078 | 20 | 1.50 / 4.31 | 3.83 / 4.00 |
| lockdown: `ReadPairRecord` from usbmuxd | 20 | 1.45 / 3.29 | 3.83 / 4.00 |
| lockdown: `StartSession` with TLS | 20 | 17.80 / 24.90 | 3.83 / 4.00 |
| lockdown: one `GetValue` | 20 | 0.84 / 2.67 | 3.83 / 4.00 |
| lockdown: total | 20 | 22.13 / 30.73 | 3.83 / 4.00 |
| tunnel: `CoreDeviceProxy` (lockdown, `StartService`, CDTunnel handshake) | 20 | 42.21 / 52.28 | 3.90 / 4.22 |
| tunnel: jktcp adapter | 20 | 0.05 / 0.08 | 3.90 / 4.22 |
| tunnel: TCP to the RSD port through jktcp | 20 | 0.92 / 1.54 | 3.90 / 4.22 |
| tunnel: RSD handshake (RemoteXPC, service list) | 20 | 15.86 / 20.67 | 3.90 / 4.22 |
| tunnel and RSD, total | 20 | 59.90 / 69.98 | 3.90 / 4.22 |
| first DVT connection through the tunnel | 20 | 36.95 / 38.50 | 3.90 / 4.22 |
| `devicectl device info lockState`, process | 20 | 228.44 / 243.37 | 3.27 / 3.65 |

- **Lockdown.** A session costs 22 ms, four fifths of it in `StartSession`, which includes the TLS handshake. The pairing record comes from usbmuxd (`ReadPairRecord`) in 1.5 ms, without reading `/var/db/lockdown`.
- **The tunnel.** It costs 60 ms with RSD's handshake. Most of that is `CoreDeviceProxy`, 42 ms, which is itself a lockdown session, a `StartService` and the CDTunnel handshake. jktcp's adapter costs nothing measurable, and the TCP connection to the RSD port about 1 ms. The handshake's 16 ms bring the list of 82 services.
- **Service connections.** The first DVT connection after the tunnel opens costs 37 ms: a TCP connection through jktcp, then the wait for DVT's capabilities message. Later ones cost less. The lifecycle series opened four or five service connections on an open tunnel in 121 to 126 ms, 25 to 30 ms each. Either way the driver opens its connections once per target.
- **`devicectl`.** Its `lockState` takes 228 ms, ten times a lockdown session that reads `PasswordProtected`.

### What the device lists

The RSD handshake lists 82 services on iOS 26.6.2: 41 lockdown shims (`*.shim.remote`), 35 RemoteXPC services and 6 others (`results/probe.json` has every name with its entitlement, features and version). The four services the research note asked about are all listed: `com.apple.coredevice.displayservice`, `com.apple.coredevice.screencaptureservice`, `com.apple.coredevice.hid.indigo` and `com.apple.coredevice.hid.universalhidservice`. A fifth HID service, `com.apple.coredevice.hid.universalhid`, carries the `remote.hid.keyboard` feature. Around the test manager and DVT:

| Service | Entitlement | Features |
|---|---|---|
| `com.apple.dt.testmanagerd.remote` | `com.apple.private.dt.testmanagerd.client` | |
| `com.apple.dt.testmanagerd.remote.automation` | `AppleInternal` | |
| `com.apple.instruments.dtservicehub` | `com.apple.private.dt.instruments.dtservicehub.client` | `com.apple.dt.profile` |
| `com.apple.coredevice.appservice` | `com.apple.private.CoreDevice.canInstallCustomerContent` | `launchapplication`, `sendsignaltoprocess`, `listprocesses`, `monitorprocesstermination`, `installapp`, `uninstallapp` and 9 more |
| `com.apple.coredevice.screencaptureservice` | `com.apple.private.CoreDevice.canViewDeviceDisplay` | `capturescreenshot` |
| `com.apple.coredevice.displayservice` | `com.apple.private.CoreDevice.canViewDeviceDisplay` | `startmediastream`, `stopmediastream`, `startvideooutput`, `startaudiooutput`, `getmediasupportinfo`, `getmediastreamserverstatus` |
| `com.apple.coredevice.hid.indigo` | `com.apple.private.CoreDevice.hid` | `remote.hid.button`, `.scroll`, `.digitizer`, `.vendordefined` |
| `com.apple.coredevice.hid.universalhidservice` | `com.apple.private.CoreDevice.hid` | `remote.universalhidservice` |
| `com.apple.coredevice.deviceinfo` | `com.apple.private.CoreDevice.canRetrieveDeviceInfo` | `getlockstate`, `getdeviceinfo`, `getdisplayinfo`, `querymobilegestalt` and 3 more |
| `com.apple.security.cryptexd.remote` | `com.apple.private.security.cryptexd.remote` | `CryptexInstall`, `Cryptex1`, `ReadIdentifiers`, `Cryptex1,UseProductClass` |
| `com.apple.mobile.mobile_image_mounter.shim.remote`, `com.apple.mobile.installation_proxy.shim.remote` | `com.apple.mobile.lockdown.remote.trusted` | |
| `com.apple.accessibility.axAuditDaemon.remoteserver.shim.remote` | `com.apple.mobile.lockdown.remote.trusted` | |
| `com.apple.accessibility.axAuditDaemon.remoteAXService` | `AppleInternal` | |

Every service the spike used answered this unprivileged host process: DVT, the app service, the screen capture service, the device information service, `cryptexd`, and the image mounter through lockdown.

### The developer disk image and Developer Mode

The DDI was already mounted when the spike started, since CoreDevice mounts it for Xcode and `devicectl`; the spike mounted nothing. The image mounter's `CopyDevices` (over lockdown) lists one image: `DiskImageType` `Personalized`, `PersonalizedImageType` `DeveloperDiskImage`, APFS, mounted at `/System/Developer`, and `LookupImage` finds its signature. `cryptexd`'s `copy-installed` lists no `com.apple.MobileAsset.DDI` cryptex. So on this iPhone the image is the personalized one through the image mounter, not the Cryptex1 form. A personalized image needs the device's chip and board in the DDI's build manifest, so the iPhone SE (3rd generation) is in Xcode 27.0's (an inference from the mount). What a mount needs (Apple's signing server for the personalization ticket, as the research note says) and how long it takes were therefore not measured. Developer Mode reads as on both through lockdown (`DeveloperModeStatus` in the `com.apple.security.mac.amfi` domain) and through the image mounter's `QueryDeveloperModeStatus`.

### Launch and terminate

p50 / p95 in ms, from the request (for devicectl, the process's spawn). Kills: DVT `sendSignal:toPid:` SIGKILL with its reply, DVT `killPid:` with the pid archived and no reply, the app service's `sendsignaltoprocess` SIGKILL, `devicectl device process terminate --kill`.

| | DVT, `sendSignal:toPid:` | DVT, `killPid:` | CoreDevice app service | devicectl |
|---|---:|---:|---:|---:|
| launch: reply (devicectl: process exit) | 42.54 / 46.41 | 42.55 / 44.04 | 41.34 / 44.48 | 294.71 / 351.14 |
| launch: `Running` notification for the pid | 40.16 / 44.50 | 39.74 / 41.54 | 39.43 / 42.67 | 284.21 / 339.75 |
| launch: pid in DVT's process list | 47.87 / 51.45 | 47.18 / 52.28 | 46.60 / 50.91 | 313.79 / 369.64 |
| kill: reply, or written when none (devicectl: process exit) | 2.93 / 5.03 | 0.12 / 0.49 | 3.42 / 8.94 | 272.37 / 319.74 |
| kill: pid gone from DVT's process list | 22.36 / 24.20 | 22.48 / 25.88 | 22.14 / 25.40 | 289.14 / 346.34 |
| kill: `Terminated` notification for the pid | 21.46 / 25.79 | 21.66 / 24.42 | 21.64 / 23.23 | 273.22 / 322.60 |
| runs (n) | 20 | 20 | 20 | 20 |
| 1-minute load before / after | 3.69 / 5.04 | 2.17 / 3.14 | 3.06 / 6.48 | 3.43 / 2.30 |

- **Launch.** The three paths through `idevice` launch alike. The reply comes in 41 to 43 ms at p50 and 44 to 46 ms at p95. The pid's `Running` notification arrives 2 to 3 ms before the reply, and the pid shows in DVT's process list one poll after it, about 5 ms later. Settings is a system app; the fixture, a third-party app, was not measured (see What was not measured). When the app is on screen is S11's question, through the first changed frame.
- **Kill.** The reply of `sendSignal:toPid:` and of the app service's signal comes in about 3 ms at p50. On every `idevice` path the process leaves the list 22 ms after the request, and its `Terminated` notification arrives at the same moment. `killPid:` returns once written, in 0.12 ms, and kills as fast. `sendSignal:toPid:` costs 3 ms more and confirms the signal, which is the form D19 takes. The app service's reply varied most, 3.4 ms at p50 against 8.9 at p95.
- **`devicectl`.** It takes 295 ms to launch and 272 ms to terminate at p50. The `Running` notification arrives 10 ms before its process exits, and `Terminated` 1 ms after. About 250 ms of each call therefore pass before its request reaches the device, in starting the process and connecting it to CoreDevice.
- **Observers.** Every run on every path delivered both notifications for its pid.

### Screenshots

| Path | n | p50 / p95 | max | First on a new connection | Image | 1-minute load before / after |
|---|---:|---:|---:|---:|---|---|
| DVT `takeScreenshot` | 20 | 42.33 / 43.54 | 43.82 | 114.48 | PNG 750x1334, 141 KB | 3.43 / 3.31 |
| CoreDevice `capturescreenshot`, PNG | 20 | 42.49 / 43.38 | 43.66 | 94.58 | PNG 750x1334, 141 KB | 3.67 / 3.67 |
| `devicectl device capture screenshot`, process | 20 | 369.62 / 393.21 | 403.86 | - | PNG 750x1334, 141 KB | 2.45 / 8.59 |

- **DVT and CoreDevice.** Both return the same 750×1334 PNG, the screen's native resolution, of 141 KB, in 42 ms at p50 and 43.5 ms at p95. The slowest of the 20 captures was within 3.5 ms of the fastest. A first capture on a new connection costs 114 ms through DVT, which also opens a connection and a channel, and 95 ms through CoreDevice.
- **As frames.** At 42 ms per capture, back-to-back captures would give about 24 a second. S11 compares that with the two frame streams.
- **`devicectl`.** It takes 370 ms at p50 and 393 ms at p95 for the same PNG, written to a file.

### The lock state

CoreDevice's device information service lists `getlockstate` among its features, but on iOS 26.6.2 it answers "Action 'com.apple.coredevice.feature.getlockstate' is not implemented", and "Action 'com.apple.coredevice.action.getlockstate' is not implemented" when asked with an action identifier, the form the screen capture service takes. Lockdown's `PasswordProtected`, read inside a session with the pairing record, follows the lock instead. It was `true` while the phone was locked and `false` once it was unlocked, as `devicectl device info lockState`'s `passcodeRequired` was at the same moments, with `unlockedSinceBoot` `true` throughout. Reading it costs one `GetValue` in a lockdown session (see Transport), against 228 ms for `devicectl`'s process.

Launches on the locked phone (`results/launch-check-locked.json`):

- DVT refused them. The reply is an archived `NSError` with code 2, which `idevice`'s `launch_app` reports only as "expected integer PID in launch app response"; the spike's own launch call reads the archive's top object, whose domain and description stay unresolved references.
- The app service refused with SpringBoard's reason: "The request was denied by service delegate (SBMainWorkspace) for reason: Locked ("Unable to launch com.apple.Preferences because the device was not, or could not be, unlocked")", in `FBSOpenApplicationErrorDomain`.
- On two occasions earlier in the session, while `devicectl` also said `passcodeRequired`, the app service launched Settings behind the lock screen, six times in all. The pid came back each time, and the notifications went `Running`, then `Terminated` when the spike killed it.

A launch on a locked phone therefore neither fails reliably nor says why in the same way on both paths. The driver reads `PasswordProtected` before it launches, as these series did. With the phone unlocked, both paths launched (`results/launch-check-unlocked.json`).

### What `idevice` 0.1.68 does not get right on iOS 26.6.2

- **`ProcessControlClient::kill_app` does not kill.** It sends `killPid:` with the pid as a primitive 32-bit integer. On this device that left Settings running, and the DVT connections dropped ("device socket io failed"), the observing connection included. The same `killPid:` with the pid archived as an `NSNumber`, which is how pymobiledevice3's `kill_pid_` sends it, killed Settings. So did `sendSignal:toPid:` with SIGKILL, which pymobiledevice3's `kill` uses because its reply confirms the kill (`results/kill-check.json`, one run of each). The lifecycle series send both working forms over a DVT channel of their own.
- **`launch_app` drops the device's refusal.** Any reply that is not a pid becomes "expected integer PID in launch app response". The crate's `launch_with_options`, which extracts the `NSError` text, is crate-private.
- **CoreDevice's JPEG screenshot is refused.** `ImageFormat::Jpeg` sends `requestedFormat` `jpeg`, and the device answers "Cannot initialize ScreenshotImageFormat from invalid String value jpeg" (`NSCocoaErrorDomain` 4864). So do `jpg`, `JPEG`, `heic` and `public.jpeg`; only `png` is taken (`results/probe.json`). pymobiledevice3 11.26.0's client documents `png` as the only format the device supports.
- **No foreground state in the app-state notifications.** On iOS 26.6.2 the payload carries `appName`, `execName`, `pid`, `mach_absolute_time` and `state_description`, whose values were `Running`, `Suspended` and `Terminated`. `Running` arrived just before the launch's reply when Settings came to the foreground, and also when it was launched behind the lock screen. It marks the process as running, not the app as on screen, so the tables call it `Running`.

### What was not measured

- **Installing the fixture** through the installation proxy, D19's install path. It needs the SwiftUI fixture signed for the device, and this host has no signing identity yet (design.md, Context: creating the certificate is the account holder's step). Task 3.3 measures it once the identity exists.
- **Mounting the DDI.** It was already mounted (see above).
- **A series with Device Hub showing the device**, the research note's addition. Opening Device Hub has effects on this shared host. It attaches to a Simulator's `dtuhidd` at boot, and quitting it shuts down every booted Simulator (archived design D6), which would touch the Simulators of other work on the host. Showing the phone would also start a display media stream on it, which takes the camera and microphone from other apps while it runs. Not done.
- **CoreDevice's JPEG screenshot**, refused by the device (see above).
- **The app's first frame on screen.** Nothing in DVT's notifications says when it is up; the launch numbers end at the reply, the `Running` notification and the process list. Spike S11's frame stream gives the first changed frame.

## Verdict

**Go.** D18's gate is met on iOS 26.6.2:

| Criterion | Measured | Verdict |
|---|---|---|
| The tunnel without root | an unprivileged process; the host's `utun` interfaces unchanged with the tunnel open | pass |
| A launch under 2 s at p95 | 46.4 ms for DVT's reply, 44.5 ms for the app service's, 351 ms for `devicectl`'s process | pass |
| A screenshot under 500 ms at p95 | 43.5 ms through DVT, 43.4 ms through CoreDevice's PNG, 393 ms through `devicectl` | pass |

**D18 stands, now with its numbers** (rewritten in design.md). The design keeps the `idevice` crate in the daemon's process, the tunnel in process through jktcp, and the RSD services over it. It rejects `devicectl`: every operation measured costs 228 to 370 ms at p50 through it, against 3 to 43 ms through `idevice`, and it gives neither tree nor input. Three facts of the platform join D18:

- the driver takes usbmuxd's USB entry for a phone;
- it reads the lock state from lockdown's `PasswordProtected`;
- it asks CoreDevice's screen capture for PNG only.

**D19 changes** (rewritten in design.md):

- Launch goes through DVT process control as planned.
- Termination goes through `sendSignal:toPid:`, or `killPid:` with the pid archived, never `idevice` 0.1.68's `kill_app`.
- The lock is read before a launch, since a locked phone's refusals are neither reliable nor uniform.
- The foreground application comes from the runner's read, since DVT's notifications carry no foreground state.
- CoreDevice's app service is as fast and stays the second path.
- The install through the installation proxy is still unmeasured.

**For the tasks:**

- **3.1, discovery and readiness.** This spike's probe gives the recorded values:
  - usbmuxd's two entries for one phone, on USB and on the network;
  - lockdown's `ProductType`, `ProductVersion`, `DeveloperModeStatus` in the AMFI domain, `PasswordProtected` and the battery's values;
  - the image mounter's `CopyDevices` entry;
  - the refusals a locked phone gives (`results/launch-check-locked.json`).

  The lock state reads from `PasswordProtected`, not from CoreDevice's `getlockstate`. A lockdown session costs 22 ms, so a readiness check before every attach is cheap.
- **3.2, the tunnel and the developer services.** Open the tunnel once per target (60 ms) and keep each service connection (37 ms for DVT). `RsdHandshake` and jktcp's handle both clone, but opening a connection inside a spawned task fails the compiler's higher-ranked check on `RsdService` for `Box<dyn ReadWrite>`. The spike therefore opens connections in the caller and moves each client into its task. The image was mounted, in the personalized form. Mounting it when it is missing still has to be verified on a device that lacks it: through the image mounter for a device in the build manifest, through `cryptexd` as a Cryptex1 otherwise, and both need Apple's signing server.
- **3.3, app lifecycle.** Launch through DVT (42.5 ms). Terminate through `sendSignal:toPid:` on the driver's own DVT channel (3 ms to the reply, 22 ms to the process's end), until `idevice` sends the pid archived. Time the fixture's install once the signing identity exists. The pid's `Running` notification and the process list tell `open` that the process runs before the frames do.
