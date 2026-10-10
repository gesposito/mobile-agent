# Benchmark report

Started 2026-10-10T12:30:16Z, finished 2026-10-10T12:30:45Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 4.04 5.46 5.37 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 14be48ea9b077a0302aba6386251162459a605b7 (dirty) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | ios-device | iPhone14,6 | iOS 26.6.2 | — | — | yes |

### Physical iPhones

| Target | Runner | Input | Frames | Battery | Auto-Lock | Installed before the run |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent-ios-runner 0.2.0 | dtuhidd | screenshots | 100%, not charging, on external power | off, as stated | expo in 31656.878 ms |

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
| mobile-agent | IPHONE-SE-UDID | expo | — | flow big-screen | — | 0 | — | — | — | 100.0 | 0.0 | — |
| agent-device | IPHONE-SE-UDID | expo | — | flow big-screen | — | 0 | — | — | — | 100.0 | 0.0 | — |
| webdriveragent | IPHONE-SE-UDID | expo | — | flow big-screen | — | 0 | — | — | — | 100.0 | 0.0 | — |

Series that could not run:

- mobile-agent on IPHONE-SE-UDID, flow big-screen: the check run failed: mobile-agent open --relaunch -- dev.mobileagent.fixture.expo: launch_failed: DVT refused to launch dev.mobileagent.fixture.expo: com.apple.dt.deviceprocesscontrolservice 2: Request to launch dev.mobileagent.fixture.expo failed.
- agent-device on IPHONE-SE-UDID, flow big-screen: the check run failed: COMMAND_FAILED: Batch failed at step 1 (open): Failed to launch iOS app: The application failed to launch. (com.apple.dt.CoreDeviceError error 10002 (0x2712))
- webdriveragent on IPHONE-SE-UDID, flow big-screen: the check run failed: POST /session/75b1f743-3d77-4530-9aff-56cbe9da14b4/execute/sync failed with 400: invalid argument: Error Domain=FBSOpenApplicationServiceErrorDomain Code=1 "The request to open "dev.mobileagent.fixture.expo" failed." UserInfo={BSErrorCodeDescription=RequestDenied, NSUnderlyingError=0x107b222e0 {Error Domain=FBSOpenApplicationErrorDomain Code=3 "Unable to launch dev.mobileagent.fixture.expo because it has an invalid code signature, inadequate entitlements or its profile has not been explicitly trusted by the user." UserInfo={BSErrorCodeDescription=Security, NSLocalizedFailureReason=Unable to launch dev.mobileagent.fixture.expo because it has an invalid code signature, inadequate entitlements or its profile has not been explicitly trusted by the user.}}, NSLocalizedDescription=The request to open "dev.mobileagent.fixture.expo" failed., FBSOpenApplicationRequestID=0xc9e8, NSLocalizedFailureReason=The request was denied by service delegate (SBMainWorkspace) for reason: Security ("Unable to launch dev.mobileagent.fixture.expo because it has an invalid code signature, inadequate entitlements or its profile has not been explicitly trusted by the user").} / the check run failed: POST /session/75b1f743-3d77-4530-9aff-56cbe9da14b4/execute/sync failed with 400: invalid argument: Error Domain=FBSOpenApplicationServiceErrorDomain Code=1 "The request to open "dev.mobileagent.fixture.expo" failed." UserInfo={BSErrorCodeDescription=RequestDenied, NSUnderlyingError=0x107b22310 {Error Domain=FBSOpenApplicationErrorDomain Code=3 "Unable to launch dev.mobileagent.fixture.expo because it has an invalid code signature, inadequate entitlements or its profile has not been explicitly trusted by the user." UserInfo={BSErrorCodeDescription=Security, NSLocalizedFailureReason=Unable to launch dev.mobileagent.fixture.expo because it has an invalid code signature, inadequate entitlements or its profile has not been explicitly trusted by the user.}}, NSLocalizedDescription=The request to open "dev.mobileagent.fixture.expo" failed., FBSOpenApplicationRequestID=0x9f3c, NSLocalizedFailureReason=The request was denied by service delegate (SBMainWorkspace) for reason: Security ("Unable to launch dev.mobileagent.fixture.expo because it has an invalid code signature, inadequate entitlements or its profile has not been explicitly trusted by the user").}

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent | agent-device | webdriveragent |
|---|---|---|---|---|---|---|
| IPHONE-SE-UDID | expo | — | flow big-screen | no samples | no samples | no samples |

Skipped: maestro (Maestro runs iOS flows on Simulators only; 2.11.0 refuses physical iPhones, its iOS device driver unfinished); maestro-runner (maestro-runner not found on PATH).

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| IPHONE-SE-UDID | mobile-agent | agent-device | 1397.976 | stopped the daemon |
| IPHONE-SE-UDID | agent-device | webdriveragent | 3546.166 | stopped agent-device's daemon |
| IPHONE-SE-UDID | webdriveragent | end of the run | 1480.974 | deleted its Appium session, stopped Appium and the helper holding WebDriverAgent's test, and ended WebDriverAgent's runner app on the iPhone |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result | Note |
|---|---|---|---|---|---|---|---|
| ios-device cli-start | — | — | p95 | 20.000 | — | not measured | — |
| ios-device cold-attach | — | — | p95 | 5000.000 | — | not measured | the runner already installed on the phone |
| ios-device warm-snapshot big | — | — | p95 | 1000.000 | — | not measured | — |
| ios-device tap-dispatch | — | — | p95 | 150.000 | — | not measured | through the host's input path, dtuhidd |
| ios-device tap-to-frame-change | — | — | p95 | — | — | not measured | not gated while the phone's frames are polled screenshots |

All 0 gate checks passed. 5 not measured.

