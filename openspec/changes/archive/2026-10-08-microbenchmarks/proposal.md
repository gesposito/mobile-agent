# Proposal

## Why

The speed claims rest on device runs: the harness of `crates/bench` times whole flows and primitives through a Simulator or an emulator, which takes a quiet host, booted devices and most of an hour, so the host-side mechanisms the claims attribute the savings to (the CBOR codec, the replica's matching of reads, the listing and its refs, visibility, delta snapshots, the frame pipeline) are measured only as part of a device round trip, if at all. A regression in one of them shows at the next device run or never: task 10.30's 200 ms hold went into every `fill` and was found when task 10.26 re-measured text entry, weeks later. `docs/overview.md` section 2 says what each mechanism saves on a device and cannot say what it costs on the host.

## What Changes

- A microbenchmark suite over the host-side hot paths, in each crate's `benches/`, with inputs of the sizes the reports measured: the 500-cell fixtures and a 1,049-node React Native screen, 10,000 events, 460x1000 frames. It measures the codec and framing (`protocol`), reads taken into the replica, snapshots, visibility, deltas, waits and settle on the fake clock (`mirror`), the daemon's JSON wire and a `snapshot` through its socket on a fake target (`daemon`), the Simulator's frame downscale, hash and JPEG encode (`simulator`, macOS only) and the emulator's frame hash (`android`).
- The benchmark harness library is chosen by a spike, S8, that measures run-to-run noise on this host, quiet and under load, and whether a 5% regression is told apart; the decision and its numbers go into this change's design.
- `mobile-agent-bench micro`, a subcommand of the existing harness binary (`just micro`): it refuses, or with `--wait` waits out, a busy host (1-minute load, builds, emulators and Simulators using CPU), runs the suite in release mode, compares the results with the committed baseline and writes a JSON and a markdown report, flagging regressions past the threshold the spike set. It exits non-zero on a regression.
- A committed baseline, `reports/micro/baseline.json`, with the provenance the device reports carry (host, OS, commit, load), refreshed by a quiet run when a change is accepted; `reports/micro/README.md` explains the workflow.
- Research note `docs/research/microbenchmarks.md` on harness libraries, noise control and continuous benchmarking, written before the spike.
- `README.md` and `docs/overview.md` section 2 gain what each mechanism costs on the host, from the baseline.
- The `justfile`'s `openspec-validate` recipe validates the active changes instead of the archived one by name.
- Not in this change: running the suite in CI (hosted runners cannot give stable timings; the CI change decides between instruction counts and a dedicated runner), device benchmarks, which stay as they are, and optimizations the numbers may suggest.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `benchmark-harness`: adds the host microbenchmarks: which paths the suite SHALL cover and with what inputs, the quiet-host guard, the comparison against the committed baseline with its threshold and exit status, and the reports. The device primitives, gates, baselines and fixtures stay as specified.

## Impact

- `crates/protocol`, `crates/mirror`, `crates/daemon`, `crates/simulator` and `crates/android` each gain `benches/` and a dev-dependency on the harness library, added through `sfw cargo add`; `[[bench]]` entries in their manifests. `cargo clippy --workspace --all-targets`, which `just lint` and CI already run, compiles the benches, so CI builds them without running them.
- `crates/bench` gains the `micro` subcommand: the host guard, the cargo invocation, the harness output parser, the baseline comparison and the reports. Its `README.md` documents it.
- `reports/micro/` holds the baseline and its README; `spikes/s8-bench-harness/` (kept privately) the throwaway spike, `docs/spikes/s8-bench-harness.md` its report.
- `justfile`: `micro` recipe; `openspec-validate` recipe.
- Design D13 (this change's design.md) records the harness choice and the threshold; `docs/overview.md`, `README.md` and the crates' READMEs gain the host costs and the suite's layout.
- No change to the daemon, the CLI, the agent, the protocol or the mirror's behaviour; the fake target and the fixtures are reused as bench inputs. One mirror fixture may be added for the 1,049-node screen if none of the existing ones is that large.
