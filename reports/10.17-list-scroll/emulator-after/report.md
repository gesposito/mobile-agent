# Benchmark report

Started 2026-09-28T14:00:59Z, finished 2026-09-28T14:24:44Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.72 3.49 3.45 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | dcb96229127ab64855f9c614052736c189fdb96d (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| emulator-5554 | android-emulator | sdk_gphone64_arm64 | Android API 36 | 36 | on | yes |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | emulator-5554 | expo | — | flow list-scroll | wall | 50 | 12600.921 | 13206.186 | 13313.729 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow list-scroll | wall | 50 | 13740.107 | 14112.943 | 14369.584 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| emulator-5554 | expo | — | flow list-scroll | 12600.921 / 13206.186 |
| emulator-5554 | compose | — | flow list-scroll | 13740.107 / 14112.943 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| emulator-5554 | mobile-agent | end of the run | 84.128 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-emulator cli-start | — | — | p95 | 20.000 | — | not measured |
| android-emulator cold-attach | — | — | p95 | 1500.000 | — | not measured |
| android-emulator warm-snapshot big | — | — | p95 | 100.000 | — | not measured |
| android-emulator tap-to-frame-change | — | — | p95 | 67.000 | — | not measured |

All 0 gate checks passed. 4 not measured.

