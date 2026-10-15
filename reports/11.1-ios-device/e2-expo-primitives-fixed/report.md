# Benchmark report

Started 2026-10-11T14:58:26Z, finished 2026-10-11T15:12:19Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 3.62 3.38 4.30 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | c68b3e48ddde7da2ba31ef2ab15504990d3b891e (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | expo in 2982.761 ms |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | IPHONE-SE-UDID | — | — | cli-start | wall | 50 | 5.321 | 7.559 | 7.807 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | — | cold-attach | wall | 50 | 2152.666 | 2301.291 | 2314.864 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | big | first-snapshot | wall | 50 | 457.754 | 569.687 | 575.394 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | expo | big | warm-snapshot | wall | 50 | 7.014 | 9.382 | 9.893 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | expo | list | tap-dispatch | tool | 50 | 53.406 | 54.622 | 56.863 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | list | tap-to-frame-change | tool | 50 | 153.993 | 179.697 | 235.619 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | list | tap-to-tree-delta | tool | 50 | 289.676 | 326.612 | 382.307 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | login | text-entry | wall | 50 | 1816.998 | 2815.180 | 3320.055 | 0.0 | 2.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | big | screenshot | wall | 50 | 139.793 | 202.162 | 684.784 | 0.0 | 2.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | list | settle | tool | 50 | 0.003 | 0.006 | 0.023 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| IPHONE-SE-UDID | — | — | cli-start | 5.321 / 7.559 |
| IPHONE-SE-UDID | expo | — | cold-attach | 2152.666 / 2301.291 |
| IPHONE-SE-UDID | expo | big | first-snapshot | 457.754 / 569.687 |
| IPHONE-SE-UDID | expo | big | warm-snapshot | 7.014 / 9.382 |
| IPHONE-SE-UDID | expo | list | tap-dispatch | 53.406 / 54.622 |
| IPHONE-SE-UDID | expo | list | tap-to-frame-change | 153.993 / 179.697 |
| IPHONE-SE-UDID | expo | list | tap-to-tree-delta | 289.676 / 326.612 |
| IPHONE-SE-UDID | expo | login | text-entry | 1816.998 / 2815.180 |
| IPHONE-SE-UDID | expo | big | screenshot | 139.793 / 202.162 |
| IPHONE-SE-UDID | expo | list | settle | 0.003 / 0.006 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | end of the run | 1836.869 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | mobile-agent | IPHONE-SE-UDID | p95 | 20.000 | 7.559 | pass | — |
| ios-device cold-attach | mobile-agent | IPHONE-SE-UDID | p95 | 5000.000 | 2301.291 | pass | the runner already installed on the phone |
| ios-device warm-snapshot big | mobile-agent | IPHONE-SE-UDID | p95 | 1000.000 | 9.382 | pass | — |
| ios-device tap-dispatch | mobile-agent | IPHONE-SE-UDID | p95 | 150.000 | 54.622 | pass | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | mobile-agent | IPHONE-SE-UDID | p95 | — | 179.697 | reported | not gated while the phone's frames are polled screenshots |

All 4 gate checks passed. 1 reported with no verdict.

### Snapshot reads on physical iPhones

What the warm and first snapshots cost the daemon beside the wall time an agent waits: its own time to answer (`readMs`) for the snapshots it served from its replica, and for those that read the phone through the runner, the read the warm snapshot's limit was set from.

| Snapshot | Tool | Target | Fixture | Screen | Wall p50 / p95 ms | Replica n | Replica p50 / p95 ms | Read n | Read p50 / p95 ms |
|---|---|---|---|---|---|---|---|---|---|
| first-snapshot | mobile-agent | IPHONE-SE-UDID | expo | big | 457.754 / 569.687 | 50 | 1.244 / 1.956 | 0 | — |
| warm-snapshot | mobile-agent | IPHONE-SE-UDID | expo | big | 7.014 / 9.382 | 50 | 0.537 / 0.643 | 0 | — |


