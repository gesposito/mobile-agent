# Benchmark report

Started 2026-10-10T08:42:27Z, finished 2026-10-10T08:56:19Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.89 2.57 2.57 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | a8671bb13d9739e1faaef59399b48e61912b3b01 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | expo in 2389.090 ms |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | IPHONE-SE-UDID | — | — | cli-start | wall | 50 | 4.526 | 4.965 | 5.006 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | — | cold-attach | wall | 50 | 2471.192 | 2804.055 | 3333.542 | 0.0 | 2.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | big | first-snapshot | wall | 50 | 558.770 | 568.028 | 576.237 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | expo | big | warm-snapshot | wall | 50 | 7.504 | 11.053 | 562.230 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | expo | list | tap-dispatch | tool | 50 | 53.587 | 54.894 | 61.051 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | list | tap-to-frame-change | tool | 50 | 219.581 | 241.564 | 261.043 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | list | tap-to-tree-delta | tool | 50 | 347.082 | 363.099 | 418.754 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | login | text-entry | wall | 50 | 1777.024 | 2163.686 | 2275.774 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | big | screenshot | wall | 50 | 214.851 | 776.244 | 844.395 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | expo | list | settle | tool | 50 | 185.323 | 197.262 | 212.998 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| IPHONE-SE-UDID | — | — | cli-start | 4.526 / 4.965 |
| IPHONE-SE-UDID | expo | — | cold-attach | 2471.192 / 2804.055 |
| IPHONE-SE-UDID | expo | big | first-snapshot | 558.770 / 568.028 |
| IPHONE-SE-UDID | expo | big | warm-snapshot | 7.504 / 11.053 |
| IPHONE-SE-UDID | expo | list | tap-dispatch | 53.587 / 54.894 |
| IPHONE-SE-UDID | expo | list | tap-to-frame-change | 219.581 / 241.564 |
| IPHONE-SE-UDID | expo | list | tap-to-tree-delta | 347.082 / 363.099 |
| IPHONE-SE-UDID | expo | login | text-entry | 1777.024 / 2163.686 |
| IPHONE-SE-UDID | expo | big | screenshot | 214.851 / 776.244 |
| IPHONE-SE-UDID | expo | list | settle | 185.323 / 197.262 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | end of the run | 2751.584 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | mobile-agent | IPHONE-SE-UDID | p95 | 20.000 | 4.965 | pass | — |
| ios-device cold-attach | mobile-agent | IPHONE-SE-UDID | p95 | 5000.000 | 2804.055 | pass | the runner already installed on the phone |
| ios-device warm-snapshot big | mobile-agent | IPHONE-SE-UDID | p95 | 1000.000 | 11.053 | pass | — |
| ios-device tap-dispatch | mobile-agent | IPHONE-SE-UDID | p95 | 150.000 | 54.894 | pass | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | mobile-agent | IPHONE-SE-UDID | p95 | — | 241.564 | reported | not gated while the phone's frames are polled screenshots |

All 4 gate checks passed. 1 reported with no verdict.

