# Benchmark report

Started 2026-10-07T11:53:33Z, finished 2026-10-07T12:05:00Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 4.17 6.52 8.60 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 85ccb9a7ea8d43944f88c486d2b5a9cc67b09b63 (clean) |
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
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | login | text-entry | wall | 50 | 376.272 | 621.917 | 632.408 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | wall | 50 | 4198.355 | 4374.053 | 4603.021 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | login | text-entry | wall | 50 | 555.399 | 624.629 | 641.062 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | wall | 50 | 4000.833 | 4298.120 | 4318.738 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | login | text-entry | 376.272 / 621.917 |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | 4198.355 / 4374.053 |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | login | text-entry | 555.399 / 624.629 |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | 4000.833 / 4298.120 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | end of the run | 135.785 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| simulator cli-start | — | — | p95 | 20.000 | — | not measured |
| simulator cold-attach | — | — | p95 | 2000.000 | — | not measured |
| simulator warm-snapshot swiftui big | — | — | p95 | 150.000 | — | not measured |
| simulator warm-snapshot expo big | — | — | p95 | 250.000 | — | not measured |
| simulator tap-dispatch | — | — | p95 | 20.000 | — | not measured |

All 0 gate checks passed. 5 not measured.

