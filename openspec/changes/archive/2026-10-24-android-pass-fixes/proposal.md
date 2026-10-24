# Proposal

## Why

The third demo pass of 2026-10-23, the same nine tasks on the iOS Simulator's and the Android emulator's default apps through the MCP server on the same model as the second, left iOS with no failed call and the fixes of `action-verdicts` holding, and Android with four failed calls against one. One of them is the tool's: Chrome's `open` ran `am start -W` over adb, the socket answered a would-block error, "Resource temporarily unavailable (os error 35)", which is how macOS reports the adb client's 10 s read timeout expiring (design D1), and the driver reported it as `adb_failed` rather than as a launch that outlasted its deadline; the snapshot that followed ran past its 10 s deadline. The other three came with an emulator short of memory (2.4 of its 2.5 GB used, 700 MB of swap), on which `open` took 10 to 19 s against 1 to 4 s in the second pass, of which `am start -W` alone took 6 to 7 s and the daemon about 8 s more waiting for the app's first tree; whether that wait is the emulator's or the daemon's is not settled. Two more things the pass counted: a click names nothing when no element with a ref lies under its point, on Safari's address field and on a tap by coordinates, where a model can least tell where it landed; and a stale ref resolved to its twin costs about 2 s a call against 0.3 to 1.0 s for other clicks and fills. The pass's notes, transcripts and videos are kept privately with the maintainer.

## What Changes

- An Android launch waits for `am start -W` until `open`'s own deadline instead of the adb client's 10 s read timeout, whose expiry macOS reports as the pass's would-block, and fails past it as a timeout naming the deadline.
- A click's `hit` names the element under the point whether or not the session holds a ref for it, the ref null when none.
- A stale ref's twin costs a replica lookup, not seconds; the cost is measured before and after.
- The daemon's wait for an Android app's first tree is measured on a cold emulator at its default memory and at 4 GB, against `am start -W`; the daemon's share is cut if it is the daemon's, else the emulator's memory is the README's advice.
- A replay of the pass's Android failures through the server, then the docs and the archive.

## Capabilities

### Modified Capabilities

- `android-driver`: a launch takes the command's deadline.
- `agent-cli`: hits are named without a ref.

## Impact

- `crates/android` (the adb shell transport), `crates/daemon`'s driver contract and Android driver (the launch's deadline), `crates/mirror` (`hit`, `twin`), `crates/daemon` (`handlers/act.rs`, `aim.rs`, `mod.rs` for the first tree), `crates/wire` if a field changes, `crates/cli` and `crates/mcp` where the lines or docs quote a hit, `reports/12.4-android-pass.md`, the READMEs.
