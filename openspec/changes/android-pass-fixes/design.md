# Design

## Context

The third pass's Android failures (notes kept privately): `open com.android.chrome` answered `✗ adb_failed: shell,v2,raw:am start -W … Resource temporarily unavailable (os error 35)`; the next `snapshot` answered `deadline_exceeded` at 10 s; a full snapshot and a screenshot then took 11 and 12 s. The emulator (`pixel_api36`, 2 GB of RAM in its configuration, 4 cores) had 2.39 of 2.53 GB in use and 697 MB of swap. Through the CLI, Settings' cold `open` took 14.65 s with `firstTreeMs` 14,438 while `am start -W` alone reported 6.3 to 6.9 s; warm, 0.72 s. The driver's socket to adb is non-blocking in places (the fake agent and the daemon's server already read again on `WouldBlock`), and the shell path does not. `Mirror::hit` returns the element a tap reaches at a point through `visibility::landing`, and the click reports it when the session holds its ref. A stale ref's twin (`Mirror::twin`) took 2.38 and 2.62 s end to end in the pass's two cases.

## Goals / Non-Goals

**Goals:**
- No command fails on a would-block the transport can wait out.
- Every click names what it hit.
- A twin costs what a lookup costs.
- The first-tree wait on Android understood, with numbers, and cut where it is the daemon's.

**Non-Goals:**
- The Material time picker's unref'd overlay, `wait --text` matching a suggestion, the click's 200 ms window on Android (recorded as limitations).
- Changing the user's AVD; the emulator's memory is set per run with its `-memory` flag.

## Decisions

### D1. Would-block on the adb socket is read again
The Android driver's shell transport reads and writes again on `ErrorKind::WouldBlock` (and `Interrupted`), as the fake agent and the daemon's server do, bounded by the command's own deadline, so `am start` and the other shell commands do not fail on a socket that is not ready; a would-block that outlasts the deadline is the timeout it always was, with its message. Measured by a test that injects the error on a shell command's stream.

### D2. A hit is named without a ref
`Mirror::hit` returns the element a tap reaches at the point whether or not the session holds a ref for it: `hit` carries `role`, `name` and `ref` null, the line `hit text "Address" (no ref)`, so a tap by coordinates or on a plain text says what it landed on; the mismatch warning compares elements, not refs. A keyboard hit stays as it is.

As built (task 1.2, 2026-10-23): `visibility::landing` keeps its pass over the keyboard and the elements with a ref, and where no element with a ref lies under the point in any window takes the top-most element the full listing lists without one (`render::listed_in_full`: a named text or image, a named or identified container, a node of another role), visible, folded into nothing and reached at the point, chosen among the windows and within one as those with a ref are, so every hit with a ref and the keyboard's are as they were; `Mirror::hit` gives it with no ref, none having been issued. The reply's `hit` keeps its shape, `{role, name, ref}`; the CLI's line, which the server's results are, says `(no ref)` after any hit but the keyboard whose `ref` is null, so also after a button a click by selector reached before any snapshot of the session listed it, as two of the server's tests show, whose expectations say so; the warning compares the elements with `holds`, as before. Red on the fake's login screen, a click at (200, 316) on its status answering `"hit":null`, and on the line without `(no ref)`; green with the workspace's 1,783 tests. On this session's Simulator Safari's full listing gave its address bar's title as `text "Address" [id=TabBarItemTitle]: example.com`, without a ref, the full `snapshot --json` carrying no bounds for it, so its point came from `get box` (149, 806, 104x20.33): `click 201 816` answered `"hit":{"name":"Address","ref":null,"role":"text"}` and `✓ Done (hit text "Address" (no ref), dispatch 0.0 ms, first frame change 121.3 ms, first tree delta 156.3 ms)`, the field taking the focus, and a click by coordinates on the Close button beside the field named it by its ref, `"hit":{"name":"Close","ref":"@e39","role":"button"}`.

### D3. A twin costs a lookup
The cost of a twin is measured in the daemon's spans (the stale check, the lookup, any wait between); the lookup itself walks the replica once. Whatever waits before it that the correctness of the action does not need (a settle, a read-back hold that belongs after the entry, a re-read) goes; the measurement after shows a resolved call within the span of a plain one plus the lookup. The numbers go in this decision's "As built".

### D4. The first tree on Android, measured
On a cold-booted emulator, quiet host, twice: at the AVD's 2 GB and with `-memory 4096`: `open` of Settings, Clock and Chrome cold and warm through the CLI with `firstTreeMs`, against `am start -W`'s TotalTime for the same launches, and the daemon's attach time. If the daemon's share beyond the app's own launch is seconds on the healthy emulator, the wait is found (the agent's first read, the attach, the poll) and cut; if it is the starved emulator's, the README's Android section says so with the `-memory` flag, and the harness's provenance records the emulator's memory where it records the AVD.

## Risks / Trade-offs

- [Retrying a would-block hides a wedged adb] → bounded by the deadline; the timeout names the command.
- [A hit without a ref names a node the model cannot act on] → the line says `(no ref)`; the next listing gives it one if it is actionable.
- [The twin's cost is a wait the correctness needs] → the measurement says which; the design records it rather than cutting it.
