# Android agent: state of the art

Research note for task 2.1 of the `mobile-automation-foundation` change. It precedes spike S1 (a shell-user `app_process` program that owns Android UI automation) and the Kotlin agent in task group 4. Snapshot date: 2026-09-26.

Versions read: scrcpy 4.1; AOSP `android-17.0.0_r1` (API 37), diffed against `android-14.0.0_r1`, `android-15.0.0_r1`, `android-16.0.0_r1` and `android16-qpr2-release`; Appium UIAutomator2 server 10.6.6; openatx `uiautomator2` 3.2.0 with `u2.jar` 0.4.0; agent-device 0.21.15; Android Emulator 37.1.11. The frameworks/base `main` branch on android.googlesource.com has not moved since 2025-03-26 [45] and is older than the 16 QPR2 and 17 release branches, so AOSP links point at release tags. Bracketed numbers refer to the Sources list. Statements marked *(inference)* are reasoned from code, not observed on a device.

## Summary

| Tool | Launch mechanism | Privilege | Tree source | Input path | Transport | Idle policy |
|---|---|---|---|---|---|---|
| scrcpy server 4.1 | push jar to `/data/local/tmp`, `CLASSPATH=… app_process / …Server <version> k=v…` | shell (uid 2000), drops root to 2000 | none | reflective `InputManager.injectInputEvent(e, ASYNC)` | 1 to 3 connections on `localabstract:scrcpy_<scid>` via `adb reverse`, forward as fallback; custom binary | none; video frames only on change |
| AOSP `uiautomator dump` | `uiautomator.sh`: `exec app_process /system/bin …Launcher` | shell, owns `UiAutomation` itself | `getRootInActiveWindow()`, or `getWindowsOnAllDisplays()` with `--windows` | none in `dump` | XML file on the device | `waitForIdle(1000, 10000)`, fails on endless animation |
| Appium UIAutomator2 server 10.6.6 | two APKs, `am instrument -w …server.test/…AndroidJUnitRunner` | instrumentation; connection hosted by the `am` shell process | androidx `UiDevice`, active window by default, XML then DOM | UiAutomator2 over `UiAutomation` | Netty HTTP, W3C WebDriver JSON, port 6790 (MJPEG 7810) | 500 ms quiet within 10 s, cache cleared before every find or source |
| openatx `u2.jar` 0.4.0 | push jar, `CLASSPATH=… app_process / com.wetest.uia2.Main -p 9008` | shell, reflective `UiAutomation` | androidx `UiDevice` dumper, XML, depth 50 | androidx `UiDevice` and `UiObject` | NanoHTTPD JSON-RPC 2.0 on TCP 9008, all interfaces | idle and selector timeouts 0, action ack 500 ms |
| agent-device 0.21.15 helpers | APK; one-shot `am instrument -w`, or resident with `-e sessionPort` | instrumentation | `getWindows()` with the interactive-window flag, cache cleared per capture | hidden 3-argument `injectInputEvent(e, sync, false)`; text through an IME | instrumentation status records (base64 XML) or a line protocol on `127.0.0.1` | 100 ms quiet within 500 ms, captures anyway |
| Emulator gRPC | `emulator -grpc <port>`, optionally `-grpc-use-token` | host process, outside the guest | none | `sendTouch`, `sendKey`, `streamInputEvent` | gRPC with a bearer token from the discovery file | none; `streamScreenshot` pushes each new frame |
| This design (D2) | push dex jar, `app_process` as shell, resident | shell, own `UiAutomationConnection` | windows, events and subtree refresh | `InputManager` with `WAIT_FOR_RESULT`, or the connection without animation wait | abstract socket through adb, framed binary (D3, D4) | bounded, default under 1 s, 0 allowed; settle in the mirror (D8) |

Each cell is sourced in the matching section below.

## 1. scrcpy server

**What it does.** The client pushes the server to `/data/local/tmp/scrcpy-server.jar` (writable by shell, not world-writable) and runs `adb shell CLASSPATH=/data/local/tmp/scrcpy-server.jar app_process / com.genymobile.scrcpy.Server <version> key=value…`; the server fails unless client and server versions match exactly [1], [2]. By default the host listens and the device connects out through `adb reverse` to `localabstract:scrcpy_<scid>`, which avoids a start-up race; with `adb forward` the device first sends a dummy byte so the host can tell a live server from a bare forward [1], [15]. Up to three sockets (video, audio, control) open in that order [1].

- Control protocol. Host to device: a 1-byte type (23 types in 4.1: key, text, touch, scroll, clipboard, display power, rotation, UHID, start app and others) followed by big-endian fixed fields and length-prefixed UTF-8, 256 KiB at most [8]. Touch and scroll positions carry the frame size the host saw, and the server drops events whose size no longer matches, which filters taps computed before a rotation [9]. Device to host: clipboard text, a clipboard acknowledgement echoing a 64-bit sequence, and UHID output [11]. There are no request ids and no input acknowledgements: nearly all input uses `INJECT_INPUT_EVENT_MODE_ASYNC` [10], which "never blocks" and "is assumed always to be successful" [35]. The protocol is internal, documented only by unit tests, and incompatible across versions [1].
- Hidden APIs. Services come from reflective `android.os.ServiceManager.getService` plus `I<Name>$Stub.asInterface` [4]; input goes through `InputManager.injectInputEvent(InputEvent, int)` found with `getMethod` [5]; mirroring uses the hidden static `DisplayManager.createVirtualDisplay(name, w, h, displayIdToMirror, surface)` with a `SurfaceControl.createDisplay` plus transaction fallback [12]. Managers that expect an app `Context` (clipboard, activity, camera, audio) get a `FakeContext` whose package is `com.android.shell` and whose `AttributionSource` carries the shell uid; `Workarounds` fabricates `ActivityThread` state (`sCurrentActivityThread`, `mSystemThread`, `AppBindData`, `ConfigurationController`, an `Application`) and carries guards for Samsung, ONYX and Vivo builds, with a Quest guard in the capture code [6], [7], [12].
- Hidden-API restrictions. The 4.1 tree contains no bypass (no `VMRuntime` exemption call; checked by searching the source). It needs none, because ART leaves enforcement off for `app_process` programs (section 6).
- Video. The display is mirrored into `MediaCodec.createInputSurface()`; the encoder sets `KEY_REPEAT_PREVIOUS_FRAME_AFTER`, real-time `KEY_PRIORITY` 0 and `KEY_LATENCY` 1 [13]. Frames exist only when the surface changes, and rotation restarts the encoding session [1].
- Lifecycle. `main` sets `Looper.sMainLooper` by reflection so the loop can quit, drops uid 0 to 2000, and ends with `System.exit` because framework threads can keep the VM alive [3]. `CleanUp` starts a second `app_process` from the same jar that calls `setsid()` to outlive the server, deletes the jar, waits for EOF on stdin, then restores `show_touches`, `stay_on_while_plugged_in`, `screen_off_timeout`, the IME policy and display power [14].

**Reuse.** The launch recipe with the version on the command line; abstract socket names with a per-launch id; reflective `InputManager` injection; the stale-geometry guard on coordinates; the watcher process for restoring settings; `server/build_without_gradle.sh` (`javac` against `android.jar`, `aidl`, stub sources, `d8`) as the template for task 1.5 [16]. For scale, the whole 4.1 server is 733,706 bytes [17].

**Differs.** scrcpy has no tree, no acknowledged input, exact version coupling and a lifetime bound to one adb shell session. The agent needs request ids, per-action acceptance, version negotiation and reuse across daemon sessions.

## 2. AOSP `uiautomator` and `UiAutomationConnection`

**What it does.** `uiautomator` is itself a shell-user `app_process` program: `uiautomator.sh` puts `uiautomator.jar` on `CLASSPATH` and runs `exec app_process ${base}/bin com.android.commands.uiautomator.Launcher`, which renames the process with `Process.setArgV0("uiautomator")` [18], [19]. `UiAutomationShellWrapper.connect()` (in `library/testrunner-src/`, not `core-src/`) obtains the role through the hidden constructor [20]:

```java
// The AccessibilityInteractionClient used by UiAutomation expects the main looper to
// be prepared. In most contexts this is normally done automatically, but must be called
// explicitly here because this is a shell tool.
Looper.prepareMainLooper();
mUiAutomation = new UiAutomation(mHandlerThread.getLooper(),
        new UiAutomationConnection());
mUiAutomation.connect();
```

`DumpCommand` connects, sets or clears `FLAG_INCLUDE_NOT_IMPORTANT_VIEWS` (`--compressed`), calls `waitForIdle(1000, 10000)` and prints `ERROR: could not get idle state.` when events never pause for a second; `--windows` adds `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` and dumps `getWindowsOnAllDisplays()`, otherwise only `getRootInActiveWindow()` is read; output is an XML file and the connection is dropped on exit [21].

- Ownership. `UiAutomationConnection.connect()` records `Binder.getCallingUid()` as owner, and every privileged call then accepts only the owner, system or root (L116, L757) [22]. It registers through `IAccessibilityManager.registerUiTestAutomationService` with `FLAG_INCLUDE_NOT_IMPORTANT_VIEWS`, `FLAG_REPORT_VIEW_IDS` and `FLAG_FORCE_DIRECT_BOOT_AWARE`, and calls `setAccessibilityTool(true)` unless `FLAG_NOT_ACCESSIBILITY_TOOL` is passed (L670) [22]. The system side enforces `RETRIEVE_WINDOW_CONTENT` [25]. The Shell package holds `MANAGE_ACCESSIBILITY`, `INJECT_EVENTS`, `RETRIEVE_WINDOW_CONTENT`, `CAPTURE_VIDEO_OUTPUT` and `READ_FRAME_BUFFER` [26].
- Where the connection runs. The object executes in the process that constructs it. `am instrument` constructs it in the `am` shell process, and only when it waits for results (`-w` or proto output) [27], so an instrumented app borrows shell privileges while `am instrument` lives *(inference)*. In `uiautomator`, in `u2.jar` and in D2, owner and connection are the same shell process.
- Single owner. `UiAutomationManager` says "Only one may be registered at a time"; a second registration throws `IllegalStateException("UiAutomationService " + client + "already registered!")`, where `client` is a binder proxy, not a package [24]; openatx quotes the real message, `…IAccessibilityServiceClient$Stub$Proxy@5deffd5already registered!` [63]. `dumpsys accessibility` prints only `Ui Automation[eventTypes=…, notificationTimeout=…]` (L343) [24]. Instrumentation holders appear under "Active instrumentation" in `dumpsys activity processes` [28]; `app_process` holders only in the process list. The owner binder is linked to death, so the role is released when the owning process dies (L116) [24]; that this covers `SIGKILL` follows from binder death notification *(inference)*.
- Hidden members, all `@hide`. `UiAutomationConnection()` is `@UnsupportedAppUsage(maxTargetSdk = R)` [22]; `UiAutomation(Looper, IUiAutomationConnection)` is `@Deprecated` and `@UnsupportedAppUsage(maxTargetSdk = P)`; `UiAutomation(Context, IUiAutomationConnection)` exists from Android 14 and is absent at `android-13.0.0_r1` [81]; `connect()`, `connect(int)` and `connectWithTimeout(int, long)` are hidden, with a 5 s default [23].

**Reuse.** This construction is the core of D2. The `--windows` path and the dumper's attributes are the baseline node schema.

**Differs.** One connection per invocation, a fixed one-second quiet requirement under a ten-second ceiling, and a file as output.

## 3. Appium UIAutomator2 server

**What it does.** Two APKs, server and test; the server runs inside an instrumentation test started with `am instrument -w io.appium.uiautomator2.server.test/androidx.test.runner.AndroidJUnitRunner` [51]. A Netty HTTP server on device port 6790 speaks W3C WebDriver JSON, with an MJPEG server on 7810 [51], [52]; screenshots are PNG, Base64-encoded inside JSON [58].

- Tree walk. Every page source and element lookup first calls `Device.waitForIdle()` and then clears the accessibility cache (`UiAutomation.clearCache()` on API 34+, `setServiceInfo(null)` before) [53], [54]. The snapshot visits each child with `getChild(i)`, keeps it if `isVisibleToUser()`, stops at depth 70, and copies a configurable set of about 40 attributes [57], [60]; the XML is pretty-printed, and XPath lookups re-parse it into a Xerces DOM [56]. Only the active window's root is read by default (`enableMultiWindows=false`), retried every 250 ms for up to 10 s [54], [57].
- Idle. `waitForIdleTimeout` 10 s, `waitForSelectorTimeout` 10 s, `actionAcknowledgmentTimeout` 3 s, `scrollAcknowledgmentTimeout` 200 ms [57]. `UiDevice.waitForIdle(timeout)` asks for 500 ms without accessibility events [59], so a read right after a UI change waits until 500 ms have passed since the last event, and an endless animation costs the full 10 s.
- Per-node cost. After the cache clear every node crosses Binder again, in requests of at most 50 prefetched nodes served on the app's UI thread (section 6), then pays attribute copies, XML text and JSON escaping. Appium's own `deferAccessibilityCacheReset` setting says the always-reset default forces "a full Binder re-traversal of the accessibility tree per find, which drives unbounded ART LinearAlloc growth" [55].

**Reuse.** The attribute vocabulary and the multi-window option. `deferAccessibilityCacheReset`, which clears the cache only after relevant events, is evidence for the direction of D5.

**Differs.** HTTP and JSON per call, XML and DOM, a cache reset and an idle wait per request, one window by default.

## 4. openatx uiautomator2

**What it does.** The original server is an instrumentation APK (`com.github.uiautomator.test`) serving JSON-RPC on device port 9008 [61]. The Python client 3.2.0 starts a different server: it pushes `u2.jar` when its MD5 differs, runs `CLASSPATH=/data/local/tmp/u2.jar app_process / com.wetest.uia2.Main -p 9008` over a streaming adb shell, and treats "already registered" in its output as a conflict [62], [63]. Calls are JSON-RPC 2.0 `POST /jsonrpc/0` through an adb-tunnelled TCP connection, and `dump_hierarchy` calls `dumpWindowHierarchy(compressed, maxDepth)` with depth 50 and receives an XML string [62], [64]. The APK is still used for IME-based text input [65].

- `u2.jar` builds `UiAutomationConnection()` and the `UiAutomation(Looper, IUiAutomationConnection)` constructor by reflection, calls `connectWithTimeout(flags, 5000)` above API 30 and `connect(flags)` below, and wraps the result in `InstrumentShellWrapper extends Instrumentation` so the stock androidx `UiDevice` runs unmodified [66]. `Main` applies a copy of scrcpy's `Workarounds` and serves NanoHTTPD [67].
- It sets `waitForIdleTimeout` and `waitForSelectorTimeout` to 0 and action acknowledgement to 500 ms (androidx defaults 10 s, 10 s, 3 s) [68].
- NanoHTTPD 2.3.1's `NanoHTTPD(int port)` binds the wildcard address [69], so this shell-privileged RPC listens on every interface of the device. The jar is 3.7 MB (androidx UiAutomator, Jackson, jsonrpc4j, NanoHTTPD) [70].

**Reuse.** Production evidence on current Android that the D2 construction works from `app_process`; the hash check before push.

**Differs.** HTTP and JSON-RPC per call, XML dumps, no event stream, and an unauthenticated TCP listener.

## 5. agent-device Android helpers

**What it does.** The snapshot helper is a plain (not `testOnly`) instrumentation APK installed with `adb install -r` and run one-shot with `am instrument -w … SnapshotInstrumentation` in `snapshot`, `viewport` or `gesture` mode, or kept resident with `-e sessionPort <port>`, serving a line protocol on `127.0.0.1` behind `adb forward` [71], [72].

- Tree. It sets `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` and serializes every root from `getWindows()`, so keyboards and system overlays appear, falling back to the active root; depth is capped at 128 and nodes at 5000, with a truncation flag [71], [73].
- Idle. `waitForIdle(min(100, bound), bound)` with a 500 ms default bound, capturing anyway on timeout [72].
- Cache. It clears the node cache before every traversal because on a persistent connection the cache "is not always invalidated" by Navigation 3 or other in-place content swaps inside one `AndroidComposeView` [73].
- Input. Gestures use the hidden 3-argument `UiAutomation.injectInputEvent(event, sync, false)` to skip the public overload's global animation wait [74]. `getUiAutomation()` runs on a worker with its own deadline because the framework call can wait 60 s [75].
- Text. The IME helper is a headless `InputMethodService` (no input view, so no keyboard nodes) that commits base64 UTF-8 text received through `am broadcast`, gated on `WRITE_SECURE_SETTINGS`, which shell holds and third-party apps cannot; switching IMEs is device-global and must be undone after a crash [76].

**Reuse.** Interactive-window capture, the short quiet window, the no-animation injection overload, a bounded connect, and the Compose staleness finding as an S1 test case.

**Differs.** An installed APK, instrumentation lifetime rules and a text-per-request protocol; the IME needs a second install and changes device-global state, which the no-install rule of D2 excludes.

## 6. Platform APIs

API levels come from the reference pages [47], [48], [49]; behaviour from the `android-17.0.0_r1` sources.

| API | Since | Behaviour that matters |
|---|---|---|
| `UiAutomation.getWindows()`, `getWindowsOnAllDisplays()` | 21, 30 | Need `FLAG_RETRIEVE_INTERACTIVE_WINDOWS`, which the connection does not set at registration [22], [23] |
| `getRootInActiveWindow()` | 18 | Prefetches with `FLAG_PREFETCH_DESCENDANTS_HYBRID`; a hidden overload takes a strategy [23] |
| `setOnAccessibilityEventListener()` | 18 | Events arrive on a `UiAutomation` handler thread and are re-posted to the looper given to the constructor [23] |
| `waitForIdle(idle, global)` | 18 | Returns once `idle` ms pass without events; throws `TimeoutException` at `global` [23] |
| `injectInputEvent(event, sync)` | 18 | "Waits for all window container animations and surface operations to complete": the connection calls `syncInputTransactions` before `DOWN` and after `UP`, then injects with `WAIT_FOR_FINISH` or `ASYNC` [22], [23] |
| `injectInputEvent(event, sync, waitForAnimations)` | 31, hidden `@TestApi` | Same path, animation wait optional [23], [81] |
| `takeScreenshot()` | 18 | `captureDisplay` into a hardware buffer, then a full `ARGB_8888` software copy [22], [23] |
| `clearCache()` | 34 | Clears the per-connection node cache [23] |
| `FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES` | 24 | Without it, other accessibility services are suppressed while connected [23], [24] |
| `FLAG_DONT_USE_ACCESSIBILITY` | 31 | No accessibility registration; injection and screenshots only [23], [24] |
| `AccessibilityNodeInfo.FLAG_PREFETCH_*`, `getChild(i, strategy)` | 33 | At most `MAX_NUMBER_OF_PREFETCHED_NODES` = 50 nodes per request [29] |
| `AccessibilityNodeInfo.refresh()` | 18 | Bypasses the cache with prefetch off, so one IPC per node [29], [30] |
| `ACTION_SET_TEXT` with `ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE` | 21 | Replaces the text of an editable node in one action [48] |
| `CONTENT_CHANGE_TYPE_*` on `TYPE_WINDOW_CONTENT_CHANGED` | 19 to 36 | `SUBTREE`, `TEXT` 19; `STATE_DESCRIPTION` 30; `ENABLED`, `ERROR` 34; `CHECKED`, `EXPANDED`, `SUPPLEMENTAL_DESCRIPTION` 36; window deltas use `TYPE_WINDOWS_CHANGED` (21) with `WINDOWS_CHANGE_*` (28) [49] |
| `InputManager.injectInputEvent(event, mode)` | hidden | `ASYNC` never blocks; `WAIT_FOR_RESULT` waits until dispatch decides whether injection is permitted, not for the app; `WAIT_FOR_FINISH` waits until the app handled it; 30 s timeout; `INJECT_EVENTS` enforced [35], [36] |

- Client cache. Lookups hit the per-connection `AccessibilityCache` first, and the cache processes each event before the listener sees it: a `SUBTREE` content change clears that subtree, other content changes re-fetch the source node, and `TYPE_WINDOW_STATE_CHANGED` or most `TYPE_WINDOWS_CHANGED` events clear the whole cache [33], [86]. Bypassing the cache also drops prefetch, prefetch is skipped for a window that scrolled within the last 150 ms (1.5 recurring-event intervals), and each interaction times out after 5 s [30], [32].
- Threading and cadence. Requests are served on the target app's UI thread [31], so a busy UI thread delays tree reads up to that timeout *(inference)*. A recurring accessibility event is sent at most once per 100 ms [32].
- Data sensitivity. Views marked `accessibilityDataSensitive`, which Android 16 also infers for `filterTouchesWhenObscured` views [34], [50], are served only to requests from accessibility tools [31]; `UiAutomationConnection` registers as a tool unless `FLAG_NOT_ACCESSIBILITY_TOOL` is set [22].

**Hidden-API enforcement for `app_process`.** ART initialises the policy to disabled: "The checks are disabled by default and we only enable them if: (a) runtime was started with a command line flag that enables the checks, or (b) Zygote forked a new process that is not exempt" [41], [42]. Zygote sets the policy per forked app [43]; `app_process` with a class name starts `RuntimeInit` [39], and `AndroidRuntime` passes no `-Xhidden-api-policy` flag [40]. The published restrictions are defined per app and target API level [46]. ART also exempts uid 2000 from the Android 14 writable-dex check: "Shell (2000): directly calling dalvikvm/app_process in ADB shell to run JARs with CLI is allowed" [44]. Reflection on hidden members therefore needs no bypass; breaking this would take an OEM change to ART or `AndroidRuntime`, and scrcpy running without a bypass across OEM devices suggests none has *(inference)*.

**Drift from Android 14 to 17.** `UiAutomationConnection.connect()` and its trust check are identical in `android-14.0.0_r1` and `android-17.0.0_r1`, and the hidden constructors and `connect` methods are present in both [22], [23], [82]. Android 16 added a display id to the hidden `IUiAutomationConnection.takeScreenshot` [38], [83]; `android16-qpr2-release` and 17 replaced the hidden `android.window.ScreenCapture` with `ScreenCaptureInternal` in that interface [37], [83]; 17 adds a `@TestApi` `takeScreenshot(int displayId)` [23], [84]. Injection enforcement in `InputManagerService` is the same in 16 and 17 [36], [84]. The public `UiAutomation` surface hides all three changes; direct calls into the connection or `ScreenCapture` would not.

## 7. Emulator gRPC controller

`emulator -grpc <port>` starts the `EmulatorController` service, which the proto still calls experimental [77], [80]; Google's container scripts launch with `-grpc 8554` [79]. `-grpc-use-token` requires `authorization: Bearer <token>` on every call, with the token read from the discovery file; `-grpc-use-jwt` validates JWTs against JWKs instead [80]. Discovery files are `pid_<pid>.ini` in `avd/running` under `$XDG_RUNTIME_DIR` (Linux), `$HOME/Library/Caches/TemporaryItems` (macOS) or `%LOCALAPPDATA%\Temp` (Windows), holding `grpc.port` and optionally `grpc.token` or `grpc.jwks` [78]. Emulator 37.1.11's help names the file `pid_%d_info.ini`, but the file it wrote on a macOS host was `pid_<pid>.ini` with `grpc.port`, `grpc.token` and `grpc.allowlist` [80], so glob `pid_*.ini` and read the keys. Input: `sendTouch(TouchEvent)`, `sendKey(KeyboardEvent)`, `sendMouse` and client-streamed `streamInputEvent`. Frames: `getScreenshot` and `streamScreenshot(ImageFormat)` in PNG, RGBA8888 or RGB888, scaled and "properly oriented", one frame "whenever the device produces a new frame", optionally through an `MMAP` shared-memory transport [77]. Input enters as virtual hardware events, not through the guest's `INJECT_EVENTS` path *(inference)*.

## What we take

**D2, agent as a shell `app_process` program**

1. Launch as scrcpy, `uiautomator` and `u2.jar` do: push a dex jar to `/data/local/tmp` when its hash differs, run `CLASSPATH=<jar> app_process / <Main> <protocol-version> …` [1], [18], [63]. No hidden-API bypass [41], [44].
2. Take the role as `UiAutomationShellWrapper` and `u2.jar` do: `Looper.prepareMainLooper()`, reflective `UiAutomationConnection()` and `UiAutomation(Looper, connection)` with a dedicated `HandlerThread` for callbacks, `connectWithTimeout(flags, bound)`, then add `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` [20], [66], [73]. Check each reflective member at start-up and report lost capabilities (design Risks, D12).
3. Name the process with `Process.setArgV0` so the daemon and conflict reports can find it [19].
4. Inject with `InputManager.injectInputEvent(event, WAIT_FOR_RESULT)` or the 3-argument overload with `waitForAnimations = false` on API 31+; the boolean result is the acceptance the spec asks for; never `ASYNC` for acknowledged actions [22], [35], [74].
5. Screenshots only through the public `UiAutomation.takeScreenshot()` [23], [37].
6. Text: `ACTION_SET_TEXT`, then key events from `KeyCharacterMap.getEvents` as scrcpy does [10], [48]; clipboard plus `KEYCODE_PASTE`, which scrcpy's set-clipboard message performs, is a candidate for Unicode [10]; no IME [76].
7. Lifecycle: explicit `System.exit` and the uid 0 to 2000 drop [3]; detach from the adb shell session with `setsid` as scrcpy's cleanup process does [14], plus an idle self-exit so a crashed daemon leaves no agent behind. Build with the no-Gradle recipe [16].

**D3, transport**

8. Abstract socket named with a per-launch id, protocol version on the command line and in the handshake, instead of scrcpy's exact-match rule [1], [15].
9. No TCP listener on the device (contrast `u2.jar` [69] and agent-device's loopback port [71]); the first frame must carry a per-launch token passed in the environment *(our proposal)*.
10. Every coordinate carries the display size and rotation it was computed for, and the agent rejects mismatches, as scrcpy does [9].

**D5, mirror updates**

11. Window deltas from `TYPE_WINDOWS_CHANGED` with `WINDOWS_CHANGE_*`; subtree deltas from `TYPE_WINDOW_CONTENT_CHANGED` with `CONTENT_CHANGE_TYPE_*` [49].
12. Refresh a dirty subtree after the platform cache has dropped it [86], reading it with `getChild(i, FLAG_PREFETCH_DESCENDANTS_BREADTH_FIRST | FLAG_PREFETCH_UNINTERRUPTIBLE)` on API 33+; each request returns at most 50 nodes, so avoid `refresh()` per node [29], [30]. Window-state changes empty the cache, so they cost a full walk, which matches D5.
13. Keep the platform cache as the first level (it is updated before the listener runs [33]) and call `clearCache()` only on resync, the idea behind Appium's opt-in deferred reset [55]; agent-device's Compose finding [73] is the counter-case S1 must test.
14. No `waitForIdle` on the snapshot path; a requested bounded wait uses a short quiet window (100 ms as in agent-device [72]) rather than androidx's 500 ms [59].

## Risks and unknowns for spike S1

1. Conflict naming. The platform reports only a binder proxy [24]. Record what `dumpsys activity processes` [28] and `ps -A -o PID,NAME,ARGS` show while Appium, Maestro, agent-device, `uiautomator dump` and `u2.jar` hold the role, and whether shell can end each holder (`am force-stop` for instrumentation, `kill` for shell processes) *(inference that these suffice)*.
2. Release. Confirm that `kill -9` of the agent frees the role at once (binder death [24]) and that the next attach succeeds.
3. Tree cost. Measure the 500-node fixture cold (after `clearCache`) and warm, per prefetch strategy, on the API 36 emulator and a device. A cold read needs at least ten requests [29], and every window-state change makes the next read cold [86]; requests run on the app UI thread with a 5 s timeout and lose prefetch while scrolling [30], [31].
4. Event completeness. Run a Compose Navigation 3 swap and a React Native list recycle, and compare the event-driven mirror with a full walk [73].
5. Event cadence. Recurring events are coalesced at 100 ms [32], and the 150 ms quiet window of D8 sits just above that; measure event-to-listener latency.
6. Tap acknowledgement against the 50 ms gate. Compare `injectInputEvent(e, true)`, the 3-argument form with `false`, and `InputManager` with `WAIT_FOR_RESULT` and `WAIT_FOR_FINISH`, on an idle screen and during an animation [22], [35].
7. Screenshot. Time `takeScreenshot()` plus its `ARGB_8888` copy plus JPEG encoding, and check landscape orientation [23].
8. Accessibility-service suppression. The default flags pause TalkBack and similar services while connected [23], [24]; choose the agent default.
9. OEM variance. scrcpy's Samsung, ONYX, Vivo and Quest workarounds [7], [12] show framework-context differences; run S1 on at least one non-Pixel device.
10. Transport. Check `adb forward tcp:… localabstract:…` and `adb reverse` against a shell-owned abstract socket; scrcpy defaults to reverse [1]. A static read of the `android-17.0.0_r1` sepolicy found `connectto` from `adbd` granted for app domains only and did not locate the rule that governs `adbd` connecting to a `shell` socket [85], so the answer is empirical.
11. Size. kotlin-stdlib 2.4.20 alone dexes to 2,279,904 bytes (660,541 zipped) with `d8 --release --min-api 26` from build-tools 36.0.0, measured on 2026-09-26, against 733,706 bytes for the whole scrcpy server [17]; choose R8 or stdlib-free Kotlin before task 1.5.
12. Connect bound. The hidden `connectWithTimeout` defaults to 5 s [23]; the instrumentation route can wait 60 s [75]. Bound it and report the cause.
13. Drift. Keep a per-API table of every reflective member (D12): screenshot plumbing changed in 16 and 16 QPR2 [37], [38], [83], and googlesource `main` is no guide to current releases [45].

## Sources

scrcpy, tag v4.1:
- [1] `doc/develop.md`: https://github.com/Genymobile/scrcpy/blob/v4.1/doc/develop.md
- [2] `app/src/server.c`: https://github.com/Genymobile/scrcpy/blob/v4.1/app/src/server.c#L210-L224
- [3] `Server.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/Server.java#L197-L290
- [4] `wrappers/ServiceManager.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/wrappers/ServiceManager.java#L17-L48
- [5] `wrappers/InputManager.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/wrappers/InputManager.java#L41-L73
- [6] `FakeContext.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/FakeContext.java#L17-L126
- [7] `Workarounds.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/Workarounds.java#L31-L146
- [8] `control/ControlMessageReader.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/control/ControlMessageReader.java#L14-L70
- [9] `control/PositionMapper.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/control/PositionMapper.java#L34-L47
- [10] `control/Controller.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/control/Controller.java#L448-L480
- [11] `control/DeviceMessageWriter.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/control/DeviceMessageWriter.java#L20-L46
- [12] `video/ScreenCapture.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/video/ScreenCapture.java#L132-L157
- [13] `video/SurfaceEncoder.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/video/SurfaceEncoder.java#L318-L335
- [14] `CleanUp.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/CleanUp.java#L125-L270
- [15] `device/DesktopConnection.java`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/src/main/java/com/genymobile/scrcpy/device/DesktopConnection.java#L47-L90
- [16] `server/build_without_gradle.sh`: https://github.com/Genymobile/scrcpy/blob/v4.1/server/build_without_gradle.sh#L79-L113
- [17] v4.1 release assets (`scrcpy-server-v4.1`, 733,706 bytes): https://github.com/Genymobile/scrcpy/releases/tag/v4.1

AOSP frameworks/base and art, tag `android-17.0.0_r1` unless stated:
- [18] `uiautomator.sh`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/cmds/uiautomator/cmds/uiautomator/uiautomator.sh#129
- [19] `Launcher.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/cmds/uiautomator/cmds/uiautomator/src/com/android/commands/uiautomator/Launcher.java#74
- [20] `UiAutomationShellWrapper.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/cmds/uiautomator/library/testrunner-src/com/android/uiautomator/core/UiAutomationShellWrapper.java#25
- [21] `DumpCommand.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/cmds/uiautomator/cmds/uiautomator/src/com/android/commands/uiautomator/DumpCommand.java#77
- [22] `UiAutomationConnection.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/app/UiAutomationConnection.java#107
- [23] `UiAutomation.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/app/UiAutomation.java#290
- [24] `UiAutomationManager.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/services/accessibility/java/com/android/server/accessibility/UiAutomationManager.java#84
- [25] `AccessibilityManagerService.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/services/accessibility/java/com/android/server/accessibility/AccessibilityManagerService.java#1958
- [26] Shell `AndroidManifest.xml`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/packages/Shell/AndroidManifest.xml#167
- [27] `Instrument.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/cmds/am/src/com/android/commands/am/Instrument.java#475
- [28] `ActivityManagerService.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/services/core/java/com/android/server/am/ActivityManagerService.java#11965
- [29] `AccessibilityNodeInfo.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/view/accessibility/AccessibilityNodeInfo.java#239
- [30] `AccessibilityInteractionClient.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/view/accessibility/AccessibilityInteractionClient.java#604
- [31] `AccessibilityInteractionController.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/view/AccessibilityInteractionController.java#135
- [32] `ViewConfiguration.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/view/ViewConfiguration.java#256
- [33] `AccessibilityService.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/accessibilityservice/AccessibilityService.java#2992
- [34] `View.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/view/View.java#16184
- [35] `InputManager.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/hardware/input/InputManager.java#209
- [36] `InputManagerService.java`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/services/core/java/com/android/server/input/InputManagerService.java#1037
- [37] `IUiAutomationConnection.aidl` (17): https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/app/IUiAutomationConnection.aidl
- [38] `IUiAutomationConnection.aidl` (16): https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-16.0.0_r1/core/java/android/app/IUiAutomationConnection.aidl
- [39] `app_main.cpp`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/cmds/app_process/app_main.cpp#338
- [40] `AndroidRuntime.cpp`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/jni/AndroidRuntime.cpp
- [41] art `runtime.cc`: https://android.googlesource.com/platform/art/+/refs/tags/android-17.0.0_r1/runtime/runtime.cc#1787
- [42] art `runtime_options.def`: https://android.googlesource.com/platform/art/+/refs/tags/android-17.0.0_r1/runtime/runtime_options.def#140
- [43] art `dalvik_system_ZygoteHooks.cc`: https://android.googlesource.com/platform/art/+/refs/tags/android-17.0.0_r1/runtime/native/dalvik_system_ZygoteHooks.cc#412
- [44] art `dalvik_system_DexFile.cc`: https://android.googlesource.com/platform/art/+/refs/tags/android-17.0.0_r1/runtime/native/dalvik_system_DexFile.cc#353
- [45] frameworks/base `main` head, committed 2025-03-26: https://android.googlesource.com/platform/frameworks/base/+/1cdfff555f4a21f71ccc978290e2e212e2f8b168

Android developer documentation:
- [46] Restrictions on non-SDK interfaces: https://developer.android.com/guide/app-compatibility/restrictions-non-sdk-interfaces
- [47] `UiAutomation` reference: https://developer.android.com/reference/android/app/UiAutomation
- [48] `AccessibilityNodeInfo` reference: https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo
- [49] `AccessibilityEvent` reference: https://developer.android.com/reference/android/view/accessibility/AccessibilityEvent
- [50] Android 16 `accessibilityDataSensitive`: https://developer.android.com/blog/posts/building-a-safer-android-and-google-play-together

Appium UIAutomator2 server 10.6.6 (commit `4a81391`) and androidx:
- [51] `README.md`: https://github.com/appium/appium-uiautomator2-server/blob/4a8139161cbb3aad078e36539995eb734297d56e/README.md
- [52] `ServerConfig.java`: https://github.com/appium/appium-uiautomator2-server/blob/4a8139161cbb3aad078e36539995eb734297d56e/app/src/main/java/io/appium/uiautomator2/server/ServerConfig.java#L31-L32
- [53] `handler/Source.java`: https://github.com/appium/appium-uiautomator2-server/blob/4a8139161cbb3aad078e36539995eb734297d56e/app/src/main/java/io/appium/uiautomator2/handler/Source.java#L35-L40
- [54] `utils/AXWindowHelpers.java`: https://github.com/appium/appium-uiautomator2-server/blob/4a8139161cbb3aad078e36539995eb734297d56e/app/src/main/java/io/appium/uiautomator2/utils/AXWindowHelpers.java#L41-L63
- [55] `DeferAccessibilityCacheReset.java`: https://github.com/appium/appium-uiautomator2-server/blob/4a8139161cbb3aad078e36539995eb734297d56e/app/src/main/java/io/appium/uiautomator2/model/settings/DeferAccessibilityCacheReset.java#L18-L28
- [56] `core/AccessibilityNodeInfoDumper.java`: https://github.com/appium/appium-uiautomator2-server/blob/4a8139161cbb3aad078e36539995eb734297d56e/app/src/main/java/io/appium/uiautomator2/core/AccessibilityNodeInfoDumper.java#L230-L326
- [57] settings defaults: https://github.com/appium/appium-uiautomator2-server/tree/4a8139161cbb3aad078e36539995eb734297d56e/app/src/main/java/io/appium/uiautomator2/model/settings
- [58] `utils/ScreenshotHelper.java`: https://github.com/appium/appium-uiautomator2-server/blob/4a8139161cbb3aad078e36539995eb734297d56e/app/src/main/java/io/appium/uiautomator2/utils/ScreenshotHelper.java#L80-L90
- [59] androidx `QueryController.java`: https://github.com/androidx/androidx/blob/a095da93f8e98dea8748ceed79ea8427aade245f/test/uiautomator/uiautomator/src/main/java/androidx/test/uiautomator/QueryController.java#L530-L539
- [60] `model/UiElementSnapshot.java`: https://github.com/appium/appium-uiautomator2-server/blob/4a8139161cbb3aad078e36539995eb734297d56e/app/src/main/java/io/appium/uiautomator2/model/UiElementSnapshot.java#L363-L390

openatx:
- [61] android-uiautomator-server `README.md`: https://github.com/openatx/android-uiautomator-server/blob/fa695aa6908bc86021c85b572f7ad82ca5540dd7/README.md
- [62] uiautomator2 `core.py`, launch and RPC: https://github.com/openatx/uiautomator2/blob/657c5d791075945cc21e78e125b220461c8ae99c/uiautomator2/core.py#L76-L80
- [63] uiautomator2 `core.py`, push and conflict detection: https://github.com/openatx/uiautomator2/blob/657c5d791075945cc21e78e125b220461c8ae99c/uiautomator2/core.py#L241-L300
- [64] uiautomator2 `__init__.py`, hierarchy dump: https://github.com/openatx/uiautomator2/blob/657c5d791075945cc21e78e125b220461c8ae99c/uiautomator2/__init__.py#L132-L147
- [65] uiautomator2 `_input.py`, IME from the APK: https://github.com/openatx/uiautomator2/blob/657c5d791075945cc21e78e125b220461c8ae99c/uiautomator2/_input.py#L47-L71
- [66] `InstrumentShellWrapper.java` (u2.jar source): https://github.com/openatx/android-uiautomator-server-jar/blob/d0449b9da4b32ad28bee0d3c3f561c49010acf35/uicore/src/main/java/uiautomator/InstrumentShellWrapper.java#L13-L117
- [67] `Main.java` (u2.jar source): https://github.com/openatx/android-uiautomator-server-jar/blob/d0449b9da4b32ad28bee0d3c3f561c49010acf35/app/src/main/java/com/wetest/uia2/Main.java#L90-L122
- [68] `AutomatorServiceImpl.java` (u2.jar source): https://github.com/openatx/android-uiautomator-server-jar/blob/d0449b9da4b32ad28bee0d3c3f561c49010acf35/app/src/main/java/com/wetest/uia2/stub/AutomatorServiceImpl.java#L105-L111
- [69] NanoHTTPD 2.3.1 wildcard bind: https://github.com/NanoHttpd/nanohttpd/blob/nanohttpd-project-2.3.1/core/src/main/java/fi/iki/elonen/NanoHTTPD.java#L2013-L2015
- [70] u2.jar 0.4.0 release (3,707,333 bytes): https://github.com/openatx/android-uiautomator-server-jar/releases/tag/0.4.0

agent-device 0.21.15 (commit `3660c7f`):
- [71] snapshot helper `README.md`: https://github.com/callstack/agent-device/blob/3660c7fa413f386447e4311ca6875db4c83b0010/android/snapshot-helper/README.md
- [72] `SnapshotInstrumentation.java`: https://github.com/callstack/agent-device/blob/3660c7fa413f386447e4311ca6875db4c83b0010/android/snapshot-helper/src/main/java/com/callstack/agentdevice/snapshothelper/SnapshotInstrumentation.java#L328-L352
- [73] `AccessibilityTreeCapture.java`: https://github.com/callstack/agent-device/blob/3660c7fa413f386447e4311ca6875db4c83b0010/android/snapshot-helper/src/main/java/com/callstack/agentdevice/snapshothelper/AccessibilityTreeCapture.java#L66-L110
- [74] `UiAutomationInputDispatcher.java`: https://github.com/callstack/agent-device/blob/3660c7fa413f386447e4311ca6875db4c83b0010/android/snapshot-helper/src/main/java/com/callstack/agentdevice/snapshothelper/UiAutomationInputDispatcher.java#L8-L29
- [75] `BoundedUiAutomationConnection.java`: https://github.com/callstack/agent-device/blob/3660c7fa413f386447e4311ca6875db4c83b0010/android/snapshot-helper/src/main/java/com/callstack/agentdevice/snapshothelper/BoundedUiAutomationConnection.java#L8-L17
- [76] IME helper `README.md`: https://github.com/callstack/agent-device/blob/3660c7fa413f386447e4311ca6875db4c83b0010/android/ime-helper/README.md

Android Emulator:
- [77] `emulator_controller.proto`: https://android.googlesource.com/platform/prebuilts/android-emulator/+/bbe98768a47ce9166f768e791768ae5f066c04df/linux-x86_64/lib/emulator_controller.proto#46
- [78] `emulator_discovery.py`: https://android.googlesource.com/platform/external/qemu/+/9f0811e72acfc46edc39d3d0baedd796f7d03309/android/android-grpc/python/aemu-grpc/src/aemu/discovery/emulator_discovery.py#164
- [79] container `launch-emulator.sh`: https://github.com/google/android-emulator-container-scripts/blob/0654f694b46794fae4b178f1e1a17cb60c5d2d34/emu/templates/launch-emulator.sh#L179
- [80] `emulator -help-all` output of Android Emulator 37.1.11.0 (build 15917651) and the discovery file it wrote, read locally on 2026-09-26 (no URL)

AOSP comparison points and further files:
- [81] `UiAutomation.java` at `android-11.0.0_r1`, `android-12.0.0_r1` (first with the 3-argument `injectInputEvent`) and `android-13.0.0_r1` (no `Context` constructor): https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-12.0.0_r1/core/java/android/app/UiAutomation.java#765 and https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-11.0.0_r1/core/java/android/app/UiAutomation.java#619 and https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-13.0.0_r1/core/java/android/app/UiAutomation.java#261
- [82] `UiAutomationConnection.java` at `android-14.0.0_r1`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-14.0.0_r1/core/java/android/app/UiAutomationConnection.java
- [83] `IUiAutomationConnection.aidl` at `android-15.0.0_r1` and `android16-qpr2-release`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-15.0.0_r1/core/java/android/app/IUiAutomationConnection.aidl and https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android16-qpr2-release/core/java/android/app/IUiAutomationConnection.aidl
- [84] `UiAutomation.java` and `InputManagerService.java` at `android-16.0.0_r1`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-16.0.0_r1/core/java/android/app/UiAutomation.java and https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-16.0.0_r1/services/core/java/com/android/server/input/InputManagerService.java#992
- [85] sepolicy `private/adbd.te` at `android-17.0.0_r1`: https://android.googlesource.com/platform/system/sepolicy/+/refs/tags/android-17.0.0_r1/private/adbd.te#129
- [86] `AccessibilityCache.java` at `android-17.0.0_r1`: https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-17.0.0_r1/core/java/android/view/accessibility/AccessibilityCache.java#211
