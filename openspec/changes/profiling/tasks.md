# Tasks

Rules that bind every task below (the archived foundation design's Delivery process): research first; spikes with go/no-go before building on an unproven assumption; measured decisions recorded in design.md; fail-first tests observed red; atomic commits without trailers; the validation-and-fix phase before the next task; installs through `sfw`; the quiet-host rule for numbers; every daemon of a task on a private socket directory; one agent at a time.

## 1. The spike

- [ ] 1.1 S15 (D1, D2): the profiles of the daemon on the Simulator and the emulator through the scripted sequences, of the CLI's start and of the server's round trip, with `xctrace` (or `samply`, said why), on a quiet host, the release build with debug info; `docs/spikes/s15-profiles.md` (Setup; Measurements: per path the split between waiting and this code's frames, the top twenty frames by self time with their crate, the allocation profile of the listing, the spans with and without the profiler; Verdict: the hot spots ranked by the share of a model's turn, a go or no-go each, and which the microbenchmarks already time), its row in `docs/spikes/README.md`; the traces, flamegraphs and scripts kept privately

## 2. The fixes the spike ranks

- [ ] 2.1 The first hot spot (D3): the microbenchmark that times it red first at the threshold the spike set, the fix, `just micro --ab` between the refs on a quiet host, the share bought recorded in D3; the mirror's and daemon's tests the guard
- [ ] 2.2 The second hot spot, the same way
- [ ] 2.3 The third hot spot, the same way, or none if the spike ranks fewer

## 3. The suite and the docs

- [ ] 3.1 The microbenchmark suite extended where the spike found a hot path it does not time (red first where it guards a fix), `reports/micro/baseline.json` recorded again on a quiet host if the suite changed, with the provenance
- [ ] 3.2 Docs: `docs/overview.md` where it says how the tool measures itself names the profiles and their report; `crates/bench/README.md` the new suites; the READMEs of the crates a fix changed; `just validate` green
- [ ] 3.3 Update the project memory and archive the change; verify `openspec validate profiling --strict` passes and every task above is ticked
