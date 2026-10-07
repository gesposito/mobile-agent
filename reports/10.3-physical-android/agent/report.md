# Benchmark report

Started 2026-10-07T16:05:41Z, finished 2026-10-07T16:34:17Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 3.11 2.79 2.76 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 552c9e223f68233ed750457fb51826e85620bf8b (clean) |
| Harness | bench 0.1.0, 20 iterations after 2 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| NEXUS5X-SERIAL | android-device | Nexus 5X | Android API 27 | 27 | — | no |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | NEXUS5X-SERIAL | — | — | cli-start | wall | 20 | 4.592 | 6.325 | 7.455 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | expo | — | cold-attach | wall | 20 | 2193.680 | 13889.442 | 13889.820 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | expo | big | first-snapshot | wall | 20 | 7582.160 | 8351.627 | 8455.783 | 0.0 | 0.0 | 507 |
| mobile-agent | NEXUS5X-SERIAL | expo | big | warm-snapshot | wall | 20 | 6.206 | 6.997 | 7.551 | 0.0 | 0.0 | 507 |
| mobile-agent | NEXUS5X-SERIAL | expo | list | tap-dispatch | tool | 20 | 8.144 | 16.376 | 22.027 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | expo | list | tap-to-frame-change | — | 0 | — | — | — | — | — | — |
| mobile-agent | NEXUS5X-SERIAL | expo | list | tap-to-tree-delta | tool | 20 | 130.236 | 139.307 | 150.419 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | expo | login | text-entry | wall | 20 | 117.703 | 196.402 | 201.752 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | expo | big | screenshot | wall | 20 | 99.987 | 109.285 | 196.230 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | expo | list | settle | tool | 20 | 228.498 | 245.339 | 247.015 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | expo | — | flow login | wall | 20 | 2552.306 | 2597.342 | 2626.346 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | expo | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | expo | — | flow big-screen | wall | 20 | 9500.801 | 10226.790 | 10339.832 | 0.0 | 0.0 | 507 |
| mobile-agent | NEXUS5X-SERIAL | compose | — | cold-attach | wall | 20 | 3050.629 | 3131.542 | 3385.442 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | compose | big | first-snapshot | wall | 20 | 121.480 | 156.362 | 167.390 | 0.0 | 0.0 | 505 |
| mobile-agent | NEXUS5X-SERIAL | compose | big | warm-snapshot | wall | 20 | 5.919 | 8.694 | 10.171 | 0.0 | 0.0 | 505 |
| mobile-agent | NEXUS5X-SERIAL | compose | list | tap-dispatch | tool | 20 | 8.579 | 11.439 | 11.522 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | compose | list | tap-to-frame-change | — | 0 | — | — | — | — | — | — |
| mobile-agent | NEXUS5X-SERIAL | compose | list | tap-to-tree-delta | tool | 20 | 313.269 | 319.826 | 324.637 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | compose | login | text-entry | wall | 20 | 438.737 | 524.341 | 539.812 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | compose | big | screenshot | wall | 20 | 9981.054 | 11160.694 | 11323.120 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | compose | list | settle | tool | 20 | 186.057 | 213.445 | 333.555 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | compose | — | flow login | wall | 20 | 7450.201 | 8114.697 | 8146.543 | 0.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | compose | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | NEXUS5X-SERIAL | compose | — | flow big-screen | — | 0 | — | — | — | 100.0 | 0.0 | — |

Series that could not run:

- mobile-agent on NEXUS5X-SERIAL, flow list-scroll: the check run failed: mobile-agent scroll --timeout 90000 --until id=row-200 -- down 600: element_not_found: id=row-200 was on screen but not mostly in view (cut off at the bottom edge of its scroll container, which scrolls on) after 25 strokes of 460.6 points down: the content stopped moving; take a snapshot to see where the content is / the check run failed: mobile-agent scroll --timeout 90000 --until id=row-200 -- down 600: element_not_found: id=row-200 was on screen but not mostly in view (cut off at the bottom edge of its scroll container, which scrolls on) after 24 strokes of 460.6 points down: the content stopped moving; take a snapshot to see where the content is
- mobile-agent on NEXUS5X-SERIAL, flow list-scroll: the check run failed: mobile-agent click --timeout 10000 -- id=tab-list: element_not_found: no element matches id tab-list after 10008 ms; the screen shows revision 75, 4 windows, 5 elements with refs, read 157 ms ago: / the check run failed: mobile-agent click --timeout 10000 -- id=tab-list: element_not_found: no element matches id tab-list after 10005 ms; the screen shows revision 77, 4 windows, 5 elements with refs, read 442 ms ago:
- mobile-agent on NEXUS5X-SERIAL, flow big-screen: the check run failed: mobile-agent click --timeout 10000 -- id=tab-big: element_not_found: no element matches id tab-big after 10009 ms; the screen shows revision 78, 4 windows, 5 elements with refs, read 29 ms ago: / the check run failed: mobile-agent click --timeout 10000 -- id=tab-big: element_not_found: no element matches id tab-big after 10010 ms; the screen shows revision 79, 4 windows, 5 elements with refs, read 180 ms ago:

Series not measured:

- mobile-agent on NEXUS5X-SERIAL, tap-to-frame-change: Android devices have no frame stream, only screenshots on demand (design D7), so a tap on one has no frame change to time
- mobile-agent on NEXUS5X-SERIAL, tap-to-frame-change: Android devices have no frame stream, only screenshots on demand (design D7), so a tap on one has no frame change to time

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent |
|---|---|---|---|---|
| NEXUS5X-SERIAL | — | — | cli-start | 4.592 / 6.325 |
| NEXUS5X-SERIAL | expo | — | cold-attach | 2193.680 / 13889.442 |
| NEXUS5X-SERIAL | expo | big | first-snapshot | 7582.160 / 8351.627 |
| NEXUS5X-SERIAL | expo | big | warm-snapshot | 6.206 / 6.997 |
| NEXUS5X-SERIAL | expo | list | tap-dispatch | 8.144 / 16.376 |
| NEXUS5X-SERIAL | expo | list | tap-to-frame-change | not measured |
| NEXUS5X-SERIAL | expo | list | tap-to-tree-delta | 130.236 / 139.307 |
| NEXUS5X-SERIAL | expo | login | text-entry | 117.703 / 196.402 |
| NEXUS5X-SERIAL | expo | big | screenshot | 99.987 / 109.285 |
| NEXUS5X-SERIAL | expo | list | settle | 228.498 / 245.339 |
| NEXUS5X-SERIAL | expo | — | flow login | 2552.306 / 2597.342 |
| NEXUS5X-SERIAL | expo | — | flow list-scroll | no samples |
| NEXUS5X-SERIAL | expo | — | flow big-screen | 9500.801 / 10226.790 |
| NEXUS5X-SERIAL | compose | — | cold-attach | 3050.629 / 3131.542 |
| NEXUS5X-SERIAL | compose | big | first-snapshot | 121.480 / 156.362 |
| NEXUS5X-SERIAL | compose | big | warm-snapshot | 5.919 / 8.694 |
| NEXUS5X-SERIAL | compose | list | tap-dispatch | 8.579 / 11.439 |
| NEXUS5X-SERIAL | compose | list | tap-to-frame-change | not measured |
| NEXUS5X-SERIAL | compose | list | tap-to-tree-delta | 313.269 / 319.826 |
| NEXUS5X-SERIAL | compose | login | text-entry | 438.737 / 524.341 |
| NEXUS5X-SERIAL | compose | big | screenshot | 9981.054 / 11160.694 |
| NEXUS5X-SERIAL | compose | list | settle | 186.057 / 213.445 |
| NEXUS5X-SERIAL | compose | — | flow login | 7450.201 / 8114.697 |
| NEXUS5X-SERIAL | compose | — | flow list-scroll | no samples |
| NEXUS5X-SERIAL | compose | — | flow big-screen | no samples |

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, the run waits until nothing that tool keeps for the target is left running, ending itself what the release did not reach, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| NEXUS5X-SERIAL | mobile-agent | end of the run | 191.515 | stopped the daemon |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-device cli-start | mobile-agent | NEXUS5X-SERIAL | p95 | 20.000 | 6.325 | pass |
| android-device cold-attach | mobile-agent | NEXUS5X-SERIAL | p95 | 1500.000 | 13889.442 | FAIL |
| android-device cold-attach | mobile-agent | NEXUS5X-SERIAL | p95 | 1500.000 | 3131.542 | FAIL |
| android-device warm-snapshot big | mobile-agent | NEXUS5X-SERIAL | p95 | 100.000 | 6.997 | pass |
| android-device warm-snapshot big | mobile-agent | NEXUS5X-SERIAL | p95 | 100.000 | 8.694 | pass |
| android-device tap-dispatch | mobile-agent | NEXUS5X-SERIAL | p95 | 50.000 | 16.376 | pass |
| android-device tap-dispatch | mobile-agent | NEXUS5X-SERIAL | p95 | 50.000 | 11.439 | pass |

2 of 7 gate checks failed.

