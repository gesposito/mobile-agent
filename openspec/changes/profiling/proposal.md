# Proposal

## Why

The project measures itself in three ways and profiles itself in none. The microbenchmarks (seven criterion suites, 41 benchmarks in `reports/micro/baseline.json` with their allocation counts, `just micro` against a baseline and an A/B of two refs) time the codec, the mirror, the daemon's socket path and the frames; the harness gates twenty latencies end to end against WebDriverAgent, agent-device and Maestro; and every reply carries its spans (`dispatchMs`, `readMs`, `firstTreeMs`, `readBackMs`). Those say which span is slow and never what inside it is slow. The last two changes measured a `fill` on Maps at 1.7 s, split into a read of 0.8 to 1.0 s while the app raises its card, keys of 0.4 to 0.8 s and a read-back of 0.2 to 0.8 s, and a Simulator tree read behind every listing, without saying how much of a read is the accessibility bridge and how much is this code's tree building, matching and diffing; the MCP server adds 0.3 ms to a listing and the CLI 6 to 8 ms a command, unexplained below that; the listing's view computation runs per element on every snapshot, hit and change wait. No sampling profile, flamegraph or allocation profile of the daemon, the mirror, a driver or the server exists. Xcode's `xctrace` is on the host; `samply` is not.

## What Changes

- A spike that profiles the daemon on the Simulator and on the Android emulator through scripted sequences (snapshots of the 500-cell screen and of Settings, clicks, fills, a change wait, screenshots), the CLI's start and the MCP server's round trip, with a sampling profiler, attributing each path's time to waiting on the device and to this code's own frames; the flamegraphs kept privately, the report public with the hot spots ranked and a go or no-go on each.
- The hot spots the spike ranks worth it, each with a microbenchmark that shows the cost red first, the fix, and an A/B of the two refs recorded in the design.
- The microbenchmark suite extended where the spike found a hot path it did not time, the baseline recorded again if the suite changed, the docs and the archive.

## Capabilities

No spec-level behavior changes: the replies, the listings and the codes stay as they are; only their cost moves. `skip_specs` is set.

## Impact

- `docs/spikes/s15-profiles.md` and its row in `docs/spikes/README.md`; `docs/overview.md` where it says how the tool measures itself; the crates the hot spots live in (`crates/mirror`, `crates/daemon`, `crates/simulator`, `crates/cli`, `crates/mcp`, `crates/android` as the spike finds), their benches and `reports/micro/baseline.json`; the private spike folder for the profiles.
