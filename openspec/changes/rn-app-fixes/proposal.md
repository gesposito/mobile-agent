# Proposal

## Why

The first smoke test of a production React Native app (the maintainer's own, driven through the CLI by another agent session on an iOS 26 Simulator, 2026-10-09) completed every flow and ran its sign-in 1.4 times faster than agent-device and 2.8 times faster than Maestro, and found five defects that every React Native app will show: a tap on a heading under a fixed footer accepted a consent dialog, because iOS reports no occlusion and the mirror cuts only modal views; a click on an element 0% in view tapped stale bounds on the number pad instead of refusing; `fill` on a six-box code field typed the code and then reported `text_mismatch` because it reads the value back from the field alone; the first keystrokes into a just-focused field were dropped in 3 of 8 runs while the keyboard slid in; and a selected tab never got a ref. The physical-device change inherits all five, so they are fixed first, with fixture tests of the shapes that showed them.

## What Changes

- The mirror cuts, on a platform whose tree order is its drawing order (iOS), what a later sibling drawn over an earlier one spans when it shows something: fixed footers, floating buttons and sticky headers, the counterpart of Android's `on_top` rule; `visibility` and `element_covered` follow.
- An action on an element none of whose bounds are in view, after one re-read of its window, fails with a code instead of tapping bounds the replica holds from before a layout change.
- `fill` verifies through the field's descendants when the field holds its value in them, and reports an entry it cannot read back as unverified rather than as a mismatch; the reply's `method` documents that `direct` is a paste on iOS, with the pasteboard side effect.
- On iOS, text typed into a field the same action focused waits for the keyboard window to be up before the first key, bounded.
- A tab control gets a ref whether selected or not; a checkbox is listed as one; a text node and a scroll bar appear once in the full listing; a heading is nested once.
- A wait whose condition already holds in the replica answers without waiting for a due read.
- Documentation: the agent guide and the CLI's README on the new codes and the read-back's outcomes.
- Not in this change: the device driver (`physical-ios` continues in parallel on disjoint files); the app's own accessibility defects (its code field's name spells the code out), which are the app's.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `ui-mirror`: "Snapshot content" gains the drawing-order cover on iOS, the tab ref and the listing corrections; "Single coordinate contract" gains the refusal of an action on an element out of view; "Wait predicates from the event stream" gains the answer from the replica when the condition already holds.
- `agent-cli`: a new requirement, "Text entry verification", on what `fill` reads back and reports.
- `simulator-driver`: "Text entry" gains the keyboard-ready wait before the first key.

## Impact

- `crates/mirror` (`view.rs` for the cover and the refs, `render.rs` for the listing corrections, `mirror.rs` for the wait; fixtures of the consent, tab bar, code field and members screens' shapes, synthetic), `crates/daemon/src/handlers/act.rs` (the refusal, the read-back through descendants, the keyboard wait on iOS), `crates/cli/src/agent-guide.txt` and `crates/cli/README.md`; the microbenchmark baseline is re-run after the mirror changes, since the listing's cost may move.
- Acceptance: the same smoke test rerun by the session that ran it, on the same Simulator and app, with the five defects gone and the timings not worse.
