# FixtureExpo

React Native reference app for the benchmark harness and the spikes. It uses Expo SDK 57 and React Native 0.86.3, with the New Architecture and Hermes; at this React Native version neither can be turned off. It implements the shared fixture contract in [`../CONTRACT.md`](../CONTRACT.md), as the other reference fixtures do, so the harness and the spikes run the same flows on each of them.

- Bundle identifier: `dev.mobileagent.fixture.expo` (Android package too)
- App name: `FixtureExpo`, URL scheme `fixtureexpo`
- Runtime dependencies: `expo`, `expo-location`, `react`, `react-native`. No navigation library: the tab bar is five Pressables, only the selected screen is mounted, and screen state lives in the root component.
- Content is fixed: nothing is random and nothing touches the network.

## Build (iOS Simulator)

```sh
./build.sh
```

`build.sh` takes each step only when its output is missing:

1. `sfw bun install --frozen-lockfile` if `node_modules/` is missing.
2. `sfw bunx expo prebuild --platform ios --no-install` if `ios/` is missing, or if `app.json`, `package.json`, `bun.lock` or `plugins/` changed since `ios/` was generated. `ios/.prebuild-inputs` records their hash; a changed hash deletes `ios/` and prebuilds again.
3. `pod install` in `ios/` if `ios/Pods/` is missing.
4. `xcodebuild -workspace ios/FixtureExpo.xcworkspace -scheme FixtureExpo -configuration Release -destination 'generic/platform=iOS Simulator' -derivedDataPath build CODE_SIGNING_ALLOWED=NO build`

All build output goes to stderr. stdout carries one line, the absolute path of the `.app`, so `app="$(./build.sh)"` works. The script exits non-zero when any step fails, including a JavaScript error that breaks the bundle phase.

Release (the default) embeds the Hermes bytecode bundle, so the app launches without a Metro server. This is the build the harness uses. The app lands in `build/Build/Products/Release-iphonesimulator/FixtureExpo.app`.

`ios/`, `android/`, `build/` and `node_modules/` are generated and git-ignored. To force a clean native build, run `rm -rf ios build && ./build.sh`. The manual equivalent is `sfw bunx expo prebuild --platform ios --clean --no-install` followed by `pod install` in `ios/`.

Type-check the sources with `bun run typecheck`. Metro strips types without checking them, so a type error does not fail `build.sh`.

## Install and launch

```sh
app="$(./build.sh)"
xcrun simctl install booted "$app"
xcrun simctl launch booted dev.mobileagent.fixture.expo
xcrun simctl terminate booted dev.mobileagent.fixture.expo
```

Replace `booted` with a Simulator UDID to target a specific device. The app opens on the Login tab.

## Debug build

In Debug, the Xcode bundle phase skips bundling and embeds no JavaScript. The app loads its bundle from Metro on `localhost:8081`, so start Metro before launching it.

```sh
app="$(CONFIGURATION=Debug ./build.sh)"   # build/Build/Products/Debug-iphonesimulator/FixtureExpo.app
bunx expo start                           # keep Metro running
xcrun simctl install booted "$app"
xcrun simctl launch booted dev.mobileagent.fixture.expo
```

Debug builds are for working on the fixture only. Timing runs use Release.

## Identifier contract

`testID` becomes `accessibilityIdentifier` on iOS and the resource id on Android. Numbering is 1-based everywhere.

| Screen | Identifier | Element | Accessibility | Content and behaviour |
|---|---|---|---|---|
| Tab bar | `tab-login`, `tab-list`, `tab-big`, `tab-anim`, `tab-system` | Pressable | role button, labels `Login`, `List`, `Big`, `Anim`, `System`; the active tab has the Selected trait | Switches the screen. Screen state survives the switch; see Navigation and state. |
| Login | `login-username` | TextInput | label `Username` | Placeholder `Username`. Autocorrect, autocapitalisation, spell check and autofill are off. |
| Login | `login-password` | TextInput, secure | label `Password` | Placeholder `Password`. |
| Login | `login-submit` | Pressable | role button, label `Sign in` | Dismisses the keyboard and sets `login-status`. |
| Login | `login-status` | Text | static text | Always present. Reads `Signed out` until the first submit, then `Welcome, <username>` when the password is `hunter2`, otherwise `Invalid credentials`. |
| List | `row-1` … `row-200` | Pressable in a FlatList | role button, label `Row <n>` | Text `Row <n>`. Tapping opens `detail-<n>`. |
| List | `detail-<n>` | Text | static text | `Detail <n>`, on a detail view drawn over the list. While the detail view is open, the list is hidden from accessibility and keeps its scroll position. |
| List | `detail-back` | Pressable | role button, label `Back` | Closes the detail view. |
| Big | `big-count` | Text | header | `500 elements`. |
| Big | `cell-1` … `cell-500` | Pressable | accessible, role button, label `Cell <n>` | See the 500-element contract below. |
| Anim | `anim-spinner` | Animated.View | accessible, role image, label `Spinner` | Turns 360° every 1000 ms with `Animated.loop` and `useNativeDriver: true`, and never settles while running. |
| Anim | `anim-toggle` | Switch | label `Animate`, value `1` or `0` | On at launch. Turning it off stops the loop and resets the spinner to 0°. |
| System | `system-permission` | Pressable | role button, label `Request location permission` | Calls `Location.requestForegroundPermissionsAsync()`, which shows the system When-In-Use location prompt on first use, and updates `system-permission-status` with the result. |
| System | `system-permission-status` | Text | static text | `Permission: unknown` until a request resolves, then `Permission: granted` or `Permission: denied`. The screen never reads the current authorization on mount, so the initial text does not depend on earlier runs. |
| System | `system-alert` | Pressable | role button, label `Show in-app alert` | `Alert.alert` titled `In-app alert` with the message `This alert is presented by the app.` and one button, `OK`. |

The contract gives the alert's OK button the identifier `inapp-alert-ok` where the platform allows it. React Native's `Alert.alert` cannot set an identifier on its buttons, so in this fixture the OK button has none; drivers locate it by its label, `OK`.

The List tab's FlatList is virtualized, with `initialNumToRender={20}`, `windowSize={5}` and a fixed row height of 48 pt. Rows far off screen are not mounted, so they are not in the accessibility tree until they are scrolled near.

The permission prompt text comes from `NSLocationWhenInUseUsageDescription`, which the `expo-location` plugin in `app.json` writes: "FixtureExpo requests location to exercise the system permission prompt." The plugin is configured to leave out the Always and motion usage keys.

## Launch options

`FIXTURE_INITIAL_TAB` selects the tab shown at launch: `login`, `list`, `big`, `anim` or `system`. It defaults to `login`, and an unknown value also selects `login`.

- **iOS Simulator:** prefix the variable with `SIMCTL_CHILD_` in the environment of `xcrun simctl launch`, which passes it to the app as `FIXTURE_INITIAL_TAB`:

  ```sh
  SIMCTL_CHILD_FIXTURE_INITIAL_TAB=anim xcrun simctl launch booted dev.mobileagent.fixture.expo
  ```

  The app reads the variable when its process starts, so pass `--terminate-running-process` if a copy may already be running.
- **Android:** the string extra `initial_tab` on the launch intent, for example `adb shell am start -n dev.mobileagent.fixture.expo/.MainActivity --es initial_tab anim`. This path has not been exercised yet; see Android below.

The native entry points pass the value to JavaScript as the root component's `initialTab` prop. The local config plugin `plugins/withInitialTab.js` writes that code into the generated `AppDelegate.swift` and `MainActivity.kt`, so it survives `expo prebuild --clean`. If the Expo template no longer contains the code the plugin edits, prebuild fails instead of dropping the option.

## Navigation and state

Only the selected tab's screen is mounted, so the other tabs' content is not in the accessibility tree. Screen state lives in the root component (`App.tsx`) rather than in the screens, so it survives tab switches, as the contract requires:

- **Login:** the typed username and password, and `login-status`. The fields stay uncontrolled, so fast programmatic typing never races a JavaScript re-render; each mount seeds them from the stored text as `defaultValue`.
- **List:** the open detail view and the exact scroll offset, which is stored on every scroll event. When the tab is shown again, the list mounts with `initialScrollIndex` at the first visible row, so those rows render in the first pass, and with `contentOffset` at the stored offset. When `contentOffset` is set, FlatList does not scroll to the index itself.
- **Anim:** the `anim-toggle` value. The spinner's angle is not kept: returning to the tab while the toggle is on restarts the rotation from 0°.
- **System:** `system-permission-status`. A request that resolves after the user has left the tab still updates it.

The Big tab has no state to keep.

## 500-element contract

On the Big tab:

- Exactly 500 accessibility elements carry the identifiers `cell-1` to `cell-500`. Each has role button (the Button trait on iOS) and the label `Cell <n>`. The number drawn inside a cell belongs to that cell's element and is not a separate node.
- `big-count` is a separate header text, `500 elements`, and is not one of the 500.
- All 500 cells mount at once, in a ScrollView flex-wrap grid with no virtualization, so the count does not depend on scroll position.
- The grid has 20 columns, and cell size follows the window width. On a 402 pt wide iPhone 17 that gives 19 pt cells in a 20 × 25 grid that fits on screen, so tools that drop off-screen nodes still see all 500. On narrower screens the ScrollView scrolls, and all 500 stay mounted.
- The tab bar adds five more buttons (`tab-*`). A snapshot filtered to buttons therefore returns 505 on this tab, and one filtered to the `cell-` prefix returns 500.

## Package installs and Socket Firewall

Every package install goes through Socket Firewall (`sfw`): `sfw bun install`, `sfw bun add`, `sfw bunx expo prebuild`. Socket does not list Bun as supported, so this was checked directly. `sfw` sets `HTTPS_PROXY` to its local proxy and injects its CA through `NODE_EXTRA_CA_CERTS`. Run under `sfw`, `bun add --verbose` shows the registry requests going through an HTTP CONNECT tunnel to that proxy (`200 Connection Established`). `sfw` prints no package counts for Bun.

`expo prebuild` uses the native template bundled in the `expo` package (`expo/template.tgz`), so it downloads nothing.

`pod install` cannot be wrapped by Socket Firewall and runs as is. No pod comes from the CocoaPods trunk: every podspec is local, in `node_modules`. However, `pod install` fetches Debug and Release tarballs of three prebuilt artifacts from `repo1.maven.org`, and those downloads bypass Socket:

- React Native core
- React Native dependencies
- Hermes

It checks each tarball's SHA1 and keeps it in React Native's shared download cache. The Xcode script phases only extract these tarballs; they download nothing.

The precompiled Expo module xcframeworks ship inside the npm packages, which `sfw bun install` fetched.

## Android

Not exercised yet: this machine has no Android SDK, so no Android prebuild or Gradle build has run for this fixture. `app.json` already sets `android.package`. One thing to check when Android is exercised: React Native's built-in `SafeAreaView` is a plain View on Android, so under edge-to-edge the screens may draw beneath the status and navigation bars.
