# Benchmark report

Started 2026-10-11T20:42:51Z, finished 2026-10-11T21:00:20Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 2.96 5.27 5.87 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 437e6aa52d13781c5d5662a06100512b26602acf (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | expo in 2727.328 ms |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |
| maestro | baseline | skipped | — | Maestro runs iOS flows on Simulators only; 2.11.0 refuses physical iPhones, its iOS device driver unfinished |
| maestro-runner | baseline | skipped | — | maestro-runner not found on PATH |
| agent-device | baseline | available | 0.21.14 | — |
| webdriveragent | baseline | available | 16.14.1 (Appium 3.8.0, XCUITest driver 12.16.0) | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | IPHONE-SE-UDID | expo | — | flow big-screen | wall | 50 | 3322.377 | 4262.107 | 4388.065 | 0.0 | 0.0 | 506 |
| agent-device | IPHONE-SE-UDID | expo | — | flow big-screen | wall | 50 | 3891.184 | 4165.737 | 4208.096 | 0.0 | 6.0 | 514 |
| webdriveragent | IPHONE-SE-UDID | expo | — | flow big-screen | wall | 50 | 3790.734 | 3841.252 | 3937.479 | 0.0 | 0.0 | 529 |

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent | agent-device | webdriveragent |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | expo | — | flow big-screen | 3322.377 / 4262.107 | 3891.184 / 4165.737 | 3790.734 / 3841.252 |

Skipped: maestro (Maestro runs iOS flows on Simulators only; 2.11.0 refuses physical iPhones, its iOS device driver unfinished); maestro-runner (maestro-runner not found on PATH).

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | agent-device | 1257.601 | stopped the daemon |
| IPHONE-SE-UDID | agent-device | webdriveragent | 1885.988 | closed agent-device session mobile-agent-bench and stopped its daemon |
| IPHONE-SE-UDID | webdriveragent | mobile-agent | 2435.405 | deleted its Appium session, stopped Appium and the helper holding WebDriverAgent's test, and ended WebDriverAgent's runner app on the iPhone |
| IPHONE-SE-UDID | mobile-agent | agent-device | 2486.449 | stopped the daemon |
| IPHONE-SE-UDID | agent-device | webdriveragent | 1746.563 | closed agent-device session mobile-agent-bench and stopped its daemon |
| IPHONE-SE-UDID | webdriveragent | end of the run | 2330.738 | deleted its Appium session, stopped Appium and the helper holding WebDriverAgent's test, and ended WebDriverAgent's runner app on the iPhone |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | — | — | p95 | 20.000 | — | not measured | — |
| ios-device cold-attach | — | — | p95 | 5000.000 | — | not measured | the runner already installed on the phone |
| ios-device warm-snapshot big | — | — | p95 | 1000.000 | — | not measured | — |
| ios-device tap-dispatch | — | — | p95 | 150.000 | — | not measured | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | — | — | p95 | — | — | not measured | not gated while the phone's frames are polled screenshots |

All 0 gate checks passed. 5 not measured.

