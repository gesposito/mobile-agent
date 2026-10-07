# Benchmark report

Started 2026-10-07T13:22:17Z, finished 2026-10-07T13:24:04Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 3.46 5.66 6.81 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 1ad66f9085df49f9556a1709bb0ebe88c3409257 (clean) |
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
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | login | text-entry | wall | 50 | 386.975 | 434.554 | 459.632 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | login | text-entry | 386.975 / 434.554 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | end of the run | 299.910 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| simulator cli-start | — | — | p95 | 20.000 | — | not measured |
| simulator cold-attach | — | — | p95 | 2000.000 | — | not measured |
| simulator warm-snapshot swiftui big | — | — | p95 | 150.000 | — | not measured |
| simulator warm-snapshot expo big | — | — | p95 | 250.000 | — | not measured |
| simulator tap-dispatch | — | — | p95 | 20.000 | — | not measured |

All 0 gate checks passed. 5 not measured.

