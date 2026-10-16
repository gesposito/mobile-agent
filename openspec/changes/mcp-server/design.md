# Design

## Context

`docs/research/agent-surfaces.md` holds the evidence: the MCP revision of 2026-07-28 is stateless (no protocol sessions, handles as arguments, logging deprecated) and older clients still speak the earlier era; Claude Code cuts instructions at 4,096 characters, backgrounds a call past 2 minutes and aborts a silent one after 30 unless progress arrives, Codex gives a call 60 s and shows only `structuredContent` when a result has it, Cursor keeps tool names alone; `rmcp` 3.5.1 (official, Tier 1, dual-era, progress and cancellation) costs about 2.9 MB and 22 crates against a 0.54 MB hand-written loop; the daemon's 18 actions read 42 parameters by name and build replies with `json!`, so no schema can be derived today; on the 500-cell fake screen `snapshot -i --json` is 45 KB of which the refs map repeats the 22 KB listing, an unchanged delta is 302 bytes, a default screenshot about 612 image tokens; a target runs its commands in order, the daemon has no cancel, and only `close` releases a claim, stopping the app. The maintainer's decisions: standalone (no agent-browser provider), the CDP subset not built, the unverified mode its own later change.

## Goals / Non-Goals

**Goals:**
- An MCP server that gives a model what the CLI gives a shell: the same refs, replies, codes, sessions and verification, in a context-sized form.
- One definition of the daemon's requests and replies, with schemas, serving the CLI, the harness and the server.
- Waits that neither hold a target nor fall to a client's timeout.
- The CLI's start time and the harness's gates untouched.

**Non-Goals:**
- The agent-browser `device` provider and the CDP subset (recorded below).
- The unverified mode (its own change).
- Streamable HTTP transport, remote servers, authentication: stdio only, one client per process.
- Pushing changes to the client (MCP notifications do not reach the model portably); the blocking `wait --change` is the form.

## Decisions

### D1. The SDK: `rmcp`, measured against a hand-written loop
`rmcp` 3.5.1 serves both protocol eras, generates input schemas from Rust types through schemars, and carries progress and cancellation; a hand-written loop (agent-browser's and agent-device's way) is 0.54 MB against rmcp's 3.43 MB in the research note's measurement and 22 fewer crates, but both eras, discovery, progress and cancellation would be written and followed by hand as the specification moves (three breaking SDK majors in 2026 mirror a moving specification). The server takes `rmcp`, pinned, in its own binary so the cost is the server's alone; the spike S13 measures the round trip the SDK adds over the socket path. Revisited if a major migration costs more than the loop would have.

### D2. Where it runs: a second binary in `crates/cli`
`mobile-agent-mcp`, a `[[bin]]` of the CLI crate sharing `client.rs` (discovery, start, version check, retries, the reply wait) and the request building; `rmcp` and `tokio` are dependencies of that target alone, so `mobile-agent` keeps its 1.64 MB and its 5 ms start, which the harness gates (`cli-start` under 20 ms). Alternative, a `mobile-agent mcp` subcommand (rejected: the CLI would link the SDK's runtime, a start-time risk the gate would have to re-measure).

### D3. The shared types: `crates/wire`
A crate of the daemon's request and reply types (`serde` and `schemars` derives, one type per action for the request and one for the reply's data, flat fields, the envelope), with a JSON Schema per request generated at build and tested against the handlers: the daemon's handlers deserialize the typed request instead of reading 42 parameters by name, the CLI's `request.rs` builds the typed request, the harness reads replies through the types, and the MCP server's tool inputs are the request types' schemas (flat, since Claude rejects `anyOf` at a schema's root). The handlers' messages and codes do not change; a round-trip test per action (the CLI's command to the daemon's parse to the reply type) is written first. Alternative: schemas written beside the tools (rejected: three copies drift, the research note's finding on agent-browser's 62.7 KB of hand-written definitions).

### D4. Names and arguments
Tool stems follow agent-browser's where the concept matches (`snapshot`, `click`, `fill`, `type`, `press`, `swipe`, `scroll`, `screenshot`, `wait`, `get`, `open`, `close`), with the daemon's own for the rest (`devices`, `status`, `doctor`, `diff_snapshot`, `help`); no prefix, since Claude Code prefixes the server's name. The element argument is `selector` as agent-browser names it (a ref `@e3`, an id `#login`, or `text=`), the other arguments the daemon's names (`interactive`, `maxDepth`, `delta`, `text`, `key`, `direction`, `amount`, `until`, `timeout`); the mapping is one table in the server and the README. Tool annotations mark `snapshot`, `diff_snapshot`, `get`, `wait`, `screenshot`, `devices`, `status`, `doctor` and `help` read-only, `close` and `shutdown` destructive.

### D5. Results: text, no map, `structuredContent` opt-in
A result's text is the reply as the CLI prints it without `--json`: the listing for snapshots (without the refs map, which repeats it and doubles the size), one line of the reply's data for actions (`clicked @e3 at (201, 139) in 343 ms, first frame change 94 ms`), the `warning` on its own line, and a failure as `isError` text naming the code and the daemon's message, which already says what to do next. `structuredContent` with an `outputSchema` is sent only with `--structured`, since Codex then shows the model that JSON alone and drops images (research note, How the clients consume servers). Snapshots default to `delta: true` after the first of a session, so an unchanged screen costs `unchanged (revision N)`, 22 to 25 characters against the CLI's 302-byte JSON reply (S13); `full: true` asks for the whole listing, which S13 counted at 12,163 tokens on Claude Code's model for the 500-cell screen, 1.81 characters a token, about twice the research note's estimate.

### D6. Screenshots inline with `ifChanged`
The screenshot tool asks the daemon for the JPEG (quality 80, long edge 1000 px, within Claude's 2000 px bound for many-image requests) and returns it as image content; with `ifChanged`, the server compares the frame hash the daemon reports with the session's last and returns a text "unchanged since the last screenshot" instead, sparing the 612 tokens; `path` returns the file's path alone for clients that cannot take images. The daemon's `screenshot` gains the bytes in its reply for a server that asks (`inline: true`), so no file is read back.

### D7. Long waits: progress, a bound, a cancel
Every tool that can wait (`wait`, `scroll --until`, `open`, `fill`, `click` with its settle) sends a progress notification every 5 s with what it waits for, which keeps Claude Code's idle timer alive and shows in its task entry; each call is bounded by `--call-timeout`, 55 s by default (under Codex's 60 s), passed to the daemon as the command's timeout so the daemon's own wait ends first; and an MCP cancellation sends the daemon the new `cancel` action, which ends the session's command in flight (it fails with `cancelled`), so a target is not held by a wait the client abandoned. The daemon runs a target's commands in order (`crates/daemon/src/worker.rs`), so `cancel` reaches the worker through a flag the waiting command checks at its next wakeup; it does not interrupt a device call in flight.

### D8. `wait --change`
A `wait` mode that returns when the replica's revision or the frames of the target move since the session's last snapshot, with the delta (as `snapshot --delta` gives it) and what moved (`tree`, `frames`), or `wait_timeout` at its bound; built on the mirror's `run_until`, `next_wakeup` and revision as the overview's section 7 assessed. It is the portable form of a change notification: the model calls it after an action whose effect may still be arriving, instead of polling snapshots. The CLI gets the same flag.

### D9. Sessions and claims
The server mints `mcp-<pid>` as its session and takes an optional `session` argument on every tool for a client that resumes a transcript with refs; on exit (stdin closed, SIGTERM) it sends `close --keep`, the new form that releases the claim without stopping the app, so the next session does not meet `target_claimed` and the app a model was driving stays where it was; `close` alone keeps today's meaning. The daemon's idle exit and `shutdown` are unchanged.

### D10. Profiles and instructions
`--tools read-only|all` (default `all`): the read-only set is `devices`, `status`, `doctor`, `snapshot`, `diff_snapshot`, `get`, `wait`, `screenshot`, `help`, which also passes Codex's write approvals without prompts; the instructions card is one paragraph under 512 characters that stands alone (what the server drives, that refs come from snapshots, that replies verify, where to read more), the whole under 4,096 characters, and the agent guide (`help agent`, 119 lines) is the `help` tool's text, as agent-device splits them.

### D11. The spike before the build
S13 (`docs/spikes/s13-mcp-server.md`, `spikes/` are kept privately, the report public): a throwaway `rmcp` stdio server with three tools (`snapshot`, `click`, `screenshot`) over the daemon's socket against the fake 500-cell target, measured on a quiet host: the tool call's round trip at p50 and p95 against the CLI's process per command (5 ms start plus the socket) over 100 calls; the result sizes in characters and Claude's token ratio for the interactive listing, an unchanged delta and a screenshot; a functional check that Claude Code on this host and Codex, if installed, list and call the three tools in their era. Go: a round trip under 20 ms at p95 and the listing under 7,000 tokens; no-go on the SDK falls back to the hand-written loop with the same tools. S13 found the round trip under the gate in every round, the server adding 0.3 ms to the 500-cell listing (1.16 to 1.25 ms at p95, against 7.4 to 9.0 through the CLI), and both eras served to Claude Code (2026-07-28) and Codex (2025-06-18), so `rmcp` stays. The listing misses the gate as Claude Code counts it, 12,163 tokens at 1.81 characters a token against the 6,290 the research note's 3.5 gave, a cost of the daemon's listing that no SDK changes; a terser line or a gate per listed element is the maintainer's call (`docs/spikes/s13-mcp-server.md`).

### D12. Recorded, not built
The agent-browser `device` provider: its plugin policy wants providers out of process returning CDP endpoints, so a provider needs either a CDP endpoint on this side or a new backend in agent-browser's actions; kept as an option for when agent-browser opens a non-CDP backend. The CDP subset: agent-browser's fill and focus run JavaScript on DOM nodes and Metro's inspector expects a JavaScript runtime; low value for the cost, a viewer at most. The unverified mode: its own change, after this one, with the server's tools gaining the flag then.

## Risks / Trade-offs

- [`rmcp` breaks on a major] → pinned, bumped on purpose by Dependabot's cooldown, the hand-written loop the recorded fallback.
- [Clients change what reaches the model] → the research note's limits are dated; the acceptance checks two clients, and the card and sizes are tuned to the smallest.
- [The shared types change how the daemon reads requests] → round-trip tests per action first, the messages and codes unchanged, the harness's and CLI's tests as the guard.
- [A cancel that cannot interrupt a device call leaves the target busy for that call's length] → bounded by the device's own deadline (10 s on the runner); recorded in the reply.
