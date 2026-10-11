# Benchmark report

Started 2026-10-11T14:36:26Z, finished 2026-10-11T14:53:02Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 3.87 4.13 4.75 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 2cfae4f77d6eff3922d3c8cf07dc8927c2d39091 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | swiftui in 2389.073 ms |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | IPHONE-SE-UDID | — | — | cli-start | wall | 50 | 5.142 | 6.958 | 8.660 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | — | cold-attach | wall | 50 | 2135.738 | 4052.002 | 4330.619 | 0.0 | 2.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | first-snapshot | wall | 50 | 766.097 | 778.192 | 780.374 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | warm-snapshot | wall | 50 | 7.348 | 8.636 | 9.418 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-dispatch | tool | 50 | 53.348 | 55.059 | 55.298 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-to-frame-change | tool | 50 | 272.742 | 364.000 | 376.302 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-to-tree-delta | tool | 50 | 526.581 | 598.705 | 613.199 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | login | text-entry | wall | 50 | 1398.678 | 1845.942 | 1860.234 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | screenshot | wall | 50 | 207.675 | 227.316 | 961.002 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | settle | tool | 50 | 364.737 | 387.046 | 388.658 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| IPHONE-SE-UDID | — | — | cli-start | 5.142 / 6.958 |
| IPHONE-SE-UDID | swiftui | — | cold-attach | 2135.738 / 4052.002 |
| IPHONE-SE-UDID | swiftui | big | first-snapshot | 766.097 / 778.192 |
| IPHONE-SE-UDID | swiftui | big | warm-snapshot | 7.348 / 8.636 |
| IPHONE-SE-UDID | swiftui | list | tap-dispatch | 53.348 / 55.059 |
| IPHONE-SE-UDID | swiftui | list | tap-to-frame-change | 272.742 / 364.000 |
| IPHONE-SE-UDID | swiftui | list | tap-to-tree-delta | 526.581 / 598.705 |
| IPHONE-SE-UDID | swiftui | login | text-entry | 1398.678 / 1845.942 |
| IPHONE-SE-UDID | swiftui | big | screenshot | 207.675 / 227.316 |
| IPHONE-SE-UDID | swiftui | list | settle | 364.737 / 387.046 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | end of the run | 2530.796 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | mobile-agent | IPHONE-SE-UDID | p95 | 20.000 | 6.958 | pass | — |
| ios-device cold-attach | mobile-agent | IPHONE-SE-UDID | p95 | 5000.000 | 4052.002 | pass | the runner already installed on the phone |
| ios-device warm-snapshot big | mobile-agent | IPHONE-SE-UDID | p95 | 1000.000 | 8.636 | pass | — |
| ios-device tap-dispatch | mobile-agent | IPHONE-SE-UDID | p95 | 150.000 | 55.059 | pass | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | mobile-agent | IPHONE-SE-UDID | p95 | — | 364.000 | reported | not gated while the phone's frames are polled screenshots |

All 4 gate checks passed. 1 reported with no verdict.

### Snapshot reads on physical iPhones

What the warm and first snapshots cost the daemon beside the wall time an agent waits: its own time to answer (`readMs`) for the snapshots it served from its replica, and for those that read the phone through the runner, the read the warm snapshot's limit was set from.

| Snapshot | Tool | Target | Fixture | Screen | Wall p50 / p95 ms | Replica n | Replica p50 / p95 ms | Read n | Read p50 / p95 ms |
|---|---|---|---|---|---|---|---|---|---|
| first-snapshot | mobile-agent | IPHONE-SE-UDID | swiftui | big | 766.097 / 778.192 | 50 | 1.213 / 1.511 | 0 | — |
| warm-snapshot | mobile-agent | IPHONE-SE-UDID | swiftui | big | 7.348 / 8.636 | 50 | 0.562 / 0.612 | 0 | — |


