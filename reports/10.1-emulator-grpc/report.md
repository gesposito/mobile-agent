# Benchmark report

Started 2026-09-27T11:51:49Z, finished 2026-09-27T14:37:01Z. Durations are milliseconds; percentiles are nearest-rank over the measured iterations, with warm-up iterations excluded. Fail % is the share of measured iterations in which no attempt succeeded, Flake % the share that succeeded only on a retry.

## Provenance

| Field | Value |
|---|---|
| Host hardware | Apple M1 Max |
| Host CPU cores | 10 |
| Host memory bytes | 68719476736 |
| Host OS | macOS 26.6.2 (25G83) |
| Load average at start | 6.35 51.28 65.82 |
| Xcode | Xcode 27.0 (27A266a) |
| Commit under test | 60e528a9137b1298b42cbe7ce2ab9143b98ea0c6 (clean) |
| Harness | bench 0.1.0, 50 iterations after 3 warm-up, 1 retry |

### Targets

| Target | Class | Model | OS | Android API level |
|---|---|---|---|---|
| emulator-5554 | android-emulator | sdk_gphone64_arm64 | Android API 36 | 36 |

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
| mobile-agent | emulator-5554 | — | — | cli-start | wall | 50 | 8.728 | 11.594 | 15.441 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | cold-attach | wall | 50 | 800.735 | 894.241 | 989.266 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | big | first-snapshot | wall | 50 | 473.933 | 615.236 | 693.409 | 0.0 | 0.0 | 506 |
| mobile-agent | emulator-5554 | expo | big | warm-snapshot | wall | 50 | 11.087 | 12.776 | 27.669 | 0.0 | 0.0 | 506 |
| mobile-agent | emulator-5554 | expo | list | tap-dispatch | tool | 50 | 2.234 | 2.811 | 2.839 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | tap-to-frame-change | tool | 50 | 45.366 | 52.456 | 53.375 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | tap-to-tree-delta | tool | 50 | 235.924 | 244.952 | 261.588 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | login | text-entry | wall | 50 | 23.822 | 50.960 | 56.438 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | big | screenshot | wall | 50 | 36.433 | 48.144 | 61.565 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | list | settle | tool | 50 | 191.291 | 197.560 | 198.605 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow login | wall | 50 | 1577.568 | 1611.768 | 1624.373 | 0.0 | 0.0 | — |
| maestro | emulator-5554 | expo | — | flow login | wall | 50 | 22869.445 | 24207.272 | 24898.952 | 0.0 | 0.0 | — |
| agent-device | emulator-5554 | expo | — | flow login | wall | 34 | 3187.059 | 4619.687 | 4628.106 | 32.0 | 22.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow list-scroll | wall | 50 | 14200.387 | 15038.074 | 24567.169 | 0.0 | 0.0 | — |
| maestro | emulator-5554 | expo | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| agent-device | emulator-5554 | expo | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | emulator-5554 | expo | — | flow big-screen | wall | 50 | 2009.698 | 2161.855 | 2884.387 | 0.0 | 0.0 | 506 |
| maestro | emulator-5554 | expo | — | flow big-screen | wall | 50 | 16859.273 | 19462.493 | 20431.837 | 0.0 | 0.0 | 1052 |
| agent-device | emulator-5554 | expo | — | flow big-screen | wall | 12 | 2385.940 | 2751.548 | 2751.548 | 76.0 | 12.0 | 1029 |
| mobile-agent | emulator-5554 | compose | — | cold-attach | wall | 50 | 1171.721 | 1208.246 | 1429.137 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | big | first-snapshot | wall | 50 | 47.149 | 79.478 | 201.764 | 0.0 | 0.0 | 504 |
| mobile-agent | emulator-5554 | compose | big | warm-snapshot | wall | 50 | 9.648 | 19.399 | 24.751 | 0.0 | 0.0 | 504 |
| mobile-agent | emulator-5554 | compose | list | tap-dispatch | tool | 50 | 2.598 | 3.321 | 3.539 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | list | tap-to-frame-change | tool | 50 | 31.741 | 55.119 | 57.226 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | list | tap-to-tree-delta | tool | 50 | 226.377 | 236.191 | 316.090 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | login | text-entry | wall | 50 | 26.090 | 46.372 | 51.360 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | big | screenshot | wall | 50 | 36.390 | 40.157 | 43.094 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | list | settle | tool | 50 | 188.372 | 199.119 | 339.655 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow login | wall | 50 | 3369.985 | 3576.701 | 6301.296 | 0.0 | 0.0 | — |
| maestro | emulator-5554 | compose | — | flow login | wall | 49 | 20177.350 | 21366.389 | 22385.922 | 2.0 | 2.0 | — |
| agent-device | emulator-5554 | compose | — | flow login | wall | 50 | 5869.681 | 5975.332 | 5999.918 | 0.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow list-scroll | wall | 50 | 15333.840 | 15519.930 | 15781.736 | 0.0 | 0.0 | — |
| maestro | emulator-5554 | compose | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| agent-device | emulator-5554 | compose | — | flow list-scroll | — | 0 | — | — | — | 100.0 | 0.0 | — |
| mobile-agent | emulator-5554 | compose | — | flow big-screen | wall | 50 | 3776.435 | 3875.951 | 3956.710 | 0.0 | 0.0 | 504 |
| maestro | emulator-5554 | compose | — | flow big-screen | wall | 50 | 16331.122 | 17215.185 | 18048.692 | 0.0 | 0.0 | 571 |
| agent-device | emulator-5554 | compose | — | flow big-screen | wall | 50 | 3987.156 | 4066.009 | 4082.419 | 0.0 | 0.0 | 528 |

Series that could not run:

- maestro on emulator-5554, flow list-scroll: the check run failed: exit status 1: - `centerElement`: current = false → Enable if you want the element to be centered after finding it / ==== Debug output (logs & screenshots) ==== / $HOME/.maestro/tests/2026-09-27_135253 / the check run failed: exit status 1: - `centerElement`: current = false → Enable if you want the element to be centered after finding it / ==== Debug output (logs & screenshots) ==== / $HOME/.maestro/tests/2026-09-27_135330
- agent-device on emulator-5554, flow list-scroll: the check run failed: COMMAND_FAILED: Batch failed at step 3 (scroll): scroll down spent its 12-pass budget without id="row-200" becoming visible
- maestro on emulator-5554, flow list-scroll: the check run failed: exit status 1: - `centerElement`: current = false → Enable if you want the element to be centered after finding it / ==== Debug output (logs & screenshots) ==== / $HOME/.maestro/tests/2026-09-27_135600 / the check run failed: exit status 1: - `centerElement`: current = false → Enable if you want the element to be centered after finding it / ==== Debug output (logs & screenshots) ==== / $HOME/.maestro/tests/2026-09-27_135634
- agent-device on emulator-5554, flow list-scroll: the check run failed: COMMAND_FAILED: Batch failed at step 3 (scroll): scroll down reached the end of the scrollable content after 0 passes without id="row-200" becoming visible

## Comparison

p50 / p95 in milliseconds per tool and target; — where a tool did not measure the subject.

| Target | Fixture | Screen | Subject | mobile-agent | maestro | agent-device |
|---|---|---|---|---|---|---|
| emulator-5554 | — | — | cli-start | 8.728 / 11.594 | — | — |
| emulator-5554 | expo | — | cold-attach | 800.735 / 894.241 | — | — |
| emulator-5554 | expo | big | first-snapshot | 473.933 / 615.236 | — | — |
| emulator-5554 | expo | big | warm-snapshot | 11.087 / 12.776 | — | — |
| emulator-5554 | expo | list | tap-dispatch | 2.234 / 2.811 | — | — |
| emulator-5554 | expo | list | tap-to-frame-change | 45.366 / 52.456 | — | — |
| emulator-5554 | expo | list | tap-to-tree-delta | 235.924 / 244.952 | — | — |
| emulator-5554 | expo | login | text-entry | 23.822 / 50.960 | — | — |
| emulator-5554 | expo | big | screenshot | 36.433 / 48.144 | — | — |
| emulator-5554 | expo | list | settle | 191.291 / 197.560 | — | — |
| emulator-5554 | expo | — | flow login | 1577.568 / 1611.768 | 22869.445 / 24207.272 | 3187.059 / 4619.687 |
| emulator-5554 | expo | — | flow list-scroll | 14200.387 / 15038.074 | no samples | no samples |
| emulator-5554 | expo | — | flow big-screen | 2009.698 / 2161.855 | 16859.273 / 19462.493 | 2385.940 / 2751.548 |
| emulator-5554 | compose | — | cold-attach | 1171.721 / 1208.246 | — | — |
| emulator-5554 | compose | big | first-snapshot | 47.149 / 79.478 | — | — |
| emulator-5554 | compose | big | warm-snapshot | 9.648 / 19.399 | — | — |
| emulator-5554 | compose | list | tap-dispatch | 2.598 / 3.321 | — | — |
| emulator-5554 | compose | list | tap-to-frame-change | 31.741 / 55.119 | — | — |
| emulator-5554 | compose | list | tap-to-tree-delta | 226.377 / 236.191 | — | — |
| emulator-5554 | compose | login | text-entry | 26.090 / 46.372 | — | — |
| emulator-5554 | compose | big | screenshot | 36.390 / 40.157 | — | — |
| emulator-5554 | compose | list | settle | 188.372 / 199.119 | — | — |
| emulator-5554 | compose | — | flow login | 3369.985 / 3576.701 | 20177.350 / 21366.389 | 5869.681 / 5975.332 |
| emulator-5554 | compose | — | flow list-scroll | 15333.840 / 15519.930 | no samples | no samples |
| emulator-5554 | compose | — | flow big-screen | 3776.435 / 3875.951 | 16331.122 / 17215.185 | 3987.156 / 4066.009 |

Skipped: maestro-runner (maestro-runner not found on PATH).

## Gates

| Gate | Tool | Target | Statistic | Limit ms | Measured ms | Result |
|---|---|---|---|---|---|---|
| android-emulator cli-start | mobile-agent | emulator-5554 | p95 | 20.000 | 11.594 | pass |
| android-emulator cold-attach | mobile-agent | emulator-5554 | p95 | 1500.000 | 894.241 | pass |
| android-emulator cold-attach | mobile-agent | emulator-5554 | p95 | 1500.000 | 1208.246 | pass |
| android-emulator warm-snapshot big | mobile-agent | emulator-5554 | p95 | 100.000 | 12.776 | pass |
| android-emulator warm-snapshot big | mobile-agent | emulator-5554 | p95 | 100.000 | 19.399 | pass |
| android-emulator tap-to-frame-change | mobile-agent | emulator-5554 | p95 | 50.000 | 52.456 | FAIL |
| android-emulator tap-to-frame-change | mobile-agent | emulator-5554 | p95 | 50.000 | 55.119 | FAIL |

2 of 7 gate checks failed.

