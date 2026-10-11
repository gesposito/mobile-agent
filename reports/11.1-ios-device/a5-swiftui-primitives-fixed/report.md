# Benchmark report

Started 2026-10-11T14:10:07Z, finished 2026-10-11T14:32:08Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.42 3.64 4.34 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 1f75140c49b598e4a1fa955d5e604619b012a0f4 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | swiftui in 4959.056 ms |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | IPHONE-SE-UDID | — | — | cli-start | wall | 50 | 3.616 | 4.024 | 4.068 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | — | cold-attach | wall | 50 | 2087.785 | 9205.735 | 29464.756 | 0.0 | 2.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | first-snapshot | wall | 50 | 676.576 | 775.877 | 1735.657 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | warm-snapshot | wall | 50 | 7.201 | 8.951 | 10.171 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-dispatch | tool | 50 | 53.439 | 56.080 | 64.319 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-to-frame-change | tool | 50 | 274.592 | 343.376 | 365.668 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-to-tree-delta | tool | 50 | 522.693 | 539.952 | 578.638 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | login | text-entry | wall | 50 | 1776.330 | 2266.805 | 2279.502 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | screenshot | wall | 50 | 195.504 | 230.576 | 972.094 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | settle | tool | 50 | 397.755 | 488.189 | 510.229 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| IPHONE-SE-UDID | — | — | cli-start | 3.616 / 4.024 |
| IPHONE-SE-UDID | swiftui | — | cold-attach | 2087.785 / 9205.735 |
| IPHONE-SE-UDID | swiftui | big | first-snapshot | 676.576 / 775.877 |
| IPHONE-SE-UDID | swiftui | big | warm-snapshot | 7.201 / 8.951 |
| IPHONE-SE-UDID | swiftui | list | tap-dispatch | 53.439 / 56.080 |
| IPHONE-SE-UDID | swiftui | list | tap-to-frame-change | 274.592 / 343.376 |
| IPHONE-SE-UDID | swiftui | list | tap-to-tree-delta | 522.693 / 539.952 |
| IPHONE-SE-UDID | swiftui | login | text-entry | 1776.330 / 2266.805 |
| IPHONE-SE-UDID | swiftui | big | screenshot | 195.504 / 230.576 |
| IPHONE-SE-UDID | swiftui | list | settle | 397.755 / 488.189 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | end of the run | 2529.238 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | mobile-agent | IPHONE-SE-UDID | p95 | 20.000 | 4.024 | pass | — |
| ios-device cold-attach | mobile-agent | IPHONE-SE-UDID | p95 | 5000.000 | 9205.735 | FAIL | the runner already installed on the phone |
| ios-device warm-snapshot big | mobile-agent | IPHONE-SE-UDID | p95 | 1000.000 | 8.951 | pass | — |
| ios-device tap-dispatch | mobile-agent | IPHONE-SE-UDID | p95 | 150.000 | 56.080 | pass | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | mobile-agent | IPHONE-SE-UDID | p95 | — | 343.376 | reported | not gated while the phone's frames are polled screenshots |

1 of 4 gate checks failed. 1 reported with no verdict.

### Snapshot reads on physical iPhones

What the warm and first snapshots cost the daemon beside the wall time an agent waits: its own time to answer (`readMs`) for the snapshots it served from its replica, and for those that read the phone through the runner, the read the warm snapshot's limit was set from.

| Snapshot | Tool | Target | Fixture | Screen | Wall p50 / p95 ms | Replica n | Replica p50 / p95 ms | Read n | Read p50 / p95 ms |
|---|---|---|---|---|---|---|---|---|---|
| first-snapshot | mobile-agent | IPHONE-SE-UDID | swiftui | big | 676.576 / 775.877 | 49 | 0.665 / 1.282 | 1 | 979.323 / 979.323 |
| warm-snapshot | mobile-agent | IPHONE-SE-UDID | swiftui | big | 7.201 / 8.951 | 50 | 0.555 / 0.629 | 0 | — |


