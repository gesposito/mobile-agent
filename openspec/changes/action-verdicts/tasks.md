# Tasks

Rules that bind every task below (the archived foundation design's Delivery process): research first; measured decisions recorded in design.md; fail-first tests observed red; atomic commits without trailers; the validation-and-fix phase before the next task; installs through `sfw`; the quiet-host rule for numbers; every daemon of a task on a private socket directory; one agent at a time.

## 1. The CLI's output

- [ ] 1.1 Delta lines (D5): failing CLI output tests for a `delta` reply printing the header line, the lines and the removed refs, and the server's result carrying them; then implement; verify red-then-green and `--json` byte for byte as before

## 2. The daemon

- [ ] 2.1 The click names what it hit (D1): failing tests on the fake that a click's reply carries `hit` and its line names it, that a click by coordinates names the element under the point, and that a layout change between the snapshot and the tap (the fake's rebuilt board, or `--fake-latency`) gives the warning naming both; then implement; verify red-then-green and the click's dispatch span unchanged within noise (`just micro --filter daemon/` on a quiet host if the path is timed there)
- [ ] 2.2 A stale ref resolves to its twin (D3): failing tests on the fake's rebuilt board that a click, a fill and a get on a stale ref act on the twin by identifier and role, else by name, role and bounds, with `resolved` in the reply and the line, and that a ref without a twin fails `stale_ref`; then implement; verify red-then-green
- [ ] 2.3 A settle after an input waits for the effect (D2): failing tests with `--fake-latency 300` that `wait --settle` right after a click settles after the change and says what changed last, that with no effect it says nothing changed within 1 s of the click, and that a settle with no input in the last second behaves as before; then implement; verify red-then-green and `open`'s settle the same way
- [ ] 2.4 `open` leaves the app in front (D4): first measure on the Simulator what a launch of a running app behind the home screen does and which call brings it to the front; failing tests on the fake (a target whose app runs behind the home screen) that `open` activates it or fails `not_in_front` naming what is in front; then implement; verify red-then-green and on the Simulator `press home` then `open com.apple.Preferences` five of five with Settings in front

## 3. Verification and docs

- [ ] 3.1 The replay through the server on this session's own Simulator: a click on a row right after a scroll (Photos' Favorites), a fill on the search box right after `open`'s snapshot (Maps), `wait --settle` right after the Enter that loads a page (Safari), `press home` then `open` (Settings), and the delta after a toggle (Settings), with the replies recorded in `reports/12.3-action-verdicts.md`, scrubbed; the transcripts kept privately
- [ ] 3.2 Docs: the READMEs' output rows and the agent guide for the changed lines (under 120 lines), `crates/mcp/README.md` and the card where they describe clicks and settles, `docs/overview.md` where it cites the passes; `just validate` green
- [ ] 3.3 Update the project memory and archive the change with its delta spec synced; verify `openspec validate action-verdicts --strict` passes and every task above is ticked
