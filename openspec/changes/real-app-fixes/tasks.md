# Tasks

Rules that bind every task below (the archived foundation design's Delivery process): research first; spikes with go/no-go before building on an unproven assumption; measured decisions recorded in design.md; fail-first tests observed red; atomic commits without trailers; the validation-and-fix phase before the next task; installs through `sfw`; the quiet-host rule for numbers; every daemon of a task on a private socket directory; one agent at a time.

## 1. Spike

- [ ] 1.1 S14 (D1): on this session's own Simulator, Settings' root, Calendar's New Event sheet and Maps' search field against the fixture's List tab: the listings, `get box` of the missing elements, the replica's windows and containers with frames and content sizes, the visibility inputs for one missing row; the switch presses (tap, 40, 80, 150 ms, ten each, on Settings' Bold Text and the fixture's Animate); `docs/spikes/s14-settings-visibility.md` with its row in `docs/spikes/README.md`, the programs kept privately; the cause named and the go/no-go recorded in D1 and D6

## 2. The mirror

- [ ] 2.1 The fake screen or mirror fixture with the spike's geometry and a failing listing test (D2); then the computation's fix (or the gated reading of D1's no-go); verify red-then-green and, on the Simulator, Settings' root rows, Calendar's event block and Maps' search field listed and clickable

## 3. The daemon and the CLI

- [ ] 3.1 `type` reads back (D3): failing tests on the fake that `type`'s reply carries the read-back and the CLI prints `✓ Done (read back "...", typed)`, that a mismatch warns and says to `fill`, and that no focused field gives `not read back`; then implement; verify red-then-green
- [ ] 3.2 Replies that name what the screen holds (D5): failing tests that a `wait_timeout` summary's refs are clickable, `get value` on a switch returns its state and `no value` otherwise, `wait --text` names the element, `open` returns at an alert naming it (the fake's permission dialog), the screenshot reply carries `points` and the line says both sizes, and an off-screen click names the screen's points; then implement; verify red-then-green
- [ ] 3.3 The switch press (D6), only if S14 says so: failing test on the fake's switch class with the gated hold; then implement; verify on Settings' Bold Text ten of ten

## 4. The server

- [ ] 4.1 The bound caps (D7): failing tests that a call without `timeout` sends none and a `click` on a missing element fails at the daemon's default, that a larger `timeout` is capped and said, and that `wait`'s description says to pass `timeout`; the screenshot text states both sizes; the card's sentence on a click that reports no change; then implement; verify red-then-green, `help` under its size tests

## 5. Verification and docs

- [ ] 5.1 The second pass (through the server, one `claude -p` per task as the demo pass ran them, on this session's own Simulator): Settings' Bold Text on and off, Calendar's event, Safari's address bar to example.com, with turns, calls, failed calls and cost against the first pass's 47 / 17 / 23 turns; `reports/12.2-real-apps.md` scrubbed; the transcripts kept privately
- [ ] 5.2 Docs: the READMEs' output rows and the agent guide for the changed lines (under 120 lines), `crates/mcp/README.md`'s waits and screenshot sections, `docs/overview.md` where it cites the first pass; `just validate` green
- [ ] 5.3 Update the project memory and archive the change with its three delta specs synced; verify `openspec validate real-app-fixes --strict` passes and every task above is ticked
