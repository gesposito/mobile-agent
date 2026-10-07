# Benchmark report

Started 2026-10-07T06:16:18Z, finished 2026-10-07T07:26:40Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.41 6.63 7.61 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 0ae21d1eaac5256c0ed2e0f57b0eae23bd0010ce (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | simulator | iPhone 17 | iOS 26.5 | — | — | yes |

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
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | login | text-entry | wall | 50 | 545.468 | 808.958 | 823.476 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | wall | 50 | 4173.003 | 4358.115 | 4504.427 | 0.0 | 0.0 | — |
| maestro | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | wall | 50 | 25459.144 | 28458.372 | 32103.174 | 0.0 | 0.0 | — |
| agent-device | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | wall | 50 | 8790.197 | 8919.262 | 8933.173 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | login | text-entry | wall | 50 | 718.264 | 804.985 | 807.642 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | wall | 50 | 3860.186 | 4269.408 | 4373.599 | 0.0 | 0.0 | — |
| maestro | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | wall | 50 | 24899.915 | 26750.680 | 27385.908 | 0.0 | 0.0 | — |
| agent-device | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | wall | 5 | 8255.461 | 8408.351 | 8408.351 | 90.0 | 2.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent | maestro | agent-device |
|---|---|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | login | text-entry | 545.468 / 808.958 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | 4173.003 / 4358.115 | 25459.144 / 28458.372 | 8790.197 / 8919.262 |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | login | text-entry | 718.264 / 804.985 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | 3860.186 / 4269.408 | 24899.915 / 26750.680 | 8255.461 / 8408.351 |

Skipped: maestro-runner (maestro-runner not found on PATH).

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 316.835 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.003 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 802.470 | closed agent-device session mobile-agent-bench and stopped its daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 338.780 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 719.272 | closed agent-device session mobile-agent-bench and stopped its daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 207.827 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.001 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 754.836 | closed agent-device session mobile-agent-bench and stopped its daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 128.034 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | end of the run | 732.599 | closed agent-device session mobile-agent-bench and stopped its daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| simulator cli-start | — | — | p95 | 20.000 | — | not measured |
| simulator cold-attach | — | — | p95 | 2000.000 | — | not measured |
| simulator warm-snapshot swiftui big | — | — | p95 | 150.000 | — | not measured |
| simulator warm-snapshot expo big | — | — | p95 | 250.000 | — | not measured |
| simulator tap-dispatch | — | — | p95 | 20.000 | — | not measured |

All 0 gate checks passed. 5 not measured.

