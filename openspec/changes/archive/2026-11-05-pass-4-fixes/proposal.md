# Proposal

## Why

The fourth pass on real apps (2026-10-30, the nine tasks of the earlier passes through the MCP server) and the site's two recording runs of the same day found nine gaps the earlier changes did not meet, beside the time picker's overlay they left: a launch the Simulator refuses right after an app closes, a read-back one keystroke early, a settle that gives up with a success exit, a tap by coordinates naming nothing, an interactive listing empty after an alert closes, a stale ref silent about the dialog in front, a text wait matching a prefix, a header covering a button in plain view, a node Calendar keeps. Several may be the host's state rather than the tool's: the passes ran on one Mac under load, and CI runs no device-backed test, while its benchmarks workflow already provisions a fresh Simulator and an emulator, where the fixture's list screen failed every tool under iOS 27. So the device-backed tests go to CI first, each finding is sorted there, and the fixes follow their case.

## What Changes

- A CI job runs the device-backed tests on a fresh Simulator and emulator, on a schedule and on demand, with tests on the default apps for the findings, each asserting what the device guarantees and never a fixed number of milliseconds.
- `open` retries a launch the Simulator refuses with `NSPOSIXErrorDomain 3` once, after a bounded wait, and says so.
- `type` and `fill` read the field back until it stops changing or holds the text, within a bound, and warn when the read-back is shorter than the text.
- A settle that stops at its timeout is told apart from one that settled, in the JSON reply and, on request, in the exit status.
- A tap by coordinates names the labeled element under the point, `generic` only when nothing under it is labeled.
- The interactive listing after a system window closes lists what the screen shows.
- A stale ref with no twin under a system dialog names the dialog.
- `wait --text` prefers an element whose text is the whole text over one that merely starts with it.
- The header cover rule, the time picker's overlay, Calendar's kept node, the list screen under iOS 27 and the click's change window: reproduced on CI's devices, fixed where they are the tool's, recorded where they are the app's or the host's.
- The public report of the fourth pass and the site's runs, the README's known issues, the docs, the archive.

## Capabilities

### Modified Capabilities

- `agent-cli`: the read-back waits for the field; a settle that gives up says so; a tap by coordinates is named; a stale ref names the dialog in front; a text wait prefers the whole text.
- `ui-mirror`: a listing after a system window closes; a header covers what lies under it, not what it holds.
- `simulator-driver`: a refused launch is retried once.

## Impact

- `crates/daemon` (`handlers/act.rs`, `handlers/view.rs`, the worker, the fake's typing and alerts), `crates/mirror` (the cover rule, the hit's name, the system dialog in a stale ref's failure), `crates/simulator` (the launch), `crates/cli` and `crates/mcp` (the settle's status and lines), `crates/wire` where a field is added.
- `.github/workflows/` (the device-tests job, reusing the benchmarks workflow's provisioning), `crates/daemon/tests/` (the gated tests on the default apps).
- `README.md` (Status and limitations, the measurements), `docs/overview.md`, `docs/agent-guide.md`, `reports/12.5-fourth-pass.md`.
