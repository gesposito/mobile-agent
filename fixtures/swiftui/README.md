# FixtureSwiftUI

A SwiftUI iOS app with no dependencies that implements the shared fixture contract in [`../CONTRACT.md`](../CONTRACT.md). The benchmark harness and the Simulator spikes use it as a deterministic target: fixed labels, no randomness, no network, no time-dependent text.

- Bundle identifier `dev.mobileagent.fixture.swiftui`, one app target `FixtureSwiftUI`, deployment target iOS 17.0, iPhone only, portrait, light appearance.
- The project uses the Xcode 16 file format (`objectVersion = 77`) so every Xcode in the CI matrix can open it. Built and verified with Xcode 27.0 (27A266a) on an iPhone 17 Simulator running iOS 26.5.
- `FixtureSwiftUI/` is a file-system-synchronized group: a Swift file added there joins the target without editing `project.pbxproj`.

## Build

```sh
./build.sh
```

Builds the Debug configuration for `generic/platform=iOS Simulator` into `build/`, without code signing. xcodebuild's log goes to stderr; stdout carries only the absolute path of the `.app`, printed as the last line. The script exits non-zero when the build fails.

```sh
APP="$(./build.sh)"
```

Debug builds use Xcode's debug dylib layout: the `FixtureSwiftUI` executable is a stub that loads `FixtureSwiftUI.debug.dylib`.

## Install and launch

```sh
xcrun simctl install booted "$APP"
xcrun simctl launch booted dev.mobileagent.fixture.swiftui
xcrun simctl terminate booted dev.mobileagent.fixture.swiftui
```

Use a Simulator UDID instead of `booted` when more than one Simulator is booted.

## Identifier contract

Identifiers are set with `.accessibilityIdentifier(...)`, or `accessibilityIdentifier` on the tab bar items. Numbering is 1-based. The element column is what XCTest reports on iOS 26.5.

### Tabs

| Identifier | Label | Element |
| --- | --- | --- |
| `tab-login` | Login | tab bar button, selected at launch |
| `tab-list` | List | tab bar button |
| `tab-big` | Big | tab bar button |
| `tab-anim` | Anim | tab bar button |
| `tab-system` | System | tab bar button |

The tab bar is a UIKit `UITabBarController` hosting one SwiftUI screen per tab. Its items get their identifiers when they are created, so the identifiers are in the first snapshot after launch. A SwiftUI `TabView` attaches them only in a later update: on iOS 26.5 they appeared 0.3 to 1 s after launch or, depending on how the tree was queried, not within 7 s.

### Login

| Identifier | Element | Contract |
| --- | --- | --- |
| `login-username` | text field | placeholder "Username"; autocapitalization and autocorrection are off, so typed text arrives verbatim |
| `login-password` | secure text field | placeholder "Password" |
| `login-submit` | button "Sign in" | submitting dismisses the keyboard |
| `login-status` | text | "Signed out" before the first submit, then "Welcome, <username>" when the password is `hunter2`, otherwise "Invalid credentials" |

### List

| Identifier | Element | Contract |
| --- | --- | --- |
| `row-<n>` | button "Row <n>", n = 1 to 200 | only rows on screen are in the tree (16 on an iPhone 17); scroll to reach the rest |
| `detail-<n>` | text "Detail <n>" | shown after tapping `row-<n>` |
| `detail-back` | button "Back" | returns to the list; the navigation bar is hidden, so this is the only back control |

### Big

| Identifier | Element | Contract |
| --- | --- | --- |
| `big-count` | header text "500 elements" | not one of the cells |
| `cell-<n>` | button "Cell <n>", n = 1 to 500 | all 500 in the tree at once |

### Anim

| Identifier | Element | Contract |
| --- | --- | --- |
| `anim-spinner` | element labelled "Spinner", value "Spinning" or "Stopped" | rotates continuously while animating |
| `anim-toggle` | switch labelled "Animate", value 1 or 0 | on at launch; off stops the animation. The element is the switch alone, so a tap at its center flips it |

### System

| Identifier | Element | Contract |
| --- | --- | --- |
| `system-permission` | button "Request location permission" | requests when-in-use location access; the system shows its alert only while the status is unknown |
| `system-permission-status` | text | "Permission: unknown", "Permission: granted" or "Permission: denied" |
| `system-alert` | button "Show in-app alert" | shows an alert titled "In-app alert" with message "This alert is presented by the app." |
| `inapp-alert-ok` | the alert's "OK" button | SwiftUI can set this identifier, so it is present |

The location alert belongs to system UI, not the app: XCTest finds it under SpringBoard, titled "Allow “FixtureSwiftUI” to use your location?" with the buttons "Allow Once", "Allow While Using App" and "Don’t Allow", none of which carries an identifier. The permission persists in the Simulator, and `system-permission-status` reads it from the system, so it shows "Permission: unknown" only while authorization is not determined. Reset it to see the alert again:

```sh
xcrun simctl privacy booted reset location dev.mobileagent.fixture.swiftui
```

On iOS 26.5, XCTest reports the in-app alert's OK action as a button nested in a button, both carrying `inapp-alert-ok` and the same frame. That is UIKit's alert structure; the app defines one button.

## Navigation and state

- Only the selected tab's screen is in the accessibility tree; the tab bar controller keeps only that screen's view in the window.
- Screen state persists across tab switches: typed text, `login-status`, the list's scroll position and any open detail, the animation toggle and `system-permission-status` are unchanged when you leave a tab and return.
- The keyboard covers the tab bar. Submitting dismisses it, and so does Return in the username field, which does not submit.

## 500-element contract

- The Big screen has exactly 500 cell elements, `cell-1` to `cell-500`, each a button labelled "Cell <n>".
- The cells sit in a 20 by 25 `LazyVGrid` sized to fit the screen without scrolling. A lazy grid creates only the cells it renders, so fitting the screen is what puts all 500 in the tree at once.
- `big-count` ("500 elements") is a separate header and is not one of the 500.
- The tree also holds the app's containers and the tab bar with its 5 buttons and their images. On an iPhone 17 with iOS 26.5, XCTest returns 546 nodes for this screen, of which 500 are cells. Count cells by the `cell-` identifier prefix rather than by total nodes, because container counts are the platform's.
- A tap on a cell only highlights it while pressed; the tree is the same before and after the tap.

## Animation and settling

- The spinner draws its rotation inside a fixed-size canvas, so while it turns only pixels change: its accessibility frame and value stay constant, and a tree-only settle signal settles while the frame signal never does.
- It turns only while the Anim tab is on screen, so the other tabs settle.
- The spinner is redrawn on SwiftUI's animation timeline schedule, which lets the system pick the update interval. Spike S2 measures the resulting frame rate.

## Determinism

- The appearance is fixed to light and the text size to Large, so host settings do not change pixels or layout. The app runs in portrait on iPhone only.
