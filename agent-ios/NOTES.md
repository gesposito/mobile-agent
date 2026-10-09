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
- 4.2: `build.sh`, `shim/Runner.m`, `just agent-ios-build`. The driver
  replaced spike S10's `0.1.0` with `0.2.0` on 2026-10-09, so the
  replacement cannot be repeated on this phone without installing another
  version first.
- 4.4: the tree through `XCAXClient_iOS`'s request without
  `XC_kAXXCAttributeChildren` (classes from `ElementBaseType`, scroll areas,
  interaction, modal views not honoured); the four screens list through the
  mirror as the Simulator's listings in `crates/ios/tests/fixtures/listings/`
  (recorded on `mobile-agent-spike`). The driver's DVT channel passes over
  the device's own messages (a launched app's output).
- 4.5: `AttachOptions::input` (`InputChoice::Auto`, `Only(InputPath)`); the
  runner's shifted keys, volume buttons, text set where typing fails and
  `XCUICoordinate` last; the device tests and the measured series.
- 4.6: `README.md`.

The phone runs the runner of the last commit that changed `shim/` or
`src/`, installed with `devicectl device install app` since `VERSION`
stayed `0.2.0`; no runner process is left after the tests.

## Next

Group 4 is done. Left for later groups or open:

- a view the app marks modal (other than an alert) is not marked on the
  device: the reply counts no children, so no pruned read can be told;
- sheets and system alerts were not exercised on the device;
- `XCUICoordinate`, the last resort, was not exercised (XCTest 25228 has
  the synthesizer);
- 3 of 60 runner taps showed no changed frame within 2 s, all in the first
  run of `runner_input_series`, whose cause was not found.

## Commands

With `MOBILE_AGENT_IOS_TEAM` and `MOBILE_AGENT_IOS_UDID` exported:

```sh
cargo test -p agent-ios                     # the host's tests
agent-ios/build.sh                          # agent-ios/dist/MobileAgentRunner-Runner.app
MOBILE_AGENT_IOS_RUNNER_APP=agent-ios/dist/MobileAgentRunner-Runner.app cargo test -p ios
cargo test -p ios --test tree -- --ignored --nocapture tree_series
cargo test -p ios --test input -- --ignored --nocapture runner_input_series
```

## What the phone needs

- On USB, unlocked, trusted, Auto-Lock at Never. On 2026-10-09 at 21:55
  usbmuxd listed no iPhone SE for a while (only the other phone, over the
  network); it was back on USB by 22:09.
- "Enable UI Automation" approved when iOS asks again.
