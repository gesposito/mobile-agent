# Fixture identifier contract

Every reference fixture (SwiftUI, Expo, Compose) implements exactly this, so the benchmark harness and the spikes run identical flows on all three. Identifiers are `accessibilityIdentifier` on SwiftUI, `testID` on Expo, and `testTag` with `testTagsAsResourceId` on Compose. Numbering is 1-based everywhere. Labels are exact strings.

## Tabs
- `tab-login` "Login", `tab-list` "List", `tab-big` "Big", `tab-anim` "Anim", `tab-system` "System". Login is selected at launch.

## Login
- `login-username` text field, placeholder "Username"
- `login-password` secure field, placeholder "Password"
- `login-submit` button, label "Sign in"
- `login-status` text, always present: "Signed out" before the first submit, then "Welcome, <username>" when the password is `hunter2`, otherwise "Invalid credentials". Submitting dismisses the keyboard.

## List
- 200 rows `row-1` … `row-200`, text "Row <n>", each actionable
- tapping a row shows `detail-<n>` with text "Detail <n>" and a button `detail-back` labelled "Back"

## Big
- header `big-count` text "500 elements", not counted among the cells
- exactly 500 actionable cells `cell-1` … `cell-500`, label "Cell <n>", button role, all present in the accessibility tree at once (no virtualization); a 20-column grid that fits on one iPhone screen is preferred

## Anim
- `anim-spinner` rotating endlessly at the display refresh rate while animating
- `anim-toggle` switch labelled "Animate", on at launch; off stops the animation

## System
- `system-permission` button, label "Request location permission", triggers the real location when-in-use system prompt
- `system-permission-status` text: "Permission: unknown" at launch, then "Permission: granted" or "Permission: denied"
- `system-alert` button, label "Show in-app alert", shows an in-app alert titled "In-app alert" with message "This alert is presented by the app." and one button labelled "OK"
- the OK button carries identifier `inapp-alert-ok` where the platform allows it (SwiftUI, Compose). React Native's Alert cannot set one; drivers locate it by its "OK" label. Fixture READMEs state which applies.

## Determinism
- no randomness, no network, no time-dependent text, no external dependencies beyond what each fixture's README lists
