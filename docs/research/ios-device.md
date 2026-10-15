# Physical iOS devices: state of the art

Research note for task 1.1 of `physical-ios`. It records how a Mac reaches a physical iPhone's services on iOS 17 to 26 (and 27 where a source tested it), what the tools that drive devices do and what each costs, what the `idevice` crate implements, the paths that bring frames to the host, and the signing a runner needs. Spikes S9 (transport and lifecycle), S10 (the resident runner) and S11 (frames), and the `crates/ios` and `agent-ios/` groups, rest on it. Every claim was read on 2026-10-08 from the sources pinned under [Sources](#sources). Library and tool facts come from current documentation. Context7 covers pymobiledevice3, go-ios, `idevice` and Appium's XCUITest driver, and each of these was then read in its repository at a pinned commit; where the two differ, the repository wins and the section says so. Context7 indexes only an archived fork of WebDriverAgent and none of Maestro's device code, agent-device, idb's device support, quicktime_video_hack, `ios-rs`, `rusty_libimobiledevice` or the objc2 framework crates. Those were read in their repositories, on crates.io and on docs.rs. Apple's facts come from its documentation, read through developer.apple.com's documentation JSON, and from the WWDC26 session page. Claims tagged `[local]` were checked read-only on the reference host with the commands under [Local evidence](#local-evidence); no device was attached to, listed or touched.

Reference host: macOS 26.6.2 (25G83), Xcode 27.0 (27A266a), CoreDevice and `devicectl` 642.16, and the iOS developer disk image (DDI) of Xcode 27.0, with its XCTest at version 25228 `[local]`. The change's device runs iOS 26.6.2.

## Verdict for this change

- **Transport: `idevice` 0.1.68 covers everything the spikes need except the accessibility audit service.** It speaks usbmuxd and lockdown with the host's pairing record. It opens the CoreDevice tunnel through lockdown's `CoreDeviceProxy`, with jktcp as an in-process TCP/IP stack, so neither a TUN device nor root is needed. On top of that it implements RSD, the image mounter with personalization and `cryptexd`, AMFI, the installation proxy, DVT process control and screenshots, the test manager's XCTest session (validated against iOS 26.4), and CoreDevice's app, screen-capture, display and HID services [idv-feat][idv#78][idv#103]. pymobiledevice3 11.26.0 made the same in-process tunnel its default on every host. The kernel tunnel needs root, both to create a `utun` interface and to suspend macOS's `remoted`, and riding `remoted`'s own tunnel makes the two evict each other [pmd3-net].
- **Input from the host exists on a device, through the display stream.** Xcode 27's Device Hub shows a paired physical device's screen and controls it: "This works the same whether it's a device or simulator", with keyboard and pointer input from iOS 18 [wwdc26-260][xc27-rn]. The mechanism is the DDI's `dtuhidd`, the daemon the Simulator's input already goes through, reached over RSD. backboardd drops its events unless a display media stream is running [idv-hid][pmd3-hid]. `idevice` and pymobiledevice3 both drive taps, drags, hardware buttons and a virtual keyboard this way. design.md's Context says no host-side input exists on a device, and D22 sends all input through the runner.
- **The display stream costs the camera.** Neither Device Hub nor a third-party client can start it while an app holds the camera or microphone (CoreDevice error 9022). While it runs, lower-priority apps "may record silence and empty video", and a phone call ends it [dh-interact][pmd3-disp]. The proposal names the camera among the reasons to test on a device.
- **Frames: three paths, none of them an exact change signal.**
  - The CoreDevice display stream is plaintext RTP carrying HEVC over UDP through the tunnel, capped near 6 Mbps, at 42 to 55 frames a second during swipes on iOS 27. Its keyframes are rare: a lost packet corrupts the picture until a requested keyframe arrives, 100 to 300 ms later [idv-ds][pmd3-stream][pmd3-offer].
  - AVFoundation capture through CoreMediaIO's allowance delivers the phone's lossy H.264 stream. It keeps arriving at 30 frames a second on a still screen with about 3% of the bytes changed, so an exact comparison sees a change in every frame [serve-sim-still].
  - DVT and CoreDevice screenshots cost a request each and come as PNG, or JPEG from CoreDevice.
- **Every runner is an XCTest bundle kept alive by a test that never ends, and the host talks HTTP to it.** That holds for WebDriverAgent, agent-device's runner and Maestro's unfinished driver. None speaks a binary protocol, and none reads the tree other than through XCTest's snapshots [wda-runner][ad-mux][mae-devctl]. WebDriverAgent turns XCTest's quiescence wait off by replacing `-[XCUIApplicationProcess waitForQuiescenceIncludingAnimationsIdle:]`. Its comments say `XCAXClient_iOS` makes its accessibility round trips in the runner's own process [wda-quiesce][wda-axproxy]. Xcode 27's device build of XCTAutomationSupport exports the calls the Simulator reader makes into that framework `[local]`.
- **Maestro 2.11.0 refuses physical iPhones**, because its device driver is unfinished [mae-sess][mae#3609]. agent-device 0.21.22 runs on devices: its runner, reached over usbmux, answered a snapshot in 440 ms on an iPhone 17 Pro [ad-usbmux]. WebDriverAgent runs through Appium, go-ios, pymobiledevice3 or `idevice`.
- **Signing is scriptable except where the phone is involved.** Xcode-managed signing runs headless with `-allowProvisioningUpdates` and an App Store Connect API key, and that API creates development certificates, registers devices and makes profiles [xcb-man][asc-cert]. A free Personal Team gets 7-day profiles and 3 devices [memberships]. Trusting the computer, Developer Mode on a phone with a passcode, "Enable UI Automation" and unlocking remain the user's steps.

### Consequences for design.md

This note does not edit design.md. These are the points the spikes and task 2.4 have to settle.

1. **Context and D22.** Host-side input through `dtuhidd` is a candidate for taps, swipes, buttons and keys. As on the Simulator, it would be acknowledged when the report is sent, with no XCTest synthesis and no idle wait. It needs the display stream open, and with it the camera free. Reports count only 300 ms after the stream starts [idv-hidcli], and Device Hub cannot send a two-finger touch [xc27-rn]. S10 times this path against the runner's synthesis; the runner keeps the tree and text set either way.
2. **D23.** Whichever stream wins, the change signal needs a tolerance for coding noise. The Simulator's `pixels_hash` of a box-downscaled frame is exact. The display stream also needs VideoToolbox decoding and keyframe requests over RTCP. The AVFoundation path needs camera permission, which macOS attributes to the app that started the daemon. On macOS 27 that path also finds the phone by name, not by UDID [stim#1689].
3. **D18.** `idevice`'s in-process tunnel is the right default; pymobiledevice3 reached the same conclusion. But jktcp waits for each segment's acknowledgement before sending the next, and its first retransmission comes after 200 ms [jktcp]. The runner's socket should therefore go through usbmuxd's `Connect`, as `idevice`'s own WebDriverAgent client and agent-device's runner transport do, and leave the tunnel to the RSD services [idv-wda][ad-usbmux].
4. **D18 and S9.** From iOS 17.4, a device missing from a DDI's build manifest can mount that DDI only as a Cryptex1 through `cryptexd` over RSD [ddi-repo]. Either form needs Apple's signing server online for personalization.
5. **D20.** The test manager accepts a runner signed outside Xcode; Appium's prebuilt WebDriverAgent and go-ios's re-signing both rely on that. The conditions: the runner's executable is named `<Name>-Runner`, its test bundle is `PlugIns/<Name>.xctest`, and it embeds no XCTest frameworks [idv-xct][wda-prebuilt]. `devicectl` is a weaker fallback launcher than D20 assumes. Appium disabled it on iOS 27, where a runner launched in the foreground "never enters the background and never starts its HTTP server" [xcd-preinstalled].
6. **D24.** Maestro is not a device baseline; agent-device and WebDriverAgent are.

## The device's services and transports

### usbmuxd, lockdown and pairing

On macOS the root daemon usbmuxd serves `/var/run/usbmuxd`. A client lists devices there, watches attach and detach with `Listen`, reads the host's pairing record with `ReadPairRecord`, and opens a stream to any device TCP port with `Connect`; `idevice` honours `USBMUXD_SOCKET_ADDRESS` for another socket [idv-mux]. Lockdown answers over such a stream, under TLS made from the pairing record. A service started through lockdown names a port, which the client then reaches through usbmuxd in turn [idv-readme]. The pairing record exists once the user tapped Trust in the device's "Trust This Computer" dialog [dh-pair]. go-ios warns that macOS 26 blocks the RemotePairing records under `/var/db/lockdown` for third-party binaries through TCC [goios-main]; `idevice` asks usbmuxd for the lockdown record instead and needs no file access. A device paired over Wi-Fi through CoreDevice does not appear in usbmuxd at all [ad-usbmux].

### Developer Mode

Apple's documentation: the user turns on Settings > Privacy & Security > Developer Mode, restarts, then confirms and enters the passcode. "Developer Mode only appears in Settings if you initiate pairing or if you previously paired the device to a Mac", and without it no app runs from Xcode [devmode]. AMFI's lockdown service takes three actions: reveal the switch, request enabling (the device reboots), and confirm the prompt after the restart. pymobiledevice3 refuses the enable action with "Device has a passcode set" [pmd3-amfi]; `idevice` exposes the same actions and a status query, and its image mounter reports the status too [idv-amfi][idv-mim].

### The developer disk image

Before iOS 17 each version had its own `DeveloperDiskImage.dmg` with a signature. From iOS 17 there is one personalized image per Xcode, made of an APFS image, a build manifest and a trust cache [ddi-repo]. The device mounts it at `/System/Developer` once Apple's signing server (TSS) has signed a ticket for its nonce and identifiers [idv-mim]; pymobiledevice3 says mounting "will require network activity" on iOS 17 (Context7). Xcode 27 installs its copy under `/Library/Developer/DeveloperDiskImages/iOS_DDI`: two images with a trust cache each, and the cryptex metadata of one `[local]`. From iOS 17.4 the same DDI can be installed as a Cryptex1 through `cryptexd` over RSD. The build manifest's personalized identity names chip and board pairs, "even Xcode 27.1's DDI stops at `iPhone18,5`", while the Cryptex1 identity names no device. Newer devices therefore "can only use the Cryptex variant" [ddi-repo]. `idevice` implements both paths (`mobile_image_mounter`, and `cryptexd` with `tss`) [idv-feat]. `devicectl device info ddiServices` checks the mounted DDI and updates it by default (`--auto-mount-ddis`) `[local]`. The DDI carries the developer daemons this note relies on: DVT, the test manager, `dtuhidd`, and the XCTest frameworks under `/System/Developer/Library` [idv-xct][idv-hid][pmd3-hid].

### The CoreDevice tunnel, RemoteXPC and RSD

From iOS 17 the developer services sit behind an encrypted tunnel to a RemoteXPC endpoint, RSD, whose handshake lists the device's services with their ports. RemoteXPC travels over "non-compliant HTTP/2" [idv-readme]. One device listed 70 services through it [idv#131]. iOS 17.0 to 17.3 reach the tunnel only through RemotePairing on the device's USB Ethernet link: SRP pairing with `com.apple.internal.dt.coredevice.untrusted.tunnelservice`, then a QUIC or TCP tunnel [goios-tun]. From 17.4, lockdown's `com.apple.internal.devicecompute.CoreDeviceProxy` carries the same CDTunnel over usbmuxd. The client sends a JSON `clientHandshakeRequest` with an MTU (`idevice` asks for 16000). The device answers with both IPv6 addresses and the RSD port, and raw IPv6 packets follow [idv-cdp][idv-tunnel].

pymobiledevice3 documents three ways to end that tunnel on a Mac [pmd3-net]:

- A kernel `utun`. It needs root to create the interface, and root again to `SIGSTOP` `remoted` during the RSD handshake: "While `remoted` is running, the device **resets** a second, independent RSD root channel". This was verified against the iOS 17.0, 26.6 and 27 device daemons. It is the only path other processes can use.
- An in-process userspace stack. It needs no root and gets a host address of its own, so the device keeps it apart from `remoted`. "External tools cannot reach it", and there is one tunnel per process. pymobiledevice3 made this the default; Context7's copy of its tunnel guide still names the third path as the macOS default.
- Apple's own `remoted` tunnel, found through `remotepairingd`. It needs no root, but "opening ours evicts `remoted`'s", `remoted` evicts it back about a second later, and Xcode and `devicectl` can lose the device. It is opt-in for that reason.

On an iPhone 17-class device the native path moved data host to device at about 36 MB/s against about 20 MB/s, and answered requests in about 8 ms against about 13 ms, with pymobiledevice3's Python stack [pmd3-net]. The RSD services each check an entitlement, recorded from an iOS 18.0 device [pmd3-rxpc]:

- `com.apple.private.dt.testmanagerd.client` for the test manager;
- `com.apple.private.dt.instruments.dtservicehub.client` for DVT;
- `com.apple.private.CoreDevice.canInstallCustomerContent` for CoreDevice's app service;
- `com.apple.mobile.lockdown.remote.trusted` for the accessibility audit shim;
- `AppleInternal` for `com.apple.dt.testmanagerd.remote.automation`, the "remote automation" service of an "automation mode" in Xcode's `XCTDaemonControl` `[local]`, which is closed to a third-party host.

### DVT

Instruments' remote server is the lockdown service `com.apple.instruments.remoteserver.DVTSecureSocketProxy` from iOS 14 (`com.apple.instruments.remoteserver` before) and the RSD service `com.apple.instruments.dtservicehub` from iOS 17 [idv-dtx][pmd3-rxpc]. It speaks DTX with NSKeyedArchiver payloads, one channel per service:
- `com.apple.instruments.server.services.processcontrol`: `launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:`, with `KillExisting`, `StartSuspendedKey` and `ActivateSuspended` options, and `killPid:` [idv-pc][idv-xct];
- `...screenshot`: `takeScreenshot` returns the image's data [idv-shot];
- device info, sysmontap, graphics and others [idv-feat].

Screenshots through DVT need Developer Mode and a mounted DDI (pymobiledevice3, through Context7). idb launches apps through DVT below iOS 17 and through `devicectl` from 17 [idb-launch].

### The test manager and a remote XCTest session

`idevice`'s XCTest orchestration follows pymobiledevice3's (its comments say "matching Python"), and the two agree on the steps below [idv-xct][pmd3-xcui]. On iOS 17 and later a session runs as follows.

1. Read `ProductVersion`, make a session UUID, and open three DTX connections. Two go to `com.apple.dt.testmanagerd.remote`, one for control and one main (`com.apple.testmanagerd.lockdown.secure` on iOS 14 to 16). The third goes to DVT for process control. Each starts with a capabilities handshake [idv-dtx].
2. Open the proxy channel `dtxproxy:XCTestManager_IDEInterface:XCTestManager_DaemonConnectionInterface`. Call `_IDE_initiateControlSessionWithCapabilities:` and `_IDE_initiateSessionWithIdentifier:capabilities:` with the IDE's capabilities, reporting protocol version 36 as Xcode's [idv-dtx].
3. Launch the runner through process control, not suspended and with `ActivateSuspended`. Its environment points `DYLD_FRAMEWORK_PATH` at its own `Frameworks` and `/System/Developer/Library/Frameworks`, and `DYLD_LIBRARY_PATH` at `/System/Developer/usr/lib`. It also sets `XCTestBundlePath`, `XCTestSessionIdentifier` and `XCTestManagerVariant=DDI`, and leaves `XCTestConfigurationFilePath` empty [idv-xct].
4. Authorize the pid with `_IDE_authorizeTestSessionWithProcessID:`.
5. The runner opens the reverse channel `XCTestDriverInterface` (`idevice` waits 30 s for it) and asks for its configuration with `_XCT_testRunnerReadyWithCapabilities:`. The reply is an `XCTestConfiguration`: the test bundle URL, the session id, `automationFrameworkPath` set to `/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework`, the target application's bundle id, path, environment and arguments, and the tests to run. Then `_IDE_startExecutingTestPlanWithProtocolVersion:` starts the plan [idv-xct][idv-dtx].
6. The three connections stay open for the runner's life. `idevice`'s `run_until_wda_ready` keeps the orchestration on a background task once WebDriverAgent answers [idv-xct].

Both projects refuse a runner whose `CFBundleExecutable` does not end in `-Runner`, and derive the test bundle's name, `PlugIns/<Name>.xctest`, from it [idv-xct][pmd3-xcui]. Xcode 27's template `XCTRunner.app` has `MinimumOSVersion` 17.0, carries `get-task-allow` as its only entitlement, and links `@rpath/XCTest.framework` `[local]`. Xcode 27's device frameworks also carry the runner's side of the protocol `[local]`:
- snapshots: `_XCT_requestSnapshotForElement:attributes:parameters:reply:` and `_XCT_fetchSnapshotForElement:attributes:parameters:reply:`;
- events: `_XCT_synthesizeEvent:completion:` and `_XCT_synthesizeEvent:implicitConfirmationInterval:completion:`;
- screenshots: `_XCT_requestScreenshotOfScreenWithID:withRect:uti:compressionQuality:withReply:`;
- actions: `_XCT_performAccessibilityAction:onElement:withValue:reply:`;
- accessibility notifications: `_XCT_registerForAccessibilityNotification:reply:`, answered by `_XCT_receivedAccessibilityNotification:fromElement:payload:`;
- `_XCT_enableAutomationModeWithReply:`.

### Installation proxy, SpringBoard services and the foreground application

The installation proxy installs an uploaded bundle and lists applications with `CFBundleExecutable`, `Path` and `Container`, which is what the XCTest configuration is built from [idv-xct][idv-feat]. SpringBoard services return icons, icon state, wallpaper previews, the interface orientation and icon metrics [idv-sbs]. None of the lockdown services read for this note names the foreground application. go-ios asks its own runner for it [goios-main], and D19 takes it from the runner's read.

### CoreDevice services and Device Hub's input

Next to the lockdown shims, the RSD list carries CoreDevice's own services [idv#131]: `com.apple.coredevice.appservice`, `deviceinfo`, `openstdiosocket`, `fileservice.control` and `.data`, `diagnosticsservice`, `debugserverproxy`, and `com.apple.coredevice.screencaptureservice`, which returns PNG ("The format Xcode's Devices window uses") or JPEG [idv-scs]. The app service lists applications and processes, launches, signals and uninstalls [idv-app][pmd3-cli], which by inference is what `devicectl`'s process commands drive.

Device Hub shows "a live display of the screen" of a paired device and accepts "click, drag, scroll or use natural trackpad gestures", with "Capture keyboard" for the Mac's keystrokes [wwdc26-260]. Xcode 27's release notes add three limits [xc27-rn]:
- input works from iOS 18.0 only;
- two-finger touches cannot be sent;
- after a disconnect with "Simulate Hardware Keyboard" on, the device "may remain in 'hardware keyboard' mode for up to 2 minutes".

`idevice` and pymobiledevice3 describe the same mechanism [idv-hid][pmd3-hid]:

- **The services.** The DDI's `dtuhidd` serves two RSD services. `com.apple.coredevice.hid.indigo` takes the `{messageType, featureIdentifier, payload}` dictionaries of the Simulator's dtuhidd. `com.apple.coredevice.hid.universalhidservice` lists the device's HID surfaces and posts raw reports to them.
- **What works.**
  - Hardware buttons: `IndigoButtonEvent` with a usage page, a usage and a state.
  - Touches: 58-byte reports to `mainTouchscreen` (service 257), a contact state and 16-bit coordinates, up to five contacts.
  - Keys: a virtual keyboard the host registers with `createService` ("DeviceHub.app uses `mainKeyboardService`"), whose reports carry the full set of pressed usages; it shows no on-screen keyboard and triggers no autocomplete [idv#132].
- **What does not.** The Indigo digitizer and keyboard events. pymobiledevice3 found `dtuhidd` "immediately logs `Resetting gesture state then canceling`", and `idevice`'s swipe through them carries the comment "Does not seem to work" [pmd3-hid][idv-hidcli].
- **The gate.** Without a stream, the surfaces stay "`authenticated: NO`" and backboardd logs "ignoring digitizer event for display <main> from unsupported service". Starting the display media stream flips them, and "its RTP payload can be discarded". `idevice` then waits 300 ms because "backboardd needs a moment to re-match the HID surfaces against the newly-authenticated stream" [pmd3-hid][idv-hidcli].

`idevice` tested multi-touch on iOS 27.0 [idv#119]. The host's CoreDevice 642.16 names the same parts `[local]`:
- `DDIUniversalHIDService` and the `com.apple.coredevice.feature.remote.hid.*` and `.universalhidservice` features;
- the media-stream actions `mediastreamstart`, `mediastreamstop`, `mediastreamstatus` and `mediastreamgetsupportinfo`;
- an AVConference negotiation (`AVConferenceMediaStreamInitOptions`, `AVCMediaStreamNegotiator`).

The camera conflict above comes with the stream [dh-interact][pmd3-disp].

### `devicectl` in Xcode 27

`xcrun devicectl` 642.16 is a process per operation. Its JSON result file "will remain stable across releases" `[local]`, and its own start cost 60 to 70 ms on this host (`simulator.md`, the pasteboard). For a device it offers `[local]`:
- `device process launch` (environment, arguments, `--start-stopped`, `--terminate-existing`, `--activate`, a display), `terminate`, `signal`, `suspend`, `resume` and `openURL`;
- `device install` and `uninstall`;
- `device info`: `apps`, `processes`, `displays`, `lockState`, `ddiServices` and `details`;
- `device capture screenshot` (PNG) and `screen-record` (an `.mp4`, H.264 by default or HEVC);
- `pasteboard`, `orientation`, `notification`, `settings` (appearance, VoiceOver, reset) and `simulate` (location, status bar, biometrics).

It still has no tree and no input subcommand.

## What the existing tools do on a device

### WebDriverAgent and Appium's XCUITest driver

**How it runs.** WebDriverAgent 16.14.1 (2026-10-07) runs as `-[UITestingUITests testRunner]`, a "Never ending test used to start WebDriverAgent". The test starts its web server and spins the main run loop. Its `+setUp` sets `XCTDisableRemoteQueryEvaluation` and turns off interruption handling and automatic screenshots [wda-runner]. It serves HTTP with JSON bodies on port 8100 (scanning 100 ports unless `USE_PORT` names one), with MJPEG on 9100 [wda-conf]. Appium 12.16.0 reaches that port through a usbmuxd forward. On iOS 18 and later it uses its RemoteXPC port forwarder, over usbmux or to the tunnel address, and that RemoteXPC use needs `sudo appium driver run xcuitest tunnel-creation` [xcd-conn][xcd-rxpc].

**How it starts.**
- `xcodebuild build-for-testing test-without-building` with `DEVELOPMENT_TEAM`, `CODE_SIGN_IDENTITY` and `PRODUCT_BUNDLE_IDENTIFIER` [wda-xcodebuild].
- A prebuilt runner. It is built with `CODE_SIGNING_ALLOWED=NO` and its `Frameworks/*` excluded "so XCTest dependencies resolve from the real device's developer support libraries", and must be signed before install [wda-prebuilt].
- `usePreinstalledWDA` from iOS 17: the driver launches the runner through RemoteXPC DVT process control, in the background (`ActivateSuspended`). Below iOS 27 it falls back to `devicectl device process launch`; on 27 the fallback is off, since a foreground runner "never enters the background and never starts its HTTP server" [xcd-preinstalled].

**Defaults.** `waitForIdleTimeout` is 10 s and `animationCoolOffTimeout` 2 s. `snapshotMaxDepth` is 50: "The performance is acceptable for values up to 100", where XCTest's own default is unbounded. `maxTypingFrequency` is 60, and MJPEG runs at 10 frames a second at quality 25 [wda-conf][xcd-settings].

**The idle wait.** `+load` replaces `waitForQuiescenceIncludingAnimationsIdle:`. The replacement returns at once when the per-process flag is off or `waitForIdleTimeout` is 0, and otherwise runs the original under `_XCTSetApplicationStateTimeout`. WebDriverAgent's own `fb_waitUntilStableWithTimeout:` still follows actions and drags, so removing every wait takes both timeouts at 0 (inference) [wda-quiesce]. The driver removed its `waitForQuiescence` capability in 11.0.0: "WDA quiescence is fixed; tune idle behavior with appium:waitForIdleTimeout" [xcd-changelog].

**Private calls.**
- Snapshots: `-[XCUIElement snapshotWithError:]`, or `-[XCUIElementQuery uniqueMatchingSnapshotWithError:]`. WebDriverAgent swizzles `XCAXClient_iOS`'s default snapshot parameters (depth, children, array counts) and guards its `requestSnapshotForElement:attributes:parameters:error:` with a deadline. Its header says `XCAXClient_iOS` "makes its AX-server round trips in-process, bounded solely by AXTimeout", where `XCTRunnerAutomationSession` uses an XPC connection [wda-axproxy].
- Events: `XCPointerEventPath` paths in an `XCSynthesizedEventRecord`, sent with `[[XCUIDevice sharedDevice] eventSynthesizer] synthesizeEvent:completion:` [wda-events].
- Screenshots: `_XCT_requestScreenshot:withReply:` through `XCTRunnerDaemonSession`, with a 20 s timeout [wda-shot].

**Costs.** One report, on iOS 26.6 with WebDriverAgent 16.1.4 driven by go-ios [wda#1210]:
- `/status` took 4 ms, `/screenshot` 0.22 s and one gesture 1.04 s;
- routes that need accessibility data could hang;
- lowering `snapshotMaxDepth` to between 1 and 24 "gave faster and much less variable calls".

On a Simulator `/source` took 64 ms at 26 nodes, about 470 ms at 261 and 2.8 to 2.9 s at 1,029 [wda#1283]. Each MJPEG frame is a daemon screenshot with a 1 s timeout, at most 60 a second, and "there is no way for WDA to understand that the screen content is changed until we actually request a new screenshot" [wda-mjpeg][wda#360].

**Setup.** Appium's device guide asks for trust, Developer Mode, and "_Settings_ -> _Developer_ -> Turn _Enable UI Automation_ ON" [xcd-setup].

### Maestro

Maestro 2.11.0 (2026-09-29) throws "Physical iOS devices are not yet supported. Maestro runs iOS flows on simulators only (physical devices are supported on Android)." [mae-sess]. Pull request #3609, merged on 2026-09-18, added that guard [mae#3609]; the code's comment calls the device driver "an unfinished spike", and almost every method of `DeviceControlIOSDevice` is `TODO("Not yet implemented")` [mae-sess][mae-stub].

The code that exists builds the XCTest driver with `xcodebuild clean build-for-testing -allowProvisioningUpdates DEVELOPMENT_TEAM=<team> CODE_SIGN_IDENTITY="Apple Development"` [mae-build]. It installs and launches the driver with `devicectl device install app` and `devicectl device process launch --terminate-existing <runner>.xctrunner`, and talks HTTP to it at 127.0.0.1:22087 [mae-devctl]. An unmerged fix added `iproxy`, because the server listens on the device's loopback and "for physical devices, the port is unreachable without forwarding" [mae#3100]. The request for real devices has been open since January 2023 [mae#686]. DeviceLab's maestro-runner 1.1.28 runs Maestro flows on real iPhones through its own agent or WebDriverAgent, given `--team-id` [mae-runner].

### agent-device

agent-device 0.21.22 has a physical lane [ad-handoff][ad-launch][ad-shot][ad-usbmux][ad-mux]:
- its XCTest runner runs under an `xcodebuild` process against the device (its start timings name `build_xctestrun` and `launch_xcodebuild` phases); that process stays alive under a lease, and the next daemon adopts it;
- app lifecycle and screenshots go through `devicectl` (`device process launch`, `device capture screenshot`), with the runner's XCTest screenshot as the fallback;
- the runner's HTTP goes over usbmux `Connect` first, and to the CoreDevice tunnel address only for a Wi-Fi device, which usbmuxd does not list.

Measured on an iPhone 17 Pro with iOS 26.5.2 and Xcode 26.2 [ad-usbmux]:
- a snapshot took 440 ms over usbmux and 350 to 450 ms over the tunnel route;
- after 40 s idle the tunnel route took 4.5 to 4.6 s (its 30 s address cache expired and the session was set up again), while usbmux stayed at 440 ms;
- a killed runner came back in 4.0 to 4.7 s;
- the first open paid about 13 s of xctestrun preparation, and resolving the device through `devicectl` listing about 6.7 s;
- a locked phone wedged capture and relaunch on both routes, and "Auto-Lock must be Never during measurement or the runner dies mid-series".

### idb

idb's FBDeviceControl, at the 2026-10-08 head, assumes MobileDevice.framework is the "canonical" way to interact with iOS devices on macOS [idb-readme]. It launches apps through DVT below iOS 17 and through `devicectl` from 17 [idb-launch], and takes screenshots through `com.apple.mobile.screenshotr` [idb-shot]. For video it sets `kCMIOHardwarePropertyAllowScreenCaptureDevices` to 1 on the system object and polls `AVCaptureDevice(uniqueID: udid)` every 100 ms until a timeout. It then adds the device to an `AVCaptureSession` with an `AVCaptureVideoDataOutput` in 32BGRA, JPEG at quality 0.2, or H.264 [idb-video][idb-fmt]. Capture "requires that the hosting process … has system-level permissions" [idb-readme]. Its device commands include no accessibility and no input [idb-cmds].

### go-ios

go-ios (head of 2026-10-02, after 1.3.2) [goios-readme][goios-tun][goios-shot][goios-main]:
- needs `sudo ios tunnel start` for iOS 17 and later, or `--userspace` for a userspace TUN; its tunnel package implements the iOS 17.0 to 17.3 RemotePairing pairing and its tunnel ("In this package only QUIC is supported") next to lockdown's CoreDeviceProxy;
- offers `runwda`, `runtest` and `runxctest` through the test manager, `ax` on the audit service, `image auto`, `devmode`, and `ui` commands through WebDriverAgent or its own DeviceKit runner, whose H.264 stream WebDriverAgent lacks;
- takes `screenshot` through DVT as PNG; `--stream` serves MJPEG on port 3333, each frame a DVT screenshot decoded and re-encoded;
- `sign provision appstoreconnect` "Creates an iOS development signing certificate, P12, and provisioning profile through App Store Connect", and `sign app` re-signs a bundle with a P12 and a profile.

### pymobiledevice3

pymobiledevice3 11.26.0 (2026-10-08):
- the three tunnels above [pmd3-net];
- `mounter auto-mount`, which installs the Cryptex variant from iOS 17.4 [ddi-repo];
- `amfi` [pmd3-amfi];
- `developer dvt` and `developer accessibility` [pmd3-ax];
- the XCUITest service [pmd3-xcui];
- `developer core-device`: the app service, processes, lock state, display information, CoreDevice screenshots, Indigo buttons, universal-HID taps, drags, swipes and typing, and the display stream, served to a browser through WebCodecs or to a VNC client after `VTDecompressionSession` decodes it to BGRA [pmd3-cli][pmd3-stream][pmd3-vnc].

### Comparison

| Concern | WebDriverAgent (Appium) | Maestro 2.11.0 | agent-device 0.21.22 | idb | go-ios | pymobiledevice3 | Our plan |
|---|---|---|---|---|---|---|---|
| Runs on a device | yes | refused | yes | no tree or input | yes, through WDA or DeviceKit | yes, through WDA; HID through CoreDevice | yes |
| Runner | XCTest bundle, endless test | XCTest driver (unfinished) | XCTest runner under a kept `xcodebuild` | none | WDA or DeviceKit | WDA | Rust XCTest bundle, resident |
| Runner transport | HTTP and JSON, usbmuxd or tunnel | HTTP, 22087 | HTTP over usbmux, tunnel for Wi-Fi | n/a | HTTP | HTTP | CBOR over usbmuxd `Connect` |
| Runner start | `xcodebuild`, DVT process control, `devicectl` below 27 | `xcodebuild` or `devicectl` | `xcodebuild` | n/a | test manager | test manager | test manager through `idevice` |
| Lifecycle | XCTest or the runner | `devicectl` | `devicectl` | MobileDevice; DVT below 17, `devicectl` from 17 | DVT | DVT, CoreDevice | DVT or CoreDevice through `idevice` (S9) |
| Frames | MJPEG of daemon screenshots, 10 fps | n/a | `devicectl` screenshot | AVFoundation, screenshotr | DVT screenshots as MJPEG | display stream, screenshots | display stream or AVFoundation, hashed (S11) |
| Idle waits | swizzled; 10 s idle and 2 s cool-off by default | n/a | not read | n/a | WDA's | WDA's | none |

## The `idevice` crate and the alternatives

Context7's `/jkcoxson/idevice` matches the repository on the tunnel API (`CoreDeviceProxy` to a `CdTunnel` fed to jktcp or a TUN device). This section reads the crate at tag `v0.1.68`, its pull requests and issues, and crates.io.

**Version and shape.** Version 0.1.68 was published on 2026-09-15 under MIT. It uses edition 2024, declares no `rust-version`, and is async on tokio [idv-cargo]. Its README warns: "Breaking changes will happen at each point release until 0.2.0. Pin your `Cargo.toml` to a specific version" [idv-readme]. Every service is a Cargo feature. `default` holds only `aws-lc`, rustls on aws-lc-rs; `ring`, `openssl` and `rustcrypto` are the other TLS choices [idv-feat]. This workspace already uses tokio in `crates/android`.

| Feature | What it gives this change |
|---|---|
| `usbmuxd`, `pair` | Discovery, pairing records and device-port streams. The runner's socket fits here, as `idevice`'s WDA client reaches WDA's port through the provider's `connect` [idv-wda]. |
| lockdown (always built) | Values, sessions and service start. |
| `core_device_proxy`, `tunnel_tcp_stack` | The CoreDevice tunnel without a TUN device: CDTunnel framing over usbmuxd, handed to jktcp by `create_software_tunnel` with the MSS taken from the device's MTU [idv-cdp]. |
| `rsd`, `xpc` | RemoteXPC and the RSD handshake. |
| `mobile_image_mounter`, `cryptexd`, `tss` | The personalized DDI in either form, its nonce and manifest queries, and `query_developer_mode_status` [idv-mim]. |
| `amfi` | Revealing, enabling and accepting Developer Mode, and its status [idv-amfi]. |
| `installation_proxy` | Install, uninstall and lookup. |
| `dvt` | Process control, screenshots, device information and sysmontap [idv-pc][idv-shot]. |
| `xctest` | The test manager session above. It pulls in `dvt`, `installation_proxy`, `afc`, `tunnel_tcp_stack`, `rsd` and `core_device_proxy` [idv-feat]. |
| `wda` | A WebDriverAgent bootstrap client and a localhost bridge, "intentionally a bootstrap layer … rather than a complete long-lived WebDriver client" [idv-readme]. |
| `core_device` | The app service and the screen-capture service [idv-app][idv-scs]. |
| `display_stream` | The display stream and the HID back-channel, which share this feature [idv-cdmod]. |
| `springboardservices`, `screenshotr` | SpringBoard state; screenshots below iOS 17 and through the shim after [idv-sbs][idv-ssr]. |

**What it does not cover.** There is no accessibility audit client and no web inspector; the README lists the web inspector as unfinished. Its `wda` module is not a full WebDriver client [idv-readme]. Its TCP stack, jktcp 0.1.7, is "stop-and-wait": a segment is retransmitted "with exponential back-off (200 ms -> 400 ms -> … → 6.4 s) until acknowledged", out-of-order segments are dropped, and the design targets "reliable, ordered transports" such as CDTunnel. It also carries UDP datagrams [jktcp]. One user measured backups through it at 0.2 to 2.5 MB/s against 14 to 16 MB/s through a kernel TUN; the maintainer reported "well over 250mbps" in a test of their own and pushed a change [idv#102].

**Maturity.**
- 64 versions since 0.1.0 of 2025-01-10, about two a month in 2026; the `v0.1.66` and `v0.1.67` tags were never published to crates.io.
- 64,380 downloads, 19,929 in the last 90 days; six crates on crates.io depend on it.
- 525 stars, 25 contributors and 8 open issues and pull requests [idv-crates]. The README names StikDebug, CrossCode and Protokolle as users, "deployed on tens of thousands of devices" [idv-readme].
- The XCTest path came in #78 (2026-04-09), "validated against a live iOS 26.4 device"; runner environment overrides in #128, tested on iOS 17.5.1; the display stream and HID in #103 (2026-06-17); multi-touch in #119, "tested on iPhone 13 Pro Max running iOS 27.0"; Device Hub's keyboard service in #132 [idv#78][idv#128][idv#103][idv#119][idv#132].
- On iOS 27 the maintainer answers that "a large majority of StikDebug users" mount the DDI through it [idv#130].
- Open: a usbmuxd client panic on a short packet [idv#136]. Closed without an answer: one device's RSD list without `displayservice` [idv#131].

**Alternatives.**
- `ios-rs` 0.1.0 (2026-05-26) provides usbmux, lockdown, CDTunnel over smoltcp, RSD and an `xctest` feature, on the modern path from iOS 17.4 only; it has 4 stars and 30 downloads [iosrs].
- `rusty_libimobiledevice` 0.2.3 (2025-01-07) binds the C libimobiledevice, which has no CoreDevice tunnel; its own README recommends `idevice` instead [rlimd].
- `objc2-xc-ui-automation` 0.3.2 and `objc2-xc-test` 0.3.2 (2025-10-04) are generated from Xcode 26.0.1's SDK for every Apple target including `aarch64-apple-ios`. They set `link = false` because XCUIAutomation, a framework of its own since Xcode 16.3, is "introduced later than XCTest, so linking directly to this is inadvisable"; objc2's unreleased head moves to the Xcode 27.0 SDK [objc2-xcui][objc2-fw]. They cover the public surface (`XCUIApplication`, `XCUIElement`, `XCUICoordinate`, `XCUIDevice`, `XCUIScreen`) and not the private snapshot and synthesis calls, which stay dynamic message sends as in the Simulator reader.
- `objc2-av-foundation`, `objc2-core-media`, `objc2-core-media-io` and `objc2-video-toolbox` 0.3.2 cover the host side of either stream.

## Frame paths from the host

| Path | Format | Rate | Change signal | Costs and conditions |
|---|---|---|---|---|
| CoreDevice display stream | RTP over UDP, HEVC; decode to BGRA | 42 to 55 fps during swipes on iOS 27, paced by the host's per-frame receipts | lossy; a dropped frame corrupts the picture until an IDR, 100 to 300 ms after a request | camera and microphone unavailable to other apps; held open for HID input |
| AVFoundation through CoreMediaIO | H.264 from the phone, delivered as 32BGRA or re-encoded | 30 fps, also on a still screen | ~3% of bytes differ between frames of a still screen | the Camera permission of the responsible app; a phone matched by name on macOS 27 |
| QuickTime USB protocol (qvh) | H.264 and audio as CMSampleBuffers | not stated | not stated | libusb, a hidden USB configuration; on macOS AVFoundation likely reads the same stream (inference) |
| DVT screenshot | PNG | one per request | exact | Developer Mode and a mounted DDI |
| CoreDevice screen capture | PNG or JPEG | one per request | exact | RSD |
| XCTest screenshot (runner) | PNG, JPEG or HEIC | one per request; 0.22 s for WDA on iOS 26.6 | exact | the runner |

### The CoreDevice display stream

**Negotiation.** The service is `com.apple.coredevice.displayservice`, answered on the device by `dtremotedisplayd` [pmd3-disp]. The host sends `mediastreamstart` with an AVConference media negotiation blob, a compressed protobuf that names a UDP receiver on the tunnel. Video then arrives as plaintext RTP (cipher suite 0) carrying HEVC as payload type 100 [idv-ds][pmd3-stream].

**Rate.**
- The device honours the maximum bitrate the host advertises, and the encoder runs near 6 Mbps [pmd3-stream].
- On iOS 27.0, during 20 s of home-screen swipes, Xcode's own offer (`VRAE:0`) made the encoder drop about 11 frames a second and send about 42. Without it the encoder dropped none and sent 53 to 55 [pmd3-offer].
- The device "paces its output framerate" to RTCP receipts the host sends, one per frame [pmd3-stream].

**Loss.** The stream has a long GOP. After a dropped access unit, VideoToolbox "silently renders the mispredicted blocks as a mosaic tear". Clients therefore request an IDR with an RTCP picture-loss indication and wait "one PLI round-trip, ~100-300 ms"; a burst of such requests on a static screen can stall the encoder [pmd3-stream]. A consumer that blocks the receive loop loses datagrams, and with keyframes this rare "the picture never recovers" [idv#126].

**Decoding.** pymobiledevice3's VNC server decodes the stream with `VTDecompressionSession` straight to BGRA [pmd3-vnc], the shape the Simulator pipeline's `scale_bgra` takes.

**Open questions.** No source states whether the stream sends frames on a still screen, its latency from a change to a frame, or the host's CPU per frame.

### AVFoundation through CoreMediaIO's screen-capture allowance

**Opting in.** The macOS 27.0 SDK documents `kCMIOHardwarePropertyAllowScreenCaptureDevices` as "A UInt32 where 1 means that screen capture devices will be presented to the process … By default, this property is 1". It adds `kCMIOHardwarePropertyAllowWirelessScreenCaptureDevices`, 0 by default `[local]`. idb sets the allowance to 1 explicitly [idb-video].

**Finding the phone.** It appears as an `AVCaptureDevice` of media type muxed after a delay. QuickTime Player's movie recording uses the same path [stim#1689]. On macOS 27 the device's `uniqueID` was a random UUID and no CoreMediaIO property named the UDID. stim therefore checks the UDID among the USB serial numbers and opens the one capture device, or the one named like the phone when several are cabled [stim#1689]. idb's lookup by UDID [idb-video] predates that finding.

**Permission.** "The first capture triggers the macOS Camera prompt" for the process that runs the server. macOS attributes the request to the app that started that process, and denies it without a prompt when the app has no `NSCameraUsageDescription` or is hardened without `com.apple.security.device.camera` [stim#1689]. serve-sim ships its capture helper as a separate app, which its host signs with the camera entitlement alone and launches "responsible for itself" [serve-sim#1].

**Frames.** The output can be 32BGRA [idb-fmt]. The stream is lossy: "a still screen never repeats byte for byte (measured: ~3% of bytes differ, by up to 51) and the byte-exact check streamed 30 fps of an unchanged home screen". 8×8 averaged thumbnails with a threshold brought that to "0 frames per 2 s on a still screen" on an iPhone 15 Pro Max [serve-sim-still].

**Status bar.** quicktime_video_hack records video "in the Apple demonstration mode (Device shows 9:41am, full battery and no cellphone carrier in the status bar)" [qvh-readme]. If the AVFoundation path does the same, which is an inference, the clock would stop changing.

**Latency.** No source measured it.

### The QuickTime USB protocol

quicktime_video_hack (Go, libusb; last commit 2023-05-04) enables a hidden USB configuration with the control request `(0x40, 0x52, 0x00, 0x02)`. Its interface (subclass 0x2A) has four bulk endpoints, over which a clock-synchronised packet protocol carries CMSampleBuffers of H.264 and audio [qvh-tech]. It runs on macOS and Linux ("stable on MacOSX ... 16 hours straight on parallel devices"), not on Windows, and "probably does something similar to what `QuickTime` and `com.apple.cmio.iOSScreenCaptureAssistant` are doing on MacOS" [qvh-readme]. On macOS, AVFoundation reaches what is likely the same stream (an inference from that sentence) without claiming the USB device, so the plan's Rust port of it is not needed there.

### Screenshots

DVT's `takeScreenshot` returns PNG [idv-shot]. CoreDevice's screen-capture service returns PNG or JPEG [idv-scs]. screenshotr serves iOS below 17, plus a shim after [idv-ssr]. A runner can ask XCTest's daemon for PNG, JPEG or HEIC, which took WebDriverAgent 0.22 s on iOS 26.6 [wda-shot][wda#1210]. `devicectl device capture screenshot` writes a PNG at the cost of a process `[local]`. No source gives a DVT screenshot's latency on a device.

## Signing and provisioning

**What the runner needs.** A development-signed runner application holding the `.xctest` in `PlugIns`, as Xcode's `XCTRunner.app` template lays it out with `get-task-allow` `[local]`. It is signed with an Apple Development certificate and a development provisioning profile that lists the device.

**Xcode-managed signing.** `-allowProvisioningUpdates` lets `xcodebuild` "create and update profiles, app IDs, and certificates" for automatically signed targets. `-allowProvisioningDeviceRegistration` registers the destination device. `-authenticationKeyPath`, `-authenticationKeyID` and `-authenticationKeyIssuerID` authenticate with an App Store Connect key instead of an account signed in to Xcode [xcb-man]. Maestro's driver build is that form with `DEVELOPMENT_TEAM` and `CODE_SIGN_IDENTITY="Apple Development"` [mae-build]. WebDriverAgent's build passes `DEVELOPMENT_TEAM`, `CODE_SIGN_IDENTITY` and `PRODUCT_BUNDLE_IDENTIFIER` [wda-xcodebuild].

**Manual signing.** A prebuilt runner is re-signed with the team's identity and profile: Appium's prebuilt WebDriverAgent, and go-ios's `sign app` with a P12 and a profile [wda-prebuilt][goios-main].

**Free and paid teams.** A free account appears in Xcode as a Personal Team. It can register "up to 10 App IDs, which expire after 7 days" and "up to 3 devices", install "up to 3 apps per device", and its profiles "expire 7 days from issuance". Certificates, Identifiers & Profiles and App Store Connect belong to the paid program [memberships]. A free account can sign WebDriverAgent, but the user must then tap "Trust {email}" on the device [xcd-prov].

**The API.** With a paid team, the App Store Connect API creates a certificate from a signing request, registers a device and creates a profile [asc-cert][asc-device][asc-profile]. go-ios does all three [goios-main].

**On the device.** Pairing through the "Trust This Computer" dialog [dh-pair]; Developer Mode, confirmed with the passcode [devmode]; "Enable UI Automation" under Settings > Developer [xcd-setup]; the phone unlocked, with Auto-Lock off for long runs [ad-usbmux].

**What a script cannot do.**
- create the App Store Connect key or the account;
- accept the trust prompt or enter the passcode;
- enable Developer Mode on a phone with a passcode, since AMFI refuses [pmd3-amfi];
- turn on "Enable UI Automation", for which no surveyed tool has a command;
- trust a free account's developer;
- unlock the phone.

With a paid team and an API key, a script can do the rest: the certificate's key and signing request, the certificate, the device registration, the profile, the build, the signing, the install and the launch.

## The accessibility audit service

The service is `com.apple.accessibility.axAuditDaemon.remoteserver` through lockdown, with an RSD shim gated by `com.apple.mobile.lockdown.remote.trusted`. It speaks DTX, and go-ios presents it as "Accessibility inspector features" [pmd3-ax][pmd3-rxpc][goios-ax][goios-main]. It drives an inspector focus:
- the focus moves next, previous, first or last;
- each move publishes `hostInspectorCurrentElementChanged:` with the focused element's caption, spoken description and an opaque platform identifier;
- label, identifier and other attributes cost one request each;
- an element can be activated (`AXAction-2010`, "Activate"), but pymobiledevice3 notes a press "can only be used for processes that carry the `task_for_pid-allow` entitlement";
- the service also runs audits and changes accessibility settings.

pymobiledevice3 lists a screen by moving the focus until an element repeats, one move and one event per element, retrying after 1 s without an event [pmd3-ax]. go-ios exposes the same moves, attribute queries and actions [goios-ax]. It is navigation, not a tree: there is no hierarchy and no read of a whole screen in one request, and `idevice` does not implement it. At most it backs actions on elements XCTest misses, as design.md's Open Questions say.

## What this change reuses and where it differs

**Reused.**
- `idevice` for the transport, the developer services and the test manager session, with its tunnel kept in process [idv-feat].
- The XCTest session's sequence and runner layout from `idevice` and pymobiledevice3 [idv-xct][pmd3-xcui].
- WebDriverAgent's knowledge of what XCTest waits for, and its snapshot parameters [wda-quiesce][wda-axproxy].
- agent-device's usbmux-first runner route and its device findings (auto-lock, lock wedges, re-adoption) [ad-usbmux][ad-handoff].
- idb's CoreMediaIO setup [idb-video].
- pymobiledevice3's display-stream receipts, keyframe requests and VideoToolbox decoding [pmd3-stream][pmd3-vnc].
- The Simulator pipeline's downscale and hashes, with a tolerance added, and the Simulator reader's extraction (D21).

**Different.**
- One CBOR socket over usbmuxd instead of HTTP and JSON per node, with the runner in Rust.
- The tree read with one snapshot per request, as on the Simulator.
- No idle wait anywhere.
- The mirror re-reading windows on frame changes, with host-side input as a candidate next to the runner's.
- Nothing else on the host: no Node, Go or Python helper and no root tunnel daemon.

## Risks for the spikes

S9, transport and lifecycle:
- `idevice` has been tested on iOS 17.5.1, 26.4 and 27.0, not on 26.6.2 with Xcode 27's DDI [idv#78][idv#128][idv#119].
- The DDI may need the Cryptex1 path, and either path needs the network for TSS [ddi-repo].
- macOS's `remoted` and Device Hub hold their own session with the device. A userspace tunnel coexists with it in pymobiledevice3 [pmd3-net], but that is unverified for `idevice`.
- jktcp sends one segment at a time [jktcp]. Installs can stay off the tunnel, since `idevice`'s installation proxy and AFC clients are lockdown services over usbmuxd, but anything that does use the tunnel pays that.
- The open usbmuxd client panic [idv#136].
- No measured DVT launch or screenshot latency on a device exists in the sources.

S10, the runner:
- A Rust `cdylib` as the test bundle's executable is an `MH_DYLIB`, not the usual `MH_BUNDLE`. XCTest loads bundles through `NSBundle`, which uses `dlopen`, so it should load (inference); a static library linked into a bundle by the shim's link step is the fallback.
- WebDriverAgent's header says XCTest's accessibility round trips run in the runner's own process [wda-axproxy]. But the Simulator reader also asserts automation mode itself, while on a device XCTest's daemon offers `_XCT_enableAutomationModeWithReply:` `[local]`, and a sandboxed runner may be refused the direct call.
- Whether `synthesizeEvent:completion:` waits for quiescence on its own is unverified [wda-events].
- The device's auto-lock kills a runner [ad-usbmux].
- Appium's troubleshooting guide reports commands slowed by "an additional 60 seconds" when `testmanagerd` crashed [xcd-trouble].
- Device Hub's own control of a device may count as the one automation session; untested.

S11, frames:
- The display stream locks out the camera and microphone [dh-interact][pmd3-disp], and host-side input needs that stream running [pmd3-hid].
- HEVC decoding's CPU share and the stream's change-to-frame latency are unmeasured. Recovery after a loss takes 100 to 300 ms, longer than the 100 ms gate [pmd3-stream].
- The AVFoundation path needs camera permission attributed to whatever starts the daemon, sends 30 noisy frames a second on a still screen, and identifies the phone by name on macOS 27 [stim#1689][serve-sim-still].
- Neither stream gives an exact signal. D23's gate on frames without a pixel change needs a stated noise tolerance, which S11 measures.

## Spike plan

The measurements and gates are design D18 to D23's; the additions come from this note and are marked. Every timed operation runs at least 20 times on a quiet host, under the harness's guard with the load recorded. The device is plugged in, unlocked, with auto-lock off and the battery above half. The report's provenance records that, along with the iOS, Xcode, CoreDevice and DDI versions and `idevice`'s.

**S9, transport and lifecycle (D18 and D19; `docs/spikes/s9-ios-transport.md`).**
- List the device over usbmuxd. Read `ProductVersion`, the Developer Mode status and the lock state through lockdown with the host's pairing record, from an unprivileged process.
- Open the CoreDevice tunnel in process (`CoreDeviceProxy`, CDTunnel, jktcp) and do the RSD handshake; time both. Addition: run once with Device Hub and Xcode closed, and once with Device Hub showing the device.
- Check the DDI and mount it when missing; record which form mounted and the RSD services it adds. Addition: record whether `com.apple.coredevice.displayservice`, `.hid.indigo`, `.hid.universalhidservice` and `.screencaptureservice` are listed on iOS 26.6.2.
- Install the SwiftUI fixture built for the device through the installation proxy. Launch and terminate it through DVT process control (`launchSuspendedProcessWithDevicePath:…` with `KillExisting`, then `killPid:`), timed to the process's appearance. Addition: the same through CoreDevice's app service (`launchapplication`, `sendsignaltoprocess`), which `devicectl`'s process commands use by inference.
- Take a DVT screenshot (PNG), timed. Addition: CoreDevice's screen capture in PNG and in JPEG.
- Time `devicectl` for the same operations (`device process launch`, `terminate`, `capture screenshot`, `info lockState`), its process start included.
- Gate: the tunnel without root, a launch under 2 s at p95, a screenshot under 500 ms.

**S10, the runner (D20 to D22; `docs/spikes/s10-ios-runner.md`).**
- Build the minimum: an Objective-C `XCTestCase` subclass and the Rust agent in one Mach-O, placed as `PlugIns/<Name>.xctest` in a copy of Xcode's `XCTRunner.app` named `<Name>-Runner`. Sign it with the team from the environment through a minimal `xcodebuild` host project, with no XCTest frameworks embedded. Record the build and signing time, and whether the loader takes the `cdylib` or needs the bundle link.
- Start it through `idevice`'s `xctest` session. Record the cold start to the runner's hello over a usbmuxd `Connect` to its port, and the start with the bundle already installed.
- Read warm snapshots of the SwiftUI and Expo 500-cell screens with the Simulator reader's extraction (D21). Addition: record whether `userTestingSnapshotForElement:options:error:` works in the runner's own process on another app's pid, and time it against `XCAXClient_iOS -requestSnapshotForElement:attributes:parameters:error:`.
- Time tap dispatch through `XCUICoordinate` and through `synthesizeEvent:completion:` with the idle wait off. Addition: through `dtuhidd` with the display stream open, from the request to the report's send and to the first changed frame.
- Type the harness's 36 characters through the runner. Addition: through Device Hub's virtual keyboard service. For set text on the focused element where typing fails, addition: try `_XCT_performAccessibilityAction:onElement:withValue:reply:`.
- Take a screenshot through XCTest, against DVT's.
- Survival: 30 minutes of mixed commands without a restart, then one cable pull and replug.
- Addition: register for accessibility notifications through `_XCT_registerForAccessibilityNotification:reply:`, and count what arrives for a tap, a scroll and a screen change. If they arrive, the device's windows could send change events, which D21 sets false.
- Gates: tap dispatch under 150 ms at p95, a tree under 1 s at p95 on either screen, no restart in the session. Record the private call sites with their iOS and Xcode versions for the compatibility table.

**S11, frames (D23; `docs/spikes/s11-ios-frames.md`).**
- Candidates:
  - the display stream through `idevice`, decoded with VideoToolbox in the daemon's process to BGRA, then downscaled and hashed with `scale_bgra`, `pixels_hash` and `content_hash`;
  - AVFoundation capture through the CoreMediaIO allowance (`objc2-av-foundation`, `objc2-core-media`, `objc2-core-media-io`), through the same pipeline;
  - DVT and CoreDevice screenshots, as the on-demand path.
- Measure on the fixtures' list screen while S10's runner taps and scrolls:
  - frames per second, and the daemon's CPU share with decoding included;
  - frames lost or undecodable, with keyframe requests after a loss;
  - the distribution of the downscaled frames' differences on a still screen, the noise floor a tolerance must clear;
  - frames counted as changes on a still screen;
  - a tap's first changed frame after its synthesis, at p50 and p95.
- Addition: whether the display stream sends frames on a still screen; the first frame's latency after the stream starts; whether either path stops the camera and microphone for the fixture; what a call does to the stream.
- Gate: 30 frames per second sustained under 25% of a core, a tap's first changed frame within 100 ms at p95, and no frame without a change counted as one, the status bar's clock excluded and the noise tolerance stated.

## Local evidence

Read-only commands on the reference host; no device was attached to, listed or touched.
- Versions:
  - `xcodebuild -version` gives Xcode 27.0, 27A266a; `xcrun devicectl --version` and `plutil -p` on CoreDevice.framework's `Info.plist` give 642.16.
  - `plutil -p` on `/Library/Developer/DeveloperDiskImages/iOS_DDI/version.plist` shows the iOS DDI of Xcode 27.0 (27A266a) with XCTest 25228. Its `Restore/` holds two disk images with a trust cache each, and the cryptex metadata of one.
- `devicectl`: `--help`, `help device`, `help device capture` with `screenshot` and `screen-record`, `help device process` and `process launch`, `help device info` with `lockState` and `ddiServices`, `help device simulate` and `help device settings`.
- CoreDevice and Xcode frameworks, with `strings -a`:
  - on `CoreDevice.framework` and `CoreDeviceUtilities.framework` under `/Library/Developer/PrivateFrameworks`: the HID feature identifiers, `DDIUniversalHIDService`, the media-stream actions and the AVConference negotiation names quoted above;
  - on Xcode's `SharedFrameworks/XCTDaemonControl.framework`: `com.apple.dt.testmanagerd.remote.automation`, `com.apple.testmanagerd.remote-automation.lockdown.secure`, `XCTMessagingRole_UIAutomation` and "automation mode".
- The iPhoneOS platform under `/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/Library`:
  - The platform holds `XCTest.framework`, `XCUIAutomation.framework` (with public headers), `XCTAutomationSupport.framework`, `XCTestCore.framework`, `XCTestSupport.framework` and `XCUnit.framework`. The first four are each `platform IOS` and `minos 17.0` under `xcrun vtool -show-build`, at version 25228.
  - `nm -gU` on `XCTAutomationSupport` lists `_XCTDefaultSnapshotParameters`, `_XCAXAccessibilityAttributesForStringAttributes` and the classes `XCTAccessibilityFramework`, `XCAccessibilityElement` and `XCElementSnapshot`; `strings -a` finds `userTestingSnapshotForElement:options:error:`, `initForRemoteAccess` and `snapshotKeyHonorModalViews`.
  - `strings -a` on `XCUIAutomation` and `XCTestCore` finds `XCTRunnerDaemonSession`, `XCSynthesizedEventRecord`, `XCPointerEventPath`, `XCAXClient_iOS`, `requestSnapshotForElement:attributes:parameters:error:`, `waitForQuiescenceIncludingAnimationsIdle:` and the `_XCT_` selectors quoted above.
  - The reader's AXRuntime and libAccessibility calls live in the device's OS and were not checkable here.
- `Library/Xcode/Agents/XCTRunner.app` in the same platform: `MinimumOSVersion` 17.0, `CFBundleExecutable` set to `$(WRAPPEDPRODUCTNAME)`, a `RunnerEntitlements.plist` holding only `get-task-allow`, and an executable that links `@rpath/XCTest.framework`.
- CoreMediaIO: `CMIOHardwareSystem.h` in the macOS 27.0 SDK, for the allowance and its wireless variant.
- `man xcodebuild`, for the provisioning and authentication-key options.

## Sources

Read in the repositories unless marked Context7. Pinned commits: `idevice` `v0.1.68` (`d32c818`), jktcp `216d796`, pymobiledevice3 `v11.26.0` (`c0d5f47`), DeveloperDiskImage `6eae353`, go-ios `273d3e0`, WebDriverAgent `v16.14.1` (`2e98ac0`), appium-xcuitest-driver `v12.16.0` (`f528f59`), Maestro `cli-2.11.0` (`c8c361b`), maestro-runner `v1.1.28` (`6e97e8d`), agent-device `v0.21.22` (`2025852`), idb `cfe8f3f`, quicktime_video_hack `d81396e`, serve-sim `aeba1a1`, objc2 `3c521cb`, `ios-rs` `637d18d`, `rusty_libimobiledevice` `80253ad`.

`idevice` (Context7 `/jkcoxson/idevice` for the tunnel API; the rest from the repository):
- [idv-readme] https://github.com/jkcoxson/idevice/blob/v0.1.68/README.md
- [idv-cargo] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/Cargo.toml
- [idv-feat] https://docs.rs/crate/idevice/0.1.68/features and https://crates.io/api/v1/crates/idevice/0.1.68
- [idv-crates] https://crates.io/crates/idevice (versions, downloads, reverse dependencies) and https://github.com/jkcoxson/idevice
- [idv-mux] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/usbmuxd/mod.rs#L85-L121 and #L240-L293
- [idv-cdp] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/core_device_proxy.rs
- [idv-tunnel] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/tunnel.rs
- [idv-mim] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/mobile_image_mounter.rs
- [idv-amfi] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/amfi.rs#L32-L51
- [idv-pc] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/dvt/process_control.rs#L156-L170
- [idv-shot] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/dvt/screenshot.rs#L54-L66
- [idv-ssr] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/screenshotr.rs
- [idv-sbs] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/springboardservices.rs
- [idv-app] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/core_device/app_service.rs
- [idv-scs] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/core_device/screencaptureservices.rs
- [idv-cdmod] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/core_device/mod.rs#L13-L40
- [idv-ds] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/core_device/display_stream/mod.rs#L1-L14
- [idv-hid] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/core_device/hid.rs#L1-L11, #L495-L507 and #L860-L866
- [idv-hidcli] https://github.com/jkcoxson/idevice/blob/v0.1.68/tools/src/hid.rs#L421-L449 and #L577-L579
- [idv-xct] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/dvt/xctest/mod.rs (#L1-L8, #L104-L235, #L247-L291, #L316-L381, #L1979-L2063)
- [idv-dtx] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/dvt/xctest/dtx_services.rs#L10-L61
- [idv-wda] https://github.com/jkcoxson/idevice/blob/v0.1.68/idevice/src/services/wda.rs#L1-L10
- [idv#78] https://github.com/jkcoxson/idevice/pull/78, [idv#102] https://github.com/jkcoxson/idevice/issues/102, [idv#103] https://github.com/jkcoxson/idevice/pull/103, [idv#119] https://github.com/jkcoxson/idevice/pull/119, [idv#126] https://github.com/jkcoxson/idevice/issues/126, [idv#128] https://github.com/jkcoxson/idevice/pull/128, [idv#130] https://github.com/jkcoxson/idevice/issues/130, [idv#131] https://github.com/jkcoxson/idevice/issues/131, [idv#132] https://github.com/jkcoxson/idevice/pull/132, [idv#136] https://github.com/jkcoxson/idevice/issues/136
- [jktcp] https://github.com/jkcoxson/jktcp/blob/216d796f8e5260ab682a9445ea8257899d16fdf7/README.md and https://crates.io/crates/jktcp (0.1.7)

pymobiledevice3 (Context7 `/doronz88/pymobiledevice3` for the tunnel guide, the mount note and the screenshot prerequisites; the rest from the repository):
- [pmd3-net] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/docs/guides/network-stacks.md#L39-L131 and #L211-L306
- [pmd3-rxpc] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/misc/RemoteXPC.md#L442-L443, #L526-L527, #L643-L652 and #L680-L681
- [pmd3-amfi] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/pymobiledevice3/services/amfi.py#L36-L70
- [pmd3-ax] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/pymobiledevice3/services/accessibilityaudit.py#L277-L302, #L504-L520 and #L627-L680
- [pmd3-xcui] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/pymobiledevice3/services/dvt/testmanaged/xcuitest.py#L237-L375
- [pmd3-disp] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/pymobiledevice3/remote/core_device/display_service.py#L15-L35 and #L53
- [pmd3-stream] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/pymobiledevice3/remote/core_device/screen_stream.py#L1-L11, #L664-L700, #L940-L962, #L1036-L1050 and #L1080-L1093
- [pmd3-offer] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/pymobiledevice3/remote/core_device/media_stream_offer.py#L128-L152
- [pmd3-hid] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/pymobiledevice3/remote/core_device/hid_service.py#L1-L18 and #L30-L109
- [pmd3-vnc] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/pymobiledevice3/remote/core_device/vnc_server.py#L1-L21
- [pmd3-cli] https://github.com/doronz88/pymobiledevice3/blob/v11.26.0/pymobiledevice3/cli/developer/core_device.py
- [ddi-repo] https://github.com/doronz88/DeveloperDiskImage/blob/6eae353ae694bda1c421d4a3eee5459ae59c99a1/README.md#L5-L45

go-ios (Context7 `/danielpaulus/go-ios` for the README and the userspace-tunnel API; the CLI from the repository):
- [goios-readme] https://github.com/danielpaulus/go-ios/blob/273d3e06e803fb6ee95e4df914d8be82c5ee4bb0/README.md#L12
- [goios-main] https://github.com/danielpaulus/go-ios/blob/273d3e06e803fb6ee95e4df914d8be82c5ee4bb0/main.go#L139-L181, #L484-L532 and #L586-L600
- [goios-tun] https://github.com/danielpaulus/go-ios/blob/273d3e06e803fb6ee95e4df914d8be82c5ee4bb0/ios/tunnel/doc.go
- [goios-ax] https://github.com/danielpaulus/go-ios/blob/273d3e06e803fb6ee95e4df914d8be82c5ee4bb0/ios/accessibility/accessibility.go#L10 and https://github.com/danielpaulus/go-ios/blob/273d3e06e803fb6ee95e4df914d8be82c5ee4bb0/ios/accessibility/accessibility_control.go#L54-L70 and #L233-L456
- [goios-shot] https://github.com/danielpaulus/go-ios/blob/273d3e06e803fb6ee95e4df914d8be82c5ee4bb0/ios/instruments/screenshot.go#L18-L82

WebDriverAgent (Context7 indexes only the archived facebook fork, so all from the repository) and Appium's XCUITest driver (Context7 for the settings and capabilities, checked in the repository):
- [wda-runner] https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentRunner/UITestingUITests.m#L22-L49
- [wda-conf] https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Utilities/FBConfiguration.m#L26-L174 and https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Utilities/FBConfiguration.h#L142-L235
- [wda-quiesce] https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Categories/XCUIApplicationProcess+FBQuiescence.m#L22-L95
- [wda-axproxy] https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Utilities/FBXCAXClientProxy.h#L54-L157 and https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Categories/XCAXClient_iOS+FBSnapshotReqParams.m#L20-L177
- [wda-events] https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Utilities/FBXCTestDaemonsProxy.m#L106-L131 and https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Utilities/FBW3CActionsSynthesizer.m#L850-L899
- [wda-shot] https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Utilities/FBXCTestDaemonsProxy.m#L84-L104 and https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Utilities/FBScreenshot.m#L24-L229
- [wda-mjpeg] https://github.com/appium/WebDriverAgent/blob/v16.14.1/WebDriverAgentLib/Utilities/FBMjpegServer.m#L24-L216
- [wda-xcodebuild] https://github.com/appium/WebDriverAgent/blob/v16.14.1/lib/xcodebuild.ts#L408-L513
- [wda-prebuilt] https://github.com/appium/WebDriverAgent/blob/v16.14.1/Scripts/ci/build-real.sh#L3-L18 and https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/docs/getting-started/provisioning-profile/generic-device-config.md#L34-L57
- [wda#1210] https://github.com/appium/WebDriverAgent/issues/1210, [wda#1283] https://github.com/appium/WebDriverAgent/pull/1283, [wda#360] https://github.com/appium/WebDriverAgent/issues/360#issuecomment-654324309
- [xcd-settings] https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/docs/reference/settings.md
- [xcd-setup] https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/docs/getting-started/device-setup.md#L38-L72
- [xcd-prov] https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/docs/getting-started/provisioning-profile/index.md#L85-L105
- [xcd-preinstalled] https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/docs/guides/run-preinstalled-wda.md#L54-L63 and https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/lib/device/wda-host-ops.ts#L166-L214
- [xcd-rxpc] https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/docs/guides/remotexpc-tunnels-real-devices.md#L9-L13 and #L83-L110
- [xcd-conn] https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/lib/device/device-connections-factory.ts#L57-L100
- [xcd-changelog] https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/CHANGELOG.md#L636-L654
- [xcd-trouble] https://github.com/appium/appium-xcuitest-driver/blob/v12.16.0/docs/troubleshooting/index.md#L129-L139

Maestro and maestro-runner:
- [mae-sess] https://github.com/mobile-dev-inc/maestro/blob/cli-2.11.0/maestro-cli/src/main/java/maestro/cli/session/MaestroSessionManager.kt#L156-L166
- [mae-stub] https://github.com/mobile-dev-inc/maestro/blob/cli-2.11.0/maestro-ios/src/main/java/ios/devicectl/DeviceControlIOSDevice.kt#L20-L70
- [mae-build] https://github.com/mobile-dev-inc/maestro/blob/cli-2.11.0/maestro-cli/src/main/java/maestro/cli/driver/DriverBuilder.kt#L64-L78
- [mae-devctl] https://github.com/mobile-dev-inc/maestro/blob/cli-2.11.0/maestro-ios-driver/src/main/kotlin/util/LocalIOSDeviceController.kt and https://github.com/mobile-dev-inc/maestro/blob/cli-2.11.0/maestro-cli/src/main/java/maestro/cli/session/MaestroSessionManager.kt#L58
- [mae#3609] https://github.com/mobile-dev-inc/maestro/pull/3609, [mae#3100] https://github.com/mobile-dev-inc/maestro/pull/3100, [mae#686] https://github.com/mobile-dev-inc/maestro/issues/686
- [mae-runner] https://github.com/devicelab-dev/maestro-runner/blob/v1.1.28/README.md#L100-L110

agent-device:
- [ad-usbmux] https://github.com/callstack/agent-device/blob/v0.21.22/docs/usbmux-runner-transport-experiment.md
- [ad-handoff] https://github.com/callstack/agent-device/blob/v0.21.22/docs/evidence/ios-physical-runner-handoff-2026-09-19.md
- [ad-shot] https://github.com/callstack/agent-device/blob/v0.21.22/packages/platform-apple/src/core/physical-device-screenshot.ts#L18-L30
- [ad-launch] https://github.com/callstack/agent-device/blob/v0.21.22/packages/platform-apple/src/core/physical-device-coredevice.ts#L25-L35
- [ad-mux] https://github.com/callstack/agent-device/blob/v0.21.22/packages/platform-apple/src/runner/runner-usbmux.ts#L1-L25

idb:
- [idb-readme] https://github.com/facebook/idb/blob/cfe8f3f6a487387108d227af0270447e4e0b8328/FBDeviceControl/README.md#L11-L21 and #L126-L132
- [idb-launch] https://github.com/facebook/idb/blob/cfe8f3f6a487387108d227af0270447e4e0b8328/FBDeviceControl/Commands/DeviceApplicationCommands.swift#L259-L265
- [idb-shot] https://github.com/facebook/idb/blob/cfe8f3f6a487387108d227af0270447e4e0b8328/FBDeviceControl/Commands/DeviceScreenshotCommands.swift#L52-L53
- [idb-video] https://github.com/facebook/idb/blob/cfe8f3f6a487387108d227af0270447e4e0b8328/FBDeviceControl/Video/DeviceVideo.swift#L38-L80
- [idb-fmt] https://github.com/facebook/idb/blob/cfe8f3f6a487387108d227af0270447e4e0b8328/FBDeviceControl/Video/DeviceCaptureFormat.swift#L36-L56
- [idb-cmds] https://github.com/facebook/idb/tree/cfe8f3f6a487387108d227af0270447e4e0b8328/FBDeviceControl/Commands

Frames:
- [qvh-readme] https://github.com/danielpaulus/quicktime_video_hack/blob/d81396e2e7758d98c2a594853b64f98b54a8a871/README.md
- [qvh-tech] https://github.com/danielpaulus/quicktime_video_hack/blob/d81396e2e7758d98c2a594853b64f98b54a8a871/doc/technical_documentation.md#L104-L138
- [stim#1689] https://github.com/appandflow/stim/issues/1689 (the issue and its comment of 2026-09-28)
- [serve-sim#1] https://github.com/nhtera/serve-sim/pull/1
- [serve-sim-still] https://github.com/nhtera/serve-sim/commit/aeba1a1395709261c87a8a46f89a0ae4e3112574

Crates:
- [iosrs] https://github.com/devicelink/ios-rs/blob/637d18d8a364565bea730b40f2ee74dc0251b301/README.md
- [rlimd] https://github.com/jkcoxson/rusty_libimobiledevice/blob/80253ad5aebc58f1fa6367f4583b24d01bb72452/README.md
- [objc2-xcui] https://github.com/madsmtm/objc2/blob/3c521cb865bed9300de2797482e432aae0513f0d/framework-crates/objc2-xc-ui-automation/translation-config.toml and https://docs.rs/crate/objc2-xc-ui-automation/0.3.2
- [objc2-fw] https://github.com/madsmtm/objc2/blob/3c521cb865bed9300de2797482e432aae0513f0d/docs/FRAMEWORKS_CHANGELOG.md#L13-L39 and #L115-L150

Apple (documentation pages read through developer.apple.com's documentation JSON on 2026-10-08; live pages, not pinned):
- [xc27-rn] Xcode 27 release notes, Device Hub and devicectl: https://developer.apple.com/documentation/xcode-release-notes/xcode-27-release-notes
- [wwdc26-260] Get the most out of Device Hub: https://developer.apple.com/videos/play/wwdc2026/260/
- [dh-pair] https://developer.apple.com/documentation/xcode/managing-your-simulated-and-physical-devices-in-device-hub
- [dh-interact] https://developer.apple.com/documentation/xcode/interacting-with-your-app-in-device-hub
- [devmode] https://developer.apple.com/documentation/xcode/enabling-developer-mode-on-a-device
- [memberships] https://developer.apple.com/support/compare-memberships/
- [asc-cert] https://developer.apple.com/documentation/appstoreconnectapi/post-v1-certificates, [asc-device] https://developer.apple.com/documentation/appstoreconnectapi/post-v1-devices, [asc-profile] https://developer.apple.com/documentation/appstoreconnectapi/post-v1-profiles
- [xcb-man] the `xcodebuild` manual page of Xcode 27.0 (`man xcodebuild`, read locally; no URL)
