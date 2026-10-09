# S10: iOS runner

Spike S10 (task 2.2 of `physical-ios`) builds the smallest resident XCTest runner in Rust for a physical iPhone and measures what design D20 to D22 rest on: the runner's start through the test manager, the tree read inside it, input through XCTest's synthesis against the host's `dtuhidd`, typed and set text, XCTest's screenshot, and whether one session survives. The research note `docs/research/ios-device.md` named the steps, the private calls and the additions. The code is `spikes/s10-ios-runner/`, and every table in Measurements is generated from its `results/` by `s10-ios-runner report`. Snapshot date: 2026-10-09.

**In brief.** Go for the runner and its tree, no-go for its input.

- **The runner works.** A Rust agent linked with an Objective-C shim into an XCTest bundle, signed outside Xcode, is accepted by the test manager on an iPhone SE (3rd generation) with iOS 26.6.2 once the runner's executable is thinned to arm64. It starts through `idevice`'s `xctest` session and says hello over usbmuxd after 772 ms at p50 (1.8 s with the install), and it served 30 minutes of commands without a restart.
- **The tree comes from XCTest, not from the Simulator reader's call.** The runner's sandbox denies the lookup of other apps' accessibility servers, so `userTestingSnapshotForElement:` cannot read. XCTest's own snapshot, fetched by testmanagerd, goes through the reader's conversion instead: the SwiftUI and Expo Big tabs' 546 and 529 nodes in 476 and 527 ms at p95, under D20's 1 s, without platform classes.
- **The runner's taps miss D20's 150 ms.** They take 294 ms synthesized and 795 ms through `XCUICoordinate`, where `dtuhidd` from the host takes 53.5 ms the same day and changes the screen 36 to 84 ms sooner. `dtuhidd` becomes D22's default and the runner's synthesis its fallback.
- **Text and screenshots go through the runner.** It types 36 characters in 813 ms or sets them in 173 ms at p95, all landing whole. XCTest's screenshot takes 42.6 ms against DVT's 54.4 ms; the daemon refuses WebDriverAgent's legacy screenshot request.

D20, D21 and D22 are rewritten with these numbers.

## Setup

**Host.** Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2 (25G83). Other sessions used it throughout: Simulators, a Maestro runner on a Simulator, Xcode builds and Java processes (see Conditions).

**Apple tools.** Xcode 27.0 (27A266a); CoreDevice and `devicectl` 642.16; the iOS developer disk image (DDI) of Xcode 27.0 (27A266a), with XCTest 25228, mounted on the phone as the personalized image at `/System/Developer` (see The developer disk image).

**Device.** iPhone SE (3rd generation), `iPhone14,6` (`D49AP`, arm64e), iOS 26.6.2 (23G90), on USB, paired, trusted, Developer Mode on, battery at 100% on external power throughout, unlocked for every series (each read lockdown's `PasswordProtected` before starting). "Enable UI Automation" was authorized once on the phone (see Starting the runner). AssistiveTouch is on. Its identifier stays in `MOBILE_AGENT_IOS_UDID`; no result or line of this report names it, the phone's name or its owner's content.

**Signing.** The login keychain's one Apple Development identity and an iOS App Development profile carrying the device and the wildcard App ID `dev.mobileagent.*`, both set up by the maintainer before the spike; the spike changed nothing in the account and Xcode has no account signed in. The team comes from `MOBILE_AGENT_IOS_TEAM` and is written only under the git-ignored `build/`. The runner is `dev.mobileagent.runner` with its test bundle `dev.mobileagent.runner.xctest`; the fixtures keep `dev.mobileagent.fixture.swiftui` and `.expo`.

**Crates.** `idevice` 0.1.68, pinned with `=`, with the features `usbmuxd`, `core_device_proxy`, `tunnel_tcp_stack`, `rsd`, `xpc`, `dvt`, `core_device`, `display_stream` (for the HID clients), `device_info`, `sysmontap`, `installation_proxy`, `afc`, `xctest`, `mobile_image_mounter`, `tss` and `syslog_relay`; tokio 1.53.2; the workspace's `protocol` and `simulator` crates as path dependencies (the frame pipeline, as in S11); in the runner's agent objc2 0.6.5, objc2-foundation 0.3.2, libc 0.2.190 and serde_json 1.0.151. Everything came through `sfw cargo add` and `sfw cargo fetch`. Rust 1.98.1 for the host and for `aarch64-apple-ios` (added with `rustup target add` and to `rust-toolchain.toml`), release profile.

**Code.** The first eight series ran before their code was committed, in `956c32d`: `cold-start-installed` and the two snapshot series on a build without two later changes they do not use (a control command for the confirmation interval, and the screenshot poller's stop), the other five on that commit's code. The screenshot series and `cold-start-install` ran on `360f38d` (screenshots through `XCUIScreen`), `tap-synthesized-noconfirm` on `50fbe40`, survival on `b14633f` and the cable pull on `119d99d`. Earlier runs of the program with `S10_DRY_RUN=1`, and the probes, went to the scratch directory and are not reported.

**Privileges.** One unprivileged process on the host; no `sudo`. Installed on the phone: the runner and the two fixtures, through the installation proxy; nothing else installed, deleted or changed.

**Method.** Every timed series first waits for a quiet host, S9's guard: a 1-minute load under 4 and no cargo, rustc, gradle, xcodebuild, kotlinc or java process above 5% of a core, checked every 30 s, each check logged to `results/hygiene.tsv`. A device-bound series may run under a load of 6 once 90 minutes of waiting are spent, marked as such in its table; none needed it. The series then reads the lock, discards its warm-up runs (2, or 3 for screenshots) and times 20. Times are from the host: a protocol request's write to its answer's arrival over usbmuxd, unless a table says otherwise. The checks without timing are one try each, in `results/checks.json`.

**Rerun**, with the phone on USB, unlocked and trusted, `MOBILE_AGENT_IOS_UDID` and `MOBILE_AGENT_IOS_TEAM` exported:

```sh
cd spikes/s10-ios-runner
runner/build.sh && ./fixtures.sh swiftui && ./fixtures.sh expo   # the bundles, signed
cargo test --workspace                          # unit tests: percentiles, guard, tolerance, PNG, tree conversion
cargo build --release
./target/release/s10-ios-runner device          # results/device.json
./target/release/s10-ios-runner install         # the runner and both fixtures
./target/release/s10-ios-runner start-wait swiftui 10   # first start after a boot: approve "Enable UI Automation" on the phone
./target/release/s10-ios-runner checks          # results/checks.json
./target/release/s10-ios-runner measured        # every timed series behind the guard, then results/tables.md
./target/release/s10-ios-runner survival        # 30 minutes of mixed commands
./target/release/s10-ios-runner replug          # one cable pull and replug, with a person at the phone
./target/release/s10-ios-runner report          # regenerate results/tables.md
```

`spikes/s10-ios-runner/README.md` lists the layout and the other commands.

### Conditions

The guard made 249 checks between 11:42:45 and 14:46:57, one row each in `results/hygiene.tsv`; the full blocks are in the git-ignored `results/hygiene.log`. 234 checks found the host busy:

- another session's Android emulator was above 5% of a core at 175 of them;
- at 53 of them a watched process was above 5% as well: java at 42, xcodebuild at 18, and rustc at 4, the last being this spike's own compiles while the guard waited;
- the 1-minute load at the busy checks ranged from 3.9 to 63.9.

Every series started at a 1-minute load under 4, so none used the fallback. The load rose during four series, from other sessions' work: to 6.98 during `tap-synthesized`, 6.62 during `tap-dtuhidd`, 6.97 during `type-36` and 27.93 by the end of `cold-start-install`. That last series' start times stay in line with `cold-start-installed`'s.

Two runs were lost and repeated:

- the first `tap-synthesized` hung in its teardown, waiting on the screenshot poller, and was stopped;
- the first `screenshot-xctest` failed on the daemon's refusal of the legacy screenshot request (see Screenshots).

The program was restarted between series for those fixes, for the `MH_DYLIB` check and after a session limit. Each run starts its 90-minute budget anew.

| Series | Checks that waited | Started | Finished | Load before / after |
|---|---:|---|---|---|
| cold-start-installed | 17 | 2026-10-09 11:51:16 | 2026-10-09 11:52:12 | 3.85 / 4.27 |
| snapshot-swiftui-big | 32 | 2026-10-09 12:08:17 | 2026-10-09 12:08:31 | 3.82 / 4.05 |
| snapshot-expo-big | 1 | 2026-10-09 12:09:01 | 2026-10-09 12:09:17 | 3.57 / 3.54 |
| tap-synthesized | 11 | 2026-10-09 12:21:14 | 2026-10-09 12:22:49 | 2.89 / 6.98 |
| tap-coordinate | 44 | 2026-10-09 12:45:15 | 2026-10-09 12:47:04 | 3.90 / 4.41 |
| tap-dtuhidd | 5 | 2026-10-09 12:49:35 | 2026-10-09 12:50:48 | 3.78 / 6.62 |
| type-36 | 73 | 2026-10-09 13:27:25 | 2026-10-09 13:28:16 | 3.55 / 6.97 |
| set-text | 3 | 2026-10-09 13:29:47 | 2026-10-09 13:30:03 | 3.90 / 3.19 |
| screenshot-xctest | 45 | 2026-10-09 14:38:10 | 2026-10-09 14:38:15 | 3.65 / 3.35 |
| screenshot-dvt | 0 | 2026-10-09 14:38:15 | 2026-10-09 14:38:20 | 3.35 / 3.08 |
| cold-start-install | 0 | 2026-10-09 14:38:20 | 2026-10-09 14:39:45 | 3.08 / 27.93 |
| tap-synthesized-noconfirm | 3 | 2026-10-09 14:43:10 | 2026-10-09 14:44:44 | 3.65 / 2.95 |
| survival | 0 | 2026-10-09 14:46:57 | 2026-10-09 15:17:02 | 3.32 / 6.56 |

## Measurements

All times are milliseconds, p50 / p95 over 20 runs, nearest rank, warm-up runs excluded, with the host's 1-minute load before and after each series.

### Building and signing

**The fixtures.** `fixtures.sh` builds both fixtures with `-destination 'generic/platform=iOS' -sdk iphoneos`, manual signing (`CODE_SIGN_STYLE=Manual`, `CODE_SIGN_IDENTITY="Apple Development"`, `DEVELOPMENT_TEAM` from the environment, `PROVISIONING_PROFILE_SPECIFIER="mobile-agent development"`) and the derived data under the spike's `build/`; `fixtures/` is not edited.

- **SwiftUI**: the Debug build, in place, 29.2 s.
- **Expo**: built within the 30-minute box. Its build generates `node_modules/`, `ios/` and the pods inside the project, so the script copies `fixtures/expo` to `build/` first. Then `sfw bun install --frozen-lockfile` (1.6 s), `sfw bunx expo prebuild --platform ios --no-install` (4.9 s), `pod install` under `sfw` (75.1 s; the fixture's README says Socket Firewall cannot wrap CocoaPods, and whether `sfw` saw its three Maven downloads was not checked), and the Release `xcodebuild` (153.3 s).

Both installed through `idevice`'s installation proxy (AFC upload to `PublicStaging`, then `Install`): 1.6 to 1.7 s for the SwiftUI app and 2.7 to 3.1 s for the Expo app; `devicectl` was not needed.

**The runner.** `runner/build.sh` builds it in five steps, timed on a rebuild: the agent for `aarch64-apple-ios` (0.4 to 3.9 s incremental, 15.7 s after the deployment target changed), the shim (`xcrun -sdk iphoneos clang -target arm64-apple-ios17.0 -fobjc-arc`, 0.6 to 1.0 s), the link (`clang -bundle`, 0.3 s), the bundle (0.2 to 0.4 s) and the signatures (0.2 to 0.4 s; 23.4 s the first time on this host).

- **The agent** is a Rust static library (`agent/`): the protocol's `AgentSession` on `127.0.0.1:27183` behind a TCP listener, its hello naming `mobile-agent-ios-runner 0.1.0`, protocol version 1, the capabilities `tree`, `input`, `screenshot` and `text_set` and a 16 MiB frame limit; `snapshot`, `tap`, `swipe`, `key`, `text_set` and `screenshot` run on one worker thread, and a request during another is answered `busy`. A second loopback port, 27184, takes the spike's JSON-lines commands (which tap path, typing speed, set-text variant, the runner's memory, the accessibility notifications). `key` takes HID keyboard usages, typed through XCTest, and the home button's consumer usage (page 0x0C, usage 0x40), pressed through `XCUIDevice`.
- **The shim** (`runner/Runner.m`) defines `MobileAgentRunnerTests`, an `XCTestCase` whose `+setUp` sets `XCTDisableRemoteQueryEvaluation` and `DisableScreenshots` as WebDriverAgent's does, and whose one test, `testRunner`, starts the agent and spins the main run loop forever. A constructor replaces XCTest's idle waits at load. The XCTest calls the agent makes are C functions here; those that XCTest wants on the main thread are sent there with `dispatch_sync`, the rest wait on a semaphore for their completion.
- **One image**: the shim's object and the static library link into one `MH_BUNDLE` of 3.7 MB (stripped) that links `@rpath/XCTest.framework` and carries rpaths to `/System/Developer`. Linked instead as an `MH_DYLIB`, the shape a Rust `cdylib` has (`S10_IMAGE=dylib`), it loaded as well: one start, hello after 707 ms.

**The bundle the test manager accepted** (`build/runner/MobileAgentRunner-Runner.app`):

```
MobileAgentRunner-Runner.app/          a copy of Xcode 27.0's Agents/XCTRunner.app
  MobileAgentRunner-Runner             its executable, renamed and thinned to arm64 with lipo
  Info.plist                           CFBundleExecutable, CFBundleName, CFBundleIdentifier dev.mobileagent.runner,
                                       UILaunchScreen added; RunnerEntitlements.plist removed
  embedded.mobileprovision             the development profile, found by name
  PlugIns/MobileAgentRunner.xctest/
    MobileAgentRunner                  the MH_BUNDLE
    Info.plist                         BNDL, dev.mobileagent.runner.xctest, XCTContainsUITests
```

No XCTest framework is embedded: the template's rpaths and the test manager's `DYLD_FRAMEWORK_PATH` load them from the developer disk image. The test bundle is signed first, without entitlements; the app second, with `application-identifier` (team and bundle identifier), `com.apple.developer.team-identifier` and `get-task-allow`.

### Starting the runner

`idevice` 0.1.68's `XCUITestService::run` drives the iOS 17 sequence on its own task: three DTX connections over the in-process tunnel (two to `com.apple.dt.testmanagerd.remote`, one to DVT), the IDE sessions, the runner launched through process control with `XCTestBundlePath`, `XCTestSessionIdentifier`, `XCTestManagerVariant=DDI` and `ActivateSuspended`, the pid authorized, `_XCT_testRunnerReadyWithCapabilities:` answered with the configuration (the SwiftUI fixture as the application under test), and the test plan started. The host then connects to port 27183 through usbmuxd's `Connect` every 25 ms until the hello arrives. Stopping aborts that task, which closes the test manager's connections; XCTest ends the runner ("Exiting due to IDE disconnection"), and the spike also sends SIGKILL to any runner process through CoreDevice's app service.

Five things stood between the first try and a hello, each with the text the phone gave (container paths and identifiers left out):

1. **The developer disk image was not mounted.** The session failed with "service not found": RSD listed 61 services, none of them `com.apple.dt.testmanagerd.remote` or the HID services, where S9 and S11 had listed 82. The phone had evidently restarted since S11, which drops the mount. See The developer disk image.
2. **The template's executable is fat and the phone ran its arm64e slice.** XCTest reported "Failed to load the test bundle" (`XCTestErrorDomain` 103), whose underlying error was "dlopen_preflight(…/PlugIns/MobileAgentRunner.xctest/MobileAgentRunner) => false … mach-o file, but is an incompatible architecture (have 'arm64', need 'arm64e')". `idevice` 0.1.68 reports code 103 as "untrusted developer certificate — go to Settings → General → VPN & Device Management and trust your developer app", which was not the cause. Fix: `lipo -thin arm64` on the runner's executable, which Xcode's own runners get too; the XCTest frameworks on the disk image carry both slices. The other two conditions the research note named (the `-Runner` executable, no embedded `Frameworks/XC*`) held from the first build.
3. **testmanagerd stopped answering after aborted sessions.** The DTX capabilities handshake with `com.apple.dt.testmanagerd.remote` timed out after 30 s ("RSD DTX handshake failed … xctest session timed out after 30.0s") on the tries that followed runners ended mid-start; it answered again a minute or two later, without any action.
4. **Automation mode wants the device owner.** XCTest waited in "enabling automation mode" (its timeout is 60 s) while testmanagerd logged "Writer daemon requires authentication to enable automation mode" and asked LocalAuthentication to evaluate a policy titled "Enable UI Automation". The maintainer approved it on the phone, at the spike's request, after which none of the more than 150 starts that followed asked again. Whether a reboot asks again was not tested.
5. **UIKit gave the runner 320×480 points** until the template's Info.plist got a `UILaunchScreen` entry; without one the runner runs in the compatibility size, and `UIScreen` reported it.

| Series | n | Start to hello | Start to `testRunnerReadyWithCapabilities` | Start to the test bundle loaded | Install | Install and start to hello | Load |
|---|---:|---:|---:|---:|---:|---:|---|
| cold-start-installed | 20 | 771.95 / 873.12 | 329.82 / 352.45 | 336.84 / 360.02 | - | - | 3.85 / 4.27 |
| cold-start-install | 20 | 730.93 / 794.60 | 296.95 / 334.47 | 319.18 / 354.64 | 1117.71 / 1290.08 | 1844.67 / 2022.90 | 3.08 / 27.93 |

- **With the runner installed** its hello comes 772 ms after the session's start at p50 and 873 ms at p95. The runner reports `_XCT_testRunnerReadyWithCapabilities:` at 330 ms, has its test bundle loaded at 337 ms and asks for automation mode at 340 ms; the remaining 430 ms are XCTest's preparation for UI testing, the test's start, the agent's listener and the host's 25 ms polling.
- **Installed anew** through the installation proxy (the 3.7 MB bundle uploaded over AFC, then `Install`), the runner takes 1,118 and 1,290 ms to install and its hello comes 1,845 and 2,023 ms after the install began. The host's load rose to 27.9 by the end of this series, from other sessions.
- **The test bundle as an `MH_DYLIB`**, the shape a Rust `cdylib` has, loaded too: one start, hello after 707 ms.

### The developer disk image

When the spike started, the image mounter's `CopyDevices` listed nothing. The spike started a personalized mount through `idevice`'s image mounter (the Xcode 27.0 image, its trust cache and build manifest, the ticket from Apple's signing server when the phone holds no manifest); the call printed no result and was stopped after 5.5 minutes, and by then `CopyDevices` listed the personalized `DeveloperDiskImage` at `/System/Developer`. Whether the spike's call or the host's CoreDevice mounted it is not known: no host log said. The mount is therefore not timed, and task 3.2 still has to verify a mount through `idevice` on a phone without one (`results/` holds no mount file).

### The tree inside the runner

The Simulator reader's guest extraction is copied into the agent (`agent/src/reader/`, `elements.rs` without its tests): XCTAutomationSupport from `/System/Developer`, AXRuntime, one `userTestingSnapshotForElement:options:error:` per read with `snapshotKeyHonorModalViews`, the same conversion into the protocol's snapshot. In the runner on iOS 26.6.2 it loads (11 ms the first time, `_AXSAutomationEnabled` true) but cannot read (`results/checks.json`):

- `+[AXElement primaryApp]` answers nil, with the fixture in front.
- Given the fixture's pid instead, every snapshot fails with `com.apple.dt.xctest.automation-support.error` 5, AX error -25215, "kAXErrorServerNotFound getting snapshot for element <AXUIElementRef …> {pid=…}". The phone's log says why: "AX Lookup problem - errorCode:1100 error:Permission denied portName:'com.apple.iphone.axserver'" and "Sandbox: MobileAgentRunner-Runner(…) deny(1) mach-lookup com.apple.iphone.axserver (per-pid)". The runner is a sandboxed app, and its sandbox gives it no route to another application's accessibility server. The Simulator reader is a plain process in the guest, with no such sandbox.
- `-[XCAXClient_iOS requestSnapshotForElement:attributes:parameters:error:]`, XCTest's in-process client, does not reach the application either: on the device it asks testmanagerd (`_XCT_fetchSnapshotForElement:attributes:parameters:reply:`). With the reader's attribute names and the client's default parameters, the reply failed to decode in the runner ("Exception while decoding argument 0 … value for key 'NS.objects' was of unexpected class 'NSArray'") and the call gave up after 65 s with "Timed out while fetching snapshot from testmanagerd". Which attribute shape the daemon wants was not found.
- The runner's sandbox also hides other processes' executables (`proc_pidpath` fails), so the reader's owner has no bundle.

What does read is XCTest's own snapshot, which testmanagerd fetches with its own privileges. The runner reads the tree this way (`agent/src/reader/xctree.rs`): the foreground's pid from `-[XCAXClient_iOS activeApplications]`, its bundle from the daemon session's `-requestApplicationSpecifierForPID:reply:`, then `-[XCUIElement snapshotWithError:]` on that bundle's `XCUIApplication`, serialized by the shim (element type, traits, frame, identifier, label, value, placeholder, keyboard focus, children) and turned into the reader's `Element`s, which the reader's `elements.rs` converts unchanged. The read checks the foreground again afterwards (`window_changed`).

That snapshot carries no platform class (`XC_kAXXCAttributeElementType`), so every node's class is its `XCUIElementType` name and the conversion's rules that need a class see none: the keyboard's `UIRemoteKeyboardWindow` stays inside the application's window instead of becoming an input-method window, and alert containers and dimming views are not recognized. No scroll area is read either, and how it treats views the app marks modal was not checked. Task 4.4's node-for-node comparison with the Simulator therefore needs the daemon's snapshot with the reader's attributes, which this spike did not get working.

| Screen | n | Request to reply | XCTest's snapshot in the runner | Conversion | Nodes | Cells | Windows | Load |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| snapshot-swiftui-big | 20 | 462.76 / 476.11 | 449.75 / 460.85 | 0.53 / 0.56 | 546 | 500 | 1 | 3.82 / 4.05 |
| snapshot-expo-big | 20 | 515.63 / 527.31 | 502.94 / 515.11 | 0.48 / 0.53 | 529 | 500 | 1 | 3.57 / 3.54 |

`snapshot-swiftui-big`, the first 20 cells as the protocol carries them:

| Identifier | Description | Class | Bounds (px) |
|---|---|---|---|
| cell-1 | Cell 1 | XCUIElementTypeButton | [16,113,52,155] |
| cell-2 | Cell 2 | XCUIElementTypeButton | [52,113,88,155] |
| cell-3 | Cell 3 | XCUIElementTypeButton | [88,113,124,155] |
| cell-4 | Cell 4 | XCUIElementTypeButton | [124,113,160,155] |
| cell-5 | Cell 5 | XCUIElementTypeButton | [160,113,196,155] |
| cell-6 | Cell 6 | XCUIElementTypeButton | [196,113,231,155] |
| cell-7 | Cell 7 | XCUIElementTypeButton | [231,113,267,155] |
| cell-8 | Cell 8 | XCUIElementTypeButton | [267,113,303,155] |
| cell-9 | Cell 9 | XCUIElementTypeButton | [303,113,339,155] |
| cell-10 | Cell 10 | XCUIElementTypeButton | [339,113,375,155] |
| cell-11 | Cell 11 | XCUIElementTypeButton | [375,113,411,155] |
| cell-12 | Cell 12 | XCUIElementTypeButton | [411,113,447,155] |
| cell-13 | Cell 13 | XCUIElementTypeButton | [447,113,483,155] |
| cell-14 | Cell 14 | XCUIElementTypeButton | [483,113,519,155] |
| cell-15 | Cell 15 | XCUIElementTypeButton | [519,113,555,155] |
| cell-16 | Cell 16 | XCUIElementTypeButton | [555,113,590,155] |
| cell-17 | Cell 17 | XCUIElementTypeButton | [590,113,626,155] |
| cell-18 | Cell 18 | XCUIElementTypeButton | [626,113,662,155] |
| cell-19 | Cell 19 | XCUIElementTypeButton | [662,113,698,155] |
| cell-20 | Cell 20 | XCUIElementTypeButton | [698,113,734,155] |

Classes in the tree: {"XCUIElementTypeApplication":1,"XCUIElementTypeButton":505,"XCUIElementTypeHeading":1,"XCUIElementTypeImage":5,"XCUIElementTypeOther":32,"XCUIElementTypeTabBar":1,"XCUIElementTypeWindow":1}

`snapshot-expo-big`, the first 20 cells as the protocol carries them:

| Identifier | Description | Class | Bounds (px) |
|---|---|---|---|
| cell-1 | Cell 1 | XCUIElementTypeButton | [35,145,69,179] |
| cell-2 | Cell 2 | XCUIElementTypeButton | [69,145,103,179] |
| cell-3 | Cell 3 | XCUIElementTypeButton | [103,145,137,179] |
| cell-4 | Cell 4 | XCUIElementTypeButton | [137,145,171,179] |
| cell-5 | Cell 5 | XCUIElementTypeButton | [171,145,205,179] |
| cell-6 | Cell 6 | XCUIElementTypeButton | [205,145,239,179] |
| cell-7 | Cell 7 | XCUIElementTypeButton | [239,145,273,179] |
| cell-8 | Cell 8 | XCUIElementTypeButton | [273,145,307,179] |
| cell-9 | Cell 9 | XCUIElementTypeButton | [307,145,341,179] |
| cell-10 | Cell 10 | XCUIElementTypeButton | [341,145,375,179] |
| cell-11 | Cell 11 | XCUIElementTypeButton | [375,145,409,179] |
| cell-12 | Cell 12 | XCUIElementTypeButton | [409,145,443,179] |
| cell-13 | Cell 13 | XCUIElementTypeButton | [443,145,477,179] |
| cell-14 | Cell 14 | XCUIElementTypeButton | [477,145,511,179] |
| cell-15 | Cell 15 | XCUIElementTypeButton | [511,145,545,179] |
| cell-16 | Cell 16 | XCUIElementTypeButton | [545,145,579,179] |
| cell-17 | Cell 17 | XCUIElementTypeButton | [579,145,613,179] |
| cell-18 | Cell 18 | XCUIElementTypeButton | [613,145,647,179] |
| cell-19 | Cell 19 | XCUIElementTypeButton | [647,145,681,179] |
| cell-20 | Cell 20 | XCUIElementTypeButton | [681,145,715,179] |

Classes in the tree: {"XCUIElementTypeApplication":1,"XCUIElementTypeButton":505,"XCUIElementTypeHeading":2,"XCUIElementTypeOther":19,"XCUIElementTypeScrollView":1,"XCUIElementTypeWindow":1}

- **The SwiftUI Big tab** reads as 546 nodes, the count the Simulator's reader and XCTest give on an iPhone 17 (`fixtures/swiftui/README.md`): 500 cells, 5 tab buttons with their 5 images, the header, 32 containers, the tab bar, the window and the application. XCTest's snapshot takes 450 of the 463 ms at p50; the conversion, 0.5 ms.
- **The Expo Big tab** reads as 529 nodes with the 500 cells, in 516 and 527 ms: the 529 nodes that agent-device's and XCTest's raw trees hold on the Simulator (`reports/10.4-rn-smoke.md`).
- **The first 20 cells** carry the identifiers `cell-1` to `cell-20`, the labels `Cell 1` to `Cell 20` as descriptions, the class `XCUIElementTypeButton` and their frames in pixels, the fields the mirror lists. The raw snapshot matches the Simulator's in node count, identifiers and labels on these screens, and the mirror's listing is built from it; where the Simulator's reader names a platform class (an `Other` or `Group` node), the device's names the type, so a listing can differ there.

### Taps, beside `dtuhidd`

The tap series repeat S11's: Settings relaunched through CoreDevice's app service, a tap on Bluetooth's row at (187, 550) points that opens its page and on the page's back button at (38, 42) that returns, 20 of each after 2 of each discarded, the frames from back-to-back DVT screenshots on a connection of their own (48 to 49 ms a capture today, against S11's 105 ms on a busy phone), a frame counted as changed when a pixel below the status bar moves by more than 2 levels (S11's tolerance), and the first changed frame searched from the tap's request. `dtuhidd` ran the same day, with the runner running.

| Path | Tap | n | Request to acknowledgement | First changed frame from the acknowledgement | From the request | Missed | Load |
|---|---|---:|---:|---:|---:|---:|---|
| tap-synthesized | open | 20 | 290.71 / 294.11 | 0.00 / 0.00 | 246.94 / 272.17 | 0 | 2.89 / 6.98 |
| tap-synthesized | back | 20 | 290.97 / 295.05 | 0.00 / 0.00 | 118.85 / 178.24 | 0 | 2.89 / 6.98 |
| tap-synthesized-noconfirm | open | 20 | 283.71 / 291.54 | 0.00 / 0.00 | 267.05 / 275.54 | 0 | 3.65 / 2.95 |
| tap-synthesized-noconfirm | back | 20 | 285.19 / 291.33 | 0.00 / 0.00 | 119.64 / 168.03 | 0 | 3.65 / 2.95 |
| tap-coordinate | open | 20 | 697.96 / 795.43 | 0.00 / 0.00 | 648.38 / 747.55 | 0 | 3.90 / 4.41 |
| tap-coordinate | back | 20 | 565.63 / 606.84 | 0.00 / 0.00 | 405.97 / 459.56 | 0 | 3.90 / 4.41 |
| tap-dtuhidd | open | 20 | 52.12 / 53.54 | 158.25 / 192.82 | 211.35 / 245.83 | 0 | 3.78 / 6.62 |
| tap-dtuhidd | back | 20 | 52.09 / 53.18 | 0.00 / 50.50 | 35.45 / 103.29 | 0 | 3.78 / 6.62 |

- **`dtuhidd` from the host** is acknowledged when the release report is written, 52.1 and 53.5 ms after the request with its 50 ms hold. The row's page showed 211 ms after the request at p50, the back button's highlight 35 ms.
- **The runner's synthesized tap** is acknowledged when the event synthesizer's completion arrives: 291 and 294 ms. The screen changes before that, 247 ms after the request for the row and 119 ms for the back button, so its first changed frame comes 0 ms after the acknowledgement, and 36 to 84 ms after `dtuhidd`'s at p50. The daemon session's implicit event confirmation interval reads 5 s; set to 0 (`tap-synthesized-noconfirm`), the acknowledgement stays at 284 and 292 ms.
- **`XCUICoordinate`** takes 698 and 795 ms for the row and 566 and 607 ms for the back button: XCTest resolves the application before it taps.
- No tap missed the screen on any path.

### Input through the runner

- **Taps.** `tap` goes through an `XCSynthesizedEventRecord` with one `XCPointerEventPath` (contact at offset 0, lift at 50 ms, S11's tap) sent to `XCUIDevice`'s event synthesizer, the daemon session, and is acknowledged when its completion arrives; or, set through the control channel, through `-[XCUICoordinate tap]` in the application's coordinate space, acknowledged when it returns. XCTest's idle waits are replaced by no-ops at load. Xcode 27's XCTest (25228) has no `-[XCUIApplicationProcess waitForQuiescenceIncludingAnimationsIdle:]`; it has `waitForQuiescenceIncludingAnimationsIdle:isPreEvent:` and `waitForQuiescenceIncludingAnimationsIdle:usingActivity:isPreEvent:`, and `-[XCAXClient_iOS waitForQuiescenceOnAllForegroundApplicationsAsPreEvent:]`, and the runner replaces all three.
- **Swipes** are one path moved to the end over the duration; with a rest they stay there, move one point on and lift, the Android `rest_ms` shape.
- **Keys** are typed (HID usages to characters) or, for the home button's usage, `-[XCUIDevice pressButton:]`.
- **Typing** goes through the daemon proxy's `_XCT_sendString:maximumFrequency:completion:`, as WebDriverAgent's `FBKeyboard` sends it, at WebDriverAgent's default of 60 characters a second.
- **Set text** finds the element with keyboard focus in XCTest's snapshot and sets its value. One try of each variant on the focused `login-username` (`results/checks.json`): the daemon proxy's `_XCT_setAttribute:value:element:reply:` with the value attribute's number (2006) set it, and so did `-[XCAXClient_iOS setAttribute:value:element:outError:]` with the attribute's name, `XC_kAXXCAttributeValue`. The same call with the number failed ("Failed to set attribute (null) -> set0 … kAXErrorIllegalArgument"), and so did `_XCT_performAccessibilityAction:onElement:withValue:reply:` with action 0 ("Error kAXErrorIllegalArgument performing AXAction AXAction 0"). The series uses the daemon's `_XCT_setAttribute`.
- **The home button.** `-[XCUIDevice pressButton:XCUIDeviceButtonHome]` left Settings for the home screen: acknowledged after 502 ms, the first changed frame after 1,046 ms. `dtuhidd`'s Indigo button event, S11's press, did the same in 89 ms and 1,086 ms; both first frames wait for the home screen's 0.7 to 0.9 MB picture. One earlier try of the `dtuhidd` press, right after the runner's, changed nothing; the second, here, did.

| Series | n | Request to acknowledgement | Landed whole | Clear (36 backspaces) | Load |
|---|---:|---:|---:|---:|---|
| type-36 | 20 | 807.13 / 813.11 | 20 of 20 | 808.24 / 813.87 | 3.55 / 6.97 |
| set-text | 20 | 162.70 / 172.58 | 20 of 20 | - | 3.90 / 3.19 |

- **Typed**: the 36 characters at 60 a second take 807 and 813 ms, all 20 read back whole from the next snapshot; 36 backspaces the same.
- **Set**: the same 36 characters through the daemon's `_XCT_setAttribute`, 163 and 173 ms with the snapshot that finds the focused field, all 20 whole.

### Screenshots

| Path | n | Request to reply | PNG bytes p50 | Load |
|---|---:|---:|---:|---|
| screenshot-xctest | 20 | 38.80 / 42.59 | 141554.0 | 3.65 / 3.35 |
| screenshot-dvt | 20 | 50.35 / 54.44 | 141179.0 | 3.35 / 3.08 |

- **XCTest's screenshot** through the runner, `-[XCUIScreen screenshot]`, returns the same 750×1334 PNG of 141 KB as DVT in 38.8 and 42.6 ms over the protocol, against 50.4 and 54.4 ms for DVT's `takeScreenshot` through the tunnel. S9 measured DVT at 42.3 and 43.5 ms on a quiet phone.
- **The daemon proxy's `_XCT_requestScreenshotOfScreenWithID:withRect:uti:compressionQuality:withReply:`**, WebDriverAgent's call, is refused: "XCTDaemonErrorDomain 39: Legacy screen requests are no longer supported".

### Survival and a pulled cable

One runner session served the SwiftUI fixture for 30 minutes. Every 3 s it took a snapshot, tapped `row-1` or the detail's Back button in turn, and took a screenshot; its memory was read every 30 s (`results/survival.json`).

| Minutes | Cycles | Target not found | Command errors | Same runner pid | Session ended | Snapshot | Tap | Screenshot | Footprint MiB first / last / max | Load |
|---:|---:|---:|---:|---|---|---:|---:|---:|---|---|
| 30 | 600 | 27 | 0 | true | false | 161.87 / 301.23 | 279.57 / 285.18 | 86.31 / 94.56 | 43.3 / 45.7 / 45.8 | 3.32 / 6.56 |

- **No restart.** The runner kept one pid and the test manager's session never ended. Over 600 cycles, no snapshot, tap or screenshot failed.
- **Another application took the foreground.** At 28.7 minutes a tap on the detail's Back button brought another application to the front. A screenshot afterwards showed that application with the status bar's way back to the fixture, so the tap most likely landed on a banner over the button (an inference). The runner went on reading that application's tree, so the remaining 27 cycles found no target to tap. A driver tapping near the top of the screen meets the same banners.
- **Memory.** The runner's physical footprint grew from 43.3 MiB to 45.7 MiB, at most 45.8 MiB.
- **Times.** Snapshots of the List tab and the detail took 162 and 301 ms; taps took 280 and 285 ms; screenshots took 86 and 95 ms, interleaved with the rest.

**A pulled cable** (`results/replug.json`, `results/replug-reattach.json`). The maintainer pulled the USB cable and plugged it back in, at the spike's request, with a runner serving:

- the protocol connection broke at once ("Broken pipe"), and `idevice`'s test manager session ended within 5 s ("device socket io failed");
- the phone was listed on USB again 15 s after the pull was noticed, still unlocked;
- the runner had outlived the pull. The next start's hello came after 35 ms, from that old process, which then closed the connection: the first snapshot failed with `disconnected`;
- once the old process was gone, a new attach said hello after 963 ms and read the List tab's 193 nodes in 226 ms.

A driver therefore ends any runner left over from the lost session before it attaches again.

### Accessibility notifications

The daemon session receives them: registered through `-[XCAXClient_iOS _registerForAXNotification:error:]`, which asks the daemon (108 ms each, 12 numbers), they arrive at `-[XCTRunnerDaemonSession _XCT_receivedAccessibilityNotification:fromElement:payload:]`, which the shim wraps to count them before XCTest's own handler runs. The daemon proxy's `_XCT_registerForAccessibilityNotification:reply:` called directly never replied within 10 s. `XCTStringFromAXNotification` names 1006 (alert), 1011 (orientation), 1018 (first responder), 1021 (pid status), 1028 (HID event received), 1053 (native focus) and 4002 (user testing); 1000 and 1001 come unnamed, the screen and layout changes of UIKit. Counted on the SwiftUI fixture, 2 s after each action:

| Action | Received | Layout changed (1001) | Screen changed (1000) | User testing (4002) | HID event (1028) |
|---|---:|---:|---:|---:|---:|
| none, 2 s | 0 | 0 | 0 | 0 | 0 |
| a tap on a Big tab cell (it only highlights) | 29 | 1 | 0 | 6 | 22 |
| a tap on the List tab: a new screen | 43 | 2 | 1 | 18 | 22 |
| a swipe that scrolls the list | 57 | 1 | 0 | 6 | 50 |

The payloads are binary property lists (`NSData`, 0.4 to 2.3 KB); the HID events come from pid 0, the layout and screen changes from the application. A screen change and a scroll each bring their notification, so they could tell the mirror when to read again; they name no element or path, and a tap that changes nothing brings a layout change too.

### Private call sites

Every private call the runner made, with where it was validated: iOS 26.6.2 (23G90) on the iPhone SE (3rd generation), with the developer disk image of Xcode 27.0 (27A266a, XCTest 25228) mounted, and the runner built with Xcode 27.0's iPhoneOS SDK. Selectors and type encodings were read from that SDK's frameworks with `otool -oV`.

| Call | Image | Outcome on iOS 26.6.2 |
|---|---|---|
| `-[XCUIApplicationProcess waitForQuiescenceIncludingAnimationsIdle:isPreEvent:]`, `…usingActivity:isPreEvent:`, `-[XCAXClient_iOS waitForQuiescenceOnAllForegroundApplicationsAsPreEvent:]` replaced at load | XCUIAutomation | replaced; `waitForQuiescenceIncludingAnimationsIdle:` alone no longer exists |
| `-[XCUIDevice eventSynthesizer]`, `-[XCTRunnerDaemonSession synthesizeEvent:completion:]` with `XCSynthesizedEventRecord -initWithName:interfaceOrientation:`, `-addPointerEventPath:`, `XCPointerEventPath -initForTouchAtPoint:offset:`, `-moveToPoint:atOffset:`, `-liftUpAtOffset:` | XCUIAutomation, XCTestCore | works; completes about 240 ms after the touch |
| `-[XCTRunnerDaemonSession implicitEventConfirmationIntervalForCurrentContext]`, `-setImplicitEventConfirmationIntervalForCurrentContextWithoutSideEffects:` | XCTestCore | reads 5 s; set to 0, the completion still comes after 284 ms at p50 |
| `+[XCTRunnerDaemonSession sharedSession]`, `-daemonProxy` | XCTestCore | works |
| daemon proxy `_XCT_sendString:maximumFrequency:completion:` | testmanagerd of the DDI | works |
| daemon proxy `_XCT_setAttribute:value:element:reply:` | testmanagerd | works with the value attribute's number (2006) |
| daemon proxy `_XCT_performAccessibilityAction:onElement:withValue:reply:` | testmanagerd | action 0: `kAXErrorIllegalArgument` |
| daemon proxy `_XCT_requestScreenshotOfScreenWithID:withRect:uti:compressionQuality:withReply:` | testmanagerd | refused: "XCTDaemonErrorDomain 39: Legacy screen requests are no longer supported" |
| daemon proxy `_XCT_registerForAccessibilityNotification:reply:` | testmanagerd | no reply within 10 s on the first numbers tried |
| `-[XCTRunnerDaemonSession requestApplicationSpecifierForPID:reply:]`, `XCUIApplicationSpecifier -bundleIdentifier` | XCTestCore, XCUIAutomation | works |
| `-[XCTRunnerDaemonSession _XCT_receivedAccessibilityNotification:fromElement:payload:]` wrapped | XCTestCore | works |
| `-[XCUIDevice accessibilityInterface]` (`XCAXClient_iOS`): `-activeApplications`, `-systemApplication`, `-_registerForAXNotification:error:`, `-setAttribute:value:element:outError:` | XCUIAutomation | work (`setAttribute` with the attribute's name, not its number) |
| `XCAXClient_iOS -requestSnapshotForElement:attributes:parameters:error:` with the reader's attribute names | XCUIAutomation, testmanagerd | the reply fails to decode in the runner; times out after 65 s |
| `XCAXClient_iOS -registerForAXNotificationsForApplicationWithPID:timeout:completion:` | XCUIAutomation | raises: "only supported in the local macOS AX interface" |
| `XCAXClient_iOS -loadAccessibility:`, `-cachedAccessibilityLoadedValueForPID:` | XCUIAutomation | `YES`, then `NO` for the fixture |
| `+[XCAccessibilityElement elementWithProcessIdentifier:]`, `XCElementSnapshot -traits`, `-hasKeyboardFocus`, `-accessibilityElement` | XCTAutomationSupport | work |
| `XCTStringFromAXNotification` | XCTAutomationSupport | works |
| The Simulator reader's calls: `XCTAccessibilityFramework -initForRemoteAccess`, `-userTestingSnapshotForElement:options:error:`, `XCAccessibilityElement -AXUIElement`, `XCTDefaultSnapshotParameters`, `XCAXAccessibilityAttributesForStringAttributes`, `+[AXElement primaryApp]`, `-[AXSettings setAutomationEnabled:]`, `_AXSAutomationEnabled` | XCTAutomationSupport, AXRuntime, AccessibilityUtilities, libAccessibility | load and resolve; `primaryApp` nil; the snapshot refused by the sandbox (`kAXErrorServerNotFound`) |

The public calls the runner makes: `-[XCUIApplication initWithBundleIdentifier:]`, `-[XCUIElement snapshotWithError:]`, `-[XCUIApplication coordinateWithNormalizedOffset:]`, `-[XCUICoordinate coordinateWithOffset:]`, `-[XCUICoordinate tap]`, `-[XCUIDevice pressButton:]`, `-[XCUIScreen screenshot]`.

### What was not measured

- **The mount of the developer disk image**, timed: see The developer disk image.
- **The tree with the reader's attributes**: neither the reader's `userTestingSnapshotForElement:` (refused by the sandbox) nor `XCAXClient_iOS`'s request with the reader's attribute names (its reply did not decode) read; the daemon's `_XCT_requestSnapshotForElement:attributes:parameters:reply:` with the attributes' numbers was not tried. So the device's tree carries no platform class and no scroll area, and the node-for-node comparison with the Simulator's reader through the mirror waits for task 4.4.
- **A keyboard window, an alert and a sheet on the device**, which the conversion's class rules handle on the Simulator; with no class they would not be recognized.
- **`dtuhidd` with the display stream open**, the research note's addition, since iOS 26.6.2 refuses the stream (S11); `dtuhidd` ran without one. Typing through Device Hub's virtual keyboard was not repeated; S11 typed through it.
- **The runner's synthesis that skips `XCUIDevice`**: the daemon proxy's `_XCT_synthesizeEvent:completion:` called directly was not timed beside the session's `synthesizeEvent:completion:`.
- **Whether "Enable UI Automation" is asked again after a restart**: the phone was not restarted.
- **The Expo fixture's taps and text**: the tap series ran on Settings, S11's screen, and the text series on the SwiftUI fixture.

## Verdict

**Go for the runner, with its tree from XCTest's snapshot and its input as the fallback.** D20's gates on iOS 26.6.2:

| Criterion | Series | Measured at p95 | Verdict |
|---|---|---:|---|
| tap dispatch under 150 ms | tap-synthesized, open | 294.1 | fail |
| tap dispatch under 150 ms | tap-synthesized, back | 295.0 | fail |
| tap dispatch under 150 ms | tap-synthesized-noconfirm, open | 291.5 | fail |
| tap dispatch under 150 ms | tap-synthesized-noconfirm, back | 291.3 | fail |
| tap dispatch under 150 ms | tap-coordinate, open | 795.4 | fail |
| tap dispatch under 150 ms | tap-coordinate, back | 606.8 | fail |
| tap dispatch under 150 ms | tap-dtuhidd, open | 53.5 | pass |
| tap dispatch under 150 ms | tap-dtuhidd, back | 53.2 | pass |
| tree under 1 s | snapshot-swiftui-big | 476.1 | pass |
| tree under 1 s | snapshot-expo-big | 527.3 | pass |
| no restart in the session | survival, 30 minutes | same runner pid true, 0 command errors | pass |

- **Tree**: passes on both fixtures, 476 and 527 ms at p95 for 546 and 529 nodes.
- **Tap dispatch**: the runner's taps fail, 294 ms at p95 synthesized, 292 ms with the implicit confirmation interval at 0 and 795 ms through `XCUICoordinate`, against 150. The host's `dtuhidd` taps pass at 53.5 ms, and change the screen sooner. D22 already allows either path; the host's is the default.
- **No restart**: passes. One runner served 600 cycles over 30 minutes with no failed command, and its memory grew by 2.4 MiB. A pulled cable ends the session; the runner outlives it, so a new attach must end it first.

**D20 changes** (rewritten in design.md): the runner is built as S10 built it: the Rust agent as a static library linked with the Objective-C shim into the test bundle by `clang -bundle` (an `MH_DYLIB` loads too), the bundle a copy of Xcode's `XCTRunner.app` with its executable thinned to arm64 and a launch screen, signed with `codesign` and the installed profile, so no `xcodebuild` host project is needed. It starts through `idevice`'s `xctest` session in 772 ms at p50 when installed and 1.8 s with the install, and the driver checks three conditions first: the disk image mounted, "Enable UI Automation" authorized once by the device owner, the phone unlocked. XCTest 25228 names its idle waits differently from WebDriverAgent's swizzle; the runner replaces all three.

**D21 changes**: the reader's `userTestingSnapshotForElement:` cannot run in the runner, whose sandbox denies the Mach lookup of other apps' accessibility servers. The tree comes from XCTest's snapshot, fetched by testmanagerd, turned into the reader's `Element`s and converted by the reader's `elements.rs` unchanged; the foreground comes from XCTest's active applications and the bundle from the daemon session. That tree has no platform class and no scroll area, so the class rules (keyboard window, alerts, sheets) and scroll semantics wait on the daemon's snapshot with the reader's attributes.

**D22 changes**: measured on the same screen and day, `dtuhidd` from the host is the default for taps, swipes, the home button and keys on iOS 26.6.2, the runner's synthesis the fallback. Text goes through the runner: typed through `_XCT_sendString:` (807 ms for 36 characters at 60 a second, all whole) or set through the daemon's `_XCT_setAttribute:` (163 ms, all whole).

**For the tasks:**

- **3.4, the runner session.** Drive `XCUITestService::run` on a task for the target's lifetime, poll usbmuxd's `Connect` to the agent's port until the hello, and stop by ending the task and sending SIGKILL through the app service. Check the mount, the authorization and the lock before the start, and name each in the error: "service not found" means no disk image; a 60 s wait in "enabling automation mode" means the owner has not approved. Retry when testmanagerd's handshake times out after an aborted session. Do not trust `idevice`'s message for XCTest error 103. After a disconnect, end the runner left over from the lost session before attaching: it outlived the pull and answered the next start's connection, then went away.
- **4.1, the reader as a library.** The device uses the conversion (`elements.rs`) and not the guest snapshot call, so the library's boundary is the conversion, with the Simulator's snapshot call and the device's XCTest snapshot as two sources of `Element`s.
- **4.2, the build script.** `runner/build.sh`'s steps: build the static library for `aarch64-apple-ios` with `IPHONEOS_DEPLOYMENT_TARGET=17.0`, link the shim and it with `clang -bundle`, copy the template, thin its executable to arm64, add `UILaunchScreen`, put the profile in, and sign the test bundle and then the app, the team from the environment.
- **4.3, the protocol end.** `AgentSession` on a loopback TCP listener, one worker thread, `busy` for a second request, the hello with the version; usbmuxd's `Connect` reaches it.
- **4.4, the tree.** Start from `xctree.rs`. To match the Simulator node for node, find the attribute shape the daemon's snapshot takes, so the platform class and the scroll areas come back; until then the device's classes are XCTest's element types.
- **4.5, input.** `dtuhidd` from the host by default (in `crates/ios`); the runner's `XCSynthesizedEventRecord` as the fallback; text typed through `_XCT_sendString:` or set through `_XCT_setAttribute:` on the focused element, the method reported.
