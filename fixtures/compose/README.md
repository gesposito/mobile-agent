# FixtureCompose

Jetpack Compose reference app for the benchmark harness and the spikes. It implements the shared fixture contract in [`../CONTRACT.md`](../CONTRACT.md) on Android, as the SwiftUI and Expo fixtures do on their platforms, so the harness and the spikes run the same flows on each of them.

- Application id: `dev.mobileagent.fixture.compose`, one activity, `MainActivity`; app name `FixtureCompose`
- Min SDK 26, compile and target SDK 36
- Libraries: `activity-compose` and Compose `ui`, `foundation` and `material3`, nothing else. No navigation library: the tab bar is a Material `NavigationBar`, and only the selected screen is composed.
- Content is fixed: nothing is random, no text depends on the time, and the app declares no `INTERNET` permission. The color scheme is Material's fixed light scheme (no dynamic color), the font scale is pinned to 1, and the system bar icons are dark in every system theme.

## Build

```sh
./build.sh
```

`build.sh` runs `./gradlew --no-daemon :app:assembleDebug`. All build output goes to stderr. stdout carries one line, the absolute path of `app/build/outputs/apk/debug/app-debug.apk`, so `apk="$(./build.sh)"` works. The script exits non-zero when the build fails, a Kotlin compile error included.

- `ANDROID_HOME` is the Android SDK root, default `$HOME/Library/Android/sdk`, exported for Gradle. The build needs `platforms/android-36` and `build-tools/36.0.0`. No `local.properties` is needed; one is git-ignored if present.
- `JAVA_HOME` is the JDK that runs Gradle, 17 or later (Temurin 21 on the reference machine). When it is unset, the script takes `java.home` from the `java` on PATH, a mise shim included, and fails with a message when there is none.

The first build downloads the Gradle distribution and the artifacts listed under [Dependencies](#dependencies) into the Gradle user home, `~/.gradle` by default. Gradle runs without a daemon and compiles Kotlin in its own process (`kotlin.compiler.execution.strategy=in-process` in `gradle.properties`), so a build leaves no Gradle or Kotlin daemon running. `.gradle/`, `.kotlin/`, `build/` and `app/build/` are generated and git-ignored.

AGP 9.4.1 triggers one Gradle deprecation warning, `Configuration.setVisible` (scheduled for removal in Gradle 11), from its own plugin code. The project's build scripts trigger none.

`build.sh` builds the debug variant. Compose runs slower in a debuggable build than in a minified release build, so timings of the app's own rendering include that overhead.

## Install and launch

```sh
apk="$(./build.sh)"
adb install -r "$apk"
adb shell am start -n dev.mobileagent.fixture.compose/.MainActivity
adb shell am force-stop dev.mobileagent.fixture.compose
```

Add `-s <serial>` to target one device. The app opens on the Login tab.

The string intent extra `initial_tab` (`login`, `list`, `big`, `anim` or `system`) selects the tab shown at launch; an absent or unknown value selects Login:

```sh
adb shell am start -S -n dev.mobileagent.fixture.compose/.MainActivity --es initial_tab big
```

The activity reads the extra when it is created. `-S` force-stops a running instance first; without it, `am start` delivers the intent to the running activity, which keeps its current tab. After a configuration change such as a rotation, the saved tab wins over the extra.

`adb install -r` keeps the location permission granted or denied. To return the app to never asked, so that the system dialog appears again:

```sh
pkg=dev.mobileagent.fixture.compose
for p in ACCESS_FINE_LOCATION ACCESS_COARSE_LOCATION; do
  adb shell pm revoke "$pkg" "android.permission.$p"
  adb shell pm clear-permission-flags "$pkg" "android.permission.$p" user-set user-fixed
done
```

Uninstalling the app with `adb uninstall dev.mobileagent.fixture.compose` before installing it resets the permission too.

## Identifier contract

Identifiers are Compose `testTag`s. The root `Scaffold` sets `Modifier.semantics { testTagsAsResourceId = true }`, so UI automation reports every tag verbatim as the node's resource id: `resource-id="cell-1"`, with no package prefix. The in-app dialog is a separate window with its own semantics tree and sets the flag again; without it, `inapp-alert-ok` has no resource id. Numbering is 1-based everywhere.

The Accessibility column is what `uiautomator dump` reports on the API 36 emulator.

| Screen | Identifier | Element | Accessibility | Content and behaviour |
|---|---|---|---|---|
| Tab bar | `tab-login`, `tab-list`, `tab-big`, `tab-anim`, `tab-system` | `NavigationBarItem` | clickable `View`, selectable; the label `Login`, `List`, `Big`, `Anim` or `System` is a descendant `TextView`. The selected tab is `selected="true"`, and Compose reports a selected tab as not clickable. | Switches the screen. |
| Login | `login-username` | `TextField` | `EditText` | Placeholder `Username`. Autocorrection and capitalization are off. |
| Login | `login-password` | `TextField` with `PasswordVisualTransformation` | `EditText`, text masked | Placeholder `Password`. |
| Login | `login-submit` | `Button` | clickable `View`; label `Sign in` on a child `TextView` | Sets `login-status`, clears focus and hides the keyboard. |
| Login | `login-status` | `Text` | `TextView` | Always present. Reads `Signed out` until the first submit, then `Welcome, <username>` when the password is `hunter2`, otherwise `Invalid credentials`. |
| List | `row-1` … `row-200` | clickable `Text` in a `LazyColumn` | leaf `android.widget.Button`, clickable, text `Row <n>` | Opens `detail-<n>`. Only rows near the viewport are composed, so the others are not in the tree until they are scrolled near. |
| List | `detail-<n>` | `Text` | heading `TextView`, text `Detail <n>` | Replaces the list while open. |
| List | `detail-back` | `Button` | clickable `View`; label `Back` on a child `TextView` | Returns to the list at its scroll position. System back does the same. |
| Big | `big-count` | `Text` | heading `TextView`, text `500 elements` | Not one of the 500 cells. |
| Big | `cell-1` … `cell-500` | clickable `Box` | leaf `android.widget.Button`, clickable, content description `Cell <n>` | See the 500-element contract below. |
| Anim | `anim-spinner` | `Box` | `View`, content description `Spinner` | Turns 360° every 1000 ms with `rememberInfiniteTransition`, linear, updated every display frame. |
| Anim | `anim-toggle` | toggleable `Row` holding the caption `Animate` and a Material `Switch` | leaf `View`, clickable, checkable, switch role, content description `Animate` | On at launch. Turning it off removes the transition, so frames stop and the spinner rests at 0°. |
| System | `system-permission` | `Button` | clickable `View`; label `Request location permission` on a child `TextView` | Requests `ACCESS_FINE_LOCATION` with `ActivityResultContracts.RequestPermission()`. On API 36 the system shows its location dialog (Precise or Approximate; While using the app, Only this time, Don’t allow) from the permission controller, a window outside the app. |
| System | `system-permission-status` | `Text` | `TextView` | `Permission: unknown` until a request returns, then `Permission: granted` when precise or approximate location was granted, otherwise `Permission: denied`, which includes dismissing the dialog with Back. The screen never reads the current grant on its own, so the initial text does not depend on earlier runs. |
| System | `system-alert` | `Button` | clickable `View`; label `Show in-app alert` on a child `TextView` | Opens a Material `AlertDialog` titled `In-app alert` with the text `This alert is presented by the app.` and one button. |
| System | `inapp-alert-ok` | `TextButton` in the dialog | clickable `View`; label `OK` on a child `TextView` | Closes the dialog. |

The contract gives the alert's OK button the identifier `inapp-alert-ok` where the platform allows it. Compose does, and this fixture's OK button carries it.

Compose builds the accessibility tree from the composable tree. For a Material component the tagged node is the clickable `android.view.View`: its label is a child `TextView`, its button role a child `android.widget.Button` node, and its own text is empty. Matching `login-submit` by identifier finds the clickable node; matching the text `Sign in` finds its child, whose centre lies inside the button. The rows, the cells and `anim-toggle` are single leaf nodes instead, with identifier, text or label, role and state on one node: a row is the tagged `Text` itself, and the cells and the toggle use `clearAndSetSemantics` placed after `clickable` or `toggleable` in the modifier chain, which keeps the action, role and state and drops the child nodes.

Android's documentation notes that some Android 12 releases ignore a request for `ACCESS_FINE_LOCATION` made without `ACCESS_COARSE_LOCATION`. On API 36 the fine-only request shows the dialog, verified on the reference emulator.

## Navigation and state

Only the selected tab's screen is composed, so the other tabs contribute no node to the accessibility tree. A `rememberSaveableStateHolder` at the root keeps each screen's saved state while its tab is away and restores it on return: the typed text, `login-status`, the list's scroll position and open detail, the animation toggle and `system-permission-status`. The same state survives a rotation.

## 500-element contract

On the Big tab:

- Exactly 500 nodes carry the identifiers `cell-1` to `cell-500`. Each is a leaf `android.widget.Button`, clickable, with the content description `Cell <n>`. The number drawn inside a cell is excluded from semantics and adds no node.
- `big-count` is a separate heading, `500 elements`, and is not one of the 500.
- All 500 cells are composed at once, eagerly, as 25 `Row`s of 20 in a non-lazy `Column` with `verticalScroll`.
- Compose reports only on-screen nodes to accessibility services. With the 500 cells stacked 48 dp tall in the scrolling column, `uiautomator dump` listed `cell-1` to `cell-15` and logged no skipped cell, so the app never reported the other 485. The grid is therefore sized to the viewport, width / 20 by height / 25 in whole pixels: every cell is on screen at once in either orientation, and the column never actually scrolls. On the 1080 × 2400, 420 dpi emulator each cell is 51 × 74 px.
- Cells are smaller than Compose's 48 dp minimum touch target. By default Compose expands each cell's touch area to 48 dp and reports the non-overlapping remainders as accessibility bounds: `cell-1` was reported 35 px wide at the screen's left edge instead of 51 px over its drawn cell. The grid provides a `ViewConfiguration` with a zero minimum, so each cell's bounds equal its drawn bounds.
- The tab bar adds five tab nodes. A full `uiautomator dump` of the tab has 541 nodes.

```sh
adb shell uiautomator dump /data/local/tmp/ui.xml
adb exec-out cat /data/local/tmp/ui.xml | grep -o 'resource-id="cell-' | wc -l   # 500
```

## Animation and settling

The spinner's rotation is applied in a graphics layer every frame, and its semantics never change, so it emits no accessibility events. `uiautomator dump` therefore succeeds on the Anim tab while the spinner turns; only the pixels never settle.

## Dependencies

| Component | Version | Notes |
|---|---|---|
| Gradle | 9.8.0 | `gradle-9.8.0-bin.zip`, pinned by `distributionSha256Sum` `bafd5ce9cfaea0fbccfdc8439a1ac42fbd4cd9c89dc9a988228d8a2639a58e6c` in `gradle/wrapper/gradle-wrapper.properties`. AGP 9.4 needs Gradle 9.6.0 or later; Gradle 9.8.0 is tested with AGP 9.0 to 9.5.0-alpha02. |
| Gradle wrapper | 9.8.0 | `gradle/wrapper/gradle-wrapper.jar` comes from tag `v9.8.0` of github.com/gradle/gradle, SHA-256 `238e777fcddd7e34f9708186085def2abd6e08e658505b38718d79d74c21abd5`, equal to the published `gradle-9.8.0-wrapper.jar.sha256` and to the jar that Gradle 9.8.0's `wrapper` task generates. `gradlew` and `gradlew.bat` come from that task; the tag's copies differ only in line endings and in the Gradle repository's own `-Dfile.encoding=UTF-8` default. |
| Android Gradle plugin | 9.4.1 | Built-in Kotlin: AGP compiles Kotlin, and `org.jetbrains.kotlin.android` is not applied. |
| Kotlin | 2.4.20 | Through the Compose compiler Gradle plugin 2.4.20, which depends on the Kotlin Gradle plugin of the same version. AGP 9.4.1 declares Kotlin Gradle plugin 2.2.10; Gradle resolves the higher version. |
| Compose BOM | 2026.06.01 | Compose 1.11.4, material3 1.4.0. The newest BOM that compiles against android-36: from 2026.08.00 (Compose 1.12) the AARs declare `minCompileSdk=37`. |
| activity-compose | 1.13.0 | |
| JDK | 17 or later | Temurin 21 on the reference machine. |
| Android SDK | platforms android-36, build-tools 36.0.0 | build-tools 36.0.0 is AGP 9.4.1's default. |

The build files name exactly these artifacts:

- plugin `com.android.application` 9.4.1: `com.android.tools.build:gradle:9.4.1`
- plugin `org.jetbrains.kotlin.plugin.compose` 2.4.20: `org.jetbrains.kotlin:compose-compiler-gradle-plugin:2.4.20`, which brings `org.jetbrains.kotlin:kotlin-gradle-plugin:2.4.20`
- `androidx.compose:compose-bom:2026.06.01`
- `androidx.activity:activity-compose:1.13.0`
- `androidx.compose.ui:ui:1.11.4`, `androidx.compose.foundation:foundation:1.11.4` and `androidx.compose.material3:material3:1.4.0`, versions from the BOM

Gradle resolves plugins and libraries from Google Maven, limited to the `androidx`, `com.android` and `com.google` groups, and from Maven Central; the Gradle Plugin Portal is not used. The APK's runtime classpath resolves to the 105 artifacts below: AndroidX and JetBrains artifacts (the Kotlin standard library, kotlinx-coroutines, kotlinx-serialization, annotations), plus `com.google.guava:listenablefuture:1.0` and `org.jspecify:jspecify:1.0.0`, which AndroidX depends on. `./gradlew -q :app:dependencies --configuration debugRuntimeClasspath` prints the tree. The build tooling adds AGP's own classpath of 113 artifacts (`./gradlew -q buildEnvironment`) and the tools that AGP and the Kotlin Gradle plugin fetch during the build, such as aapt2 and the Kotlin and Compose compilers.

<details>
<summary>APK runtime classpath, 105 artifacts</summary>

```
androidx.activity:activity:1.13.0
androidx.activity:activity-compose:1.13.0
androidx.activity:activity-ktx:1.13.0
androidx.annotation:annotation:1.9.1
androidx.annotation:annotation-experimental:1.4.1
androidx.annotation:annotation-jvm:1.9.1
androidx.arch.core:core-common:2.2.0
androidx.arch.core:core-runtime:2.2.0
androidx.autofill:autofill:1.0.0
androidx.collection:collection:1.5.0
androidx.collection:collection-jvm:1.5.0
androidx.collection:collection-ktx:1.5.0
androidx.compose:compose-bom:2026.06.01
androidx.compose.animation:animation:1.11.4
androidx.compose.animation:animation-android:1.11.4
androidx.compose.animation:animation-core:1.11.4
androidx.compose.animation:animation-core-android:1.11.4
androidx.compose.foundation:foundation:1.11.4
androidx.compose.foundation:foundation-android:1.11.4
androidx.compose.foundation:foundation-layout:1.11.4
androidx.compose.foundation:foundation-layout-android:1.11.4
androidx.compose.material:material-ripple:1.11.4
androidx.compose.material:material-ripple-android:1.11.4
androidx.compose.material3:material3:1.4.0
androidx.compose.material3:material3-android:1.4.0
androidx.compose.runtime:runtime:1.11.4
androidx.compose.runtime:runtime-android:1.11.4
androidx.compose.runtime:runtime-annotation:1.11.4
androidx.compose.runtime:runtime-annotation-android:1.11.4
androidx.compose.runtime:runtime-retain:1.11.4
androidx.compose.runtime:runtime-retain-android:1.11.4
androidx.compose.runtime:runtime-saveable:1.11.4
androidx.compose.runtime:runtime-saveable-android:1.11.4
androidx.compose.ui:ui:1.11.4
androidx.compose.ui:ui-android:1.11.4
androidx.compose.ui:ui-geometry:1.11.4
androidx.compose.ui:ui-geometry-android:1.11.4
androidx.compose.ui:ui-graphics:1.11.4
androidx.compose.ui:ui-graphics-android:1.11.4
androidx.compose.ui:ui-text:1.11.4
androidx.compose.ui:ui-text-android:1.11.4
androidx.compose.ui:ui-unit:1.11.4
androidx.compose.ui:ui-unit-android:1.11.4
androidx.compose.ui:ui-util:1.11.4
androidx.compose.ui:ui-util-android:1.11.4
androidx.concurrent:concurrent-futures:1.1.0
androidx.core:core:1.18.0
androidx.core:core-ktx:1.18.0
androidx.core:core-viewtree:1.0.0
androidx.customview:customview-poolingcontainer:1.0.0
androidx.documentfile:documentfile:1.0.0
androidx.dynamicanimation:dynamicanimation:1.0.0
androidx.emoji2:emoji2:1.4.0
androidx.graphics:graphics-path:1.0.1
androidx.interpolator:interpolator:1.0.0
androidx.legacy:legacy-support-core-utils:1.0.0
androidx.lifecycle:lifecycle-common:2.9.4
androidx.lifecycle:lifecycle-common-java8:2.9.4
androidx.lifecycle:lifecycle-common-jvm:2.9.4
androidx.lifecycle:lifecycle-livedata:2.9.4
androidx.lifecycle:lifecycle-livedata-core:2.9.4
androidx.lifecycle:lifecycle-livedata-core-ktx:2.9.4
androidx.lifecycle:lifecycle-process:2.9.4
androidx.lifecycle:lifecycle-runtime:2.9.4
androidx.lifecycle:lifecycle-runtime-android:2.9.4
androidx.lifecycle:lifecycle-runtime-compose:2.9.4
androidx.lifecycle:lifecycle-runtime-compose-android:2.9.4
androidx.lifecycle:lifecycle-runtime-ktx:2.9.4
androidx.lifecycle:lifecycle-runtime-ktx-android:2.9.4
androidx.lifecycle:lifecycle-viewmodel:2.9.4
androidx.lifecycle:lifecycle-viewmodel-android:2.9.4
androidx.lifecycle:lifecycle-viewmodel-ktx:2.9.4
androidx.lifecycle:lifecycle-viewmodel-savedstate:2.9.4
androidx.lifecycle:lifecycle-viewmodel-savedstate-android:2.9.4
androidx.loader:loader:1.0.0
androidx.localbroadcastmanager:localbroadcastmanager:1.0.0
androidx.navigationevent:navigationevent:1.0.0
androidx.navigationevent:navigationevent-android:1.0.0
androidx.navigationevent:navigationevent-compose:1.0.0
androidx.navigationevent:navigationevent-compose-android:1.0.0
androidx.print:print:1.0.0
androidx.profileinstaller:profileinstaller:1.4.0
androidx.savedstate:savedstate:1.3.2
androidx.savedstate:savedstate-android:1.3.2
androidx.savedstate:savedstate-compose:1.3.2
androidx.savedstate:savedstate-compose-android:1.3.2
androidx.savedstate:savedstate-ktx:1.3.2
androidx.startup:startup-runtime:1.1.1
androidx.tracing:tracing:1.2.0
androidx.transition:transition:1.6.0
androidx.versionedparcelable:versionedparcelable:1.1.1
androidx.window:window:1.5.0
androidx.window:window-core:1.5.0
androidx.window:window-core-android:1.5.0
com.google.guava:listenablefuture:1.0
org.jetbrains:annotations:23.0.0
org.jetbrains.kotlin:kotlin-stdlib:2.4.20
org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0
org.jetbrains.kotlinx:kotlinx-coroutines-bom:1.9.0
org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0
org.jetbrains.kotlinx:kotlinx-coroutines-core-jvm:1.9.0
org.jetbrains.kotlinx:kotlinx-serialization-bom:1.7.3
org.jetbrains.kotlinx:kotlinx-serialization-core:1.7.3
org.jetbrains.kotlinx:kotlinx-serialization-core-jvm:1.7.3
org.jspecify:jspecify:1.0.0
```

</details>

## Downloads and Socket Firewall

Every package install in this repository goes through Socket Firewall (`sfw`). `sfw` cannot wrap Gradle or its Maven resolution, so the Gradle distribution from services.gradle.org, the wrapper files from GitHub, and every artifact from Google Maven (`dl.google.com`) and Maven Central are downloaded without Socket scanning. They are this fixture's one class of unwrapped downloads. That is why the dependencies stay within AndroidX, the Compose BOM and Kotlin, every version is exact, the distribution is pinned by SHA-256, and the wrapper jar's SHA-256 was checked against Gradle's published checksum.
