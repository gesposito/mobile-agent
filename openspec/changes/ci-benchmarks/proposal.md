# Proposal

## Why

Every number this project has measured came from a hand-run session on the maintainer's Mac: the harness on the Simulator and the emulator (`reports/10.2-simulator.md`, `reports/10.1-emulator.md`), on a Nexus 5X (`reports/10.3-physical-android.md`) and on an iPhone SE (`reports/11.1-ios-device.md`), and the 68 host microbenchmarks against `reports/micro/baseline.json` (`just micro`). CI validates only: the Linux and macOS legs of `.github/workflows/ci.yml` build, lint, test and validate, the microbenchmark suite compiles there through `clippy --all-targets` and never runs, and the comparison harness has never run outside this host. A change that slows a hot path, or a baseline tool that gets faster, is found only when someone runs the suites by hand on a quiet machine.

The maintainer wants the comparisons measurable in CI without paying for them on every push: on a schedule, and on demand against two refs of their choice. GitHub Actions on this repository has not run since the account's payments failed, so the first run waits on that, or on the repository going public, where the macOS minutes are free.

## What Changes

- A second workflow, `.github/workflows/benchmarks.yml`, triggered only by a schedule and by manual dispatch, never by a push or a pull request; CI stays validation alone.
- `mobile-agent-bench micro` gains an A/B mode: two refs built on one runner and run interleaved, judged by the relative difference of their medians with a threshold set from the runner's measured noise, allocations exact as today; the committed baseline stays the local comparison.
- The harness gains a reduced plan for a runner (one fixture per platform, fewer iterations) and a ratio-only judgement: mobile-agent against Maestro, agent-device and WebDriverAgent as ratios within one run, no absolute gates on a shared VM.
- The workflow publishes the tables in the job summary and the raw reports as artifacts, and compares a scheduled run with the previous one from its artifact, so trends show without a commit.
- A research note on benchmarking in CI (runner noise, A/B methods, the hosted services) and a spike measuring the noise of GitHub's runners decide the thresholds.
- Physical devices stay out: they need a self-hosted runner and, on iOS, a person for the approval prompt.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `benchmark-harness`: the microbenchmarks and the harness run in CI on a schedule or on demand, two refs compared on one runner by their relative difference, the harness judged by ratios there, results published as summaries and artifacts.

## Impact

- `.github/workflows/benchmarks.yml` (new), `.github/workflows/ci.yml` (a comment only), `justfile` (recipes).
- `crates/bench`: the A/B mode of `micro`, the reduced plan and the ratio judgement of `run`, their tests and README.
- `docs/research/ci-benchmarks.md`, `docs/spikes/s12-runner-noise.md`, `README.md` (How measurements are made), `docs/overview.md` section 3.
- No change to the drivers, the mirror or the daemon.
