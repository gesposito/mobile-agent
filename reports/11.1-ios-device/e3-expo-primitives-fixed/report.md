# Benchmark report

Started 2026-10-11T15:16:52Z, finished 2026-10-11T15:30:00Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.33 2.71 3.53 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 60e5ee7979bec237e62bf3e99c52542fd24e957e (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | expo in 3000.542 ms |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | IPHONE-SE-UDID | — | — | cli-start | wall | 50 | 3.647 | 4.095 | 4.191 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | — | cold-attach | wall | 17 | 2170.767 | 2315.901 | 2315.901 | 66.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | big | first-snapshot | wall | 45 | 459.833 | 564.441 | 574.853 | 10.0 | 2.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | expo | big | warm-snapshot | wall | 50 | 5.750 | 6.490 | 6.864 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | expo | list | tap-dispatch | tool | 50 | 53.788 | 56.382 | 62.170 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | list | tap-to-frame-change | tool | 50 | 161.503 | 237.813 | 242.712 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | list | tap-to-tree-delta | tool | 50 | 295.038 | 347.214 | 370.079 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | login | text-entry | wall | 50 | 1810.880 | 2278.807 | 3068.388 | 0.0 | 2.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | big | screenshot | wall | 50 | 181.821 | 711.449 | 726.542 | 0.0 | 18.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | list | settle | tool | 50 | 191.943 | 199.158 | 202.658 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| IPHONE-SE-UDID | — | — | cli-start | 3.647 / 4.095 |
| IPHONE-SE-UDID | expo | — | cold-attach | 2170.767 / 2315.901 |
| IPHONE-SE-UDID | expo | big | first-snapshot | 459.833 / 564.441 |
| IPHONE-SE-UDID | expo | big | warm-snapshot | 5.750 / 6.490 |
| IPHONE-SE-UDID | expo | list | tap-dispatch | 53.788 / 56.382 |
| IPHONE-SE-UDID | expo | list | tap-to-frame-change | 161.503 / 237.813 |
| IPHONE-SE-UDID | expo | list | tap-to-tree-delta | 295.038 / 347.214 |
| IPHONE-SE-UDID | expo | login | text-entry | 1810.880 / 2278.807 |
| IPHONE-SE-UDID | expo | big | screenshot | 181.821 / 711.449 |
| IPHONE-SE-UDID | expo | list | settle | 191.943 / 199.158 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | end of the run | 2259.145 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | mobile-agent | IPHONE-SE-UDID | p95 | 20.000 | 4.095 | pass | — |
| ios-device cold-attach | mobile-agent | IPHONE-SE-UDID | p95 | 5000.000 | 2315.901 | pass | the runner already installed on the phone |
| ios-device warm-snapshot big | mobile-agent | IPHONE-SE-UDID | p95 | 1000.000 | 6.490 | pass | — |
| ios-device tap-dispatch | mobile-agent | IPHONE-SE-UDID | p95 | 150.000 | 56.382 | pass | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | mobile-agent | IPHONE-SE-UDID | p95 | — | 237.813 | reported | not gated while the phone's frames are polled screenshots |

All 4 gate checks passed. 1 reported with no verdict.

### Snapshot reads on physical iPhones

What the warm and first snapshots cost the daemon beside the wall time an agent waits: its own time to answer (`readMs`) for the snapshots it served from its replica, and for those that read the phone through the runner, the read the warm snapshot's limit was set from.

| Snapshot | Tool | Target | Fixture | Screen | Wall p50 / p95 ms | Replica n | Replica p50 / p95 ms | Read n | Read p50 / p95 ms |
|---|---|---|---|---|---|---|---|---|---|
| first-snapshot | mobile-agent | IPHONE-SE-UDID | expo | big | 459.833 / 564.441 | 45 | 1.958 / 3.514 | 0 | — |
| warm-snapshot | mobile-agent | IPHONE-SE-UDID | expo | big | 5.750 / 6.490 | 50 | 0.477 / 0.582 | 0 | — |


