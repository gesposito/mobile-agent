# benchmark-harness Specification

## Purpose
Measures the latency and reliability of every primitive and of end-to-end flows against explicit gates and against existing tools on the same flows, so that speed claims are verified rather than asserted.

## Requirements

### Requirement: Measured primitives
The harness SHALL measure CLI process start, cold attach, first snapshot after application launch, warm snapshot, tap-to-frame-change, tap-to-tree-delta, text entry of a fixed string, screenshot and settle detection, and SHALL report p50, p95, maximum and sample count for each.

#### Scenario: Primitive report
- **WHEN** a run completes on one target
- **THEN** every listed primitive appears in the report with its percentiles and sample count

### Requirement: Latency gates
Gates SHALL be configurable per target class; the defaults SHALL be CLI process start under 20 ms on every class and, per class, the Simulator's, Android's and a physical iOS device's default gates. A run that misses any gate SHALL exit non-zero and print the failing gates in a table.

#### Scenario: Gate missed
- **WHEN** Simulator warm snapshot p95 measures 210 ms
- **THEN** the run exits non-zero and the table names the gate, the limit and the measured value

#### Scenario: All gates met
- **WHEN** every measured value is within its gate
- **THEN** the run exits zero and the table shows each gate as passed

### Requirement: Default gates on the Simulator
The Simulator's default gates SHALL be warm snapshot p95 under 150 ms on the SwiftUI reference screen and under 250 ms on the React Native one, whose own accessibility snapshot costs about 236 ms, tap dispatch p95 under 20 ms, and cold attach under 2 s.

#### Scenario: Simulator gates
- **WHEN** a run on the Simulator uses the default gates
- **THEN** the table shows warm snapshot p95's limits of 150 ms on the SwiftUI reference screen and 250 ms on the React Native one, tap dispatch p95's 20 ms and cold attach's 2 s

### Requirement: Default gates on Android
Android's default gates SHALL be cold attach under 1.5 s and, on the emulator, warm snapshot p95 under 100 ms on the reference screen and tap-to-frame-change p95 under 67 ms with the guest rendering on the host GPU, four frames of the AVD's 60 Hz display, since the guest's own frame pipeline takes about three at p95; under software rendering tap-to-frame-change SHALL be reported and not gated.

#### Scenario: Emulator tap-to-frame gate under host rendering
- **WHEN** an emulator rendering on the host GPU measures tap-to-frame-change p95 at 59 ms
- **THEN** the gate passes and the table shows its 67 ms limit

### Requirement: Default gates on a physical iOS device
A physical iOS device's default gates SHALL be warm snapshot p95 under 1 s on either reference screen, tap dispatch p95 under 150 ms through the host's input path and cold attach under 5 s with the runner already installed, and tap-to-frame-change SHALL be reported and not gated while the device's frames are polled screenshots.

#### Scenario: iOS device gates
- **WHEN** a run on a physical iPhone measures warm snapshot p95 at 640 ms, tap dispatch p95 at 90 ms and cold attach at 3.1 s
- **THEN** the three gates pass, the table shows their limits, and tap-to-frame-change is listed with its value and no verdict

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

### Requirement: Host microbenchmarks
The harness SHALL provide a microbenchmark suite over the host-side paths the speed claims rest on, as the scenarios below list them, which runs without a device or an emulator and measures each path on inputs of the sizes the device reports measured. The suite SHALL compile with the workspace's lint of all targets.

#### Scenario: The protocol's and the replica's paths
- **WHEN** the suite runs
- **THEN** it measures encoding and decoding of the device protocol's messages, a snapshot of at least 500 nodes, one of at least 1,000 nodes, a run of 10,000 events and 1,000 requests, and the framing of a byte stream carrying them; and a read of at least 500 nodes taken into the replica for the first time, again unchanged, with one node changed and with a window added or removed, and a replica started over

#### Scenario: The listings', the queries' and the waits' paths
- **WHEN** the suite runs
- **THEN** it measures the interactive and full listings of a screen of at least 500 nodes with their refs, first and again unchanged; an element's visibility, the scroll container under a point and what of an area the bars above a window leave; a delta snapshot and a structural diff against a changed screen; and a wait that holds and a settle of a quiet screen on a scripted target

#### Scenario: The daemon's and the frames' paths
- **WHEN** the suite runs
- **THEN** it measures the daemon's request parsing and reply writing for a snapshot of a screen of at least 500 nodes, and a snapshot request through its socket to a fake target; the Simulator's frame downscale, hash and JPEG encode at the default dimension, on macOS; and the emulator frame hash

#### Scenario: Suite runs without devices
- **WHEN** the suite runs on a host with no Simulator booted and no emulator or device attached
- **THEN** every listed benchmark reports its median, spread and sample count, and no benchmark is skipped for want of a target

#### Scenario: Suite builds with the lint
- **WHEN** the workspace's lint of all targets runs
- **THEN** every benchmark compiles and lints clean, without running

### Requirement: Microbenchmarks verify their result and count allocations
Every benchmark SHALL verify its result outside the timed region and report the median time, a measure of its spread and the sample count, and those over the codec, the replica, the listings and the daemon's wire SHALL also report the allocations per iteration.

#### Scenario: A listing's benchmark
- **WHEN** the benchmark of a listing runs
- **THEN** its result is verified outside the timed region, and it reports the median time, the spread, the sample count and the allocations per iteration

### Requirement: Quiet-host guard
A microbenchmark run SHALL measure only on a quiet host: the 1-minute load average under a threshold, 4 by default, and no build, test, emulator or Simulator process using the CPU. Otherwise it SHALL refuse with exit status 2, naming the load and the busy processes, or, when asked to wait, check again every 30 seconds up to a time limit and refuse at the limit. Every report SHALL record the load average at the start of the run and at its end.

#### Scenario: Busy host refused
- **WHEN** a run starts with the 1-minute load average at 6.5
- **THEN** nothing is measured, the run exits with status 2 and prints the load and any busy process it found

#### Scenario: Waiting for a quiet host
- **WHEN** a run is asked to wait and the host goes quiet within the limit
- **THEN** the run measures once the host is quiet and its report records the load at that start

#### Scenario: Still busy at the limit
- **WHEN** a run is asked to wait and the host stays busy past the limit
- **THEN** nothing is measured and the run exits with status 2

### Requirement: Comparison against the committed baseline
A microbenchmark run SHALL compare each benchmark's median against the committed baseline and SHALL report a regression when the median is slower by more than a threshold, configurable per run with a default set from the measured run-to-run noise of a quiet host, and lies outside the interval the baseline recorded around its median; and an improvement when it is faster by more than the threshold and outside the interval the same way.

#### Scenario: Within the threshold
- **WHEN** every benchmark's median is within the threshold of its baseline and its allocations are unchanged
- **THEN** the run exits with status 0 and the report shows each difference

#### Scenario: Within the baseline's interval
- **WHEN** the baseline holds a decode at 133.5 µs with an interval of 132.2 to 143.5 µs around its median and the run measures 141.4 µs with a 5 percent threshold
- **THEN** the benchmark is reported as the same, with its difference shown, since a median inside that interval is within the baseline's own uncertainty, and the exit status does not change

### Requirement: Allocations compared at any size
A difference in allocations per iteration SHALL be reported as a regression or an improvement at any size, since the count is deterministic.

#### Scenario: More allocations
- **WHEN** a listing allocated 412 times per iteration in the baseline and allocates 418 times in the run
- **THEN** the report lists it as a regression naming both counts, whatever its time

### Requirement: Regressions fail the run
A run with any regression SHALL exit with status 1 and list the regressions with the baseline and measured values. A benchmark the baseline lacks SHALL be reported as new, and one the run lacks as missing, neither a regression.

#### Scenario: Regression flagged
- **WHEN** the baseline holds a snapshot decode at 59 µs and the run measures it at 77 µs with a 10 percent threshold
- **THEN** the run exits with status 1 and the report names the benchmark, 59 µs, 77 µs and the threshold

#### Scenario: New benchmark
- **WHEN** the run measures a benchmark the baseline does not hold
- **THEN** the report lists it as new with its values and the run's exit status does not change

### Requirement: The baseline saved from a run
A run asked to save the baseline SHALL rewrite it with the run's medians, spreads, allocations and provenance.

#### Scenario: Baseline saved
- **WHEN** a quiet run is asked to save the baseline
- **THEN** the committed baseline file holds that run's values and provenance, and the next run compares against them

### Requirement: Microbenchmark reports
A microbenchmark run SHALL write a machine-readable JSON report and a markdown summary from the same data, with the provenance the device reports carry (host hardware, host OS, the commit under test and whether the checkout was dirty, the toolchain) plus the harness library and its version, the sample settings and the load averages, and per benchmark the median, the spread, the sample count, the allocations where measured and the comparison against the baseline.

#### Scenario: Two outputs agree
- **WHEN** a run completes
- **THEN** the JSON report and the markdown summary exist and agree on every number and every comparison
