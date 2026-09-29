# Benchmark report

Started 2026-09-28T17:46:10Z, finished 2026-09-29T00:17:34Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 1.70 15.63 37.58 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | e84d86e11072afabef2a53bef37aa78e73beced3 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level | Emulator controller | Frame source |
|---|---|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | simulator | iPhone 17 | iOS 26.5 | — | — | yes |

### Tools

| Tool | Role | Status | Version | Note |
|---|---|---|---|---|
| mobile-agent | under test | available | 0.1.0 | — |
| maestro | baseline | available | 2.10.0 | — |
| maestro-runner | baseline | skipped | — | maestro-runner not found on PATH |
| agent-device | baseline | available | 0.21.14 | — |

## Measurements

| Tool | Target | Fixture | Screen | Subject | Clock | n | p50 ms | p95 ms | max ms | Fail % | Flake % | Nodes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | — | — | cli-start | wall | 50 | 4.628 | 5.882 | 6.334 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | cold-attach | wall | 50 | 922.166 | 946.969 | 969.254 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | big | first-snapshot | wall | 50 | 314.079 | 319.470 | 320.650 | 0.0 | 0.0 | 506 |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | big | warm-snapshot | wall | 50 | 6.131 | 7.880 | 10.240 | 0.0 | 0.0 | 506 |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | list | tap-dispatch | tool | 50 | 0.067 | 1.555 | 3.050 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | list | tap-to-frame-change | tool | 50 | 72.754 | 75.279 | 75.597 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | list | tap-to-tree-delta | tool | 50 | 343.197 | 349.139 | 349.768 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | login | text-entry | wall | 50 | 249.269 | 293.858 | 701.746 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | big | screenshot | wall | 50 | 13.114 | 17.571 | 20.052 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | list | settle | tool | 50 | 435.891 | 669.026 | 675.171 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | wall | 50 | 3817.086 | 4039.316 | 4096.643 | 0.0 | 0.0 | — |
| maestro | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | wall | 50 | 25264.378 | 28227.140 | 33065.128 | 0.0 | 0.0 | — |
| agent-device | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | wall | 50 | 8665.957 | 8748.529 | 8865.890 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow list-scroll | wall | 50 | 16327.813 | 16805.219 | 17237.446 | 0.0 | 0.0 | — |
| maestro | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow list-scroll | wall | 50 | 98990.612 | 100193.625 | 100972.600 | 0.0 | 0.0 | — |
| agent-device | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow big-screen | wall | 50 | 2491.781 | 2691.962 | 2701.129 | 0.0 | 0.0 | 506 |
| maestro | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow big-screen | wall | 50 | 21216.758 | 22189.136 | 22526.782 | 0.0 | 0.0 | 558 |
| agent-device | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow big-screen | wall | 50 | 3079.880 | 3307.469 | 3657.444 | 0.0 | 0.0 | 513 |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | cold-attach | wall | 50 | 809.217 | 837.939 | 883.413 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | big | first-snapshot | wall | 50 | 272.296 | 284.457 | 284.717 | 0.0 | 0.0 | 507 |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | big | warm-snapshot | wall | 50 | 5.860 | 7.652 | 7.911 | 0.0 | 0.0 | 507 |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | list | tap-dispatch | tool | 50 | 0.119 | 0.178 | 0.183 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | list | tap-to-frame-change | tool | 50 | 47.675 | 52.689 | 54.523 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | list | tap-to-tree-delta | tool | 50 | 254.600 | 280.615 | 283.154 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | login | text-entry | wall | 50 | 228.342 | 256.898 | 266.056 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | big | screenshot | wall | 50 | 13.857 | 18.525 | 271.028 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | list | settle | tool | 50 | 190.194 | 198.879 | 221.518 | 0.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | wall | 50 | 3706.834 | 4101.510 | 4153.675 | 0.0 | 0.0 | — |
| maestro | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | wall | 50 | 24627.613 | 26213.922 | 26912.780 | 0.0 | 0.0 | — |
| agent-device | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow list-scroll | wall | 50 | 14995.416 | 15678.675 | 15778.147 | 0.0 | 0.0 | — |
| maestro | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow list-scroll | wall | 37 | 90741.125 | 93214.853 | 94177.216 | 26.0 | 22.0 | — |
| agent-device | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow big-screen | wall | 50 | 2761.071 | 2974.261 | 2979.662 | 0.0 | 0.0 | 507 |
| maestro | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow big-screen | wall | 50 | 19672.750 | 20980.381 | 21895.108 | 0.0 | 0.0 | 539 |
| agent-device | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow big-screen | — | 0 | — | — | — | 100.0 | 0.0 | — |

Series that could not run:

- agent-device on 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683, flow list-scroll: the check run failed: COMMAND_FAILED: Batch failed at step 3 (scroll): scroll down spent its 12-pass budget without id="row-200" becoming visible
- agent-device on 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683, flow login: the check run failed: COMMAND_FAILED: Batch failed at step 2 (fill): Selector did not match: id="login-username"
- agent-device on 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683, flow list-scroll: the check run failed: COMMAND_FAILED: Batch failed at step 3 (scroll): scroll down spent its 12-pass budget without id="row-200" becoming visible / the check run failed: COMMAND_FAILED: Batch failed at step 2 (press): Selector did not match: id="tab-list"
- agent-device on 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683, flow big-screen: the check run failed: COMMAND_FAILED: Batch failed at step 2 (press): Selector did not match: id="tab-big"

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent | maestro | agent-device |
|---|---|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | — | — | cli-start | 4.628 / 5.882 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | cold-attach | 922.166 / 946.969 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | big | first-snapshot | 314.079 / 319.470 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | big | warm-snapshot | 6.131 / 7.880 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | list | tap-dispatch | 0.067 / 1.555 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | list | tap-to-frame-change | 72.754 / 75.279 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | list | tap-to-tree-delta | 343.197 / 349.139 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | login | text-entry | 249.269 / 293.858 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | big | screenshot | 13.114 / 17.571 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | list | settle | 435.891 / 669.026 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow login | 3817.086 / 4039.316 | 25264.378 / 28227.140 | 8665.957 / 8748.529 |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow list-scroll | 16327.813 / 16805.219 | 98990.612 / 100193.625 | no samples |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | swiftui | — | flow big-screen | 2491.781 / 2691.962 | 21216.758 / 22189.136 | 3079.880 / 3307.469 |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | cold-attach | 809.217 / 837.939 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | big | first-snapshot | 272.296 / 284.457 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | big | warm-snapshot | 5.860 / 7.652 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | list | tap-dispatch | 0.119 / 0.178 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | list | tap-to-frame-change | 47.675 / 52.689 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | list | tap-to-tree-delta | 254.600 / 280.615 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | login | text-entry | 228.342 / 256.898 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | big | screenshot | 13.857 / 18.525 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | list | settle | 190.194 / 198.879 | — | — |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow login | 3706.834 / 4101.510 | 24627.613 / 26213.922 | no samples |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow list-scroll | 14995.416 / 15678.675 | 90741.125 / 93214.853 | no samples |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | expo | — | flow big-screen | 2761.071 / 2974.261 | 19672.750 / 20980.381 | no samples |

Skipped: maestro-runner (maestro-runner not found on PATH).

## Hand-offs

Between series, before another tool drives a target and once the run is over, the tool that drove it lets go of it, and an Android target gets back the input method the run found.

| Target | From | To | ms | What happened |
|---|---|---|---|---|
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 326.140 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 143.865 | closed agent-device session mobile-agent-bench |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 211.729 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 155.650 | closed agent-device session mobile-agent-bench |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 247.229 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 158.739 | closed agent-device session mobile-agent-bench |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 314.536 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.001 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 185.706 | closed agent-device session mobile-agent-bench |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 238.509 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 186.752 | closed agent-device session mobile-agent-bench |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 388.856 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 190.243 | closed agent-device session mobile-agent-bench |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 201.638 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 152.351 | closed agent-device session mobile-agent-bench |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 327.912 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | mobile-agent | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 262.439 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | agent-device | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | agent-device | mobile-agent | 145.065 | closed agent-device session mobile-agent-bench |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 323.980 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | mobile-agent | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 187.261 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | mobile-agent | 0.000 | nothing held |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | mobile-agent | maestro | 190.437 | stopped the daemon |
| 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | maestro | end of the run | 0.001 | nothing held |

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| simulator cli-start | mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | p95 | 20.000 | 5.882 | pass |
| simulator cold-attach | mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | p95 | 2000.000 | 946.969 | pass |
| simulator cold-attach | mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | p95 | 2000.000 | 837.939 | pass |
| simulator warm-snapshot swiftui big | mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | p95 | 150.000 | 7.880 | pass |
| simulator warm-snapshot expo big | mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | p95 | 250.000 | 7.652 | pass |
| simulator tap-dispatch | mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | p95 | 20.000 | 1.555 | pass |
| simulator tap-dispatch | mobile-agent | 7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 | p95 | 20.000 | 0.178 | pass |

All 7 gate checks passed.

