# Benchmark report

Started 2026-09-28T13:33:51Z, finished 2026-09-28T14:00:14Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.59 3.88 3.55 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | dd4ed5a4f3b470a0bac23f21d4a436ce2d333083 (clean) |
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
| mobile-agent | emulator-5554 | expo | — | flow list-scroll | wall | 50 | 14064.724 | 14258.399 | 14525.179 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow list-scroll | wall | 50 | 15216.088 | 15533.870 | 15600.875 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| emulator-5554 | expo | — | flow list-scroll | 14064.724 / 14258.399 |
| emulator-5554 | compose | — | flow list-scroll | 15216.088 / 15533.870 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| emulator-5554 | mobile-agent | end of the run | 78.013 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-emulator cli-start | — | — | p95 | 20.000 | — | not measured |
| android-emulator cold-attach | — | — | p95 | 1500.000 | — | not measured |
| android-emulator warm-snapshot big | — | — | p95 | 100.000 | — | not measured |
| android-emulator tap-to-frame-change | — | — | p95 | 67.000 | — | not measured |

All 0 gate checks passed. 4 not measured.

