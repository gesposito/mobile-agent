# agent-ios working notes

Checkpoint notes for group 4 of `physical-ios`, so that the work can be
resumed from the branch alone. `README.md` describes the runner; this file
is not part of it.

## Done

- 4.1: `agent-ios` is a workspace member converting XCTest's snapshot with
  `simulator_reader::elements`; the reader's library builds for
  `aarch64-apple-ios` unchanged. The Simulator driver's suite passed on this
  session's own Simulator, `mobile-agent-spike`, now shut down.

## Next

- 4.3 on the host: `src/server.rs`, `src/dispatch.rs`, `src/shim.rs` with
  their tests (the session over TCP, the golden frames, the dispatch over a
  fake shim).
- 4.2: `build.sh`, `shim/Runner.m`, `.gitignore`, `just agent-ios-build`, a
  device test that the built bundle replaces spike S10's 0.1.0 runner.
- 4.4 and 4.5 on the device; 4.6 the README.

## Commands

With `MOBILE_AGENT_IOS_TEAM` and `MOBILE_AGENT_IOS_UDID` exported:

```sh
cargo test -p agent-ios                     # the host's tests
agent-ios/build.sh                          # agent-ios/dist/MobileAgentRunner-Runner.app
MOBILE_AGENT_IOS_RUNNER_APP=agent-ios/dist/MobileAgentRunner-Runner.app cargo test -p ios
```

## What the phone needs

- On USB: on 2026-10-09 at 21:55 usbmuxd listed no iPhone SE at all (only
  the other phone, over the network) and the device tests failed with
  `not_found`; plug the SE in, unlocked, trusted, Auto-Lock at Never.
- "Enable UI Automation" approved when iOS asks again.
