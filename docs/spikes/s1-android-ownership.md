# S1: shell-user ownership of Android UI automation

Spike S1, task 2.4 of `mobile-automation-foundation`. A throwaway Kotlin program started with `app_process` as the shell user takes the UI automation role, walks the accessibility tree, injects taps, records accessibility events, takes screenshots and serves an abstract socket through adb, on the API 36 emulator. It tests design decisions D2 (shell-user ownership), D3 (socket direction and token), D5 (tree cost model and mirror updates) and D8 (quiet window). Code: `spikes/s1-android-ownership/`, with the raw output of every run in its `results/`, both kept privately since the repository was made public (`docs/spikes/README.md`).

## Setup

| Item | Value |
|---|---|
| Host | Apple M1 Max, 10 cores, 64 GB, macOS 26.6.2 (25G83) |
| Host load | Shared with other agents: iOS Simulator boots, Xcode and Gradle builds, and spike S5 running `app_process` benchmarks on the same emulator. Every run records the load averages in its result file. Before timing runs `measure.sh` waited for no build process and a 1-minute load under 4, up to 20 minutes the first time and 5 minutes after. Only the socket run found such a moment; the other timing runs started when the wait ran out, the tap and event runs only after spike S7 had finished measuring on the same emulator. Each timing run's 1-minute load is given with its numbers. The adb transport dropped once, at 23:02 just after the screenshot runs, and the shared adb server restarted at 23:13:19; no run kept here spans either moment. |
| Emulator | Android Emulator 37.1.11.0, AVD `pixel_api36`, `google/sdk_gphone64_arm64/emu64a:16/BE2A.250530.026.F3/13894323:userdebug/dev-keys`, API 36, arm64-v8a, 1080x2400 at 420 dpi. Shared: another agent drove it during these runs, at times with the emulator process above 400% CPU. |
| Physical device | None attached. Not measured. |
| Toolchain | Kotlin 2.4.20 on Temurin 21.0.12.1, `d8` from build-tools 37.0.0, `platforms/android-36/android.jar`, adb 37.0.1, Rust 1.95.0 for the host tool |
| Program | One `classes.dex` of 2.36 MB (the Kotlin stdlib included) in `s1-spike.jar`, started as `CLASSPATH=/data/local/tmp/s1-spike.jar app_process /data/local/tmp dev.mobileagent.spike.s1.Main <subcommand>` through `adb shell`, so it runs as uid 2000 |
| Screens | Settings home; the Pixel launcher home; the Compose fixture `dev.mobileagent.fixture.compose` 1.0 (Compose UI 1.11.4, debug build) on Big, List, Anim and Login; the Expo fixture `dev.mobileagent.fixture.expo` 1.0.0 (React Native, release build with Hermes) on the same tabs |
| Spike code | branch `spike/s1-android-ownership`: `f44d033` (check, red), `d4e6935` (ownership, tree), `b697144` (tap, events, screenshot, staleness), `07a80a0` (socket), `c32c770` (service stream), `168d677` (cleanup), `7df158f` (rotation note); `measure.sh` and the raw results come with this report |
| Rerun | from the private copy: `cd spikes/s1-android-ownership && ./build.sh && (cd host && cargo build --release) && ./check.sh && ./measure.sh` |

Timings are p50 and p95 by nearest rank over the stated runs, with warm-up runs excluded. Round trips are exact counts: the spike replaces the accessibility client's connection in `AccessibilityInteractionClient.sConnectionCache` with a proxy that counts every call, and the client's interaction id counter agreed with the count on every run.

`check.sh` asserts the shape of every subcommand's output and the conflict texts. It was red before the program existed (`FAIL: spike jar not found`, then `FAIL: probe exited non-zero: unknown subcommand` against a stub), and is green on emulator-5554.

## Measurements

### Ownership

| Measure | Result |
|---|---|
| Hidden members (`probe`) | All present on API 36: `UiAutomationConnection()`, `UiAutomation(Looper, IUiAutomationConnection)`, `UiAutomation(Context, IUiAutomationConnection)`, `connect()`, `connect(int)`, `connectWithTimeout(int, long)`, `disconnect()`, `injectInputEvent(InputEvent, boolean, boolean)`, `InputManagerGlobal.getInstance()`, `InputManagerGlobal.injectInputEvent(InputEvent, int)`, `Process.setArgV0(String)`, `AccessibilityRecord.getSourceNodeId()`, and the accessibility client's `sConnectionCache` and `mInteractionIdCounter` fields used for counting. `UiAutomation.takeScreenshot(int)` is absent (it arrives in API 37); the connection's screenshot call is `takeScreenshot(Rect, ScreenCaptureListener, int)`. |
| Process start to `main` | p50 120 ms, p95 133 ms (10 attaches) |
| Construct and connect | p50 18.3 ms, p95 31.0 ms (10 attaches) |
| Disconnect | p50 4.6 ms, p95 12.7 ms |
| Window list after setting `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` | Complete at the first poll in 1 of 22 attaches, empty in 10, one window of two in 11; complete after 9 to 55 ms (median 19 ms) |
| Listing while held | `ps -A` shows `shell s1-spike s1-spike` after `setArgV0("s1-spike")`, under its `timeout … app_process … Main own …` parent; `dumpsys accessibility` prints `Ui Automation[eventTypes=TYPES_ALL_MASK, notificationTimeout=0]`; `dumpsys activity processes` has no instrumentation entry |
| After release | No `Ui Automation` line in `dumpsys accessibility` |
| After `kill -9` of the holder | The registration was still listed at the first `dumpsys` poll and gone at the second (one poll takes about 100 ms); the next attach connected in 12 ms |

The conflict texts, verbatim:

1. `adb shell uiautomator dump` from a second shell while the spike holds the role exits with status 137 and prints nothing on stdout or stderr. The exception is uncaught, so the runtime's kill handler logs it and SIGKILLs the process; the text reaches only `logcat -b crash`:

   ```
   E AndroidRuntime: FATAL EXCEPTION: main
   E AndroidRuntime: PID: 21608
   E AndroidRuntime: java.lang.IllegalStateException: UiAutomationService android.accessibilityservice.IAccessibilityServiceClient$Stub$Proxy@5a8b303already registered!
   E AndroidRuntime: 	at android.os.Parcel.createExceptionOrNull(Parcel.java:3348)
   …
   E AndroidRuntime: 	at android.app.UiAutomationConnection.registerUiTestAutomationServiceLocked(UiAutomationConnection.java:688)
   E AndroidRuntime: 	at android.app.UiAutomationConnection.connect(UiAutomationConnection.java:117)
   E AndroidRuntime: 	at android.app.UiAutomation.connectWithTimeout(UiAutomation.java:386)
   E AndroidRuntime: 	at android.app.UiAutomation.connect(UiAutomation.java:331)
   ```

2. A second instance of the spike gets the same exception from `connectWithTimeout`, not wrapped, with the system server's stack as the cause:

   ```
   java.lang.IllegalStateException: UiAutomationService android.accessibilityservice.IAccessibilityServiceClient$Stub$Proxy@5a8b303already registered!
   	at android.os.Parcel.createExceptionOrNull(Parcel.java:3348)
   …
   	at android.app.UiAutomationConnection.connect(UiAutomationConnection.java:117)
   	at android.app.UiAutomation.connectWithTimeout(UiAutomation.java:386)
   Caused by: android.os.RemoteException: Remote stack trace:
   	at com.android.server.accessibility.UiAutomationManager.registerUiTestAutomationServiceLocked(UiAutomationManager.java:109)
   	at com.android.server.accessibility.AccessibilityManagerService.registerUiTestAutomationService(AccessibilityManagerService.java:1643)
   ```

3. The spike while `uiautomator events` holds the role: the same text with that holder's proxy, `Proxy@a1c7bea`. `ps` lists the holder as `shell uiautomator uiautomator`, because it renames itself as the spike does.

The proxy in the message is the holder's client, not the newcomer's: the dump and the second instance both name `Proxy@5a8b303` while the spike holds the role (check.sh asserts this). It identifies the registration but not the process.

### Tree

A full walk calls `getWindows()` with `FLAG_RETRIEVE_INTERACTIVE_WINDOWS`, `getRoot(strategy)` per window and `getChild(i, strategy)` breadth first, and reads the attributes a snapshot serializes. The strategy is `bfs-u`, `FLAG_PREFETCH_DESCENDANTS_BREADTH_FIRST | FLAG_PREFETCH_UNINTERRUPTIBLE`, as the research note recommends. Cold clears the client cache before each walk, which is the state a window-state change leaves; warm walks over the cache the previous walk filled; winchange opens and closes the notification shade, waits 1 s, then walks. Bound 0 reads at once; bound 500 first calls `waitForIdle(100, 500)` and reads on timeout.

| Screen | Mode | Bound ms | Runs | Nodes (visible to user) | Wall p50 ms | Wall p95 ms | Round trips p50 | ms per round trip p50 | Load 1m |
|---|---|---|---|---|---|---|---|---|---|
| Compose Big | cold | 0 | 30 | 573 (568) | 349.0 | 450.6 | 465 | 0.72 | 4.8 |
| Compose Big | cold | 500 | 30 | 573 (568) | 345.1 | 400.1 | 465 | 0.70 | 4.3 |
| Compose Big | warm | 0 | 30 | 573 (568) | 3.1 | 4.0 | 2 | 0.90 | 4.8 |
| Compose Big | warm | 500 | 30 | 573 (568) | 3.7 | 5.9 | 2 | 1.19 | 4.3 |
| Expo Big | cold | 0 | 20 | 1049 (1047) | 390.3 | 462.1 | 506 | 0.73 | 8.0 |
| Expo Big | warm | 0 | 20 | 1049 (1047) | 4.0 | 5.4 | 2 | 1.01 | 8.0 |
| Compose List | cold | 0 | 20 | 88 (83) | 21.1 | 26.2 | 14 | 0.85 | 7.4 |
| Compose List | warm | 0 | 20 | 88 (83) | 2.1 | 2.5 | 2 | 0.89 | 7.4 |
| Expo List | cold | 0 | 20 | 102 (98) | 24.3 | 25.6 | 29 | 0.68 | 8.0 |
| Expo List | warm | 0 | 20 | 102 (98) | 2.3 | 2.8 | 2 | 0.90 | 8.0 |
| Compose Anim | cold | 0 | 20 | 75 (70) | 32.2 | 43.7 | 6 | 2.36 | 7.2 |
| Compose Anim | cold | 500 | 20 | 75 (70) | 28.9 | 56.0 | 6 | 2.32 | 7.2 |
| Expo Anim | cold | 0 | 20 | 50 (49) | 16.5 | 26.9 | 6 | 2.29 | 7.9 |
| Expo Anim | cold | 500 | 20 | 50 (49) | 113.8 (idle 81.6) | 135.5 | 6 | 2.41 | 7.9 |
| Settings | cold | 0 | 30 | 171 (97) | 37.7 | 44.3 | 50 | 0.68 | 10.1 |
| Settings | cold | 500 | 30 | 171 (97) | 36.6 | 40.3 | 50 | 0.66 | 10.1 |
| Settings | warm | 0 | 30 | 171 (97) | 2.1 | 2.6 | 2 | 0.88 | 10.1 |
| Settings | winchange, 1 s settle | 0 | 20 | 171 (97) | 50.1 | 75.4 | 50 | 0.85 | 9.7 |
| Settings | winchange, 1 s settle | 500 | 20 | 171 (97) | 45.9 | 71.6 | 50 | 0.82 | 13.3 |
| Settings | winchange, no settle | 0 | 20 | 12 (11) | 533.2 | 590.8 | 16 (10 to 28) | 27.43 | 15.5 |
| Launcher | cold | 0 | 30 | 61 (59) | 6.4 | 17.0 | 6 | 1.09 | 14.9 |
| Launcher | warm | 0 | 30 | 61 (59) | 2.1 | 2.6 | 2 | 0.84 | 14.9 |

Prefetch strategies, cold, bound 0. Yield is the growth of the client cache per round trip.

| Screen | Strategy | Runs | Round trips p50 (range) | Wall p50 ms | Wall p95 ms | ms per round trip p50 | Yield p50 / max |
|---|---|---|---|---|---|---|---|
| Compose Big | bfs-u | 10 | 465 (465 to 466) | 352.2 | 397.4 | 0.71 | 1 / 50 |
| Compose Big | dfs-u | 10 | 464 | 353.2 | 461.7 | 0.72 | 1 / 50 |
| Compose Big | hybrid-u | 10 | 465 | 339.4 | 383.9 | 0.68 | 1 / 50 |
| Compose Big | hybrid (the `getChild(i)` default) | 10 | 509 (479 to 536) | 391.6 | 456.9 | 0.71 | 1 / 51 |
| Compose Big | bfs | 10 | 533 (466 to 537) | 385.6 | 458.1 | 0.69 | 1 / 51 |
| Compose Big | siblings + bfs-u | 10 | 459 (459 to 460) | 1242.3 | 1359.6 | 2.64 | 1 / 50 |
| Compose Big | none | 10 | 574 | 411.9 | 440.3 | 0.69 | 1 / 1 |
| Expo Big | dfs-u | 5 | 492 | 372.3 | 473.3 | 0.72 | |
| Settings | dfs-u | 10 | 25 | 28.1 | 31.8 | 1.03 | |
| Settings | none | 10 | 172 | 108.0 | 120.4 | 0.61 | |

What the tree numbers show:

- **Windows.** Every settled screen has two interactive windows: the status bar (28 nodes) and the app. Gesture navigation shows no navigation bar window.
- **Visible against total.** `uiautomator dump` keeps only nodes visible to the user, so it reports 541 for the Compose Big app window, which has 545 nodes; Expo Big has 1021 app nodes with 1020 visible. Settings has 143 app nodes of which 70 are visible: its RecyclerView keeps laid-out rows beyond the screen. The virtualized lists expose little beyond the screen (Compose List 60 app nodes, 56 visible; Expo List 74, 71 visible).
- **Warm reads cost one round trip per window.** `AccessibilityWindowInfo.getRoot(strategy)` passes `bypassCache = true` (`AccessibilityWindowInfo.java` L256 at `android-16.0.0_r1`), and a bypassing request drops prefetch (`AccessibilityInteractionClient.java` L645). Everything below the roots comes from the cache: 573 nodes in 3.1 ms.
- **Cold reads cost one round trip per child beyond the 49th.** A request prefetches at most 50 nodes, and only descendants of the requested node, or, with `FLAG_PREFETCH_SIBLINGS`, the parent's children starting again from the first. A parent with n > 50 children therefore costs about one round trip per child after the first 49, whatever the strategy: the 500 cells under one node cost 465 round trips for 573 nodes, and the Expo grid, whose cells each carry a text child that comes along with its cell, 506 for 1049. The design's "at least ten round trips" for a 500-node read is 46 times too low for this layout. Sibling prefetch is slower still, because every request rebuilds up to 50 siblings that are already cached.
- **Strategy matters on View trees.** On Settings, depth-first prefetch halves the round trips (25 against 50); without prefetch a walk takes one round trip per node (172).
- **A round trip costs 0.7 ms on a static screen**, 2.3 ms while the app animates (both Anim tabs), and 27 ms while SystemUI animates: a walk during the shade's collapse (no settle) sees only the shade window and needs half a second for 12 nodes.
- **A real window change empties the cache.** After the shade closes and settles, a walk costs exactly the cold count (50).
- **The idle bound.** On static screens `waitForIdle(100, 500)` returns at once. On the Expo Anim tab, whose spinner emits a content change about every 101 ms, it returned after 82 ms (p50) instead of timing out: a 100 ms gap between two events of an endless animation passes for idle. Compose's spinner emits no events at all, so the wait returns at once there too.

### Cache staleness

`stale` performs an action, waits until no event arrived for 300 ms (at most 3 s), walks over the client cache (warm), clears the cache and walks again (cold), and compares the two node multisets. A round is stale when they differ.

| Fixture | Actions | Rounds | Stale rounds | Rounds without any event | Resource ids of the differing nodes |
|---|---|---|---|---|---|
| Compose | tap `anim-toggle` | 6 | 5 | 5 | `anim-toggle`, spinner child |
| Compose | tap `tab-login`, `tab-anim` in turn | 6 | 5 | 5 | every node of both tabs |
| Compose | swipe up, swipe down on List | 6 | 6 | 5 | `row-1` to `row-28` |
| Compose | tap `row-1`, `detail-back` in turn | 4 | 3 | 3 | `detail-1`, `detail-back`, `row-1` to `row-16` |
| Expo | tap `anim-toggle` | 6 | 2 | 0 | `anim-spinner` and its child only |
| Expo | tap `tab-login`, `tab-anim` in turn | 6 | 3 | 0 | `anim-spinner` and its child only |
| Expo | swipe up, swipe down on List | 6 | 0 | 0 | none |
| Expo | tap row 2, back, in turn | 4 | 0 | 0 | none |
| Settings | swipe up, swipe down | 4 | 0 | 0 | none |

- **Compose sends no semantics events to a UiAutomation client.** In Compose UI 1.11.4, `AndroidComposeViewAccessibilityDelegateCompat.isEnabled` is `accessibilityManager.isEnabled() && getEnabledAccessibilityServiceList(FEEDBACK_ALL_MASK).isNotEmpty()` (read from the fixture's `ui-android-1.11.4` bytecode), and `AccessibilityManagerService.getEnabledAccessibilityServiceList` returns bound services only (L1493 to L1531 at `android-16.0.0_r1`), never a UiAutomation registration. The few Compose events seen came on the first taps after an attach, with sources that are real Views without virtual ids, which fits Material ripple host views being added; after those, nothing. So after a tab switch the warm walk still returns the previous tab, after a toggle the old checked state, and after a scroll the old rows.
- **View and React Native windows stay current.** Every Expo and Settings action produced events, and in all 26 rounds the cached tree matched a cold read apart from the Expo spinner, which rotates between the two walks (5 rounds).
- **The quiet wait fails on Compose too.** Without events a quiet window passes while a list still flings, and a tap during a fling only stops it; the first run of the Compose list-to-detail rounds failed that way.
- Enabling a bound accessibility service to watch Compose start sending events was not tried: it changes device-global state on the shared emulator.

### Tap

A tap is DOWN then UP at one point. The acknowledgement runs from the start of the DOWN injection to the return of the UP injection. Before each tap the spike waits until no event has arrived for 150 ms; after it, the spike waits up to 1 s for the first `TYPE_WINDOW_CONTENT_CHANGED` stamped after the DOWN. Target: the Expo List tab, alternating row 2 (which opens its detail) and the lower half of Back, so that every tap changes the screen and emits events. 200 taps per method in two runs of 100, the fixture relaunched before each run, 1-minute load 3.7 to 7.3.

| Method | Accepted | Ack p50 ms | Ack p95 ms | Ack max ms | Over 50 ms | DOWN / UP p50 ms | First content change p50 / p95 ms |
|---|---|---|---|---|---|---|---|
| `InputManagerGlobal.injectInputEvent(e, WAIT_FOR_RESULT)` | 200 of 200 | 1.90 | 3.11 | 5.7 | 0 | 1.20 / 0.66 | 30 / 32 |
| `InputManagerGlobal.injectInputEvent(e, WAIT_FOR_FINISH)` | 200 of 200 | 5.20 | 7.40 | 8.5 | 0 | 3.90 / 1.24 | 30 / 32 |
| `InputManagerGlobal.injectInputEvent(e, ASYNC)`, 50 taps, no acceptance | always true | 1.29 | 2.19 | 2.4 | 0 | 0.68 / 0.48 | 30 / 32 |
| hidden `UiAutomation.injectInputEvent(e, true, false)` | 200 of 200 | 35.28 | 57.16 | 494.8 | 39 | 24.93 / 11.51 | 47 / 64 |
| public `UiAutomation.injectInputEvent(e, true)` | 200 of 200 | 45.24 | 57.64 | 61.2 | 65 | 25.38 / 17.41 | 47 / 64 |

"First content change" is the app's event time minus the DOWN time; the listener received the event about 1 ms later (p50).

- **Only the `InputManager` modes meet the 50 ms gate.** `WAIT_FOR_RESULT` acknowledges in 1.9 ms (p50). The hidden three-argument overload with `sync = true` is `WAIT_FOR_FINISH` plus `syncInputTransactions` before DOWN and after UP: 30 ms more than `WAIT_FOR_FINISH` alone, and 39 of 200 taps over the gate. The public overload also waits for window animations, although none ran here: 10 ms more again, and 65 of 200 taps over the gate.
- **The slow paths also delay the tap itself.** The DOWN reaches the app only after the first `syncInputTransactions` returns, so the first content change comes 17 ms later than with the `InputManager` modes.
- **`WAIT_FOR_FINISH` returns after the app has handled each event**, in 5.2 ms here; it lasts as long as the app's input handling does.
- **Compose.** On the Compose Anim toggle, 50 taps per method at a 1-minute load of 4.8 and 8.0: public 30.1 ms p50 and 54.0 ms p95 (6 of 50 over the gate), `WAIT_FOR_RESULT` 2.2 ms and 9.1 ms. Within 300 ms of these 100 taps a single content change arrived, 217 ms after one tap, and the spike does not record its source; the events recordings below show no event from the Compose app at all.

### Events

`events` records every accessibility event for 15 s. On the List tabs and Settings the host scrolls with `adb shell input swipe`, up and down about every 0.8 s; on the Anim tabs the spinner animates on its own. A batch is a run of content changes stamped within 5 ms of each other, and the gaps are measured between batch starts. 1-minute load 9 to 11.

| Scenario | Events | Content changes | Gap p50 / p95 / max ms | Naming a source | SUBTREE flag | Sources | Generation to listener p50 / p95 ms |
|---|---|---|---|---|---|---|---|
| Settings, scrolled | 251 | 140 | 113 / 143 / 281 | all 140 | 140 | the RecyclerView and the ScrollView around it | 3.3 / 7.8 |
| Expo List, scrolled | 236 | 127 | 116 / 147 / 187 | all 127 | 126 | the list's ScrollView; the status bar's signal icon once | 3.7 / 8.2 |
| Expo Anim, spinner running | 146 | 145 | 101 / 118 / 127 | all 145 | 145 | the spinner `ImageView` (`anim-spinner`) | 2.1 / 8.5 |
| Compose List, scrolled | 5 | 3 | | | 0 | SystemUI only (clock, signal icon) | |
| Compose Anim, spinner running | 1 | 1 | | | 0 | SystemUI only (signal icon) | |

- **Cadence.** Views and React Native send content changes about every 100 ms during a continuing change (the platform's 100 ms recurring-event interval): gaps p50 101 to 116 ms, p95 118 to 147 ms. The gaps under 90 ms (21 in Settings, 20 in Expo) are second events 8 to 17 ms after the first, from the same burst.
- **Longest gaps.** 10 of the 265 scroll gaps exceed 150 ms and 2 exceed 200 ms (209 and 281 ms, both in Settings). Every gap over 150 ms falls where no scroll event arrived either, at the reversals of the host's swipes, where the list may have stopped. The spinner's gaps stay under 130 ms.
- **Named subtree.** Every content change names a source node, and all but one carry `CONTENT_CHANGE_TYPE_SUBTREE`. The source is the smallest view that contains the change: the scrolling container for a scroll, which is the whole list, and the spinner for the animation.
- **Latency.** Events reach the listener 2 to 4 ms after the app stamps them (p50), 8 ms at p95.
- **Compose.** Fifteen seconds of scrolling or of the endless spinner produce no event from the app at all.

### Screenshot

`takeScreenshot()` on the Compose Big screen, 20 runs after 2 warm-up runs, 1-minute load 34.9:

| Measure | p50 | p95 |
|---|---|---|
| `takeScreenshot()` (display capture and the `ARGB_8888` copy) | 23.7 ms | 31.0 ms |
| JPEG encode at quality 80 with `Bitmap.compress` | 12.1 ms | 12.9 ms |
| JPEG size | 295,803 bytes | |

Frozen at 90 degrees with `setRotation(ROTATION_FREEZE_90)`, the capture is 2400x1080 and upright (checked by eye), and took 29.4 ms. The rotation settings: `disconnect()` restores rotation to the state the connection recorded at connect (`UiAutomationConnection.restoreRotationStateLocked` at `android-16.0.0_r1`). In a retest without the spike's own restore, `disconnect()` turned auto-rotate back on but left `user_rotation` at 1. The first landscape run, which froze the display at 90 and then at 0 degrees before disconnecting, left auto-rotate off (`accelerometer_rotation` 0) until it was restored by hand. The agent should therefore undo any rotation it sets itself. With the Expo fixture in front, whose manifest fixes portrait, the capture at 90 degrees stayed 1080x2400.

### Socket

The agent (`socket`) listens on or connects to an abstract Unix socket and requires the per-launch token, passed in its environment, as the first frame; frames are a 4-byte length and the payload. The host tool (`host/`, std-only Rust) times 20 connections per path with 200 echoes of 64 bytes each, at a 1-minute load of 4.0. Connect is the host's TCP connect (for reverse, the agent's `connect()`); handshake runs from the start of the connect to the agent's "ok" (for reverse, measured by the agent from its `connect()` to the host's "ok").

| Path | Works | Setup (`adb` command) | Connect p50 / p95 | Handshake p50 / p95 | Echo p50 / p95 |
|---|---|---|---|---|---|
| `adb forward tcp:0 localabstract:<name>`, agent listens | yes | 17.2 ms | 0.26 / 0.41 ms | 2.66 / 3.33 ms | 1.54 / 1.92 ms |
| `adb reverse localabstract:<name> tcp:<port>`, agent connects out | yes | 20.0 ms | 0.04 / 0.06 ms | 2.56 / 3.69 ms | 1.54 / 1.93 ms |
| adb server service stream: `host:transport:<serial>`, then `localabstract:<name>`, agent listens | yes | none | 1.58 / 1.91 ms | 3.10 / 3.71 ms | 1.51 / 2.09 ms |

- No SELinux denial was logged during any path (`logcat -b all`, `avc: denied` lines naming adbd or shell: 0). The peer of every accepted connection, forward and service stream alike, is adbd: `SO_PEERCRED` gives uid 2000, pid 541.
- A wrong token is refused on both listening paths: the agent closes the connection without replying.
- With nothing listening on the device, a forward still accepts the host's TCP connection and the host learns of the failure only as EOF after its first write, while the service request fails at once with `FAIL` and the message `closed`. This repeats spike S6's finding independently.

## Verdict

Physical devices and non-Pixel OEM builds were not measured; everything below holds for the API 36 emulator.

### D2, shell-user ownership: go

The construction D2 describes works unchanged on API 36. Every hidden member is present, the role is taken in 18 ms (p50) after a 120 ms `app_process` start, it is exclusive, and it is released by `disconnect()` in 5 ms or by process death within one `dumpsys` poll. No package, instrumentation or prompt is involved. Three corrections:

1. **Acceptance path.** Only `InputManagerGlobal.injectInputEvent(event, WAIT_FOR_RESULT)` meets the 50 ms gate (p50 1.9 ms, p95 3.1 ms, none of 200 taps over 50 ms). The hidden three-argument `UiAutomation.injectInputEvent` is not an acceptance path: with `sync = true` it still calls `syncInputTransactions` before DOWN and after UP and waits for the app to finish each event (p50 35.3 ms, p95 57.2 ms, 39 of 200 taps over 50 ms), and with `sync = false` it injects ASYNC, whose result is always true. The public overload costs more still: p50 45.2 ms, p95 57.6 ms, 65 of 200 taps over 50 ms.
2. **Attach readiness.** For 9 to 55 ms after `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` is set, `getWindows()` is empty or lists one window of two, so the first snapshot must wait for an active window.
3. **Conflict reporting.** The refused attach is the only signal, and the binder proxy in its message is the holder's, so it names no process. `uiautomator dump` refused the same way dies by SIGKILL (status 137) and prints nothing. Naming the holder relies on `ps`, where an `app_process` holder shows the name it gave itself with `setArgV0` (`uiautomator` for the platform tool); the agent should rename itself too.

Gated tasks: 4.2 should test conflicts against a lasting holder such as `uiautomator events`, since a conflicting `uiautomator dump` dies within milliseconds; 4.6 uses `WAIT_FOR_RESULT`; 4.8 can rely on `takeScreenshot()` returning an upright 2400x1080 image in landscape, provided the app in front allows rotation.

### D3, socket direction and token: go

S1 confirms D3 as rewritten from spike S6, with independent measurements. Under this emulator's SELinux policy adbd reaches the shell-owned abstract socket through a forward rule and through an adb server service stream, and the agent can connect out through a reverse rule; each path gives a token handshake near 3 ms and a 64-byte echo of 1.5 ms (p50), and no denial was logged. With no agent listening, the service stream fails at once with `closed`, while a forward rule accepts the connection and yields EOF only after the first write, which is why D3's service stream is the better path. The token check works: a wrong first frame gets the connection closed without a reply. One addition: `SO_PEERCRED` shows every legitimate peer as uid 2000 (adbd runs as the shell user), so the agent can refuse any other uid before it reads the token. S1 did not test whether SELinux lets an app's domain connect to a shell-domain socket at all; the token and the uid check cover that case either way.

### D5, mirror update strategy: no-go as written

Three measured facts contradict the cost model and the update path:

1. **Cold cost follows fan-out.** A request prefetches at most 50 nodes, and only below the requested node or, for siblings, from the parent's first child again. A parent with more than 50 children therefore costs about one round trip per extra child, whatever the strategy: 465 round trips and 349 ms for the 573 nodes of the Compose 500-cell screen, 506 and 390 ms for the Expo one with 1049 nodes. The design's floor of ten round trips is 46 times too low there. On View trees depth-first prefetch halves the count (Settings: 25 against 50 round trips).
2. **Warm reads are cheap but always touch each window.** Window roots bypass the cache, so every read pays one round trip per window: 3 ms for 573 nodes. A window change empties the cache; a round trip costs 0.7 ms on a static screen and up to 27 ms while SystemUI animates.
3. **Compose windows get no events.** Compose sends semantics events only while `getEnabledAccessibilityServiceList` is non-empty, and that list never contains the UiAutomation connection. With only the agent attached, the cached tree of a Compose window went stale after 19 of 22 actions (tab switches, toggles, scrolls, navigation), while the cached View and React Native trees matched a cold read after all 26, apart from an animating spinner. D5's subtree deltas therefore do not exist for Compose, the most common toolkit of new Android apps; the "not reliably invalidated" finding of agent-device is this gate, not a Navigation 3 quirk.

The mirror keeps the event path for View-based windows (Views and React Native), and for windows that host `AndroidComposeView` clears that window's cache and re-reads it after each injected action and on a poll, diffing the result. Gated tasks: 4.5 cannot pass on the Compose fixture as written, because no event names the edited field; 4.3 should count total and visible nodes separately; 7.1 needs a re-read source besides recorded events; 7.4's continuous-animation test gets no tree signal from the Compose spinner at all.

### D8, quiet window: 150 ms is too short

The quiet window has to outlast the gap between two batches of one continuing change. S1 measured those gaps at 101 to 116 ms p50 and 118 to 147 ms p95. The longest gap inside a scroll was 187 ms, plus a 281 ms gap at a swipe reversal where the list may have stopped. D8's 150 ms default would declare the screen settled inside continuing motion: 10 of 265 scroll gaps exceed it, against 2 over 200 ms, both at reversals. agent-device's 100 ms window is worse: `waitForIdle(100, 500)` declared the endless Expo spinner idle after 82 ms. S1 proposes 200 ms, the measured cadence plus margin and still short against the 3 s maximum. Windows hosting Compose send no events, so their tree half of settle comes from re-reads and the frame hash carries it; task 7.4 should test both.

### Proposed replacement sentences for design.md

For task 2.11. Each item names the passage it replaces.

- **D2**, replacing "Input acknowledgements use the hidden three-argument `injectInputEvent` overload (API 31+) or `InputManager.injectInputEvent` with `WAIT_FOR_RESULT`, whose return value is the accepted flag the spec asks for; the public synchronous overload waits for animations around touch-down and touch-up and would blow the 50 ms gate.":
  > Input acknowledgements use `InputManagerGlobal.injectInputEvent(event, INJECT_INPUT_EVENT_MODE_WAIT_FOR_RESULT)`, whose return value is the accepted flag the spec asks for; S1 measured a tap (DOWN and UP) at 1.9 ms p50 and 3.1 ms p95 on the API 36 emulator. Neither `UiAutomation` overload qualifies: with `sync = true` both call `syncInputTransactions` around the tap and wait for the app to finish each event (public: 45.2 ms p50, 65 of 200 taps over 50 ms; hidden three-argument: 35.3 ms p50, 39 of 200 over), and with `sync = false` the injection is ASYNC and always reports success. The agent sets `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` after connecting and waits for an active window before its first snapshot, because the window list stays empty or partial for up to 55 ms. A refused attach raises `IllegalStateException` ending in `already registered!`, whose binder proxy is the holder's and names no process, so the conflict report names the likely holder from `ps`, where `app_process` holders appear under their `setArgV0` names.
- **D3**, replacing "The agent listens on an abstract Unix socket named with a per-launch id and speaks first, and the handshake carries a per-launch token.":
  > The agent listens on an abstract Unix socket named with a per-launch id and speaks first. The handshake carries a per-launch token: the agent closes a connection whose first frame from the host is not that token, and refuses a peer whose `SO_PEERCRED` uid is not 2000, the uid adbd runs as; spike S1 measured a 3 ms token handshake and a 1.5 ms echo on the service-stream, forward and reverse paths alike.
- **D5**, replacing the first words "Android: the agent forwards" with "Android, View-based windows (Views and React Native): the agent forwards", and the sentences from "Costs the research note established, which S1 measures:" through "D8's quiet window is retuned from S1's measurements." with:
  > S1 measured the costs on the API 36 emulator. A round trip costs about 0.7 ms on a static screen and up to 27 ms while SystemUI animates; requests run on the app's UI thread. Prefetch returns at most 50 nodes, and only below the requested node or from the parent's first child, so a cold read costs about one round trip per child beyond the 49th of any parent: 465 round trips and 349 ms for the 573-node Compose fixture screen with 500 cells under one parent, 506 and 390 ms for the 1049-node React Native one, 50 and 38 ms for Settings, where depth-first prefetch halves the count. Window roots bypass the cache, so even a warm read costs one round trip per window (3 ms for 573 nodes), and a window change empties the cache. Jetpack Compose sends no accessibility events for its content to a UiAutomation client, because it sends them only while `getEnabledAccessibilityServiceList` is non-empty and that list never includes UiAutomation; S1 found the cached tree of a Compose window stale after 19 of 22 actions, and those of View and React Native windows current after all 26 apart from an animating spinner. The mirror therefore refreshes View windows from content-change events and subtree reads, and re-reads windows hosting `AndroidComposeView` in full, with the platform cache cleared, after each injected action and on a poll, deriving their deltas by diffing; the same full read serves resync on demand.
- **D8**, replacing "default 150 ms":
  > Settled means no tree delta and no frame hash change for a quiet window, default 200 ms, bounded by a maximum, default 3 s. Spike S1 measured Android content changes arriving every 101 to 116 ms (p50) during a continuing change, with gaps up to 187 ms inside a scroll, so a 150 ms window would end inside a change. Windows hosting Jetpack Compose send the agent no content changes; for them the tree signal comes from re-reads and the frame hash carries settle.
- **Risks**, replacing "[Event mirror misses in-place swaps and 100 ms event batching narrows the quiet window] → periodic and on-demand full re-read with cache clearing; quiet window retuned after S1.":
  > [Compose windows send no accessibility events to UiAutomation, and a full re-read of a wide Compose screen costs hundreds of round trips] → re-read Compose windows after each action and on a poll, report their read cost in snapshot metadata, and let the frame hash carry settle there.

### Not measured

- A physical device and a non-Pixel OEM build (research note, risk 9): none was attached.
- Whether enabling a bound accessibility service makes Compose send events: it changes device-global state on the shared emulator.
- Compose Navigation 3: the fixture has no navigation library. Its tab swaps inside one `AndroidComposeView` are the closest case and go stale the same way.
- The keyboard window, TalkBack suppression under `FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES` (the spike always set it), and the connect timeout path.
