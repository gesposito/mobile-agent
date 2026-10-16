# S13: the MCP server's round trip and result sizes

Spike S13 (task 2.1 of `mcp-server`) measures what an MCP stdio server built on `rmcp` adds to the daemon's socket path, against the CLI's process per command, and what its results cost a model, for design D11's gates: a round trip under 20 ms at p95 and the interactive listing under 7,000 tokens. It also checks that the server answers both protocol eras and that Claude Code and Codex on this host list and call its tools. The research note `docs/research/agent-surfaces.md` measured an rmcp probe's size and the daemon's snapshot replies, which the spike extends. The program, a throwaway server and a scripted client, is kept privately with the other spike programs (`docs/spikes/README.md`); every table below comes from its `results/`. Snapshot date: 2026-10-16.

**In brief.** Go for rmcp (D1); the listing misses the token gate as Claude Code counts it.

- **The server adds 0.3 ms.** An interactive `snapshot` of the fixture's 500-cell screen took 1.10 ms at p50 and 1.25 at p95 through the server, against 0.77 / 0.88 on the daemon's socket alone and 7.68 / 8.95 through the CLI. Paired call by call, the server's share was 0.29 to 0.33 ms at p50 and at most 0.46 at p95 in all four rounds: both protocol eras, a socket connection per request or one kept open.
- **Both eras, both clients.** One server answered `initialize` and `server/discover` with the same three tools and results. Claude Code 2.1.301 opened with `server/discover` in the 2026-07-28 revision; Codex 0.162.1 with `initialize` at 2025-06-18. Each listed the tools and called `snapshot`.
- **The listing is 12,163 tokens, not 6,290.** The 505-line listing is 22,014 characters, 6,290 tokens at the research note's 3.5 characters a token, under the gate. Claude Code's session counted 12,163 tokens for it: 1.81 characters a token, over the 7,000 gate. The CLI prints the same text, so the cost is the listing's, whichever surface carries it.
- **The text form pays.** Without the refs map the listing is half of the CLI's 45,414-byte `--json` reply; an unchanged delta is `unchanged (revision 5)`, 22 characters against the reply's 302 bytes; a click is one line of 46.
- **Clicks and screenshots.** A click that changes nothing waits out the daemon's 200 ms watch on every path, the server adding under 0.5 ms at p50. A screenshot's file, written by the daemon and read back by the server, cost the server 1.2 to 2.1 ms at p50, which D6's inline bytes remove.

D11 gets the verdict; D5's cost of an unchanged delta and of the full listing is restated with these numbers. D1 and D6 stand.

## Setup

**Host.** Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2 (25G83). The 1-minute load average stayed between 1.97 and 6.44 through the timed series, against the quiet-host limit of 10, the core count.

**Toolchain and crates.** Rust 1.98.1, the repository's toolchain, release profile without LTO, as the research note's probe was built. The spike's workspace has two crates and its own lockfile of 84 packages besides them, every one added through `sfw cargo add` and fetched through `sfw cargo fetch`; 22 of them are not in the repository's lockfile, the research note's count.

- `rmcp` 3.5.1, pinned with `=`, with `server`, `macros` and `transport-io` (which brings `transport-async-rw`) on top of the defaults `base64` and `schemars`; `rmcp-macros` 3.5.1.
- `schemars` 1.2.3, tokio 1.53.3 (`rt`, `macros`, `io-std`, `io-util`, `net`, `sync`, `time`; the current-thread runtime), serde 1.0.229, serde_json 1.0.154 and base64 0.23.1.
- The server's binary is 3,859,824 bytes; the research note's probe, three tools with no daemon behind them, was 3,429,776.

**The repository.** The worktree of `mcp-server` at `ff772f4` (the change's proposal on `main`), built with `cargo build --release -p daemon -p cli` in 1 min 18 s from a clean target directory: `mobile-agent` 1,641,712 bytes, `mobile-agent-daemon` 15,492,592 bytes with the Android agent embedded.

**Clients.** Claude Code 2.1.301 and Codex (`codex-cli`) 0.162.1, each run once with a configuration given on its command line, neither added to the client's own configuration.

**The daemon and the screen.** One daemon on a private socket directory under the temporary directory, `mobile-agent-daemon --socket-dir <dir> --fake android --drivers none --idle-timeout 2h`: the fake emulator `emulator-5554` alone, no device driver registered, no device or emulator touched. Before its calls, each server's session was brought to the fixture app's Big tab as `crates/daemon/benches/daemon.rs` brings it there, through the CLI: `open dev.mobileagent.fixture.expo` with `--force` (taking the claim from the session before), `click #tab-big`, `wait --settle`; the session's first interactive snapshot then issued its refs. The tab lists 505 elements: 500 cells and the five tab buttons.

**The server.** `s13-mcp-server`, one file of 513 lines over rmcp's `#[tool_router]` and `#[tool_handler]`, with the router kept in a field; it starts no daemon and mints its session at start, `mcp-<pid>`. Each tool call is one request on `daemon.sock`, the line the CLI sends for the same command, keys in the CLI's order and an id as the CLI makes them; `S13_CONNECTION` chooses a connection per request, as the CLI opens them, or one kept open and opened again when the daemon has closed it (it closes a connection idle for 60 s).

- `snapshot` (`interactive`, default true; `delta`, default false; `maxDepth`): the reply's listing as text, without the refs map; for a delta, the tree of a `full` one, `unchanged (revision N)` as the CLI prints an unchanged one, or the changes as JSON. A `warning` follows on a line of its own.
- `click` (`selector`): one line, `clicked @e7 at (204, 880) in 202 ms, no change`, or with `first frame change` and `first tree delta` when they came.
- `screenshot` (`ifChanged`, default false): asks the daemon for a JPEG at quality 80, long edge 1000 px, written to a file of the server's under the socket directory, reads the file back and returns it as image content in base64. With `ifChanged`, when the hash of the file's bytes equals the session's last, it returns `unchanged since the last screenshot` instead.
- A failure is an execution error (`isError`) whose text is the code, a colon and the daemon's message. `snapshot` and `screenshot` are annotated read-only. The instructions are one paragraph of 299 characters.
- For the client checks only, `S13_SESSION` names the session instead of `mcp-<pid>`, so the harness could open the Big tab in it before the client started the server, and `S13_WIRE_LOG` copies every line in and out to a file.

**The client.** `s13-client`, 1,425 lines of Rust in the same workspace. It starts the server as a child and speaks JSON-RPC over its stdin and stdout, one request in flight: in the legacy era `initialize` at 2025-11-25 and `notifications/initialized`; in the stateless era no `initialize`, `server/discover` first, then every request with `io.modelcontextprotocol/protocolVersion` 2026-07-28, `io.modelcontextprotocol/clientInfo` and `io.modelcontextprotocol/clientCapabilities` in its `_meta`, as the revision's changelog and schema give them. It runs the CLI as a process, and sends the daemon the CLI's lines itself.

**Method.** The timed series are four rounds, one per protocol era and server connection mode, each with a server and a session of its own. For each call type a round makes 10 warm-up iterations, which are dropped, then 100; each iteration makes the call once through each of four paths, in an order that rotates by one every iteration:

- the daemon's socket, a connection per request, the floor every path pays;
- the socket on a connection kept open;
- the round's MCP server, timed from the write of the request line to its stdin to the read of the complete result line from its stdout;
- the CLI, one process per command (`snapshot -i --json`, `snapshot -i --delta --json`, `click @e7 --json`, `screenshot <path> --quality 80 --max-dimension 1000 --json`), timed from its spawn to its exit, its output read.

The paths share the round's session, so its claim, refs and delta baseline. A socket call is timed from the connect, or the write on the kept connection, to the read of the reply line. Each result is checked outside its span (505 lines; `unchanged`; a click that changed nothing; image content; the unchanged text), and none of the 6,800 timed calls of either run failed. "MCP over the socket, paired" is, per iteration, the MCP call's time less that of the socket call on the server's own connection mode, and the percentiles of those 100 differences. Before each round and each call type the client required a 1-minute load under 10, checked every 60 s for up to 40 minutes; every first check passed. Percentiles are nearest-rank, as the earlier spikes compute them.

- **The click** goes to the Big tab's own button, already selected, which the fake leaves unchanged, so every path waits out the daemon's 200 ms watch for an effect (`crates/cli/README.md`, Timing spans).
- **The screenshot** on the fake is a 447x1000 frame whose JPEG is the fake's placeholder text (`jpeg 447x1000 q80 frame N`, 26 or 27 bytes; `crates/daemon/README.md`, The fake driver). Its bytes say nothing of a real picture's; its token cost follows from its size.
- **Tokens** are estimated by the research note's method: characters divided by 3.5, Claude's ratio for models before 4.7, with the note's 30% more for later models, and ⌈w/28⌉ × ⌈h/28⌉ for an image. The listing was also counted by Claude Code: Claude Code records each API response's usage in the session's transcript, and the input grew from the request that called `snapshot` to the one that carried its result by that result plus the turn between.
- **Sizes** are bytes and characters of the MCP text content, the JSON-RPC result line, and the CLI's `--json` and human outputs for the same calls, in a fresh session.

The final run, reported here, ran `s13-client all` from 13:58 to 14:05 with the server as kept; the client's `sizes` later gained a copy of its listing to a file and ran once more for it. A first run from 13:46 to 13:52, with the server built before `S13_SESSION` was added, gave p50s within 0.12 ms of these for the MCP and socket paths and within 1.1 ms for the CLI.

**Rerun**, with a daemon on a private socket directory `$DIR`, fake target only, and the repository built:

```sh
cd spikes/s13-mcp-server                          # in the private copy
cargo build --release
mobile-agent-daemon --socket-dir "$DIR" --fake android --drivers none --idle-timeout 2h &
export MOBILE_AGENT_SOCKET_DIR="$DIR" S13_CLI=<repo>/target/release/mobile-agent
target/release/s13-client all     # eras, checks, sizes, bench, startup, then results/tables.md
```

Each command also runs alone. `S13_ROUNDS` picks rounds, and `S13_SAMPLES` and `S13_WARMUP` change the counts. The client checks are in the program's `notes.md`, with their command lines.

## Measurements

### Round trips

The stateless era with a connection per request, the configuration Claude Code meets and the CLI's way of connecting; ms, p50 / p95, 100 calls each after 10 warm-up, 1-minute load 2.08 to 3.15:

| Call | Socket, a connection per call | Socket, kept | MCP | MCP over the socket, paired | CLI |
|---|---:|---:|---:|---:|---:|
| `snapshot` interactive, 505 lines | 0.77 / 0.88 | 0.70 / 0.81 | 1.10 / 1.25 | 0.33 / 0.46 | 7.68 / 8.95 |
| `snapshot` delta, unchanged | 0.88 / 0.96 | 0.84 / 0.93 | 0.99 / 1.13 | 0.11 / 0.26 | 7.35 / 8.94 |
| `click` on the selected tab | 202.85 / 206.42 | 202.90 / 206.24 | 203.80 / 207.43 | 0.45 / 5.28 | 210.84 / 223.33 |
| `screenshot` | 0.91 / 4.21 | 0.61 / 4.91 | 2.55 / 5.06 | 1.18 / 4.23 | 5.98 / 11.12 |
| `screenshot` with `ifChanged`, unchanged | - | - | 2.79 / 8.00 | - | - |

The MCP server in each round, ms, p50 / p95, its paired share over the socket in parentheses:

| Call | Stateless, a connection per call | Stateless, kept | Legacy, a connection per call | Legacy, kept |
|---|---:|---:|---:|---:|
| `snapshot` interactive, 505 lines | 1.10 / 1.25 (0.33 / 0.46) | 0.98 / 1.19 (0.30 / 0.42) | 1.00 / 1.21 (0.29 / 0.43) | 1.01 / 1.16 (0.32 / 0.41) |
| `snapshot` delta, unchanged | 0.99 / 1.13 (0.11 / 0.26) | 0.88 / 0.96 (0.07 / 0.12) | 1.14 / 1.80 (0.13 / 0.48) | 0.93 / 1.06 (0.09 / 0.19) |
| `click` on the selected tab | 203.80 / 207.43 (0.45 / 5.28) | 203.41 / 206.95 (-0.01 / 4.16) | 203.80 / 206.24 (0.25 / 3.72) | 202.88 / 205.91 (0.29 / 2.95) |
| `screenshot` | 2.55 / 5.06 (1.18 / 4.23) | 3.92 / 10.88 (2.12 / 7.43) | 3.02 / 7.96 (1.82 / 5.75) | 2.62 / 5.41 (1.43 / 4.18) |
| `screenshot` with `ifChanged`, unchanged | 2.79 / 8.00 | 2.15 / 3.87 | 3.29 / 6.29 | 2.84 / 5.99 |

- **The listing.** The daemon answers a warm interactive snapshot of the Big tab from its replica in about 0.7 ms through the socket, as the microbenchmark `daemon/socket/snapshot-interactive-big` has it (0.77 ms in `reports/micro/baseline.json`). The server parses the 45 KB reply, drops the refs map and writes a result line of about 24 KB: 0.29 to 0.33 ms more at p50, at most 0.46 at p95, in every round. Neither the era nor the connection mode moved it: the kept connection saves the connect, 0.04 to 0.07 ms on the socket, which the server's share does not show.
- **The CLI.** 6.5 to 7.7 ms at p50 for a snapshot across the rounds and 7.4 to 9.0 at p95: the process start, the version check and a connection, as the CLI's own overhead measurement gives for `status` (`crates/cli/README.md`, Process overhead). A CLI click took 8 to 14 ms more than the socket's at p50, against 5.8 to 6.9 for a snapshot; the spike did not find why.
- **Clicks.** Every path waits out the 200 ms watch: 202.6 to 203.4 ms on the socket at p50 across the rounds, 202.9 to 203.8 through the server. The server's share at p95, 3 to 5 ms, is the spread of that watch between two calls rather than work of the server's, whose p50 share is under 0.5 ms.
- **Screenshots.** The daemon's own call varied with the file it writes, 0.6 to 2.0 ms at p50 and 3.3 to 12.4 at p95 across the rounds; the server's share, its read of the file and the encoding of 27 bytes, was 1.2 to 2.1 ms at p50 and up to 7.4 at p95. `ifChanged` on a still screen does the same work and returns 35 characters instead.
- **Load.** The final run's rounds ran from 13:58:23 to 14:05:00 at a 1-minute load of 2.08 to 6.44, the first run's at 1.97 to 4.07.

### Server start

Spawn to the answer to the era's opening, and spawn to exit once stdin closes; ms, p50 / p95, 30 starts per era after 3, 1-minute load 4.54:

| Era | Opening | To the answer | To the exit |
|---|---|---:|---:|
| legacy | `initialize` | 3.64 / 4.46 | 16.10 / 17.00 |
| stateless | `server/discover` | 5.02 / 10.90 | 17.55 / 23.53 |

- A server whose stdin closed after `server/discover` alone exits with status 1: rmcp's serve loop is still waiting for a request that chooses the lifecycle and reports "connection closed: initialize request". A client goes on with requests, as Claude Code did.
- Before and during the first run, the first start of the server by a freshly built client took 363, 223 and 536 ms; every other start took 3.2 to 15.2 ms. The cause was not looked into.

### Result sizes

For a listing of 22,014 bytes, the length of the one Claude Code counted below, whose cells had the refs `e527` to `e1026` and its tabs `e5` to `e9`; bytes:

| Result | MCP text content | JSON-RPC result line | CLI `--json` | CLI, human output |
|---|---:|---:|---:|---:|
| `snapshot` interactive, 505 lines | 22,014 | 23,642 | 45,414, of which the refs map 21,607 | 22,015 |
| `snapshot` delta, unchanged | 22: `unchanged (revision 5)` | 136 | 302 | 23 |
| `click` on the selected tab | 46: `clicked @e7 at (204, 880) in 204 ms, no change` | 160 | 142 | 48 |
| `screenshot` | 36 characters of base64: the fake's 26-byte placeholder, 447x1000 | 175 | 194, the path and the picture's size | 72 |
| `screenshot` with `ifChanged`, unchanged | 35 | 149 | - | - |

- The MCP texts are ASCII, so their characters equal their bytes. The CLI's human output is the MCP text plus a newline for a snapshot, and its own line for a click (`✓ Done (dispatch 0.3 ms, no change in 202 ms)`, the ✓ three bytes).
- The listing's length follows its refs' digits: the same screen listed 21,897 bytes with the cells at `e10` to `e509`, and 22,487 with them at `e1044` to `e1543` in the final run's size sessions, whose CLI reply was 46,361 bytes, its refs map 22,080. An unchanged delta grows with the replica's revision, `unchanged (revision 3652)` being 25 characters and the CLI's reply 309 bytes.
- The JSON-RPC line adds 7% to the listing: its quotes and newlines escaped, and the envelope.

### Tokens

| Result | Characters | At 3.5 characters a token | With 30% more (Claude 4.7 and later) | Counted by Claude Code |
|---|---:|---:|---:|---:|
| `snapshot` interactive, 505 lines | 22,014 | 6,290 | 8,177 | 12,163 |
| CLI `snapshot -i --json` | 45,414 | 12,975 | 16,868 | - |
| `snapshot` delta, unchanged | 22 | 6 | 8 | - |
| `click` line | 46 | 13 | 17 | - |
| `screenshot` with `ifChanged`, unchanged | 35 | 10 | 13 | - |
| `screenshot`, 447x1000 | - | 576 image tokens, ⌈447/28⌉ × ⌈1000/28⌉ = 16 × 36 | - | - |

- **The count.** In the Claude Code check below, the request that carried the `snapshot` result held 40,743 input tokens and the one before it 28,498; the 12,245 between them hold the result, the 82 output tokens of the turn that called the tool and a per-turn reminder of a few tokens that Claude Code adds, which leaves 12,163 at most for the 22,014-character listing, on the model Claude Code 2.1.301 uses by default on this host: 1.81 characters a token. Claude Code passed the text unchanged, its transcript holding the 22,014 characters.
- **Why.** A line such as `- button "Cell 123" [id=cell-123, ref=e650]` is punctuation, short words and digits, about 24 tokens a line at that ratio; the research note's 3.5 characters is Claude's ratio for prose, and its 30% correction gives 2.7.
- The research note's 612 image tokens were for a 460x1000 screenshot; the fake emulator's is 447x1000.

### Protocol eras

The first exchange of each era with a fresh server (`results/era-legacy.jsonl` and `era-stateless.jsonl`, the lines as they went over the pipes):

| Era | Opening | The server's answer | `tools/list` | `tools/call` `snapshot` |
|---|---|---|---|---|
| legacy | `initialize`, protocol 2025-11-25, then `notifications/initialized` | protocol 2025-11-25, the `tools` capability, the instructions | `click`, `screenshot`, `snapshot`, sorted by name | the 505-line listing |
| 2026-07-28 | `server/discover`, then every request with its `_meta` | `supportedVersions` 2024-11-05, 2025-03-26, 2025-06-18, 2025-11-25 and 2026-07-28, the capability, the instructions, `ttlMs` 0, `cacheScope: private` and `resultType: complete` | the same three, with `ttlMs` 0 and `cacheScope: public` | the same listing, with `resultType: complete` |

The server's code has nothing per era: rmcp's serve loop takes the lifecycle from the client's first request other than `server/discover`.

### Claude Code and Codex

| Client | Command | First message on the server's stdin | Era | Listed | Called | Run |
|---|---|---|---|---|---|---|
| Claude Code 2.1.301 | `claude -p "List the tools of the MCP server s13 with their input schemas, then call its snapshot tool with interactive true and report the first three lines of the result." --mcp-config <file> --strict-mcp-config --max-turns 4 --output-format json --allowedTools mcp__s13__snapshot` | `server/discover`, `_meta` protocol 2026-07-28 | 2026-07-28 | the three tools with their input schemas, which its tool search loaded in a turn of its own | `snapshot` with `interactive: true`; it reported the first three lines | 3 turns, 20 s, no permission denial |
| Codex 0.162.1 | `codex exec --skip-git-repo-check --ephemeral --ignore-user-config -s read-only --json -c 'mcp_servers.s13.command="<server>"' -c 'mcp_servers.s13.env={...}' "List the s13 server's tools and call snapshot once; report the first three lines."` | `initialize`, protocol 2025-06-18 | 2025-06-18 | the three names | `snapshot` with `{}`, so `interactive` took its default, true; it reported the first three lines | 1 turn, 15 s |

- Claude Code ran inside the agent session that ran the spike without refusing. Print mode denies an MCP tool that is not allowed, so `--allowedTools` names the one tool the prompt calls.
- Codex sends `initialize` at 2025-06-18, which rmcp answers in kind, and `_meta` with a progress token on `tools/list`. `--ignore-user-config` kept the user's other servers out of the run, and the server's socket connection worked under the read-only sandbox.
- Each check ran its server in a session the harness had opened the Big tab in (`S13_SESSION`).

### What the tools return

Checked through a server whose wire was logged (`results/checks.json`), all 11 passed:

- `snapshot` returns the 505-line listing alone, with no refs map; a session's first delta is the whole listing; the next, on the still screen, `unchanged (revision N)`; after a move to the List tab the delta falls back to the whole new listing, as the mirror does past 70% of its size.
- `click` on the selected tab returns one line ending `no change`; on another tab, the line with `first frame change 0.1 ms, first tree delta 0.5 ms`; on an unknown ref, an execution error whose text starts `unknown_ref: ` with the daemon's message.
- A click on a row mostly out of view, as the CLI's test `a_click_on_an_element_mostly_out_of_view_warns` makes one, returns its line and, on the next line, `warning: #row-20 was not mostly in view ... scroll --until #row-20 brings it into view first`.
- `screenshot` returns image content, `image/jpeg`, whose base64 decodes to the file the daemon wrote; `ifChanged` on the still screen returns the unchanged text, and after the tab changed, the image.

### What was not measured

- **A real screenshot.** The fake's JPEG is placeholder text, so neither the base64 payload of a picture, its encoding time, nor how a client scales it.
- **Tokens beyond the listing,** on Claude Code's model: only the listing's count came out of the check's transcript.
- **Progress, cancellation, waits and `structuredContent`** (D5, D7): not among the spike's three tools.
- **Claude Desktop and Cursor.**
- **Real targets.** The fake answers from its replica; a Simulator's or a phone's reads add their time to every path alike.
- **The CLI's extra on a click**, 8 to 14 ms against 5.8 to 6.9 on a snapshot: not explained.

## Verdict

**Go for rmcp, D1's SDK; the listing gate is missed as Claude Code counts it, and the miss is the listing's, not the SDK's.**

| Criterion (D11) | Measured | Verdict |
|---|---|---|
| A round trip under 20 ms at p95: interactive `snapshot` | 1.16 to 1.25 ms in the four rounds (1.11 to 1.56 in the first run) | pass |
| the same, an unchanged delta | 0.96 to 1.80 ms (0.90 to 2.21) | pass |
| the same, `screenshot` | 5.06 to 10.88 ms (5.25 to 7.56) | pass |
| the same, `screenshot` with `ifChanged` | 3.87 to 8.00 ms (5.36 to 13.53) | pass |
| the same, `click` on the selected tab | 205.91 to 207.43 ms, the daemon's 200 ms watch included, which the socket path pays too (205.90 to 206.43); the server's share 2.95 to 5.28 ms at p95 | pass on the server's share |
| The interactive listing under 7,000 tokens, at 3.5 characters a token | 6,290 | pass |
| The same, as Claude Code counted it | 12,163 | fail |
| Both eras answered | `initialize` at 2025-11-25 and `server/discover` with per-request metadata at 2026-07-28, the same tools and results | pass |
| Claude Code lists and calls the tools | in the 2026-07-28 revision | pass |
| Codex lists and calls the tools | at 2025-06-18 | pass |

**D1, confirmed.** rmcp 3.5.1 adds about 0.3 ms to a 22 KB listing, under 0.5 ms at p50 to a delta or a click and 1.2 to 2.1 ms to a screenshot whose file it reads back, where the CLI adds 6 to 8 ms; it served both clients in their own eras with no code per era: the legacy handshake for Codex, the stateless revision for Claude Code. The server starts in 3.6 to 5.0 ms at p50 and is 3.86 MB, 22 crates beyond the workspace's lockfile, as the research note measured. The no-go fallback, a hand-written loop, does not apply.

**D5, restated with the numbers.** The text form holds: the listing without its map is 48% of the CLI's JSON reply, an unchanged delta 22 to 25 characters, a click one line. What the spike changes is the cost: the listing of the 500-cell screen is 12,163 tokens on Claude Code's model, 1.81 characters a token, about twice the research note's estimate, and the CLI's human output is the same text, so the shell path pays it too. D11's 7,000 tokens assumed the estimate. Delta snapshots by default after a session's first are what keep later snapshots small. A terser listing line, or a gate restated per listed element (about 24 tokens each here), is the maintainer's decision; the spike records the miss in D11 and D5 and changes neither the tools nor the format.

**D6, confirmed.** Image content travels through rmcp as the specification gives it, and `ifChanged` answers a still screen from a hash of the bytes. Reading the daemon's file back cost the server 1.2 to 2.1 ms at p50 and up to 7.4 at p95 for 27 bytes, and the daemon's own write varied its screenshot call from 0.6 to 2.0 ms at p50; `inline: true` (task 4.3) removes both.

**For the tasks:**

- **3.1 and 3.2, the shared types.** schemars 1.2.3, through rmcp, gave flat 2020-12 schemas from serde types: `#[serde(default = ...)]` became `default`, `Option<u32>` became `"type": ["integer", "null"]` with `"default": null`, doc comments became descriptions, and Claude Code took them as they were. Codex called `snapshot` with `{}`, so a tool's defaults belong in the types.
- **4.3, `screenshot` with `inline: true`.** Justified by the read-back's cost above.
- **5.1, the server.** Pin rmcp 3.5.1 and give `#[tool_handler]` the router kept in a field (`router = self.tool_router`), as the spike did: its default, `Self::tool_router()`, builds the router and every tool's definition again on each call, the schemas coming from a per-thread cache; the spike did not time the default. A connection per request is enough, a kept one saving nothing the server's share shows, while the daemon closes an idle one after 60 s; the CLI's `client.rs` serves as it is. rmcp lists tools sorted by name, the deterministic order the revision asks for.
- **5.2, sessions and exit.** A server exits 16 to 24 ms after stdin closes; one that got only `server/discover` exits through rmcp's initialization error with status 1, so releasing the claim on exit has to run on that path as well.
- **5.5, instructions.** The spike's 299-character card reached both clients, in Codex's `initialize` result and Claude Code's `server/discover` result.
- **6.2, the acceptance.** Claude Code in print mode needs `--allowedTools mcp__<server>__<tool>` for a call to run, and its tool search loads the schemas in a turn of its own before the first call.
