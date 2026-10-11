# Benchmark report

Started 2026-10-11T09:00:13Z, finished 2026-10-11T09:18:11Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 1.87 3.40 5.51 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 180dc234347e3ba22468071126b9c2fd9b37cdae (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | swiftui in 2274.389 ms |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | IPHONE-SE-UDID | — | — | cli-start | wall | 50 | 3.562 | 3.863 | 4.277 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | — | cold-attach | wall | 50 | 2326.924 | 2402.848 | 2573.633 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | first-snapshot | wall | 50 | 573.534 | 681.007 | 774.211 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | warm-snapshot | wall | 50 | 5.807 | 6.543 | 7.963 | 0.0 | 0.0 | 506 |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-dispatch | tool | 50 | 53.673 | 54.972 | 56.116 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-to-frame-change | tool | 50 | 278.392 | 290.362 | 292.657 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | tap-to-tree-delta | tool | 50 | 539.813 | 549.334 | 549.710 | 0.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | login | text-entry | wall | 39 | 1780.306 | 2269.668 | 2270.595 | 22.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | screenshot | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | settle | — | 0 | — | — | — | 100.0 | 0.0 | — |

Series that could not run:

- mobile-agent on IPHONE-SE-UDID, screenshot: preparation failed: mobile-agent open --relaunch --env FIXTURE_INITIAL_TAB=big -- dev.mobileagent.fixture.swiftui: busy: the runner refused snapshot: the runner is still serving request 1097
- mobile-agent on IPHONE-SE-UDID, settle: preparation failed: mobile-agent open --relaunch --env FIXTURE_INITIAL_TAB=list -- dev.mobileagent.fixture.swiftui: busy: the runner refused snapshot: the runner is still serving request 1097

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| IPHONE-SE-UDID | — | — | cli-start | 3.562 / 3.863 |
| IPHONE-SE-UDID | swiftui | — | cold-attach | 2326.924 / 2402.848 |
| IPHONE-SE-UDID | swiftui | big | first-snapshot | 573.534 / 681.007 |
| IPHONE-SE-UDID | swiftui | big | warm-snapshot | 5.807 / 6.543 |
| IPHONE-SE-UDID | swiftui | list | tap-dispatch | 53.673 / 54.972 |
| IPHONE-SE-UDID | swiftui | list | tap-to-frame-change | 278.392 / 290.362 |
| IPHONE-SE-UDID | swiftui | list | tap-to-tree-delta | 539.813 / 549.334 |
| IPHONE-SE-UDID | swiftui | login | text-entry | 1780.306 / 2269.668 |
| IPHONE-SE-UDID | swiftui | big | screenshot | no samples |
| IPHONE-SE-UDID | swiftui | list | settle | no samples |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | end of the run | 7062.024 | stopped the daemon; failed: mobile-agent left runner app on the iPhone (pid 21755) running after its release; the hand-off sent it SIGTERM, and it exited |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | mobile-agent | IPHONE-SE-UDID | p95 | 20.000 | 3.863 | pass | — |
| ios-device cold-attach | mobile-agent | IPHONE-SE-UDID | p95 | 5000.000 | 2402.848 | pass | the runner already installed on the phone |
| ios-device warm-snapshot big | mobile-agent | IPHONE-SE-UDID | p95 | 1000.000 | 6.543 | pass | — |
| ios-device tap-dispatch | mobile-agent | IPHONE-SE-UDID | p95 | 150.000 | 54.972 | pass | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | mobile-agent | IPHONE-SE-UDID | p95 | — | 290.362 | reported | not gated while the phone's frames are polled screenshots |

All 4 gate checks passed. 1 reported with no verdict.

### Snapshot reads on physical iPhones

What the warm and first snapshots cost the daemon beside the wall time an agent waits: its own time to answer (`readMs`) for the snapshots it served from its replica, and for those that read the phone through the runner, the read the warm snapshot's limit was set from.

| Snapshot | Tool | Target | Fixture | Screen | Wall p50 / p95 ms | Replica n | Replica p50 / p95 ms | Read n | Read p50 / p95 ms |
|---|---|---|---|---|---|---|---|---|---|
| first-snapshot | mobile-agent | IPHONE-SE-UDID | swiftui | big | 573.534 / 681.007 | 50 | 1.874 / 2.999 | 0 | — |
| warm-snapshot | mobile-agent | IPHONE-SE-UDID | swiftui | big | 5.807 / 6.543 | 50 | 0.488 / 0.560 | 0 | — |


