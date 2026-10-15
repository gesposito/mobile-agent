# Benchmark report

Started 2026-09-28T07:47:02Z, finished 2026-09-28T08:06:02Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 1.92 3.06 3.53 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 202bb4d529c0823d2e0215faca4a07043624af72 (clean) |
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
| mobile-agent | emulator-5554 | — | — | cli-start | wall | 50 | 4.328 | 4.928 | 6.008 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | cold-attach | wall | 50 | 792.379 | 997.390 | 1128.018 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | big | first-snapshot | wall | 50 | 23.793 | 33.529 | 38.207 | 0.0 | 0.0 | 506 |
| mobile-agent | emulator-5554 | expo | big | warm-snapshot | wall | 50 | 6.324 | 8.273 | 9.009 | 0.0 | 0.0 | 506 |
| mobile-agent | emulator-5554 | expo | list | tap-dispatch | tool | 50 | 4.107 | 4.843 | 5.292 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | tap-to-frame-change | — | 0 | — | — | — | — | — | — |
| mobile-agent | emulator-5554 | expo | list | tap-to-tree-delta | tool | 50 | 61.953 | 73.179 | 94.383 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | login | text-entry | wall | 50 | 17.138 | 33.888 | 39.201 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | big | screenshot | wall | 50 | 31.805 | 40.869 | 47.149 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | settle | tool | 50 | 269.139 | 274.913 | 275.593 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow login | wall | 50 | 1209.863 | 1264.800 | 1360.476 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow list-scroll | wall | 50 | 13581.014 | 13718.504 | 14019.021 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow big-screen | wall | 50 | 1782.911 | 1877.168 | 1969.768 | 0.0 | 0.0 | 506 |

Series not measured:

- mobile-agent on emulator-5554, tap-to-frame-change: emulator-5554 gives mobile-agent no frames, so a tap on it has no frame change to time

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| emulator-5554 | — | — | cli-start | 4.328 / 4.928 |
| emulator-5554 | expo | — | cold-attach | 792.379 / 997.390 |
| emulator-5554 | expo | big | first-snapshot | 23.793 / 33.529 |
| emulator-5554 | expo | big | warm-snapshot | 6.324 / 8.273 |
| emulator-5554 | expo | list | tap-dispatch | 4.107 / 4.843 |
| emulator-5554 | expo | list | tap-to-frame-change | not measured |
| emulator-5554 | expo | list | tap-to-tree-delta | 61.953 / 73.179 |
| emulator-5554 | expo | login | text-entry | 17.138 / 33.888 |
| emulator-5554 | expo | big | screenshot | 31.805 / 40.869 |
| emulator-5554 | expo | list | settle | 269.139 / 274.913 |
| emulator-5554 | expo | — | flow login | 1209.863 / 1264.800 |
| emulator-5554 | expo | — | flow list-scroll | 13581.014 / 13718.504 |
| emulator-5554 | expo | — | flow big-screen | 1782.911 / 1877.168 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| emulator-5554 | mobile-agent | end of the run | 78.660 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-emulator cli-start | mobile-agent | emulator-5554 | p95 | 20.000 | 4.928 | pass |
| android-emulator cold-attach | mobile-agent | emulator-5554 | p95 | 1500.000 | 997.390 | pass |
| android-emulator warm-snapshot big | mobile-agent | emulator-5554 | p95 | 100.000 | 8.273 | pass |
| android-emulator tap-to-frame-change | — | — | p95 | 67.000 | — | not measured |

All 3 gate checks passed. 1 not measured.


## Comparison with the 10.11 run

This section was added after the run; the rest of this file and `report.json` are as the bench wrote them, and neither names a home directory, so nothing was rewritten to `$HOME`.

Task 10.11's agent-path run (`reports/10.11-agent-path/`, 2026-09-27, 20:28 to 20:35 UTC, commit ce96be4) and this one (2026-09-28, 07:47 to 08:06 UTC, commit 202bb4d, tasks 10.13 and 10.14) measured emulator-5554 started with `-gpu host -grpc 8554 -grpc-use-token`, its controller served and left unused by `MOBILE_AGENT_EMULATOR_CONTROLLER=off`, with the Expo fixture only, through the same command (this run's daemon kept its socket in a directory of its own, through `MOBILE_AGENT_SOCKET_DIR`):

```sh
MOBILE_AGENT_DRIVERS=android MOBILE_AGENT_EMULATOR_CONTROLLER=off \
  target/release/mobile-agent-bench run --target emulator-5554 --fixture expo \
  --no-baselines --cli target/release/mobile-agent --out <a scratch directory>
```

The Expo flows, 50 measured iterations after 3 warm-up, 1 retry:

| Flow | 10.11 fail % | 10.11 p50 / p95 ms | This run fail % | This run flake % | This run p50 / p95 ms |
|---|---|---|---|---|---|
| login | 0.0 | 1205.004 / 1279.486 | 0.0 | 0.0 | 1209.863 / 1264.800 |
| list-scroll | 100.0 (check run failed) | no samples | 0.0 | 0.0 | 13581.014 / 13718.504 |
| big-screen | 0.0 | 1781.284 / 1877.297 | 0.0 | 0.0 | 1782.911 / 1877.168 |

- **list-scroll passes: its check run and all 53 iterations, each on its first attempt,** each reaching `row-200` in 8 scroll steps. In 10.11 both attempts of the check run timed out waiting for `Detail 200` after a successful click on `row-200`. The flow's last stroke pulls the list against its end, where Android 12 and later draw a stretch that springs back with no event: 333 to 383 ms after the finger lifted, in `dumpsys gfxinfo` frame stats on this emulator, and until it rests the list takes a touch that comes down as the start of a drag. Reproduced with the task's own runs, the clicks sent 307 to 353 ms after that stroke went to the centre of `row-200` and opened nothing, and a daemon started afresh still read the list (task 10.14, design D8). A settle without frames now does not end until 500 ms have passed since a stroke that leaves the content under it at an end of its range and since the tree's last change, so the click comes after the stretch has come to rest.
- **The flow's 13.6 s at p50 includes that hold once:** in the task's own runs the settle after the last stroke took 644 to 679 ms, against 290 to 378 ms before, and the click came 657 to 701 ms after the stroke.
- **login and big-screen are within 5 ms of 10.11's at p50;** at p95 login is 14.7 ms lower and big-screen the same.
- **The primitives are within 3 ms of 10.11's at p50,** except first-snapshot (23.793 ms against 31.165), text-entry (17.138 against 23.728) and cold attach (792.379 against 775.368; at p95 997.390 against 816.536, under its 1,500 ms gate). The tap-to-frame-change gate reads 67 ms since task 10.13 and is not measured, the target giving no frames.
