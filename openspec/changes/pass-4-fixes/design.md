# Design

## Context

Three changes fixed what the passes on real apps found (`real-app-fixes`, `action-verdicts`, `android-pass-fixes`), each proven by the next pass; the fourth pass left iOS with no failed call and found nine new gaps with the site's recording runs (the private demo notes of 2026-10-30 and the site's recording report; `reports/12.5-fourth-pass.md` is this change's public record). Two of the gaps came one run in five or three runs in a prep, which is the shape of a host effect as much as a tool's; the passes have only ever run on one Mac, under the load of its other sessions. CI runs the fakes alone: the device-backed tests are gated on `MOBILE_AGENT_SIM_UDID` and `MOBILE_AGENT_ANDROID_SERIAL`, which `ci.yml` never sets, while `benchmarks.yml` creates a fresh iPhone 17 Simulator on the runner's iOS 27.0 runtime and boots the API 36 emulator for the harness. The first CI harness run (41181272250) showed the fixture's list screen failing every tool under iOS 27, a version this Mac does not run.

## Goals / Non-Goals

Goals: every finding reproduced where it can be, on CI's fresh devices or on the fakes, before a fix; each fix gated on its case; a public record of the pass and the known issues. Non-Goals: per-pull-request device tests (40 minutes of macOS runner per run); fixes for the apps' own behavior (Calendar's node, Chrome's first run) beyond naming them; the noise study of the benchmarks (ci-benchmarks task 2.1).

## Decisions

### D1. Device-backed tests run in CI, on fresh devices, on a schedule and on demand
A workflow `device-tests.yml` provisions what `benchmarks.yml`'s harness jobs provision (the Simulator created, booted and given the SwiftUI fixture on the macOS runner; the emulator through `reactivecircus/android-emulator-runner` with the Compose fixture on Linux), then runs the workspace's tests with `MOBILE_AGENT_SIM_UDID` and `MOBILE_AGENT_ANDROID_SERIAL` set, so that the gated tests run, and a new gated test module on the default apps (`crates/daemon/tests/real_apps.rs`: Settings, Maps, Calendar, Photos on the Simulator; Settings, Clock, Chrome on the emulator), one test per finding of this change. Every assertion is on what the device guarantees, the reply's fields and the listing, with generous timeouts and no fixed number of milliseconds, since the runners coalesce timers (the wall-clock lesson of the timing fixes of 2026-10-30). The job runs weekly, off the hour, and on dispatch; the summary lists each test's verdict. Prior art: `docs/research/ci-benchmarks.md` on the runners, the emulator runner's documentation, the runner images' Simulator runtimes. Triage protocol: a finding whose test is red on CI is the tool's and its task fixes it; a finding whose test is green on CI and red only on this Mac is the host's, recorded in the README's known issues with the condition it needs (load, a Simulator not booted by the driver); the record goes in "As found" below.

### D2. A refused launch is retried once
`simctl launch` right after an app's termination can fail with `NSPOSIXErrorDomain 3: Simulator device failed to launch <bundle>`, ten times across two runs, a retry after 3 s succeeding every time. The driver retries exactly once, after a bounded wait measured on CI's Simulator and this Mac as the shortest that succeeds (start from 500 ms, double to 3 s), gated on that error alone; the reply says it retried and how long it waited; a second refusal fails as today. No general retry of launches.

### D3. The read-back waits for the field
`type`'s read-back came one keystroke early once in five runs: the field was read while the last key was in flight. After the keys, the field is read until two consecutive reads agree, or the value holds the text, within a bound of the keys' own time plus a margin, measured on the fake and the Simulator; the line says what it read, and when the read-back is shorter than the text it warns `shorter than typed` instead of ticking. The fake gets a typing mode whose last key lands after the first read, which makes the test red first.

### D4. A settle that gives up says so
A settle that stops at its timeout prints ⚠ and exits 0, by the convention of `action-verdicts` (a spinner that keeps turning is not a failure for a model, and a failed call would make it retry). Scripts need to tell the two apart: the reply carries `settled: true|false` and, when false, `stoppedAfterMs` and the reason; the exit status stays 0 unless `--strict` is given, which fails `settle_timeout` in the wait's timeout family. The server's tool keeps the warning and gains the field.

### D5. A tap by coordinates names the labeled element under the point
`hit generic (no ref)` tells the model nothing. The hit's name is the deepest node under the point that has a name, an identifier or a role other than generic; failing that, the nearest labeled ancestor; failing that, the text under the point; `generic` only when nothing under the point is labeled. The reply's `hit` keeps its fields; the line names what it names.

### D6. The listing after a system window closes
Right after an alert's dismissal, `snapshot` listed no interactive element and the settle said nothing changed, while the full listing held the buttons: the replica still carried the alert's cover over the application window after the alert left, or the read after the dismissal predated it. The mirror re-reads the application's windows when a system window leaves the replica, through the catch-up of `profiling` (`Mirror::reread_now`), and the interactive listing never lists a cover that is gone; the fake gets an alert whose dismissal is scripted, which makes the test red first; CI's fresh Simulator reproduces it on Photos' first launch.

### D7. A stale ref names the dialog in front
When a stale ref has no twin and `Mirror::system_dialog` holds a dialog, `stale_ref` names the dialog and says to dismiss it; without a dialog the failure is as today.

### D8. A text wait prefers the whole text
`wait --text Lisbon` resolved on "Lisbon Airport (LIS)". Among the elements whose text matches, one whose text is the whole sought text (equal, case and whitespace aside) wins over one whose text merely contains it; ties keep the listing's order; `--text` keeps matching substrings so that a wait for a word still resolves when no element holds it alone.

### D9. The device findings are sorted on CI first
(a) The header cover: Maps' search field's clear button was refused as under the header while in plain view. The rule "a bar or a pinned header covers what lies within its bounds" must not cover the header's own descendants: a control the header holds is reachable; the test is the fixture's and Maps' on CI. (b) The time picker's overlay: Material's dial overlay has no refs, so a click on a ref under it fails `element_covered`; the mirror lists the overlay's touch targets with refs, or the click says which control of the overlay to use; decided on the emulator's Clock on CI. (c) Calendar's kept node: a full read right after the deletion tells the replica's eviction from the app's own node; the replica's is fixed, the app's recorded. (d) The list screen under iOS 27: the reader's tree and the fixture's List and tab bar read on CI's Simulator and compared with iOS 26.5; the fixture is fixed if its accessibility changed, the reader if its traversal did. (e) The click's change window (Photos 486 ms) and Chrome's not-responding dialogs on a just-booted emulator: measured on CI; recorded as the host's if they do not reproduce.

## Risks / Trade-offs

- The runners' timers coalesce and their load is shared: every device test asserts guarantees, never milliseconds, and the job is not a merge gate.
- A fresh Simulator shows first-launch sheets and permission alerts the tests must dismiss; that is also where D6 reproduces.
- Maps' search and Chrome's first page need the runner's network; a test that cannot reach it is skipped with a reason, not failed.
- `--strict` on settle is one more flag; the default stays the model's convention.
- Fixes to the fixture for iOS 27 must keep iOS 26.5's behavior, which this Mac runs.
