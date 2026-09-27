# Benchmark report

Started 2026-09-27T15:33:07Z, finished 2026-09-27T16:20:56Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 1.17 2.07 3.32 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 05a47564f0b872d7f937f6df8b060b615e349cd5 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level |
|---|---|---|---|---|
| emulator-5554 | android-emulator | sdk_gphone64_arm64 | Android API 36 | 36 |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |
| maestro | baseline | skipped | — | left out with --no-baselines: the emulator's controller flag does not change its flows, so the numbers of Maestro 2.10.0 in reports/10.1-emulator-grpc stand for this run |
| maestro-runner | baseline | skipped | — | left out with --no-baselines, and not installed on this host (reports/10.1-emulator-grpc lists it as not found on PATH) |
| agent-device | baseline | skipped | — | left out with --no-baselines: the emulator's controller flag does not change its flows, so the numbers of agent-device 0.21.14 in reports/10.1-emulator-grpc stand for this run |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | emulator-5554 | — | — | cli-start | wall | 50 | 7.571 | 9.053 | 14.216 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | cold-attach | wall | 50 | 778.517 | 927.563 | 1089.121 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | big | first-snapshot | wall | 50 | 31.573 | 792.390 | 930.950 | 0.0 | 0.0 | 218–506 |
| mobile-agent | emulator-5554 | expo | big | warm-snapshot | wall | 50 | 11.462 | 13.162 | 28.588 | 0.0 | 0.0 | 506 |
| mobile-agent | emulator-5554 | expo | list | tap-dispatch | tool | 50 | 4.089 | 5.068 | 5.196 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | tap-to-frame-change | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | tap-to-tree-delta | tool | 50 | 57.767 | 67.714 | 71.097 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | login | text-entry | wall | 50 | 27.986 | 43.946 | 58.432 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | big | screenshot | wall | 50 | 35.279 | 44.031 | 47.857 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | settle | tool | 50 | 266.395 | 272.329 | 336.061 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow login | wall | 50 | 977.609 | 1033.670 | 1462.497 | 0.0 | 6.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow big-screen | wall | 4 | 1590.111 | 1603.563 | 1603.563 | 92.0 | 2.0 | 506 |
| mobile-agent | emulator-5554 | compose | — | cold-attach | wall | 50 | 744.911 | 880.058 | 1030.142 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | big | first-snapshot | wall | 50 | 35.902 | 44.336 | 49.246 | 0.0 | 0.0 | 504 |
| mobile-agent | emulator-5554 | compose | big | warm-snapshot | wall | 50 | 10.609 | 11.301 | 17.134 | 0.0 | 0.0 | 504 |
| mobile-agent | emulator-5554 | compose | list | tap-dispatch | tool | 50 | 5.069 | 6.747 | 8.611 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | list | tap-to-frame-change | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | list | tap-to-tree-delta | tool | 50 | 222.845 | 231.750 | 293.518 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | login | text-entry | wall | 50 | 23.481 | 46.027 | 47.366 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | big | screenshot | wall | 50 | 35.758 | 39.626 | 49.126 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | list | settle | tool | 50 | 188.458 | 196.565 | 409.803 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow login | wall | 50 | 2898.558 | 2950.274 | 3187.981 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow list-scroll | wall | 50 | 10446.901 | 10838.518 | 11024.241 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow big-screen | wall | 50 | 3594.486 | 3668.238 | 3680.266 | 0.0 | 0.0 | 504 |

Series that could not run:

- mobile-agent on emulator-5554, flow list-scroll: the check run failed: mobile-agent wait --timeout 10000 --text=Detail 200: wait_timeout: waited 10002 ms for text "Detail 200" to appear; the screen now shows revision 23, 2 windows, 22 elements with refs, read 10224 ms ago: / the check run failed: id=row-200 was not visible after 50 scroll steps of 600 points down

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| emulator-5554 | — | — | cli-start | 7.571 / 9.053 |
| emulator-5554 | expo | — | cold-attach | 778.517 / 927.563 |
| emulator-5554 | expo | big | first-snapshot | 31.573 / 792.390 |
| emulator-5554 | expo | big | warm-snapshot | 11.462 / 13.162 |
| emulator-5554 | expo | list | tap-dispatch | 4.089 / 5.068 |
| emulator-5554 | expo | list | tap-to-frame-change | no samples |
| emulator-5554 | expo | list | tap-to-tree-delta | 57.767 / 67.714 |
| emulator-5554 | expo | login | text-entry | 27.986 / 43.946 |
| emulator-5554 | expo | big | screenshot | 35.279 / 44.031 |
| emulator-5554 | expo | list | settle | 266.395 / 272.329 |
| emulator-5554 | expo | — | flow login | 977.609 / 1033.670 |
| emulator-5554 | expo | — | flow list-scroll | no samples |
| emulator-5554 | expo | — | flow big-screen | 1590.111 / 1603.563 |
| emulator-5554 | compose | — | cold-attach | 744.911 / 880.058 |
| emulator-5554 | compose | big | first-snapshot | 35.902 / 44.336 |
| emulator-5554 | compose | big | warm-snapshot | 10.609 / 11.301 |
| emulator-5554 | compose | list | tap-dispatch | 5.069 / 6.747 |
| emulator-5554 | compose | list | tap-to-frame-change | no samples |
| emulator-5554 | compose | list | tap-to-tree-delta | 222.845 / 231.750 |
| emulator-5554 | compose | login | text-entry | 23.481 / 46.027 |
| emulator-5554 | compose | big | screenshot | 35.758 / 39.626 |
| emulator-5554 | compose | list | settle | 188.458 / 196.565 |
| emulator-5554 | compose | — | flow login | 2898.558 / 2950.274 |
| emulator-5554 | compose | — | flow list-scroll | 10446.901 / 10838.518 |
| emulator-5554 | compose | — | flow big-screen | 3594.486 / 3668.238 |

Skipped: maestro (left out with --no-baselines: the emulator's controller flag does not change its flows, so the numbers of Maestro 2.10.0 in reports/10.1-emulator-grpc stand for this run); maestro-runner (left out with --no-baselines, and not installed on this host (reports/10.1-emulator-grpc lists it as not found on PATH)); agent-device (left out with --no-baselines: the emulator's controller flag does not change its flows, so the numbers of agent-device 0.21.14 in reports/10.1-emulator-grpc stand for this run).

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-emulator cli-start | mobile-agent | emulator-5554 | p95 | 20.000 | 9.053 | pass |
| android-emulator cold-attach | mobile-agent | emulator-5554 | p95 | 1500.000 | 927.563 | pass |
| android-emulator cold-attach | mobile-agent | emulator-5554 | p95 | 1500.000 | 880.058 | pass |
| android-emulator warm-snapshot big | mobile-agent | emulator-5554 | p95 | 100.000 | 13.162 | pass |
| android-emulator warm-snapshot big | mobile-agent | emulator-5554 | p95 | 100.000 | 11.301 | pass |
| android-emulator tap-to-frame-change | mobile-agent | emulator-5554 | p95 | 50.000 | no samples | FAIL |
| android-emulator tap-to-frame-change | mobile-agent | emulator-5554 | p95 | 50.000 | no samples | FAIL |

2 of 7 gate checks failed.

