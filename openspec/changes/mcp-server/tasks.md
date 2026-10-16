# Tasks

Rules that bind every task below (the archived foundation design's Delivery process): research first; spikes with go/no-go before building on an unproven assumption; measured decisions recorded in design.md; fail-first tests observed red; atomic commits without trailers; the validation-and-fix phase before the next task; installs through `sfw`; the quiet-host rule for numbers; every daemon of a task on a private socket directory.

## 1. Research

- [x] 1.1 Write `docs/research/agent-surfaces.md` (done on 2026-10-16: MCP's 2026-07-28 revision and the clients' limits, `rmcp` against a hand-written loop measured at 3.43 against 0.54 MB, the four mobile MCP servers and their users' costs, agent-browser's server and plugin policy, the CDP subset's value against cost, the measured design inputs on the fake 500-cell screen; the provider route kept as a documented option by the maintainer's decision to stay standalone)

## 2. Spike

- [x] 2.1 S13, the server's round trip and sizes (D11): a throwaway `rmcp` stdio server with `snapshot`, `click` and `screenshot` over the daemon's socket against the fake 500-cell target, 100 calls each on a quiet host against the CLI's process per command; the result sizes for the interactive listing, an unchanged delta and a screenshot; Claude Code on this host (and Codex if installed) listing and calling the tools; the report `docs/spikes/s13-mcp-server.md` with its row in `docs/spikes/README.md`, the programs kept privately; go/no-go as D11 states, recorded in the design (done on 2026-10-16: `snapshot -i` of the 500-cell screen at 1.10 / 1.25 ms p50 / p95 through the server against 0.77 / 0.88 on the socket and 7.68 / 8.95 through the CLI, the server adding 0.3 ms in both eras and either connection mode; both eras served, Claude Code 2.1.301 opening at 2026-07-28 and Codex 0.162.1 at 2025-06-18, both listing and calling the tools; the listing 22,014 characters, 12,163 tokens as Claude Code counted it against 6,290 at 3.5 characters a token, an unchanged delta 22 characters, a fake screenshot 576 image tokens; go on the round trip and `rmcp`, the listing gate missed, recorded in D5 and D11)

## 3. The shared types

- [ ] 3.1 `crates/wire`: the envelope, one flat parameter struct per action with serde derives and its defaults, a reply-data struct per action, `schemars` behind the `schema` feature; failing tests first: each CLI command's line parses into its action's struct and serializes back to the same JSON (the cases of the CLI's `request.rs`), each handler's reply on the daemon's fake parses into its reply-data struct with unknown fields denied, and, with the feature, each request schema is a flat object whose properties are typed and described with no `anyOf` at its root; then implement; verify red-then-green
- [ ] 3.2 The daemon's handlers read the typed requests (`crates/daemon/src/handlers/{mod,act,view}.rs`, `server.rs` for `shutdown`), the CLI's `request.rs` builds them, the harness's `adapter.rs` reads `readMs` and `servedFromReplica` through the reply types; failing tests that no message, code or reply field changes (the CLI's and daemon's existing tests as the guard, with the round-trip tests of 3.1); then implement; verify `cargo test --workspace` green, the CLI's binary size and the `overhead` tests unchanged within noise, and the micro benchmarks `daemon/wire/*` within the threshold of `reports/micro/baseline.json` on a quiet host (`just micro`)

## 4. The daemon's additions

- [ ] 4.1 `wait --change` (D8): failing tests through the daemon's fake that it returns at the first tree or frame change with the delta and what moved, and `wait_timeout` on a still screen; then implement on the mirror's `run_until` and revision; verify red-then-green and on this session's own Simulator a `click` followed by `wait --change` returning with the row's change
- [ ] 4.2 `close --keep` and `cancel` (D7, D9): failing tests that `--keep` releases the claim with the app running and another session attaches, that `cancel` ends a session's waiting command with `cancelled` and the next command runs at once, and that a command past its device call is not interrupted; then implement; verify red-then-green
- [ ] 4.3 `screenshot` with `inline: true` returning the bytes (D6): failing test; then implement; verify red-then-green

## 5. The server

- [ ] 5.1 `mobile-agent-mcp` (D1, D2, D4): the binary with `rmcp` pinned, the tool table from the `wire` schemas under agent-browser's stems with the `selector` argument and the annotations, each call one request in the server's session; failing tests through a scripted MCP client over stdio (both eras) that the tools list with their schemas and that `snapshot`, `click`, `fill`, `get` and `devices` answer as D5 states (listing without the map, data in one line, `warning` kept, failures as execution errors with the code); then implement; verify red-then-green
- [ ] 5.2 Sessions and exit (D9): failing tests that a server mints its session, takes `session` on a tool, and releases its claim with `close --keep` on stdin's end and on SIGTERM; then implement; verify red-then-green
- [ ] 5.3 Screenshots (D6): failing tests that the tool returns image content within the long-edge bound, that `ifChanged` returns the unchanged text on a still screen, and that `path` returns the path alone; then implement; verify red-then-green
- [ ] 5.4 Waits (D7): failing tests that a waiting tool sends progress every 5 s, ends at `--call-timeout` (55 s default) with the daemon's `wait_timeout`, and that an MCP cancellation sends `cancel` and the next call runs at once; then implement; verify red-then-green
- [ ] 5.5 Profiles and instructions (D10): failing tests that `--tools read-only` lists the read-only set alone, that the instructions card's first 512 characters stand alone and the whole is under 4,096, and that `help` returns the agent guide; then implement; verify red-then-green
- [ ] 5.6 `--structured` (D5): failing test that `structuredContent` with the `outputSchema` is sent only with the flag; then implement; verify red-then-green

## 6. Documentation and acceptance

- [ ] 6.1 Docs: `README.md` gains an "Agent surfaces" section (the CLI and the server, how to add the server to Claude Code, Claude Desktop, Codex and Cursor, the profiles), `docs/overview.md` section 7 rewritten for what exists, `crates/cli/README.md` (the server, the tool table, the mapping to agent-browser's names), `crates/daemon/README.md` (the three actions, the types crate), `crates/wire/README.md`, the agent guide where `wait --change` and `close --keep` belong; verify `help agent` under 120 lines and `just validate` passes
- [ ] 6.2 Acceptance: the server added to Claude Code on this host and driven through the fixtures on this session's own Simulator by a fresh agent session given only the instructions card and `help` (a login, a scroll to a row, a screenshot, a wait for a change), with the turns, tool calls and tokens counted and the outcome recorded here; Codex the same if installed; any gap becomes a task before archive
- [ ] 6.3 Update the project memory and archive the change with the two delta specs synced; verify `openspec validate mcp-server --strict` passes and every task above is ticked
