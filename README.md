# mobile-agent

mobile-agent drives Android devices, Android emulators and iOS Simulators for AI agents, and is designed around the latency of each tap-and-observe step. A daemon keeps a live mirror of every attached screen (its windows, accessibility tree and latest frame), updates it from the platform's change events and frame callbacks, and answers snapshots, waits and settle checks from that mirror instead of reading the device and sleeping on every request. On Android, a single-dex agent pushed over adb runs as the shell user under `app_process` and holds the device's UI automation role: no APK is installed and no instrumentation runs. On the iOS Simulator the driver works from the host: input goes through the Simulator's dtuhidd HID service, frames come from the display's IOSurface, and the accessibility tree comes from a Rust reader that the daemon spawns into the Simulator, with no XCTest runner and nothing installed. The `mobile-agent` CLI follows agent-browser's command grammar, `@eN` refs and JSON envelope, so prompts and skills written for agent-browser carry over where devices have the same concepts.

## Supported platforms

| Platform | Status | Tested on |
|---|---|---|
| macOS host | Simulator and Android drivers | macOS 26.6.2, Apple M1 Max |
| Linux host | Android driver only; the Simulator driver builds as a stub | CI builds and tests it on `ubuntu-latest`, without a device |
| Xcode | 26.0 up to, not including, 28; other versions are refused with `unsupported_xcode` | Xcode 27.0 (27A266a); CI also builds with Xcode 26 |
| iOS Simulator | booted Simulators; no gate on the runtime version | iOS 26.5 (23F77), iPhone 17 |
| Android emulator | fast path through the emulator's gRPC controller when it is enabled | API 36, arm64, Pixel 8 profile, Android Emulator 37.1.11 |
| Android agent | minimum API 26: the dex is built with `--min-api 26` | API 36 only |
| Physical Android device | expected to work through the same agent, without a frame stream; not yet measured (task 10.3 waits for a device) | none |
| Physical iOS device | not supported: only an XCTest bundle can drive one | none |

CI (`.github/workflows/ci.yml`) runs `just validate` in three jobs: Linux, which also builds the Android agent and the Compose fixture; macOS with Xcode 26 on GitHub's `macos-26` image; and macOS with Xcode 27 on its `xcode-27` image. Each macOS job keeps the image's default Xcode when it has that major version and otherwise selects the newest `Xcode_<major>*.app`. The Simulator integration tests, the spikes and every measurement ran on Xcode 27.0 with CoreSimulator 1171.7 and iOS 26.5. Nothing has run on Xcode 26, where input would be touch only, through the older Indigo transport (`crates/simulator/README.md`, Compatibility).

## Build

Requirements:

- **Rust.** Install rustup. It takes the toolchain from `rust-toolchain.toml`: the version pinned there, with clippy, rustfmt and the `aarch64-apple-ios-sim` target. Run `rustup toolchain install` in the checkout, as CI does, to install them before the first build, or when cargo reports that the pinned toolchain is not installed; `rustup show active-toolchain` there names the toolchain in use.
- **macOS, for the Simulator.** Xcode 26 or 27, selected with `xcode-select`, with an iOS Simulator runtime; and libjpeg-turbo, `brew install jpeg-turbo`, which the frame encoder links statically from Homebrew's Apple silicon prefix (`.cargo/config.toml`; set `TURBOJPEG_LIB_DIR` and `TURBOJPEG_INCLUDE_DIR` for another prefix).
- **The Android agent.** The daemon's build compiles the agent with `agent-android/build.sh` (kotlinc and R8, no Gradle) and embeds the jar, so it needs a JDK 17 or newer with `java` and `jar` on `PATH` (tested with Temurin 21), the Kotlin compiler, `kotlinc` on `PATH` or `KOTLIN_HOME` set to the compiler's home, the directory holding `bin/kotlinc` and `lib/` (tested with 2.4.20), and an Android SDK at `ANDROID_HOME`, by default `$HOME/Library/Android/sdk`, with `platforms;android-36` and a `build-tools` package (CI installs 37.0.0). On Linux, set `ANDROID_HOME`.
- **Android at run time.** The SDK's `platform-tools`, whose adb server the driver talks to on port 5037, and for an emulator, the `emulator` package and an AVD.

From the checkout:

```sh
cargo build --release                      # target/release/mobile-agent and mobile-agent-daemon
export PATH="$PWD/target/release:$PATH"    # the CLI starts the daemon it finds beside itself
```

- `SKIP_AGENT_BUILD=1 cargo build --release` builds without the Android toolchain. That daemon deploys an agent only from a jar that `MOBILE_AGENT_JAR` names at run time; `MOBILE_AGENT_JAR` set at build time embeds that jar instead of building one.
- The Simulator's tree reader is cross-compiled for `aarch64-apple-ios-sim` during the build and embedded in the daemon, behind the `reader` feature of `crates/simulator`, which is on by default. The daemon spawns it into each booted Simulator it attaches. Without that Rust target the build warns and embeds no reader, and tree reads fall back to the slower host translator, marked degraded; `doctor` shows which one a build has.
- Contributors: every dependency install goes through Socket Firewall. Install it with `npm i -g sfw`, then prefix installs with it: `sfw cargo fetch`, `sfw cargo add`, `sfw cargo install just`, `sfw npm install -g @fission-ai/openspec`. `just validate` runs the checks a change must pass: build, clippy with warnings denied, format check, `check-paths`, tests, the agent build, the Kotlin codec cross-check, the fixture builds and `openspec validate`. The tests that need a Simulator or an emulator skip unless their environment names one (`crates/daemon/README.md`, `crates/simulator/README.md`).

## Quick start

The daemon attaches to targets that are already running; it boots nothing. For Android, put the SDK's tools on `PATH`, start an adb server, and start an emulator with its gRPC control endpoint, whose port is the console port plus 3000, or connect a device with USB debugging on and its prompt accepted:

```sh
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"
adb start-server
emulator -avd <name> -grpc 8554 -grpc-use-token      # in a terminal of its own
```

Without the control endpoint, taps and screenshots go through the agent and there is no frame stream; `doctor` and `status` say how to enable it on a running emulator: `adb -s emulator-5554 emu grpc 8554`. For the iOS Simulator, boot one in Simulator.app or with `xcrun simctl boot <name or UDID>`.

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
mobile-agent shutdown                       # stop the daemon, which stops the Android agent
```

With several targets connected, give `open` a `--platform android|ios`, or a `--device <id>` when targets share a platform; the session then keeps using the target it holds. The first command starts the daemon, which exits after an hour without requests (`--idle-timeout`). `mobile-agent status` reports the daemon, its targets and, for each attached target, the paths its input, frames and tree take.

A run on an iOS 26.5 Simulator, trimmed where `…` stands. In `doctor`, ⚠ marks a warning, which does not fail it; `sfw` and `just` matter only to contributors. Ref numbers are never reused, and `open` lists the screen while it waits for the app, so the first listing need not start at e1. The degraded verdict is the quality check at work: Settings' search field holds two identical Dictate buttons that only their position tells apart.

```
$ mobile-agent doctor
✓ daemon               running, pid 11976, version 0.1.0+640310.18d934c42feb802c
✓ drivers              simulator, android
✓ xcode                Xcode 27.0 (27A266a) at /Applications/Xcode.app/Contents/Developer
✓ simulator-runtimes   iOS 26.5
✓ android-sdk          /Users/<user>/Library/Android/sdk, adb 1.0.41 (37.0.1-15733141)
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

## Agent guide

`mobile-agent help agent` prints a compact form of this section for an agent's prompt (`crates/cli/src/agent-guide.txt`), and every command's `--help` ends with examples and states its defaults. The JSON examples below come from a daemon serving fake targets (`crates/daemon/README.md`, The fake driver): a launcher, a Settings app with one Wi-Fi switch and the fixture app, whose timings are not a device's.

```sh
mobile-agent snapshot -i       # 1. list actionable elements with their refs
mobile-agent click @e3         # 2. act on a ref from that snapshot
mobile-agent wait --settle     # 3. let the screen settle
mobile-agent snapshot -i       # 4. snapshot again before the next action
```

**Output.** `--json` (or `MOBILE_AGENT_JSON=1`) prints one line per command in agent-browser's envelope: `success`, `data`, `error`, then `code` and `warning` only when set. Every failure exits with status 1 and carries a snake_case `code`; in JSON mode its message travels in the envelope on stdout, without `--json` it goes to stderr. A `warning` marks a success not to take at face value: a degraded snapshot, an `open` whose screen did not settle, a settle that timed out.

```json
{"success":false,"data":null,"error":"e4 is stale: the button \"Fixture\" it named is no longer on the screen. Take a new snapshot and use its refs","code":"stale_ref"}
```

**Snapshots.** A line reads `- role "name" [attributes, ref=eN]: value`. `snapshot -i` lists only elements with refs: actionable elements and named headings, cells and list items. `snapshot` lists every element, texts and system bars included, and `-d N` limits the depth. A line in parentheses, such as `(12 more actionable items below, scroll down to reveal)`, or `(more items below, scroll down to reveal)` when the scroll range gives no count, stands for content scrolled out of view. In `data`, `snapshot` holds the listing, `refs` maps `eN` to `{role, name}`, `removedRefs` names the refs the previous snapshot listed that are gone, `origin` is the app in front, and `verdict` gives the `backend`, whether the read is `degraded` or came from a `fallback`, and the `reasons` with advice (`fallback_backend`, `truncated`, `sparse_tree`, `sibling_collision`, `agent_warning`, `disconnected`); a degraded verdict also sets `warning`. `readMs` is how long the daemon took to answer, device reads included, and `servedFromReplica` is true when the mirror answered without reading the device. Windows that send no change events, every Simulator window and Jetpack Compose windows on Android, are re-read after each injected action, when frames change and on a 1 s poll, so a snapshot right after an action can wait for that read.

**Delta snapshots.** `snapshot --delta` answers with agent-browser's delta object, relative to this session's last delta snapshot. Its `kind` is `full` (the whole listing, and always the first time), `unchanged`, or `delta`, which carries ref `changes` (`add`, `remove`, or `replace` of the role or name) and a `treeChange` splice (`startLine`, `deleteCount`, `lines`) that rebuilds the new listing from the previous one. The whole listing comes back when the delta would be 70% of its size or more, or when the options, the replica or the app in front changed; `--full` asks for it. `diff snapshot` instead compares the screen with this session's last snapshot element by element.

```json
{"success":true,"data":{"origin":"com.android.settings","readMs":0.123,"replicaRevision":5,"servedFromReplica":true,"snapshot":{"baseRevision":1,"kind":"unchanged","revision":2},"target":"emulator-5554","verdict":{"backend":"fake","degraded":false,"fallback":false,"reasons":[]}},"error":null}
```

**Refs.** Write `@e3`; `ref=e3` and `e3` work too, and a number is never reused. A ref names one control for as long as it stays on screen: its element and, for an element without an identifier, also its accessible name. When the element goes, even briefly (another screen, a relaunch, a sheet that hides the screen beneath it), or an element without an identifier changes its name, the next snapshot gives it a fresh ref and the old one fails with `stale_ref`. A resync after an event sequence gap, a reconnection or a fall back to another tree backend rebuilds the mirror and makes every earlier ref stale. A ref from another session's snapshots fails with `unknown_ref`, an element under a dialog with `element_covered`, one scrolled out of view with `element_offscreen`. Refs never wait.

**Acting and reading.**

| Command | Does |
|---|---|
| `click @e4`, `click 200 480`, `click @e4 --hold 800` | taps a ref, a selector or a point in device points; holds for a long press |
| `fill @e2 agent` | replaces a field's text; the reply's `method` says whether it was set `direct` or typed as `keys` |
| `type @e2 ' smith'` | adds key events to the field, or to the focused one when no target is given |
| `press Enter` | presses a key, such as `Home`, `Back` on Android, a letter, or modifiers joined with `+` as in `Control+a` |
| `swipe up`, `swipe 200 700 200 200` | moves the finger in a direction or between two points |
| `get text @e5`, `get value @e2`, `get box @e4` | reads an element's text, value or bounds |
| `screenshot [path]` | saves the latest frame as a JPEG, quality 80 and longest edge 1000 px by default |

**Selectors.** Where a ref goes, a selector works too, and waits up to the timeout for a match: `#id` or `id=...` for a test identifier, `text=...` or bare text for a name or value that contains the text, `text==...` for one that is exactly the text, `role=...`.

**Clicks and their timing.** `click` (alias `tap`) replies once the tap's first frame change and first tree delta have arrived, or after 200 ms without a change, at most 1 s; after the first tap of a new input session it waits up to 1.5 s for the first change and 2.5 s in all. The reply's spans are milliseconds on the daemon's clock, from the dispatch: `dispatchMs` until the target accepted the input, `frameChangeMs` until the first frame whose hash differs, `treeDeltaMs` until the first tree delta, and `watchedMs` for how long the watch lasted. `frameChangeMs` and `treeDeltaMs` are absent, never 0 or null, when the signal did not come: the tap changed nothing that way, or the target has no frame source (Android devices, and emulators without the gRPC endpoint).

```json
{"success":true,"data":{"accepted":true,"clicked":"@e7","dispatchMs":0.101,"frameChangeMs":0.026,"ref":"@e7","treeDeltaMs":0.112,"watchedMs":5.183,"x":204.0,"y":120.0},"error":null}
```

**Settle and waits.** `wait --settle` resolves once neither the tree nor the frames have changed for the quiet window (`--quiet`, default 200 ms), at most `--timeout` (default 3000 ms). `endedBy` names the signal that changed last: `tree`, `frame`, `both` (within 20 ms of each other), `none` (nothing changed since the last action, or the screen was already quiet), or `timeout`. A timeout is still a success: `settled` is false, `stillChanging` names what kept changing, and `warning` says so. `--tree-only` ignores frames, for video. `wait <ref or selector>` waits for an element (`--state visible|hidden|detached|enabled`) and `wait --text <text>` for text; both fail with `wait_timeout` and a summary of the screen. `open` waits for the app's first tree, past any launch splash, and for its screen to settle, and replies `settled`, `elapsedMs` and `firstTreeMs`. On the fixture's endless animation:

```json
{"success":true,"data":{"elapsedMs":3000.329,"endedBy":"timeout","frameChanges":175,"settled":false,"stillChanging":["frame"],"treeChanges":0,"waited":"settle"},"error":null,"warning":"the screen did not settle within 3000 ms: frame changes kept arriving"}
```

**Scrolling.** `scroll down 600` moves the content 600 points. A stroke must be longer than a tap, more than 8 points on Android and 10 on iOS, or the command fails with `stroke_too_short`. `scroll down 600 --until '#row-150'` scrolls stroke by stroke, letting the screen settle after each, until the element is on screen, and fails with `element_not_found` once the content stops moving or at its timeout.

**Timeouts.** Waiting commands take `--timeout <ms>`; otherwise the session's default applies (`set timeout <ms>`), else `--default-timeout` or `MOBILE_AGENT_DEFAULT_TIMEOUT`, else 10000 ms. The maximum is 600000 ms.

**Batch.** `batch` runs commands in order, given as quoted strings or as a JSON array of argv arrays on stdin, and `--bail` stops at the first failure. With `--json` it prints one array of `{command, success, result, error}` entries, with `code` on failures, and exits 1 when any entry failed. Here the third command never ran:

```
$ mobile-agent --json batch --bail 'get text #settings-wifi' 'click @e99' 'snapshot -i'
[{"command":["get","text","#settings-wifi"],"error":null,"result":{"text":"Wi-Fi"},"success":true},{"code":"unknown_ref","command":["click","@e99"],"error":"@e99 is not a ref from session \"default\"'s snapshots of emulator-5554; take a snapshot and use the refs it prints","result":null,"success":false}]
```

**Sessions and targets.** One daemon serves every session. `--session <name>` (default `default`) keeps target claims, refs, diff baselines and default timeouts apart. A command claims its target for its session, and another session is refused with `target_claimed`, naming the holder in `data.holder`, unless it passes `--force`; `close` releases the claim. With several targets and no `--device` or `--platform`, a command fails with `ambiguous_target` and lists them in `data.candidates`. Android allows one UI automation owner per device, so while the daemon holds one, Maestro, Appium and `uiautomator dump` cannot run against it; attaching while another tool holds the role fails with `automation_conflict` naming the likely holder, and `--force` takes it over.

**Error codes.**

| Code | Meaning |
|---|---|
| `stale_ref` | the ref's element is gone, or the mirror was rebuilt since the ref was issued; snapshot again |
| `unknown_ref` | the ref is not from this session's snapshots of the target |
| `element_covered`, `element_offscreen` | a dialog covers the element; it is scrolled out of view |
| `element_not_found`, `wait_timeout` | no element matched within the timeout; the waited-for condition did not hold |
| `stroke_too_short` | a scroll stroke no longer than the platform's tap threshold |
| `no_target`, `ambiguous_target`, `target_not_found`, `target_claimed` | target selection and session claims |
| `app_not_found`, `input_rejected`, `no_frame`, `disconnected` | no such app; the target refused the input; no frame source or no frame yet; the target is gone |
| `automation_conflict`, `target_not_ready`, `agent_failed`, `adb_failed` | Android: another holder of UI automation; the device is not in the `device` state; the agent did not start; adb failed |
| `unsupported_xcode`, `target_unavailable`, `no_input_transport` | Simulator: Xcode outside the range; a private framework call failed; no input transport works |
| `daemon_unavailable`, `daemon_timeout`, `invalid_argument` | the CLI could not reach the daemon or had no reply in time; bad usage |

The full lists are in `crates/daemon/README.md` (Errors) and `crates/cli/README.md` (Output).

## Measured

The harness judges each gate at p95 (`crates/bench/README.md`, Gates).

iOS Simulator:

| Gate | Limit | Measured |
|---|---|---|
| CLI process start | under 20 ms | placeholder-pending-10.2 |
| Cold attach | under 2 s | placeholder-pending-10.2 |
| Warm snapshot, SwiftUI 500-node screen | under 150 ms | placeholder-pending-10.2 |
| Warm snapshot, React Native (Expo) 500-node screen | under 250 ms | placeholder-pending-10.2 |
| Tap dispatch | under 20 ms | placeholder-pending-10.2 |

Android emulator, from `reports/10.1-emulator-grpc/report.md`: Android Emulator 37.1.11, an API 36 arm64 AVD started with `-gpu host -grpc 8554 -grpc-use-token`, commit 05a4756, 50 measured iterations. With the control endpoint on, taps and frames go through the emulator's gRPC controller; with it off, taps go through the agent and there is no frame stream.

| Gate | Limit | Controller on | Verdict | Controller off |
|---|---|---|---|---|
| CLI process start | under 20 ms | 11.6 ms through the bench's Android shim; 5.0 ms for the binary alone | pass | placeholder-pending-10.1 |
| Cold attach | under 1.5 s | Expo 894 ms, Compose 1208 ms | pass | placeholder-pending-10.1 |
| Warm snapshot, 500-cell screen | under 100 ms | Expo 12.8 ms, Compose 19.4 ms | pass | placeholder-pending-10.1 |
| Tap to frame change | under 50 ms | Expo 52.5 ms, Compose 55.1 ms | fail, over by 2.5 and 5.1 ms | placeholder-pending-10.1 |
| Tap dispatch | no emulator gate | Expo 2.8 ms, Compose 3.3 ms | | placeholder-pending-10.1 |

Spike measurements. The throwaway programs under `spikes/` measured these before the crates existed, on one host (Apple M1 Max, macOS 26.6.2, Xcode 27.0, CoreSimulator 1171.7, iOS 26.5, an API 36 arm64 emulator); the spike verdicts table in `openspec/changes/mobile-automation-foundation/design.md` summarizes them. They are not harness results.

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

`crates/bench` (`mobile-agent-bench`, run with `just bench`) drives the release CLI as an agent does, one `--json` process per command, on the reference fixtures of `fixtures/CONTRACT.md` (SwiftUI and Expo on the Simulator, Expo and Compose on Android), with 3 warm-up and 50 measured iterations per series and nearest-rank p50, p95, maximum, failure and flake rates. The same login, list-scroll and 500-node flows run through Maestro 2.10.0 (on Java 21.0.12) and agent-device 0.21.14 (on Node 25.8.0) as baselines, as the emulator run's provenance in `reports/10.1-emulator-grpc/report.md` records, and through maestro-runner when it is installed, each timed by wall clock around the tool's process. Each run writes `report.json` and `report.md`, committed under `reports/`, and exits non-zero when a gate fails.

## Status and limitations

- Physical Android devices have not been measured; the device run (task 10.3) waits for one. Devices have no frame stream, so clicks there report no `frameChangeMs` and settle rests on the tree. OEM Android builds are unverified.
- Only Xcode 27.0, iOS 26.5 and the API 36 emulator have run.
- The emulator misses its 50 ms tap-to-frame gate: with host GPU rendering (`-gpu host`) and the controller on, p95 was 52.5 ms on the Expo fixture and 55.1 ms on the Compose one. Under software rendering spike S7 had measured 107 ms p50, so the GPU mode matters.
- On a Simulator whose first boot did not come from the driver, the first tree reads can fail for several seconds, until the reader has turned automation mode on (spike S4); retry.
- React Native: `reports/10.4-rn-smoke.md` compared 17 React Native screens on the Simulator (Bluesky logged out, Ignite's demo app, the Expo fixture), but no XCTest read there reported a sparse root, so the claim that mobile-agent's snapshots stay whole where XCTest's go sparse is untested until a signed-in Bluesky account reaches its feed, thread and profile screens.
- The drivers use private Apple Simulator frameworks and hidden Android APIs. This is development tooling; nothing of it ships inside an app.
- Not built: physical iOS, an MCP server, a CDP-subset endpoint and Android video streaming.
- The OpenSpec change in `openspec/changes/mobile-automation-foundation/` (proposal, design, specs, tasks) is the source of truth for the design, its decisions and the remaining tasks.
