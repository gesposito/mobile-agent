# Host microbenchmarks

`baseline.json` is the committed result of the last accepted run of the host microbenchmark suite (`crates/*/benches/`, design D13 to D16 of the `microbenchmarks` change): one entry per benchmark with its median time, the median's 95% interval, the sample count and, where the bench counts them, the allocations and bytes per iteration, under the provenance of the run that wrote it (host, OS, commit, toolchain, criterion version and sample settings, the load averages at its start and end, how long it waited for a quiet host). The suites measure the host-side paths the speed claims rest on, without a device: the codec and framing (`protocol`), reads taken into the replica, listings, visibility, deltas, waits and settle on the fake clock (`mirror`), the daemon's JSON wire and a `snapshot -i` through its socket to a fake target (`daemon`), the frame pipeline (`simulator`), the emulator frame hash (`android`) and a snapshot through the MCP server (`mcp`).

## Running

```sh
just micro                                # every suite on a quiet host, compared with baseline.json
just micro --wait 60                      # wait up to an hour for the host to go quiet first
just micro --filter mirror/snapshot       # the ids a filter matches, compared with their baseline entries
cargo bench -p mirror -- --noplot pump    # one suite alone, criterion's own output and comparison
```

`micro` measures only on a quiet host: a 1-minute load average under 4 and no build, emulator or Simulator process above 5% of a core. A busy host is refused with exit status 2, or waited out with `--wait`, and the load averages go into every report, so a number from a busy host is never recorded. A run takes about ten seconds per benchmark at criterion's default sample settings, which the regression threshold was measured with (spike S8, `docs/spikes/s8-bench-harness.md`), plus the build the first time; `cargo bench --workspace --bench codec --bench mirror --bench daemon --bench frames --bench mcp --no-run` builds the suites beforehand.

## Reading a report

The run writes `report.json` and `report.md` to `target/bench/micro/<UTC start time>/` (or `--out`) and prints the summary line, the regressions and the improvements. A benchmark regresses when its median is slower than the baseline's by more than the threshold, 5% by default, and lies above the 95% interval the baseline recorded around its own median, or when its allocations or bytes per iteration rose at all, since those are deterministic; it improves when faster by more than the threshold and below that interval, or when its allocations fell. A median inside the baseline's interval is the same whatever its difference, which the report still shows: the interval is there for the few benchmarks whose samples fall in two modes, so that their median's swing between runs is not read as a change in the code. Benchmarks the baseline lacks are new and ones the run lacks are missing; neither fails the run. Exit status 1 means at least one regression, 2 that nothing was measured (busy host, a refused option, a run that could not complete). The fields are listed in `crates/bench/README.md`, Microbenchmarks.

## Comparing two refs

```sh
just bench-ab main --wait 60              # the checkout against main, both built and run here
```

`micro --ab <ref>` neither reads nor writes `baseline.json`: it checks the ref out into a worktree of its own, builds both trees, runs the suites three times on each side with the side that goes first alternating, and judges the checkout's medians against the ref's at 10% by default, allocations and bytes exactly. CI's benchmarks workflow runs it on GitHub's runners, where a baseline measured on this host would compare another machine (`crates/bench/README.md`, Microbenchmarks).

## Refreshing the baseline

```sh
just micro --wait 60 --save-baseline      # compares with the old baseline, then rewrites it
```

Refresh it when a change moves the numbers on purpose, or after a toolchain bump, and commit `baseline.json` with that change: its diff shows every benchmark the change moved, beside the provenance it was measured with, so a regression a change accepts is reviewed where it happened. `--save-baseline` is refused with `--filter` (a baseline holds every suite) and with a `--max-load` above the default (a baseline never comes from a host the guard would refuse). Numbers from different machines are not comparable; the report names the provenance fields in which a baseline's environment differs from the run's.
