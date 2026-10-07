# Spec Delta

## ADDED Requirements

### Requirement: Host microbenchmarks
The harness SHALL provide a microbenchmark suite over the host-side paths the speed claims rest on, which runs without a device or an emulator and measures each path on inputs of the sizes the device reports measured: encoding and decoding of the device protocol's messages, a snapshot of at least 500 nodes, one of at least 1,000 nodes, a run of 10,000 events and 1,000 requests, and the framing of a byte stream carrying them; a read of at least 500 nodes taken into the replica for the first time, again unchanged, with one node changed and with a window added or removed, and a replica started over; the interactive and full listings of such a screen with their refs, first and again unchanged; an element's visibility, the scroll container under a point and what of an area the bars above a window leave; a delta snapshot and a structural diff against a changed screen; a wait that holds and a settle of a quiet screen on a scripted target; the daemon's request parsing and reply writing for a snapshot of such a screen, and a snapshot request through its socket to a fake target; the Simulator's frame downscale, hash and JPEG encode at the default dimension, on macOS; and the emulator frame hash. Every benchmark SHALL verify its result outside the timed region and report the median time, a measure of its spread and the sample count, and those over the codec, the replica, the listings and the daemon's wire SHALL also report the allocations per iteration. The suite SHALL compile with the workspace's lint of all targets.

#### Scenario: Suite runs without devices
- **WHEN** the suite runs on a host with no Simulator booted and no emulator or device attached
- **THEN** every listed benchmark reports its median, spread and sample count, and no benchmark is skipped for want of a target

#### Scenario: Suite builds with the lint
- **WHEN** the workspace's lint of all targets runs
- **THEN** every benchmark compiles and lints clean, without running

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
A microbenchmark run SHALL compare each benchmark's median against the committed baseline and SHALL report a regression when the median is slower by more than a threshold, configurable per run with a default set from the measured run-to-run noise of a quiet host, and an improvement when it is faster by more than it; a difference in allocations per iteration SHALL be reported as a regression or an improvement at any size, since the count is deterministic. A benchmark the baseline lacks SHALL be reported as new, and one the run lacks as missing, neither a regression. A run with any regression SHALL exit with status 1 and list the regressions with the baseline and measured values. A run asked to save the baseline SHALL rewrite it with the run's medians, spreads, allocations and provenance.

#### Scenario: Regression flagged
- **WHEN** the baseline holds a snapshot decode at 59 µs and the run measures it at 77 µs with a 10 percent threshold
- **THEN** the run exits with status 1 and the report names the benchmark, 59 µs, 77 µs and the threshold

#### Scenario: Within the threshold
- **WHEN** every benchmark's median is within the threshold of its baseline and its allocations are unchanged
- **THEN** the run exits with status 0 and the report shows each difference

#### Scenario: More allocations
- **WHEN** a listing allocated 412 times per iteration in the baseline and allocates 418 times in the run
- **THEN** the report lists it as a regression naming both counts, whatever its time

#### Scenario: New benchmark
- **WHEN** the run measures a benchmark the baseline does not hold
- **THEN** the report lists it as new with its values and the run's exit status does not change

#### Scenario: Baseline saved
- **WHEN** a quiet run is asked to save the baseline
- **THEN** the committed baseline file holds that run's values and provenance, and the next run compares against them

### Requirement: Microbenchmark reports
A microbenchmark run SHALL write a machine-readable JSON report and a markdown summary from the same data, with the provenance the device reports carry (host hardware, host OS, the commit under test and whether the checkout was dirty, the toolchain) plus the harness library and its version, the sample settings and the load averages, and per benchmark the median, the spread, the sample count, the allocations where measured and the comparison against the baseline.

#### Scenario: Two outputs agree
- **WHEN** a run completes
- **THEN** the JSON report and the markdown summary exist and agree on every number and every comparison
