# Spike S8: microbenchmark harness

Task 1.2 of `microbenchmarks`. It measures, on this host, how far each candidate harness's reported value for one microbenchmark moves from run to run, and whether the harness tells a 5% slower variant apart, on a quiet host and under load. The results choose the harness of design D13 and set the threshold of D16 and the load bound of D15. The candidates come from the research note `docs/research/microbenchmarks.md`. The spike code is `spikes/s8-bench-harness/`, kept privately since the repository was made public (`docs/spikes/README.md`); every table in Measurements is generated from its `results/` by `cargo run --release --bin s8 -- report`.

**In brief.** On the quiet host, criterion, divan and tango pass design D13's gate and glassbench does not. The coefficient of variation (CV) of the reported `decode` value across ten runs is 1.15% for criterion, 1.37% for divan, 1.73% for tango and 2.16% for glassbench. The variant made 5.06% slower comes out slower in 10 of 10 runs under the first three and 9 of 10 under glassbench. Of the three that pass, only criterion gives a machine-readable result per benchmark without parsing text (`estimates.json`), so it is the choice. It costs 48 crates against divan's 17 and tango's 80, and 19.6 s per run of two benchmarks against 0.1 and 0.2 s. Under four busy cores criterion also held best: CV 0.34%, and the variant told apart in 10 of 10 runs with its 95% intervals apart each time. Tango fell to 8 of 10 with a CV of 7.38%. But that load moved criterion's median up by 4.4%, close to a 5% threshold, so the guard's load bound stays at 4. D16's threshold is the 5% floor, since twice criterion's quiet CV is 2.3%.

## Setup

**Host.** Apple M1 Max (8 performance and 2 efficiency cores), 64 GiB, macOS 26.6.2 (25G83), shared with other sessions that build and run emulators (see Measurement hygiene).

**Toolchain and crates.** Rust 1.98.1, pinned by `rust-toolchain.toml`, in the bench profile the workspace's `cargo bench` uses (opt-level 3, no LTO, 16 codegen units). The harnesses are criterion 0.8.2 with default features off, divan 0.1.21, tango-bench 0.8.0 and glassbench 0.5.0, the latest releases on 2026-10-07. The shipped `protocol` crate comes in by path, on minicbor 2.3.0. Every crate came through `sfw cargo add` and `sfw cargo fetch`, and the builds ran offline.

**Code.** The measured binaries are those of commit `9a35256`. A later commit adds the list's row count to `s8 input`, and to the tables the shift under load, the total wait and the column naming each harness's statistics; none of it changes the timed code.

**Input.** `src/generator.rs` ports the tree half of S5's generator to the shipped `protocol` types. It draws from S5's seed (20260926) in S5's order, so the tree is S5's `T500`:

- 500 nodes, depth 7, and a string table of 14 view classes and 2 resource-id prefixes;
- resource ids on 274 nodes, text on 250, content descriptions on 62;
- 8 nodes with negative bounds, and 8,069 bytes of node strings.

`tests/input.rs` checks these against S5's manifest. The tree hangs in one application window. The list also carries the scroll semantics that the protocol has and S5's model did not: an item range over its 109 rows, with rows 5 to 15 in view. The response encodes to 21,410 bytes; S5's model, with per-node window ids and a 4-byte frame header, took 22,863.

**Benchmarks.** Every harness times the same two functions of the spike's library. They are `#[inline(never)]`, so each bench binary calls the same machine code, and the harnesses call them through `std::hint::black_box`:

- `decode`: `Message::decode` of the 21,410 bytes, with the decoded message dropped inside the timed region; it returns the node count. Before any timing, the message decoded from the bytes is compared once with the original.
- `decode_slow`: `extra_work` over N input bytes, each passed through `black_box`, then `decode`. N comes from the calibration in Measurements, and reaches the binaries through `S8_EXTRA_BYTES`.

**Harnesses, each with its default settings.**

| Harness | Run as | Settings and value read |
|---|---|---|
| criterion | `s8-criterion --bench --noplot`, with a fresh `CRITERION_HOME` per run, so no run is compared with an earlier one | 3 s warm-up, then 100 linear samples of 8 to 800 iterations, 5.1 s of measurement per benchmark; 100,000 bootstrap resamples on one thread, since `rayon` is off. Value: the median's point estimate and 95% interval from `estimates.json` |
| divan | `s8-divan --bench` | 100 samples of one iteration each, since one call takes more than 100 times the 41 ns timer precision divan measured. Value: the median column of its text table |
| tango | `s8-tango --color never compare -d <dir>` with no path, so the one binary runs as both workers; the candidate worker runs `decode_slow` where the baseline runs `decode`, with no difference of code layout between two builds | 100 ms per benchmark in 10 ms samples, noise threshold 1%, no outlier filter. Value: the means of the samples it dumps; significance from its text line |
| glassbench | `s8-glassbench -- --tag <series>-<round>`, with one database per series | 2 warm-up and 5 estimating iterations, then one loop of about 2 s. Value: `total_duration_ns / iterations` from its `task` table |

**Method (`bench.sh`).** The calibration runs first, then a quiet series and a loaded series. A series is ten rounds, and a round runs the four harnesses in turn, starting one harness later each round, so drift of the host falls on all of them alike. Before every run, and before the calibration, `bench.sh` applies design D15's guard:

- the 1-minute load average is under 4, or under 8 in the loaded series, whose four `yes` processes keep four cores busy;
- no process outside the script's own process tree named `cargo`, `rustc`, `gradle*`, `xcodebuild`, `kotlinc*`, `java`, `qemu-system*`, `emulator*`, `Simulator` or `com.apple.CoreSimulator*` is above 5% of a core in `ps`.

Otherwise it checks again every 30 s, and after 90 minutes of waiting in all it stops without measuring. The loaded series starts only from a quiet host; it then starts four `yes > /dev/null` processes and stops them after its last run. Each harness's raw output goes to `results/<series>/<harness>/`, the wall time of each run to `results/runs.tsv`, and every check to `results/hygiene.log` (git-ignored), summarized in `results/hygiene.tsv`.

**Measures.** Per harness and series:

- the CV, the sample standard deviation over the mean, of the ten `decode` values the harness reported;
- in how many of the ten runs `decode_slow` came out slower than `decode`, and by how much;
- whether the harness's own statistics told the two apart, where it has any: criterion's 95% intervals of the two medians not overlapping, or tango's significance mark;
- the wall time of each run, from start to exit;
- the crates the harness alone adds to the spike's lock file and builds for this host;
- whether its output is machine-readable per benchmark without parsing prose.

**Measurement hygiene.** All 81 measured runs (the calibration, 40 quiet and 40 loaded) started on a host the guard passed; nothing was measured on a busy host and no run was repeated. Before the calibration the guard waited 36 checks of 30 s, from 22:56:00 to 23:14:03. At 35 of them one or two emulator processes (`qemu-system-aarch64-headless`, another session's) were above 5% of a core, once together with a `java` process, and the 1-minute load ranged from 1.63 to 14.57. The calibration started at a load of 3.14 4.64 6.29 and ended at 2.89 4.53 6.23. The quiet series ran from 23:14:17 to 23:18:49, every run starting at a 1-minute load between 2.89 and 3.95; it waited once, 30 s before round 8, at 4.65. It ended at 3.27 3.93 5.49. The loaded series started from that quiet host, ran from 23:18:49 to 23:25:58 at 1-minute loads up to 7.99 at the start of a run, and ended at 7.93 7.44 6.65. It waited six checks before round 9, at loads of 8.70 to 11.84: other work on top of the four `yes` processes, with no watched process busy. In all, 125 checks, 43 of them waits, or 21.5 minutes. `results/hygiene.tsv` lists every check.

**Rerunning.**

```sh
cd spikes/s8-bench-harness                 # in the private copy
cargo test                               # the input's shape and bytes, the parsers
./bench.sh all                           # env, build, deps, calibrate, quiet, loaded -> results/
cargo run --release --bin s8 -- report   # the tables below
```

**Not measured.**

- Gungraun (formerly iai-callgrind) and CodSpeed's simulation instrument: both count instructions under Valgrind, which has no arm64 macOS port. They are kept for the CI change's Linux leg.
- CodSpeed's criterion and divan layers on their own: outside CodSpeed's environment they run criterion's and divan's code, measured here, and their results go to CodSpeed's service.
- Non-default settings. criterion passed with its defaults; divan's `--sample-count` and `--min-time`, tango's `-t` and `-o`, and glassbench, which has no settings, were left as shipped.
- Allocation counting. divan's `AllocProfiler` was left off, since it adds work to every allocation in the timed region, and criterion has none.

## Measurements

### Calibration of `decode_slow`

Pilot rounds of a plain timing loop (`s8 calibrate`): 200 rounds, each one sample of each body of 78 iterations, the order flipped every round; medians per iteration, and the median of the per-round ratios. The extra work costs 1.032 ns per byte. Load before: `{ 3.14 4.64 6.29 }`; after: `{ 2.89 4.53 6.23 }`.

| Pilot | Extra bytes | `decode` (µs) | `decode_slow` (µs) | Slower by |
|---|---:|---:|---:|---:|
| floor | 0 | 124.91 | 124.67 | -0.01% |
| predicted | 6059 | 124.96 | 130.94 | +4.76% |
| corrected | 6365 | 125.39 | 131.68 | +5.06% |

`decode_slow` reads 6365 extra bytes in every run below.

### Quiet host: ten runs per harness

| Harness | Reports | Samples | `decode` median of runs (µs) | CV of `decode` | `decode_slow` slower | Slower by: median (min to max) | Told apart by its statistics | Wall time per run (s) |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| criterion | median, bootstrap 95% interval | 100 | 125.23 | 1.15% | 10/10 | +5.0% (+3.2% to +5.9%) | 10/10: 95% intervals of the medians apart | 19.6 (19.3 to 20.0) |
| divan | median of 100 samples | 100 | 125.15 | 1.37% | 10/10 | +4.6% (+2.0% to +6.7%) | none | 0.1 (0.1 to 0.1) |
| tango | mean of paired 10 ms samples | 12 to 15 | 127.90 | 1.73% | 10/10 | +5.2% (+5.0% to +5.7%) | 10/10: significance mark | 0.2 (0.2 to 0.2) |
| glassbench | mean of one 2 s loop | 1 loop | 127.09 | 2.16% | 9/10 | +5.1% (-0.1% to +8.6%) | none | 3.9 (3.6 to 4.1) |

Per run, `decode` in µs and how much slower `decode_slow` came out:

| Round | criterion | divan | tango | glassbench |
|---:|---:|---:|---:|---:|
| 1 | 124.95 +5.1% | 128.30 +2.1% | 128.28 +5.2% | 127.30 +2.4% |
| 2 | 125.84 +5.4% | 129.10 +4.9% | 132.83 +5.5% | 125.94 +6.9% |
| 3 | 125.64 +4.9% | 128.00 +2.0% | 127.24 +5.0% | 126.94 +7.8% |
| 4 | 128.65 +3.2% | 124.70 +6.7% | 127.75 +5.4% | 128.16 +5.0% |
| 5 | 125.03 +5.2% | 125.20 +4.2% | 127.72 +5.2% | 133.92 -0.1% |
| 6 | 125.70 +4.8% | 125.20 +4.6% | 127.14 +5.7% | 127.25 +3.6% |
| 7 | 125.43 +4.5% | 124.80 +4.3% | 133.02 +5.1% | 126.77 +8.6% |
| 8 | 123.71 +4.5% | 124.80 +4.6% | 127.54 +5.4% | 128.80 +5.6% |
| 9 | 123.65 +5.5% | 125.10 +4.6% | 128.04 +5.0% | 124.44 +4.6% |
| 10 | 124.05 +5.9% | 124.90 +5.4% | 128.06 +5.0% | 124.06 +5.2% |

### Under four busy cores: ten runs per harness

| Harness | Reports | Samples | `decode` median of runs (µs) | CV of `decode` | `decode_slow` slower | Slower by: median (min to max) | Told apart by its statistics | Wall time per run (s) |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| criterion | median, bootstrap 95% interval | 100 | 130.71 | 0.34% | 10/10 | +5.2% (+4.7% to +5.5%) | 10/10: 95% intervals of the medians apart | 20.2 (19.7 to 20.6) |
| divan | median of 100 samples | 100 | 132.90 | 1.53% | 10/10 | +4.2% (+0.3% to +8.6%) | none | 0.1 (0.1 to 0.1) |
| tango | mean of paired 10 ms samples | 12 to 18 | 131.36 | 7.38% | 8/10 | +4.2% (-9.3% to +6.2%) | 8/10: significance mark | 0.2 (0.2 to 0.2) |
| glassbench | mean of one 2 s loop | 1 loop | 130.76 | 2.88% | 9/10 | +4.5% (-3.6% to +5.2%) | none | 4.0 (3.6 to 4.1) |

Per run, `decode` in µs and how much slower `decode_slow` came out:

| Round | criterion | divan | tango | glassbench |
|---:|---:|---:|---:|---:|
| 1 | 130.27 +5.3% | 132.20 +4.8% | 130.13 +6.0% | 130.33 +4.4% |
| 2 | 130.27 +4.9% | 132.90 +4.7% | 132.22 +3.4% | 130.50 +5.1% |
| 3 | 130.53 +5.5% | 132.90 +4.7% | 131.54 +3.9% | 130.49 +4.5% |
| 4 | 130.20 +5.1% | 132.90 +0.3% | 131.18 +4.1% | 130.97 +4.5% |
| 5 | 130.33 +5.5% | 128.20 +8.6% | 153.75 -9.3% | 130.83 +4.9% |
| 6 | 130.91 +5.2% | 132.60 +3.8% | 130.13 +6.2% | 130.69 +5.0% |
| 7 | 131.56 +5.4% | 135.60 +3.5% | 155.87 -0.6% | 142.48 -3.6% |
| 8 | 131.06 +4.7% | 132.20 +2.3% | 130.49 +5.2% | 134.05 +2.5% |
| 9 | 130.91 +5.0% | 133.50 +4.8% | 132.55 +4.9% | 131.07 +4.1% |
| 10 | 130.89 +5.0% | 135.40 +2.4% | 130.81 +4.3% | 130.01 +5.2% |

### Dependencies

Crates each harness alone adds (`bench.sh deps`): a crate holding `protocol` and that one harness is resolved against the spike's lock file, so versions match, and its lock file and the crates `cargo tree` builds for this host are compared with those of `protocol` alone.

| Harness | Version | Features | Lock-file crates added | Built for this host |
|---|---|---|---:|---:|
| criterion | 0.8.2 | default features off (as measured) | 48 | 40 |
| criterion-csv | 0.8.2 | default features off, csv_output (for raw.csv) | 51 | 43 |
| criterion-defaults | 0.8.2 | default features (rayon, plotters) | 70 | 48 |
| divan | 0.1.21 | default features | 17 | 14 |
| tango | 0.8.0 | default features | 80 | 44 |
| glassbench | 0.5.0 | default features | 184 | 129 |

### Output per benchmark

| Harness | Machine-readable without parsing prose |
|---|---|
| criterion | yes: `estimates.json` per benchmark (median and interval), which criterion's book calls unstable; `raw.csv` only with the deprecated `csv_output` |
| divan | no: a text table with units, parsed here |
| tango | partly: `-d <dir>` writes raw paired samples per benchmark as CSV; the means and the verdict are a text line and the exit status |
| glassbench | yes, through SQL: `glassbench_v1.db`, table `task` |

### Gate (design D13)

Quiet host: `decode` CV at or under 2% and `decode_slow` slower in at least 9 of 10 runs.

| Harness | Quiet CV | Detected | Passes | Loaded CV | Loaded detected | `decode` under load against quiet |
|---|---:|---:|:-:|---:|---:|---:|
| criterion | 1.15% | 10/10 | yes | 0.34% | 10/10 | +4.4% |
| divan | 1.37% | 10/10 | yes | 1.53% | 10/10 | +6.2% |
| tango | 1.73% | 10/10 | yes | 7.38% | 8/10 | +2.7% |
| glassbench | 2.16% | 9/10 | **no** | 2.88% | 9/10 | +2.9% |

### Hygiene

125 checks before runs, 43 of them on a busy host, each followed by 30 s of waiting: 21.5 minutes in all.

| Series | First check | Last entry | Checks that waited 30 s | Runs | Load at the first quiet check | Load after the last run | Highest 1-min load a run started at |
|---|---|---|---:|---:|---|---|---:|
| calibration | 2026-10-07 22:56:00 | 2026-10-07 23:14:17 | 36 | 1 | 3.14 4.64 6.29 | 2.89 4.53 6.23 | 3.14 |
| quiet | 2026-10-07 23:14:17 | 2026-10-07 23:18:49 | 1 | 40 | 2.89 4.53 6.23 | 3.27 3.93 5.49 | 3.95 |
| loaded | 2026-10-07 23:18:49 | 2026-10-07 23:25:58 | 6 | 40 | 3.27 3.93 5.49 | 7.93 7.44 6.65 | 7.99 |

### Observations

- **The decode itself.** The shipped crate decodes the 500-node response in about 125 µs here: 125.23 µs by criterion's median of runs, and 124.91 µs in the calibration's own loop. The four harnesses agree within 2.2% on the quiet host. Tango's two workers run at once on two cores and read 2.1% above criterion's lone process. Glassbench's mean takes in the slow iterations that a median leaves out. S5 measured 59.1 µs for the same tree on its own model types, with the drop outside the timed region and a fat-LTO, single-codegen-unit build. This spike times the shipped decoder, the drop included, in the workspace's default profile. Which of these accounts for the difference was not measured; task 3.2 names it when it sets the suite's numbers beside S5's.
- **criterion.** Each run measures each benchmark for 5.1 s: 100 linear samples, 40,400 iterations. Its quiet CV of 1.15% comes mostly from round 4, whose median was 128.65 µs with an interval of 126.33 to 130.98 µs, against 123.65 to 125.84 µs in the other nine rounds. Even there the variant's interval, 132.14 to 135.30 µs, stayed apart. The reported slowdown ranged from +3.2% to +5.9% quiet and +4.7% to +5.5% loaded. A run takes 19.6 s for two benchmarks: 3 s of warm-up and 5 s of measurement each, plus the single-threaded bootstrap.
- **divan.** Each value is the median of 100 single calls, about 13 ms of measurement per benchmark, so a run takes 0.1 s. That median is steady across runs (CV 1.37%), but the difference within a run is not: +2.0% to +6.7% quiet and +0.3% to +8.6% loaded. A 5% change sits at the edge of what one run tells.
- **tango.** It gave the tightest difference on the quiet host: +5.0% to +5.7%, significant in 10 of 10 runs, in 0.2 s. Under load it was the least robust. In loaded round 5 the baseline worker's first two samples took 272 and 339 µs per iteration, the worker preempted or on an efficiency core. In round 7 both workers' samples scattered from 129 to 198 µs. The default 100 ms of sampling, 12 to 18 samples without outlier filtering, cannot absorb that: the variant read −9.3% and −0.6%, and the CV of the baseline mean was 7.38%. `-o` filters such samples and `-t` samples longer, but neither is a default. Its two workers also occupy two cores at once.
- **glassbench.** It keeps one mean of a 2 s loop and handles no outliers. One disturbed run, quiet round 5 at 133.92 µs, both lifted its CV past 2% and hid the variant (−0.1%).
- **Load.** Four busy cores raised every harness's `decode` value by 2.7% to 6.2%, criterion's by 4.4%, while criterion's spread fell to a CV of 0.34%. The cause was not isolated: macOS places threads on performance or efficiency cores as it sees fit, and the guard alone keeps that out of the measurement. A baseline measured quietly and compared with a run at a 1-minute load near 8 would therefore spend most of a 5% threshold on load alone.
- **Dependencies.** With default features off, criterion adds 48 crates to a lock file and builds 40 here; 23 of the 48 are new to this workspace's `Cargo.lock` by name. Its default features add `rayon` and `plotters` (70 and 48), and `csv_output`, which `raw.csv` needs, adds 3 more. 30 of tango's 80 are Windows crates. Glassbench's 184 include bundled SQLite and libgit2, compiled from C.

## Verdict

**Go for criterion 0.8.2, with default features off, run with `--noplot` and its default sample settings, its results read from `estimates.json`.** Criterion, divan and tango pass design D13's gate on the quiet host and glassbench does not (CV 2.16%). Criterion is the only one of the three whose result per benchmark is machine-readable without parsing text: divan prints a table with units, and tango dumps raw samples but gives its means and verdict as text and an exit status. The order of preference, machine-readable output before dependency count, therefore picks criterion over divan's 17 crates and tango's 80. Criterion's numbers also make the strongest case on their own: the lowest CV under load (0.34%), the variant told apart in all 20 runs with its 95% intervals apart each time, and a reported slowdown between +3.2% and +5.9% for a 5.06% change.

What it means for the decisions it sets:

- **D13.** criterion 0.8.2, default features off. Criterion's book calls `estimates.json` private, and the stable `raw.csv` needs a deprecated feature, so the version stays pinned in `Cargo.lock` and `micro`'s parser is tested on files this spike recorded (`results/quiet/criterion/`). Criterion counts no allocations, so D14's counting allocator over `System` is what reports them.
- **D15.** The default load bound stays at 4. The quiet series ran under it, at 1-minute loads of 2.89 to 3.95, with criterion's CV at 1.15%. Four busy cores, at loads up to 7.99, moved criterion's median by 4.4%, so raising the bound would spend most of the threshold on load. Nothing measured asks for a lower bound.
- **D16.** The default threshold is 5%. Twice criterion's quiet-host CV of 1.15% is 2.3%, under the 5% floor.
- **Run time.** Criterion's defaults take about 10 s per benchmark (3 s of warm-up, 5 s of measurement, the bootstrap), so `micro`'s run time is about 10 s times the number of benchmarks. Shorter settings would need this measurement repeated at them before the threshold could rely on them.

What it means for the tasks S8 gates:

- **Task 2.1.** `sfw cargo add --dev criterion --no-default-features` in the five crates, with `[[bench]] harness = false`.
- **Tasks 2.2 to 2.5.** Each benchmark body goes through `Bencher::iter`, which drops the routine's output inside the timed region, as this spike's bodies did. Allocations come from the counting allocator, counted where they cannot add to the timed iterations.
- **Task 3.1.** `micro` runs `cargo bench` with `-- --noplot`, sets `CRITERION_HOME`, and reads each benchmark's `new/estimates.json` (the median's point estimate and 95% interval) and `new/sample.json` (the sample count).
- **Task 3.2.** The baseline comes from a guarded run, compared with a 5% threshold.

Glassbench is a no-go, and tango and divan remain alternatives recorded in D13. Gungraun and CodSpeed's instruction counting stay with the CI change.
