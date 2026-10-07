# Simulator driver: state of the art

Research note for task 2.2 of `mobile-automation-foundation`. It records how existing tools drive the iOS Simulator from the host (lifecycle, HID input, accessibility trees, frames), what spikes S2 (lifecycle and frames), S3 (HID input), S4 (accessibility) and the `crates/simulator` group should reuse, and where this design differs. Repositories were read at the commits pinned under [Sources](#sources) on 2026-09-26. Claims tagged `[local]` were checked read-only on the reference host with the commands under [Local evidence](#local-evidence).

Reference host: macOS 26.6.2 (25G83), Xcode 27.0 (27A266a), CoreSimulator 1171.7, SimulatorKit 1005.2, one runtime iOS 26.5 (23F77), an iPhone 17 booted while DeviceHub.app was not running `[local]`.

## Verdict for the reference host

- **Input: the dtuhidd XPC transport is expected to work and should be the default.** The host CoreSimulator (1171.7) is newer than 1155.4, the first version that injects `dtuhidd` into the guest [idb-sel]. The daemon and its launchd job ship inside the host framework, not in the iOS 26.5 runtime root, and vend `com.apple.coredevice.feature.remote.hid.digitizer`, `.button`, `.keyboard`, `.vendordefined`, `...remote.universalhid` and `...remote.universalhidservice` `[local]`. The private `xpc_endpoint_create_mach_port_4sim` and `xpc_connection_enable_sim2host_4sim` are exported by this macOS's libxpc `[local]`. iosef delivered taps, swipes and typed text through dtuhidd on iOS 26.4 and 26.5 under Xcode 27.0 with CoreSimulator 1171.6 [iosef#15]; stim did so on Xcode 27.0 (27A266a, this build) with iOS 27.0 and iOS 18.6 guests [stim#1326].
- **The legacy Indigo transport is expected to be unreliable for touch, dead for keys and alive for hardware buttons.** Whether Indigo touch reaches backboardd on a boot is a race with dtuhidd, and once dtuhidd has been activated (by DeviceHub.app, another tool, or our own first dtuhidd connection) Indigo touch stays dead until reboot [iosef#15][stim#1326]. From CoreSimulator 1155.4 Indigo keyboard events "are delivered byte-correctly and then dropped" while Indigo buttons keep working [idb-sel]; iosef relies on that for iOS 26.4 and 26.5 [iosef-comp], while baguette and stim found Indigo buttons dead on iOS 27.0 guests [bag-hub][stim#1326]. Every Indigo builder is still exported by Xcode 27's SimulatorKit `[local]`.
- **Accessibility: the host translator path is the only one that meets the spec here; the fast snapshot API needs a process inside the guest.** The host path is `AXPTranslator` from `/System/Library/PrivateFrameworks/AccessibilityPlatformTranslation.framework`, answered through `-[SimDevice sendAccessibilityRequestAsync:completionQueue:completionHandler:]`; both exist on this host `[local]`. Under Xcode 27 with iOS 26.4 and 26.5 it returned a 62-element tree after the `com.apple.Accessibility` defaults were written and `com.apple.CoreSimulator.bridge` was restarted once [iosef#14]. It pays one XPC round trip per element and attribute and reports composed views as single elements [idb-axdoc]; a report of a 1-element tree on iOS 26.5 is open upstream [idb#964]. The snapshot API (`XCTAccessibilityFramework -userTestingSnapshotForElement:options:error:`) exists in this runtime, but its image is built for platform `IOSSIMULATOR` `[local]`, and dyld refuses an image whose `LC_BUILD_VERSION` names another platform, which is why idb ships one guest per platform [idb-sfb]; the macOS daemon therefore cannot load it, and agent-device and idb reach it only from a helper compiled for and spawned inside the Simulator [ad-spawn][idb-sfb].
- **Frames: CoreSimDeviceIO's SimScreen callbacks.** `registerScreenCallbacksWithUUID:callbackQueue:frameCallback:surfacesChangedCallback:propertiesChangedCallback:`, the older `registerCallbackWithUUID:ioSurfacesChangeCallback:` and `registerCallbackWithUUID:damageRectanglesCallback:`, and `framebufferSurface` are all present in CoreSimDeviceIO 1171.7 `[local]`; idb prefers the SimScreen form on Xcode 27 [idb-fb].

### Consequences for design.md, to settle in task 2.11

This note does not edit design.md; these are the points 2.11 has to resolve.

1. D6 chooses the transport "per booted Simulator by probing once". No probe is free of side effects: dtuhidd is demand-launched by the first message to its service [idb-dtu] and publishes `com.apple.coredevice.dtuhidd.active` on start, which disconnects the legacy services for that boot [bag-hub][stim#1326]. The sources select by host CoreSimulator version and fall back to Indigo only when dtuhidd is unreachable [idb-neg][iosef-comp]. With CoreSimulator 1155.4 or later every guest tested (iOS 18.6, 26.4, 26.5, 27.0) gets dtuhidd [stim#1326][iosef#15], so the spec's "Mixed runtimes" scenario yields one transport per host, not per runtime.
2. D6 reads the tree through "an in-process bridge ... ported from the idb-derived Objective-C reader". That reader is a guest process: agent-device compiles it with `xcrun --sdk iphonesimulator clang` and starts it with `xcrun simctl spawn` [ad-spawn]. The simulator-driver spec forbids "launching any test runner or helper inside" the Simulator. S4 must time the host translator path against the gates; if it misses them, the choice is between allowing a transient spawned reader (nothing installed) and accepting the translator's cost and fidelity.
3. Task 2.6 asks for the Mach transport "on an iOS 26 runtime" and XPC "on an Xcode 27 runtime". This host has only CoreSimulator 1171.7, so both halves run against iOS 26.5; the Indigo half must run first on a fresh boot with DeviceHub.app closed, and repeated Indigo runs need a reboot in between.

## Prior art

### idb and FBSimulatorControl (Swift since 2026, MIT)

- Loading: CoreSimulator comes from `/Library/Developer/PrivateFrameworks/CoreSimulator.framework`, a system framework the Xcode installer overwrites, so behaviour is keyed on the loaded bundle's `CFBundleVersion` rather than the selected Xcode [idb-sel]. SimulatorKit comes from `$DEVELOPER_DIR/../SharedFrameworks` on Xcode 27 and `$DEVELOPER_DIR/Library/PrivateFrameworks` on Xcode 26 and earlier; `AccessibilityPlatformTranslation` from `/System/Library/PrivateFrameworks`; loading refuses root and checks the bundle path [idb-load]. Private classes are looked up by name with `objc_lookUpClass`, never linked, because a link-time class reference breaks when Apple moves the class; every send runs under an exception guard because an `NSException` unwinding through Swift aborts the process [idb-load][idb-client].
- Negotiation: DTUHID when CoreSimulator is 1155.4 or later, Indigo otherwise; a requested transport is never substituted and only an unreachable dtuhidd falls back. Indigo and DTUHID "both claim `mainTouchscreen` and whichever sends first on a boot keeps it", so they are mixed only on tvOS, which has no touchscreen [idb-sel][idb-neg].
- Indigo: `SimulatorKit.SimDeviceLegacyHIDClient` with `-initWithDevice:error:` and `-sendWithMessage:freeWhenDone:completionQueue:completion:`, handed a `malloc`ed message the client frees [idb-client]. Builders are resolved with `dlsym`: touch through `IndigoHIDMessageForMouseNSEvent(point, point2 or NULL, 0x32, NSEventType, NSSize {1, 1}, edge)` with the ratios rewritten afterwards and single touch re-enveloped by hand; buttons through `IndigoHIDMessageForButton(source, op, 0x33)`; Consumer-page buttons through `IndigoHIDMessageForHIDArbitrary(0x32, page, usage, op)`; keys through `IndigoHIDMessageForKeyboardArbitrary(usage, op)`, whose keyboard target is 0x64 [idb-indigo][idb-h][stim#1326]. The older 5-parameter binding worked on arm64 because its last argument landed in the edge register as 0 and the uninitialized `NSSize` only skewed ratios that idb overwrote [idb-02eb93c].
- DTUHID: `-[SimDevice lookup:error:]` for `com.apple.coredevice.feature.remote.hid.digitizer`, `xpc_endpoint_create_mach_port_4sim(port, 0, 0)`, `xpc_connection_create_from_endpoint`, then `xpc_connection_enable_sim2host_4sim`, "without which the service observes the peer but never a payload" [idb-xpc]. Messages are XPC dictionaries `{messageType, isBarrier, featureIdentifier, payload}`: `IndigoDigitizerEvent {pointOne {x, y}, pointTwo?, eventType, edge, target}` with `eventType` 0 start, 1 position, 2 end; `IndigoKeyboardButtonEvent {usageCode, state}`; `IndigoButtonEvent {usagePage, usageCode, state}`. Integers travel as `uint64`, coordinates as `double` ratios from the top left, and `state` is 1 for down and 2 for up because 0 is rejected [idb-dtu]. Liveness is a barrier `IndigoKeyboardButtonEvent` with usage 0 and a 4 s reply deadline followed by a 200 ms tail, with up to five attempts 4 s apart because dtuhidd has a 10 s minimum runtime and aborts when started before a display exists; an 80 ms drain precedes disconnect. A second connection to `...hid.vendordefined` carries hinge and orientation reports [idb-dtu].
- Framebuffer: `SimDevice.io`, then `ioPorts`, then the port whose `descriptor` (a ROCKit remote proxy) conforms to `SimDisplayIOSurfaceRenderable` and `SimDisplayRenderable` and reports `state.displayClass == 0`. SimScreen callbacks are preferred, delivered on a dedicated serial `.userInteractive` queue and kept minimal because the render server invokes them with `clientQueue.sync`; the fallback registers the surface-change callbacks plus `damageRectanglesCallback`, which on modern CoreSimulator is a bare per-frame signal. Since Xcode 27 these protocols live in CoreSimDeviceIO. Images are rendered with `CIImage(ioSurface:)` and a reused `CIContext` [idb-fb].
- Accessibility: `--api ax` runs on the host, where the `AXPTranslator` singleton's token delegate turns each lazy attribute fetch into a `sendAccessibilityRequestAsync` round trip, serialized on one process-wide queue because the singleton is not thread-safe [idb-ax]. `--api axbridge` spawns the `SimulatorFrameworkBridge` guest tool and reads the whole tree in one call, roughly 30 ms on an idle screen, asserting `AutomationEnabled` on every read, which costs about 1.8 times the wall time for about 15 percent more elements on a large app [idb-sfb][idb-axdoc].
- `CoreDevice/`: guest CoreDevice features over the same sim2host XPC, namely `getdisplayinfo`, `monitormotion`, `remote.devicecontrol.orientation` and `remote.universalhidservice` for mapping touchscreens to displays [idb-cd]. CoreDevice's own Swift client needs the `com.apple.private.CoreDevice.hid` entitlement, which the direct XPC route avoids [idb-cf91155].
- Reuse: the selection rule, timing constants, wire models, loader paths, framebuffer registration order and exception discipline. Differs: one Rust daemon with no gRPC companion, no guest bridge, and the translator treated as a resource shared by every target.

### baguette (Swift, MIT)

- Says iOS 26 broke the call idb and AXe used and that Xcode 26 exposes a "9-argument" `IndigoHIDMessageForMouseNSEvent` routing touches to digitizer target 0x32 [bag-arch]. Xcode 27's SimulatorKit embeds the prototype `IndigoHIDMessageForMouseNSEvent(CGPoint *, CGPoint *, IndigoHIDTarget, NSEventType, NSSize, IndigoHIDEdge)`; the function saves x0 to x4 and d0/d1 and bounds-checks x4 against 4, the edge `[local]`. On arm64, idb's 6-parameter declaration, baguette's "real 7-arg" one and its 9-argument one all map onto those registers; the 9-argument call puts its direction value in x4, the edge slot [bag-hid], which may be why it saw taps misrouted (inference).
- Taps, swipes and edge gestures therefore go through a real `IOHIDEvent`: an `IOHIDEventCreateDigitizerEvent` parent with an `IOHIDEventCreateDigitizerFingerEvent` child joined by `IOHIDEventAppendEvent`, converted with `IndigoHIDMessageForTrackpadEventFromHIDEventRef`, then target 0x32 patched at offsets 0x6c and 0x10c and edge bits at 0x3a/0x3b and 0xda/0xdb; verified on iOS 26.4 with Xcode 26 [bag-iohid]. The Xcode 27 prototype of that converter takes `(IOHIDEventRef, IndigoHIDTarget)` `[local]`, while baguette binds one argument and patches the target afterwards [bag-iohid]. Keys go as HID usages to 0x32 through `IndigoHIDMessageForHIDArbitrary` [bag-hid].
- Process model: spawning costs "~1.2 s (framework resolution)" and the Indigo pipeline "a ~40 ms per-session warmup" (pointer and mouse services created on first use), so hosts keep one `baguette input --udid` per device, and warm gestures take "a few milliseconds" [bag-arch][bag-hid]. It says the mouse builder reads AppKit thread-local state and must run on the main actor [bag-arch]; Xcode 27's builder calls only `calloc`, `mach_absolute_time` and `free` but keeps the process-global statics `had_second_point`, `last_second_point` and `last_timestamp` `[local]`, and stim reports it returns nil for a drag within 16 ms of the previous message [stim-hid].
- Xcode 27: DeviceHub.app, which replaced Simulator.app, attaches dtuhidd; dtuhidd publishes `com.apple.coredevice.dtuhidd.active`, backboardd's SimulatorHID then disconnects `ExternalKeyboardService`, `MainScreenButtonsService` and the main `ScreenTouchService`, and on iOS 27 re-adds a dead service on the next Indigo event. `baguette heal` clears the notify state and then runs `launchctl kickstart -k system/com.apple.backboardd`, which kills running apps; verified on iOS 27.0 (24A434) [bag-hub].
- Frames: per-present callbacks on every display descriptor, captures coalesced and an autorelease pool drained per capture because `framebufferSurface` is a synchronous XPC round trip through ROCKit [bag-screen]; MJPEG through ImageIO `CGImageDestination`, H.264 through `VTCompressionSession`, streams advertised at 60 fps [bag-arch].
- Accessibility: the host translator walk plus a 32 pt hit-test sweep, because the walk misses the status bar and items inside childless SwiftUI containers; the sweep adds hundreds of XPC round trips and 1.5 to 2 s per read [bag-sweep].
- Reuse: edge semantics, one warm-up per session, capture coalescing, encoder candidates. Differs: we move to dtuhidd on Xcode 27 instead of restarting backboardd to keep Indigo alive.

### iosef (Swift, MIT): PR #15, "send touch and keyboard through dtuhidd on Xcode 27"

- Connection as in idb, whose wire format it follows: `lookup:error:` called through its IMP, the `_4sim` endpoint, `xpc_connection_create_from_endpoint`, `enable_sim2host`, an error handler, resume. Liveness is `xpc_connection_send_message_with_reply` of a barrier `IndigoKeyboardButtonEvent {usageCode 0, state 2}` with a 4 s deadline and three attempts, then a 200 ms tail; each send is followed by an `xpc_connection_send_barrier` wait, and teardown drains 80 ms [iosef-dtu].
- Routing: touch and keys go over dtuhidd when CoreSimulator is 1155.4 or later and dtuhidd answers, over Indigo otherwise with a 30 s retry window; hardware buttons always go over Indigo; `IOSEF_HID_TRANSPORT=indigo` forces the legacy path [iosef-comp]. The PR also added the missing `NSSize` and edge parameters to the Indigo builder binding, whose size register had held garbage [iosef#15].
- Finding: once dtuhidd is active SimulatorHID disconnects the legacy touch, button and keyboard services; an Indigo event reconnects a suppressed service, but if touch had connected earlier in the boot the reconnect is torn down in the same instant ("Service added" then "Service removed"), so some boots work and others never do, and Indigo keys are dropped regardless. Tested on fresh iPhone 17 Pro simulators on iOS 26.4 and 26.5 with Xcode 27.0 and CoreSimulator 1171.6 [iosef#15].
- PR #14: on Xcode 27 the translator's frontmost request returned nil where accessibility had been enabled after boot; writing `AccessibilityEnabled` and `ApplicationAccessibilityEnabled` in `com.apple.Accessibility` and restarting `com.apple.CoreSimulator.bridge` once fixed it [iosef#14][iosef-axen]. iosef's IOSurface screenshots take about 3 ms against about 200 ms through `simctl` [iosef-shot].

### stim (TypeScript and Swift): PR #1326 and issue #1049

- #1049 added Desktop input through `SimDeviceLegacyHIDClient` and Indigo messages, with a note to verify the selectors on Xcode 27 [stim#1049].
- #1326 moved input to dtuhidd after finding that any CoreDevice client (DeviceHub.app, Siniulator) starts dtuhidd and that legacy input "stays broken until the simulator reboots"; resetting the notify state or stopping dtuhidd did not restore touch, keys (target 0x64) and buttons (0x33) fail the same way, and keys sent to 0x32 work only while the legacy touch service survives. Input is queued until the activation barrier is answered, a connection with no answer in 5 s is dropped, and the legacy client is retried after 10 s. The first input activates dtuhidd and locks legacy clients out until reboot. Keeping one connection per session required replacing a display-identity watch with a device-state poll, because CoreSimulator returns a new proxy on every lookup. Quitting DeviceHub.app shuts down every booted simulator [stim#1326].
- Wire details: `IndigoButtonEvent` uses usage page 0x0C; the digitizer `target` is 0 for the main screen and the screen ID for any other; backboardd aborts on an Indigo target it has no service for [stim-hid].

### AXe (Swift CLI on an idb fork, MIT)

- Exposes `describe-ui`, `tap` by point, label or identifier, `swipe`, `drag`, `touch`, gesture presets, `type`, `key`, `key-sequence`, `key-combo`, `button`, `slider`, `screenshot`, `record-video`, `stream-video` and `batch` [axe-src]. It supports Xcode 26 and 27, validated with Xcode 26.5 on iOS 26.5 and Xcode 27 beta 3 on iOS 27, and on Xcode 27 runs through DeviceHub.app without Simulator.app [axe-readme]; #60 added the `SharedFrameworks` SimulatorKit path [axe#60].
- Keeps one `axe hid-broker --udid` process per simulator behind a Unix socket with a 60 s idle timeout, and waits until the simulator has been up 10 s before using dtuhidd [axe-broker]. Open issue #71: on Xcode 27.0 (27A266a) with iOS 27.0 a short-lived `axe tap` loses its gesture because dtuhidd "creates and activates the virtual services that carry events only once it has a peer, and drops anything addressed to a service that is not yet active"; upstream idb fixed it by priming the connection [axe#71]. The unmerged #66 proposed a point-grid hit test for WKWebView content; its output on iOS 26.5 shows the default host walk returning none of the page's elements, because the page renders in another process [axe#66].

### agent-device snapshot bridge (Objective-C guest)

- Compiled on first use with `xcrun --sdk iphonesimulator clang -arch <arch> -mios-simulator-version-min=15.0` and started with `xcrun simctl spawn <udid> <bridge> serve <socket>`; frames are a big-endian `uint32` length plus one UTF-8 JSON object on a Unix socket in the shared file system [ad-spawn][ad-bridge].
- Loads `/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework` and `/System/Library/PrivateFrameworks/AXRuntime.framework` from the guest root, creates `XCTAccessibilityFramework` with `-initForRemoteAccess`, roots the read at `+[XCAccessibilityElement elementWithProcessIdentifier:]` and calls `-userTestingSnapshotForElement:options:error:` with `XCTDefaultSnapshotParameters()`, attribute numbers from `XCAXAccessibilityAttributesForStringAttributes` for the `XC_kAXXCAttribute*` names, and `maxDepth`, `maxChildren` and `maxArrayCount` bounds. It asserts automation mode through `AXSettings`, wraps each call in `@try`, and a watchdog kills the guest on overrun [ad-bridge].
- Foreground ownership: `+[AXElement primaryApp].pid` is compared with the target before, during and after capture, and the read fails as `foreground-owner-unverified` when system UI covers the app. WebKit content arrives as an `AXRemoteElement` leaf; a tree with such a leaf inside a web view is refused and that app generation is routed to XCTest, which is the fallback for any bridge failure [ad-bridge][ad-adr4].
- Measured on an iOS 26.2 runtime: warm p50/p95 from 8.6/9.3 ms on a quiet screen to 118.2/120.9 ms on a list; spawn, connect and first tree p95 1136.6 ms; at most 220 ms guest CPU per read; `AutomationEnabled` and `IgnoreAXServerEntitlements` set before boot [ad-ev].

### Apple's public surface

- `xcrun simctl` in Xcode 27 boots, installs, launches, spawns, opens URLs, grants privacy, pushes notifications, overrides the status bar and moves the pasteboard, but has no touch, key or tree command `[local]`. `simctl io <udid> screenshot` writes one PNG, TIFF, BMP, GIF or JPEG per call and `recordVideo` writes a movie file `[local]`. `simctl launch` and `simctl spawn` pass `SIMCTL_CHILD_`-prefixed variables to the child `[local]`; the CoreSimulator launch call takes an `environment` dictionary directly [idb-launch].
- `xcrun devicectl` 642.16 adds screen capture, orientation, Darwin notifications and simulated location, status bar and biometrics, still without input or a tree `[local]`.

## Comparison

| Concern | idb | baguette | iosef | agent-device | Our plan |
|---|---|---|---|---|---|
| Shape | Swift companion behind gRPC | Swift CLI and server, persistent input process per device | Swift CLI and MCP, one process per command | TypeScript daemon, ObjC guest reader, XCTest runner | One Rust daemon using objc2, nothing in the guest |
| Lifecycle | CoreSimulator API | CoreSimulator API and `simctl` | CoreSimulator API | `simctl` | CoreSimulator API (S2) |
| Touch on Xcode 27 | dtuhidd, Indigo if unreachable | Indigo via IOHIDEvent, backboardd restart | dtuhidd, Indigo fallback | XCTest | dtuhidd (S3) |
| Touch before 1155.4 | Indigo | Indigo | Indigo | XCTest | Indigo |
| Keys | dtuhidd; Indigo before 1155.4 | Indigo HID usages to 0x32 | dtuhidd; Indigo before 1155.4 | XCTest | dtuhidd; Indigo before 1155.4; paste for long text |
| Buttons | dtuhidd Consumer usages; Indigo before 1155.4 | Indigo | Indigo | XCTest | dtuhidd; Indigo before 1155.4 |
| Frames | SimScreen callbacks, Core Image | per-present callbacks, ImageIO, VideoToolbox | IOSurface or `simctl` screenshot | `simctl` or runner screenshot | SimScreen callbacks, encode on a worker (S2) |
| Tree | host translator or guest bridge | host translator plus hit-test sweep | host translator | guest snapshot reader, XCTest fallback | host translator unless S4 changes the spec |
| Private-call failures | exception guard per send | none found in source | none found in source | `@try` and a watchdog | `objc2::exception::catch` per call, target marked unavailable |

## Input transports by Xcode

Xcode's version numbers go from 16 to 26, so there is no Xcode 17 to 25. Rows describe the host CoreSimulator that the Xcode installs; the guest runtime matters only where a row names it.

| Host | SimulatorKit location | Indigo touch | Indigo keys | Indigo buttons | dtuhidd | Sources |
|---|---|---|---|---|---|---|
| Xcode 16 | `Contents/Developer/Library/PrivateFrameworks` | works | works | works | absent | [idb-load][idb-02eb93c] |
| Xcode 26.x, CoreSimulator below 1155.4 | same | works with idb's binding (AXe validated iOS 26.5); baguette moved taps to the IOHIDEvent path on iOS 26.4 | works | works | absent | [idb-sel][bag-iohid][axe-readme] |
| Xcode 27.0, CoreSimulator 1155.4 or later, iOS 18.6 or 26.x guest | `Contents/SharedFrameworks` | boot race, dead once dtuhidd is active | dropped | work | works | [iosef#15][iosef-comp][idb-sel][stim#1326] |
| Xcode 27.0, iOS 27.0 guest | `Contents/SharedFrameworks` | boot race, dead once active; a backboardd restart recovers it | dead | dead | works | [bag-hub][stim#1326][axe-readme] |

The reference host is row three. Row one rests on idb's long-standing Indigo path and its register analysis [idb-02eb93c]; no source in this survey reports a 2026 test on Xcode 16.

## Private symbols and frameworks per spike

Images, all present on the reference host `[local]`:
- `CS`: `/Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/CoreSimulator` (1171.7), installed by Xcode outside `Xcode.app`.
- `IO`: `CoreSimulator.framework/Versions/A/Frameworks/CoreSimDeviceIO.framework/Versions/A/CoreSimDeviceIO` (1171.7), loaded with CS.
- `SK`: `/Applications/Xcode.app/Contents/SharedFrameworks/SimulatorKit.framework/Versions/A/SimulatorKit` (1005.2, arm64e); Xcode 26 and earlier use `Xcode.app/Contents/Developer/Library/PrivateFrameworks/SimulatorKit.framework`.
- `XPC` (`/usr/lib/system/libxpc.dylib`), `IOK` (`/System/Library/Frameworks/IOKit.framework`) and `AXP` (`/System/Library/PrivateFrameworks/AccessibilityPlatformTranslation.framework`), all in the dyld shared cache.
- Guest daemons injected by CS: `CoreSimulator.framework/Versions/A/Resources/Platforms/iphoneos/usr/libexec/dtuhidd` and `.../CoreSimulatorBridge`, with their plists under `.../iphoneos/Library/LaunchDaemons/`.
- Guest images, usable only from a spawned reader: `<RuntimeRoot>/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework` and `<RuntimeRoot>/System/Library/PrivateFrameworks/AXRuntime.framework`, with `<RuntimeRoot>` from `xcrun simctl list runtimes -j`.

**S2, lifecycle and frames.** Objective-C only. From CS: `SimServiceContext +sharedServiceContextForDeveloperDir:error:` and `-defaultDeviceSetWithError:`; `SimDeviceSet -devicesByUDID`; `SimDevice -bootAsyncWithOptions:completionQueue:completionHandler:`, `-shutdownAsyncWithCompletionQueue:completionHandler:`, `-installApplication:withOptions:error:`, `-launchApplicationAsyncWithID:options:completionQueue:completionHandler:` (option keys `arguments`, `environment`, `stdout`, `stderr` [idb-launch]), `-terminateApplicationWithID:error:`, `-state`, `-deviceType` (`mainScreenSize`, `mainScreenScale`), `-runtime` (`versionString`), `-dataPath`, `-io` and `-registerNotificationHandler:`. From IO: `-ioPorts`, `-descriptor`, `-state` (`displayClass`), `SimScreenAdapter -enumerateScreensWithCompletionQueue:completionHandler:`, `SimScreen -registerScreenCallbacksWithUUID:callbackQueue:frameCallback:surfacesChangedCallback:propertiesChangedCallback:`, `-unregisterScreenCallbacksWithUUID:`, `-screenProperties` (`uniqueId`), `-framebufferSurface`, and the fallbacks `-registerCallbackWithUUID:ioSurfacesChangeCallback:` and `-registerCallbackWithUUID:damageRectanglesCallback:`.

**S3, input.** dtuhidd: `SimDevice -lookup:error:` from CS; private C `xpc_endpoint_create_mach_port_4sim(mach_port_t, uint64_t, uint64_t)` and `xpc_connection_enable_sim2host_4sim(xpc_connection_t)` from XPC; the rest is public libxpc. Indigo: the class `_TtC12SimulatorKit24SimDeviceLegacyHIDClient` (its mangled runtime name is in SK) with `-initWithDevice:error:` and `-sendWithMessage:freeWhenDone:completionQueue:completion:`, plus C functions from SK with these Xcode 27 prototypes: `IndigoHIDMessageForMouseNSEvent(CGPoint *, CGPoint *, IndigoHIDTarget, NSEventType, NSSize, IndigoHIDEdge)`, `IndigoHIDMessageForButton(IndigoHIDButtonKeyCode, IndigoHIDButtonOp, IndigoHIDTarget)`, `IndigoHIDMessageForHIDArbitrary(IndigoHIDTarget, uint32_t, uint32_t, IndigoHIDButtonOp)` and `IndigoHIDMessageForKeyboardArbitrary(uint32_t, IndigoHIDButtonOp)`. Only if edge gestures need baguette's recipe: `IndigoHIDMessageForTrackpadEventFromHIDEventRef(IOHIDEventRef, IndigoHIDTarget)` from SK with `IOHIDEventCreateDigitizerEvent`, `IOHIDEventCreateDigitizerFingerEvent` and `IOHIDEventAppendEvent` from IOK. The version gate reads `CFBundleVersion` from the bundle that vends `SimDevice` [idb-sel].

**S4, accessibility.** Host translator, from AXP: `AXPTranslator +sharedInstance`, `bridgeTokenDelegate`, `-frontmostApplicationWithDisplayId:bridgeDelegateToken:`, `-objectAtPoint:displayId:bridgeDelegateToken:` and `-macPlatformElementFromTranslation:`; `AXPTranslatorResponse +emptyResponse`; `AXPTranslationObject` `pid` and `bridgeDelegateToken`; `AXPMacPlatformElement -accessibilityChildren` and `-accessibilityAttributeValue:`; a Rust-defined class implementing `AXPTranslationTokenDelegateHelper -accessibilityTranslationDelegateBridgeCallbackWithToken:`, which returns a block; and `SimDevice -sendAccessibilityRequestAsync:completionQueue:completionHandler:` from CS [idb-ax][idb-h-axp][iosef#14]. Comparison arm, guest only: `XCTAccessibilityFramework -initForRemoteAccess` and `-userTestingSnapshotForElement:options:error:`, `XCAccessibilityElement +elementWithProcessIdentifier:` and `-AXUIElement`, C functions `XCTDefaultSnapshotParameters`, `XCAXAccessibilityAttributesForStringAttributes`, `AXValueGetType`, `AXValueGetValue` and `_AXSAutomationEnabled`, `AXSettings -setAutomationEnabled:` and `AXElement +primaryApp` [ad-bridge].

The Indigo builders, the two `_4sim` functions, the IOHIDEvent constructors and the guest `XCT*`/`AX*` functions are C symbols, not Objective-C methods: they are resolved with `dlsym`, cast to `extern "C"` function pointer types with the prototypes above, and called directly.

## Rust building blocks

Versions from crates.io on 2026-09-26.

| Need | Crate | Use |
|---|---|---|
| Objective-C runtime | `objc2` 0.6.4 | `AnyClass::get(c"...")` for lookup by name, `msg_send!`, `define_class!` for the translator delegate, `objc2::exception::catch` behind the `exception` feature (needs `panic = "unwind"`); the `catch-all` feature wraps every send [objc2] |
| Foundation | `objc2-foundation` 0.3.2 | `NSString`, `NSDictionary`, `NSUUID`, `NSError` for options and callbacks |
| Blocks | `block2` 0.6.2 | completion, frame and translator blocks; XPC handlers |
| Queues | `dispatch2` 0.3.1 | callback queues and the serial translator queue |
| IOSurface | `objc2-io-surface` 0.3.2 | `lock`, `unlock`, `base_address`, `bytes_per_row`, `pixel_format`, `seed` [objc2-io-surface] |
| Core Foundation | `objc2-core-foundation` 0.3.2 or `core-foundation` 0.10.1 | `IOHIDEventRef` ownership, bundle versions |
| Mach | `mach2` 0.7.0 | `mach_port_t` from `lookup:error:`; `mach_msg` only for Purple GSEvents or a direct Indigo port |
| XPC | hand-written `extern "C"` block | `xpc-connection` 0.2.3 and `xpc-connection-sys` 0.1.1 (last published 2020) and `xpc-sys` 1.0.0 all run bindgen over the public `xpc.h` at build time and bind no `_4sim` symbol [xpc-crates]; about 20 public functions and two `dlsym` lookups avoid a libclang build dependency |
| Loading | `libloading` 0.9.0 or `libc` | framework handles and `dlsym` of C SPI |
| Encoding | `turbojpeg` 1.5.1 or `objc2-image-io` 0.3.2; `objc2-core-image` 0.3.2 for scaling; `objc2-video-toolbox` 0.3.2 later | S2 candidates |

## What we take

For D6:
1. Load CS by absolute path and SK from `$DEVELOPER_DIR/../SharedFrameworks`, falling back to `$DEVELOPER_DIR/Library/PrivateFrameworks`; refuse root; record the CoreSimulator, SimulatorKit and runtime versions for `doctor` [idb-load].
2. Look up every private class by name with `AnyClass::get` and never link it; wrap every private send in `objc2::exception::catch`, and mark the target unavailable when one raises [idb-load][idb-client].
3. Select the input transport by CoreSimulator version (1155.4 or later means dtuhidd), connect once per session, and fall back to Indigo only when dtuhidd is unreachable, reporting that the fallback costs the keyboard [idb-sel][idb-neg].
4. Port the dtuhidd connection and wire format: the `_4sim` endpoint, `enable_sim2host`, the barrier probe with a 4 s deadline and 4 s retries, input queued until the barrier reply, and one connection held for the whole session so activation is paid once [idb-dtu][stim-hid][axe#71].
5. Port Indigo for hosts below 1155.4: `SimDeviceLegacyHIDClient`, `malloc`ed messages, the 6-parameter builder with a unit `NSSize`, target 0x32, and every builder call on one serial queue [idb-indigo][local].
6. Read accessibility through the host translator: one serial translator queue for all targets, a Rust delegate class, accessibility defaults written into the device's preferences under `-dataPath` before boot where possible, and one `CoreSimulatorBridge` restart when they are written later [idb-ax][iosef#14][ad-ev].
7. Reuse agent-device's vocabulary for degraded reads (foreground owner, remote-content boundary, truncation) even though our reader differs [ad-bridge][ad-adr4].

For D7:
8. Find the main display by `displayClass == 0`, or by `uniqueId` through `SimScreenAdapter`; register SimScreen callbacks on a dedicated serial queue, falling back to the surface-change and damage callbacks [idb-fb].
9. Take the `IOSurface` from `surfacesChangedCallback` and read it on `frameCallback`; never call `framebufferSurface` per frame; coalesce captures and drain an autorelease pool per capture [idb-fb][bag-screen].
10. Encode on a worker thread; S2 compares `turbojpeg` with ImageIO, which baguette uses [bag-arch], and scales with Core Image as idb does [idb-fb] or on the CPU.
11. Use `frameCallback` as the change signal for settle (D5), hash a downscaled copy, and detect reboots by polling device state rather than proxy identity [stim#1326].

## Risks and unknowns

S2, lifecycle and frames:
- No source publishes per-frame CPU or encode time for this pipeline; baguette only advertises 60 fps [bag-arch].
- Whether a read of the surface can tear while the render server composites, and whether `seed` advances on every present, is unverified.
- The pixel format is assumed BGRA [stim-hid]; S2 must check `pixel_format`.
- Boot completion arrives when the state flips, seconds before SpringBoard and dtuhidd are ready [bag-hub]; S2 needs a readiness signal before timing a launch.
- DeviceHub.app attaches to devices as they boot, and quitting it shuts every booted device down [bag-hub][stim#1326]; spikes should run with it closed.

S3, input:
- No source publishes a warm dtuhidd tap latency, so the 20 ms warm-tap gate is untested; idb's 80 ms drain is a teardown cost, not a per-event one [idb-dtu].
- Events sent before activation are dropped [axe#71]; the session must finish the probe before reporting ready, which can take up to 4 s plus the 200 ms tail, longer during a slow boot [idb-dtu].
- The Indigo half is order-dependent on this host: fresh boot, DeviceHub.app closed, and `xcrun simctl spawn <udid> notifyutil -g com.apple.coredevice.dtuhidd.active` reading 0 before the first tap [bag-hub]. After the dtuhidd half only a reboot, or baguette's backboardd restart (verified on iOS 27.0 only), brings Indigo back [stim#1326][bag-hub].
- The Indigo builder keeps unsynchronized statics and returns nil for moves closer than 16 ms `[local]` [stim-hid]; baguette's main-thread requirement is not confirmed by the Xcode 27 code [bag-arch].
- Edge gestures over dtuhidd depend on the `edge` field [idb-dtu]; no source reports a home-indicator swipe through it.
- An unknown Indigo target aborts backboardd [stim-hid]; only 0x32 (digitizer), 0x33 (buttons) and 0x64 (keyboard) appear in the sources.
- Coordinates are ratios of the native portrait framebuffer, so rotation needs a mapping [stim-hid].
- Paste-based text entry: under Xcode 27 `simctl pbcopy` exits 0 and writes nothing on iOS 26.5 guests while `devicectl device pasteboard copy` lands [bag-pb]; the guest also vends `com.apple.coredevice.feature.pasteboard` `[local]`, an untested direct route.

S4, accessibility:
- The spec and D6 conflict with the only fast snapshot path (see the verdict). S4 should time the host translator and a spawned reader on the same 500-node screens so 2.11 decides with numbers.
- The translator pays one XPC round trip per element and attribute [idb-axdoc], so a 500-node tree may miss the p95 gate; baguette's completeness sweep adds 1.5 to 2 s [bag-sweep].
- Fidelity gaps map to degraded verdicts: composed views collapse [idb-axdoc], the status bar and children of SwiftUI containers can be missing [bag-sweep][axe-changelog], and web content in another process is invisible without a hit-test grid [axe#66].
- The translator is a process-wide singleton with one delegate slot [idb-ax], so AX reads for several Simulators serialize inside one daemon; the parallel-Simulators requirement has to be measured with that in mind.
- Automation mode changes tree size and cost, is device-wide, and is cleared when an accessibility observer exits [idb-axdoc].
- The 1-element failure on iOS 26.5 [idb#964] and the nil frontmost result repaired by a bridge restart [iosef#14] must be detected and handled automatically.
- Writing the accessibility defaults at runtime spawns Apple's `defaults` in the guest [iosef-axen], itself a process launched inside the Simulator; writing the device's preferences before boot avoids that, as agent-device does for its keys [ad-ev].
- Reporting system-owned foreground on the host path (the frontmost translation's `pid` against the app's pid) is untested; agent-device's check runs in the guest [ad-bridge].

## Local evidence

Read-only commands on the reference host; the booted Simulator was not touched.
- Versions: `plutil -p` on the `Info.plist` of CS (1171.7), IO (1171.7) and SK (1005.2); `xcrun simctl list runtimes -j` (iOS 26.5, 23F77).
- SK: `nm -gU` lists 25 `IndigoHID*` exports; `strings -a` shows the prototypes quoted above, `_TtC12SimulatorKit24SimDeviceLegacyHIDClient`, `initWithDevice:error:` and `sendWithMessage:freeWhenDone:completionQueue:completion:`; `otool -tV -p _IndigoHIDMessageForMouseNSEvent` shows x0 to x4 and q0/q1 saved, `cmp x24, #0x4`, and calls to `calloc`, `mach_absolute_time` and `free` only; `nm` shows the `had_second_point`, `last_second_point`, `last_timestamp` and `once_token` statics.
- CS and IO: `strings -a` finds every CS and IO selector listed for S2, S3 and S4.
- Shared cache: `dyld_info -exports` lists `_xpc_endpoint_create_mach_port_4sim`, `_xpc_connection_enable_sim2host_4sim` and `_xpc_connection_create_from_endpoint` in libxpc, the three `IOHIDEvent` constructors in IOKit, and `_OBJC_CLASS_$_AXPTranslator`, `AXPTranslationObject`, `AXPTranslatorRequest`, `AXPTranslatorResponse` and `AXPMacPlatformElement` in AXP.
- dtuhidd: `plutil -p` on `com.apple.coredevice.dtuhidd.plist` shows the Mach services above and `EnablePressuredExit`, and `com.apple.coredevice.dtpasteboardd.plist` vends `com.apple.coredevice.feature.pasteboard`; its man page says it "receives HID events encoded using UniversalHID from the wire and posts them to the IOHIDEventSystem"; `usr/libexec/dtuhidd` is absent from the iOS 26.5 runtime root, which ships `UniversalHID.framework`.
- Guest reader image: `xcrun vtool -show-build` on the runtime's `XCTAutomationSupport` prints `platform IOSSIMULATOR`, `minos 26.5`.
- Public tools: `xcrun simctl help`, `help io`, `help launch` and `help spawn`; `xcrun devicectl device --help`; `Xcode.app/Contents/Applications` holds `DeviceHub.app` and there is no `Contents/Developer/Applications`; `pgrep` found no DeviceHub.app process.

## Sources

Pinned commits: idb `1c5c81f6cb`, baguette `bb74901c11`, iosef `f0a0c3f2b5`, stim `bbce7aa2fa`, AXe `30f4bfa9bc`, agent-device `3660c7fa41`.

- [idb-sel] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/HID/SimulatorHIDTransportSelection.swift
- [idb-neg] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/HID/SimulatorHIDTransport.swift
- [idb-client] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/HID/SimulatorIndigoHIDClient.swift
- [idb-indigo] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/HID/SimulatorIndigoHID.swift and `SimulatorIndigoHIDTransport.swift` in the same directory
- [idb-h] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/PrivateHeaders/SimulatorApp/Indigo.h
- [idb-dtu] https://github.com/facebook/idb/tree/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/HID (`SimulatorDTUHIDConnection.swift`, `DTUHIDModels.swift`, `SimulatorDigitizerHIDTransport.swift`, `SimulatorVendorHIDTransport.swift`)
- [idb-xpc] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/XPC/SimulatorXPCConnection.swift
- [idb-fb] https://github.com/facebook/idb/tree/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/Framebuffer and https://github.com/facebook/idb/tree/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/PrivateHeaders/CoreSimDeviceIO
- [idb-load] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBControlCore/Utility/WeakFramework.swift and `ObjCRuntimeClass.swift` in the same directory
- [idb-ax] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/Accessibility/AXTranslationDispatcher.swift and https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/Commands/SimulatorAccessibilityCommands.swift
- [idb-h-axp] https://github.com/facebook/idb/tree/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/PrivateHeaders/AccessibilityPlatformTranslation
- [idb-sfb] https://github.com/facebook/idb/tree/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/SimulatorFrameworkBridge and https://github.com/facebook/idb/tree/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/SimulatorFrameworkBridge (`SimulatorFrameworkBridgeSelection.swift` for the platform rule)
- [idb-cd] https://github.com/facebook/idb/tree/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/CoreDevice
- [idb-launch] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/FBSimulatorControl/Strategies/SimulatorProcessSpawnStrategy.swift
- [idb-axdoc] https://github.com/facebook/idb/blob/1c5c81f6cbe3a31986eda66349fd22a2f9b47858/website/docs/idb/accessibility.mdx
- [idb-02eb93c] https://github.com/facebook/idb/commit/02eb93c76d6ebc099540495a3996bd25ac857159
- [idb-cf91155] https://github.com/facebook/idb/commit/cf91155ab3b1963b89b647b00650f59f2e796754
- [idb#964] https://github.com/facebook/idb/issues/964
- [bag-arch] https://github.com/tddworks/baguette/blob/bb74901c114619523267d7c707341b2df3562185/docs/ARCHITECTURE.md
- [bag-hid] https://github.com/tddworks/baguette/blob/bb74901c114619523267d7c707341b2df3562185/Sources/Baguette/Infrastructure/Input/IndigoHIDInput.swift
- [bag-iohid] https://github.com/tddworks/baguette/blob/bb74901c114619523267d7c707341b2df3562185/Sources/Baguette/Infrastructure/Input/IOHIDDigitizerDispatch.swift
- [bag-hub] https://github.com/tddworks/baguette/blob/bb74901c114619523267d7c707341b2df3562185/docs/features/device-hub/design.md and `README.md` in the same directory
- [bag-screen] https://github.com/tddworks/baguette/blob/bb74901c114619523267d7c707341b2df3562185/Sources/Baguette/Infrastructure/Screen/SimulatorKitScreen.swift
- [bag-sweep] https://github.com/tddworks/baguette/blob/bb74901c114619523267d7c707341b2df3562185/docs/features/ax-hit-test-sweep/README.md
- [bag-pb] https://github.com/tddworks/baguette/blob/bb74901c114619523267d7c707341b2df3562185/Sources/Baguette/Infrastructure/Pasteboard/SimctlPasteboard.swift
- [iosef#15] https://github.com/riwsky/iosef/pull/15
- [iosef#14] https://github.com/riwsky/iosef/pull/14
- [iosef-dtu] https://github.com/riwsky/iosef/blob/f0a0c3f2b5d95eb2a878b32902747385c1a8b2ed/Sources/SimulatorKit/Input/DTUHIDTransport.swift
- [iosef-comp] https://github.com/riwsky/iosef/blob/f0a0c3f2b5d95eb2a878b32902747385c1a8b2ed/Sources/SimulatorKit/Input/CompositeHIDTransport.swift
- [iosef-axen] https://github.com/riwsky/iosef/blob/f0a0c3f2b5d95eb2a878b32902747385c1a8b2ed/Sources/SimulatorKit/Accessibility/SimulatorAccessibilityEnabler.swift
- [iosef-shot] https://github.com/riwsky/iosef/blob/f0a0c3f2b5d95eb2a878b32902747385c1a8b2ed/Sources/SimulatorKit/Screenshot/ScreenCapture.swift
- [stim#1326] https://github.com/appandflow/stim/pull/1326
- [stim#1049] https://github.com/appandflow/stim/issues/1049
- [stim-hid] https://github.com/appandflow/stim/blob/bbce7aa2faff9571d3844c350b0c89590ff6f2fe/apps/desktop/Sources/SimulatorFrames/SimulatorHID.swift
- [axe-readme] https://github.com/cameroncooke/AXe/blob/30f4bfa9bc81817906a60fadedbc913d7314b7e1/README.md
- [axe-src] https://github.com/cameroncooke/AXe/tree/30f4bfa9bc81817906a60fadedbc913d7314b7e1/Sources/AXe/Commands
- [axe-broker] https://github.com/cameroncooke/AXe/blob/30f4bfa9bc81817906a60fadedbc913d7314b7e1/Sources/AXe/Utilities/HIDBroker.swift
- [axe-changelog] https://github.com/cameroncooke/AXe/blob/30f4bfa9bc81817906a60fadedbc913d7314b7e1/CHANGELOG.md (v1.7.0, SwiftUI `TabView` items)
- [axe#60] https://github.com/cameroncooke/AXe/pull/60
- [axe#66] https://github.com/cameroncooke/AXe/pull/66
- [axe#71] https://github.com/cameroncooke/AXe/issues/71
- [ad-bridge] https://github.com/callstack/agent-device/tree/3660c7fa413f386447e4311ca6875db4c83b0010/apple/snapshot-bridge (`README.md`, `SnapshotBridgeRuntime.m`)
- [ad-spawn] https://github.com/callstack/agent-device/blob/3660c7fa413f386447e4311ca6875db4c83b0010/packages/platform-apple/src/snapshot-source/host.ts and `cache.ts` in the same directory
- [ad-adr4] https://github.com/callstack/agent-device/blob/3660c7fa413f386447e4311ca6875db4c83b0010/docs/adr/0004-ios-snapshot-backend-strategy.md
- [ad-ev] https://github.com/callstack/agent-device/blob/3660c7fa413f386447e4311ca6875db4c83b0010/docs/evidence/ios-simulator-ax-bridge-2026-09-02-corrected.md
- [objc2] https://docs.rs/objc2/0.6.4/objc2/exception/fn.catch.html, https://docs.rs/objc2/0.6.4/objc2/runtime/struct.AnyClass.html and https://docs.rs/objc2/0.6.4/objc2/macro.define_class.html
- [objc2-io-surface] https://docs.rs/objc2-io-surface/0.3.2/objc2_io_surface/struct.IOSurfaceRef.html
- [xpc-crates] https://github.com/dfrankland/xpc-connection-rs/blob/master/xpc-connection-sys/build.rs and https://github.com/mach-kernel/launchk/blob/master/xpc-sys/build.rs
- [ad-text] https://github.com/callstack/agent-device/blob/v0.21.14/apple/runner/AgentDeviceRunner/AgentDeviceRunnerUITests/RunnerTests+TextTyping.swift, with `RunnerTests+TextEntry.swift` and `RunnerTests+TypeExecution.swift` in the same directory
- [ad-android] https://github.com/callstack/agent-device/blob/v0.21.14/packages/platform-android/src/fill-verification.ts and `text-input.ts` in the same directory
- [ad-pace] https://github.com/callstack/agent-device/blob/v0.21.22/apple/runner/AgentDeviceRunner/AgentDeviceRunnerUITests/RunnerTests+TextEntry.swift (lines 94 to 108)
- [wda-type] https://github.com/appium/WebDriverAgent/blob/v16.14.0/WebDriverAgentLib/Categories/XCUIElement+FBTyping.m and https://github.com/appium/WebDriverAgent/blob/v16.14.0/WebDriverAgentLib/Utilities/FBConfiguration.m
- [xcd-settings] https://github.com/appium/appium-xcuitest-driver/blob/v12.15.0/docs/reference/settings.md (`maxTypingFrequency`)
- [mae-input] https://github.com/mobile-dev-inc/maestro/blob/cli-2.11.0/maestro-ios-xctest-runner/maestro-driver-iosUITests/Routes/Helpers/TextInputHelper.swift and https://github.com/mobile-dev-inc/maestro/blob/cli-2.11.0/maestro-android/src/androidTest/java/dev/mobile/maestro/MaestroDriverService.kt
- [pw-fill] https://github.com/microsoft/playwright/blob/v1.63.0/packages/injected/src/injectedScript.ts and https://github.com/microsoft/playwright/blob/v1.63.0/packages/playwright-core/src/server/dom.ts
- [ab-fill] https://github.com/vercel-labs/agent-browser/blob/v0.37.1/cli/src/native/interaction.rs
- [uia2-set] https://github.com/appium/appium-uiautomator2-server/blob/v10.6.6/app/src/main/java/io/appium/uiautomator2/utils/ElementHelpers.java
- [idb-text] https://github.com/facebook/idb/blob/c697b74a47ead5f6f6fd8e9ecbef24201e5cce65/idb/common/hid.py (lines 255 to 262) and https://github.com/facebook/idb/blob/c697b74a47ead5f6f6fd8e9ecbef24201e5cce65/FBSimulatorControl/HID/SimulatorDigitizerHIDTransport.swift
- [mae-paste] https://github.com/mobile-dev-inc/maestro/blob/60f9ba47b9df8a7508f3e663b7defd17d754c7ec/maestro-orchestra/src/main/java/maestro/orchestra/Orchestra.kt (lines 1830 to 1867)
- [wda-pb] https://github.com/appium/WebDriverAgent/blob/2e98ac0ad4dbc99ab7c8bb01b7216fde252afb54/WebDriverAgentLib/Utilities/FBPasteboard.m and `Commands/FBCustomCommands.m` in the same library
- [ad-pb] https://github.com/callstack/agent-device/blob/44e4ec5c25da7866ee70b70037b44723dfd1813e/apple/runner/AgentDeviceRunner/AgentDeviceRunnerUITests/RunnerTests+CommandDispatch.swift (lines 70 to 85), from pull request 3065
- [simctl-pb] https://github.com/appium/node-simctl/blob/6dffa8b32cb2489130b3093611c17a52b4e3c829/lib/subcommands/pbcopy.ts
- [coresim-pb] https://github.com/appium/appium-ios/blob/f674d8563c83ddaf61921464dfa6cc4618ef8e5e/packages/coresim/src/native/sim_pasteboard.mm
- [auto-mobile#8347] https://github.com/kaeawc/auto-mobile/issues/8347
- [pmd3-pb] https://github.com/doronz88/pymobiledevice3/blob/90b8d44c83999db6c57e5fbcd5b796be5567de3f/pymobiledevice3/remote/core_device/pasteboard_service.py (lines 1 to 54)


## Corrections after the spikes

Measured on this host by S2, S3 and S4; details and numbers are in the spike reports.

- ROCKit proxies: everything reached from `-[SimDevice ioPorts]` onward is a forwarding proxy that objc2's debug-build checks reject, so those calls go through a direct `objc_msgSend` helper, and blocks handed to ROCKit need a signature (S2).
- `framebufferSurface` is cheap (0.3 ms median) and never changes within a session; cache it for that reason, not for cost. BGRA rows are padded to 4864 bytes; late reads tear, so throttle by skipping frames (S2). `SimScreenAdapter` is not needed; `displayClass == 0` is the main display. `zune-jpeg` is decode-only (S2).
- Indigo touch kept working after dtuhidd was active on this host, and neither transport locked the other out; input sent before dtuhidd's activation reply was never dropped; the 200 ms wait after the reply copied from idb and iosef only delays the first action; one dtuhidd connection carries touches, keys and buttons; dtuhidd stays running after its first launch (S3). `-lookup:error:` returns a `mach_port_t` through a plain message send (S3). The 9-argument versus 6-argument question is moot for dtuhidd.
- The fast tree reader must run inside the guest; a Rust binary for `aarch64-apple-ios-sim` spawned through the SimDevice API is ready in 292 ms, while `xcrun simctl spawn` costs 570 ms or more (S4). `+[AXElement springBoardElement]` does not exist on iOS 26.5; SpringBoard comes from `systemWideElement` then `springBoardApplication` (S4). The translator path works without iosef's bridge restart once the three `com.apple.Accessibility` preferences are written before first boot, but it flattens the tree and drops UIKit tab bar children and React Native text identifiers (S4).
- New risk: a Simulator display can stop presenting while input is still accepted; a reboot cleared it (S3).

## Typed text read back (task 10.23)

`fill #searchScreenInput nobel` typed its keys 15 ms apart into Bluesky's search field, a React Native `TextInput` whose value the app owns, and replied success while the field read "noelb" (`reports/10.4-rn-smoke.md`, third run, finding 3). Of the tools compared, only agent-device reads typed text back. Its iOS runner wants the exact text to hold for 0.2 s, polled every 20 ms, clears the field and retypes the text once on a mismatch, then fails with `TEXT_ENTRY_MISMATCH` naming the expected and observed text, and leaves secure fields unverified [ad-text]; its Android path samples the field at 0, 150 and 350 ms, compares a password field by length, retries once, and calls a field that changed to other text an unconfirmed success, for apps that format their values [ad-android]. agent-device 0.21.22 slowed its private typing route from 60 to 12 characters a second, because keys 13 ms apart outran "an app that owns its field's value and re-applies it after the edit (a controlled React Native `TextInput`…)" [ad-pace]. WebDriverAgent types 60 characters a second unless told to slow down "if your tests are failing due to typing errors", and reads a field back only while clearing it, three tries at most [wda-type][xcd-settings]; Maestro's iOS runner types the first character alone, waits 500 ms because "characters after the first on are often skipped" and types the rest at 30 a second, and its Android driver sleeps 75 ms between keys, neither reading back [mae-input]. Playwright's `fill` and agent-browser's select the old text and insert the new in one `Input.insertText`, and UiAutomator2 sets it with `ACTION_SET_TEXT`; none reads it back, except Playwright for the input types whose value it assigns ("Malformed value") [pw-fill][ab-fill][uia2-set]. None pastes. What we take: read the field back after every fill, a secure field by its length, and let the text hold for the settle's quiet window, as agent-device holds it 0.2 s; on a mismatch, enter the text again in one piece, a paste on iOS as those single inserts and `ACTION_SET_TEXT` are, rather than retype keys the same field can reorder again; then fail naming what the field holds.

## The pasteboard (task 10.31)

A paste on the Simulator set the pasteboard with `devicectl device pasteboard copy`, 113 to 202 ms of a 36-character fill (`reports/10.26-remeasure.md`, 10.29 rerun). idb has no pasteboard command; `idb ui text` sends a key down and up per character over dtuhidd [idb-text]. Maestro never writes the iOS pasteboard: `pasteText` types the text it kept through `inputText` [mae-paste], whose runner types the first character, waits 500 ms and types the rest at 30 a second [mae-input]. WebDriverAgent sets `UIPasteboard.generalPasteboard` from its XCTest runner, one HTTP round trip, and types through XCTest's synthesized events [wda-pb][wda-type]. agent-device 0.21.14 writes a Simulator's clipboard with `xcrun simctl pbcopy` (`dist/src/interactor.js` in its package) and types through its runner; a later version sets `UIPasteboard.general.string` from the runner instead, 9.6 s cold and 0.19 s warm, because `pbcopy` left a promise that read back empty [ad-pb]. Appium's `mobile: setPasteboard` ran `xcrun simctl pbcopy` until September 2026 [simctl-pb] and now pushes a uniquely named host `NSPasteboard` through SimPasteboardPlus's `SimPasteboardInterface` in its own process, then sleeps 0.5 s, since `-push` has no completion [coresim-pb]. baguette runs `devicectl device pasteboard copy` and falls back to `simctl pbcopy`, which it found writes nothing under Xcode 27 [bag-pb]; others found `pbcopy` working, or no longer changing the text after a `devicectl` copy [auto-mobile#8347]. This host shows why both are seen, in the guest's log (`simctl spawn <udid> log stream`) while `devicectl`, `pbcopy` and a probe of our own set the pasteboard. `pbcopy` pushes through CoreSimulator's SimPasteboardPlus, the framework of the host pasteboard's sync, to `CoreSimulatorBridge` in the guest, which logs "Ignoring snapshot for com.apple.UIKit.pboard.general: the CoreDevice pasteboard stack is in use" once `dtpasteboardd` has started and published `com.apple.coredevice.pasteboard.active`; so `pbcopy` works on a fresh boot and not after the first `devicectl` copy `[local]`. `devicectl` itself connects to that `dtpasteboardd`, through the guest's `com.apple.coredevice.feature.pasteboard`, the CoreDevice feature service family that dtuhidd belongs to. It sends a `SET`, `{command, pasteboardName, items: [{types, data: {<type>: {data}}}]}`, which `dtpasteboardd` answers with `SET_REPLY`, or with an error naming the key it could not decode `[local]`; pymobiledevice3 documents the same commands on a device's RemoteXPC service, where its `dtpasteboardd` answered one request per connection [pmd3-pb]; the Simulator's answered four in a row on one connection `[local]`. Most of `devicectl`'s time is its own process: `devicectl --version` alone takes 60 to 70 ms on this host, and the guest logs the `SET` and the pasteboard's save within 16 ms `[local]`. What we take: send that `SET` from the driver's process, over its own connection per paste, as the dtuhidd transport sends its events, and keep `devicectl` for a guest that does not vend or decode it.
