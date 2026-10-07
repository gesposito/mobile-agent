# Benchmark report

Started 2026-10-07T16:38:39Z, finished 2026-10-07T17:42:30Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.95 2.64 2.63 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 552c9e223f68233ed750457fb51826e85620bf8b (clean) |
| Harness | bench 0.1.0, 20 iterations after 2 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| NEXUS5X-SERIAL | android-device | Nexus 5X | Android API 27 | 27 | — | no |

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
| mobile-agent | NEXUS5X-SERIAL | expo | — | flow login | wall | 20 | 2723.410 | 2934.789 | 3006.245 | 0.0 | 0.0 | — |
| maestro | NEXUS5X-SERIAL | expo | — | flow login | wall | 20 | 62920.299 | 64828.607 | 67165.609 | 0.0 | 0.0 | — |
| agent-device | NEXUS5X-SERIAL | expo | — | flow login | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | compose | — | flow login | wall | 20 | 7347.389 | 7716.124 | 8053.071 | 0.0 | 0.0 | — |
| maestro | NEXUS5X-SERIAL | compose | — | flow login | wall | 20 | 55718.815 | 57366.449 | 60568.258 | 0.0 | 0.0 | — |
| agent-device | NEXUS5X-SERIAL | compose | — | flow login | wall | 20 | 27675.130 | 28044.847 | 28081.043 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent | maestro | agent-device |
|---|---|---|---|---|---|---|
| NEXUS5X-SERIAL | expo | — | flow login | 2723.410 / 2934.789 | 62920.299 / 64828.607 | no samples |
| NEXUS5X-SERIAL | compose | — | flow login | 7347.389 / 7716.124 | 55718.815 / 57366.449 | 27675.130 / 28044.847 |

Skipped: maestro-runner (maestro-runner not found on PATH).

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| NEXUS5X-SERIAL | mobile-agent | maestro | 192.767 | stopped the daemon |
| NEXUS5X-SERIAL | maestro | agent-device | 107.436 | nothing held |
| NEXUS5X-SERIAL | agent-device | mobile-agent | 1016.361 | closed agent-device session mobile-agent-bench and stopped its daemon |
| NEXUS5X-SERIAL | mobile-agent | maestro | 181.732 | stopped the daemon |
| NEXUS5X-SERIAL | maestro | agent-device | 97.897 | nothing held |
| NEXUS5X-SERIAL | agent-device | mobile-agent | 956.728 | closed agent-device session mobile-agent-bench and stopped its daemon |
| NEXUS5X-SERIAL | mobile-agent | maestro | 170.456 | stopped the daemon |
| NEXUS5X-SERIAL | maestro | agent-device | 93.888 | nothing held |
| NEXUS5X-SERIAL | agent-device | mobile-agent | 888.501 | closed agent-device session mobile-agent-bench and stopped its daemon |
| NEXUS5X-SERIAL | mobile-agent | maestro | 170.065 | stopped the daemon |
| NEXUS5X-SERIAL | maestro | agent-device | 107.001 | nothing held |
| NEXUS5X-SERIAL | agent-device | end of the run | 928.875 | closed agent-device session mobile-agent-bench and stopped its daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-device cli-start | — | — | p95 | 20.000 | — | not measured |
| android-device cold-attach | — | — | p95 | 1500.000 | — | not measured |
| android-device warm-snapshot big | — | — | p95 | 100.000 | — | not measured |
| android-device tap-dispatch | — | — | p95 | 50.000 | — | not measured |

All 0 gate checks passed. 4 not measured.

