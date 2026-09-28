# Benchmark report

Started 2026-09-28T14:25:26Z, finished 2026-09-28T15:10:07Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.55 2.89 2.81 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | dd4ed5a4f3b470a0bac23f21d4a436ce2d333083 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | simulator | iPhone 17 | iOS 26.5 | — | — | yes |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow list-scroll | wall | 50 | 25265.455 | 25517.752 | 25594.456 | 0.0 | 16.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow list-scroll | wall | 50 | 20565.332 | 21033.210 | 21111.500 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow list-scroll | 25265.455 / 25517.752 |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow list-scroll | 20565.332 / 21033.210 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | end of the run | 316.162 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| simulator cli-start | — | — | p95 | 20.000 | — | not measured |
| simulator cold-attach | — | — | p95 | 2000.000 | — | not measured |
| simulator warm-snapshot swiftui big | — | — | p95 | 150.000 | — | not measured |
| simulator warm-snapshot expo big | — | — | p95 | 250.000 | — | not measured |
| simulator tap-dispatch | — | — | p95 | 20.000 | — | not measured |

All 0 gate checks passed. 5 not measured.

