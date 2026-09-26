# Proposal

## Why

Agents that drive mobile apps today pay for JVM cold starts, HTTP servers on the device, fixed idle waits and per-step polling, so a single tap-and-observe step costs hundreds of milliseconds to seconds even though the platform primitives underneath can answer in single-digit milliseconds. agent-browser has shown what a daemon-based, ref-driven CLI does for web agents; nothing equivalent exists for iOS and Android, and the two fastest paths the platforms allow, a resident shell-user process on Android and fully host-side Simulator control on macOS, are used by no agent tool today.

## What Changes

- New Rust workspace hosting a persistent daemon, a thin CLI, the wire protocol, the UI mirror, an Android driver, an iOS Simulator driver and a benchmark harness.
- New Android resident agent: a single-dex Kotlin program pushed over ADB and started with `app_process` as the shell user. It owns the accessibility tree, injects input, takes screenshots and streams change events. No APK install, no instrumentation run, no user consent prompt.
- New iOS Simulator driver that lives entirely on the host: HID input through the Simulator's own input services, accessibility tree reads through the Simulator runtime's private accessibility client, frames from the display surface. No runner bundle, no xcodebuild.
- New live UI mirror: the daemon keeps an always-current replica of windows, tree and latest frame fed by device events, serves snapshots from the replica, assigns stable element refs, and answers wait and settle queries from the event stream instead of polling.
- New emulator fast path: when the Android target is an emulator with its control port enabled, input and frames go through the emulator's gRPC controller.
- New binary wire protocol between daemon and device agents with version and capability negotiation, request multiplexing and an event channel.
- New CLI whose command grammar, ref syntax and JSON output follow agent-browser for the subset that applies to devices, with daemon auto-start and per-target sessions.
- New benchmark harness that measures per-primitive and end-to-end latency, compares against Maestro, agent-device and maestro-runner on the same flows, and fails on stated gates.
- Explicitly deferred to follow-up changes: physical iOS devices, the in-process injected Simulator agent, the MCP server, the CDP-subset endpoint, the local decision-model hook, and Android video streaming.

## Capabilities

### New Capabilities
- `device-protocol`: framing, handshake, capability negotiation, request and event multiplexing and error model shared by every device agent and the daemon.
- `ui-mirror`: the event-driven replica of device UI state, snapshot semantics, ref stability, settle and wait predicates, and quality verdicts on degraded data.
- `android-driver`: deployment and lifecycle of the resident Android agent, tree acquisition, input injection, screenshots, event streaming, emulator fast path, single-owner conflict handling.
- `simulator-driver`: Simulator discovery, boot and app launch, host-side tree reads, HID input with per-runtime transport negotiation, frame capture.
- `agent-cli`: the command surface exposed to agents, daemon lifecycle, sessions, output formats, batch execution.
- `benchmark-harness`: what is measured, how baselines run, the report format and the pass or fail gates.

### Modified Capabilities
None. This is a greenfield project with no existing specs.

## Impact

- New repository at this project root. Rust crates for everything host-side; one Kotlin source tree for the Android agent built with the Android SDK command-line tools directly, no Gradle.
- New external dependencies: an ADB client crate, tonic for the emulator gRPC controller, objc2 and mach2 for Simulator private frameworks, a CBOR codec, turbojpeg. Toolchains: stable Rust, Android SDK build-tools and platform 36, Xcode with at least one Simulator runtime.
- Uses private Apple Simulator frameworks and hidden Android framework APIs. This is development tooling and never ships inside an app.
- Android permits one UI test automation service per device. While the daemon holds a device, Maestro, Appium and `uiautomator dump` cannot run against it; the daemon reports the conflict rather than failing silently.
- macOS host required for the Simulator driver; macOS or Linux for the Android driver.
