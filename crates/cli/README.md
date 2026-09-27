# cli

`mobile-agent`, the command line of spec `agent-cli` (design D1 and D9): a
thin client of the daemon in `crates/daemon` that speaks agent-browser's
grammar, ref syntax and JSON envelope. It links nothing of the daemon; each
command is one JSON request over the daemon's Unix socket, and the first
command starts the daemon.

## Process overhead

The spec keeps CLI process overhead, from process start to the daemon
request, under 20 ms. The measurement times whole `status` runs against a
running daemon on a fake target, the daemon's reply included, so it bounds
that overhead from above:

```sh
cargo build --release -p cli -p daemon
export MOBILE_AGENT_SOCKET_DIR="$(mktemp -d)" MOBILE_AGENT_FAKE_TARGETS=android
target/release/mobile-agent status > /dev/null    # starts the daemon
hyperfine --warmup 5 --runs 300 -N --export-json overhead.json 'target/release/mobile-agent status'
target/release/mobile-agent shutdown
```

On an Apple M1 Max (10 cores) under macOS 26.6.2, with a load average of 5.6 from
parallel builds:

```
Benchmark 1: target/release/mobile-agent status
  Time (mean ± σ):       7.3 ms ±   1.4 ms    [User: 1.7 ms, System: 1.4 ms]
  Range (min … max):     5.2 ms …  14.1 ms    300 runs
```

The exported samples give a median of 6.88 ms and a p95 of 10.00 ms. On the
same host `/usr/bin/true` took 4.6 ms ± 4.2 ms and `mobile-agent --version`,
which parses the command line and contacts no daemon, 5.3 ms ± 1.8 ms: process
creation is most of the cost.

The test `overhead::status_overhead_against_a_running_daemon_is_under_20_ms`
is the gate: it starts the daemon with a first `status`, then requires the
median of 30 `status` runs of the debug build to stay under 20 ms, measured
with hyperfine when it is on `PATH` and timed by the test otherwise.
