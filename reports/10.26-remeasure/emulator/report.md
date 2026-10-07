# Benchmark report

Started 2026-10-07T08:07:53Z, finished 2026-10-07T09:06:42Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 3.24 5.18 6.52 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 0ae21d1eaac5256c0ed2e0f57b0eae23bd0010ce (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| emulator-5554 | android-emulator | sdk_gphone64_arm64 | Android API 36 | 36 | on | yes |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |
| maestro | baseline | available | 2.10.0 | — |
| maestro-runner | baseline | skipped | — | maestro-runner not found on PATH |
| agent-device | baseline | available | 0.21.14 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | emulator-5554 | expo | login | text-entry | wall | 50 | 224.067 | 237.300 | 249.419 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow login | wall | 50 | 1969.252 | 2009.442 | 2309.757 | 0.0 | 0.0 | — |
| maestro | emulator-5554 | expo | — | flow login | wall | 50 | 23417.413 | 25143.072 | 26135.604 | 0.0 | 0.0 | — |
| agent-device | emulator-5554 | expo | — | flow login | wall | 38 | 4589.877 | 4762.418 | 4770.505 | 24.0 | 12.0 | — |
| mobile-agent | emulator-5554 | compose | login | text-entry | wall | 50 | 240.047 | 258.975 | 260.004 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow login | wall | 50 | 3567.005 | 3961.855 | 6635.306 | 0.0 | 0.0 | — |
| maestro | emulator-5554 | compose | — | flow login | wall | 50 | 23530.629 | 24466.083 | 26447.115 | 0.0 | 0.0 | — |
| agent-device | emulator-5554 | compose | — | flow login | wall | 50 | 5940.347 | 6098.489 | 6138.412 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent | maestro | agent-device |
|---|---|---|---|---|---|---|
| emulator-5554 | expo | login | text-entry | 224.067 / 237.300 | — | — |
| emulator-5554 | expo | — | flow login | 1969.252 / 2009.442 | 23417.413 / 25143.072 | 4589.877 / 4762.418 |
| emulator-5554 | compose | login | text-entry | 240.047 / 258.975 | — | — |
| emulator-5554 | compose | — | flow login | 3567.005 / 3961.855 | 23530.629 / 24466.083 | 5940.347 / 6098.489 |

Skipped: maestro-runner (maestro-runner not found on PATH).

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| emulator-5554 | mobile-agent | maestro | 78.299 | stopped the daemon |
| emulator-5554 | maestro | agent-device | 35.907 | nothing held |
| emulator-5554 | agent-device | mobile-agent | 443.566 | closed agent-device session mobile-agent-bench |
| emulator-5554 | mobile-agent | maestro | 103.574 | stopped the daemon |
| emulator-5554 | maestro | agent-device | 34.432 | nothing held |
| emulator-5554 | agent-device | mobile-agent | 443.164 | closed agent-device session mobile-agent-bench |
| emulator-5554 | mobile-agent | maestro | 75.555 | stopped the daemon |
| emulator-5554 | maestro | agent-device | 34.429 | nothing held |
| emulator-5554 | agent-device | mobile-agent | 443.243 | closed agent-device session mobile-agent-bench |
| emulator-5554 | mobile-agent | maestro | 86.665 | stopped the daemon |
| emulator-5554 | maestro | agent-device | 33.849 | nothing held |
| emulator-5554 | agent-device | end of the run | 428.000 | closed agent-device session mobile-agent-bench |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-emulator cli-start | — | — | p95 | 20.000 | — | not measured |
| android-emulator cold-attach | — | — | p95 | 1500.000 | — | not measured |
| android-emulator warm-snapshot big | — | — | p95 | 100.000 | — | not measured |
| android-emulator tap-to-frame-change | — | — | p95 | 67.000 | — | not measured |

All 0 gate checks passed. 4 not measured.

