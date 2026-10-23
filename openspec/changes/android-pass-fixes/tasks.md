# Tasks

Rules that bind every task below (the archived foundation design's Delivery process): research first; measured decisions recorded in design.md; fail-first tests observed red; atomic commits without trailers; the validation-and-fix phase before the next task; installs through `sfw`; the quiet-host rule for numbers; every daemon of a task on a private socket directory; one agent at a time.

## 1. The driver and the hit

- [ ] 1.1 Would-block retried (D1): failing test on the Android driver's shell transport with an injected `WouldBlock` on `am start` and on a read, that the command completes and that one outlasting the deadline is the timeout; then implement; verify red-then-green
- [ ] 1.2 A hit without a ref (D2): failing tests on the fake that a tap by coordinates on a plain text names it with `ref` null and the line says `(no ref)`, and that a click on a ref'd element keeps its ref; then implement; verify red-then-green and on this session's own Simulator a tap on Safari's address bar naming the field

## 2. The costs

- [ ] 2.1 The twin's cost (D3): measure the spans of a resolved click and fill on the fake's rebuilt board and on the Simulator (Maps' search box), find the wait, cut what the action does not need; failing test that a resolved call's span is within a plain one's plus 50 ms on the fake; then implement; verify red-then-green with the numbers before and after in D3
- [ ] 2.2 The first tree on Android (D4): the measurement on a cold-booted `pixel_api36` at 2 GB and with `-memory 4096`, quiet host, Settings, Clock and Chrome cold and warm, `firstTreeMs` against `am start -W`; the daemon's share cut if it is the daemon's (failing test first), else the README's advice and the harness's provenance; the numbers in D4

## 3. Verification and docs

- [ ] 3.1 The replay through the server: on the emulator Chrome's `open` five times with the retry in place and the opens' times, on the Simulator the hit on Safari's address bar and a coordinate tap, and a resolved twin's time; `reports/12.4-android-pass.md`, scrubbed; the transcripts kept privately
- [ ] 3.2 Docs: the READMEs where they quote a hit or describe stale refs, the Android section's emulator memory advice, `crates/mcp/README.md` and the card if they describe hits; `just validate` green
- [ ] 3.3 Update the project memory and archive the change with its two delta specs synced; verify `openspec validate android-pass-fixes --strict` passes and every task above is ticked
