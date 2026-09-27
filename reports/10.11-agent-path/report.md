# Benchmark report

Started 2026-09-27T20:28:12Z, finished 2026-09-27T20:35:43Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 3.66 3.83 4.14 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | b32ea8c1ab2ed1ae1f5671e40bb1c5109ed635e2 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| emulator-5554 | android-emulator | sdk_gphone64_arm64 | Android API 36 | 36 | off | no |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | emulator-5554 | — | — | cli-start | wall | 50 | 4.173 | 4.651 | 4.886 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | cold-attach | wall | 50 | 775.368 | 816.536 | 842.037 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | big | first-snapshot | wall | 50 | 31.165 | 42.362 | 44.497 | 0.0 | 0.0 | 506 |
| mobile-agent | emulator-5554 | expo | big | warm-snapshot | wall | 50 | 6.088 | 7.473 | 7.912 | 0.0 | 0.0 | 506 |
| mobile-agent | emulator-5554 | expo | list | tap-dispatch | tool | 50 | 4.369 | 5.325 | 6.082 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | tap-to-frame-change | — | 0 | — | — | — | — | — | — |
| mobile-agent | emulator-5554 | expo | list | tap-to-tree-delta | tool | 50 | 60.548 | 67.177 | 71.127 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | login | text-entry | wall | 50 | 23.728 | 40.345 | 51.835 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | big | screenshot | wall | 50 | 31.334 | 36.093 | 49.204 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | settle | tool | 50 | 271.860 | 284.962 | 500.331 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow login | wall | 50 | 1205.004 | 1279.486 | 1300.999 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow big-screen | wall | 50 | 1781.284 | 1877.297 | 1958.170 | 0.0 | 0.0 | 506 |

Series that could not run:

- mobile-agent on emulator-5554, flow list-scroll: the check run failed: mobile-agent wait --timeout 10000 --text=Detail 200: wait_timeout: waited 10001 ms for text "Detail 200" to appear; the screen now shows revision 22, 2 windows, 22 elements with refs, read 10433 ms ago: / the check run failed: mobile-agent wait --timeout 10000 --text=Detail 200: wait_timeout: waited 10004 ms for text "Detail 200" to appear; the screen now shows revision 43, 2 windows, 22 elements with refs, read 538 ms ago:

Series not measured:

- mobile-agent on emulator-5554, tap-to-frame-change: emulator-5554 gives mobile-agent no frames, so a tap on it has no frame change to time

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| emulator-5554 | — | — | cli-start | 4.173 / 4.651 |
| emulator-5554 | expo | — | cold-attach | 775.368 / 816.536 |
| emulator-5554 | expo | big | first-snapshot | 31.165 / 42.362 |
| emulator-5554 | expo | big | warm-snapshot | 6.088 / 7.473 |
| emulator-5554 | expo | list | tap-dispatch | 4.369 / 5.325 |
| emulator-5554 | expo | list | tap-to-frame-change | not measured |
| emulator-5554 | expo | list | tap-to-tree-delta | 60.548 / 67.177 |
| emulator-5554 | expo | login | text-entry | 23.728 / 40.345 |
| emulator-5554 | expo | big | screenshot | 31.334 / 36.093 |
| emulator-5554 | expo | list | settle | 271.860 / 284.962 |
| emulator-5554 | expo | — | flow login | 1205.004 / 1279.486 |
| emulator-5554 | expo | — | flow list-scroll | no samples |
| emulator-5554 | expo | — | flow big-screen | 1781.284 / 1877.297 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| emulator-5554 | mobile-agent | end of the run | 64.055 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-emulator cli-start | mobile-agent | emulator-5554 | p95 | 20.000 | 4.651 | pass |
| android-emulator cold-attach | mobile-agent | emulator-5554 | p95 | 1500.000 | 816.536 | pass |
| android-emulator warm-snapshot big | mobile-agent | emulator-5554 | p95 | 100.000 | 7.473 | pass |
| android-emulator tap-to-frame-change | — | — | p95 | 50.000 | — | not measured |

All 3 gate checks passed. 1 not measured.


## Comparison with 10.1 run 2

This section was added after the run; the rest of this file and `report.json` are as the bench wrote them, and neither names a home directory, so nothing was rewritten to `$HOME`.

Run 2 of task 10.1 (`reports/10.1-emulator-agent/`, 15:33 to 16:20 UTC) measured the agent path on an emulator started with `-ports 5554,5555` and no `-grpc`, which leaves the controller unserved, at commit 05a4756 through the 10.1 CLI shim, with both fixtures. This run (20:28 to 20:35 UTC) measured it on the same emulator started with `-grpc 8554 -grpc-use-token`, the controller served and left unused by `MOBILE_AGENT_EMULATOR_CONTROLLER=off` (task 10.12), at commit b32ea8c (tasks 10.11 and 10.12), with the Expo fixture only:

```sh
MOBILE_AGENT_DRIVERS=android MOBILE_AGENT_EMULATOR_CONTROLLER=off \
  target/release/mobile-agent-bench run --target emulator-5554 --fixture expo \
  --no-baselines --cli target/release/mobile-agent --out <a scratch directory>
```

The Expo flows, 50 measured iterations after 3 warm-up, 1 retry:

| Flow | Run 2 fail % | Run 2 flake % | Run 2 p50 / p95 ms | This run fail % | This run flake % | This run p50 / p95 ms |
|---|---|---|---|---|---|---|
| login | 0.0 | 6.0 (3 of 50) | 977.609 / 1033.670 | 0.0 | 0.0 | 1205.004 / 1279.486 |
| list-scroll | 100.0 (check run failed) | 0.0 | no samples | 100.0 (check run failed) | 0.0 | no samples |
| big-screen | 92.0 (46 of 50) | 2.0 (1 of 50) | 1590.111 / 1603.563 (4 passed) | 0.0 | 0.0 | 1781.284 / 1877.297 |

- **big-screen, 46 of 50 failures to none.** Run 2's `open` reported the Expo screen settled while the platform still reported the app's window at one frame of its launch animation (scaled over the launcher, slid in from the right over another app), so the click that followed aimed at where the Big tab was in that frame and the wait for `cell-500` timed out. `open` now waits until the window is at rest (the agent's `transformed` window flag, design D8), and the click lands on the tab in every iteration.
- **login, 3 flakes in 50 to none.** Run 2's three failed first attempts timed out waiting for `Welcome, agent` after the Sign in click, as a click aimed at a frame of the launch animation would; this run retried nothing.
- **Both flows take longer, by 227 ms (login) and 191 ms (big-screen) at p50:** the time `open` now waits for the launch animation to end and a quiet window to pass. Measured on its own, `open` on the agent path ends after 1,050 ms p50 over the launcher and 1,255 ms over another app, against 787 and 768 ms before, with the screen at rest in 20 of 20 opens against 1 of 20 (task 10.11's commit).
- **list-scroll is unchanged: its check run failed on both attempts,** each at `wait --text=Detail 200` after `click #row-200` had succeeded, the screen still showing the list's last rows (22 elements with refs); in the first attempt nothing had been read for the 10 s after the click. The same steps with the same CLI, daemon and emulator, run by hand right after this run (`open --relaunch`, `click #tab-list`, `wait --settle` and a check for `#row-200` before each `scroll -- down 600`, `click #row-200`, `wait --text=Detail 200`), reached `row-200` in 8 scroll steps at y 786.3 and opened Detail 200 in 8 of 8 tries, 3 of them with a fresh daemon; the cause was not found. In run 2 one attempt failed the same way and the other with `row-200` not visible after 50 scroll steps.
- **The primitives** are within a few milliseconds of run 2's, except the list screen's settle, 5.5 ms slower at p50 (271.860 against 266.395 ms): a settle without frames now ends with one bounds probe, 4.5 ms p50. `tap-to-frame-change` is not measured, the target giving no frames, where run 2 counted it as a failed gate.
