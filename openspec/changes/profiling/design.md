# Design

## Context

Everything the project knows of its own cost comes from timers it set itself: criterion around a function, a span around a command, a gate around a flow. A sampling profiler attributes a path's time to its frames without choosing them in advance, which is what the open questions need: where a Simulator tree read's time goes between the bridge's reply and the mirror's replica update, what a listing costs per element in `crates/mirror`'s view computation (reach, covers, hosts, hints, now run on every snapshot, hit and change wait), what the daemon does between a tap's dispatch and its reply, what the CLI's 6 to 8 ms a command are between the process start and the socket, what the server's 0.3 ms are, and whether the Android agent's tree read or the daemon's handling of it dominates a cold `open`. The host has Xcode 27.1's `xctrace` (Instruments' Time Profiler and Allocations from the command line, `xctrace record --template 'Time Profiler' --attach <pid>` or `--launch`, `xctrace export --xpath` to XML). `samply` records Rust binaries with symbols into the Firefox Profiler's format with a flamegraph; it is not installed and would come through `sfw cargo install samply`. The profiles hold the host's paths and symbol names, which stay in the private spike folder; the report names functions and crates.

## Goals / Non-Goals

**Goals:**
- Each hot path attributed: waiting on the device or the bridge, this code's CPU, allocation, with the top frames by self time.
- The hot spots ranked by what a fix would buy a model's turn (a listing, a click, a fill), with a go or no-go each.
- Every fix measured by a benchmark that showed the cost first and an A/B after.

**Non-Goals:**
- Changing what the bridge or the agent returns; the drivers' protocols stay.
- Optimizing paths the profile does not rank: no speculative work.
- Profiling the physical iPhone (its runner is the same code as the Simulator's reader where it counts; the phone needs the user).

## Decisions

### D1. The profiler: `xctrace` first, `samply` if it earns its install
`xctrace` is on the host and needs nothing; its Time Profiler samples the daemon by pid at 1 ms with symbols from the release build with debug info (`CARGO_PROFILE_RELEASE_DEBUG=1`, or a `profiling` profile inheriting release with `debug = 1` and `strip = false`), and `xctrace export` gives the call tree as XML the spike's script folds into a table. If reading the export costs more than the install, `samply` comes through `sfw cargo install samply` and its profile is read in the Firefox Profiler; the report says which was used and why. Allocations come from the same tool's Allocations template, on the listing path only, where the microbenchmarks already count them.

**As built (S15).** xctrace's Time Profiler at 1 ms, at 0.1 ms and with waiting threads, and its Allocations template; samply was not installed, xctrace attaching by pid on the host and, with `--device`, to the reader inside the Simulator. The build is the `profiling` Cargo profile, release with `debug = 1` and `strip = false`, whose debug map gave the frames their inlined functions and lines. kperf takes one Time Profiler recording at a time, so the daemon and the reader were recorded in separate runs; Allocations needs `get-task-allow`, so a copy of the daemon signed again ad hoc ran its four runs, and since it lists only live allocations the listing's 10.9 thousand allocations per listing are the difference of runs with and without 50 listings (`docs/spikes/s15-profiles.md`).

### D2. What is profiled, and how time is attributed
The daemon in release with debug info, on a private socket directory, attached by pid, during scripted sequences through the CLI: on the Simulator, the fixture's 500-cell Big tab (100 interactive snapshots, 100 delta snapshots, 50 clicks on the selected tab, 20 fills on the Login tab, 20 change waits, 20 screenshots) and Settings' root (50 snapshots, 20 clicks into General and back); on the Android emulator the same with the fixture's Expo build and Settings; the CLI's start (100 `status` calls under the profiler attached to the CLI, `--launch`); the MCP server's round trip (100 snapshot calls through the server, attached to the server's pid). Each path's samples fold into: waiting (blocked in a socket read, a Mach message, a semaphore: the bridge, the agent, the device), this code's frames by crate and function (self time, top twenty), and the runtime's (allocation, formatting, JSON). The report gives the split per path and the top frames, with the microbenchmark that already times each frame where one exists, so the two agree or the gap is named.

**As built (S15).** The sequences ran as planned on the iOS 26.5 Simulator and the API 36 emulator, through a client sending the CLI's lines, a connection per request, which times every command where `batch` times none; each ran without a profiler, at 1 ms, at 0.1 ms and with waiting threads, and the spans moved within their spread but for the listing's 0.07 to 0.25 ms at 0.1 ms. The attribution separated waiting from CPU per thread and per handler stage (the catch-up, the watch, the read-back), the bridge's read from the reader's CPU, and the reader's waits on the app's answer (61% of a read) from those on its foreground-owner queries (25%). It could not see into the app's accessibility server, the Android agent or the emulator's guest, whose time is the daemon's wait on them, nor attribute about 2 ms of the server's 3.3 per call, which the sampling saw as hand-offs between the runtime's thread and tokio's blocking pool.

### D3. Fixes gated on the ranking
A hot spot is worth a fix when its self time is a measurable share of a model's turn: a listing's or a click's wall time at the daemon, or the CLI's start. Each fix comes with a microbenchmark that times the spot and showed the cost red first (a threshold the spike's number sets), the fix, and `just micro --ab <ref>` between the two refs on a quiet host, recorded in this decision's "As built" with the share it bought. A hot spot that is the bridge's or the agent's time is recorded as such, not worked around. No more than three fixes in this change; the rest go to the backlog with their numbers.

### D4. What the profiles leave behind
The traces and flamegraphs hold the host's paths, the user's name in them and every symbol; they stay in the private spike folder beside the scripts, as S13's and S14's programs do. The report holds tables of functions and crates, shares and times, and the commands to rerun.

## Risks / Trade-offs

- [The profiler's overhead moves the numbers] → 1 ms sampling on a release build; the spans in the replies, taken with and without the profiler, say by how much.
- [A hot spot is in the bridge or the agent] → recorded as the floor; the report says what a different read would need.
- [A fix changes a listing] → the mirror's tests are the guard; a fix that changes a line is no fix.
