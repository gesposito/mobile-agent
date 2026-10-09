# agent-ios working notes

Checkpoint notes for group 4 of `physical-ios`, so that the work can be
resumed from the branch alone. `README.md` describes the runner; this file
is not part of it.

## Done

- 4.1: `agent-ios` is a workspace member converting XCTest's snapshot with
  `simulator_reader::elements`; the reader's library builds for
  `aarch64-apple-ios` unchanged. The Simulator driver's suite passed on this
  session's own Simulator, `mobile-agent-spike`, now shut down.
- 4.3: `src/server.rs`, `src/dispatch.rs`, `src/shim.rs`, tested on the host
  (the session over TCP, the golden frames, the dispatch over a fake shim).
- 4.2: `build.sh`, `shim/Runner.m`, `just agent-ios-build`. The phone runs
  the built runner, `mobile-agent-ios-runner 0.2.0`: the driver replaced
  spike S10's `0.1.0` on 2026-10-09, so the replacement cannot be repeated
  on this phone without installing another version first.

## Next

- 4.4: record the Simulator's listings of the SwiftUI and Expo Big tabs (and
  the Login tab with the keyboard, the System tab's alert) through the
  mirror; compare with the device's; try the attribute shapes of
  `_XCT_requestSnapshotForElement:` and `XCAXClient_iOS`'s request (at most
  about 2 hours), recording each shape and its reply.
- 4.5: the driver's choice of the runner's synthesis, the runner's input
  checks on the device, the 36-character text entry.
- 4.6: `README.md`.

## Commands

With `MOBILE_AGENT_IOS_TEAM` and `MOBILE_AGENT_IOS_UDID` exported:

```sh
cargo test -p agent-ios                     # the host's tests
agent-ios/build.sh                          # agent-ios/dist/MobileAgentRunner-Runner.app
MOBILE_AGENT_IOS_RUNNER_APP=agent-ios/dist/MobileAgentRunner-Runner.app cargo test -p ios
```

## What the phone needs

- On USB, unlocked, trusted, Auto-Lock at Never. On 2026-10-09 at 21:55
  usbmuxd listed no iPhone SE for a while (only the other phone, over the
  network); it was back on USB by 22:09.
- "Enable UI Automation" approved when iOS asks again.
