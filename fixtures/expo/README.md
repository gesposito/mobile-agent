# FixtureExpo

React Native reference app for the benchmark harness and the spikes. It uses Expo SDK 57 and React Native 0.86.3, with the New Architecture and Hermes; at this React Native version neither can be turned off. It implements the shared fixture contract in [`../CONTRACT.md`](../CONTRACT.md), as the other reference fixtures do, so the harness and the spikes run the same flows on each of them.

- Bundle identifier: `dev.mobileagent.fixture.expo` (Android package too)
- App name: `FixtureExpo`, URL scheme `fixtureexpo`
- Runtime dependencies: `expo`, `expo-location`, `react`, `react-native` and `react-native-safe-area-context`, which supplies the system insets; see Safe area and orientation. No navigation library: the tab bar is five Pressables, only the selected screen is mounted, and screen state lives in the root component.
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

## Build (physical iPhone)

```sh
MOBILE_AGENT_IOS_TEAM=<team> DESTINATION=device ./build.sh
```

`DESTINATION` picks what an iOS build is for: `simulator`, the default, or `device`. A device build leaves the project as it is. It copies the sources into `build/device/` with `rsync`, without `node_modules/`, `ios/`, `android/`, `build/` and `.expo/`, then takes the four steps above in that copy, each only when its output there is missing or stale: `sfw bun install --frozen-lockfile`, `sfw bunx expo prebuild --platform ios --no-install`, `pod install`, and `xcodebuild` for `generic/platform=iOS` with the `iphoneos` SDK, its derived data in the copy's `build/`. The copy keeps what it generated from one build to the next, as the project does for the Simulator. The Release app lands in `build/device/build/Build/Products/Release-iphoneos/FixtureExpo.app`; a Debug one would load its JavaScript from Metro on the Mac, which has not been exercised on a phone.

The build is signed manually, so `xcodebuild` registers nothing with the account, from the variables `agent-ios/build.sh` signs the runner with:

| Variable | Default | Meaning |
|---|---|---|
| `MOBILE_AGENT_IOS_TEAM` | none, required | the development team; it stays in the environment and enters no tracked file |
| `MOBILE_AGENT_IOS_IDENTITY` | `Apple Development` | the signing identity's name in the keychain |
| `MOBILE_AGENT_IOS_PROFILE` | `mobile-agent development` | the installed provisioning profile's name; the profile must include the device and cover `dev.mobileagent.fixture.expo`, as a wildcard App ID `dev.mobileagent.*` does |

Once the bundle is built, `codesign --verify --strict` checks it, and stdout carries its path. Without `MOBILE_AGENT_IOS_TEAM` the script builds nothing, says why on stderr and exits with status 3, which `../build.sh` reports as a skipped device build. `DESTINATION=device` with `PLATFORM=android` exits with status 2, as does any `DESTINATION` other than `simulator` and `device`: the APK serves emulators and devices alike.

On a host with Xcode 27.0 at a 1-minute load of 3 to 4, the first device build took 195.8 s: the copy, `bun install` of 456 packages in 1.2 s, the prebuild, `pod install` of 88 pods and the Release build. One with nothing to install, prebuild or compile took 16.8 s, in the script phases React Native's and Expo's pods run on every build and the JavaScript bundle's.

Install the bundle with `xcrun devicectl device install app --device <udid> "$app"` and launch it through `mobile-agent` with the `ios` driver (`MOBILE_AGENT_DRIVERS=ios`), whose `open --env` passes `FIXTURE_INITIAL_TAB`; see Launch options.

## Build (Android)

```sh
PLATFORM=android ./build.sh
```

`PLATFORM` defaults to `ios`. With `PLATFORM=android`, `build.sh` installs `node_modules/` in the same way. It prebuilds `android/` under the same rule as `ios/` (`sfw bunx expo prebuild --platform android --no-install`, with the hash in `android/.prebuild-inputs`), then runs `./gradlew --no-daemon -Pandroid.builder.sdkDownload=false :app:assembleRelease` in `android/`. stdout carries the absolute APK path, `android/app/build/outputs/apk/release/app-release.apk`.

Requirements:

- An Android SDK at `ANDROID_HOME` (default `ANDROID_SDK_ROOT`, then `$HOME/Library/Android/sdk`) with these packages:
  - `platforms;android-36`
  - `build-tools;36.0.0`, used by the app module
  - `build-tools;35.0.0`: the Expo library modules set no build-tools version, so they fall back to the Android Gradle Plugin's default, 35.0.0
  - `ndk;27.1.12297006` and `cmake;3.22.1`: React Native's Gradle plugin always compiles C++ for the app, and `expo-modules-core` compiles C++ too
- A JDK 17 or newer through `JAVA_HOME` or on `PATH` (tested with Temurin 21).

`-Pandroid.builder.sdkDownload=false` stops Gradle from installing a missing SDK package on its own. The build fails and names the package instead; install it with `sdkmanager --install "<package>"` from the SDK's `cmdline-tools`.

The release APK embeds the Hermes bytecode bundle, so it runs without Metro. It is signed with the debug keystore from Expo's template and carries native code for `armeabi-v7a`, `arm64-v8a`, `x86` and `x86_64`, so the same APK installs on ARM and x86 emulators. `CONFIGURATION=Debug` builds `:app:assembleDebug`, which loads JavaScript from Metro; that variant has not been exercised.

`./gradlew buildEnvironment` reports the versions Expo generated: Gradle 9.3.1 (wrapper), Android Gradle Plugin 8.12.0 and Kotlin Gradle plugin 2.1.20. The app compiles and targets SDK 36, with a minimum of 24.

## Install and launch

```sh
app="$(./build.sh)"
xcrun simctl install booted "$app"
xcrun simctl launch booted dev.mobileagent.fixture.expo
xcrun simctl terminate booted dev.mobileagent.fixture.expo
```

Replace `booted` with a Simulator UDID to target a specific device. The app opens on the Login tab.

On an Android emulator or device:

```sh
apk="$(PLATFORM=android ./build.sh)"
adb install -r "$apk"
adb shell am start -S -n dev.mobileagent.fixture.expo/.MainActivity
adb shell am force-stop dev.mobileagent.fixture.expo
```

`-S` force-stops a running copy first, so the activity and its launch options start fresh.

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
- **Physical iPhone:** the launch environment, which `mobile-agent open --env` passes through the `ios` driver's launch:

  ```sh
  MOBILE_AGENT_DRIVERS=ios mobile-agent --device <udid> open --relaunch --env FIXTURE_INITIAL_TAB=big dev.mobileagent.fixture.expo
  ```
- **Android:** the string extra `initial_tab` on the launch intent:

  ```sh
  adb shell am start -S -n dev.mobileagent.fixture.expo/.MainActivity --es initial_tab big
  ```

  On an API 36 emulator this opens the Big tab.

The native entry points pass the value to JavaScript as the root component's `initialTab` prop. The local config plugin `plugins/withInitialTab.js` writes that code into the generated `AppDelegate.swift` and `MainActivity.kt`, so it survives `expo prebuild --clean`. If the Expo template no longer contains the code the plugin edits, prebuild fails instead of dropping the option.

## Navigation and state

Only the selected tab's screen is mounted, so the other tabs' content is not in the accessibility tree. Screen state lives in the root component (`App.tsx`) rather than in the screens, so it survives tab switches, as the contract requires:

- **Login:** the typed username and password, and `login-status`. The fields stay uncontrolled, so fast programmatic typing never races a JavaScript re-render; each mount seeds them from the stored text as `defaultValue`.
- **List:** the open detail view and the exact scroll offset, which is stored on every scroll event. When the tab is shown again, the list mounts with `initialScrollIndex` at the first visible row, so those rows render in the first pass, and with `contentOffset` at the stored offset. When `contentOffset` is set, FlatList does not scroll to the index itself.
- **Anim:** the `anim-toggle` value. The spinner's angle is not kept: returning to the tab while the toggle is on restarts the rotation from 0°.
- **System:** `system-permission-status`. A request that resolves after the user has left the tab still updates it.

The Big tab has no state to keep.

## Safe area and orientation

The root component wraps the screens and the tab bar in a `SafeAreaView` from `react-native-safe-area-context`, inside that library's `SafeAreaProvider`. The view pads its content by the system insets on every edge:

- **iOS:** the window's safe area, which in portrait is the status bar and the home indicator: the same insets React Native's own `SafeAreaView` applies.
- **Android:** Expo draws edge to edge (`edgeToEdgeEnabled=true` in the generated `android/gradle.properties`), and React Native's own `SafeAreaView` is a plain View there. The library reads the status bar, navigation bar, display cutout and caption bar insets from the window and pads by them. The measured insets and bounds are under Android below.

The provider gets `initialMetrics={initialWindowMetrics}`, the insets the native module reads at startup. Without them it renders nothing until its first native insets event.

The dependency is `react-native-safe-area-context@~5.7.0`, the range Expo SDK 57 lists for it in `expo/bundledNativeModules.json`, added with `sfw bun add`. It is a native module: autolinking adds its pod on iOS and its Gradle project on Android, both built from `node_modules`.

`app.json` sets `"orientation": "default"`, so the app follows the device. Prebuild writes `android:screenOrientation="unspecified"` on `MainActivity` and lists all four orientations under `UISupportedInterfaceOrientations` in `Info.plist`. Expo's template declares `orientation` and `screenSize` in the activity's `android:configChanges`, so a rotation keeps the process, and the app stays on the selected tab. The Big tab switches to 25 columns in landscape; see the 500-element contract.

To rotate an Android emulator from a shell, note both settings, then turn auto-rotate off and set the rotation:

```sh
adb shell settings get system accelerometer_rotation
adb shell settings get system user_rotation
adb shell settings put system accelerometer_rotation 0   # auto-rotate off
adb shell settings put system user_rotation 1            # 90 degrees
```

Restore both afterwards, `accelerometer_rotation` first. On the API 36 emulator, restoring `user_rotation` first, while auto-rotate was still off, left auto-rotate off although it was set back to on straight after. When the display returns to portrait while rotation is locked, SystemUI locks it at portrait again (`RotationButtonController#onRotationWatcherChanged` in the rotation lock history of `adb shell dumpsys window displays`), and that write came after the restored value.

## 500-element contract

On the Big tab:

- Exactly 500 accessibility elements carry the identifiers `cell-1` to `cell-500`. Each has role button (the Button trait on iOS) and the label `Cell <n>`. On iOS the number drawn inside a cell belongs to that cell's element and is not a separate node.
- `big-count` is a separate header text, `500 elements`, and is not one of the 500.
- All 500 cells mount at once, in a ScrollView flex-wrap grid with no virtualization, so the count does not depend on scroll position.
- The grid is 20 columns by 25 rows while its ScrollView is taller than it is wide, and 25 columns by 20 rows while it is wider, as in landscape. Cells take the largest whole size at which the whole grid fits in the ScrollView, whose size `onLayout` reports, so the grid is on screen in either orientation and tools that drop off-screen nodes still see all 500. On a 402 pt wide iPhone 17 in portrait that gives 19 pt cells. Until the first `onLayout`, the window's size stands in, which gives the same cells in portrait; in landscape the cells shrink to fit once `onLayout` arrives.
- The tab bar adds five more buttons (`tab-*`). A snapshot filtered to buttons therefore returns 505 on this tab, and one filtered to the `cell-` prefix returns 500.
- **Android:** on an API 36 emulator (1080 × 2400, 420 dpi), `adb shell uiautomator dump` of the Big tab has exactly 500 `resource-id="cell-` matches: `cell-1` to `cell-500`, each a clickable `android.widget.Button` with content-desc `Cell <n>`. The grid fits the screen as 50 px (19 dp) cells, so the dump loses none. Rotated to landscape (2400 × 1080), the grid is 25 × 20 with 29 px (11 dp) cells, and the dump again has all 500. There the ScrollView is 672 px, or 256 dp, tall: exactly what 12 dp cells need with the padding. The height `onLayout` reports is a fraction below 256 dp, so the cells are 11 dp.

  That matters, because the dump leaves out nodes outside the screen. A dump of the List tab lists only the rows on screen, `row-1` to `row-17` in portrait and `row-1` to `row-7` in landscape, although at least 20 are mounted.

  Unlike iOS, the dump also lists each cell's number as a child `android.widget.TextView` with no id. The Big tab's dump therefore holds 1022 nodes in either orientation, including 505 Buttons (cells and tabs) and 505 TextViews.

## Package installs and Socket Firewall

Every package install goes through Socket Firewall (`sfw`): `sfw bun install`, `sfw bun add`, `sfw bunx expo prebuild`. Socket does not list Bun as supported, so this was checked directly. `sfw` sets `HTTPS_PROXY` to its local proxy and injects its CA through `NODE_EXTRA_CA_CERTS`. Run under `sfw`, `bun add --verbose` shows the registry requests going through an HTTP CONNECT tunnel to that proxy (`200 Connection Established`). `sfw` prints no package counts for Bun.

`expo prebuild` uses the native template bundled in the `expo` package (`expo/template.tgz`), so it downloads nothing.

`pod install` cannot be wrapped by Socket Firewall and runs as is. No pod comes from the CocoaPods trunk: every podspec is local, in `node_modules`. However, `pod install` fetches Debug and Release tarballs of three prebuilt artifacts from `repo1.maven.org`, and those downloads bypass Socket:

- React Native core
- React Native dependencies
- Hermes

It checks each tarball's SHA1 and keeps it in React Native's shared download cache. The Xcode script phases only extract these tarballs; they download nothing.

The precompiled Expo module xcframeworks ship inside the npm packages, which `sfw bun install` fetched.

The Android build downloads more that Socket Firewall cannot wrap:

- **The Gradle 9.3.1 distribution:** the wrapper fetches it from `services.gradle.org`.
- **Gradle plugins and Maven dependencies:** these include the Android Gradle Plugin, Kotlin, React Native, Hermes and the Expo modules' Android artifacts. They come from Google's Maven repository, Maven Central and JitPack, the repositories `android/build.gradle` declares, and are cached under `~/.gradle`.
- **SDK packages:** `ndk;27.1.12297006`, `cmake;3.22.1` and `build-tools;35.0.0`, installed with `sdkmanager` from Google's repository. Gradle never installs these itself here; see Build (Android).

`expo prebuild --platform android` uses the same bundled template as iOS and downloads nothing.

## Android

Verified on an API 36 arm64 emulator (`sdk_gphone64_arm64`, 1080 × 2400, 420 dpi):

- The release APK installs with `adb install -r`.
- `--es initial_tab big` opens the Big tab.
- `uiautomator dump` finds all 500 cells in portrait and in landscape; see the 500-element contract.
- In portrait on every tab, and in landscape on every tab but Anim, the centre of every contract element in the dump lies in the area the system bars leave free. Tapping the centres of `row-1` and then `detail-back` opens and closes the detail view. The Anim tab was checked with the spinner stopped, because `uiautomator dump` waits for a second without accessibility events, which the running spinner never allows.
- Rotating the emulator rotates the app, and the process and the selected tab survive the rotation.

The system bars on that emulator, from `adb shell dumpsys window` with gesture navigation:

| Orientation | Status bar | Navigation bar | Display cutout | Free area |
|---|---|---|---|---|
| Portrait | top 132 px | bottom 63 px, from y = 2337 | top 132 px | y from 132 to 2337 |
| Landscape (`user_rotation` 1) | top 74 px | bottom 63 px, from y = 1017 | left 132 px | x from 132, y from 74 to 1017 |

Bounds from `uiautomator dump`:

| Element | Portrait | Landscape |
|---|---|---|
| `row-1` | [0,132][1080,258] | [132,74][2400,200] |
| `detail-back` | [63,195][247,311] | [195,137][379,253] |
| `tab-login` | [0,2191][216,2337] | [132,871][586,1017] |
| `big-count` | [0,132][1080,256] | [132,74][2400,198] |

The tab bar ends where the navigation bar starts. Its centres, at y = 2264 in portrait and y = 944 in landscape, therefore also clear the 84 px band at the bottom where the system always takes swipes (`mandatorySystemGestures`, from y = 2316 and y = 996).

## Physical iPhone

Verified on an iPhone SE (3rd generation) with iOS 26.6.2: the Release device build installed through CoreDevice (`devicectl device install app`). Through `mobile-agent` with the `ios` driver, every identifier of the contract listed on its tab:

| Tab | `snapshot -i` | Only in the full `snapshot` |
|---|---|---|
| Login | the five tabs, `login-username`, `login-password`, `login-submit` | `login-status` |
| List | the five tabs and the rows on screen, `row-1` to `row-14`; with a detail open, `detail-back`, the list hidden from accessibility | `detail-<n>` |
| Big | the five tabs, `big-count` and the 500 cells | |
| Anim | the five tabs, `anim-toggle` | `anim-spinner` |
| System | the five tabs, `system-alert`, `system-permission`; with the alert up, its title and its `OK` button, which carries no identifier | `system-permission-status` |

`scroll --until id=row-200` reached the last row in 22 strokes, and its detail opened. The alert's message did not list on the device.
