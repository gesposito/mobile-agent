# Design

## Context

See proposal.md for motivation. What the first change established and this one builds on (`openspec/changes/archive/2026-10-07-mobile-automation-foundation/design.md`, D1 to D12): a persistent daemon with a thin CLI; the device protocol, CBOR frames with a per-connection handshake and capabilities (`tree`, `input`, `screenshot`, `frames`, `events`, `text_set`, `bounds`); the mirror, which keeps a replica per target and already handles windows that send no change events by re-reading them after injected input, on frame hash changes bounded by a minimum interval, and on a poll, which is exactly how the Simulator driver's tree is kept current; the Simulator's in-guest reader, a Rust binary built for `aarch64-apple-ios-sim` that reads the foreground tree with one `-[XCTAccessibilityFramework userTestingSnapshotForElement:options:error:]` per request, prunes modal views and bounds the keyboard window by what it draws, and serves the protocol over a Unix socket; the harness with its target classes, gates, baselines and hand-offs; the compatibility table `doctor` prints.

What the platform allows on a physical device, from the investigation plan and the research so far:

- There is no host-side accessibility read and no host-side input on a device: the tree and input synthesis live in the test automation session, which only an XCTest runner holds. WebDriverAgent, Maestro's iOS driver and agent-device's runner are all XCTest bundles kept alive by a test that never ends; their costs are the runner's cold start (seconds), XCTest's idle wait before each action (which WebDriverAgent disables) and recursive snapshots that grow with the tree.
- The developer services are reachable over USB through usbmuxd and lockdown, and on iOS 17 and later through the CoreDevice tunnel (RemoteXPC, RSD) that Xcode's `devicectl` and pymobiledevice3's tunnels use. The `idevice` Rust crate implements usbmuxd, lockdown and pairing, the CoreDevice proxy with an in-process TCP stack (no TUN device, no root), RSD, the developer disk image mounter, the installation proxy, DVT (process control, screenshots), an `xctest` session with the test manager, a WebDriverAgent bootstrap and a `display_stream` over CoreDevice; its coverage on iOS 26 is what spike S9 verifies.
- Frames: Xcode records a device's screen through CoreDevice, which is what `display_stream` names, and macOS exposes a connected iPhone as a screen-capture device to AVFoundation once CoreMediaIO is told to allow it, the path QuickTime Player's movie recording takes; DVT screenshots are a request each. Which gives a hashable stream at 30 fps under a quarter of a core, and at what latency after a tap, is spike S11's question.
- The accessibility audit service that Xcode's Accessibility Inspector and go-ios's `ax` use navigates the screen element by element (focus, label, perform action) rather than returning a tree, so it is a complement for actions on an element at most, not a tree source.
- Signing: the runner must be signed for a team with a provisioning profile that includes the device; the user has a paid team. Developer Mode must be on, and the device unlocked and trusted. Nothing removes those steps; the driver reports which is missing.
- The device for this change: an iPhone SE (3rd generation, A15) on iOS 26.6.2, under the host's Xcode 27.0; its identifier stays in the environment and the reports' provenance. Hardware buttons and biometrics cannot be assumed to work on a test phone, so the home press goes through the runner's software press and unlocking is done by the person at the device; device names are not unique, so the driver and every spike address a device by identifier, never by name.
- Signing is the account holder's, never a session's: creating the development certificate and registering the device with the account happen in Xcode by hand, and the build scripts here sign with automatic signing only once the device is already in a development profile, so that `xcodebuild` registers nothing. A host with no development identity, profile or team is the starting state `doctor` must name.

Research note: `docs/research/ios-device.md` (task 1.1). Spike reports: `docs/spikes/s9-ios-transport.md`, `docs/spikes/s10-ios-runner.md`, `docs/spikes/s11-ios-frames.md` (tasks 2.1 to 2.3).

## Goals / Non-Goals

**Goals:**
- A physical iPhone as a third target class behind the same protocol, mirror, CLI and harness, with numbers: the gates of the benchmark spec for the `ios-device` class, measured on the user's phone against the baselines that support devices.
- A runner that starts once per session and never restarts in it, speaks the device protocol over a socket, and reads the tree the way the Simulator's reader does, so the mirror's listing rules, refs, modal views and keyboard handling carry over without change.
- A frame stream from the host as the change signal, so settle and the mirror's re-reads work without an event stream, as on the Simulator, and no idle wait anywhere.
- Every host-side piece in Rust, in one new crate; the runner in Rust with an Objective-C shim only where a bundle's entry point or a private call needs one.
- Keep the core lean: nothing device-specific enters `protocol`, `mirror`, the CLI or the Android driver; what the device needs is gated in `crates/ios` and the runner.

**Non-Goals:**
- Wi-Fi devices, watchOS, tvOS, visionOS, more than one iOS version, an in-app SDK, running XCTest test suites, CI with a device.
- Matching the Simulator's numbers. The device tier is the slowest by construction; the goal is the model holding, measured.

## Decisions

Decisions D18 to D23 are candidates until their spike records numbers here (Delivery process, Measured decisions).

### D18. Transport: the `idevice` crate in the daemon's process
`crates/ios` speaks usbmuxd and lockdown for discovery, pairing records and the device's services, and opens the CoreDevice tunnel with `idevice`'s in-process TCP stack to reach the developer services over RSD; no `devicectl`, no Python or Go helper, no root. Alternatives: `xcrun devicectl` (a process per operation, 0.5 to 2 s each, no tree or input, and its tunnel is Xcode's); pymobiledevice3's `tunneld` or go-ios's tunnel (a helper process and another runtime); libimobiledevice bindings (no CoreDevice tunnel). Spike S9 verifies on iOS 26.6.2: pairing reuse, the tunnel from an unprivileged process, the developer disk image mounted or mountable, launch and kill of the fixture through DVT process control with their latencies, and a DVT screenshot's latency; gate: the tunnel without root, a launch under 2 s at p95, a screenshot under 500 ms.

### D19. Lifecycle through the developer services
Install from the signed fixture bundle through the installation proxy, launch and terminate through DVT process control, and the foreground application's identity from the runner's read. `open` settles as on the Simulator, on frames.

### D20. The runner: a resident XCTest bundle in Rust, speaking the device protocol
`agent-ios/` builds an `.xctest` bundle: a Rust `cdylib` with an Objective-C shim that defines the `XCTestCase` subclass whose one test method starts the agent and waits, as WebDriverAgent's does, and the agent is the device protocol's `AgentSession` over a TCP listener on the device that the host reaches through usbmuxd's port connection or the tunnel. The host starts it through the test manager with `idevice`'s `xctest` session, with the fixture's bundle identifier as the application under test when one is wanted, and keeps the session for the target's lifetime; the runner's version travels in its hello, as the Android agent's does, and a mismatch replaces the bundle. The bundle is signed with the team the environment names (`MOBILE_AGENT_IOS_TEAM`, never in a tracked file) and a provisioning profile Xcode manages. Alternatives: WebDriverAgent itself (HTTP and JSON per node, an idle wait per action, a Node and Carthage build); agent-device's runner (its own protocol; a Node dependency); `devicectl`-launched runner without the test manager (no automation session). Spike S10 builds the minimum and measures: cold start to the runner's hello, warm tree read of the two fixtures' 500-cell screens through `userTestingSnapshotForElement:` inside the runner (the Simulator reader's `extract` ported to the device target), tap dispatch through `XCUIDevice`'s event synthesis and through the private `XCTRunnerDaemonSession` synthesis that skips the idle wait, typed text, a screenshot through XCTest against DVT's, and whether the runner survives 30 minutes of commands; gates: tap dispatch p95 under 150 ms, tree p95 under 1 s on either screen, no restart in the session.

### D21. Tree: the Simulator reader's code, in the runner
The reader's guest side (`crates/simulator/reader/src/guest/`) runs inside the XCTest runner on a device as it runs inside the Simulator: the same framework classes, the same snapshot options, the same pruning of modal views and keyboard bounds, the same CBOR snapshot with `emits_events` false on every window. The crate is restructured so the reader's extraction is a library both binaries use (the Simulator's `aarch64-apple-ios-sim` binary and the device's `aarch64-apple-ios` bundle), without changing what the Simulator path does: its tests and golden frames stay. System screens (SpringBoard alerts) are read from the system application's tree as the Simulator's reader does when `primaryApp` is not the target.

### D22. Input through the runner, acknowledged on synthesis
Taps, long presses, swipes that rest before lifting (the Android `rest_ms` semantics, task 10.16), key presses and typed text go through the runner's event synthesis with XCTest's idle wait off, and a request is acknowledged once the event is synthesized, the frame stream carrying what follows. Text goes in typed, or set on the focused element where typing fails, the method reported as on Android. A pasteboard path exists on a device too (`XCUIDevice` has none; the runner can set `UIPasteboard` in its own process, which is not the app's) and is not used.

### D23. Frames: a host-side stream, hashed as the Simulator's
Two candidates, measured by spike S11 on the fixtures' list screen: the CoreDevice display stream (`idevice` `display_stream`) and AVFoundation screen capture of the device through CoreMediaIO's screen-capture allowance (the QuickTime path, through `objc2-av-foundation` and `objc2-core-media` in the daemon's process), each decoded to a pixel buffer, downscaled and hashed with the Simulator pipeline's functions (`scale_bgra`, `pixels_hash`, `content_hash`, which the microbenchmarks time), with DVT screenshots as the on-demand full-resolution path and the fallback when neither stream is available. Gate: 30 frames per second sustained under 25% of a core, a tap's first changed frame within 100 ms of its synthesis at p95, no frame delivered without a pixel change counted as one (the status bar's clock excluded as on the emulator). Without a stream, settle falls back to the mirror's probe-less path for windows without events (re-reads every minimum interval, as the device has no `bounds` capability unless S10 adds one).

### D24. The harness's `ios-device` class
`TargetClass::IosDevice` with the gates the spec delta states, the SwiftUI and Expo fixtures built for the device (`-destination generic/platform=iOS`, the team from the environment), installed by the harness through the driver, and the baselines that support a device: Maestro (its iOS driver needs the team too), agent-device if its runner signs for the team, WebDriverAgent as a reference where it runs. Hand-offs between tools stop and uninstall the other tool's runner as on the Simulator (task 10.18), since one automation session exists per device.

### D25. Workspace layout
```
crates/ios          discovery, readiness, tunnel, lifecycle, runner session, frames, screenshots   (macOS only; a stub elsewhere, as crates/simulator)
agent-ios/          the runner bundle: Rust cdylib, ObjC shim, Info.plist, build and sign script, VERSION
crates/simulator/reader/   gains a library target for the tree extraction both binaries share
```
The driver registers as `ios` in the daemon's driver list; `MOBILE_AGENT_DRIVERS=ios` runs it alone. The daemon's fake targets gain nothing: the fake already stands in for a target without events.

## Delivery process

The rules of the archived design's Delivery process bind this change unchanged (research first, spikes before construction, measured decisions, tests first with red-then-green, atomic commits without trailers, validation and fix after every task, installs through `sfw`, the pinned toolchain, recap before dispatch), with these specific to a device:

- **The device is the user's.** Pairing, trust, Developer Mode, the passcode and the team's certificates are the user's steps; a task that needs one asks and waits, never works around it. The device identifier and the team id go in the environment and the reports' provenance, never in a tracked file. No command touches another session's Simulators or the emulator.
- **One automation session per device.** Spikes and tasks that start a runner stop it when they end; a task that finds another runner holding the device (Maestro's, agent-device's, WebDriverAgent) reports it and stops.
- **Quiet host, charged device.** Measured runs follow the harness's guard and record the load; the device is plugged in, unlocked, with auto-lock off for the run and the battery above half, recorded in the report's provenance.

## Risks / Trade-offs

- [The `idevice` crate does not reach a service on iOS 26.6.2] → S9 finds out first; the fallback for lifecycle is `devicectl` as a process per operation, recorded as such, and the runner's launch through `devicectl`'s process launch without the test manager, which S10 tests as the second path.
- [The test manager refuses a runner that is not built by Xcode, or the provisioning profile does not carry the device] → the build script runs `xcodebuild` on a minimal host project so Xcode manages the profile; the profile and certificate are the user's.
- [XCTest's private event synthesis changes under iOS 26] → the compatibility table records the call site per version, as D12 does for the Simulator; the public `XCUICoordinate` path stays as the slower fallback.
- [No host-side frame stream works at 30 fps] → the mirror runs the device as a target without frames (re-reads every minimum interval while settling, poll otherwise), which the Android device path already is; the gate on tap-to-frame-change stays unstated.
- [The device sleeps, locks or disconnects mid-run] → readiness checks before every attach, `disconnected` with the cause, a resumed attach starts over; auto-lock off is a documented prerequisite of measured runs.
- [The runner's cold start dominates cold attach] → the runner stays installed and the session is kept for the target's lifetime; cold attach's gate counts the session start, not the install.

## Migration Plan

Additive: a new crate, a new bundle, a new target class. Nothing deployed changes for Simulators or Android. Rolling back is removing the crate from the driver list.

## Open Questions

- Whether hardware buttons (home, lock, volume) need a protocol method of their own or go through `key` codes; decided by what S10 finds the runner can synthesize.
- Whether the accessibility audit service's element actions are worth a fallback for apps whose XCTest tree is sparse, as some React Native screens were on the Simulator (`reports/10.4-rn-smoke.md`); measured if a device run shows such a screen.
- Which baselines support a physical device on this host without their own signing setup; found out in task 7.3 and recorded in the report.
