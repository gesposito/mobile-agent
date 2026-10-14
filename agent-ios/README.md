# agent-ios

The resident runner for a physical iPhone (design D20 to D22, spec
`ios-device-driver`): a Rust static library and an Objective-C shim linked
into an XCTest bundle, inside a re-signed copy of Xcode's `XCTRunner.app`.
The driver in `crates/ios` installs it, starts it through the device's test
manager once per session and reaches it over usbmuxd's `Connect` to the
loopback port 27183, where it serves the device protocol of
`crates/protocol`: the foreground's accessibility tree, read through
XCTest's accessibility client and converted by the Simulator reader's
`elements`; text typed or set into the focused field; and taps, swipes,
keys and buttons through XCTest's event synthesis, the fallback of the
host's `dtuhidd`. It replaces spike S10's throwaway runner
(`docs/spikes/s10-ios-runner.md`).

## Layout

```
agent-ios/
  VERSION          the runner's version: its hello's (build.rs) and the bundle's (build.sh)
  Cargo.toml       crate agent-ios: a static library for aarch64-apple-ios, an rlib for the host's tests
  build.rs         VERSION into the crate as AGENT_IOS_VERSION
  build.sh         builds and signs dist/MobileAgentRunner-Runner.app, no xcodebuild project
  shim/
    Runner.m       the XCTestCase, XCTest's idle waits replaced, the XCTest calls as C functions
  src/
    lib.rs         the name, version and port; agent_ios_start, which the shim's test calls
    server.rs      the protocol's AgentSession on the loopback port, one worker thread
    dispatch.rs    the requests onto the shim's calls, pixels to points, keys to characters
    shim.rs        the Shim trait, its calls into Runner.m on iOS and stubs elsewhere
    xctree.rs      the snapshot's JSON as the Simulator reader's Elements, and the read
  NOTES.md         checkpoint notes of the change's group 4
  out/   (git-ignored)   Runner.o, the test bundle's image, the entitlements
  dist/  (git-ignored)   MobileAgentRunner-Runner.app, which embeds the profile
```

The crate is a workspace member, so `cargo test`, `clippy` and `fmt` over the
workspace cover it on the host, where the shim's calls fail and the rest
runs: the server, the dispatch over a fake shim and the conversion.

## Requirements

On the Mac:

- Xcode 27.0 with the iPhoneOS SDK: its `XCTRunner.app` template, `clang`,
  `lipo`, `codesign`. Other Xcode versions were not tried.
- The Rust target `aarch64-apple-ios`, which `rust-toolchain.toml` names.
- A paid development team, and on the Mac its signing identity and
  provisioning profile, both the maintainer's to set up, since nothing here
  signs in to an account or registers a device:
  - the development certificate, created in Xcode (Settings > Accounts >
    Manage Certificates) with the team's account, so that the login
    keychain holds a valid "Apple Development" identity
    (`security find-identity -v -p codesigning` lists it);
  - an iOS App Development provisioning profile for the wildcard App ID
    `dev.mobileagent.*` that includes the phone, downloaded through Xcode;
    `build.sh` finds it by its name, `mobile-agent development` unless
    `MOBILE_AGENT_IOS_PROFILE` names another;
  - `MOBILE_AGENT_IOS_TEAM` exported, the team's identifier, which no
    tracked file holds.

On the phone, each step the person holding it takes:

- Developer Mode on: Settings > Privacy & Security > Developer Mode, then
  the restart and the passcode it asks for.
- The Mac trusted: unlock the phone, connect it over USB, tap Trust and
  enter the passcode.
- Unlocked, and Auto-Lock at Never for long runs.
- "Enable UI Automation" approved when iOS asks, with Touch ID or the
  passcode: testmanagerd asks the first time a runner starts after a
  restart, and iOS 26.6.2 asked once more the same day without one; XCTest
  waits 60 s for it, and the driver reports `automation_not_enabled` when
  nobody answers.

The developer disk image, which carries the test manager, is the driver's
to mount (`crates/ios`, Transport).

## Building and signing

```sh
export MOBILE_AGENT_IOS_TEAM=<team>
agent-ios/build.sh          # prints dist/MobileAgentRunner-Runner.app's path
just agent-ios-build        # the same, skipped without the team or the iPhoneOS SDK
```

`build.sh` runs four steps:

1. **The agent.** `cargo build --release --locked -p agent-ios --target
   aarch64-apple-ios` with `IPHONEOS_DEPLOYMENT_TARGET=17.0`: the static
   library `libagent_ios.a`.
2. **One image.** The iPhoneOS SDK's `clang` compiles `shim/Runner.m` with
   ARC and links it with the library by `clang -bundle` into the test
   bundle's `MH_BUNDLE`, 3.7 MB stripped, linking `@rpath/XCTest.framework`
   with rpaths to `/System/Developer`, where the developer disk image puts
   XCTest. Spike S10 found an `MH_DYLIB`, a `cdylib`'s shape, loads too.
3. **The bundle.** A copy of Xcode's `XCTRunner.app` named
   `MobileAgentRunner-Runner.app`:

   ```
   MobileAgentRunner-Runner.app/
     MobileAgentRunner-Runner          the template's executable, renamed and thinned to arm64
     Info.plist                        dev.mobileagent.runner, UILaunchScreen, VERSION
     embedded.mobileprovision          the development profile
     PlugIns/MobileAgentRunner.xctest/
       MobileAgentRunner               the image of step 2
       Info.plist                      dev.mobileagent.runner.xctest, BNDL, XCTContainsUITests, VERSION
   ```

   The template is fat, and an A12 or later phone runs its arm64e slice,
   which refuses the arm64 test bundle ("incompatible architecture (have
   'arm64', need 'arm64e')"), so the executable is thinned to arm64 as
   Xcode thins its own runners. Without a `UILaunchScreen` entry UIKit gives
   the runner the 320x480-point compatibility size. No XCTest framework is
   embedded: the template's rpaths and the test manager's
   `DYLD_FRAMEWORK_PATH` load them from the disk image.
4. **Signatures**, inside out with `codesign`: the test bundle, then the
   app with the team's `application-identifier`,
   `com.apple.developer.team-identifier` and `get-task-allow`, written to
   `out/entitlements.plist`; then `codesign --verify --deep --strict`. The
   identity is the one named `Apple Development` unless
   `MOBILE_AGENT_IOS_IDENTITY` names another. The test manager accepted the
   bundle signed outside Xcode on iOS 26.6.2.

`build.sh` refuses with a message naming the step when the team, the
iPhoneOS SDK, the template, the identity or the profile is missing. A
release build from scratch took 17.5 s on the maintainer's Mac and an
incremental one 1.7 s. `out/` and `dist/` are git-ignored: the signed bundle
embeds the profile, which names the team and its devices. `just
agent-ios-build` skips with a message without `MOBILE_AGENT_IOS_TEAM`,
without the iPhoneOS SDK, or with `SKIP_AGENT_BUILD=1`, and `just validate`
runs it after `agent-build`.

## Versions

`VERSION` holds one semantic version, `0.2.2`. `build.rs` reads it into the
crate, so the hello names `mobile-agent-ios-runner 0.2.2`, and `build.sh`
writes it into both Info.plists' `CFBundleShortVersionString`: the three
cannot disagree. The driver asks for a version (`ios::RunnerSpec`), and a
runner whose hello names another is replaced by the bundle the spec names;
`crates/ios`'s tests ask for `agent_ios::VERSION` and take the bundle from
`MOBILE_AGENT_IOS_RUNNER_APP`. Spike S10's runner was `0.1.0`, so the first
attach with this one replaced it on the phone (task 4.2); `0.2.1` serves the
keyboard's window bounded by what it draws while a hardware keyboard serves
the field in focus, nothing or the strip of a keyboard toolbar (rn-app-fixes,
design D52), and `0.2.2` the text effects window, which holds iOS's edit
menu, bounded by what it draws, the menu's list or nothing (design D55).
Change `VERSION` whenever the runner changes what it serves.

## How it runs

The driver starts the runner through `idevice`'s `xctest` session; XCTest
loads the test bundle and runs `MobileAgentRunnerTests.testRunner`, which
never returns: it logs the idle waits the shim replaced, calls
`agent_ios_start(27183)` and spins the main run loop, on which the shim
runs the XCTest calls that want the main thread. Its `+setUp` sets
`XCTDisableRemoteQueryEvaluation` and `DisableScreenshots`, as
WebDriverAgent's does.

- **Idle waits.** At load the shim replaces XCTest's waits for the
  application to go idle with no-ops, as WebDriverAgent's
  `XCUIApplicationProcess+FBQuiescence` does: XCTest 25228 (Xcode 27.0)
  names them `-[XCUIApplicationProcess
  waitForQuiescenceIncludingAnimationsIdle:isPreEvent:]`,
  `…usingActivity:isPreEvent:` and `-[XCAXClient_iOS
  waitForQuiescenceOnAllForegroundApplicationsAsPreEvent:]`; it has no
  `waitForQuiescenceIncludingAnimationsIdle:`, the selector WebDriverAgent
  replaces.
- **The server.** One host connection at a time on `127.0.0.1:27183`,
  driven by the protocol's `AgentSession`; every request runs on one worker
  thread, so the connection answers pings while a slow one runs, and a
  request during another is answered `busy`. A host that goes away, or
  `shutdown`, ends the connection and the runner takes the next; the runner
  ends with the test manager's session, which XCTest ends.
- **The hello** names `mobile-agent-ios-runner` and `VERSION`, protocol
  version 1, a 16 MiB frame limit and the capabilities `tree`, `input`,
  `screenshot` and `text_set`. It sends no change events and streams no
  frames: the host takes the device's frames itself (design D23).

Each shim call runs in an autorelease pool of its own, since the worker
thread has none, and returns a status with a message the agent frees.

## Serving the protocol

### `snapshot`

The foreground is the first of `-[XCAXClient_iOS activeApplications]`, or
its `systemApplication`, SpringBoard, when none is active, as on the home
screen; its bundle comes from the daemon session's
`requestApplicationSpecifierForPID:reply:`, asked once per pid, since the
runner's sandbox hides other processes' executables. The tree is
`-[XCAXClient_iOS requestSnapshotForElement:attributes:parameters:error:]`
on `+[XCAccessibilityElement elementWithProcessIdentifier:]` of that pid,
which testmanagerd reads with its own privileges, called off the main
thread since it waits for testmanagerd's reply:

- **Attributes.** The Simulator reader's names: the element type and base
  type, automation type, label, value, placeholder, identifier, frame,
  traits, whether the element takes touches, the vertical and horizontal
  scroll areas and the visible frame, but not `XC_kAXXCAttributeChildren`.
  With it, as spike S10 asked, testmanagerd's reply holds an array the
  runner's decoder refuses, and the call gives up after 65 s ("Timed out
  while fetching snapshot from testmanagerd"); without it the reply decodes.
  The snapshot's `additionalAttributes` are keyed by numbers, which
  `XCAXAccessibilityAttributesForStringAttributes` gives for the names.
- **The platform class** is `XC_kAXXCAttributeElementType` where
  testmanagerd returns it and `XC_kAXXCAttributeElementBaseType` otherwise;
  on iOS 26.6.2 it never returns the former, and the base type names
  `UIRemoteKeyboardWindow`, `UITransitionView`,
  `_UIAlertControllerPhoneTVMacView` or `UILabel` as the Simulator reader's
  class does, and `UIResponder` for a SwiftUI element, which the reader
  names `SwiftUI.AccessibilityNode`. The conversion names typed elements by
  their `XCUIElementType` either way, so only untyped containers differ.
- **Modal views are not honoured** (`snapshotKeyHonorModalViews` false), so
  what an alert covers is read too, as the Simulator reader's last read
  keeps it: 88 nodes on the SwiftUI fixture's System tab with its alert,
  where the default left out the 47 the alert covers.
- **Conversion.** `xctree` turns the JSON the shim writes into the reader's
  `Element`s, a scroll view keeping its scroll areas and visible frame and a
  dimming view whether it takes touches, as the reader reads them, keyboard
  focus becoming the editing trait; the reader's `elements::snapshot` makes
  the payload, every window sending no change events. The foreground is
  checked again after the read, and a change is `window_changed`, which the
  host repeats; no foreground is `no_foreground`, a failed request
  `snapshot_failed`.

On the phone the SwiftUI and Expo fixtures' Big tabs, the SwiftUI Login tab
with the keyboard up and its System tab with the in-app alert list through
the mirror as the Simulator reader's reads of the same screens
(`crates/ios/tests/tree.rs`): the keyboard as one `- keyboard` line, the
alert in a modal window above the application's. What the screens' sizes
decide differs and is left out of the comparison: a scroll indicator's page
count, the Expo grid's scroll hint, and the 24 cells of its last column
that the iPhone 17's scroll indicator covers and the SE's does not.

Measured with `tree::tree_series` (debug build, 20 reads after 2, request
to reply): the SwiftUI Big tab's 546 nodes in 550.3 and 554.3 ms at p50 and
p95 (load 3.80 before, 3.52 after), the Expo Big tab's 529 in 445.6 and
451.9 ms (3.64 and 4.17), under D20's 1 s. The same request with XCTest's
own 22 attribute names took 526 ms for the 546 nodes, and asking a scroll
view's areas element by element through `attributesForElement:attributes:error:`
107 ms each, which is why they are in the one request.

What the device's tree does not tell:

- a view the app marks modal (`accessibilityViewIsModal`) other than an
  alert: the Simulator reader marks it from a second read when the first
  one, honouring modal views, came back with fewer children than an
  element counted; this reply counts none, and a second read on every
  snapshot would double its cost;
- sheets, whose dimming view's interaction the read carries, and system
  alerts over an application were not exercised on the phone.

### `tap`, `swipe` and `key`

The fallback of the host's `dtuhidd` (D22): the driver sends them here
where the device's iOS takes no `dtuhidd` report without a display stream,
or when the session asks for it (`ios::InputChoice::Only(InputPath::Runner)`).
Positions come in device pixels and go to XCTest in points, at
`UIScreen`'s scale.

- A **tap** is an `XCSynthesizedEventRecord` with one `XCPointerEventPath`,
  a contact lifted after 50 ms or the request's duration, sent to
  `-[XCUIDevice eventSynthesizer]`, the daemon session, and acknowledged
  when its completion arrives, about 240 ms after the touch (S10), with no
  idle wait.
- A **swipe** moves the path to its end over its duration; with a rest it
  stays there that long, then creeps on along its line three moves of a
  point 16 ms apart and lifts at the last, as the host's swipes do, since
  UIKit takes a drag's velocity from its last moves.
- A **key** of the HID Keyboard page is typed as the character a US
  keyboard gives it, with shift when the meta state's first bit says so,
  through `_XCT_sendString`; other modifiers are refused. A hardware
  button's usage on the Consumer page in the code's high half is pressed
  through `-[XCUIDevice pressButton:]`: home (0x40), volume up (0xE9) and
  down (0xEA); `XCUIDevice` has no lock button, which the host presses.
- Where XCTest has no event synthesizer or no event classes, taps and
  swipes go through the public `XCUICoordinate` in SpringBoard's coordinate
  space (`pressForDuration:`, `pressForDuration:thenDragToCoordinate:…`),
  which resolves the application first: S10 timed its tap at 795 ms at
  p95. XCTest 25228 has the synthesizer, so that path was not exercised.

On the phone, through the session's choice of the runner (task 4.5,
`input::the_runners_synthesis_taps_presses_swipes_types_and_sets_text`,
functional, load about 10): a tap on `row-1` acknowledged in 313 ms and
`detail-1` in the tree 239 ms later, a 600 ms press acknowledged in 831 ms,
a resting swipe of 400 px acknowledged in 661 ms that moved the list 347 px
and stopped, a shifted key typed `A`, and the volume buttons pressed in 246
and 245 ms. The measured taps are in
Numbers; they miss D22's 150 ms, which is why `dtuhidd` is the default.

### `text_set`

Into the element with keyboard focus; a request naming a node is refused
(`invalid_params`), since the runner keeps no element of an earlier
snapshot:

- `keys`: typed through the daemon proxy's
  `_XCT_sendString:maximumFrequency:completion:` at 60 characters a second,
  WebDriverAgent's default, at the cursor;
- `direct`: the focused element, found in `-[XCUIElement
  snapshotWithError:]` of the foreground's application, gets its value
  through the daemon proxy's `_XCT_setAttribute:value:element:reply:` with
  the value attribute's number, 2006, replacing what it held;
- left to the runner: typed, and set where typing fails.

The reply names the method that took the text. On the phone the harness's
36 characters (`abcdefghijklmnopqrstuvwxyz0123456789`) went into the SwiftUI
fixture's `login-username` typed in 815 ms, set in 207 ms and typed on the
runner's choice in 811 ms, each read back whole from the tree.

### `screenshot`

`-[XCUIScreen screenshot]`'s PNG at the screen's size, whatever the request
asks: 39 ms for the SE's 750x1334 PNG. The daemon proxy's
`_XCT_requestScreenshotOfScreenWithID:withRect:uti:compressionQuality:withReply:`,
WebDriverAgent's call, is refused as a legacy request (S10). The driver
takes its screenshots and frames from the host (D23).

## Numbers

Measured on 2026-10-09 on an iPhone SE (3rd generation) with iOS 26.6.2,
on USB, unlocked, from an Apple M1 Max with macOS 26.6.2 and Xcode 27.0, by
`crates/ios`'s device tests (debug builds of the driver; the runner is
always a release build), each series started behind the quiet-host guard
(a 1-minute load under 4, no build or emulator process above 5% of a core).
Percentiles are nearest rank, times from the host's request to its reply.

| Series | n | p50 | p95 | Load before / after |
|---|---:|---:|---:|---|
| tree, SwiftUI Big tab, 546 nodes | 20 | 550.3 ms | 554.3 ms | 3.80 / 3.52 |
| tree, Expo Big tab, 529 nodes | 20 | 445.6 ms | 451.9 ms | 3.64 / 4.17 |
| tap through the runner's synthesis, three runs | 20 each | 292.2 to 295.2 ms | 298.4 to 300.0 ms | 3.12 / 2.91, 3.48 / 2.13, 1.66 / 2.73 |
| a tap's first changed frame from the request, three runs | 20 each | 198 to 210 ms | 246 to 278 ms | |
| 36 characters typed, three runs | 20 each | 809 to 812 ms | 813 to 817 ms | |
| 36 characters set, three runs | 20 each | 215 to 216 ms | 225 to 237 ms | |

Every text entry was read back whole, 120 of 120; 3 of 60 taps, all in
the first run, showed no changed frame within 2 s. The tree passes D20's
1 s; the taps miss D22's 150 ms, which the host's `dtuhidd` meets at 57 ms
(`crates/ios`, Numbers). Functional numbers, not measured (loads of 4 to
11): the start to the hello 776 to 842 ms, an attach 1.5 to 1.7 s, with
the install of a new version 4.9 s; a screenshot 36 to 39 ms; fifty mixed
commands on one runner process with no restart.

## Call sites

Every private or undocumented call the runner makes, validated on iOS
26.6.2 (23G90) on an iPhone SE (3rd generation) with the developer disk
image of Xcode 27.0 (27A266a, XCTest 25228), the runner built with Xcode
27.0's iPhoneOS SDK. Selectors and type encodings were read from that
SDK's frameworks with `otool -oV`. `crates/ios`'s compatibility table,
which `doctor` prints, carries the same rows.

| Call | Image or service | Outcome on iOS 26.6.2 |
|---|---|---|
| `-[XCUIApplicationProcess waitForQuiescenceIncludingAnimationsIdle:isPreEvent:]`, `…usingActivity:isPreEvent:`, `-[XCAXClient_iOS waitForQuiescenceOnAllForegroundApplicationsAsPreEvent:]` replaced at load | XCUIAutomation | replaced; `waitForQuiescenceIncludingAnimationsIdle:` alone no longer exists |
| `-[XCAXClient_iOS activeApplications]`, `-systemApplication` | XCUIAutomation | the foreground, SpringBoard when no application is active |
| `-[XCTRunnerDaemonSession requestApplicationSpecifierForPID:reply:]`, `-[XCUIApplicationSpecifier bundleIdentifier]` | XCTestCore, testmanagerd | the foreground's bundle |
| `-[XCAXClient_iOS requestSnapshotForElement:attributes:parameters:error:]` on `+[XCAccessibilityElement elementWithProcessIdentifier:]`, `defaultParameters` with `snapshotKeyHonorModalViews` false; `XCUIElementSnapshotRequestResult` `_rootElementSnapshot`; `XCElementSnapshot` `additionalAttributes` | XCUIAutomation, testmanagerd (`_XCT_requestSnapshotForElement:attributes:parameters:reply:`, the daemon session's `useLegacySnapshotPath` false) | the tree, without `XC_kAXXCAttributeChildren` (with it, no reply after 65 s); `XC_kAXXCAttributeElementType` never returned, the class from `XC_kAXXCAttributeElementBaseType` |
| `XCAXAccessibilityAttributesForStringAttributes` | XCTAutomationSupport | ElementType 5003, ElementBaseType 5004, IsUserInteractionEnabled 5034, VisibleVerticalScrollArea 5027, VisibleHorizontalScrollArea 5026, VisibleFrame 2057 |
| `-[XCUIDevice eventSynthesizer]`, `-synthesizeEvent:completion:` with `XCSynthesizedEventRecord -initWithName:interfaceOrientation:`, `-addPointerEventPath:`, `XCPointerEventPath -initForTouchAtPoint:offset:`, `-moveToPoint:atOffset:`, `-liftUpAtOffset:` | XCUIAutomation, XCTestCore, testmanagerd | works; the completion comes about 240 ms after the touch |
| daemon proxy `_XCT_sendString:maximumFrequency:completion:` | testmanagerd | works, 60 characters a second |
| daemon proxy `_XCT_setAttribute:value:element:reply:` with 2006, on the focused element of `-[XCUIElement snapshotWithError:]` | testmanagerd | works; `-[XCAXClient_iOS setAttribute:value:element:outError:]` with the number and `_XCT_performAccessibilityAction:onElement:withValue:reply:` fail with `kAXErrorIllegalArgument` (S10) |
| `-[XCUIDevice pressButton:]` | XCUIAutomation | home acknowledged in 502 ms (S10); no lock button |
| `-[XCUIScreen screenshot]` | XCUIAutomation | a 750x1334 PNG in 39 ms; the daemon proxy's legacy screenshot request is refused (S10) |
| `XCUICoordinate -pressForDuration:`, `-pressForDuration:thenDragToCoordinate:withVelocity:thenHoldForDuration:` | XCUIAutomation | not exercised: XCTest 25228 has the synthesizer; S10 timed `-[XCUICoordinate tap]` at 795 ms at p95 |

The public calls: `-[XCUIApplication initWithBundleIdentifier:]`,
`-[XCUIElement snapshotWithError:]`, `-[XCUIApplication
coordinateWithNormalizedOffset:]`, `-[XCUICoordinate
coordinateWithOffset:]`, `-[XCUIDevice pressButton:]`, `-[XCUIScreen
screenshot]`, `UIScreen`'s scale.

## Tests

`cargo test -p agent-ios` runs the host's tests, which need no device:

- `server`: the hello's name, version and capabilities; requests answered
  through the handler and its failures; `busy` for a request during
  another; pings answered through a 3.5 s request; `shutdown` and a host
  that goes away, the runner taking the next connection; and the protocol's
  golden request frames (`crates/protocol/golden`) written byte for byte
  after the golden welcome, each reaching the handler as the call it holds
  or answered by the session itself.
- `dispatch`, over a fake shim on a screen of scale 2: the snapshot's
  foreground, bundle and `window_changed`, SpringBoard as system UI,
  `no_foreground`, `snapshot_failed`; the screenshot's size; taps and
  swipes in points with their hold and rest; keys with and without shift,
  the buttons, refused modifiers and the lock button; text typed, set and
  set where typing fails, a node refused.
- `xctree`: the JSON as `Element`s with their classes, scroll areas and
  interaction, and through the reader's conversion: the keyboard's window,
  an alert's modal window, a scroll view's semantics.

The device tests live with the driver's, in `crates/ios/tests/`, and drive
this runner through the driver's `Session`; they skip and pass without
`MOBILE_AGENT_IOS_UDID`. With the phone on USB, unlocked, the fixtures
installed and the runner built:

```sh
export MOBILE_AGENT_IOS_UDID=<udid> MOBILE_AGENT_IOS_TEAM=<team>
export MOBILE_AGENT_IOS_RUNNER_APP=agent-ios/dist/MobileAgentRunner-Runner.app
cargo test -p ios                    # runner.rs, tree.rs, input.rs among them
cargo test -p ios --test tree -- --ignored --nocapture tree_series
cargo test -p ios --test input -- --ignored --nocapture runner_input_series
# the Simulator's listings tree.rs compares with, on a booted Simulator with both fixtures
MOBILE_AGENT_SIM_UDID=<udid> cargo test -p ios --test tree -- --ignored --nocapture record
```

`MOBILE_AGENT_IOS_RUNNER_APP` names the bundle an attach installs when the
phone's runner is missing or says hello with another version, a relative
path taken from the workspace's root. A runner of the same version is not
reinstalled: after changing the runner without changing `VERSION`, install
it by hand (`xcrun devicectl device install app --device <udid>
agent-ios/dist/MobileAgentRunner-Runner.app`) or change `VERSION`.

## Spec requirements and their checks

| Requirement | Scenario | Checks |
|---|---|---|
| One resident runner per session | Runner stays up | `crates/ios`: `runner::the_runner_serves_fifty_commands_with_one_process_and_no_restart`; here: `server::tests` |
| One resident runner per session | Runner replaced on version change | `crates/ios`: `runner::the_built_runner_replaces_another_version_and_says_hello_with_its_own`, which replaced spike S10's `0.1.0` on the phone; here: `tests::the_hello_names_the_runner_and_the_version_file`, `server::tests::the_hello_names_the_runner_its_version_and_its_capabilities` |
| Foreground tree read | Reference screen read | `crates/ios`: `tree::the_fixtures_screens_list_through_the_mirror_as_on_the_simulator`, `tree::tree_series`; here: `xctree::tests`, `dispatch::tests::a_snapshot_reads_the_foreground_applications_tree_and_asks_its_bundle_once`, `dispatch::tests::the_home_screen_is_read_from_springboard_as_system_ui`, `dispatch::tests::a_foreground_that_changes_during_the_read_is_window_changed` |
| Input actions without idle waits | Tap dispatched | `crates/ios`: `input::the_runners_synthesis_taps_presses_swipes_types_and_sets_text`, `input::runner_input_series`, `macos::hid::tests::a_session_that_asks_for_the_runners_synthesis_gets_it_where_dtuhidd_takes_reports`; here: `dispatch::tests` on taps, swipes, keys and text |
| Compatibility statement | Table lists the device paths | `crates/ios`: `compat::tests::every_call_site_names_the_ios_and_xcode_it_was_validated_on`, which names this runner's calls; this README's Call sites |
