# Spec Delta

## ADDED Requirements

### Requirement: Benchmarks in CI on a schedule or on demand
The microbenchmarks and the comparison harness SHALL run in CI only on a schedule or on a manual dispatch, never on a push or a pull request, and a dispatch SHALL take the suite to run, the two refs to compare and the iterations; the run SHALL publish its tables in the job's summary and its raw reports as artifacts, and a scheduled run SHALL compare itself with the previous scheduled run's artifact.

#### Scenario: Push runs no benchmark
- **WHEN** a commit is pushed or a pull request opened
- **THEN** the validation workflow runs and no benchmark job starts

#### Scenario: Dispatch against two refs
- **WHEN** the workflow is dispatched with a base ref and a head ref for the microbenchmarks
- **THEN** both are built on one runner, run interleaved, and the summary table lists each benchmark's medians, allocations and verdict

### Requirement: Two refs compared on one runner
A comparison of two refs SHALL build both on the same runner and run their suites interleaved, SHALL judge each benchmark by the relative difference of the two medians against a threshold set from the runner's measured noise and by exact allocation counts, and SHALL fail the run only on a regression past the threshold; a comparison against the committed baseline SHALL stay the local judgement and SHALL not be used on a shared runner.

#### Scenario: Noise under the threshold
- **WHEN** a benchmark's head median is 4% slower than its base median on a runner whose measured noise set the threshold at 10%
- **THEN** the benchmark is listed as the same and the run passes

#### Scenario: Regression past the threshold
- **WHEN** a benchmark's head median is 20% slower than its base median, or its allocations differ
- **THEN** the benchmark is listed as a regression with both values and the run fails

### Requirement: Harness comparison in CI by ratios
On a runner the harness SHALL run a reduced plan, one fixture per platform with fewer iterations, on a Simulator and on an emulator, with the baseline tools installed at pinned versions, and SHALL judge mobile-agent by its ratios against each baseline within that run rather than by the absolute gates, which apply only on the maintainer's measured hosts; a baseline that cannot run on the runner SHALL be skipped with a note, as on a host.

#### Scenario: Weekly comparison
- **WHEN** the scheduled run completes on a Simulator and an emulator
- **THEN** the summary lists, per flow and platform, mobile-agent's time and its ratio against each baseline that ran, and the artifact holds the full reports
