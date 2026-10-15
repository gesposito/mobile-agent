# Benchmark report

Started 2026-10-11T09:27:26Z, finished 2026-10-11T09:34:08Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.29 2.98 3.15 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 97a6b1567f408217a1fd3c310a73f9a9f8fdbdb7 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | swiftui in 2488.250 ms |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | IPHONE-SE-UDID | swiftui | login | text-entry | wall | 39 | 1595.446 | 2270.767 | 2273.086 | 22.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | big | screenshot | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | IPHONE-SE-UDID | swiftui | list | settle | — | 0 | — | — | — | 100.0 | 0.0 | — |

Series that could not run:

- mobile-agent on IPHONE-SE-UDID, screenshot: preparation failed: mobile-agent open --relaunch --env FIXTURE_INITIAL_TAB=big -- dev.mobileagent.fixture.swiftui: busy: the runner refused snapshot: the runner is still serving request 294
- mobile-agent on IPHONE-SE-UDID, settle: preparation failed: mobile-agent open --relaunch --env FIXTURE_INITIAL_TAB=list -- dev.mobileagent.fixture.swiftui: busy: the runner refused snapshot: the runner is still serving request 294

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| IPHONE-SE-UDID | swiftui | login | text-entry | 1595.446 / 2270.767 |
| IPHONE-SE-UDID | swiftui | big | screenshot | no samples |
| IPHONE-SE-UDID | swiftui | list | settle | no samples |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | end of the run | 7105.202 | stopped the daemon; failed: mobile-agent left runner app on the iPhone (pid 21968) running after its release; the hand-off sent it SIGTERM, and it exited |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | — | — | p95 | 20.000 | — | not measured | — |
| ios-device cold-attach | — | — | p95 | 5000.000 | — | not measured | the runner already installed on the phone |
| ios-device warm-snapshot big | — | — | p95 | 1000.000 | — | not measured | — |
| ios-device tap-dispatch | — | — | p95 | 150.000 | — | not measured | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | — | — | p95 | — | — | not measured | not gated while the phone's frames are polled screenshots |

All 0 gate checks passed. 5 not measured.

