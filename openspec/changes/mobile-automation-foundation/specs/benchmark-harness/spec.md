# Spec Delta

## Purpose

Measures the latency and reliability of every primitive and of end-to-end flows against explicit gates and against existing tools on the same flows, so that speed claims are verified rather than asserted.

## ADDED Requirements

### Requirement: Measured primitives
The harness SHALL measure CLI process start, cold attach, first snapshot after application launch, warm snapshot, tap-to-frame-change, tap-to-tree-delta, text entry of a fixed string, screenshot and settle detection, and SHALL report p50, p95, maximum and sample count for each.

#### Scenario: Primitive report
- **WHEN** a run completes on one target
- **THEN** every listed primitive appears in the report with its percentiles and sample count

### Requirement: Latency gates
Gates SHALL be configurable per target class with these defaults: CLI process start under 20 ms; Simulator warm snapshot p95 under 150 ms on the SwiftUI reference screen and under 250 ms on the React Native one, whose own accessibility snapshot costs about 236 ms, and tap dispatch p95 under 20 ms; Android emulator warm snapshot p95 under 100 ms on the reference screen and tap-to-frame-change p95 under 50 ms; cold attach under 2 s on the Simulator and under 1.5 s on Android. A run that misses any gate SHALL exit non-zero and print the failing gates in a table.

#### Scenario: Gate missed
- **WHEN** Simulator warm snapshot p95 measures 210 ms
- **THEN** the run exits non-zero and the table names the gate, the limit and the measured value

#### Scenario: All gates met
- **WHEN** every measured value is within its gate
- **THEN** the run exits zero and the table shows each gate as passed

### Requirement: Baseline tools on the same flows
The harness SHALL execute the same flows through Maestro, maestro-runner and agent-device when they are installed, SHALL skip an absent tool with a note, and SHALL include their results in the comparison table.

#### Scenario: One baseline missing
- **WHEN** maestro-runner is not installed
- **THEN** the report notes it as skipped and still compares the tools that ran

### Requirement: Reference fixtures
The harness SHALL ship reference applications for React Native, Jetpack Compose and SwiftUI with deterministic screens, including one screen of at least 500 accessibility nodes, and flows described once and executed through each tool's adapter.

#### Scenario: Large screen fixture
- **WHEN** the 500-node screen is used for the warm snapshot measurement
- **THEN** the report records the node count actually returned by each tool

### Requirement: Reproducible runs
Every report SHALL record host hardware, host OS, Xcode version, Android API level, device or emulator model, tool versions and the commit under test, and SHALL exclude warm-up iterations from statistics.

#### Scenario: Report provenance
- **WHEN** two runs are compared
- **THEN** each carries enough provenance to tell whether the environments matched

### Requirement: Reliability metrics
The harness SHALL report failure rate and flake rate per primitive across the iterations of a run.

#### Scenario: Intermittent failure
- **WHEN** 2 of 50 tap iterations fail
- **THEN** the report shows a 4 percent failure rate for taps alongside the latency percentiles

### Requirement: Output formats
The harness SHALL write a machine-readable JSON report and a markdown summary with a comparison table across tools and targets.

#### Scenario: Two outputs
- **WHEN** a run completes
- **THEN** both the JSON report and the markdown summary exist and agree on every number
