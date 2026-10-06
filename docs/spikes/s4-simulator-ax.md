# S4: Simulator accessibility tree without XCTest

Spike S4 (task 2.7) measures the two ways D6 considers for reading a Simulator's accessibility tree without XCTest. The first is a reader spawned into the guest that snapshots the tree through the runtime's private XCTAutomationSupport client, as agent-device's Objective-C bridge does, ported here to a Rust binary built for `aarch64-apple-ios-sim`. The second is the host-only translator path: `AXPTranslator`, answered through `-[SimDevice sendAccessibilityRequestAsync:completionQueue:completionHandler:]`. The Rust port worked, so no Objective-C reference was needed; it was built only for a parity check. The verdict is go for the spawned Rust reader, launched through CoreSimulator rather than `xcrun simctl spawn`, and no-go for the translator as the tree reader, which stays the fallback D6 names.

## Setup

- Host: Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2 (25G83). Other agents' builds and benchmarks, an Android emulator and two other booted Simulators ran throughout, including spike S3's timing runs. Before each timed series, and before each screen's reads, `measure.sh` waited until the 1-minute load average was below 6 with no gradle, xcodebuild or kotlinc process running, for up to 20 minutes; the translator's reads of a screen followed its guest reads without a second wait. Every table gives the 1-minute load its step started under.
- Xcode 27.0 (27A266a). CoreSimulator 1171.7. AccessibilityPlatformTranslation 545.18 on the host. Runtime iOS 26.5 (23F77).
- Devices, all iPhone 17 (402x874 points) booted headless; DeviceHub.app and Simulator.app never ran:
  - `mobile-agent-spike-ax` carried the timed reads, the warm spawns and the relaunch series.
  - `mobile-agent-spike-ax2` was erased and booted three times for the accessibility-enablement observations.
  - Both were deleted after the relaunch series; a new `mobile-agent-spike-ax`, created the same way, carried the cold series.
- Apps: the SwiftUI fixture (Debug, `dev.mobileagent.fixture.swiftui`) and the Expo fixture (Release, `dev.mobileagent.fixture.expo`), both built on main and opened on the wanted tab with `FIXTURE_INITIAL_TAB`. The in-app alert and the location permission alert were opened without HID input, by the reader's `press` method, an accessibility activation through `-[AXElement press]`, after `xcrun simctl privacy <udid> reset location <bundle>`; the permission alert was dismissed with its "Don’t Allow" button the same way.
- Accessibility preferences. No `defaults` process was spawned, and `CoreSimulatorBridge` was never restarted.
  - On the first `mobile-agent-spike-ax`, the reader's first reads, which assert automation mode, wrote `AccessibilityEnabled`, `ApplicationAccessibilityEnabled` and `AutomationEnabled` into `data/Library/Preferences/com.apple.Accessibility.plist`; every timed run came after that.
  - The cold-series device had the three keys written into that file with `plutil` before its first boot.
  - The reader asserts automation mode before every read, as agent-device's does.
- Rust 1.95.0 with the `aarch64-apple-ios-sim` target from rustup. Crates, all added with `sfw cargo add`, from `Cargo.lock`:
  - Both binaries: `objc2` 0.6.4 with the `exception` feature, `objc2-foundation` 0.3.2, `libc` 0.2.189, `serde` 1.0.229, `serde_json` 1.0.151.
  - The host client also uses `block2` 0.6.2 and `dispatch2` 0.3.1.
  - The `exception` feature compiles `objc2-exception-helper` 0.1.1's C file with `cc` 1.5.1, which found the iOS Simulator SDK by itself.
  - Both profiles set `panic = "unwind"`.
- Guest binary (release):
  - 767,328 bytes, Mach-O arm64, `LC_BUILD_VERSION` platform `IOSSIMULATOR`, minos 14.0, SDK 27.0.
  - Links Foundation, CoreFoundation, libobjc, libSystem and libiconv only.
  - Its only signature is the one the linker adds to every arm64 binary (`flags=0x20002(adhoc,linker-signed)`); nothing ran `codesign`.
- Code: `spikes/s4-simulator-ax` at `5e6fb2e`. `wire` holds the framing, the tree model and the statistics; `guest-reader` is the reader; `host-client` spawns and times it and holds the translator path.
- Rerun:
  - Build: `cargo build --release -p host-client` and `cargo build --release -p guest-reader --target aarch64-apple-ios-sim`.
  - Measure: `./measure.sh reads|spawn|relaunch|cold <udid> <out-dir>` on a booted device with both fixtures installed; `QUIET_LOAD` sets the load threshold (default 8; these runs used 6).
  - Parity: `./measure.sh parity <udid> <out-dir>` builds agent-device's bridge through `reference.sh` (which needs `gh`) and times it against the reader.
  - Tests: `cargo test` runs the `wire` tests and the host loader test.

Symbols the guest resolves, and the image each came from inside the iOS 26.5 runtime root:

| Symbol | Kind | Image |
|---|---|---|
| `XCTAccessibilityFramework`, `XCAccessibilityElement` | classes | `/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework/XCTAutomationSupport` |
| `XCTDefaultSnapshotParameters`, `XCAXAccessibilityAttributesForStringAttributes` | C, `dlsym` | XCTAutomationSupport |
| `AXElement` | class | `/System/Library/PrivateFrameworks/AXRuntime.framework/AXRuntime` |
| `AXValueGetValue`, `AXValueGetTypeID` | C, `dlsym` | AXRuntime |
| `_AXSAutomationEnabled` | C, `dlsym` | `/usr/lib/libAccessibility.dylib` |
| `AXSettings` | class | `/System/Library/PrivateFrameworks/AccessibilityUtilities.framework/AccessibilityUtilities`, which XCTAutomationSupport loads |

Selectors sent in the guest: `-initForRemoteAccess`, `-userTestingSnapshotForElement:options:error:`, `+elementWithProcessIdentifier:`, `-AXUIElement`, `+[AXSettings sharedInstance]`, `-setAutomationEnabled:`, `+[AXElement primaryApp]`, `-pid`, `-label`, `+systemWideElement`, `-springBoardApplication`, `-currentApplications`, `+elementWithAXUIElement:` and `-press`.

`XCAXAccessibilityAttributesForStringAttributes` numbered the ten requested attributes, agent-device's list, as follows on iOS 26.5:
- 5003 element type, 5004 base type, 2001 label, 2006 value;
- 5043 placeholder, 5019 identifier, 2003 frame;
- 5058 automation type, 2004 traits, 5001 children.

`XCTDefaultSnapshotParameters()` returned `maxArrayCount`, `maxChildren` and `maxDepth` of 2147483647 and `traverseFromParentsToChildren` 1. The reader overrides the three bounds with 64 levels and 10,000 nodes.

Host translator path, from AccessibilityPlatformTranslation: `AXPTranslator +sharedInstance`, `-setBridgeTokenDelegate:`, `-frontmostApplicationWithDisplayId:bridgeDelegateToken:` and `-macPlatformElementFromTranslation:`; `AXPTranslationObject -setBridgeDelegateToken:` and `-pid`; `AXPTranslatorResponse +emptyResponse`; `AXPMacPlatformElement -translation`, `-accessibilityRole`, `-accessibilitySubrole`, `-accessibilityLabel`, `-accessibilityIdentifier`, `-accessibilityValue`, `-accessibilityFrame`, `-accessibilityAttributeValue:` with `AXTraits`, and `-accessibilityChildren`. The token delegate is a class defined in Rust with objc2's `define_class!`. From CoreSimulator: `-sendAccessibilityRequestAsync:completionQueue:completionHandler:` for the translator, and `-spawnWithPath:options:terminationQueue:terminationHandler:pid:error:` to start the reader without `xcrun`.

## Measurements

Timings are wall-clock milliseconds from `Instant` on the host. The reader stamps its own start points with the wall clock (a spawned guest process runs on the host kernel and reads the same clock) and `proc_pidinfo` gives the kernel's start time of the process, so spawn-to-ready splits into its parts. Read times for the guest path are host round trips over the socket: request written to response read. Read times for the translator are the whole read: the frontmost-application request plus the walk. Percentiles are nearest-rank. Timed reads exclude 3 warm-up reads.

### Tests, red then green

- `f01ff69` stubbed every `wire` function with `unimplemented!`: 34 of 39 tests failed, and the 5 that passed check serde attributes declared on the tree model. The tests cover length-prefixed framing (big-endian prefix, empty and oversized frames rejected before writing, truncation inside the prefix and inside the payload, reassembly from one-byte reads), the request and response envelopes, the tree model's JSON (absent fields omitted, frames as arrays, traits keeping all 64 bits), node counting, depth, pre-order walks, identifier prefixes and families, the letterbox transform, `XCUIElementType` names and nearest-rank percentiles. The red test binary built for `aarch64-apple-ios-sim` also ran inside the Simulator under `xcrun simctl spawn`, and its statistics tests failed the same way.
- `187de04` made all 39 pass and added `guest_role`, whose test was written and seen failing after the first live read showed that iOS 26.5 reports automation type 1 for the application element and 2 for a `UIWindow`, where `XCUIElementType` says Other and Application. 40 pass.
- The host client's 2 tests (exception guard, framework loading) pass, and `cargo clippy --all-targets -D warnings` is clean for both targets.

### Spawn to ready

Spawn-to-ready runs from the spawn call to the reader's answer to `hello` on its socket. The reader binds the socket only after loading its runtime, so the answer proves it can read. Every run then read a Big screen: the SwiftUI one once in the warm and cold runs, and each fixture's until `cell-500` appeared in the relaunch runs.

| Route | Condition | n | Ready p50 | Ready p95 | Max | First tree after spawn, p50 / p95 | Load |
|---|---|---|---|---|---|---|---|
| CoreSimulator spawn | warm, the same binary again | 20 | 291.9 | 316.7 | 318.3 | 426.2 / 450.7 | 5.95 |
| `xcrun simctl spawn` | warm | 20 | 568.0 | 607.5 | 673.3 | 703.8 / 757.0 | 5.55 |
| CoreSimulator spawn | a new copy of the file each run | 10 | 388.4 | 542.7 | 542.7 | 593.9 / 694.6 | 5.90 |
| CoreSimulator spawn | right after an app relaunch, SwiftUI | 20 | 297.3 | 338.8 | 356.8 | see below | 5.92 |
| CoreSimulator spawn | right after an app relaunch, Expo | 20 | 325.9 | 342.6 | 352.4 | see below | 5.82 |
| `xcrun simctl spawn` | right after an app relaunch, SwiftUI | 20 | 643.1 | 685.4 | 759.3 | see below | 5.68 |
| `xcrun simctl spawn` | right after an app relaunch, Expo | 20 | 749.1 | 1183.0 | 1293.1 | see below | 5.71 |
| CoreSimulator spawn | first spawn after a device boot | 3 | 298.8 | 298.8 | 298.8 | 545.1 / 560.8 | 5.53 to 5.95 |
| `xcrun simctl spawn` | first spawn after a device boot | 3 | 567.6 | 653.1 | 653.1 | 829.4 / 913.1 | 5.53 to 44.23 |

The cold rows are single spawns, each after `simctl shutdown`, `simctl boot` and terminal boot status. Through CoreSimulator the reader was ready in 281.6, 298.8 and 298.8 ms: a boot costs the reader nothing measurable. Through `simctl` it was ready in 544.1, 567.6 and 653.1 ms; the 567.6 ms spawn started at load 44, after `measure.sh` gave up waiting for a quiet host. The first read after a boot took 234 to 262 ms instead of about 130. That is the first read of a new app process, not the boot: the first warm run, the first read of that app process, took 236.5 ms and the other 19 took 126.8 to 137.0.

Where the time goes, p50 of the 20 warm CoreSimulator spawns:

| Point | ms after the spawn call |
|---|---|
| kernel starts the process | 1.0 |
| `-spawnWithPath:...` returns | 105.3 |
| reader's `main` | 202.8 |
| runtime loaded | 280.2 |
| socket listening | 291.6 |
| `hello` answered | 291.9 |

- About 200 ms pass between the kernel starting the process and `main`. That is the Simulator's cost of starting any process, not the reader's: a C program that only prints its start time, built with `xcrun -sdk iphonesimulator clang`, took 196 to 201 ms from start to `main` in four runs.
- Loading the runtime takes 78 ms. `dlopen` of XCTAutomationSupport takes 62 to 75 ms: it weak-links UIKit and more, and brings in AccessibilityUtilities. `+[AXSettings sharedInstance]` takes 5 to 7 ms and `dlopen` of AXRuntime 2 to 3 ms. Resolving symbols, creating the remote-access client and numbering the attributes take under 1 ms together.
- `xcrun simctl spawn` starts the process 295 ms after the call (p50, n=20). That is `xcrun` resolving the developer directory plus `simctl` loading CoreSimulator, which spike S2 put at 323 to 333 ms per process. The daemon already has CoreSimulator loaded and pays none of it.
- The first run of a new copy of the file is slower before `main`: 278 ms p50 against 203, and up to 362. That is the cost of a file the system has not seen yet; it recurs only when the binary changes.
- A resident reader's peak resident size was 120 to 125 MB. Its CPU time per read was 1.7 to 7.3 ms on the small screens, 23.6 ms on the SwiftUI Big screen and 19.3 ms on the Expo one.

### Warm reads per screen

One reader per screen and fixture, spawned through CoreSimulator, 3 warm-up reads then 30 timed ones; the translator path did the same on the same screen right after. Payload is the length of the response frame for the guest path. The translator's traffic is XPC requests, counted per read; its payload column is the length of the same tree model serialized on the host, for comparison. Cells are the `cell-` identifiers on Big and the `row-` identifiers on List; tabs are the `tab-` identifiers.

| Fixture | Screen | Path | p50 | p95 | Max | Nodes | Cells or rows | Tabs | Payload bytes | Requests per read | Foreground | Load |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| SwiftUI | Login | guest | 13.1 | 13.6 | 13.7 | 48 | | 5 | 7,056 | 1 | target | 4.74 |
| SwiftUI | Login | translator | 15.1 | 17.2 | 17.8 | 6 | | 0 | 849 | 43 | target | 4.74 |
| SwiftUI | List | guest | 51.0 | 54.1 | 56.3 | 227 | 16 | 5 | 30,611 | 1 | target | 4.76 |
| SwiftUI | List | translator | 46.8 | 57.3 | 58.9 | 18 | 16 | 0 | 2,148 | 127 | target | 5.18 |
| SwiftUI | Big | guest | 126.4 | 130.4 | 131.6 | 546 | 500 | 5 | 101,293 | 1 | target | 5.01 |
| SwiftUI | Big | translator | 1,032.4 | 1,263.8 | 1,295.7 | 503 | 500 | 0 | 84,481 | 3,522 | target | 5.33 |
| SwiftUI | System, in-app alert | guest | 20.3 | 21.2 | 22.2 | 89 | | 5 | 12,161 | 1 | target | 5.71 |
| SwiftUI | System, in-app alert | translator | 11.1 | 12.3 | 13.1 | 4 | | 0 | 513 | 31 | target | 5.71 |
| SwiftUI | System, permission alert | guest, the app | 1.1 | 1.1 | 1.1 | refused | | | | 1 | SpringBoard | 5.42 |
| SwiftUI | System, permission alert | guest, SpringBoard | 31.0 | 32.7 | 33.1 | 166 | | | 20,187 | 1 | | 5.86 |
| SwiftUI | System, permission alert | translator | 14.7 | 18.8 | 20.5 | 6 | | | 775 | 45 | SpringBoard | 5.42 |
| Expo | Login | guest | 9.8 | 10.2 | 10.4 | 21 | | 5 | 3,340 | 1 | target | 5.87 |
| Expo | Login | translator | 21.4 | 22.8 | 23.1 | 10 | | 5 | 1,560 | 71 | target | 9.65 |
| Expo | List | guest | 28.8 | 30.2 | 37.4 | 72 | 46 | 5 | 10,097 | 1 | target | 5.85 |
| Expo | List | translator | 106.8 | 122.4 | 133.2 | 52 | 46 | 5 | 6,586 | 365 | target | 5.85 |
| Expo | Big | guest | 241.9 | 248.5 | 252.4 | 528 | 500 | 5 | 82,739 | 1 | target | 5.33 |
| Expo | Big | translator | 1,067.8 | 1,250.3 | 1,489.2 | 507 | 500 | 5 | 71,506 | 3,550 | target | 4.89 |
| Expo | System, in-app alert | guest | 15.6 | 16.1 | 16.1 | 63 | | 5 | 8,310 | 1 | target | 4.47 |
| Expo | System, in-app alert | translator | 11.3 | 12.4 | 12.6 | 4 | | 0 | 480 | 31 | target | 8.67 |
| Expo | System, permission alert | guest, the app | 1.1 | 1.3 | 1.4 | refused | | | | 1 | SpringBoard | 5.32 |
| Expo | System, permission alert | guest, SpringBoard | 29.9 | 32.9 | 33.6 | 164 | | | 19,831 | 1 | | 5.32 |
| Expo | System, permission alert | translator | 13.9 | 15.2 | 15.3 | 6 | | | 765 | 45 | SpringBoard | 5.32 |

- Node counts were identical across the 30 reads of every row. Guest payloads varied by at most 3 bytes, the digits of the timing fields each response carries.
- No timed read needed a fallback. Every guest read took one snapshot request, none was rejected at its depth, no node withheld children, and nothing was truncated. No translator request went unanswered, and no element listed an ancestor as its child.
- The guest's time is the app's. The snapshot call is 95 and 97 percent of a read on the Big screens (119.6 of 126.4 ms on SwiftUI, 235.6 of 241.9 on Expo) and 60 to 90 percent on the others, where the two foreground checks, 3.3 to 4.7 ms per read, are most of the rest. Converting the snapshot into the tree model took at most 1.0 ms.
- The translator's time is its requests: about 7 per element for the 8 attributes read (role, subrole, label, identifier, value, frame, traits and children), at 0.29 to 0.37 ms each. The frontmost-application request alone took 1.4 to 1.7 ms, and 0.4 ms when SpringBoard owned the foreground.
- XCTest baseline (task 1.6): a snapshot of the SwiftUI Big screen took 0.66 to 0.71 s for the same 546 nodes the guest returns. The guest is 5.2 to 5.6 times faster at p50. The translator is slower than XCTest and returns 43 fewer nodes.

### After an app relaunch

Each run terminated the app, launched it on the Big tab through CoreSimulator, then either spawned a reader and read until the tree held `cell-500`, or read through the translator until it did. 20 runs per row.

| Fixture | Path | First usable tree after launch, p50 / p95 | After the reader's spawn, p50 / p95 | Not ready yet |
|---|---|---|---|---|
| SwiftUI | guest, CoreSimulator spawn | 1,537.4 / 1,592.9 | 1,316.3 / 1,374.3 | the snapshot call failed, 15 to 18 reads |
| SwiftUI | guest, `xcrun simctl spawn` | 1,527.1 / 1,548.8 | 1,312.4 / 1,331.4 | none: the first read, sent about 643 ms after spawn, blocked until the app answered |
| SwiftUI | translator | 4,038.4 / 4,441.5 | | a childless application element, 16 to 24 reads |
| Expo | guest, CoreSimulator spawn | 2,005.0 / 2,052.6 | 1,779.9 / 1,817.8 | the snapshot call failed, then 5 or 6 nodes while React Native mounts, 21 to 37 reads |
| Expo | guest, `xcrun simctl spawn` | 1,989.2 / 2,058.7 | 1,753.3 / 1,817.3 | 5 or 6 nodes, 0 to 22 reads |
| Expo | translator | 3,608.9 / 4,201.3 | | a childless application element, 34 to 43 reads |

Terminating and relaunching the app took 219 ms (SwiftUI) and 230 ms (Expo) at p50, until the new pid arrived. The first usable tree is the app's readiness: 1.5 s for the SwiftUI Debug build and 2.0 s for the Expo Release build, whichever way the reader was spawned, because a reader spawned through CoreSimulator is ready in about 300 ms, long before the app. The translator named the new app as frontmost from its first read but reported it as an application element without children for 16 to 43 consecutive reads, and its first full tree came 1.6 to 2.5 s after the guest's at p50, a full read of the Big screen included.

### Accessibility enablement

Single observations, each under a first-boot load burst (1-minute load 196 to 403), so the times are indicative only. The first three were on `mobile-agent-spike-ax2`, each after `simctl erase` and a boot:

- No preferences, translator first. The first `-frontmostApplicationWithDisplayId:bridgeDelegateToken:` took 3,094 ms and returned nil. Polling every 100 ms returned nil until 5.3 s after the first call, then the app's pid. Reads worked from then on. The preferences file then held `AccessibilityEnabled` and `ApplicationAccessibilityEnabled`, which nothing on the host had written. No bridge restart was needed. The same 3 s nil answer was seen on this device's very first boot too.
- No preferences, reader first. The first snapshot failed with XCTAutomationSupport error 5, `kAXErrorServerNotFound`. The next reader's three reads, seconds later, were refused because `+[AXElement primaryApp]` returned no application. A reader spawned less than a minute later read all 546 nodes, and the file then also held `AutomationEnabled`.
- The three keys written before boot. The translator's first frontmost request answered in 64 ms with the app's pid. On the next erase and boot, the reader's first spawn was ready in 1,032 ms (at load 333) and its first read returned all 546 nodes.
- On the cold-series device, with the keys written before its first boot, the first spawn was ready in 286 ms and the translator's first frontmost request answered in 44 ms, both under the same burst.

### Parity with agent-device's Objective-C reader

Run once, with the same steps as `measure.sh parity`, at 1-minute loads of 4.0 to 5.1. `reference.sh` fetched the `apple/snapshot-bridge` sources at agent-device's pinned commit `3660c7fa41` and built them with agent-device's own command (`xcrun --sdk iphonesimulator clang -arch arm64 -mios-simulator-version-min=15.0 -fobjc-arc -Wall -Wextra -framework Foundation -framework CoreGraphics`), a 99,056-byte binary. It spawned the bridge with `xcrun simctl spawn` and sent one `describe` request (depth 64, 10,000 nodes, automation mode) and then 30 timed ones. The Rust reader then read the same app process once and then 30 times, through its own `xcrun simctl spawn`.

| Screen | Objective-C bridge, p50 / p95 | Rust reader, p50 / p95 | Nodes, both | Bridge bytes | Reader bytes |
|---|---|---|---|---|---|
| SwiftUI Big | 130.6 / 136.5 | 121.3 / 131.0 | 546 | 219,184 | 101,293 |
| SwiftUI Login | 14.4 / 19.9 | 12.4 / 12.9 | 48 | 16,232 | 7,056 |
| Expo Big | 243.3 / 254.9 | 236.0 / 247.6 | 528 | 192,364 | 82,739 |
| Expo Login | 11.0 / 11.4 | 9.3 / 9.8 | 21 | 7,216 | 3,340 |

- The port returns the same nodes as the reference on every screen and is 1.7 to 9.3 ms faster at p50.
- The difference is serialization. The bridge's JSON keys each attribute by its `XC_kAXXCAttribute` name and writes each frame as an object: 2.2 to 2.3 times the bytes of the reader's tree model for the same tree.
- The bridge accepted a connection 499 to 555 ms after `xcrun simctl spawn`, a little sooner than the reader answered `hello` (531 to 558 ms), because it loads its runtime on the first request rather than before listening. Its first tree arrived 648 to 946 ms after the spawn.

## Fidelity

What each path returns for the fixtures' contract identifiers, and what it misses:

| Screen | Guest | Translator |
|---|---|---|
| SwiftUI Login | all 4 login identifiers and 5 tabs | the 4 login identifiers; no tabs |
| SwiftUI List | 16 rows, each a cell holding a button with two children, its text and its chevron image; 5 tabs | 16 rows, each a button without children; no tabs |
| SwiftUI Big | 500 cells, `big-count`, 5 tabs and their 5 images: XCTest's 546 nodes | 500 cells and `big-count` as a heading; no tabs |
| SwiftUI in-app alert | the alert's title, message and `inapp-alert-ok` (twice, UIKit's nested button, as XCTest reports it), plus the whole screen and tab bar under the alert | only the alert: title, message and `inapp-alert-ok` |
| Expo Login | all 4 login identifiers and 5 tabs | 3 of 4: `login-status` arrives as unidentified text; 5 tabs |
| Expo List | 46 mounted rows, 5 tabs | 46 rows, 5 tabs |
| Expo Big | 500 cells, `big-count`, 5 tabs | 500 cells and 5 tabs; `big-count` arrives as an unidentified heading |
| Expo in-app alert | title, message and an "OK" button without identifier (React Native cannot set one), plus the screen and tabs under the alert | only title, message and "OK" |
| Permission alert, both fixtures | the app read is refused as `foreground-not-target` in about 1 ms; SpringBoard's tree (164 to 166 nodes) holds the title, the message and "Allow Once", "Allow While Using App" and "Don’t Allow", without identifiers | SpringBoard is the frontmost application; its 6 elements are exactly the alert's title, message and three buttons |

- Hierarchy. The guest returns the whole view hierarchy the accessibility server snapshots, containers included, with the element's class (`SwiftUI.AccessibilityNode`, `_UITabButton`, `RCTParagraphComponentView`) and an `XCUIElementType` role. The translator returns what the server exposes to assistive technology: a flat list of leaf elements under the application, with AX roles (`AXButton`, `AXHeading`, `AXTextField` with subrole `AXSecureTextField`).
- Container children. Every element of the fixtures' SwiftUI content appears through both paths: the cells, rows, fields and header. The guest also returns SwiftUI's children inside them, such as each List row button's text and chevron image; the translator returns the button alone, as one combined element. What the translator misses outright in the SwiftUI app is the UIKit tab bar's content. It reports the bar as one `AXGroup` labelled "Tab Bar" with no children, so no `tab-` identifier reaches it on any SwiftUI screen, while the guest reads the five `_UITabButton` elements with their identifiers. This is the childless-container failure that baguette's and iosef's hit-test sweeps work around, at 1.5 to 2 s per read in baguette's case. No sweep was measured here. The Expo tab bar is five Pressables, which the translator does return.
- Composed text. React Native puts a `Text`'s identifier on an `RCTParagraphComponentView` whose child `RCTAccessibilityElement` carries the same label. The guest returns both. The translator returns only the child, so the identifier is lost.
- Modal content. Under an in-app alert the guest still returns the covered screen (89 nodes against the translator's 4), so the host must cut what the alert covers itself, as agent-device's ADR 0004 does with its modal-containment rule. Neither path types the alert as an alert: the guest reports its container as a Group of class `_UIAlertControllerPhoneTVMacView` labelled with the title, and XCTest's Alert type would be a promotion by class on the host.
- System UI. Both paths detect it. The guest compares `+[AXElement primaryApp].pid` with the target before and after each read. The translator compares the frontmost application's pid. Both can list the permission alert's buttons: the guest by snapshotting SpringBoard's pid, which `+[AXElement systemWideElement]` and `-springBoardApplication` give, and the translator directly, because SpringBoard is then its frontmost application.
- Frames, values and traits agree. Both report frames in iOS points, and the same elements carry the same frames (for example `cell-1` at 8, 98.33, 19.33 by 27.67) and the same trait bits. The translator's root frame was 0, 0, 402 by 874, so iosef's letterbox transform, which maps a Simulator window's frames to points, is the identity for a headless device. Empty text fields report their placeholder as value through both paths.
- Not covered: web content (no fixture has a web view), the status bar inside an app's tree (neither path has it; the guest sees it only in SpringBoard's tree), and several Simulators at once.

### Prior art: the keyboard's window and views marked modal

Gathered for tasks 10.21 and 10.22, after the Bluesky smoke test (`reports/10.4-rn-smoke.md`, third run, findings 1 and 2), from Xcode 27.0 with its iOS 27.0 Simulator SDK and the iOS 26.5 runtime, agent-device 0.21.14 (`622435ecc5`) and WebDriverAgent at `v11.0.0` and `d17782422d`.

The keyboard. None of the three measures what the keyboard covers by its window, which the runtime reports as large as the screen. XCTest hit-tests instead: `XCUIElement.isHittable` is false when the element "is covered by another element" (Apple's XCUIAutomation documentation), and the keyboard's window takes only the touches over the keyboard. WebDriverAgent asks the app whether an element is visible (`XC_kAXXCAttributeIsVisible`, `XCUIElement+FBIsVisible.m`) and finds the keyboard as `app.keyboards.firstMatch` (`FBKeyboard.m`), the element XCTest types `Keyboard`. agent-device states the problem: a visible keyboard "leaves the app window rect intact", so its tap guard measures a band "from the topmost keyboard node down to the bottom of the viewport, across the keyboard's own columns", because the `Keyboard` element (the key plane) stops above the dock that holds the globe and dictation keys (`UIKeyboardDockItemButton`), at 816 of 874 points on an iPhone 17 Pro with iOS 26.2. It measures the band only when that geometry ends within 80 points of the bottom edge and the keys tile one run of columns, so an iPad's floating or split keyboard stays unmeasured (`packages/contracts/src/tap-keyboard-occlusion.ts` and `contracts/fixtures/tap-keyboard-occlusion-policy.json`, agent-device #2589), and its runner publishes the band from `app.keyboards.firstMatch` where it can (#2660).

Views marked modal. The SDK's `UIAccessibility.h` says of `accessibilityViewIsModal`: "If YES, then elements outside this view will be ignored. Only elements inside this view will be exposed." XCTest gets that by asking the app for it. Its queries leave out non-modal elements unless told to include them (XCUIAutomation's private `includingNonModalElements` and `excludingNonModalElements`; WebDriverAgent's `includeNonModalElements` setting, off by default, called the first in `FBXCodeCompatibility.m` at `v11.0.0` and was dropped in v12), and XCTAutomationSupport's snapshot takes the option `snapshotKeyHonorModalViews`, which the app's UIAccessibility serves in `_accessibilityUserTestingSnapshotDescendantsWithAttributes:maxDepth:maxChildren:maxArrayCount:honorsModalViews:` by replacing a view's subviews with its modal one (`_subviewsReplacedByModalViewSubviewsIfNecessary:`). None of the 81 attribute names XCTAutomationSupport knows (`XC_kAXXCAttribute*`) carries the flag, so a read that keeps what a modal view covers cannot tell which view is modal. agent-device's bridge sends `XCTDefaultSnapshotParameters()`, which hold no such option, as the reader does, and cuts only presentations whose dimming view takes touches (ADR 0004, amendment "modal containment in the presentation cut"), deliberately after acquisition, since its raw read "owes the reader the tree the platform reported"; its bridge therefore lists what Bluesky's typeahead covers, while its XCTest runner does not.

## Call sites

Every call site worked from Rust through objc2 and none needed the Objective-C shim D6 keeps as an escape hatch. Unlike spike S2's display path, no object here is a ROCKit proxy, so `msg_send!` worked with objc2's debug-build signature checks on.

| Call site | Outcome |
|---|---|
| `dlopen` of XCTAutomationSupport and AXRuntime by their guest paths | works; inside the Simulator the paths resolve against the runtime root, and XCTAutomationSupport brings in AccessibilityUtilities |
| `dlsym` of the five C functions, called through `extern "C"` function pointers | works |
| `-[XCAccessibilityElement AXUIElement]` and the element argument of `-userTestingSnapshotForElement:options:error:` | works once typed as `^{__AXUIElement=}`: objc2's debug check refused an object (`@`) there, so a `#[repr(transparent)]` wrapper encodes `AXUIElementRef` with that encoding and the check stays on |
| `-userTestingSnapshotForElement:options:error:` | works; returns nested `NSDictionary` nodes keyed by `UIAccessibilitySnapshotKey*`, attribute values as `NSString`, `NSNumber` or `AXValueRef` (frames, unpacked with `AXValueGetValue` type 3 after a `CFGetTypeID` check) |
| `+[AXSettings sharedInstance]`, `-setAutomationEnabled:` | works; the class is registered once XCTAutomationSupport is loaded, so the reader's own load of AccessibilityUtilities, a fallback for other runtimes, never ran |
| `+[AXElement primaryApp]`, `-pid` | works; the check before and the check after a read cost 1.8 to 4.7 ms together |
| `+[AXElement springBoardElement]` | does not exist on iOS 26.5; objc2's debug check turned the send into a panic before it was made. The method lives on `AXElementFetcher`. The reader uses `+systemWideElement` then `-springBoardApplication` and `-currentApplications`, the latter an empty array here |
| `+[AXElement elementWithAXUIElement:]`, `-press` | works; used to open the alerts |
| `AXPTranslator` and its elements, from the host | works; `-accessibilityFrame` returns an `NSRect` through `msg_send!` |
| A Rust class as the translator's token delegate | works with `define_class!`; the callback method returns a `block2` block handed over as a retained object (`RcBlock::into_raw`), and each response is returned autoreleased (`Retained::autorelease_return`) |
| `-sendAccessibilityRequestAsync:completionQueue:completionHandler:` | works with an unsigned `block2` completion block on a serial dispatch queue |
| `-spawnWithPath:options:terminationQueue:terminationHandler:pid:error:` with `arguments`, `environment` and `standalone` NO | works; returns about 100 ms after the process starts |

A scratch C program built for the Simulator confirmed the order: `AXSettings` is not registered after loading AXRuntime and is registered after loading XCTAutomationSupport, which maps AccessibilityUtilities.

## Gates

**Readiness, 500 ms from spawn** (simulator-driver spec, "No test runner and no per-app build"):
- CoreSimulator spawn passes: warm p95 316.7 ms, after an app relaunch 338.8 and 342.6 ms, and after a device boot 298.8 ms at most in 3 runs.
- A new copy of the binary reached 542.7 ms at p95 (n=10), over the gate. That cost belongs to the first spawn of a new file, so it falls once on the first spawn after the reader is installed or updated.
- `xcrun simctl spawn` fails everywhere: p95 607.5 ms warm and 685.4 to 1,183.0 ms after a relaunch. About 295 ms of it is `xcrun` and `simctl` starting, before the reader exists.
- The reader meets the requirement's other terms. Nothing is installed: the binary is spawned from its path on the host. It is not code signed: the linker's ad hoc signature is automatic and no identity or `codesign` step is involved. There is no per-app build: one binary served both fixtures and SpringBoard. Its readiness time is available to target status from `hello`.

**Warm snapshot p95 under 150 ms** (benchmark-harness spec, Latency gates):
- Guest: passes on 9 of 10 screens, the SwiftUI Big screen included at 130.4 ms p95; the Login, List and alert screens take 10.2 to 54.1 ms at p95. It fails on the Expo Big screen at 248.5 ms, 242.5 ms of which is the app's own snapshot. The reader cannot shorten that; a narrower attribute list or a subtree read might, and neither was measured.
- Translator: fails on both Big screens, 1,263.8 and 1,250.3 ms, about 8 times the gate; passes on the other 8 screens at 12.3 to 122.4 ms.
- Under D5 the mirror serves a warm snapshot from its replica without a device round trip, so these read times bound how quickly the replica refreshes after a change rather than the snapshot latency the harness reports.

**Cold attach under 2 s** (benchmark-harness): loading CoreSimulator (323 to 333 ms in spike S2), spawning the reader (at most 299 ms after a device boot) and the first read of the Big screen (at most 262 ms) add up to at most 0.9 s.

## Verdict

**Go for the spawned Rust reader**, which confirms D6's plan:
- The port of agent-device's Objective-C bridge to Rust builds for `aarch64-apple-ios-sim` with unmodified crates and runs inside the Simulator, every private call going through objc2 or `dlsym` with no Objective-C. The two call sites that failed at first were caught by objc2's debug checks and fixed in Rust (see Call sites).
- Spawned through CoreSimulator it meets the 500 ms readiness gate warm, after an app relaunch and after a device boot; only the first spawns of a newly copied binary exceeded it. It meets the 150 ms warm-read gate on every screen but the Expo Big screen.
- On the SwiftUI Big screen it returns the same 546 nodes as XCTest, 5.2 to 5.6 times faster.
- It refuses a read when system UI owns the foreground and can read SpringBoard's tree instead.
- Readers for different Simulators are separate processes, so their reads do not serialize the way the translator's process-wide singleton does. Several Simulators at once were not measured.
- Its failure containment was not exercised: no private call raised and no request hung in any run, so neither the exception guard nor the 20 s watchdog fired.

It comes with conditions for tasks 6.x and 2.11:
- Spawn it through `-[SimDevice spawnWithPath:...]` from the daemon, never through `xcrun simctl spawn`.
- Keep one reader per booted Simulator for the session. A spawn costs 300 ms; a read on a quiet screen costs 10 to 20 ms.
- Write `AccessibilityEnabled`, `ApplicationAccessibilityEnabled` and `AutomationEnabled` into the device's `com.apple.Accessibility` preferences under `-dataPath` before its first boot.
- Cut modal-covered content on the host.
- Ship the reader prebuilt, cross-compiled with the workspace. Unlike agent-device, which compiles its bridge with `clang` on first use, nothing is built on the user's machine.

**No-go for the translator path as the tree reader. It stays the fallback D6 names** for a Simulator where the reader cannot run:
- It is slower than XCTest on a 500-node screen: 3,522 requests and 1.0 to 1.3 s.
- It flattens the tree.
- It misses the UIKit tab bar's buttons in the SwiftUI app and the identifiers of React Native text.
- After a launch it answers with a childless application element for seconds, so its snapshots must carry a degraded verdict.
- Its strengths:
  - It needs nothing in the guest.
  - It answers the frontmost-application question in under 2 ms, which is a cheap foreground-owner probe.
  - It returns only what a user can reach under a modal alert.
  - On the alerts and the SwiftUI Login and List screens it is within 2 ms of the reader or faster, because it returns 4 to 18 elements where the reader returns 48 to 227. On the Expo Login screen, 10 elements, it is twice as slow.

Proposed replacement for D6's accessibility sentences, from "The accessibility tree cannot be read" to "without SwiftUI container children or web content." (this spike does not edit design.md):

> The accessibility tree cannot be read from a macOS process at useful speed: the host translator (`AXPTranslator` answered through `-[SimDevice sendAccessibilityRequestAsync:completionQueue:completionHandler:]`) makes about seven XPC requests per element and took 1.0 to 1.3 s on the fixtures' 500-cell screens, slower than XCTest (spike S4, `docs/spikes/s4-simulator-ax.md`). The tree is therefore read by a Rust binary built for the `aarch64-apple-ios-sim` target, a port of agent-device's snapshot bridge, which the daemon spawns once per booted Simulator through `-[SimDevice spawnWithPath:options:terminationQueue:terminationHandler:pid:error:]` with no installation, no signature beyond the linker's ad hoc one and no per-app build, and which serves the device protocol on a Unix socket. It loads XCTAutomationSupport and AXRuntime from the runtime root, reads the whole tree with one `-[XCTAccessibilityFramework userTestingSnapshotForElement:options:error:]` per request, and refuses the read when `+[AXElement primaryApp]` is not the target, reading SpringBoard's tree for a system alert instead. S4 measured it ready 292 ms after spawn at p50, within 343 ms at p95 warm and after an app relaunch, and within 299 ms after a device boot, against 568 to 1,183 ms through `xcrun simctl spawn`, which misses the 500 ms gate; a read took 126 to 130 ms at p50 and p95 for the 546 nodes XCTest returns on the SwiftUI 500-cell screen, and 242 to 249 ms on the Expo one, where the app's own snapshot takes 236 ms. Before a device's first boot the daemon writes `AccessibilityEnabled`, `ApplicationAccessibilityEnabled` and `AutomationEnabled` into its `com.apple.Accessibility` preferences under `-dataPath`; without them both read paths fail for several seconds after boot. The translator path stays as the fallback when the reader cannot run: it needs nothing in the guest and names the frontmost application in under 2 ms, but it flattens the tree, drops the children of UIKit's tab bar and the identifiers of React Native text, and reports a childless application for seconds after a launch, so its snapshots are marked degraded.

And in D6's call-site sentence, after "the outcome per call site is recorded in the compatibility table":

> Spike S4 reached every guest and translator call site through objc2 and `dlsym` without the shim; `AXUIElementRef` is passed with the encoding `^{__AXUIElement=}` so objc2's signature checks stay on.

Proposed corrections to `docs/research/simulator.md`; this spike does not edit it:
1. The verdict's "the host translator path is the only one that meets the spec here" does not hold against the current simulator-driver spec, which permits a spawned helper that is ready within 500 ms; the reader is, through CoreSimulator's spawn.
2. `+[AXElement springBoardElement]` is on `AXElementFetcher`, not `AXElement`, on iOS 26.5. SpringBoard comes from `+[AXElement systemWideElement]` and `-springBoardApplication`.
3. The note's S4 symbol list gives no image for `AXSettings` or `_AXSAutomationEnabled`. They are AccessibilityUtilities, which XCTAutomationSupport loads, and libAccessibility, so the two `dlopen` calls are all the reader needs.
4. iosef's `CoreSimulatorBridge` restart was not needed on this host. With no preferences written, the translator's frontmost request returned nil for about 5 s and then answered, by which time the guest had enabled accessibility in the device's preferences. Writing the three keys before boot removed the wait for both paths. That is the note's what-we-take item 6 minus the restart.
5. A one-element tree like idb#964's appears after every app launch: for seconds the translator reports the application with no children. The spec's sparse-tree verdict covers it.
6. The translator's frames are already iOS points on a headless device. iosef's letterbox transform matters only when a Simulator window is involved.
7. Any process spawned into an iOS 26.5 Simulator spends about 200 ms before `main`, so no spawned helper can answer much sooner than 200 ms after its spawn.
