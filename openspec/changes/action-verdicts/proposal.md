# Proposal

## Why

The second demo pass of 2026-10-22, nine tasks on the iOS Simulator's and the Android emulator's default apps through the MCP server, completed every task with 2 failed calls on iOS against the first pass's 9, and 8 of the first pass's 12 gaps fixed. Four things it met are the tool's to fix, each a reply that does not say what happened. A click answers that something changed, not what the tap landed on: Clock's "Add alarm" tap opened the Bedtime tab and the reply said done; Photos' tap on Favorites right after a scroll reported a tree delta and did not navigate. A settle right after an input can end before the input's effect: `wait --settle` after the Enter that loads a page settled "after 0.01 ms: nothing changed", and once a menu's tree arrived after its settle ended, the next listing showing no element and the one after seven buttons. A ref can be stale on the very next call with nothing between, when the app rebuilds its nodes after a launch (Maps' search box right after `open`'s snapshot), and the only answer is `stale_ref`. `open` of an app that is running behind the home screen answers opened while the home screen stays in front, two of two. And a delta snapshot whose kind is `delta` prints as pretty JSON on both surfaces, where a model reads listings everywhere else. The pass's notes, transcripts and videos are kept privately with the maintainer.

## What Changes

- A click's reply names the element the tap landed on, as the replica had it at dispatch, and warns when it is not the target.
- A settle requested right after the session's input waits for the input's first effect before it counts quiet, and says when none came.
- A stale ref whose element has a twin in the replica as it is, by identifier and role, else by name, role and bounds, resolves to the twin and says so; one without a twin stays `stale_ref`.
- `open` verifies the app is in front once it says opened, activates a running app that stayed behind, and fails naming what is in front when it cannot.
- A delta snapshot's changes print as listing lines in the human output, which the server's results carry; `--json` keeps the delta object.
- A replay of the pass's cases through the server, then the docs and the archive.

## Capabilities

### Modified Capabilities

- `agent-cli`: clicks name what they hit; settles after an input wait for its effect; stale refs resolve to their twin; `open` leaves the app in front; delta snapshots print their changes.

## Impact

- `crates/daemon` (`handlers/act.rs` for the hit element and the settle after an input, `handlers/aim.rs` for the twin, `handlers/mod.rs` for `open` in front, `handlers/view.rs` for the settle), `crates/simulator` and `crates/android` only if activating a running app needs the driver, `crates/wire` (reply fields), `crates/cli` (`output.rs` for the delta lines and the click's line), `crates/mcp` (the card where it describes clicks and settles), `reports/12.3-action-verdicts.md`, the READMEs and the agent guide.
