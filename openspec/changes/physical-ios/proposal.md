# Proposal

## Why

mobile-agent drives iOS Simulators from the host and Android devices through a shell-user agent, and a physical iPhone has been a stated non-goal since the first change: the only supported route to its screen is an XCTest runner, which cannot carry the host-side speed story. Agents that test shipping apps still need the phone, for what a Simulator does not have (push notifications, the camera, Face ID, real performance, carrier and Bluetooth paths, device-only bugs), and the ranked backlog's last item asks whether the model holds there at all: a resident runner that never restarts, a live mirror fed by the host's frame stream instead of XCTest's idle waits, and the same protocol, CLI and harness as the other two targets. The investigation that preceded the foundation change (its spike C, kept privately) reserved it for a later change with its own spikes and gates; this is that change, on the maintainer's iPhone on iOS 26.6.2 with a paid developer team to sign with.

## What Changes

- A research note on driving physical iOS devices: the device services and their transports on iOS 17 and later (usbmuxd, lockdown, the CoreDevice tunnel, RemoteXPC, DVT), what WebDriverAgent, Maestro's iOS driver, agent-device's runner, idb, go-ios and pymobiledevice3 do and cost, the `idevice` Rust crate's coverage, the accessibility audit service, the screen capture paths, and signing and Developer Mode constraints.
- Three spikes with go or no-go gates, recorded in `docs/spikes/`: S9 transport and lifecycle from Rust (tunnel without root, launch and kill of the fixture, screenshots), S10 a resident XCTest runner in Rust that speaks the device protocol (cold start, warm tree read, tap dispatch, typing, whether it survives the session), S11 a frame stream from the host (the CoreDevice display stream or screen capture against DVT screenshots: fps, latency, CPU).
- A new driver crate, `crates/ios`, behind the existing traits: device discovery with readiness checks, the tunnel, app lifecycle, the runner's deployment and session, tree reads, input, frames and screenshots.
- A runner bundle, `agent-ios/`, built once per signing team and kept resident for a session: the device-side counterpart of `agent-android/`, speaking the same CBOR protocol with the `tree`, `input`, `screenshot` and `text_set` capabilities.
- The daemon registers the driver (`MOBILE_AGENT_DRIVERS` gains `ios`), `devices` lists physical devices with their readiness, `doctor` checks pairing, Developer Mode, the developer disk image, the signing identity and the runner build, and the mirror treats the device's windows as windows without change events, re-read on frame changes and polls, as the Simulator's are.
- The SwiftUI and Expo fixtures build for a device with the team's signing, the harness gains the `ios-device` target class with its gates, and the primitives and flows run on the maintainer's iPhone through `mobile-agent` and through the baselines that support devices, in a report under `reports/`.
- Not in this change: watchOS, tvOS and visionOS; devices over Wi-Fi (USB only); an in-app SDK; more than one iOS version validated (the maintainer's device runs iOS 26.6.2); CI runs with a device.

## Capabilities

### New Capabilities

- `ios-device-driver`: what the host does with a physical iOS device: discovery and readiness, the transport without root, the runner's deployment and lifetime, tree reads, input, frames and screenshots, app lifecycle, failure containment, and the compatibility statement per iOS and Xcode version.

### Modified Capabilities

- `benchmark-harness`: the latency gates gain the `ios-device` target class with its defaults, set by the spikes and the first device run; the primitives, baselines, fixtures and reports apply to the new class as they are.

## Impact

- New: `crates/ios` (the driver; depends on `protocol`, `mirror` and the `idevice` crate), `agent-ios/` (the runner bundle: Rust `cdylib` with an Objective-C shim, its Info.plist, a build script running `xcodebuild` or `codesign` with the team's identity), `docs/research/ios-device.md`, `docs/spikes/s9-*.md` to `s11-*.md`, `spikes/s9-*` to `s11-*`, `reports/<n>-ios-device.md`.
- Changed: `crates/daemon` (driver registration, `devices`, `status`, `doctor` rows, target class), `crates/cli` (`doctor` output and the agent guide where the device differs), `crates/bench` (`TargetClass::IosDevice`, gates, provenance), `fixtures/swiftui` and `fixtures/expo` build scripts (a device destination and signing from an environment variable, never a team id in tracked files), `justfile` and CI (the new crate builds on macOS only; nothing runs against a device in CI), `README.md`, `docs/overview.md`, the compatibility table.
- Unchanged by design: `protocol`, `mirror` and the Android driver. A new protocol capability or method is added only if a spike shows the runner needs one (a candidate: hardware buttons).
- Prerequisites on the host and device, done by hand: the iPhone plugged in over USB, unlocked and trusted, Developer Mode on, the team's signing identity available to Xcode or the keychain, and the environment variable naming the team.
