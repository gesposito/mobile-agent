# Tasks

Rules that bind every task below (the archived foundation design's Delivery process): research first; fail-first tests on fixtures of the runners' outputs; measured decisions recorded in design.md; atomic commits without trailers; the validation-and-fix phase before the next task; installs through `sfw`; no benchmark trigger on a push or a pull request, ever.

## 1. Research

- [ ] 1.1 Write `docs/research/ci-benchmarks.md`: how projects compare benchmarks in CI (criterion's baselines and `cargo-criterion`, interleaved A/B runs, the hosted services CodSpeed, Bencher and github-action-benchmark with what each measures and costs), what is known of GitHub's hosted runners' noise and core counts, KVM on Ubuntu runners and the Android emulator runner action, Simulators on macOS runners, the per-minute prices and an estimate of the weekly schedule's cost; what this change reuses (`just micro`'s guard and reports, the harness's adapters and provenance) and where it differs (relative judgement between refs, ratios on runners)

## 2. Spike (needs Actions)

- [ ] 2.1 S12, the runners' noise: run the micro suite five times on an Ubuntu runner and five on a macOS runner and an A/B of `main` against `main` in three rounds, record each benchmark's coefficient of variation and the false-regression rate at thresholds of 5, 10 and 15% per runner kind in `docs/spikes/s12-runner-noise.md` with the raw files beside it, and set D2's default threshold from it (go/no-go: a threshold under 15% at which `main` against `main` shows no regression; otherwise the micro job reports without failing and D2's alternative is written up)

## 3. Implementation

- [ ] 3.1 The A/B mode of `micro` (`crates/bench/src/micro/`, `main.rs`): failing tests on fixtures of two sides' criterion files that `micro --ab <ref>` reads base and head rounds, judges by the relative difference and exact allocations with the threshold, lists same, faster and regressed with both values, fails only on a regression, and refuses `--save-baseline` with `--ab`; then implement the worktree checkout, the two builds in separate target directories, the interleaved rounds and the report; verify red-then-green and locally an A/B of `main` against `HEAD~5` on a quiet host recorded in the task
- [ ] 3.2 The workflow (`.github/workflows/benchmarks.yml`): `schedule` and `workflow_dispatch` with D1's inputs, jobs `micro-linux` and `micro-macos` (the Simulator frame benches on macOS alone), `harness-simulator` and `harness-emulator` gated on the suite, the summary tables, the artifacts and the previous scheduled run's artifact download and comparison; a comment in `ci.yml`; verify with `actionlint` and a dry parse, and with the workflow's push and pull-request triggers absent by a test that reads the file
- [ ] 3.3 The harness's reduced plan and ratio judgement (`crates/bench/src/{plan,gates,markdown,main}.rs`): failing tests that `run --plan ci` measures one fixture per platform with 10 iterations after 2 warm-up, that `--gates none` judges nothing and the report lists mobile-agent's ratios against each baseline per flow and platform with the skipped ones, and that the summary markdown carries the tables; then implement; verify red-then-green and a local `run --plan ci --gates none` on this session's own Simulator recorded in the task
- [ ] 3.4 Docs: `crates/bench/README.md` (the A/B mode, the reduced plan, the ratios), `README.md` How measurements are made (what CI measures and what it does not), `docs/overview.md` section 3, `justfile` recipes (`bench-ab`); verify `just validate` passes

## 4. First runs (need Actions)

- [ ] 4.1 The first dispatched micro A/B on both runner kinds after S12, with the threshold set; record the summary's table and the minutes used in this task
- [ ] 4.2 The first scheduled run of both suites and the comparison with the dispatched run's artifact; record the ratios and the minutes
- [ ] 4.3 Update the project memory and archive the change with the delta spec synced; verify `openspec validate ci-benchmarks --strict` passes and every task above is ticked
