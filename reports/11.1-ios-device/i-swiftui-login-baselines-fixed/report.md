# Benchmark report

Started 2026-10-11T16:31:46Z, finished 2026-10-11T16:47:03Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 3.06 5.14 4.97 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 4f2548fd3a788f008dfb2b25980df4715c519fa3 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | — |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | — | — | — | — | off, as stated | swiftui in 2513.456 ms |

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
| mobile-agent | IPHONE-SE-UDID | swiftui | — | flow login | — | 0 | — | — | — | 100.0 | 0.0 | — |
| agent-device | IPHONE-SE-UDID | swiftui | — | flow login | wall | 50 | 11335.780 | 11681.985 | 11931.130 | 0.0 | 0.0 | — |
| webdriveragent | IPHONE-SE-UDID | swiftui | — | flow login | — | 0 | — | — | — | 100.0 | 0.0 | — |

Series that could not run:

- mobile-agent on IPHONE-SE-UDID, flow login: the check run failed: mobile-agent open --relaunch -- dev.mobileagent.fixture.swiftui: device_error: DVT's capabilities failed on the device: device socket io failed: remote server connection closed
- webdriveragent on IPHONE-SE-UDID, flow login: the check run failed: no reply to POST /session/eb8f210a-75a5-4b44-98ca-b159e2eab76c/element/15000000-0000-0000-FE08-000000000000/click: Resource temporarily unavailable (os error 35) / the check run failed: no reply to POST /session/eb8f210a-75a5-4b44-98ca-b159e2eab76c/execute/sync: Resource temporarily unavailable (os error 35)

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent | agent-device | webdriveragent |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | swiftui | — | flow login | no samples | 11335.780 / 11681.985 | no samples |

Skipped: maestro (Maestro runs iOS flows on Simulators only; 2.11.0 refuses physical iPhones, its iOS device driver unfinished); maestro-runner (maestro-runner not found on PATH).

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | agent-device | 424.848 | stopped the daemon |
| IPHONE-SE-UDID | agent-device | webdriveragent | 1881.078 | closed agent-device session mobile-agent-bench and stopped its daemon |
| IPHONE-SE-UDID | webdriveragent | agent-device | 2126.172 | deleted its Appium session, stopped Appium and the helper holding WebDriverAgent's test, and ended WebDriverAgent's runner app on the iPhone |
| IPHONE-SE-UDID | agent-device | end of the run | 1683.026 | closed agent-device session mobile-agent-bench and stopped its daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | — | — | p95 | 20.000 | — | not measured | — |
| ios-device cold-attach | — | — | p95 | 5000.000 | — | not measured | the runner already installed on the phone |
| ios-device warm-snapshot big | — | — | p95 | 1000.000 | — | not measured | — |
| ios-device tap-dispatch | — | — | p95 | 150.000 | — | not measured | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | — | — | p95 | — | — | not measured | not gated while the phone's frames are polled screenshots |

All 0 gate checks passed. 5 not measured.

