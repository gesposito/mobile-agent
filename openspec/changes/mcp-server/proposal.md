# Proposal

## Why

The project has one agent surface, the `mobile-agent` CLI, which follows agent-browser's grammar and ref syntax so that existing skills transfer, one process per command over the daemon's socket. Agents that work through the Model Context Protocol (Claude Code, Claude Desktop, Codex, Cursor) have no way in but a shell tool. The research note `docs/research/agent-surfaces.md` (2026-10-16) found that every mobile automation tool with an MCP server (mobile-mcp, Appium, Maestro, agent-device) exposes 10 to 60 tools that push nothing and return screen dumps of 4 to 6 thousand tokens, that their users' cost is verification turns, and that this project's replies already carry what those turns look for: refs a session owns, `stale_ref` and `element_covered` with advice, verified fills, clicks that report their first frame change and tree delta, delta snapshots of 302 bytes when nothing changed. It also measured what stands in the way: the daemon pushes nothing and has no cancel, a claim is released only by `close`, which stops the app, `snapshot -i --json` carries a refs map that doubles the 22 KB listing, screenshots go to a path, and no schema exists for the 18 actions' 42 parameters, which the handlers read by name.

The maintainer decided the tool stays standalone: its own MCP server over the daemon, mirroring agent-browser's tool stems where the concepts match, with the agent-browser `device` provider and a CDP subset recorded as options and not built.

## What Changes

- A new crate `crates/mcp` whose binary, `mobile-agent-mcp`, is an MCP server over stdio built on the official Rust SDK (`rmcp`), serving both protocol eras, one tool per daemon action an agent drives under agent-browser's stems, flat input schemas, results as text (the listing without the refs map, the CLI's lines for the rest, the `warning` kept), failures as execution errors with the daemon's code and message, screenshots inline with `ifChanged`, progress notifications during waits with a per-call bound, a session per process released on exit, a `--tools` profile (read-only at least) and a short instructions card with the agent guide behind a `help` tool.
- A shared types crate for the daemon's requests and replies, with serde and schemars, used by the daemon's handlers, the CLI's request builder, the harness's reply reader and the MCP server's schemas, so that one definition serves every surface.
- Three daemon additions, exposed by the CLI as well: `wait --change` (returns when the replica's revision or the frames move, with the delta), `close --keep` (releases the claim without stopping the app), and `cancel` (ends the session's command in flight so a target is not held by a wait a client gave up on).
- A spike measuring the server's round trip and result sizes against the CLI's before the server is built on, and an acceptance through a scripted MCP client and through Claude Code on this host.
- The agent-browser provider and the CDP subset: recorded in the design as options, not built.

## Capabilities

### New Capabilities

- `mcp-server`: the MCP surface over the daemon: tools, sessions, results, screenshots, waits, profiles and instructions.

### Modified Capabilities

- `agent-cli`: `wait --change`, `close --keep` and `cancel`, and the shared types the CLI's requests and replies are built from.

## Impact

- A new `crates/mcp` (the server's binary `mobile-agent-mcp`, on the CLI's library for `client.rs` and `output.rs`, with `rmcp`, `tokio` and `wire`'s `schema` feature as its own dependencies, so the CLI's build and start time are untouched), a new `crates/wire` (the request and reply types with their schemas), `crates/daemon` (handlers read the typed requests; `wait --change`, `close --keep`, `cancel`), `crates/bench` (reads replies through the types), `docs/overview.md` section 7, `README.md` (an "Agent surfaces" section), `crates/cli/README.md`, the agent guide, `docs/spikes/s13-mcp-server.md`.
- No change to the drivers, and none to the mirror beyond what `wait --change` reads of it and the cancel flag its waits look at (design D7).
