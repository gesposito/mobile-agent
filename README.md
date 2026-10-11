# mobile-agent

mobile-agent drives Android devices, Android emulators, iOS Simulators and physical iPhones for AI agents, and is designed around the latency of each tap-and-observe step. A daemon keeps a live mirror of every attached screen (its windows, accessibility tree and latest frame), updates it from the platform's change events and frame callbacks, and answers snapshots, waits and settle checks from that mirror instead of reading the device and sleeping on every request. On Android, a single-dex agent pushed over adb runs as the shell user under `app_process` and holds the device's UI automation role: no APK is installed and no instrumentation runs. On the iOS Simulator the driver works from the host: input goes through the Simulator's dtuhidd HID service, frames come from the display's IOSurface, and the accessibility tree comes from a Rust reader that the daemon spawns into the Simulator, with no XCTest runner and nothing installed. A physical iPhone's tree can be read only from an XCTest runner, so there a Rust runner built in this repository, started through the phone's test manager and kept for the session, serves the tree and text entry over the same device protocol, while the driver does the rest from the host: input through the developer disk image's dtuhidd, frames from screenshots it polls, and the phone's services over a CoreDevice tunnel it opens in its own process, without root or a helper. The `mobile-agent` CLI follows agent-browser's command grammar, `@eN` refs and JSON envelope, so prompts and skills written for agent-browser carry over where devices have the same concepts.

## Supported platforms

| Platform | Status | Tested on |
|---|---|---|
| macOS host | Simulator, Android and physical iPhone drivers, the iPhone's only when `MOBILE_AGENT_DRIVERS` names `ios` | macOS 26.6.2, Apple M1 Max |
| Linux host | Android driver only; the Simulator and iPhone drivers build as stubs | CI builds and tests it on `ubuntu-latest`, without a device |
| Xcode | for the Simulator driver, 26.0 up to, not including, 28; other versions are refused with `unsupported_xcode`. The iPhone's runner is built from Xcode's `XCTRunner.app` and iPhoneOS SDK, tried with 27.0 alone | Xcode 27.0 (27A266a); CI also builds with Xcode 26 |
| iOS Simulator | booted Simulators; no gate on the runtime version | iOS 26.5 (23F77), iPhone 17 |
| Android emulator | fast path through the emulator's gRPC controller when it answers, the agent's path with `--emulator-controller off` | API 36, arm64, Pixel 8 profile, Android Emulator 37.1.11 |
| Android agent | minimum API 26: the dex is built with `--min-api 26`, and a platform member that exists only from a later level is read behind a check of the level (`agent-android/src/dev/mobileagent/agent/Levels.kt`) | API 36 on the emulator; API 27 (Android 8.1) on a Nexus 5X, from agent 0.4.3 |
| Physical Android device | through the same agent, without a frame stream: taps and screenshots go through the agent | a Nexus 5X on Android 8.1 (API 27), a `user` build (`reports/10.3-physical-android.md`) |
| Physical iPhone | on USB, through a runner signed for a paid development team (Quick start); iOS 17.4 up to, not including, 27, other versions refused with `unsupported_ios`; input from the host from iOS 18, through the runner below it; built and measured for iOS 26, whose dtuhidd takes the host's input without a display stream, while iOS 27's, which needs one, is a later path (`crates/ios/README.md`, Compatibility) | an iPhone SE (3rd generation) on iOS 26.6.2 under Xcode 27.0, with `idevice` 0.1.68 (`reports/11.1-ios-device.md`) |

CI (`.github/workflows/ci.yml`) runs `just validate` in three jobs: Linux, which also builds the Android agent and the Compose fixture; macOS with Xcode 26 on GitHub's `macos-26` image; and macOS with Xcode 27 on its `xcode-27` image. Each macOS job keeps the image's default Xcode when it has that major version and otherwise selects the newest `Xcode_<major>*.app`. The macOS jobs build and test the iPhone driver (`crates/ios`, on its fake device) and the runner's crate (`agent-ios`, on the host); nothing runs against a phone, the device tests skipping without `MOBILE_AGENT_IOS_UDID`, and the signed runner bundle is not built there, since `just agent-ios-build` skips with `SKIP_AGENT_BUILD=1` or without `MOBILE_AGENT_IOS_TEAM`, as `just agent-build` skips the Android agent with `SKIP_AGENT_BUILD=1`. The Simulator integration tests, the spikes and every measurement ran on Xcode 27.0 with CoreSimulator 1171.7 and iOS 26.5. Nothing has run on Xcode 26, where input would be touch only, through the older Indigo transport (`crates/simulator/README.md`, Compatibility).

## Build

Requirements:

- **Rust.** Install rustup. It takes the toolchain from `rust-toolchain.toml`: the version pinned there, with clippy, rustfmt and the `aarch64-apple-ios-sim` target. Run `rustup toolchain install` in the checkout, as CI does, to install them before the first build, or when cargo reports that the pinned toolchain is not installed; `rustup show active-toolchain` there names the toolchain in use.
- **macOS, for the Simulator.** Xcode 26 or 27, selected with `xcode-select`, with an iOS Simulator runtime; and libjpeg-turbo, `brew install jpeg-turbo`, which the frame encoder links statically from Homebrew's Apple silicon prefix (`.cargo/config.toml`; set `TURBOJPEG_LIB_DIR` and `TURBOJPEG_INCLUDE_DIR` for another prefix).
- **The Android agent.** The daemon's build compiles the agent with `agent-android/build.sh` (kotlinc and R8, no Gradle) and embeds the jar, so it needs a JDK 17 or newer with `java` and `jar` on `PATH` (tested with Temurin 21), the Kotlin compiler, `kotlinc` on `PATH` or `KOTLIN_HOME` set to the compiler's home, the directory holding `bin/kotlinc` and `lib/` (tested with 2.4.20), and an Android SDK at `ANDROID_HOME`, by default `$HOME/Library/Android/sdk`, with `platforms;android-36` and a `build-tools` package (CI installs 37.0.0). On Linux, set `ANDROID_HOME`.
- **Android at run time.** The SDK's `platform-tools`, whose adb server the driver talks to on port 5037, and for an emulator, the `emulator` package and an AVD.
- **The iPhone's runner.** `agent-ios/build.sh`, which `just agent-ios-build` runs, builds the runner for `aarch64-apple-ios`, the second target `rust-toolchain.toml` names, links it into a copy of Xcode's `XCTRunner.app` with the iPhoneOS SDK's `clang` and signs it with `codesign` for the team `MOBILE_AGENT_IOS_TEAM` names, with no `xcodebuild` project, into the git-ignored `agent-ios/dist/` (`agent-ios/README.md`, Building and signing). It needs Xcode 27.0's iPhoneOS SDK, the only one tried, and the team's identity and profile (Quick start). The daemon installs that bundle from the workspace it was built in, or the one `MOBILE_AGENT_IOS_RUNNER_APP` names, when the phone's runner is missing or says hello with another version (`crates/daemon/README.md`, The physical iPhone driver).

From the checkout:

```sh
cargo build --release                      # target/release/mobile-agent and mobile-agent-daemon
export PATH="$PWD/target/release:$PATH"    # the CLI starts the daemon it finds beside itself
```

- `SKIP_AGENT_BUILD=1 cargo build --release` builds without the Android toolchain. That daemon deploys an agent only from a jar that `MOBILE_AGENT_JAR` names at run time; `MOBILE_AGENT_JAR` set at build time embeds that jar instead of building one.
- The Simulator's tree reader is cross-compiled for `aarch64-apple-ios-sim` during the build and embedded in the daemon, behind the `reader` feature of `crates/simulator`, which is on by default. The daemon spawns it into each booted Simulator it attaches. Without that Rust target the build warns and embeds no reader, and tree reads fall back to the slower host translator, marked degraded; `doctor` shows which one a build has.
- Contributors: every dependency install goes through Socket Firewall. Install it with `npm i -g sfw`, then prefix installs with it: `sfw cargo fetch`, `sfw cargo add`, `sfw cargo install just`, `sfw npm install -g @fission-ai/openspec`. `just validate` runs the checks a change must pass: build, clippy with warnings denied, format check, `check-paths`, tests, the agent build, the iPhone runner's build, the Kotlin codec cross-check, the fixture scripts' checks with fake build tools, the fixture builds (with the SwiftUI and Expo fixtures' device builds when `MOBILE_AGENT_IOS_TEAM` is set) and `openspec validate`. The tests that need a Simulator, an emulator or an iPhone skip unless their environment names one (`crates/daemon/README.md`, `crates/simulator/README.md`, `crates/ios/README.md`).

## Quick start

The daemon attaches to targets that are already running; it boots nothing. For Android, put the SDK's tools on `PATH`, start an adb server, and start an emulator with its gRPC control endpoint, whose port is the console port plus 3000, or connect a device with USB debugging on and its prompt accepted:

```sh
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"
adb start-server
emulator -avd <name> -grpc 8554 -grpc-use-token      # in a terminal of its own
```

Without the control endpoint, taps and screenshots go through the agent and there is no frame stream; `doctor` and `status` say how to enable it on a running emulator: `adb -s emulator-5554 emu grpc 8554`. Emulator 37.1.11 serves the endpoint even when started without `-grpc` (only `-ports` leaves it off), so to take the agent's path on an emulator that serves it, as a physical device does, start the daemon with `--emulator-controller off` or `MOBILE_AGENT_EMULATOR_CONTROLLER=off`; `status` and `doctor` report the setting. For the iOS Simulator, boot one in Simulator.app or with `xcrun simctl boot <name or UDID>`.

For a physical iPhone, the daemon registers its driver only when `MOBILE_AGENT_DRIVERS` names `ios` (`MOBILE_AGENT_DRIVERS=ios` drives iPhones alone): its listing checks the runner's signing identity and probes each phone through lockdown, and a phone whose lockdown does not answer holds the listing for 10 s, so a daemon that drives no phone pays for neither (`crates/daemon/README.md`, Drivers). The rest are the maintainer's steps, done once by hand, since nothing here signs in to an account or registers a device (`agent-ios/README.md`, Requirements):

- On the Mac: a paid development team's Apple Development identity in the login keychain and an iOS development provisioning profile for the App ID `dev.mobileagent.*` that includes the phone, both made in Xcode; the team's identifier exported as `MOBILE_AGENT_IOS_TEAM`; and the runner built with `just agent-ios-build`.
- On the phone: Developer Mode on (Settings > Privacy & Security, then the restart it asks for), the Mac trusted, the phone unlocked on USB, and "Enable UI Automation" approved with the passcode when a runner's start asks for it. XCTest waits 60 s for the approval, and the driver then reports `automation_not_enabled`. The SE asked again from time to time, also with Settings > Developer > Enable UI Automation turned on (2026-10-10): one approval lasted the eight hours of task 9.6's runs, and the next start, about an hour after them, asked again (task 9.9).

`devices` lists each phone in its readiness state, `ready` or the first step it lacks (`not_trusted`, `developer_mode_off`, `disk_image_unmounted`, `device_locked`, `no_signing_identity`, `unsupported_ios`), and `doctor` names the step, with its `ios-signing`, `ios-runner` and `ios-devices` checks and the iPhone driver's rows in the compatibility table: the supported range, then each call site with the iOS and Xcode it was validated on (`crates/ios/README.md`, Compatibility). The driver mounts the developer disk image when the phone lacks it and installs the runner itself.

Settings ships with every device, so it needs no fixture app. Type, using the refs your own listing prints:

```sh
mobile-agent doctor                         # toolchains, adb server, emulators, Xcode compatibility
mobile-agent devices                        # connected targets and their state
mobile-agent open com.apple.Preferences     # on Android: mobile-agent open com.android.settings
mobile-agent snapshot -i                    # actionable elements, each with a ref
mobile-agent click @e11                     # a ref from that listing, General in the run below
mobile-agent wait --settle
mobile-agent --json snapshot -i
mobile-agent close                          # stop the app and release the target
mobile-agent shutdown                       # stop the daemon; it returns once its targets have stopped
```

With several targets connected, give `open` a `--platform android|ios`, or a `--device <id>` when targets share a platform; the session then keeps using the target it holds. The first command starts the daemon, which exits after an hour without requests (`--idle-timeout`). `shutdown` replies once the daemon has stopped its targets, an Android agent or a Simulator's reader, and removed its socket, and names a target that had not stopped within 5 s in `data.notStopped`. `mobile-agent status` reports the daemon, its targets and, for each attached target, the paths its input, frames and tree take.

A run on an iOS 26.5 Simulator, trimmed where `…` stands. In `doctor`, ⚠ marks a warning, which does not fail it; `sfw` and `just` matter only to contributors. Ref numbers are never reused, and `open` lists the screen while it waits for the app, so the first listing need not start at e1. The degraded verdict is the quality check at work: Settings' search field holds two identical Dictate buttons that only their position tells apart.

```
$ mobile-agent doctor
✓ daemon               running, pid 11976, version 0.1.0+640310.18d934c42feb802c
✓ drivers              simulator, android
✓ xcode                Xcode 27.0 (27A266a) at /Applications/Xcode.app/Contents/Developer
✓ simulator-runtimes   iOS 26.5
✓ android-sdk          $HOME/Library/Android/sdk, adb 1.0.41 (37.0.1-15733141)
✓ adb-server           running on port 5037, protocol 41
✓ emulator-control     gRPC control: emulator-5554 on port 8554
⚠ sfw                  sfw is not on PATH
    hint: install it with npm i -g sfw; every install goes through Socket Firewall
✓ just                 just 1.58.0
✓ xcode-compatibility  29 validated combinations, listed below

Xcode compatibility:
  Xcode 27.0 (27A266a)  Xcode 26.0 up to but not including 28  validated  validated on Xcode 27.0; the CI matrix also builds on Xcode 26
  CoreSimulator 1171.7  1155.4 or later for dtuhidd  validated  input through dtuhidd
  …
$ mobile-agent devices
B0647CDD-575F-45E9-9398-F6F5AFF23C1F  ios      simulator         iPhone 17           booted
7B292E1E-7A29-41A5-9C69-2ABA1CEB3683  ios      simulator         mobile-agent-spike  booted
emulator-5554                         android  android-emulator  sdk_gphone64_arm64  device
$ mobile-agent --device 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 open com.apple.Preferences
✓ Opened com.apple.Preferences on 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683
$ mobile-agent snapshot -i
- heading "Settings" [ref=e9]
- button "Apple Account, Sign in to access your iCloud data, the App Store, Apple services, and more." [id=com.apple.settings.primaryAppleAccount, ref=e10]
- button "Ready for Apple Intelligence" [id=com.apple.settings.followUpItem.com.apple.icloud.gm, ref=e24]
- button "General" [id=com.apple.settings.general, ref=e11]
- button "Accessibility" [id=com.apple.settings.accessibility, ref=e12]
…
- button "Screen Time" [id=com.apple.settings.screenTime, ref=e19]
(more items below, scroll down to reveal)
- searchbox "Search" [ref=e21]
  - button "Dictate" [id=Dictate, ref=e22]
  - button "Dictate" [id=Dictate, ref=e23]
⚠ degraded snapshot (sibling_collision): e22, e23 are identical siblings told apart only by position; take a new snapshot right before acting on one of them
$ mobile-agent click @e11
✓ Done (dispatch 0.1 ms, first frame change 346.0 ms, first tree delta 445.4 ms)
$ mobile-agent wait --settle
✓ Settled after 1054.8 ms: frames changed last
$ mobile-agent --json snapshot -i
{"success":true,"data":{"origin":"com.apple.Preferences","readMs":0.073,"refs":{"e25":{"name":"Settings","role":"button"},"e26":{"name":"General","role":"heading"},…},"removedRefs":["@e9","@e10",…,"@e24"],"replicaRevision":7,"servedFromReplica":true,"snapshot":"- button \"Settings\" [id=BackButton, ref=e25]\n- heading \"General\" [ref=e26]\n…","target":"7B292E1E-7A29-41A5-9C69-2ABA1CEB3683","verdict":{"backend":"simulator-reader","degraded":false,"fallback":false,"reasons":[]}},"error":null}
$ mobile-agent close
✓ Closed com.apple.Preferences and released 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683
$ mobile-agent shutdown
✓ Daemon stopped
```

The Android half was verified on the API 36 emulator on 2026-09-28, with a release build made as above (trimmed):

```
$ mobile-agent doctor
…
✓ android-sdk          $HOME/Library/Android/sdk, adb 1.0.41 (37.0.1-15733141)
✓ adb-server           running on port 5037, protocol 41
✓ emulator-control     gRPC control: emulator-5554 on port 8554
…
$ mobile-agent devices
emulator-5554  android  android-emulator  sdk_gphone64_arm64  device
$ mobile-agent --platform android open com.android.settings
✓ Opened com.android.settings on emulator-5554
$ mobile-agent snapshot -i
- button "Search Settings" [id=search_action_bar, ref=e1]
(1 more item above, scroll up to reveal)
- button "Network & internet Mobile, Wi‑Fi, hotspot" [ref=e3]
…
(11 more actionable items below, scroll down to reveal)
$ mobile-agent close
✓ Closed com.android.settings and released emulator-5554
$ mobile-agent shutdown
✓ Daemon stopped
```

## Agent guide

`mobile-agent help agent` prints a compact form of this section for an agent's prompt (`crates/cli/src/agent-guide.txt`), and every command's `--help` ends with examples and states its defaults. The JSON examples below come from a daemon serving fake targets (`crates/daemon/README.md`, The fake driver): a launcher, a Settings app with one Wi-Fi switch and the fixture app, whose timings are not a device's.

```sh
mobile-agent snapshot -i       # 1. list actionable elements with their refs
mobile-agent click @e3         # 2. act on a ref from that snapshot
mobile-agent wait --settle     # 3. let the screen settle
mobile-agent snapshot -i       # 4. snapshot again before the next action
```

**Output.** `--json` (or `MOBILE_AGENT_JSON=1`) prints one line per command in agent-browser's envelope: `success`, `data`, `error`, then `code` and `warning` only when set. Every failure exits with status 1 and carries a snake_case `code`; in JSON mode its message travels in the envelope on stdout, without `--json` it goes to stderr. A `warning` marks a success not to take at face value: a degraded snapshot, an `open` whose screen did not settle, a settle that timed out, a click on an element partly but not mostly in view, a `fill` whose field showed nothing to read back, a `shutdown` that left a target stopping.

```json
{"success":false,"data":null,"error":"e4 is stale: the button \"Fixture\" it named is no longer on the screen. Take a new snapshot and use its refs","code":"stale_ref"}
```

**Snapshots.** A line reads `- role "name" [attributes, ref=eN]: value`. `snapshot -i` lists only elements with refs: actionable elements and named headings, cells and list items. `snapshot` lists every element, texts and system bars included, and `-d N` limits the depth. A line in parentheses, such as `(12 more actionable items below, scroll down to reveal)`, or `(more items below, scroll down to reveal)` when the scroll range gives no count, stands for content scrolled out of view. In `data`, `snapshot` holds the listing, `refs` maps `eN` to `{role, name}`, `removedRefs` names the refs the previous snapshot listed that are gone, `origin` is the app in front, and `verdict` gives the `backend`, whether the read is `degraded` or came from a `fallback`, and the `reasons` with advice (`fallback_backend`, `truncated`, `sparse_tree`, `sibling_collision`, `agent_warning`, `disconnected`); a degraded verdict also sets `warning`. `readMs` is how long the daemon took to answer, device reads included, and `servedFromReplica` is true when the mirror answered without reading the device. Windows that send no change events, every Simulator and iPhone window and Jetpack Compose windows on Android, are re-read after each injected action, when frames change and on a 1 s poll, an iPhone's every 10 s while its frames hold still, so a snapshot right after an action can wait for that read.

**Delta snapshots.** `snapshot --delta` answers with agent-browser's delta object, relative to this session's last delta snapshot. Its `kind` is `full` (the whole listing, and always the first time), `unchanged`, or `delta`, which carries ref `changes` (`add`, `remove`, or `replace` of the role or name) and a `treeChange` splice (`startLine`, `deleteCount`, `lines`) that rebuilds the new listing from the previous one. The whole listing comes back when the delta would be 70% of its size or more, or when the options, the replica or the app in front changed; `--full` asks for it. `diff snapshot` instead compares the screen with this session's last snapshot element by element.

```json
{"success":true,"data":{"origin":"com.android.settings","readMs":0.123,"replicaRevision":5,"servedFromReplica":true,"snapshot":{"baseRevision":1,"kind":"unchanged","revision":2},"target":"emulator-5554","verdict":{"backend":"fake","degraded":false,"fallback":false,"reasons":[]}},"error":null}
```

**Refs.** Write `@e3`; `ref=e3` and `e3` work too, and a number is never reused. A ref names one control for as long as it stays on screen: its element and, for an element without an identifier, also its accessible name. When the element goes, even briefly (another screen, a relaunch, a sheet that hides the screen beneath it), or an element without an identifier changes its name, the next snapshot gives it a fresh ref and the old one fails with `stale_ref`. A resync after an event sequence gap, a reconnection or a fall back to another tree backend rebuilds the mirror and makes every earlier ref stale. A ref from another session's snapshots fails with `unknown_ref`; an element under a dialog, or on iOS under a view drawn over it (a fixed footer, a floating button, a sticky header), fails with `element_covered` naming the cover, and one with none of it in view after a fresh read with `element_offscreen`. Refs never wait.

**Acting and reading.**

| Command | Does |
|---|---|
| `click @e4`, `click 200 480`, `click @e4 --hold 800` | taps a ref, a selector or a point in device points; holds for a long press |
| `fill @e2 agent` | replaces a field's text and reads it back; the reply's `method` says whether it was set `direct` or typed as `keys`. Typed text counts once the field has held it 200 ms, since an app that sets its field's value again can interleave with the keys; text set or pasted in one piece counts at the first read that shows it (task 10.30). Text typed that the field took otherwise is entered again in one piece, pasted on a Simulator and set directly on an iPhone and on Android, with a warning naming what the keys left and `retried: true`; a field that still holds other text fails with `text_mismatch` (task 10.23). The read-back takes the field's value, or what lies inside it (`readFrom: descendants`, a code field's boxes), and a field that shows nothing to read succeeds with `verified: false`, `reason: no_value_to_read` and a warning. On a Simulator `direct` is a paste, which replaces the pasteboard; on an iPhone, through its runner, and on Android a set. A field `fill` focuses itself on iOS gets its keys once the keyboard window is up (`rn-app-fixes`) |
| `type @e2 ' smith'` | adds key events to the field, or to the focused one when no target is given |
| `press Enter` | presses a key, such as `Home`, `Back` on Android, a letter, or modifiers joined with `+` as in `Control+a` |
| `swipe up`, `swipe 200 700 200 200` | moves the finger in a direction or between two points |
| `get text @e5`, `get value @e2`, `get box @e4` | reads an element's text, value or bounds |
| `screenshot [path]` | saves the latest frame as a JPEG, quality 80 and longest edge 1000 px by default |

**Selectors.** Where a ref goes, a selector works too, and waits up to the timeout for a match: `#id` or `id=...` for a test identifier, `text=...` or bare text for a name or value that contains the text, `text==...` for one that is exactly the text, `role=...`.

**Clicks and their timing.** `click` (alias `tap`) replies once the tap's first frame change and first tree delta have arrived, or after 200 ms without a change, at most 1 s; after the first tap of a new input session it waits up to 1.5 s for the first change and 2.5 s in all. The reply's spans are milliseconds on the daemon's clock, from the dispatch: `dispatchMs` until the target accepted the input, `frameChangeMs` until the first frame whose hash differs, `treeDeltaMs` until the first tree delta, and `watchedMs` for how long the watch lasted. `frameChangeMs` and `treeDeltaMs` are absent, never 0 or null, when the signal did not come: the tap changed nothing that way, or the target has no frame source (Android devices, and emulators without the controller or with `--emulator-controller off`). `click` makes no stroke: an element partly but not mostly in view, as `scroll --until` judges it, is tapped at the centre of its bounds with a warning; one with none of it in view gets its window read again once, then is tapped where it is or refused with `element_offscreen` (close the keyboard, or `scroll --until` it); a covered element is refused with `element_covered` before any read, so a tap never lands on the view over it (`rn-app-fixes`).

```json
{"success":true,"data":{"accepted":true,"clicked":"@e7","dispatchMs":0.101,"frameChangeMs":0.026,"ref":"@e7","treeDeltaMs":0.112,"watchedMs":5.183,"x":204.0,"y":120.0},"error":null}
```

**Settle and waits.** `wait --settle` resolves once neither the tree nor the frames have changed for the quiet window (`--quiet`, default 200 ms), at most `--timeout` (default 3000 ms). `endedBy` names the signal that changed last: `tree`, `frame`, `both` (within 20 ms of each other), `none` (nothing changed since the last action, or the screen was already quiet), or `timeout`. A timeout is still a success: `settled` is false, `stillChanging` names what kept changing, and `warning` says so. `--tree-only` ignores frames, for video. On an Android target without frames, the settle also reads a few elements' bounds again every 50 ms and does not end while the platform still slides or scales a window in, and after a stroke that leaves a list at its end it holds for 500 ms while the list springs back. `wait <ref or selector>` waits for an element (`--state visible|hidden|detached|enabled`) and `wait --text <text>` for text; both fail with `wait_timeout` and a summary of the screen. `open` waits for the app's first tree, past any launch splash, and for its screen to settle, and replies `settled`, `elapsedMs` and `firstTreeMs`. On the fixture's endless animation:

```json
{"success":true,"data":{"elapsedMs":3000.329,"endedBy":"timeout","frameChanges":175,"settled":false,"stillChanging":["frame"],"treeChanges":0,"waited":"settle"},"error":null,"warning":"the screen did not settle within 3000 ms: frame changes kept arriving"}
```

**Scrolling.** `scroll down 600` moves the content 600 points. A stroke must be longer than a tap, more than 8 points on Android and 10 on iOS, or the command fails with `stroke_too_short`. Strokes keep off the keyboard and the system bars: each goes through what they leave of the screen, or of the element `-s` names, since a keyboard takes a stroke that comes down on its keys for typing (task 10.32: Gboard glide-typed "by" into the focused field at every stroke). When the keyboard leaves too little for a stroke, the scroll fails with `covered_by_keyboard` before it touches the screen. `scroll down 600 --until '#row-150'` scrolls until the element is mostly in view: its centre and at least half of it inside what its list shows, less the bars over it. Each stroke comes to rest before the finger lifts, so the list moves only as far as the finger, and is at most three quarters of what the list shows, so the element cannot pass through the list between two looks; the screen settles after each stroke. The reply's `visible` is the share of the element's bounds in view, from 0 to 1. The scroll fails with `element_not_found` once the content stops moving or no stroke along its axis brings more of the element in, and waits at most `--timeout`.

**Timeouts.** Waiting commands take `--timeout <ms>`; otherwise the session's default applies (`set timeout <ms>`), else `--default-timeout` or `MOBILE_AGENT_DEFAULT_TIMEOUT`, else 10000 ms. The maximum is 600000 ms.

**Batch.** `batch` runs commands in order, given as quoted strings or as a JSON array of argv arrays on stdin, and `--bail` stops at the first failure. With `--json` it prints one array of `{command, success, result, error}` entries, with `code` on failures, and exits 1 when any entry failed. Here the third command never ran:

```
$ mobile-agent --json batch --bail 'get text #settings-wifi' 'click @e99' 'snapshot -i'
[{"command":["get","text","#settings-wifi"],"error":null,"result":{"text":"Wi-Fi"},"success":true},{"code":"unknown_ref","command":["click","@e99"],"error":"@e99 is not a ref from session \"default\"'s snapshots of emulator-5554; take a snapshot and use the refs it prints","result":null,"success":false}]
```

**Sessions and targets.** One daemon serves every session. `--session <name>` (default `default`) keeps target claims, refs, diff baselines and default timeouts apart. A command claims its target for its session, and another session is refused with `target_claimed`, naming the holder in `data.holder`, unless it passes `--force`; `close` releases the claim. With several targets and no `--device` or `--platform`, a command fails with `ambiguous_target` and lists them in `data.candidates`. Android allows one UI automation owner per device, so while the daemon holds one, Maestro, Appium and `uiautomator dump` cannot run against it; attaching while another tool holds the role fails with `automation_conflict` naming the likely holder, and `--force` takes it over.

**iPhones.** A phone on USB is a target once `MOBILE_AGENT_DRIVERS` names `ios` (Quick start). A command on a phone that is not ready fails with its readiness state as the code, and `devices` and `doctor` name the step. The tier is slower: a tree read of a 500-cell screen takes about 0.5 s and frames come 14 to 19 a second, so settle and waits take longer. Text goes in through the phone's runner, typed (`keys`) or set (`direct`), so the pasteboard is left alone; text whose method is left to the daemon is typed into an empty field and set into one that holds text. A runner that stops answering is started again: the call it failed fails with `runner_stalled`, which can be repeated, and a second stall in the session, or a runner that cannot start again, is `disconnected`, after which the next command attaches again (`crates/daemon/README.md`, The physical iPhone driver).

**Error codes.**

| Code | Meaning |
|---|---|
| `stale_ref` | the ref's element is gone, or the mirror was rebuilt since the ref was issued; snapshot again |
| `unknown_ref` | the ref is not from this session's snapshots of the target |
| `element_covered`, `element_offscreen` | a dialog, or on iOS a view drawn over the element, covers it (act on the dialog, or scroll the element out from under the view); none of the element is in view after a fresh read |
| `element_not_found`, `wait_timeout` | no element matched within the timeout; the waited-for condition did not hold |
| `stroke_too_short` | a scroll stroke no longer than the platform's tap threshold |
| `covered_by_keyboard` | the keyboard covers what a scroll strokes; close it (`press Back` on Android), then scroll again |
| `text_mismatch` | `fill` left other text in the field after typing the text and entering it again in one piece; the message and `data.value` name what the field holds |
| `no_target`, `ambiguous_target`, `target_not_found`, `target_claimed` | target selection and session claims |
| `app_not_found`, `input_rejected`, `no_frame`, `disconnected` | no such app; the target refused the input; no frame source or no frame yet; the target is gone |
| `automation_conflict`, `target_not_ready`, `agent_failed`, `adb_failed` | Android: another holder of UI automation; the device is not in the `device` state; the agent did not start; adb failed |
| `unsupported_xcode`, `target_unavailable`, `no_input_transport` | Simulator: Xcode outside the range; a private framework call failed; no input transport works |
| `device_locked`, `developer_mode_off`, `not_trusted`, `disk_image_unmounted`, `no_signing_identity`, `automation_not_enabled`, `unsupported_ios`, `runner_stalled` | iPhone: the phone is not ready, the message naming the step (unlock it, approve Enable UI Automation on it); the runner stalled and is starting again, so repeat the command |
| `daemon_unavailable`, `daemon_timeout`, `invalid_argument` | the CLI could not reach the daemon or had no reply in time; bad usage |

The full lists are in `crates/daemon/README.md` (Errors) and `crates/cli/README.md` (Output).

## Measured

The harness judges each gate at p95, and "under" is strict (`crates/bench/README.md`, Gates).

iOS Simulator (`reports/10.2-simulator.md`, `reports/10.2-simulator/report.md`): an iPhone 17 Simulator on iOS 26.5, Xcode 27.0, commit e84d86e, 50 measured iterations.

| Gate | Limit | Measured | Verdict |
|---|---|---|---|
| CLI process start | under 20 ms | 5.882 ms | pass |
| Cold attach | under 2 s | SwiftUI 946.969 ms, Expo 837.939 ms | pass |
| Warm snapshot, SwiftUI 500-node screen | under 150 ms | 7.880 ms | pass |
| Warm snapshot, React Native (Expo) 500-node screen | under 250 ms | 7.652 ms | pass |
| Tap dispatch | under 20 ms | SwiftUI 1.555 ms, Expo 0.178 ms | pass |

The warm snapshot comes from the mirror's replica; in the driver's suite, reading the SwiftUI screen from the device took 149.9 to 178.4 ms, under a load that the suite's own Simulator boots raised to between 100 and 250.

Android emulator (`reports/10.1-emulator.md`): an API 36 arm64 AVD on Android Emulator 37.1.11 started with `-gpu host`, commit 05a4756, 50 measured iterations. With the controller on (`-grpc 8554 -grpc-use-token`, `reports/10.1-emulator-grpc/report.md`), taps and frames go through the emulator's gRPC controller; with it off (`-ports 5554,5555`, `reports/10.1-emulator-agent/report.md`), taps go through the agent and there is no frame stream.

| Gate | Limit | Controller on | Controller off |
|---|---|---|---|
| CLI process start | under 20 ms | 11.6 ms: pass | 9.1 ms: pass |
| Cold attach | under 1.5 s | Expo 894.2 ms, Compose 1208.2 ms: pass | Expo 927.6 ms, Compose 880.1 ms: pass |
| Warm snapshot, 500-cell screen | under 100 ms | Expo 12.8 ms, Compose 19.4 ms: pass | Expo 13.2 ms, Compose 11.3 ms: pass |
| Tap to frame change | under 67 ms, with host GPU rendering | Expo 52.5 ms, Compose 55.1 ms: pass | not measured: no frame source |
| Tap dispatch | no emulator gate | Expo 2.8 ms, Compose 3.3 ms | Expo 5.1 ms, Compose 6.7 ms |

- Both runs drove the CLI through a shim (`reports/10.1-emulator-grpc/bench-android-cli.rs`) that task 10.10 has since made unnecessary. It adds a process start to every wall-clock sample: without it, `mobile-agent --version` took 5.0 and 5.3 ms at p95 (`reports/10.1-emulator.md`, finding 9).
- The runs' own reports judged tap to frame change against the 50 ms gate of the time, and failed it. Task 10.13 restated the gate for host rendering, which these values pass (`docs/research/emulator-frames.md`). The controller-off report, made before task 10.10, counts that series as failed where the harness now reports it as not measured.

Physical Android device (`reports/10.3-physical-android.md`, `reports/10.3-physical-android/agent/report.md`): a Nexus 5X on Android 8.1 (API 27), commit 552c9e2 with agent 0.4.3, 20 measured iterations after 2 warm-up. A device has no frame stream: taps and screenshots go through the agent. The tap acknowledgement and warm snapshot gates are those the task named, taken from the android-driver spec's 50 ms "Tap acknowledged" bound and the Android warm snapshot gate, through a gate file (`reports/10.3-physical-android/gates.json`).

| Gate | Limit | Measured | Verdict |
|---|---|---|---|
| CLI process start | under 20 ms | 6.3 ms | pass |
| Cold attach | under 1.5 s | Expo 13,889.4 ms (2,193.7 at p50), Compose 3,131.5 ms | fail |
| Warm snapshot, 500-cell screen | under 100 ms | Expo 7.0 ms, Compose 8.7 ms | pass |
| Tap acknowledgement (tap dispatch) | under 50 ms | Expo 16.4 ms, Compose 11.4 ms | pass |
| Tap to frame change | no device gate | not measured: no frame stream | — |

- Cold attach misses its gate on this phone: `app_process` alone takes 1.1 to 1.4 s to start there, and two of Expo's 20 opens did not settle within 10 s.
- Agent 0.4.2 and earlier could not run on Android 8.0 to 9: they called three platform members those levels lack (task 10.3, agent 0.4.3).
- On Android 8.1 the Compose fixture's launch focuses its first field and raises the keyboard over the tab bar, so the Compose list-scroll and big-screen flows failed their golden checks. Expo's list-scroll failed its check at the list's end.

Physical iPhone (`reports/11.1-ios-device.md`, The gates on the fixed run; `reports/11.1-ios-device/a6-swiftui-primitives-fixed/report.md`, `reports/11.1-ios-device/e2-expo-primitives-fixed/report.md`): an iPhone SE (3rd generation) on iOS 26.6.2, Xcode 27.0, the release build at commit 9a13a43 with the fixes of group 9 of `physical-ios` (task 9.6), the runner 0.2.0 already on the phone, 50 measured iterations after 3 warm-up. Input goes from the host through the disk image's dtuhidd, and frames are screenshots polled on DVT and CoreDevice. The gates are the `ios-device` class's defaults (`crates/bench/README.md`, Gates).

| Gate | Limit | Measured | Verdict |
|---|---|---|---|
| CLI process start | under 20 ms | 7.0 ms in SwiftUI's phase, 7.6 ms in Expo's | pass |
| Cold attach, the runner installed | under 5 s | SwiftUI 4,052.0 ms (2,135.7 at p50), Expo 2,301.3 ms | pass |
| Warm snapshot, 500-cell screen | under 1 s | SwiftUI 8.6 ms, Expo 9.4 ms | pass |
| Tap dispatch, through dtuhidd | under 150 ms | SwiftUI 55.1 ms, Expo 54.6 ms | pass |
| Tap to frame change | no gate while the frames are polled screenshots | SwiftUI 364.0 ms, Expo 179.7 ms | reported |

- The first run, on 2026-10-10 at commit 97ea3f5 before group 9's fixes, passed the same four gates (`reports/11.1-ios-device.md`, Gates).
- Cold attach starts a daemon, lists the phone, opens the tunnel and the runner's session and brings the fixture up. SwiftUI's p95 is four attaches of 3.1 to 4.3 s among 49 of 2.0 to 2.4 s, in a series that raised the host's load to 27.2 itself; the phase it repeats, `a5`, failed the gate at 9,205.7 ms, four of its attaches taking 5.4 to 29.5 s while the host held process launches.
- The warm snapshot is answered from the daemon's replica, as on the Simulator: all 200 of the fixed run were, the handler taking about 0.5 ms. Its 1 s limit was set from the runner's read of the same 500-cell screens, 450 to 650 ms, which no warm snapshot waits for, and the gate table shows the reads beside the replica's answers (`reports/11.1-ios-device.md`, Snapshot reads on physical iPhones). The first snapshot after a relaunch waits for a read the daemon begins as `open` returns: 766.1 and 457.8 ms at p50 on SwiftUI and Expo.
- Tap dispatch counts the 50 ms the tap holds the finger down. Tap to frame change runs from the dispatch to the arrival of the first screenshot whose pixels below the status bar moved, at 14 to 19 captures a second: 272.7 and 154.0 ms at p50 on SwiftUI and Expo, against 72.8 and 47.7 ms on the Simulator, whose frames stream at 30 a second.

Against the baselines: how many times faster mobile-agent's p50 was on each reference flow. "Not timed" means the tool failed the flow's golden check, so the harness did not time it. mobile-agent's flows had no failed or flaky iteration in the runs below; a baseline's failure and flake rates follow its factor where they are not zero.

The login rows were measured again on 2026-10-07, after `fill` began reading the field back (task 10.23), together with the text-entry series (`reports/10.26-remeasure.md`); the other rows are the published runs'. On the emulator, mobile-agent's two series ran once more that day, after the read-back's 200 ms hold was kept for typed text alone (task 10.30), and its login factors there are taken against the baselines of the earlier run (`reports/10.26-remeasure.md`, 10.30 rerun). On the Simulator mobile-agent's two series ran once more, after the mirror began reading a field's window again as soon as a pasted text's frame changes it (task 10.29), and its text entry and SwiftUI login once more, after a paste began putting its text on the pasteboard without `devicectl` (task 10.31); the login's fills are typed and read back as before, so its Simulator rows stand (`reports/10.26-remeasure.md`, 10.29 and 10.31 reruns). A text entry of 36 characters took 199.2 and 387.0 ms at p50 on the Simulator (SwiftUI, Expo), which pastes it, against 376.3 and 555.4 ms before task 10.31, 545.5 and 718.3 ms before task 10.29 and 249.3 and 228.3 ms before the read-back, and 43.0 and 234.9 ms on the emulator (Expo, Compose), which sets it directly, against 23.8 and 26.1 ms before the read-back and 224.1 and 240.0 ms with the hold on every fill. At the median the read-back is 21.2 ms of it on the emulator's Expo fixture, whose fields report a change at once, 218.1 ms on Compose, whose window is read again after the action, and 92.4 to 100.4 ms on the Simulator. The Simulator's entries fall in two clusters about 250 ms apart, the slower waiting for a re-read after the emptied field's caret blinked, and its p50 lands in either.

iOS Simulator (`reports/10.2-simulator.md`, Baselines; login, `reports/10.26-remeasure.md`):

| Flow | Maestro 2.10.0, SwiftUI | Maestro, Expo | agent-device 0.21.14, SwiftUI | agent-device, Expo |
|---|---|---|---|---|
| login | 6.10x | 6.45x | 2.11x | 2.14x (90 % failed, 2 % flaky) |
| list-scroll | 6.06x | 6.05x (26 % failed, 22 % flaky) | not timed | not timed |
| big-screen | 8.51x | 7.13x | 1.24x | not timed |

Android emulator, controller on (`reports/10.1-emulator.md`, Baselines; login, `reports/10.26-remeasure.md`, 10.30 rerun):

| Flow | Maestro 2.10.0, Expo | Maestro, Compose | agent-device 0.21.14, Expo | agent-device, Compose |
|---|---|---|---|---|
| login | 14.76x | 6.75x | 2.89x (24 % failed, 12 % flaky) | 1.70x |
| list-scroll | not timed | not timed | not timed | not timed |
| big-screen | 8.39x | 4.32x | 1.19x (76 % failed, 12 % flaky) | 1.06x |

Physical Android device, the login flow alone (`reports/10.3-physical-android.md`, Login against the baselines; 20 measured iterations):

| Flow | Maestro 2.10.0, Expo | Maestro, Compose | agent-device 0.21.14, Expo | agent-device, Compose |
|---|---|---|---|---|
| login | 23.10x | 7.58x | not timed: 100 % failed | 3.77x |

Physical iPhone, the fixed run: each flow through mobile-agent, agent-device and WebDriverAgent in one phase, with the harness's hand-offs (`reports/11.1-ios-device.md`, The flows on the fixed run; 50 measured iterations). A factor under 1 means the baseline was faster.

| Flow | WebDriverAgent 16.14.1, SwiftUI | WebDriverAgent, Expo | agent-device 0.21.14, SwiftUI | agent-device, Expo |
|---|---|---|---|---|
| login | 0.94x | 0.97x | 1.80x | 1.85x (2 % flaky) |
| list-scroll | 0.36x | 0.37x (6 % failed, 8 % flaky) | not timed | not timed |
| big-screen | 0.76x | 1.12x | 0.79x (4 % flaky) | 1.14x (2 % failed, 8 % flaky) |

- At p50 mobile-agent took 6.3 and 5.8 s on login, 30.6 and 28.4 s on list-scroll and 4.6 and 3.4 s on big-screen, on SwiftUI and Expo; WebDriverAgent 5.9 and 5.7 s, 11.1 and 10.4 s and 3.5 and 3.8 s; agent-device 11.3 and 10.8 s on login and 3.6 and 3.9 s on big-screen; and mobile-agent on the Simulator 4.1 and 3.9 s, 16.3 and 15.0 s and 2.5 and 2.8 s.
- What the comparison measures: mobile-agent's flows settle and verify after every step, as an agent does. A click waits for its first changed screenshot, then for a read of 450 to 650 ms, and `scroll --until` makes 22 to 25 resting strokes on the SE's 667-point screen, each followed by a settle and a read. WebDriverAgent's flows fire their requests and check at the end: its list-scroll flings the list in 9 swipes and polls for `row-200` (`reports/11.1-ios-device.md`, Flows).
- agent-device's list-scroll failed its golden check on both fixtures, its `scroll --until` spending its fixed 12-pass budget before `row-200` came into view (`reports/11.1-ios-device.md`, finding 9). The harness skips Maestro on iPhones with the note that 2.11.0 refuses them, its device driver unfinished (the host has 2.10.0, whose unfinished driver lacks that guard), and maestro-runner was not installed. In the first run mobile-agent's runner connection was lost beside the baselines in every phase but Expo's big-screen (`reports/11.1-ios-device.md`, finding 5), so the comparison is the fixed run's.

What each tool costs the host and the device, memory and CPU over ten login flows and a minute idle, is in `reports/10.20-resources.md`: per flow, mobile-agent's processes used 0.35 CPU-s on the Simulator against agent-device's 2.75 and Maestro's 18.82, and on the emulator 0.28 on the host and 0.05 in the guest against 0.62 and 0.22, and 13.45 and 1.82. With an iPhone attached and idle, the daemon took 10.4% of a core and usbmuxd 7.6% in the first run, its screenshots and its 1 s poll together, and 4.7% and 7.7% in the fixed run, with the poll of a still screen at 10 s and DVT's screenshot failing on the phone; the phone's own share was not measured (`reports/11.1-ios-device.md`, The idle poll).

The list-scroll flow reaches `row-200` with one `scroll --until`, as an agent would, since task 10.17; before, the harness ran its own loop of settles, checks and plain strokes. p50 of the flow before and after (`reports/10.17-list-scroll.md`, with the Simulator's after column from the 10.2 run), the emulator with its controller on:

| Target | Fixture | Before | After |
|---|---|---|---|
| iOS Simulator | SwiftUI | 25.3 s, 16 % flaky | 16.3 s, no flake |
| iOS Simulator | Expo | 20.6 s | 15.0 s |
| Android emulator | Expo | 14.1 s | 12.6 s |
| Android emulator | Compose | 15.2 s | 13.7 s |

Host costs, from the host microbenchmarks (`reports/micro/baseline.json`; `reports/micro/README.md` says how to run and read them): what each host-side path costs without a device, measured by `just micro` on 2026-10-08 at commit b724af0 on the same Apple M1 Max under macOS 26.6.2, with rustc 1.98.1 and criterion 0.8.2, from a 1-minute load of 2.46 that the quiet-host guard passed. Each value is the median of criterion's 100 samples, in microseconds; allocations are per iteration, one pass over the input, where the benches of `protocol`, `mirror` and `daemon` count them. The mirror's paths run on its scripted fake target, whose read hands over a copy of the screen where a driver would decode one, and the socket's on a daemon serving a fake emulator. The 1,049-node screen is the Expo fixture's Big tab as spike S1 read it on the emulator, reconstructed, since the repository holds no capture.

| Path | Benchmark | Median (µs) | Allocations |
|---|---|---:|---:|
| A 500-node snapshot in S5's shape, 21,340 bytes, decoded | `protocol/decode/snapshot-500` | 123.9 | 765 |
| The same snapshot encoded into a reused buffer | `protocol/encode/snapshot-500` | 55.6 | 0 |
| A 1,049-node snapshot of the React Native 500-cell screen, decoded | `protocol/decode/snapshot-1049` | 240.1 | 2,104 |
| The same snapshot encoded | `protocol/encode/snapshot-1049` | 110.4 | 0 |
| 10,000 events decoded | `protocol/decode/events-10k` | 830.9 | 8,975 |
| 10,000 events encoded | `protocol/encode/events-10k` | 716.6 | 0 |
| The frame decoder splitting those events' frames out of their stream, fed 64 KiB at a time | `protocol/frame-decoder/events-10k` | 42.6 | 2 |
| The SwiftUI 500-cell screen, 510 nodes, read into the replica the first time | `mirror/pump/first-read-grid` | 97.4 | 2,092 |
| The same read again, matched unchanged | `mirror/pump/unchanged-grid` | 208.3 | 3,114 |
| The React Native 500-cell screen, 1,049 nodes, read the first time | `mirror/pump/first-read-grid-rn` | 192.6 | 4,180 |
| The same read again, matched unchanged | `mirror/pump/unchanged-grid-rn` | 412.8 | 7,334 |
| The SwiftUI screen's interactive listing, 505 refs, right after a read | `mirror/snapshot/interactive-grid` | 327.3 | 8,155 |
| The React Native screen's interactive listing, 505 refs | `mirror/snapshot/interactive-grid-rn` | 404.4 | 9,689 |
| Visibility of a row in the Expo list | `mirror/visibility/row-list` | 7.1 | 183 |
| A delta snapshot after one row changed | `mirror/delta_snapshot/one-change-list` | 56.2 | 1,243 |
| A diff after one row changed | `mirror/diff/one-change-list` | 43.8 | 893 |
| A settle with frames: one change taken in, the quiet window waited out on the fake clock | `mirror/settle/frames-grid` | 0.126 | 0 |
| The same without frames on the React Native screen, whose windows send change events | `mirror/settle/no-frames-grid-rn` | 0.152 | 0 |
| The same without frames on the SwiftUI screen, whose window sends none and is read again once | `mirror/settle/no-frames-grid` | 209.9 | 3,116 |
| `snapshot -i` through the daemon's socket to a fake emulator on the 500-cell Big tab, 505 refs, one connection per request | `daemon/socket/snapshot-interactive-big` | 798.2 | — |
| Its reply line written, 45 KB | `daemon/wire/reply-snapshot-big` | 98.4 | 21 |
| Its request line parsed | `daemon/wire/parse-snapshot-request` | 0.370 | 8 |
| A 1206x2622 BGRA frame downscaled to 460x1000 | `simulator/scale_bgra/1206x2622-to-460x1000` | 1,092.9 | — |
| The 460x1000 frame hashed whole, the frame thread's test for a changed frame | `simulator/pixels_hash/460x1000` | 100.7 | — |
| Its 32-pixel thumbnail hashed | `simulator/thumbnail_hash/460x1000` | 114.7 | — |
| Its thumbnail below the status bar hashed, the settle's frame signal | `simulator/content_hash/460x1000` | 106.8 | — |
| Its JPEG at quality 80 | `simulator/encode/460x1000-q80` | 930.8 | — |
| Downscale, hash, JPEG and content hash in sequence | `simulator/pipeline/frame-to-jpeg` | 2,241.5 | — |
| The emulator's frame key of a 540x1200 RGB888 frame | `android/frame-hasher/key-540x1200` | 52.7 | — |

- The socket's span runs from writing the request out to the reply parsed, the host's whole path less the CLI process; most of its work is in the daemon's process, whose allocations the bench does not count. The baseline's other 39 benchmarks cover the remaining inputs and cases: the golden sample and the 1,000 requests, `to_vec` and `write_frame`, the list and Bluesky fixtures, full listings and listings taken again, reads with one change, with a window added or removed and after a resync, `scroll_viewport`, `uncovered`, waits, and the React Native screen's settle with frames.
- The daemon's Simulator frame thread runs that sequence without the JPEG: it downscales and hashes each frame it reads, at most 30 a second, and encodes a JPEG only for a screenshot (`crates/daemon/src/devices/simulator.rs`). Spike S2's pipeline encoded every frame; at 60 frames a second the sequence with the JPEG would take 13.4% of a core, where S2 measured 20.9% for its whole pipeline (`docs/spikes/s2-simulator-frames.md`, Verdict).
- Beside spike S5, which encoded its 500-node tree in 26.9 µs and decoded it in 59.1 µs on its own model types, built with fat LTO and one codegen unit and with each decoded value dropped outside the timed region (`docs/spikes/s5-wire-format.md`, Rust on the host): the suite's 500-node snapshot has S5's shape on the shipped crate's types, 21,340 bytes against the 21,410 of S5's own tree on those types (`docs/spikes/s8-bench-harness.md`, Input), and its decode drops the message inside the timed region, under the workspace's release profile. Built with S5's profile, on a quiet host at a 1-minute load of 2.45, the same benchmarks gave 25.4 µs encode, 25.6 µs `write_frame`, 27.4 µs `to_vec` and 119.6 µs decode, against 54.4 µs encode and 123.8 µs decode in the first baseline, taken the same day (task 3.2 of the `microbenchmarks` change): the encode's gap to S5 is the build profile, and the decode's, twice S5's under the same profile, the shipped node decoder.
- The baseline was measured again on 2026-10-11 at commit 8a46e41 (task 9.6 of `physical-ios`), from a 1-minute load of 3.11 after 90 s of the guard's wait: every benchmark came within the threshold of the values above, from −4.6% (`mirror/snapshot/full-list`) to +2.7% (`daemon/wire/reply-snapshot-big`), the median 0.0%, with no allocation count changed. `reports/micro/baseline.json` holds that run's values; the table keeps the first run's.

Spike measurements. The throwaway programs under `spikes/` measured these before the crates existed, on one host (Apple M1 Max, macOS 26.6.2, Xcode 27.0, CoreSimulator 1171.7, iOS 26.5, an API 36 arm64 emulator); the spike verdicts table in `openspec/changes/archive/2026-10-07-mobile-automation-foundation/design.md` summarizes them. They are not harness results.

| Spike | Measurement | Source |
|---|---|---|
| S1 Android ownership | tap acknowledged in 1.9 ms p50; cold read of the Compose fixture's 500-cell screen 349 ms over 465 round trips, warm 3 ms | `docs/spikes/s1-android-ownership.md` |
| S2 Simulator frames | 60 fps at 21% of one core at 460x1000 | `docs/spikes/s2-simulator-frames.md` |
| S3 Simulator input | dispatch 0.03 ms p50, 0.16 ms p95; first changed frame 36 ms after the tap | `docs/spikes/s3-simulator-hid.md` |
| S4 Simulator tree | reader ready 292 ms after its spawn; 546 nodes in 126 ms, against 0.7 s through XCTest | `docs/spikes/s4-simulator-ax.md` |
| S5 wire format | CBOR 2.49x smaller than JSON on the tree; device-to-host path 9.5x faster | `docs/spikes/s5-wire-format.md` |
| S6 ADB transport | shell round trip 5.7 ms p50, against 24 ms through the `adb` binary | `docs/spikes/s6-adb-transport.md` |
| S7 emulator controller | a tap through the controller in 0.96 ms p50; 540 px frame stream at 45 fps with 15 ms lag | `docs/spikes/s7-emulator-grpc.md` |

## How measurements are made

`crates/bench` (`mobile-agent-bench`, run with `just bench`) drives the release CLI as an agent does, one `--json` process per command, on the reference fixtures of `fixtures/CONTRACT.md` (SwiftUI and Expo on the Simulator, Expo and Compose on Android), with 3 warm-up and 50 measured iterations per series and nearest-rank p50, p95, maximum, failure and flake rates. The same login, list-scroll and 500-node flows run through Maestro 2.10.0 (on Java 21.0.12) and agent-device 0.21.14 (on Node 25.8.0) as baselines, as the runs' setups record (`reports/10.1-emulator.md`, `reports/10.2-simulator.md`), and through maestro-runner when it is installed, each timed by wall clock around the tool's process. Each run writes `report.json` and `report.md`, committed under `reports/`, and exits non-zero when a gate fails.

On a physical iPhone the harness drives the same CLI with `MOBILE_AGENT_DRIVERS=ios` (`crates/bench/README.md`, Physical iPhones). The phone ran twice, on 2026-10-10 (tasks 7.1 to 7.3 of `physical-ios`) and, after group 9's fixes, on 2026-10-11 (task 9.6), whose numbers the tables above give: on USB and external power, unlocked, Auto-Lock at Never, which no tool reads, so the run states it (`--auto-lock-off`), and the runner already installed. Before each phase the harness installs the fixtures' device builds, which their own scripts sign for the team (`DESTINATION=device fixtures/<fixture>/build.sh`), through CoreDevice's `devicectl` rather than through the tool under test, and it records the phone's model, iOS version, battery, runner and input and frame paths as the daemon reports them. The baselines on the phone are WebDriverAgent 16.14.1, built from the copy in Appium's XCUITest driver 12.16.0, re-signed for the team, started through the phone's test manager by the harness's own helper and attached to Appium 3.8.0, and agent-device 0.21.14, which builds and runs its runner with `xcodebuild` and so needs the Mac's DevToolsSecurity on (`sudo DevToolsSecurity -enable`, the maintainer's step, for that baseline alone); Maestro is skipped, since 2.11.0 refuses physical iPhones (`crates/bench/README.md`, Baselines). A phone has one automation session, so each tool's release ends its runner app there and the hand-off checks the phone's processes. The host's endpoint security inspects process starts and at times held them for seconds to minutes, and every CLI command is a process of its own, so each phase first waited for a quiet host, in the fixed run a 1-minute load under 4, no build above 5% of a core and a launch of `/usr/bin/true` within 1 s in two checks 60 s apart; the report names the phases run again and the series that still carry load or a launch hold (`reports/11.1-ios-device.md`, Method, findings 4 and 10).

`just micro` (`mobile-agent-bench micro`, `crates/bench/README.md`, Microbenchmarks) times the host-side paths without a device, through the suites in the crates' `benches/` under criterion 0.8.2 at its default settings: 3 s of warm-up, then 100 samples over about 5 s per benchmark, of which it keeps the median, the median's 95% interval and the sample count, beside the allocations and bytes per iteration that the benches of `protocol`, `mirror` and `daemon` count. It measures only on a quiet host, a 1-minute load under 4 and no build, emulator or Simulator process above 5% of a core, and otherwise exits 2 or, with `--wait`, waits for one. It compares the run with the committed baseline, `reports/micro/baseline.json`, and exits 1 on a regression: a median more than 5% slower than the baseline's and above the 95% interval the baseline recorded around its own, or any rise in allocations or bytes, which compare exactly. The threshold is twice criterion's run-to-run coefficient of variation on a quiet host, with a floor of 5%; spike S8 measured that coefficient at 1.15%, so the floor holds (`docs/spikes/s8-bench-harness.md`). A baseline is refreshed only by a run that passed the guard, `--save-baseline`, and committed with the change that moved it. CI compiles the suites, since `just lint` covers all targets, and runs none of them: hosted runners give no stable timings.

## Status and limitations

- One physical Android device has run: a Nexus 5X on Android 8.1 (task 10.3, `reports/10.3-physical-android.md`). Cold attach missed its 1.5 s gate there. With the keyboard up, a `scroll` stroke that starts over the keyboard reaches it, and Gboard typed the strokes into the focused field. On Compose's 500-cell screen commands waited about 10 s, since Android 8.1's node cache is one for the whole process and every eviction empties it. Devices have no frame stream, so clicks there report no `frameChangeMs` and settle rests on the tree. OEM Android builds and the levels between 8.1 and 16 are unverified.
- One physical iPhone has run: an iPhone SE (3rd generation) on iOS 26.6.2 under Xcode 27.0 (`reports/11.1-ios-device.md`). Every path the driver takes is gated on the phone's iOS version; iOS 27, whose dtuhidd takes the host's input only while a display stream runs, the stream that would also give frames, is a later path, and no other iOS version has run.
- An iPhone has no frame stream on iOS 26: the CoreDevice display stream needs iOS 27 (CoreDevice error 9021 on 26.6.2), and AVFoundation's capture of the phone waits on the macOS camera permission for the process that starts the daemon, so it has not been measured. Frames are screenshots polled on DVT and CoreDevice, 14 to 19 a second, so tap to frame change is reported rather than gated, settle's quiet window lasts at least two capture intervals, and the re-read after a tap waits up to 500 ms for the first changed frame (`openspec/changes/physical-ios/design.md`, D23).
- What an iPhone's tree costs and lacks: a read of a 500-cell screen takes 450 to 650 ms; an idle phone's still screen is read again every 10 s, which keeps the runner reading about 5% of the time; and the first snapshot after a relaunch waits for a read the daemon begins as `open` returns, about 670 ms on SwiftUI and 450 ms on Expo (`reports/11.1-ios-device.md`, Read costs and The idle poll). A view the app marks modal, other than an alert, is not marked, since testmanagerd's reply carries no children count, and XCTest leaves out a row a floating tab bar covers, as SwiftUI's `row-12` under the SE's bar, which `scroll --until` brings out. Text goes in through the runner, typed at 60 characters a second or set, never pasted.
- The phone's developer services failed twice in the day of the fixed run (`reports/11.1-ios-device.md`, finding 11): DVT's screenshot stopped answering and testmanagerd stalled the runner on the 43rd emptying text set of every session, until a reboot; later DVT refused its process-control channel at attach, and answered again by itself about 1 h 30 min after. A runner that stops answering is ended and started again once, and a screenshot poller whose captures time out is reopened, then left out and named in `status`, the screenshot coming through CoreDevice; but a phone whose DVT refuses that channel cannot be driven, since `open` launches through it, and CoreDevice's app service as the launch's fallback is an open question (`openspec/changes/physical-ios/design.md`, Open Questions).
- An iPhone needs a person at times: the passcode for "Enable UI Automation" at a runner's start (Quick start), and an unlock, since a locked phone fails with `device_locked`. One automation session exists per phone, so mobile-agent's runner and another tool's take turns.
- Only Xcode 27.0, iOS 26.5, the API 36 emulator, one API 27 device and one iPhone on iOS 26.6.2 have run.
- The emulator's tap-to-frame gate is p95 under 67 ms, four frames of the AVD's 60 Hz display, and holds for host GPU rendering (`-gpu host`) with the controller on: the guest's own frame pipeline takes about three frames at p95, and the tool's dispatch about 3 ms (`docs/research/emulator-frames.md`). The 10.1 run measured 52.5 ms on the Expo fixture and 55.1 ms on the Compose one. Software rendering is not gated; spike S7 measured 146.5 ms at p95 there.
- On a Simulator whose first boot did not come from the driver, the first tree reads can fail for several seconds, until the reader has turned automation mode on (spike S4); retry.
- React Native: `reports/10.4-rn-smoke.md` compared 17 logged-out React Native screens on the Simulator (Bluesky, Ignite's demo app, the Expo fixture), where no XCTest read reported a sparse root, then Bluesky's signed-in feeds, threads, profiles, notifications, search, chat and settings on the Simulator and the emulator. Signed in, agent-device's XCTest tree came back `sparse` or timed out on the search screen with the keyboard up and on a 711-node thread, and mobile-agent's snapshots of those screens were whole and not degraded. The same run found that with the keyboard up on the Simulator every element counts as 0% in view, so `click` warns wrongly and `scroll --until`, which stops by the same measure, cannot find its element in view (task 10.21).
- The drivers use private Apple Simulator frameworks, the developer services of Apple's disk image and private XCTest calls on an iPhone, and hidden Android APIs. This is development tooling; nothing of it ships inside an app.
- Not built: an MCP server, a CDP-subset endpoint and Android video streaming.
- The OpenSpec change in `openspec/changes/archive/2026-10-07-mobile-automation-foundation/` (proposal, design, specs, tasks) is the source of truth for the design, its decisions and the remaining tasks; the `microbenchmarks` change, archived in `openspec/changes/archive/2026-10-08-microbenchmarks/`, adds the host microbenchmarks, design D13 to D17, and the `physical-ios` change under `openspec/changes/` the physical iPhone, design D18 to D26.
- `docs/overview.md` answers, with sources, how the design differs from Maestro, agent-device and Appium, where it is and is not faster, what it leaves out, which platforms and versions it supports, which protocol it speaks and what an MCP edge would take.
