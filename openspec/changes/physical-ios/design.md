# Design

## Context

See proposal.md for motivation. What the first change established and this one builds on (`openspec/changes/archive/2026-10-07-mobile-automation-foundation/design.md`, D1 to D12): a persistent daemon with a thin CLI; the device protocol, CBOR frames with a per-connection handshake and capabilities (`tree`, `input`, `screenshot`, `frames`, `events`, `text_set`, `bounds`); the mirror, which keeps a replica per target and already handles windows that send no change events by re-reading them after injected input, on frame hash changes bounded by a minimum interval, and on a poll, which is exactly how the Simulator driver's tree is kept current; the Simulator's in-guest reader, a Rust binary built for `aarch64-apple-ios-sim` that reads the foreground tree with one `-[XCTAccessibilityFramework userTestingSnapshotForElement:options:error:]` per request, prunes modal views and bounds the keyboard window by what it draws, and serves the protocol over a Unix socket; the harness with its target classes, gates, baselines and hand-offs; the compatibility table `doctor` prints.

What the platform allows on a physical device, from the investigation plan and the research so far:

- There is no host-side accessibility read on a device: the tree lives in the test automation session, which only an XCTest runner holds. WebDriverAgent, agent-device's runner and Maestro's unfinished driver are all XCTest bundles kept alive by a test that never ends, reached over HTTP; their costs are the runner's cold start (seconds), XCTest's idle wait before each action (which WebDriverAgent disables by replacing `-[XCUIApplicationProcess waitForQuiescenceIncludingAnimationsIdle:]`) and snapshots whose per-node attribute fetches grow with the tree (`docs/research/ios-device.md`). Host-side input does exist from iOS 18: Xcode 27's Device Hub drives a paired phone through the developer disk image's `dtuhidd`, the daemon the Simulator's input already goes through, reached over RSD (`com.apple.coredevice.hid.indigo`, `.hid.universalhidservice`), which `idevice` and pymobiledevice3 both implement. The sources say its reports count only while a display media stream is running; on iOS 26.6.2 spike S11 found taps, drags, the home button, a key and the virtual keyboard delivered with no stream at all, which that iOS cannot start anyway (`docs/spikes/s11-ios-frames.md`).
- The developer services are reachable over USB through usbmuxd and lockdown, and on iOS 17 and later through the CoreDevice tunnel (RemoteXPC, RSD) that Xcode's `devicectl` and pymobiledevice3's tunnels use. The `idevice` Rust crate implements usbmuxd, lockdown and pairing, the CoreDevice proxy with an in-process TCP stack (no TUN device, no root), RSD, the developer disk image mounter, the installation proxy, DVT (process control, screenshots), an `xctest` session with the test manager, a WebDriverAgent bootstrap and a `display_stream` over CoreDevice. Spike S9 (`docs/spikes/s9-ios-transport.md`) verified it on iOS 26.6.2 from one unprivileged process: the tunnel and RSD handshake in 60 ms at p50, 82 RSD services listed including the display, screen-capture and both HID services, the personalized disk image of Xcode 27.0 mounted at `/System/Developer`, DVT and CoreDevice launching Settings in 42 ms and taking a 750x1334 PNG in 42 ms at p50 against `devicectl`'s 295 and 370 ms, with two faults of the crate or the phone recorded in D19 (`kill_app`'s unarchived pid, which stalls the DVT service, and CoreDevice's lock state, which the phone answers as not implemented; JPEG captures refused).
- Frames: Xcode records a device's screen through CoreDevice, which is what `display_stream` names, and macOS exposes a connected iPhone as a screen-capture device to AVFoundation once CoreMediaIO is told to allow it, the path QuickTime Player's movie recording takes; DVT screenshots are a request each. Which gives a hashable stream at 30 fps under a quarter of a core, and at what latency after a tap, is spike S11's question.
- The accessibility audit service that Xcode's Accessibility Inspector and go-ios's `ax` use navigates the screen element by element (focus, label, perform action) rather than returning a tree, so it is a complement for actions on an element at most, not a tree source.
- Signing: the runner must be signed for a team with a provisioning profile that includes the device; the maintainer has a paid team. Developer Mode must be on, and the device unlocked and trusted. Nothing removes those steps; the driver reports which is missing.
- The device for this change: an iPhone SE (3rd generation, A15) on iOS 26.6.2, under the host's Xcode 27.0; its identifier stays in the environment and the reports' provenance. Hardware buttons and biometrics cannot be assumed to work on a test phone, so the home press goes through the runner's software press and unlocking is done by the person at the device; device names are not unique, so the driver and every spike address a device by identifier, never by name.
- Signing is the account holder's, never a session's: creating the development certificate and registering the device with the account happen in Xcode by hand, and the build scripts here sign with automatic signing only once the device is already in a development profile, so that `xcodebuild` registers nothing. A host with no development identity, profile or team is the starting state `doctor` must name.

Research note: `docs/research/ios-device.md` (task 1.1). Spike reports: `docs/spikes/s9-ios-transport.md`, `docs/spikes/s10-ios-runner.md`, `docs/spikes/s11-ios-frames.md` (tasks 2.1 to 2.3).

## Goals / Non-Goals

**Goals:**
- A physical iPhone as a third target class behind the same protocol, mirror, CLI and harness, with numbers: the gates of the benchmark spec for the `ios-device` class, measured on the maintainer's phone against the baselines that support devices.
- A runner that starts once per session and never restarts in it, speaks the device protocol over a socket, and hands the tree to the Simulator reader's conversion, so the mirror's listing rules and refs carry over without change (modal views and the keyboard window as far as the device's tree reports them, D21).
- Frames from the host as the change signal, a stream where the device offers one and polled screenshots where it does not (D23), so settle and the mirror's re-reads work without an event stream, as on the Simulator, and no idle wait anywhere.
- Every host-side piece in Rust, in one new crate; the runner in Rust with an Objective-C shim only where a bundle's entry point or a private call needs one.
- iOS 26 first, as a specific path: the driver is built and measured for what the device at hand runs, iOS 26.6.2, where the display stream is refused and `dtuhidd` needs no stream (S11), and every path it takes is gated on the device's iOS version as the Android agent gates its reads on the API level (`Levels.kt`) and the Simulator driver its transport on the CoreSimulator version. iOS 27's display stream, and its input that needs the stream, come later as another gated path, added when a device runs it, costing the iOS 26 path nothing.
- Keep the core lean: nothing device-specific enters `protocol`, `mirror`, the CLI or the Android driver; what the device needs is gated in `crates/ios` and the runner.

**Non-Goals:**
- Wi-Fi devices, watchOS, tvOS, visionOS, more than one iOS version, an in-app SDK, running XCTest test suites, CI with a device.
- Matching the Simulator's numbers. The device tier is the slowest by construction; the goal is the model holding, measured.

## Decisions

Decisions D18 to D23 were candidates until their spikes recorded numbers here (Delivery process, Measured decisions): S9 for D18 and D19, S10 for D20 to D22, S11 for D22's host path and D23, all on an iPhone SE (3rd generation) with iOS 26.6.2 under Xcode 27.0; the spike review (task 2.4) consolidated them on 2026-10-09.

### D18. Transport: the `idevice` crate in the daemon's process, with the tunnel in process
`crates/ios` lists devices over usbmuxd and takes the device's USB entry: usbmuxd lists a phone with Wi-Fi sync on a second time, on the network. It reads the host's pairing record from usbmuxd (`ReadPairRecord`) and opens lockdown sessions with it. It opens the CoreDevice tunnel through lockdown's `CoreDeviceProxy`, with `idevice`'s in-process TCP stack (jktcp), and reaches the developer services over RSD. There is no `devicectl`, no Python or Go helper, no root and no TUN device. Spike S9 (`docs/spikes/s9-ios-transport.md`) measured these p50 and p95 times on an iPhone SE (3rd generation) with iOS 26.6.2, from an unprivileged process, with `idevice` 0.1.68:

- usbmuxd's device listing: 0.40 and 0.93 ms;
- a lockdown session with the pairing record and one value: 22.1 and 30.7 ms, 17.8 and 24.9 ms of it the TLS session;
- the tunnel with RSD's handshake: 59.9 and 70.0 ms (`CoreDeviceProxy` 42.2 and 52.3 ms, the handshake 15.9 and 20.7 ms); the host's `utun` interfaces stay the same while it is open;
- the first DVT connection after the tunnel opens: 37.0 and 38.5 ms, later ones 25 to 30 ms, so the driver keeps its service connections open for the target's lifetime;
- a screenshot through DVT, 42.3 and 43.5 ms, or through CoreDevice's screen capture, 42.5 and 43.4 ms: the same 750×1334 PNG of 141 KB.

The device lists 82 RSD services. Among them are the display service, the screen capture service and the two HID services (`hid.indigo`, `hid.universalhidservice`) that D22 and D23 build on, as well as the app service, the device information service, `testmanagerd.remote` and DVT's `instruments.dtservicehub`. Three behaviours of iOS 26.6.2 shape the driver:

- The lock state comes from lockdown's `PasswordProtected`, which follows the lock as `devicectl`'s `passcodeRequired` does. CoreDevice's `getlockstate` is listed but answers that it is not implemented.
- CoreDevice's screen capture takes only `png`; `idevice`'s JPEG request is refused.
- The developer disk image of Xcode 27.0 was mounted as the personalized image at `/System/Developer`, through the image mounter. From iOS 17.4, a device missing from a disk image's build manifest mounts it only as a Cryptex1 through `cryptexd`, and either form needs Apple's signing server online. S9 exercised neither mount, since the host's CoreDevice had already mounted the image.

The tunnel carries the RSD services only. Its TCP stack sends one segment at a time and retransmits after 200 ms, so the runner's socket goes through usbmuxd's port connection, as `idevice`'s own WebDriverAgent client and agent-device's transport do.

Alternatives:

- `xcrun devicectl` (rejected for the daemon: a process per operation, 228 ms for `device info lockState`, 370 ms for `capture screenshot`, 295 ms for `process launch` and 272 ms for `process terminate` at p50, against 22, 42, 42.5 and 2.9 ms for the same through `idevice`, about 250 ms of each call passing before its request reaches the device; no tree or input; its tunnel is CoreDevice's);
- pymobiledevice3's `tunneld` or go-ios's tunnel (not measured: a helper process and another runtime, and a kernel tunnel needs root);
- libimobiledevice bindings (not measured: no CoreDevice tunnel).

D18's gate is met: the tunnel runs without root, the slowest launch path through `idevice` answered in 46.4 ms at p95 against 2 s, and a screenshot took 43.5 ms at p95 against 500 ms.

### D19. Lifecycle through DVT process control, with the lock read first
Launch with DVT process control's `launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:` with `KillExisting`, whose reply carries the pid. Terminate with `sendSignal:toPid:` and SIGKILL, whose reply confirms the signal; `killPid:` with the pid archived as an `NSNumber` is the form without a reply. `idevice` 0.1.68's `kill_app` is not used. It sends the pid as a primitive integer, and on iOS 26.6.2 that left the app running and dropped every DVT connection, so the driver sends the call itself until the crate changes. S9 measured these p50 and p95 times on Settings:

- the launch's reply: 42.5 and 46.4 ms;
- the pid's `Running` app-state notification: 40.2 and 44.5 ms, so the process runs before the reply arrives;
- the pid in DVT's process list: 47.9 and 51.4 ms;
- the kill's reply: 2.9 and 5.0 ms through `sendSignal:toPid:`, 0.12 and 0.49 ms to write `killPid:`;
- the pid gone from the process list: 22.4 and 24.2 ms after `sendSignal:toPid:`, 22.5 and 25.9 ms after `killPid:`.

A locked device makes a launch's outcome unreliable, so the driver reads `PasswordProtected` before a launch and reports `locked`:

- DVT refuses with an `NSError` that the crate does not decode.
- The app service refuses with SpringBoard's reason, "Locked", but at other moments it launched the app behind the lock screen.

The foreground application's identity comes from the runner's read. DVT's app-state notifications report only `Running`, `Suspended` and `Terminated` on iOS 26.6.2, and `Running` came for an app behind the lock screen too. Apps are installed from the signed fixture bundle through the installation proxy. S9 did not measure the install, since the host has no signing identity yet, so task 3.3 measures it. `open` settles as on the Simulator, on frames.

Alternatives:

- CoreDevice's app service (`launchapplication`, `sendsignaltoprocess`; kept as the second path): 41.3 and 44.5 ms to launch and 3.4 and 8.9 ms to the kill's reply, the same as DVT within noise, and its refusals are readable;
- `devicectl device process launch` and `terminate` (rejected: 295 and 351 ms to launch and 272 and 320 ms to terminate, at p50 and p95, each a process).

### D20. The runner: a resident XCTest bundle in Rust, started through the test manager
`agent-ios/` builds the runner the way spike S10 (`docs/spikes/s10-ios-runner.md`) built it on an iPhone SE (3rd generation) with iOS 26.6.2:

- **The agent** is a Rust static library for `aarch64-apple-ios` holding the device protocol's `AgentSession` behind a TCP listener on the device's loopback, its requests run on one worker thread. Its hello names the runner and its version, as the Android agent's does, and a mismatch replaces the bundle. It serves `snapshot` (D21), `text_set` and `screenshot`, and `tap`, `swipe` and `key` as D22's fallback.
- **The shim** is Objective-C, compiled with the iPhoneOS SDK's clang: the `XCTestCase` subclass whose one test starts the agent and spins the main run loop forever, as WebDriverAgent's does; the replacement of XCTest's idle waits at load; and the XCTest calls, as C functions, that need blocks, structs or the main thread.
- **One image**: `clang -bundle` links the shim and the static library into the test bundle's `MH_BUNDLE`; an `MH_DYLIB`, the shape a Rust `cdylib` has, loaded as well.
- **The bundle** is a copy of Xcode's `XCTRunner.app` named `<Name>-Runner.app` with its executable `<Name>-Runner` **thinned to arm64** (the template is fat, and an A12 or later phone runs its arm64e slice, which refuses the arm64 test bundle: "incompatible architecture (have 'arm64', need 'arm64e')"), a `UILaunchScreen` entry (else UIKit gives the runner the 320×480-point compatibility size), the test bundle at `PlugIns/<Name>.xctest`, no XCTest framework embedded, the development profile inside, both signed inside out with `codesign`, the identity and profile installed and the team the environment names (`MOBILE_AGENT_IOS_TEAM`, never in a tracked file), so no `xcodebuild` host project is needed. The test manager accepted it signed outside Xcode. Building and signing it took about 2 s incrementally.
- **Starting it**: `idevice` 0.1.68's `xctest` session (`XCUITestService::run`) with the fixture as the application under test, kept on a task for the target's lifetime; the host reaches the agent through usbmuxd's `Connect` and polls until the hello. Stopping closes the session, XCTest ends the runner, and the driver also signals any runner process left through CoreDevice's app service.
- **XCTest's idle waits**: Xcode 27's XCTest (25228) has no `-[XCUIApplicationProcess waitForQuiescenceIncludingAnimationsIdle:]`, the call WebDriverAgent replaces. The runner replaces its successors, `waitForQuiescenceIncludingAnimationsIdle:isPreEvent:` and `…usingActivity:isPreEvent:`, and `-[XCAXClient_iOS waitForQuiescenceOnAllForegroundApplicationsAsPreEvent:]`. The compatibility table records each call site with the iOS and Xcode versions it was validated on (`docs/spikes/s10-ios-runner.md`, Private call sites).

S10 measured these p50 and p95 times over 20 runs on a quiet host:

- the start to the runner's hello, the bundle installed: 772 and 873 ms, of which 330 and 352 ms to the runner's `_XCT_testRunnerReadyWithCapabilities:` and 337 and 360 ms to the test bundle loaded;
- installing the runner through the installation proxy, then starting it to the hello: 1,845 and 2,023 ms, of which the install takes 1,118 and 1,290 ms;
- the runner's physical footprint: 43.3 MiB at the start of 30 minutes of commands, 45.7 MiB at the end.

Three conditions of the phone come first, and the driver reports each: the developer disk image mounted (without it the RSD list lacks `com.apple.dt.testmanagerd.remote` and the session fails with "service not found"); "Enable UI Automation" authorized by the device owner once (testmanagerd asks LocalAuthentication for it, and XCTest waits 60 s in "enabling automation mode"; after one approval no start asked again in the session); and the phone unlocked. `idevice` reports the bundle-load failure, XCTest error 103, as an untrusted certificate, which it was not. testmanagerd stopped answering the DTX handshake for a minute or two after runners were ended mid-start, so the driver retries the session rather than failing at once.

One runner served 30 minutes of mixed commands (a snapshot, a tap and a screenshot every 3 s, 600 cycles) with one pid, the session never ending and no command failing. A pulled USB cable ends the test manager's session within 5 s, but the runner outlives it: after the replug the next start's hello came from that old process, which then closed the connection. The driver therefore ends any runner left from a lost session before it attaches again; once the old process was gone, a new attach said hello in 963 ms.

Alternatives: WebDriverAgent itself (HTTP and JSON per node, an idle wait per action, a Node and Carthage build); agent-device's runner (its own protocol; a Node dependency); a `devicectl`-launched runner without the test manager (no automation session, and on iOS 27 a runner launched in the foreground never backgrounds itself or serves, which is why Appium disabled that path there).

D20's gates on iOS 26.6.2: a tree under 1 s at p95 passes on both fixtures (D21), and no restart in the session passes; tap dispatch under 150 ms at p95 fails through the runner, 294 ms synthesized and 795 ms through `XCUICoordinate`, and passes through the host's `dtuhidd`, 53.5 ms, which D22 makes the default.

### D21. Tree: XCTest's snapshot, fetched by the test manager, through the reader's conversion
The Simulator reader's `userTestingSnapshotForElement:` does not run in the runner on iOS 26.6.2 (S10). The runner is a sandboxed app, and its sandbox denies the Mach lookup of another application's accessibility server ("deny(1) mach-lookup com.apple.iphone.axserver"), so every snapshot fails with `kAXErrorServerNotFound`; `+[AXElement primaryApp]` answers nil; `proc_pidpath` fails for other processes. XCTest's own snapshot does read, because testmanagerd fetches it with its own privileges. The runner therefore reads:

- the foreground's pid from `-[XCAXClient_iOS activeApplications]` (its system application, SpringBoard, when none is active), checked again after the read (`window_changed`);
- its bundle from the daemon session's `-requestApplicationSpecifierForPID:reply:`;
- the tree from `-[XCAXClient_iOS requestSnapshotForElement:attributes:parameters:error:]` on that pid's application element, which testmanagerd fetches: the reader's attribute names without `XC_kAXXCAttributeChildren`, whose array keeps the reply from decoding in the runner, and modal views not honoured (task 4.4); the element type, traits, frame, identifier, label, value, placeholder, keyboard focus, platform class, scroll areas and interaction become the reader's `Element`s, which the reader's `elements.rs` converts into the same CBOR snapshot with `emits_events` false on every window. The platform class is `XC_kAXXCAttributeElementBaseType`'s, since `XC_kAXXCAttributeElementType` never comes back on iOS 26.6.2: `UIRemoteKeyboardWindow`, `_UIAlertControllerPhoneTVMacView` and `UITransitionView` as the reader names them, `UIResponder` for a SwiftUI element.

S10 read the Big tab's 500 cells this way, 20 times on a quiet host, p50 and p95:

- SwiftUI: 546 nodes, as the Simulator's reader reads, in 463 and 476 ms, of which XCTest's snapshot is 450 and 461 ms and the conversion 0.5 and 0.6 ms;
- Expo: 529 nodes in 516 and 527 ms, XCTest's snapshot 503 and 515 ms.

Both list `cell-1` to `cell-500` with their labels and frames. XCTest's public snapshot carries no platform class, so in S10 the conversion's rules that need one saw none. S10 asked `XCAXClient_iOS`'s request with the reader's attribute names, and testmanagerd's reply did not decode in the runner ("value for key 'NS.objects' was of unexpected class 'NSArray'"); task 4.4 found the cause in `XC_kAXXCAttributeChildren` and reads the tree through that request without it, as above, so the keyboard's window, alerts and scroll areas come through as on the Simulator: the fixtures' Big tabs, the Login tab with the keyboard up and the System tab's alert list through the mirror as the Simulator reader's reads do, where the screens' sizes allow. Measured so, the SwiftUI Big tab's 546 nodes take 550 and 554 ms at p50 and p95, the Expo one's 529 take 446 and 452. What it does not tell is a view the app marks modal other than an alert, which the Simulator reader finds by a second read when the first comes back short of the children an element counts; this reply counts none. The reader's extraction stays the library both binaries share (task 4.1), its conversion used on the device as it is, its snapshot call the Simulator's. System screens come from the system application's tree when it is the foreground.

The accessibility notifications reach the runner: registered through `-[XCAXClient_iOS _registerForAXNotification:error:]` (108 ms each), they arrive at the daemon session's `_XCT_receivedAccessibilityNotification:fromElement:payload:` with a binary property list as payload. A tap on a cell brought 29, among them one layout change (1001) from the application; a switch to another tab one screen change (1000) and two layout changes; a scroll one layout change among 50 HID events. They name no element path, so they are a coarse change signal at most, like an Android window-content change, and the windows stay `emits_events` false.

### D22. Input: `dtuhidd` from the host as the default, the runner's synthesis as the fallback
Two candidates. Through the runner: taps, long presses and swipes that rest before lifting (the Android `rest_ms` semantics, task 10.16) go through XCTest's event synthesis with the idle waits replaced, and keys and text through XCTest's typing. Spike S10 (`docs/spikes/s10-ios-runner.md`) sent taps on the same phone, screen and day as S11's `dtuhidd` taps, on S11's Settings points, 20 runs each on a quiet host, the first changed frame counted with S11's tolerance on back-to-back DVT screenshots that took 48 ms each:

| Path | Request to acknowledgement, p50 / p95 | First changed frame from the request, p50 / p95 |
|---|---:|---:|
| `dtuhidd` from the host (contact, 50 ms, release) | 52.1 / 53.5 ms (open), 52.1 / 53.2 (back) | 211 / 246 ms (open), 35 / 103 (back) |
| the runner: an `XCSynthesizedEventRecord` to `XCUIDevice`'s event synthesizer | 291 / 294 ms (open), 291 / 295 (back) | 247 / 272 ms (open), 119 / 178 (back) |
| the runner, as above, the confirmation interval set to 0 | 284 / 292 ms (open), 285 / 291 (back) | 267 / 276 ms (open), 120 / 168 (back) |
| the runner: `-[XCUICoordinate tap]` | 698 / 795 ms (open), 566 / 607 (back) | 648 / 748 ms (open), 406 / 460 (back) |

The synthesizer's completion comes after the screen has already changed: the daemon session synthesizes with `_XCT_synthesizeEvent:implicitConfirmationInterval:completion:` and an implicit confirmation interval of 5 s, and the completion arrived about 240 ms after the 50 ms touch. Setting that interval to 0 left the completion where it was. `XCUICoordinate` resolves the application before it taps. The home button through `-[XCUIDevice pressButton:]` was acknowledged in 502 ms, against 89 ms for `dtuhidd`'s Indigo button event; both reached the home screen.

From the host: touches, hardware buttons and keys go through the disk image's `dtuhidd` over RSD, as the Simulator driver already sends input to its `dtuhidd`, with no XCTest in the path. They are acknowledged when the report is written to the tunnel. Spike S11 (`docs/spikes/s11-ios-frames.md`) drove an iPhone SE (3rd generation) with iOS 26.6.2 this way, through `idevice` 0.1.68's HID clients on the in-process tunnel:

- touches as 58-byte `mainTouchscreen` reports to `hid.universalhidservice`: a report is written in 0.08 ms at p50 and 0.71 ms at p95, and a tap (contact, 50 ms, release) is acknowledged in 52.2 and 53.7 ms; every one of 40 taps and 20 drags changed the screen;
- the home button as an Indigo button event (consumer page 0x0C, usage 0x40), a home press without the physical button, which does not work on this phone;
- keys as Indigo keyboard events, and through the virtual keyboard the host registers (`mainKeyboard`, removed afterwards): both typed into Safari's address field in one try each;
- no display stream: on iOS 26.6.2 the reports land without one, so they cost no camera, and none can start there (D23). pymobiledevice3 and `idevice` report that iOS 27 drops the reports unless a display stream runs, so the driver opens one only on a device whose display service lists media features, and only there does the target cost the camera.

D22's gate is met for the host's path on iOS 26.6.2: a tap's dispatch, 53.7 ms at p95 with its 50 ms hold in S11 and 53.5 ms in S10, is under 150 ms, and every tap reached the screen; the runner's synthesis misses it at 294 ms, and `XCUICoordinate` at 795 ms. `dtuhidd` from the host is therefore the default for touches, the home button and keys on iOS 26.6.2, and the runner's synthesis the fallback, gated on the device's iOS version (a stream on iOS 27). The frame stream carries what follows either way: from the request, the host's taps changed the screen 36 ms (the row) and 84 ms (the back button) sooner at p50 than the runner's.

Text goes through the runner, measured by S10 on the SwiftUI fixture's focused `login-username`, 20 runs each:

- typed, the harness's 36 characters through the daemon proxy's `_XCT_sendString:maximumFrequency:completion:` at WebDriverAgent's 60 characters a second: 807 and 813 ms at p50 and p95, all 20 read back whole;
- set, the same 36 characters: 163 and 173 ms, all 20 whole. The runner finds the element with keyboard focus in XCTest's snapshot and sets its value through the daemon's `_XCT_setAttribute:value:element:reply:` with the value attribute's number (2006), or `-[XCAXClient_iOS setAttribute:value:element:outError:]` with its name. The latter with the number, and `_XCT_performAccessibilityAction:onElement:withValue:reply:` with action 0, fail with `kAXErrorIllegalArgument`.

Text goes in typed, or set on the focused element where typing fails, the method reported as on Android. A pasteboard path exists on a device too (`XCUIDevice` has none; the runner can set `UIPasteboard` in its own process, which is not the app's) and is not used.

### D23. Frames: screenshots on iOS 26, compared below the status bar with a two-level tolerance
Spike S11 (`docs/spikes/s11-ios-frames.md`) tried the three paths on the iPhone SE (3rd generation) with iOS 26.6.2, from one unprivileged process:

- **The CoreDevice display stream is refused.** Every start form gets "Remote control requires iOS 27.0 or later on this device" (CoreDevice error 9021). That covers `idevice`'s form and pymobiledevice3's, the host's CoreDevice version in the request, and the Camera app in front. The display service's `getmediasupportinfo` lists no features. The spike's RTP receive loop and VideoToolbox decoder are written and unexercised.
- **AVFoundation capture is not measured.** Once CoreMediaIO's screen-capture allowance was set (it reads 0 by default on macOS 26.6.2), the phone appeared as an external muxed device. The spike found it by its name, since its unique identifier is not the UDID. Without the macOS camera permission, a session delivered no frame. The spike asked for the permission and no one answered within 10 minutes; the phone later stopped appearing.
- **Screenshots run.** The spike requested DVT's `takeScreenshot` and CoreDevice's `capturescreenshot` back to back on connections of their own, decoded each PNG to BGRA and hashed it with the Simulator pipeline's functions. The pictures are lossless: on a still screen, no pixel of consecutive frames differed by more than one level, and the rare one-level moments were the screen's own.

S11's numbers for screenshots, at p50 and p95 where timed. The host was quiet but the phone busy: `contactsd` held one of its cores at 100%, and every transport step took several times S9's.

- One capture: 105 and 184 ms through DVT (S9: 42 and 44), 131 and 307 ms through CoreDevice, for a 141 KB 8-bit PNG. The home screen's 3 MB 16-bit PNG takes 1.1 s.
- Frames a second, on a still screen and while scrolling: 7.8 and 9.5 with one poller, 16.8 and 15.1 with two (DVT and CoreDevice), 18.2 and 19.4 with four. The phone serves about 19 captures a second.
- CPU: 6.5 to 15.5% of a core in this process, the decode included, and 3.2 to 7.4% in usbmuxd beside it; 22.9% together at most.
- A tap's first changed frame after its acknowledgement: 134 and 251 ms for a back button, which highlights on contact, and 274 and 439 ms for a row that opens a page. A drag's first changed frame: 272 and 506 ms after its first report. All with one poller.
- With two pollers, the same taps' first changed frames came at 68 and 222 ms (back button) and 241 and 309 ms (row).

The change signal is a frame with a pixel below the status bar more than 2 levels away from the frame before it. That tolerance counted none of 1,283 still-screen frames and caught every tap and drag on its first changed frame. `content_hash`'s 32-pixel thumbnail is not the signal for screenshots: it averaged away the back button's highlight in 8 of 20 taps. The status bar's band comes from the device: 20 points on this phone. `status_bar_rows` takes 7.5% of the height, an iPhone 17's proportion, which on the SE covers most of the back button.

D23's gate is not met on this phone. The rate peaks at 19.4 frames a second against 30, and a tap's first changed frame comes at 222 to 439 ms at p95 against 100; the CPU (22.9% at most) and the still screen (no frame counted) pass. The driver therefore:

- polls screenshots on two connections, DVT and CoreDevice, as the device's frame source on iOS 26. It compares a PNG's bytes with the previous one's first and decodes only a picture that differs: on a still screen 246 of 247 consecutive PNGs were identical. DVT's screenshot stays the on-demand full-resolution path;
- treats the device as a target with slow frames. Settle waits for at least two capture intervals without a change, and a re-read after input waits up to 500 ms for the first changed frame;
- uses AVFoundation instead once S11's `avf-*` series pass the gate. That needs the camera permission for the app that starts the daemon, which is the maintainer's to grant, and the tolerance is then set from that stream's noise floor;
- uses the display stream only on a device whose display service lists media features, iOS 27 by its refusal.

Without any frames, settle falls back to the mirror's probe-less path for windows without events: re-reads every minimum interval, as the device has no `bounds` capability unless S10 adds one.

### D24. The harness's `ios-device` class
`TargetClass::IosDevice` with the gates the spec delta states, the SwiftUI and Expo fixtures built for the device (`-destination generic/platform=iOS`, the team from the environment), installed by the harness through the driver, and the baselines that support a device: agent-device, whose runner signs for the team and answered a snapshot in 440 ms over usbmux on an iPhone 17 Pro in its own measurements, and WebDriverAgent through Appium's driver; Maestro 2.11.0 refuses physical iPhones, its device driver unfinished, so it is skipped with that note. Hand-offs between tools stop and uninstall the other tool's runner as on the Simulator (task 10.18), since one automation session exists per device.

### D25. Workspace layout
```
crates/ios          discovery, readiness, tunnel, lifecycle, runner session, host input (dtuhidd), frames, screenshots   (macOS only; a stub elsewhere, as crates/simulator)
agent-ios/          the runner bundle: Rust static library and ObjC shim linked with clang -bundle into PlugIns/<Name>.xctest inside a thinned copy of XCTRunner.app, Info.plists, the build and sign script (codesign with the installed profile, no xcodebuild project), VERSION
crates/simulator/reader/   gains a library target for the element conversion (elements.rs) both binaries share; the snapshot call stays the Simulator's
```
The driver registers as `ios` in the daemon's driver list; `MOBILE_AGENT_DRIVERS=ios` runs it alone. The daemon's fake targets gain nothing: the fake already stands in for a target without events.

## Delivery process

The rules of the archived design's Delivery process bind this change unchanged (research first, spikes before construction, measured decisions, tests first with red-then-green, atomic commits without trailers, validation and fix after every task, installs through `sfw`, the pinned toolchain, recap before dispatch), with these specific to a device:

- **The device is the maintainer's.** Pairing, trust, Developer Mode, the passcode and the team's certificates are the maintainer's steps; a task that needs one asks and waits, never works around it. The device identifier and the team id go in the environment and the reports' provenance, never in a tracked file. No command touches another session's Simulators or the emulator.
- **One automation session per device.** Spikes and tasks that start a runner stop it when they end; a task that finds another runner holding the device (Maestro's, agent-device's, WebDriverAgent) reports it and stops.
- **Quiet host, charged device.** Measured runs follow the harness's guard and record the load; the device is plugged in, unlocked, with auto-lock off for the run and the battery above half, recorded in the report's provenance.

## Risks / Trade-offs

- [An `idevice` call misbehaves on iOS 26.6.2, as 0.1.68's `kill_app` did (a pid sent unarchived stalls the phone's DVT service) and CoreDevice's lock state does (not implemented on the phone)] → S9 reached every service the driver needs and recorded the calls that work (`sendSignal:toPid:`, lockdown's `PasswordProtected` for the lock, PNG captures only); the driver uses those, and `devicectl` stays the per-operation fallback for lifecycle at about 250 ms more per call. The runner's launch goes through the test manager; a `devicectl` foreground launch never serves on iOS 27, so it is no second path.
- [A later iOS refuses a runner signed outside Xcode, which iOS 26.6.2 accepted (S10: a thinned copy of `XCTRunner.app`, the test bundle in `PlugIns`, no XCTest framework embedded, `codesign` with the installed development profile)] → the fallback is an `xcodebuild` host project that Xcode signs; the profile and certificate stay the maintainer's.
- [XCTest's private calls change under a later iOS: the idle-wait replacements, the daemon session's synthesis, screenshot, `_XCT_sendString` and `_XCT_setAttribute`, `XCAXClient_iOS`'s notifications] → the compatibility table records each call site per version, as D12 does for the Simulator; `dtuhidd` carries input without XCTest, the public `XCUICoordinate` path and `XCUIScreen` stay as the slower fallbacks.
- [A runner outlives a lost session, as it did after S10's cable pull, and answers the next attach with a stale process] → the driver ends any runner process left before it starts a session, and retries the test manager's handshake, which stops answering for a minute or two after a runner is ended mid-start.
- [After a reboot the disk image is gone and "Enable UI Automation" may be asked again; `idevice`'s personalized mount hung once in S10] → readiness reports the image and the approval as states with their steps; the mount goes through `idevice` with `devicectl`'s mount as the fallback, timed in task 3.2.
- [No host-side frame stream works at 30 fps, which S11 found to be the case on iOS 26.6.2: the display stream needs iOS 27 and AVFoundation capture waits on the camera permission] → the device is a slow-frame target, as D23 now says: screenshots polled on two connections at up to 19 frames a second with a two-level tolerance tell changes exactly, settle counts them, and the tap-to-frame-change gate stays unstated; AVFoundation replaces the polling once its series pass with the camera allowed, the display stream once the phone runs iOS 27.
- [The device sleeps, locks or disconnects mid-run] → readiness checks before every attach, `disconnected` with the cause, a resumed attach starts over; auto-lock off is a documented prerequisite of measured runs.
- [The runner's cold start dominates cold attach] → the runner stays installed and the session is kept for the target's lifetime; cold attach's gate counts the session start, not the install.

## Migration Plan

Additive: a new crate, a new bundle, a new target class. Nothing deployed changes for Simulators or Android. Rolling back is removing the crate from the driver list.

## Open Questions

- iOS 27: the CoreDevice display stream (HEVC over RTP, keyframes on request) and `dtuhidd` input behind a running stream are a later path, gated on the device's version, to be spiked on a phone that runs iOS 27 (the iPhone 17e once upgraded, or the SE) with S11's measurements repeated there; until then the iOS 26 path is the only one, and the harness's tap-to-frame-change on this class stays reported, not gated.
- AVFoundation capture is host-side and not tied to the iOS version: once the camera permission is granted, its series decide whether it replaces polled screenshots on iOS 26 (30 frames a second and a first changed frame within 100 ms at p95 would pass D23's gate); a failure leaves the polling.
- Hardware buttons: the home press goes through `dtuhidd`'s Indigo button event (89 ms, S11 and S10) and needs no protocol method, since the host sends it; lock and volume are validated in task 3.8, and the runner's `-[XCUIDevice pressButton:]` (502 ms) is the fallback.
- A real app on the device: the maintainer's own development build, or an App Store app with a read-only account, driven through the runner and `dtuhidd` (task 2.5, after the same app is checked on the Simulator), to see what XCTest's tree loses on a React Native screen before the driver's shape is fixed.
- Whether the accessibility audit service's element actions are worth a fallback for apps whose XCTest tree is sparse, as some React Native screens were on the Simulator (`reports/10.4-rn-smoke.md`); measured if a device run shows such a screen.
- Which baselines support a physical device on this host without their own signing setup; found out in task 7.3 and recorded in the report.
