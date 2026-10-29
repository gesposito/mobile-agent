# S15: profiles of the daemon, the CLI's start and the server's round trip

Spike S15 (task 1.1 of `profiling`) takes the first sampling profiles of this code, for designs D1 and D2 of that change: the daemon on the iOS Simulator and on the Android emulator through scripted sequences (listings of the fixture's 500-cell Big tab and of Settings, clicks, fills, change waits, screenshots, cold opens), the CLI's start and the MCP server's round trip, and the listing's allocations. Each path's time is split between waiting (on the device, the accessibility bridge, the Android agent, the mirror's own windows) and this code's frames, by crate and function, so that design D3 can choose what to fix. The profiles, their exports, flamegraphs and the scripts that fold them are kept privately with the other spike programs (`docs/spikes/README.md`): they hold the host's paths and environment. Snapshot date: 2026-10-29.

**In brief.** This code's CPU is a small part of every command but the listing; the time of a click or a fill is waiting, and on the Simulator most of it is the bridge's tree read and the mirror's own waits for frames.

- **A listing is CPU, under a millisecond.** An interactive snapshot of the Big tab (506 lines) took 0.73 / 1.35 ms at p50 / p95 on the daemon's socket on the Simulator and 0.83 / 0.95 on the emulator, all of it CPU: the mirror's listing 0.48 ms (the lines, the views, the refs), the reply's JSON 0.12 ms, the connection's read and write 0.24 ms. Allocation is 21 to 31% of its samples: 10.9 thousand allocations and 1.52 MB per listing, against the 7.7 thousand and 0.84 MB the microbenchmarks count, the gap being the handler's refs map, built as JSON values, which no benchmark counts.
- **A click is waiting.** A click on the Simulator's selected Big tab took 491 ms at p50 back to back, the target's worker thread on CPU for 7 ms of a mean 645 (1.1%): 54% of the click waited for 2.2 tree reads through the reader, 144 ms each, and 45% for frames, in the catch-up before the tap and in the watch after it. A fill was 99% waiting: its keys' cadence, the read-back's 200 ms hold and its reads. On the emulator a click on the selected tab was 205 ms, 97% the 200 ms quiet window on the agent's events.
- **The tree read is the bridge's.** Inside the Simulator, a read of the reader's spent 61% waiting for the app's answer to the accessibility snapshot, 25% in the two queries of the foreground owner around it, and 13% on CPU, three quarters of that in CoreFoundation decoding the answer; the reader's own extraction is about 1% of a read and the daemon's decode and replica update under 1 ms.
- **The catch-up waits on screens that animate.** With a second between commands, as a model's turn leaves, a listing of the fixture's Anim tab, whose spinner turns, took 204 ms at p50 against 0.65 ms with the spinner stopped: 106 ms waiting for the re-read the mirror schedules no sooner than 200 ms after the last, 10 ms reading.
- **The server adds 3.3 ms, not 0.3.** The production server added 3.3 ms at p50 to a listing over the socket, where spike S13's server adds 0.32 ms with the same client, host and daemon: per call it parses the daemon's 45 KB reply into a JSON tree (0.27 ms), checks the daemon's version by opening its version file and looking up its executable, and hands the call to tokio's blocking pool and back, about 2 ms of waits that no frame of this code holds.
- **The CLI's start is the host's.** `mobile-agent status` took 8.3 ms at p50 against 2.8 ms for `/usr/bin/true`: dyld and libSystem's start 2.3 ms of CPU, the kernel 1.0 ms, and the CLI's own `main` 0.4 ms, of which argument parsing 0.25 ms and the socket and version check 0.15.
- **The frame stream is the daemon's largest CPU.** On the Simulator the frames thread downscales every presented frame before hashing it, 1.2 to 1.3 ms a frame: the frames thread was 81 to 82% of the daemon's samples over the Simulator's sequence, the downscale alone 73 to 74%, 7 to 15% of a core whenever the screen changes and none at rest, on no command's path.

The verdict ranks eight hot spots and gives three a go: the catch-up's wait on an animating screen, the server's per-call path, and the listing's reply built through JSON values. D1 and D2 get their As built.

## Setup

**Host.** Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 27.2 (26B123), Xcode 27.1 (27A9275), xctrace 27.1. Other sessions shared the host: before each profiled sequence the 1-minute load average was waited for under 6 (60 s polls, up to 20 minutes), and every sequence started at 3.0 to 6.0. Six ended above it, at 6.6 to 9.3, and one at 41.9, the emulator's 1 ms run, whose last minute met another session's job; their spans agree with the other runs' (Measurements, first table). The host runs an endpoint security agent that inspects file opens and process starts by programs outside the system; an open and read of a small file took 0.32 ms at p50 from a script, a `stat` 2 µs. It weighs on the CLI's start and the server's version check below.

**Toolchain and build.** Rust 1.98.1, the repository's toolchain. The worktree of `profiling` at `87068c9`, which adds the `profiling` Cargo profile: release with `debug = 1` (line tables) and `strip = false`, since release builds strip the debug information and a profiler then names neither inlined functions nor lines. `cargo build --profile profiling -p daemon -p cli -p mcp` took 1 min 41 s from a clean target directory: `mobile-agent-daemon` 17,959,000 bytes with the Android agent embedded, `mobile-agent` 1,896,272, `mobile-agent-mcp` 6,066,384. The debug information stays in the object files, which the binary's debug map names, and xctrace read it there: its exports carry inlined frames and source lines. The Simulator's reader is built by the daemon's build script in release, with symbols and no line tables.

**Devices.**

- The Simulator: an iPhone 17 on iOS 26.5 (23F77), CoreSimulator 1174.9.2 with dtuhidd as the input transport, booted headless with `simctl boot`; the daemon on a private socket directory with `--drivers simulator`, the reader spawned into the Simulator (`simulator-reader` 0.1.0, ready 452 ms after its spawn). The SwiftUI fixture (`dev.mobileagent.fixture.swiftui`, Debug) and Settings.
- The emulator: the AVD `pixel_api36` (Android 16, API 36, `sdk_gphone64_arm64`, 1080x2400 at 420 dpi), cold-booted with `-no-snapshot-load -no-window -no-audio -no-boot-anim -gpu host -grpc 8554 -grpc-use-token`; the daemon with `--drivers android`, the agent 0.4.3 over protocol 1, taps, swipes and frames through the emulator's controller. The Expo fixture (`dev.mobileagent.fixture.expo`, Release, installed before) and Settings.

**The tool, and why.** Prior art on the host: Xcode's `xctrace` records Instruments templates from the command line. `xctrace record --template 'Time Profiler'` attaches to a process by pid or name (`--attach`), starts one (`--launch -- <command>`) or records all (`--all-processes`); with `--device <Simulator>` it attaches to a process running inside a Simulator, which is how the reader was profiled. Its recording options (`--show-recording-options`, then `--recording-options <file>`) give the sampling: every 1 ms on CPU by default, every 0.1 ms with `highFrequencySampling`, and with `recordWaitingThreads` blocked threads too, a blocked interval then being reported when the thread wakes, its weight the time it was blocked, while CPU is sampled every 5 ms. `xctrace export --input <trace> --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]'` writes the samples as XML, one row per sample with its time, thread, weight and backtrace, frames and binaries given once and referred to after. The Allocations template's data is exported through its track's details, `/trace-toc/run[@number="1"]/tracks/track[@name="Allocations"]/details/detail[@name="Statistics"]` and `[@name="Allocations List"]`. `samply` records Rust binaries into the Firefox Profiler's format (design D1); it was not installed, since xctrace did everything D2 asks, the process inside the Simulator included, and its export folds into tables with a short script. Three limits of xctrace's, found here and kept in the method: kperf, which Time Profiler samples through, takes one recording at a time (a second recording at once failed with "could not lock kperf" and starved the first), so the daemon and the reader were recorded in separate runs of the same sequence; Allocations attaches to no process without the `get-task-allow` entitlement, which the linker's ad hoc signature lacks (`--attach` hung, `--launch` failed with "Failed to attach to target process"), so a copy of the daemon was signed again ad hoc with it for those runs alone; and Allocations exports the whole run's statistics, freed allocations included, but lists only the allocations still live at its end, so the listing's allocations come from a difference of two runs.

**The sequences.** A client of the private copy (`drive.py`) sends the daemon the lines the CLI sends, one connection per request as the CLI connects, and records each request's time from the connect to the reply line with the reply's spans. It keeps the CLI's process starts off the host during the profiles as `batch` would, and times every request, which `batch` does not print. On the Simulator, in one session:

1. `open dev.mobileagent.fixture.swiftui` with `relaunch`, `click #tab-big`, `wait --settle`, `snapshot -i`;
2. 100 `snapshot -i`, then 100 `snapshot -i --delta` (each unchanged);
3. 50 `click #tab-big`, the tab already selected;
4. `click #tab-login`, `wait --settle`, `snapshot -i`; 20 pairs of `fill #login-username alice<n>` and `fill #login-username ""`; `click #login-submit`, which dismisses the keyboard, and `wait --settle`;
5. `click #tab-anim`, `wait --settle --timeout 1500`, `snapshot -i`; 20 pairs of `click #anim-toggle` and `wait --change`;
6. 20 `screenshot <file>`;
7. `open com.apple.Preferences` with `relaunch`, `wait --settle`, `snapshot -i`; 50 `snapshot -i`; 20 rounds of `click text=General`, `wait --settle`, `click #BackButton`, `wait --settle` (iOS has no back key, and the button is the navigation bar's).

The emulator ran the same with the Expo fixture, `com.android.settings`, `click text=Network` and `press back`, at the same counts, the emulator being quick; then 10 `open com.android.settings` with `relaunch`, each followed by a `snapshot -i`. A third Simulator sequence put a second between commands, as a model's turn does: 30 `snapshot -i` of the Anim tab with the spinner turning, 30 with it stopped, then 20 `click #tab-big` on the Big tab.

**Runs.** Each sequence ran once without a profiler, once with the daemon attached at 1 ms, once at 0.1 ms and once with waiting threads; on the Simulator twice more with the reader attached instead (1 ms and waiting threads), the third sequence without a profiler and with waiting threads, the cold opens without, at 1 ms and with waiting threads. Runs on 2026-10-29 from 21:28 to 22:50. The CLI's start: 100 `status` against the Simulator's daemon, timed by `hyperfine --warmup 10 --runs 100 -N` beside `/usr/bin/true` and `mobile-agent --version`, and recorded at 0.1 ms with all processes while a shell ran them, with 100 `--version` and 100 `/usr/bin/true` in a second recording. The server: the private client `mcpclient.py` started `mobile-agent-mcp` on the same daemon, opened the 2025-11-25 era with `initialize`, made 10 warm-up calls and 100 timed `tools/call snapshot` (`interactive: true`, `delta: false`, `session: default`), each paired with the same request on the daemon's socket, their order alternating, as S13 paired them; once without a profiler, once at 0.1 ms and once with waiting threads attached to the server, once with unchanged deltas, once with `MOBILE_AGENT_DAEMON_BIN` set, and once through S13's spike server. Allocations: four Allocations runs launching the signed copy on the Simulator, two with the setup and 50 `snapshot -i` of the Big tab, two with the setup and a pause as long as those 50 took.

**Folding.** The private scripts read each export, demangle the names (`rustc-demangle`) and align the samples to the client's requests: a trace's start date is its time zero to the millisecond (within 0.04 ms of the alignment on the listing's handler frames in the 1 ms run), refined within 3 ms by the share of snapshot-handler samples that fall inside snapshot requests (98.6 to 100%). A path's samples are those inside its requests. A sample's leaf names where the CPU went: this code (the workspace's crates), allocation (libsystem_malloc and `alloc`'s allocator calls), formatting (`core::fmt`), JSON (serde_json), memory copies, the Rust library and other crates, kernel calls, system frameworks. Its nearest frame of this code charges it, with the runtime it calls, to one of this code's functions. In the waiting runs a blocked interval counts where it overlaps a request, by the call it blocked in and the frames of this code above it. Percentiles are nearest-rank.

**Rerun**, with the Simulator booted, the emulator cold-booted and the binaries built with the `profiling` profile, on private socket directories `$IOS` and `$ANDROID`:

```sh
mobile-agent-daemon --socket-dir "$IOS" --drivers simulator --idle-timeout 4h &
xcrun xctrace record --template 'Time Profiler' --attach "$(cat "$IOS/daemon.pid")" --output daemon.trace &
python3 drive.py "$IOS" ios client.jsonl          # in the private copy; then stop the recording with SIGINT
xcrun xctrace export --input daemon.trace --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]' --output daemon-tp.xml
python3 fold.py daemon-tp.xml --client client.jsonl --phases 'big-snapshot;big-click-selected'
```

The private copy's `run-seq.sh <ios|android> <none|cpu|hf|wait> <tag> [sequence] [daemon|reader]` does the above with the quiet-host wait, `mcp-run.sh` the server's runs, `alloc-run.sh` the Allocations runs, `gen-tables.sh` this report's tables and `flame.py` the flamegraphs. Without the private copy, the sequences are CLI commands, `mobile-agent batch "snapshot -i" ...` with the counts above, and the spans are in each entry's `result`.

## Measurements

### The spans, with and without the profiler

The Simulator's and the emulator's sequences, ms at p50 / p95 over the counts of the sequence, the client's time from the connect to the reply (wall) and the reply's own spans:

| Path | Span | No profiler | 1 ms | 0.1 ms | Waiting threads |
|---|---|---:|---:|---:|---:|
| Simulator, `snapshot -i` of the Big tab (100) | wall | 0.73 / 1.35 | 0.73 / 1.43 | 0.98 / 1.15 | 0.74 / 1.23 |
| | `readMs` | 0.47 / 0.86 | 0.46 / 0.89 | 0.57 / 0.64 | 0.47 / 0.80 |
| Simulator, unchanged delta (100) | wall | 0.99 / 1.12 | 0.82 / 0.84 | 1.11 / 1.28 | 0.84 / 0.86 |
| Simulator, `click #tab-big`, selected (50) | wall | 491.26 / 1234.32 | 495.43 / 1235.30 | 496.40 / 1238.17 | 492.73 / 1239.06 |
| | `watchedMs` | 146.88 / 892.64 | 149.78 / 886.81 | 147.71 / 886.13 | 145.98 / 888.93 |
| | `dispatchMs` | 0.19 / 1.05 | 0.19 / 1.23 | 0.17 / 0.58 | 0.38 / 1.66 |
| Simulator, `fill` of 6 characters (20) | wall | 457.65 / 487.59 | 459.11 / 482.13 | 459.78 / 502.95 | 453.01 / 494.21 |
| | `readBackMs` | 301.76 / 309.06 | 304.98 / 313.80 | 298.88 / 320.47 | 300.55 / 315.07 |
| Simulator, `fill ""` (20) | wall | 364.78 / 382.72 | 367.56 / 371.69 | 366.04 / 383.91 | 364.60 / 381.57 |
| Simulator, `click #anim-toggle` (20) | wall | 289.10 / 599.09 | 286.69 / 498.16 | 225.67 / 510.49 | 221.12 / 518.03 |
| Simulator, `screenshot` (20) | wall | 4.99 / 9.66 | 4.83 / 8.25 | 5.81 / 9.23 | 5.71 / 11.57 |
| Simulator, Settings' `snapshot -i` (50) | wall | 0.16 / 0.27 | 0.11 / 0.17 | 0.12 / 0.25 | 0.12 / 0.28 |
| Simulator, `click text=General` (20) | wall | 195.21 / 456.60 | 204.84 / 218.33 | 199.51 / 473.41 | 199.68 / 468.75 |
| Simulator, `wait --settle` in Settings (40) | `elapsedMs` | 1321.03 / 2181.60 | 1288.35 / 2183.97 | 1316.53 / 2181.35 | 1309.18 / 2180.80 |
| Emulator, `snapshot -i` of the Big tab (100) | wall | 0.83 / 0.95 | 0.85 / 0.96 | 0.90 / 1.02 | 0.79 / 0.95 |
| Emulator, unchanged delta (100) | wall | 1.11 / 1.17 | 1.17 / 1.38 | 1.10 / 1.28 | 1.09 / 1.16 |
| Emulator, `click #tab-big`, selected (50) | wall | 205.17 / 209.22 | 204.12 / 208.18 | 205.94 / 208.76 | 205.54 / 209.31 |
| Emulator, `fill` (20) | wall | 32.46 / 89.95 | 34.33 / 86.94 | 34.86 / 86.02 | 34.75 / 88.41 |
| Emulator, `click #anim-toggle` (20) | wall | 228.05 / 236.95 | 233.07 / 319.25 | 221.88 / 250.59 | 227.55 / 300.89 |
| Emulator, `wait --change` (20) | wall | 16.16 / 49.77 | 13.07 / 17.60 | 15.28 / 18.44 | 16.53 / 52.04 |
| Emulator, `screenshot` (20) | wall | 39.75 / 43.94 | 39.04 / 47.39 | 38.79 / 43.95 | 42.08 / 52.69 |
| Emulator, `click text=Network` (20) | wall | 265.13 / 281.62 | 263.80 / 354.20 | 260.10 / 273.67 | 260.17 / 272.40 |
| Emulator, `wait --settle` in Settings (40) | `elapsedMs` | 726.92 / 756.48 | 735.75 / 758.47 | 731.36 / 767.75 | 725.12 / 755.44 |

- **The profiler's overhead.** At 1 ms and with waiting threads, every span stays within the run-to-run spread; at 0.1 ms the sub-millisecond listing gains 0.07 to 0.25 ms at p50 on both targets. The tables below take the listing's frames from the 0.1 ms run for its resolution, and the 1 ms run, which agrees on the shares, is in the private copy.
- **Bimodal spans.** The Simulator's `wait --change` on the Anim tab answered at once in some runs (1.7 ms at p50) and after a re-read in others (218 ms), with the toggle's alternation; a click back in Settings likewise (175 against 404 ms at p50). They depend on what the screen did before the command, not on the profiler.
- **A click on the selected tab on the Simulator is no no-op.** The tab bar redraws on the press, frames change and the tree changes, so its watch sees both (`treeDeltaMs` 143 ms at p50 in 31 of 50) and lasts up to the 1 s cap (893 ms at p95); on the emulator it changes nothing and waits out the 200 ms quiet window, as on S13's fake.

### Where a command's time goes at the daemon (Simulator)

The target's worker thread runs every command; mean ms per request over the sequence's requests, from the waiting-thread run, its blocked time split by the call it blocked in and the stage of the handler above it:

| Command | Mean wall | Worker on CPU | Tree reads through the reader | Waits for frames, due re-reads and quiet windows | Input (holds, keys and their cadence) |
|---|---:|---:|---:|---:|---:|
| `click #tab-big`, back to back | 645.3 | 6.9 (1.1%) | 346.5: 107.1 in the catch-up before the tap, 239.4 in the watch after it | 295.4: 140.8 in the catch-up, 154.6 in the watch | 0.2 |
| `fill alice<n>` | 493.0 | 3.6 (0.7%) | 127.5: 98.5 in the read-back | 212.1: 202.1 the read-back's hold | 152.5 |
| `fill ""` | 365.9 | 2.4 (0.7%) | 109.4 | 201.5 | 54.8 |
| `click #anim-toggle` (held 40 ms) | 236.7 | 3.8 (1.6%) | 41.4 | 148.6 | 44.5 |
| `wait --change` after it | 130.5 | 0.3 (0.2%) | 10.5 | 119.3, in the catch-up | |
| `click text=General` | 257.4 | 1.4 (0.5%) | 226.2, in the watch | 30.1 | |
| `click #BackButton` | 327.5 | 8.6 (2.6%) | 190.9 | 130.5 | |
| `wait --settle` in Settings | 1555.6 | 4.3 (0.3%) | 103.3 | 1445.9, the settle's quiet window | |
| `screenshot` | 7.4 | 5.2 (64%) | | 2.9, the display's capture | |
| `snapshot -i` of the Big tab | 0.8 | all of it, with the connection's thread | | | |

- **Reads.** The worker's tree reads, counted as runs of consecutive samples inside the driver's read: 2.16 per click on the Big tab at 144 ms at p50 (292 ms at p95) for its 546 nodes, 2.10 per fill at 49 ms on the Login screen, 1.15 per click into General at 196 ms, 1.70 per click back at 109 ms; none during the listings, which the replica serves (`servedFromReplica`), and one in the whole sequence between requests.
- **The catch-up** (`Attached::catch_up`) runs before every snapshot, ref and selector: when a frame changed after the latest read began, it waits for the re-read the mirror schedules no sooner than its minimum interval, 200 ms, after the previous read ended, then for the read. Back to back, each click on the Big tab found the previous click's frames newer than the last read: 249 ms of the 645, the click's largest stage after the watch.
- **The other threads** during the clicks: the frames thread 45.1 ms of CPU per click (below), the unnamed threads 6.1 ms, 94% of it CoreSimulator's and libdispatch's callbacks and XPC with none of this code on the stack, and the connection threads under 0.1 ms.

### The tree read, through the bridge

The reader recorded inside the Simulator over the whole sequence, its read thread in the waiting run; 384 reads, each from a request on its connection to its answer:

| Per read | p50 | p95 | Total | Share |
|---|---:|---:|---:|---:|
| The whole read | 100.7 ms | 207.6 ms | 42.2 s | 100% |
| Waiting for the app's answer to `userTestingSnapshotForElement` | 62.9 ms | 111.1 ms | 25.9 s | 61% |
| Waiting on the foreground owner, `+[AXElement primaryApp]`, before and after the snapshot (1.95 queries a read) | 9.3 ms | 99.0 ms | 10.8 s | 25% |
| On CPU (sampled every 5 ms) | 10 ms | 30 ms | 5.6 s | 13% |

The reader's CPU at 1 ms in its own run, 14.4 ms a read on average:

| Where | Share of the reader's CPU |
|---|---:|
| The snapshot call's own work in XCTest's frameworks: CoreFoundation decoding the answer's binary property list and building its dictionaries (CoreFoundation 49% of all samples, libobjc 14%, libsystem_malloc 10%) | 74.5% |
| The foreground-owner queries | 6.7% |
| Asserting automation mode | 5.3% |
| The reader's extraction of the dictionaries into elements (`Extractor::node`, `extract::text`) | 6.4% |
| The elements' payload, its CBOR encoding and the connection | about 2% |
| Elsewhere: the threads' start, the process lookups | about 5% |

- The daemon's side of a read, inside the worker: decoding the payload (`protocol` and minicbor, 0.36 ms a read), the replica's update (`Replica::integrate`, 0.45 ms) and the view's facts, under 1% of a read's 144 ms.
- **The floor.** A read is the app's accessibility server answering and the system's owner queries; this code's share, the reader's extraction and encoding and the daemon's decode and update, is about 2 ms of a 100 to 150 ms read.

### The listing's frames

**The Simulator's interactive listing of the Big tab**, 100 requests at 0.1 ms, 0.893 ms of CPU per request on all threads (the worker 0.59, the connection's thread 0.28, the accept loop 0.03):

| Where the CPU went | Share | ms per listing |
|---|---:|---:|
| Allocation | 31.4% | 0.280 |
| This code | 17.5% | 0.156 |
| The Rust library and other crates | 16.5% | 0.147 |
| Kernel calls (the socket's read and write) | 13.0% | 0.116 |
| JSON (serde_json) | 6.9% | 0.062 |
| Memory copy and compare | 6.8% | 0.061 |
| Formatting | 5.8% | 0.052 |
| System libraries | 2.1% | 0.019 |

| # | Function (self time) | Crate or library | Share | ms per listing |
|---:|---|---|---:|---:|
| 1 | `_xzm_malloc_tc` | libsystem_malloc | 7.1% | 0.063 |
| 2 | `<Vec as SpecFromIterNested<mirror::refs::Ref, …>>::from_iter` | alloc | 6.0% | 0.054 |
| 3 | `_platform_memmove` | libsystem_platform | 5.4% | 0.048 |
| 4 | `_xzm_free_tc` | libsystem_malloc | 5.0% | 0.045 |
| 5 | `__sendto` | libsystem_kernel | 4.4% | 0.039 |
| 6 | `_malloc_zone_malloc` | libsystem_malloc | 4.3% | 0.038 |
| 7 | `<String as core::fmt::Write>::write_str` | core::fmt | 3.7% | 0.033 |
| 8 | `<serde_json::Value as Display>::fmt`'s writer, `write_all` | serde_json | 3.5% | 0.031 |
| 9 | `<core::hash::sip::Hasher as Hasher>::write` | core | 2.6% | 0.023 |
| 10 | `mirror::view::hints` | mirror | 2.2% | 0.020 |
| 11 | `mirror::mirror::element` | mirror | 2.0% | 0.018 |
| 12 | `RawVecInner::reserve::do_reserve_and_handle` | alloc | 1.9% | 0.017 |
| 13 | `serde_json::ser::format_escaped_str` | serde_json | 1.9% | 0.017 |
| 14 | `mirror::json::push_str` | mirror | 1.8% | 0.016 |
| 15 | `_free` | libsystem_malloc | 1.7% | 0.015 |
| 16 | `mirror::view::window_facts` | mirror | 1.6% | 0.014 |
| 17 | `__xzm_xzone_free_to_freelist_chunk` | libsystem_malloc | 1.6% | 0.014 |
| 18 | `xzm_malloc_zone_size` | libsystem_malloc | 1.6% | 0.014 |
| 19 | `_platform_memcmp` | libsystem_platform | 1.3% | 0.012 |
| 20 | `RefTable::assign` | mirror | 1.3% | 0.012 |

| # | This code's function, with the runtime it calls | Share | ms per listing |
|---:|---|---:|---:|
| 1 | `daemon::wire::reply_line`, the reply's JSON | 13.4% | 0.120 |
| 2 | `mirror::render::element_line`, a listing line | 12.9% | 0.115 |
| 3 | `Mirror::snapshot`'s own body | 12.3% | 0.110 |
| 4 | `daemon::server::serve`, the connection's read and write | 11.8% | 0.105 |
| 5 | `mirror::mirror::element` | 8.8% | 0.079 |
| 6 | `mirror::json::push_str` | 4.9% | 0.044 |
| 7 | `mirror::view::window_facts` | 4.8% | 0.043 |
| 8 | the snapshot handler's closure that builds the refs map | 4.0% | 0.036 |
| 9 | `<Ref as Display>::fmt` | 3.4% | 0.030 |
| 10 | `daemon::handlers::view::remember` | 2.8% | 0.025 |
| 11 | none of this code on the stack | 2.8% | 0.025 |
| 12 | `daemon::handlers::view::snapshot` | 2.7% | 0.024 |
| 13 | `daemon::server::run` | 2.5% | 0.022 |
| 14 | `mirror::view::hints` | 2.4% | 0.021 |
| 15 | `RefTable::assign` | 2.0% | 0.018 |
| 16 | `worker::Handle::run` | 1.5% | 0.013 |
| 17 | `mirror::visibility::listed` | 0.9% | 0.008 |
| 18 | `View::build` | 0.9% | 0.008 |
| 19 | `daemon::worker::serve` | 0.9% | 0.008 |
| 20 | `render::Walker::walk` | 0.8% | 0.007 |

- Inclusive per listing: `Mirror::snapshot` 0.480 ms, its rendering (`mirror::render`) 0.186, `daemon::wire::reply_line` 0.120, `View::build` 0.055 (the view is current, served from the replica), SipHash 0.023.
- **The emulator's listing** has the same shape: 0.846 ms of CPU per listing, allocation 30.7%, this code 24.1%, the Rust library 18.9%, kernel calls 8.3%, JSON 5.8%, formatting 5.1%; on top `element_line` 14.3%, `Mirror::snapshot` 13.7%, `reply_line` 11.9%, `server::serve` 8.5%.
- **An unchanged delta costs more than the listing.** 1.08 ms of CPU against 0.89: the handler lists the screen, then `Mirror::delta_snapshot` lists it again to compare and writes its JSON, which the handler parses back into a value (`Mirror::snapshot` 0.905 ms inclusive, twice the listing's; `delta_snapshot` 0.470), allocation 34.8% of its samples, for a result that says `unchanged`.
- **Settings' listing**, 15 lines, 0.11 ms of CPU; nothing in it stands out.

### A click's frames

**The worker during the Simulator's clicks on the Big tab**, 50 clicks at 0.1 ms, 4.24 ms of CPU per click:

| # | Function (self time) | Crate or library | Share | ms per click |
|---:|---|---|---:|---:|
| 1 | `<core::hash::sip::Hasher as Hasher>::write` | core | 8.5% | 0.362 |
| 2 | `__psynch_cvwait` (entering the wait) | libsystem_kernel | 8.1% | 0.344 |
| 3 | `Replica::integrate` | mirror | 5.3% | 0.226 |
| 4 | `_xzm_malloc_tc` | libsystem_malloc | 4.8% | 0.202 |
| 5 | `_xzm_free_tc` | libsystem_malloc | 3.7% | 0.158 |
| 6 | `mirror::view::window_facts` | mirror | 3.0% | 0.128 |
| 7 | `poll` | libsystem_kernel | 2.4% | 0.102 |
| 8 | `View::build` | mirror | 2.3% | 0.098 |
| 9 | `_free` | libsystem_malloc | 2.2% | 0.094 |
| 10 | `minicbor::Decoder::skip` | minicbor | 2.2% | 0.094 |
| 11 | `<protocol::snapshot::Node as Decode>::decode` | protocol | 2.1% | 0.088 |
| 12 | a function the linker folded | this code | 2.0% | 0.086 |
| 13 | `_malloc_zone_malloc` | libsystem_malloc | 2.0% | 0.086 |
| 14 | `mirror::render::element_line` | mirror | 1.8% | 0.076 |
| 15 | `RefTable::of` | mirror | 1.7% | 0.074 |
| 16 | `_platform_memmove` | libsystem_platform | 1.7% | 0.072 |
| 17 | `<Vec as SpecFromIterNested<mirror::refs::Ref, …>>::from_iter` | alloc | 1.5% | 0.064 |
| 18 | `<String as FromIterator>::from_iter` | alloc | 1.5% | 0.062 |
| 19 | `minicbor::Decoder::unsigned` | minicbor | 1.5% | 0.062 |
| 20 | `mach_absolute_time` | libsystem_kernel | 1.4% | 0.060 |

- Inclusive per click: `Mirror::refresh` 1.99 ms (2.16 reads: decoding 0.78, the replica's update 0.98), the catch-up's CPU 1.19, `Mirror::snapshot` 0.81, `View::build` 0.52, SipHash 0.36 (in the replica's identity matching and the ref table), `Mirror::hit` 0.07.
- By crate of the nearest frame: mirror 61.6%, simulator 17.8% (the frame queue and the reader's link), protocol 15.8%, daemon 4.6%.
- **The emulator's click** on the selected tab: 2.89 ms of CPU per click on the worker, `View::build` 18.3% of it, SipHash 11.5%, `window_facts` 10.3%; the tree was read 3 times in the 50 clicks, the agent's events keeping the replica current.

**The frames thread**, the same clicks: 45.1 ms of CPU per click, 7.7% of a core while they ran.

| # | Function (self time) | Crate or library | Share | ms per click |
|---:|---|---|---:|---:|
| 1 | `simulator::pipeline::scale_bgra`, the box downscale of each frame | simulator | 90.0% | 40.59 |
| 2 | `simulator::macos::frames::read_frames` | simulator | 5.7% | 2.58 |
| 3 | `__bzero` | libsystem_platform | 1.0% | 0.43 |
| 4 | `iokit_user_client_trap` (the surface's lock) | IOKit | 0.6% | 0.28 |
| 5 | `mach_msg2_trap` | libsystem_kernel | 0.4% | 0.20 |
| 6 | `__semwait_signal` | libsystem_kernel | 0.4% | 0.19 |

- Inclusive: `read_frames` 100%, `scale_bgra` 91.7%, the surface's read 86.1%, `content_hash` and `thumbnail_hash` 6.5% (2.9 ms per click). The frames were not counted; at the display's 60 a second throughout a click, 45.1 ms is 1.3 ms a frame, the microbenchmarks' `simulator/scale_bgra/1206x2622-to-460x1000` (1.14 ms) and `thumbnail_hash/460x1000` (0.12 ms) together.
- **At rest it does nothing.** In 15 s on the still Big tab after a settle the daemon was on CPU 34 ms in all, the frames thread 9 ms of it; over the whole Simulator sequence the frames thread was 81 to 82% of the daemon's samples and `scale_bgra` alone 73 to 74%, in the 1 ms and the 0.1 ms runs.

### Fill, screenshot and Settings

- **A fill** on the worker: 2.47 ms of CPU; by the nearest function of this code `window_facts` 17.6%, `Replica::integrate` 11.5%, the input's XPC submit 10.1%, `class_role` 8.5%; SipHash 7.3% in self time.
- **A screenshot**: 5.05 ms of CPU on all threads, `scale_bgra` 35% (1.77 ms), the JPEG encoder 15.6% with libjpeg-turbo's SIMD routines 12%, the display's capture through CoreSimulator 25% with none of this code on the stack; against `simulator/pipeline/frame-to-jpeg` (2.34 ms), which times the downscale and the encoding without the capture and the file.
- **A click into General**, on the worker: 2.09 ms of CPU, `RefTable::prune` from the listing 17.9% (sorting and collecting refs, 0.375 ms), `window_facts` 10.0%, `View::build` 9.8%, `Replica::integrate` 9.6%.

### With a model's pause between commands

The third sequence on the Simulator, a second between commands; ms, p50 / p95 without the profiler, and the mean split from its waiting-thread run:

| Command | Wall | Split |
|---|---:|---|
| `snapshot -i` of the Anim tab, spinner turning (30) | 204.14 / 319.82 | the catch-up's wait for the re-read's due time 106.3, the read 10.3, CPU 0.6 |
| `snapshot -i` of the Anim tab, spinner stopped (30) | 0.65 / 1.73 | served from the replica |
| `click #tab-big`, selected, after a second (20) | 293.86 / 459.29 | the catch-up 179.0 (a read 135.9, a wait 40.3, CPU 2.8); the watch 143.9 (a read 142.1); CPU 9.2 in all |

- On a screen that animates, every snapshot waits for the next scheduled re-read, between 0 and 200 ms after the previous read ended, then reads: 0.65 ms becomes 204 ms at p50.
- On the Big tab the display went on presenting frames for more than the second after a click on the selected tab (the frames thread busy in every 100 ms of the second before the next click), so the next click's catch-up still found a frame newer than the last read and read the tree first: 179 ms of the 326.

### Emulator

The worker's time per command from the waiting-thread run, mean ms:

| Command | Mean wall | Worker on CPU | The agent's replies | The mirror's waits |
|---|---:|---:|---:|---:|
| `click #tab-big`, selected | 214.9 | 2.4 (1.1%) | input 3.0, a read 1.8 | 209.5, the quiet window on the agent's events |
| `fill alice<n>` (set directly) | 41.7 | under 0.1 | tree reads 25.6, input 14.5 | 1.1 |
| `fill ""` | 15.7 | 0.1 | reads 9.1, input 7.3 | |
| `click #anim-toggle` | 234.6 | under 0.1 | a read 26.5 | 171.1, a read's wait until the controller's tap is 200 ms old; 36.5 on events |
| `wait --change` | 25.1 | 0.3 | a read 25.1 | |
| `screenshot` | 44.3 | 1.6 | the agent's screenshot 28.6; adb 14.8 | |
| `click text=Network` | 260.9 | 1.3 | a read 53.2 | 109.1, the tap's 200 ms; 97.8 on events |
| `press back` | 6.9 | under 0.1 | input 7.1 | |
| `wait --settle` in Settings | 714.3 | 1.1 | reads 54.8 | 659.2, the quiet window |
| `open com.android.settings`, cold (10) | 1756.4 (p50 1352, `firstTreeMs` 918) | 5.6 (0.3%) | `am start -W` 528.1, tree reads 378.1 | 833.4 on events |

- **The controller's frame stream** is the emulator's background CPU: while Settings settled, 26.2 ms of CPU per settle on the controller's thread (3.6% of a core), `__recvfrom` of the gRPC frames 59.8%, copies 10.4%, xxh3 6.6% (`FrameHasher::key`, which `android/frame-hasher/key-540x1200` times at 55 µs), `writev` 6.1%. A cold open: 60.6 ms of CPU, 95% of it on that thread.
- The agent runs inside the emulator, out of the host profiler's reach: its time is the worker's wait on the agent's socket.

### The CLI's start

100 runs each, ms, p50 / p95, `hyperfine -N` with 10 warm-up runs, load 3.96:

| Command | Wall | User | System |
|---|---:|---:|---:|
| `/usr/bin/true` | 2.81 / 3.13 | 0.49 | 0.80 |
| `mobile-agent --version` | 5.86 / 6.85 | 1.21 | 1.00 |
| `mobile-agent status`, the daemon running | 8.32 / 10.17 | 1.46 | 1.24 |

CPU per run of `status` at 0.1 ms, all processes recorded, the shell's child before its exec left out (it is the shell's fork, 0.72 ms):

| Stage | ms per run | Share |
|---|---:|---:|
| dyld and libSystem's start, before `main` | 2.33 | 60.0% |
| The kernel, no user stack (the exec, page faults) | 1.02 | 26.3% |
| Argument parsing (clap) | 0.25 | 6.4% |
| The socket: connect, the version check, the request and the reply | 0.15 | 3.8% |
| Elsewhere in `main` (output, the settings file, the Rust runtime's start) | 0.14 | 3.5% |
| In all | 3.89 | 100% |

- **dyld.** `/usr/bin/true` spends 1.52 ms in dyld and libSystem's start, the CLI's `--version` 2.40: symbol lookups for the binary's 124 imports (`GenericTrie::find`, `hasExportedSymbol`, 0.33 ms a run), file opens (0.27 ms) and page protection (0.13 ms). The binary binds through classic opcodes (`LC_DYLD_INFO_ONLY`), not chained fixups.
- **The rest of the wall time**, 8.3 against 3.9 ms on CPU, is the process off CPU: its creation and exec, which `/usr/bin/true` also waits through (2.8 ms of wall against 1.3 on CPU), and the daemon's reply, 0.35 ms of the daemon's CPU per `status` in the same recording.
- The CLI's own code is 0.4 ms of 8.3; the README's 6.88 ms at p50 (macOS 26.6.2, a fake target) and this 8.32 differ by the host's process start.

### The server's round trip

100 timed calls after 10 warm-up, ms, p50 / p95, paired per call with the same request on the socket:

| Server | Call | Through the server | Socket | The server's share, paired |
|---|---|---:|---:|---:|
| `mobile-agent-mcp`, no profiler | `snapshot`, 506 lines | 4.43 / 5.57 | 1.05 / 1.23 | 3.40 / 4.46 |
| the same at 0.1 ms | | 4.00 / 6.61 | 0.73 / 1.93 | 3.26 / 5.75 |
| the same, waiting threads | | 4.02 / 6.02 | 0.72 / 1.69 | 3.26 / 4.76 |
| the same, `MOBILE_AGENT_DAEMON_BIN` set | | 4.04 / 6.22 | 0.74 / 0.84 | 3.30 / 5.33 |
| `mobile-agent-mcp`, no profiler | unchanged delta, 1 line | 3.26 / 5.70 | 0.84 / 1.06 | 2.41 / 4.83 |
| S13's spike server, no profiler | `snapshot`, 506 lines | 1.05 / 1.19 | 0.72 / 0.84 | 0.32 / 0.39 |

- **The server's CPU**, 0.587 ms per call at 0.1 ms: `cli::client::send` 0.274 (the daemon's 45 KB reply parsed into a `serde_json` tree, `BTreeMap` inserts for its 505 refs), the rmcp loop on the main thread 0.084, the version check 0.120 (`Endpoint::version`'s open of the daemon's version file, `daemon_executable`'s lookups), the call's future 0.054, the result's text 0.010. Kernel calls are 37.1% of the samples, allocation 21.5%, JSON 15.3%.
- **The waits.** In the two waiting-thread runs the version file's open blocked 1.2 and 2.7 ms at p50 the times it was caught (26 and 30 of the 110 calls), 0.4 and 0.8 ms per call on average. In the calls read in the 0.1 ms run's timeline, the call's first sample on the blocking pool's thread came 1.3 to 1.4 ms after the runtime's thread took the request, the version check 0.8 to 2.2 ms after that, then the socket. So the 3.3 ms are the server's CPU, 0.6, the version file's opens, 0.4 to 0.8, and about 2 ms of waits the sampling attributes to no frame of this code: the hand-offs between the runtime's thread, tokio's blocking pool and the stdio threads, 1.3 ms of them before the pool's thread starts the call.
- **Size-dependent and fixed.** An unchanged delta's share, 2.41 ms, is the fixed part; the listing's 0.9 ms more is its reply's parsing and its text's writing. The client's own parse of the result line, 0.04 ms, is in the server's column.
- S13 measured its spike server at 0.29 to 0.33 ms on 2026-10-16; the same binary gave 0.32 here, so the 3 ms are the production server's path, not the host's: the spike's server held one session, checked no version and called the socket through tokio's own Unix stream on its runtime's thread.

### The listing's allocations

The Allocations template, launching the signed copy of the daemon on the Simulator, heap allocations over the whole run; ms are under Allocations' recording, the 50 listings taking 246 ms in all, 4.9 ms each:

| Run | Heap allocations | Bytes | Live at the end |
|---|---:|---:|---:|
| Setup and 50 listings, first | 710,617 | 114,958,496 | 41,622 |
| Setup and a pause, first | 167,520 | 38,920,304 | 41,605 |
| Setup and 50 listings, second | 714,949 | 116,136,560 | 41,656 |
| Setup and a pause, second | 168,241 | 39,396,064 | 41,597 |
| **Per listing, the difference over 50** | **10,862 and 10,934** | **1,520,764 and 1,534,810** | under 1 |

- Against the microbenchmarks: `mirror/snapshot/interactive-again-grid` counts 7,635 allocations and 542,331 bytes, `daemon/wire/reply-snapshot-big` 21 and 295,346, `daemon/wire/parse-snapshot-request` 4 and 526: 7,660 and 838,203 in all. The daemon's listing makes 3,200 more allocations and 690 KB more.
- **The gap** is the handler's: `handlers::view::snapshot` builds the reply's refs map as `serde_json` values, a string key and an object of two strings for each of the 505 refs, about six allocations each, and copies the listing's text into the reply. `reply-snapshot-big` serializes a reply already built, and `daemon/socket/snapshot-interactive-big`, which runs the whole path, counts no allocations. The SwiftUI tree's 546 nodes against the bench grid's also weigh.
- Nothing grew between runs: 17 to 59 more live allocations in the runs with listings than without.

### The profile against the microbenchmarks

| Frame or path | In the profile | Microbenchmark | Its median | The two |
|---|---|---|---:|---|
| The listing at the socket | 0.73 ms p50 (Simulator), 0.83 (emulator) | `daemon/socket/snapshot-interactive-big` (fake emulator) | 868 µs | agree within 15% |
| `Mirror::snapshot`, interactive, warm | 0.480 ms per listing | `mirror/snapshot/interactive-again-grid` | 343 µs | 1.4 times: the fixture's 546-node SwiftUI tree against the synthetic grid |
| The reply's JSON | 0.120 ms | `daemon/wire/reply-snapshot-big` | 95 µs | agree within 30% |
| The handler's refs map and text | 0.04 to 0.06 ms, about 3,200 allocations | none | | not timed; allocations counted nowhere |
| An unchanged delta of the Big tab | 1.08 ms of CPU | `mirror/delta_snapshot/one-change-list` (a 20-row list, one change) | 56 µs | not comparable: no benchmark times an unchanged delta of the 500-cell screen |
| A tree read's decode | 0.36 ms a read | `protocol/decode/snapshot-500` | 128 µs | 2.8 times: the reader's payload for 546 nodes against the synthetic 500 |
| The replica's update after a read | 0.45 ms a read | `mirror/pump/unchanged-grid` | 210 µs | 2.1 times, the same tree difference |
| The frame's downscale and hash | about 1.2 ms a frame, 45 ms per click | `simulator/scale_bgra/1206x2622-to-460x1000`, `simulator/thumbnail_hash/460x1000` | 1,140 and 120 µs | agree |
| The screenshot's pipeline | 5.05 ms of CPU, 5.0 ms p50 wall | `simulator/pipeline/frame-to-jpeg` | 2,337 µs | the bench leaves out the capture and the file |
| The emulator's frame hash | 1.74 ms per settle | `android/frame-hasher/key-540x1200` | 55 µs | agree at about 30 frames a settle |
| Allocations per listing | 10.9 thousand, 1.52 MB | mirror, reply and parse benches | 7,660, 838 KB | the gap above |
| The catch-up's wait | 106 ms per snapshot of an animating screen | none | | not timed |
| The server's call | 3.3 ms paired | none | | not timed |
| The CLI's start | 8.3 ms | none in the suite; the CLI's test gates the median under 20 ms | | not timed by `just micro` |

The baseline `reports/micro/baseline.json` holds 68 benchmarks: mirror 37, protocol 21, simulator 6, daemon 3 and android 1.

### What was not measured

- **Inside the app's accessibility server, the Android agent and the emulator.** The app answers the reader's snapshot in its own process, and the agent and the emulator's guest are beyond the host's profiler; their time is the waits above.
- **The physical iPhone**, a non-goal of the change.
- **The reader's per-phase split.** Its runs give a read's split over the whole sequence; per command the reads are counted on the daemon's side.
- **The 2 ms of the server's call** that the sampling attributes to no frame.
- **Allocations on the emulator, of a click or of the server**: the Allocations runs covered the Simulator's listing alone, as D1 asks.

## Verdict

**Go on three fixes; the rest is the bridge's, the agent's or the replies' guarantees.** This code's CPU is under 3% of every command that reads or acts, so no CPU fix shortens a click or a fill; the profile names two waits of this code's own making and one CPU path that a listing pays in full.

The hot spots, ranked by the share of a model's turn they take at the daemon:

| # | Hot spot | Where | Share of the command at the daemon | A benchmark times it | Go | The fix, in a sentence |
|---:|---|---|---|---|---|---|
| 1 | The catch-up's wait for the re-read's due time | `daemon::worker::Attached::catch_up`, the mirror's minimum re-read interval; Simulator | 106 of 119 ms of a listing of an animating screen (0.65 ms still); 141 of 645 ms of a click back to back, 40 of 326 after a pause | no | **go** | When a command finds frames still changing and the next re-read not yet due, read at once instead of waiting for the due time, the background pump keeping its interval; gated on frames that kept changing past the interval, so a transition right after a tap is still read once it has run. |
| 2 | The bridge's tree read | the reader inside the Simulator | 54% of a click back to back, 85% after a pause, 26 to 30% of a fill | no; it needs a Simulator | no-go | The app's answer is the floor (D3); the reader's two foreground-owner queries, 25% of a read, could be one, which only a Simulator can measure: backlog. |
| 3 | The quiet windows and holds | the click's watch, the settle, the fill's hold, the Android read's 200 ms after a controller tap | 97% of the emulator's click, 93% of a settle, 41% of a fill | no | no-go | They are what the replies promise: the tap's effects seen, the text held, the screen at rest. |
| 4 | The server's call path | `crates/mcp`'s call through `cli::client::call` | 3.3 of 4.0 ms of a listing through the server, 2.4 of a delta | no | **go** | Check the daemon's version once and again only when a call cannot reach the daemon, call the socket without the hand-off to the blocking pool, and take the reply's text without a full JSON tree; S13's server shows 0.3 ms is reachable. |
| 5 | The listing's reply built through JSON values | `daemon::handlers::view::snapshot`, `daemon::wire::reply_line` | all of a listing's 0.73 ms is CPU: allocation 21 to 31%, the refs map 3,200 of 10,900 allocations, the reply's JSON 0.12 ms; 18% of a listing through the server, 9% through the CLI | partly: the reply's serialization, not its building | **go** | Write the refs map and the text straight into the reply line, as `mirror::json` writes the listing's strings, instead of building `serde_json` values to serialize. |
| 6 | The frame stream's downscale | `simulator::pipeline::scale_bgra` on the frames thread | none: on no command's path; 7 to 15% of a core while the screen changes, 73 to 74% of the daemon's CPU over the sequence | yes, `simulator/scale_bgra` (1.14 ms) | no-go under D3 | Hash a strided sample of the surface at the thumbnail's size instead of downscaling the whole frame: the backlog's first item, for CPU. |
| 7 | The CLI's start | process creation, exec, dyld | 8.3 ms per command, this code's `main` 0.4 of it | no | no-go | The floor is the host's process start and dyld (1.5 ms for a system binary, 2.3 here); a link with chained fixups could shave dyld's binds, about 0.3 ms, and the server avoids the start altogether. |
| 8 | Android's agent and controller | the agent's replies, `am start -W`, the controller's gRPC frames | the rest of the emulator's commands; 528 ms of a cold open in `am start -W` | the frame hash, yes | no-go | The agent's and the emulator's time; the daemon's CPU is 0.3% of a cold open. |

**D1, as built.** xctrace's Time Profiler, at 1 ms, at 0.1 ms and with waiting threads, and its Allocations template; samply was not needed. The `profiling` profile gave the frames their inlined functions and lines.

**D2, as built.** The sequences ran as planned, through a client sending the CLI's lines rather than `batch`, which times no command. The attribution separated waiting from CPU per thread and per handler stage, the bridge's read from the reader's CPU, and the reader's waits on the app from those on the owner queries. It could not see into the app's accessibility server, the Android agent or the emulator, nor attribute about 2 ms of the server's 3.3 per call.

**For the tasks:**

- **2.1, the catch-up.** A daemon benchmark on a fake target whose frames keep changing (the fake's Anim tab, or a frame source that changes every frame), timing `snapshot -i` through the socket: red first above 100 ms, against the 0.87 ms of `daemon/socket/snapshot-interactive-big`. The catch-up's own tests, and the transition tests behind it, guard the change; the target is a read's time and no wait.
- **2.2, the server.** A benchmark in `crates/mcp` timing a `tools/call snapshot` through the server's handler over a daemon on a fake target, red first above 1 ms, three times S13's 0.32; the fix's A/B also through the private client against the Simulator, where the 3.3 ms was measured.
- **2.3, the listing's reply.** `daemon/wire/reply-snapshot-big` extended, or a new benchmark beside it, to build the reply from the snapshot as the handler does, with its allocations: red first at the handler's 3,200 allocations over the mirror's count, the reply's bytes unchanged.
- **3.1, the suite.** The benchmarks above, an unchanged delta of the 500-cell grid (1.08 ms of CPU, more than the listing, which no benchmark times), and the frame hash end to end on a full-size frame, which `scale_bgra` and `thumbnail_hash` time apart.
- **The backlog**, with the numbers above: the frame stream's downscale (CPU), the reader's second owner query (25% of a read), SipHash in the replica and the ref table (3 to 12% of a command's CPU), the CLI's link (dyld's binds).
