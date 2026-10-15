# Benchmark report

Started 2026-10-10T07:30:47Z, finished 2026-10-10T07:50:12Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.84 5.00 5.28 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | f8e840b0020c2c8568c80162e7cb63a89cabaf0c (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | swiftui in 1469.867 ms |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | IPHONE-SE-UDID | — | — | cli-start | wall | 50 | 5.158 | 9.959 | 29.691 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | — | cold-attach | wall | 50 | 2501.526 | 2603.121 | 2813.982 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | first-snapshot | wall | 50 | 176.991 | 192.931 | 195.919 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | warm-snapshot | wall | 50 | 7.645 | 13.715 | 665.018 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-dispatch | tool | 50 | 54.020 | 55.443 | 55.604 | 0.0 | 2.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-to-frame-change | tool | 50 | 340.004 | 360.057 | 361.634 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-to-tree-delta | tool | 50 | 592.587 | 609.386 | 611.952 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | login | text-entry | wall | 50 | 1838.779 | 2272.565 | 2281.029 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | screenshot | wall | 50 | 843.609 | 968.077 | 1018.816 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | settle | tool | 50 | 350.366 | 484.001 | 491.808 | 0.0 | 0.0 | — |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| IPHONE-SE-UDID | — | — | cli-start | 5.158 / 9.959 |
| IPHONE-SE-UDID | swiftui | — | cold-attach | 2501.526 / 2603.121 |
| IPHONE-SE-UDID | swiftui | big | first-snapshot | 176.991 / 192.931 |
| IPHONE-SE-UDID | swiftui | big | warm-snapshot | 7.645 / 13.715 |
| IPHONE-SE-UDID | swiftui | list | tap-dispatch | 54.020 / 55.443 |
| IPHONE-SE-UDID | swiftui | list | tap-to-frame-change | 340.004 / 360.057 |
| IPHONE-SE-UDID | swiftui | list | tap-to-tree-delta | 592.587 / 609.386 |
| IPHONE-SE-UDID | swiftui | login | text-entry | 1838.779 / 2272.565 |
| IPHONE-SE-UDID | swiftui | big | screenshot | 843.609 / 968.077 |
| IPHONE-SE-UDID | swiftui | list | settle | 350.366 / 484.001 |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | end of the run | 2629.347 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | mobile-agent | IPHONE-SE-UDID | p95 | 20.000 | 9.959 | pass | — |
| ios-device cold-attach | mobile-agent | IPHONE-SE-UDID | p95 | 5000.000 | 2603.121 | pass | the runner already installed on the phone |
| ios-device warm-snapshot big | mobile-agent | IPHONE-SE-UDID | p95 | 1000.000 | 13.715 | pass | — |
| ios-device tap-dispatch | mobile-agent | IPHONE-SE-UDID | p95 | 150.000 | 55.443 | pass | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | mobile-agent | IPHONE-SE-UDID | p95 | — | 360.057 | reported | not gated while the phone's frames are polled screenshots |

All 4 gate checks passed. 1 reported with no verdict.

