# Benchmark report

Started 2026-10-07T10:06:36Z, finished 2026-10-07T10:12:26Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 3.95 4.33 4.69 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 6c1b4d550fbacc997e2425bb7efceb8907651157 (clean) |
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
| mobile-agent | emulator-5554 | expo | login | text-entry | wall | 50 | 43.018 | 57.048 | 64.586 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow login | wall | 50 | 1586.900 | 1606.927 | 1628.668 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | login | text-entry | wall | 50 | 234.921 | 258.574 | 265.016 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow login | wall | 50 | 3488.106 | 3686.781 | 3795.098 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| emulator-5554 | expo | login | text-entry | 43.018 / 57.048 |
| emulator-5554 | expo | — | flow login | 1586.900 / 1606.927 |
| emulator-5554 | compose | login | text-entry | 234.921 / 258.574 |
| emulator-5554 | compose | — | flow login | 3488.106 / 3686.781 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| emulator-5554 | mobile-agent | end of the run | 83.213 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-emulator cli-start | — | — | p95 | 20.000 | — | not measured |
| android-emulator cold-attach | — | — | p95 | 1500.000 | — | not measured |
| android-emulator warm-snapshot big | — | — | p95 | 100.000 | — | not measured |
| android-emulator tap-to-frame-change | — | — | p95 | 67.000 | — | not measured |

All 0 gate checks passed. 4 not measured.

