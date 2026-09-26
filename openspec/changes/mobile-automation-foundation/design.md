# Design

## Context

See proposal.md for motivation. The constraints that shape the approach come from the platform investigation that preceded this change, kept privately:

- Neither OS exposes a CDP-like protocol. Android's only privileged UI primitive is the UI automation connection normally used by instrumentation; a shell-user process started with `app_process` may own it, because the connection records whoever connects as its owner and the Shell package holds the accessibility, event-injection and frame-buffer permissions. Only one such owner exists per device.
- On the iOS Simulator every primitive is reachable from the host process: CoreSimulator for lifecycle, SimulatorKit's input services for HID, the runtime's private accessibility client for trees, and the display surface for frames. These are private frameworks whose shapes changed in iOS 26 and Xcode 27, so transports must be negotiated at runtime.
- On a physical iPhone the only supported route is an XCTest bundle. That is a separate change.
- Emulators expose a gRPC controller for input and frame streaming when started with the control flag.
- agent-browser's daemon is pure Rust, speaks CDP directly, and its command grammar is what agents already know. The exact envelope, ref syntax, snapshot line format, delta snapshots and batch semantics to mirror are catalogued in the protocol research note.

Research notes (task group 2): `docs/research/protocol.md`, `docs/research/simulator.md`, `docs/research/android-agent.md`.

## Goals / Non-Goals

**Goals:**
- Prove the live-mirror model end to end on Android and Simulator with numbers, not claims.
- Keep every host-side component in Rust and keep the only non-Rust component, the Android agent, as small as a single dex.
- Make each backend swappable behind the protocol so later changes can add physical iOS, an in-process agent, and CDP or MCP edges without touching the mirror or the CLI.
- Partition work so independent agents can implement crates in parallel with disjoint files.

**Non-Goals:**
- Physical iOS devices, tvOS, HarmonyOS, web, desktop.
- Video encoding on Android beyond what the emulator controller streams.
- MCP server, CDP-subset endpoint, decision-model hooks, agent-browser provider integration.
- React Native specific inspection. React Native apps are exercised only as fixtures through the accessibility layer.

## Decisions

### D1. Persistent host daemon with a thin CLI
A daemon holds device connections, replicas and sessions; the CLI is a client over a Unix socket that auto-starts the daemon. Alternative: one-shot CLI that connects per command. Rejected because attach and replica warm-up dominate latency and the whole point is amortizing them; agent-browser and agent-device both converged on the same shape.

### D2. Android agent as a single-dex Kotlin program started with `app_process`
The agent is compiled with `kotlinc` against the platform jar and dexed with `d8`, pushed to `/data/local/tmp`, and started through `adb shell` with `app_process` as the shell user, the way scrcpy's server starts. It obtains the UI automation connection through reflection on the hidden constructor and connects with the accessibility service client, then uses the tree, event listener, input injection and screenshot APIs. Alternatives: an instrumentation APK started with `am instrument` (rejected: install step, signing, test-only package handling on some OEMs, still one owner); an accessibility service app (rejected: user toggle, no screenshots without consent, real-time gesture playback); the existing UIAutomator2 server (rejected: HTTP and JSON per node, its own idle waits). Fallback to an instrumentation APK is deferred to a later change if an OEM build refuses the shell path. openatx's current client already runs a jar through `app_process` with the same reflective construction, so the novelty here is the resident binary protocol and the event mirror, not the launch path. Input acknowledgements use the hidden three-argument `injectInputEvent` overload (API 31+) or `InputManager.injectInputEvent` with `WAIT_FOR_RESULT`, whose return value is the accepted flag the spec asks for; the public synchronous overload waits for animations around touch-down and touch-up and would blow the 50 ms gate. Screenshots use only the public `UiAutomation.takeScreenshot()`, because the hidden screenshot plumbing changed in Android 16, 16 QPR2 and 17. The Kotlin standard library alone dexes to 2.28 MB, so the shipped agent is shrunk with R8 (task 4.10).

### D3. ADB through the local server protocol, no `adb` binary in the hot path
The driver speaks the ADB server protocol on port 5037 through a Rust client crate for shell, push and forward, and connects to the agent through a forwarded abstract socket. Alternatives: shelling out to `adb` (rejected: process spawn per operation); direct USB via the ADB device protocol (deferred: it conflicts with a running adb server over USB claims; keep as an option for hosts without an adb server). The agent listens on an abstract Unix socket and the handshake carries a per-launch token; it never opens a TCP port, unlike openatx's server on 0.0.0.0:9008. Whether adbd may connect to a shell-owned abstract socket through `adb forward` under Android 17's SELinux policy is unverified, so S1 and S6 test both `forward` and `reverse` (the agent connecting out to the host, as scrcpy does by default) and reverse stays as the fallback. Spike S6 measures shell round trip and push throughput for the adb-server client, direct USB and the `adb` binary before this decision is final; the numbers go here.

### D4. Wire format: CBOR is the candidate, the benchmark spike decides
Frames are a 32-bit big-endian length followed by one CBOR map. Message kinds: request with id, method and params; response with id and either result or error; event with stream, sequence and payload. Rust uses a small no-std CBOR codec; the Kotlin agent hand-rolls an encoder and decoder for the fixed message set, a few hundred lines with no dependency. Alternatives: protobuf (rejected: runtime and code generation in the dex); JSON over HTTP (rejected: per-node cost and no event channel); a Rust-only serde format (rejected: the agent is not Rust). Version and capability negotiation are the first exchange on every connection. Spike S5 benchmarks CBOR against MessagePack, protobuf-lite, FlatBuffers and a JSON baseline on a 500-node tree and on 10,000 small events, encoded and decoded in both Rust and Kotlin, measuring bytes on the wire, encode and decode time, and dex size added to the agent. The format becomes final only when those numbers are recorded here. The pass criterion is at least three times smaller and faster than compact JSON on the same logical fields, with under 100 KB added to the shrunk dex. The protocol research note predicts that no format reaches 3x on the tree dataset; if S5 confirms that, D4 records the measured ratios and decides on time, dex delta, schema evolution and debuggability rather than weakening the baseline. On the Kotlin side only hand-rolled codecs and platform JSON are measured, because the library codecs would need Maven downloads that Socket Firewall cannot wrap; the report lists them as not measured for that reason.

### D5. Mirror update strategy
Android: the agent forwards content-changed and window-changed events with the source node path; the mirror marks that subtree dirty and refreshes it with descendant prefetch, and performs a full walk only on window state changes or a sequence gap. Simulator: display surface callbacks are the change signal; the tree is re-read after frames change and once more when they stop, bounded by a minimum interval. Refs are assigned from a stable identity built from window, class, identifier and sibling path, and are reissued only when the identity disappears. Alternative: always re-read on request (rejected: that is the model being replaced). Costs the research note established, which S1 measures: prefetch is capped at 50 nodes per request, so a cold 500-node read is at least ten round trips; requests execute on the app's UI thread with a 5 s timeout; prefetch is skipped while a window scrolls; `refresh()` is one round trip per node; every window-state change empties the platform node cache. Content-change events are batched at about 100 ms, and agent-device found the node cache is not reliably invalidated by Compose Navigation 3 in-place swaps, so the mirror clears the platform cache and re-reads in full on a resync path that is also available on demand, and D8's quiet window is retuned from S1's measurements. Two facts from the Compose fixture shape the mirror's node model: Compose reports only nodes inside the viewport, so scroll-hidden hints come from scroll-range semantics rather than from off-screen nodes, and Material components keep the label and role on child nodes of the tagged clickable container, so an element's accessible name is computed from its own text or description and otherwise from its non-actionable descendants, the way accessibility services do.

### D6. Simulator control from Rust: host-side lifecycle, input and frames, a spawned in-guest reader for the tree
CoreSimulator (in `/Library/Developer/PrivateFrameworks`), SimulatorKit (`Xcode.app/Contents/SharedFrameworks`) and CoreSimDeviceIO are loaded with `dlopen` and driven through objc2 dynamic messaging with exception catching around every private call, which needs `panic = "unwind"`. The input transport is selected from the host's CoreSimulator version, as idb and iosef do: the dtuhidd XPC service (`com.apple.coredevice.feature.remote.hid.digitizer`, reached through hand-written bindings to the `_4sim` XPC functions resolved with dlsym) on CoreSimulator 1155.4 and later, which injects it into every guest, and the Indigo Mach messages through SimulatorKit's legacy HID client before that. Probing is forbidden because opening dtuhidd disables the Indigo services for the rest of the boot. The session sends dtuhidd's activation probe first, waits for the reply, queues input until then and keeps one connection open. The Indigo message builders keep unsynchronized global state, so all HID calls run on one serial queue. The accessibility tree cannot be read from a macOS process at useful speed: the fast reader used by agent-device links the Simulator runtime's private accessibility client, which is built for the iOS Simulator platform, and runs inside the guest through `simctl spawn`. This design therefore ports that reader to a small Rust binary built for the `aarch64-apple-ios-sim` target, spawned by the daemon into the booted Simulator with no installation and no code signing, speaking the device protocol over a Unix socket. The host-only translator path (`AXPTranslator` with `sendAccessibilityRequestAsync`) is measured in S4 as the fallback that needs nothing in the guest, at one XPC round trip per element and without SwiftUI container children or web content. Alternatives: shelling out to an existing Swift tool (rejected: process per command, no frames); an XCTest runner (rejected: install, build, idle waits); an Objective-C shim compiled into the binary (kept as an escape hatch for any single private call that proves impractical through objc2, isolated in one file). Spikes S2, S3 and S4 exercise each private call site from Rust first; a call site that proves impractical moves to the shim, and the outcome per call site is recorded in the compatibility table. DeviceHub.app stays closed during spikes: it attaches dtuhidd at boot, and quitting it shuts down every booted Simulator.

### D7. Frames
Simulator frames come from the display surface as BGRA, are downscaled and JPEG-encoded on a worker thread, and are also hashed at low resolution to feed the settle predicate. Android emulators stream frames from the gRPC controller. Android devices use screenshots on demand; a video path is a later change. Alternative: screenshots everywhere (rejected: no change signal, too slow for settle). The JPEG encoder is chosen from Spike S2's measured encode time per frame at the default dimension. The display surface is cached when its change callback fires rather than fetched per frame, because each fetch is an XPC round trip.

### D8. Settle semantics
Settled means no tree delta and no frame hash change for a quiet window, default 150 ms, bounded by a maximum, default 3 s. The result always reports elapsed time and, on timeout, which signal kept changing. This replaces platform idle waits, which the Android agent caps at a small bound and the Simulator path never invokes.

### D9. CLI grammar
The command set, ref syntax, interactive snapshot flag, JSON flag, batch mode and session flag mirror agent-browser where the concept exists on devices. Alternatives: Maestro YAML (rejected for this change: a flow runner can be layered later; agents act step by step); a new grammar (rejected: no transfer of existing skills).

### D10. Benchmark harness as a first-class crate
A Rust binary drives this project's CLI and, through adapters, the baseline tools, on flows described once. Fixtures are an Expo React Native app, a Compose app and a SwiftUI app checked into the repository with a deterministic 500-node screen each. Reports are JSON plus markdown. Gates are those in the benchmark spec.

### D11. Workspace layout and work partitioning
```
crates/protocol    message types, framing, negotiation           (no device code)
crates/mirror      replica, refs, settle, wait, diff, verdicts   (depends on protocol)
crates/android     adb client, agent deploy, agent session, emulator gRPC
crates/simulator   CoreSimulator bindings, HID, AX bridge, frames
crates/daemon      target registry, sessions, Unix socket API
crates/cli         thin client, output formatting
crates/bench       harness and baseline adapters
agent-android/     Kotlin sources, build script using kotlinc and d8
fixtures/          reference apps and flow descriptions
```
Each crate is a separately assignable unit with disjoint files. `protocol` and the agent's message set are written first because everything else depends on them.

### D12. Hidden and private API policy
All private Apple framework use and hidden Android API use is confined to `crates/simulator` and `agent-android`, behind traits defined in `crates/protocol` and `crates/mirror`. Every such call site records the Xcode or Android version it was validated on. On Android, hidden-API enforcement is off for processes started by `app_process` (ART enables it only for zygote-forked apps or with an explicit runtime flag) and the shell user is exempt from Android 14's writable-dex rule, so no bypass is planned; the real risk is OEM framework differences. The compatibility table cites `android-17.0.0_r1` (API 37); `UiAutomationConnection` ownership is unchanged from Android 14 to 17. A compatibility table is generated into the CLI's `doctor` output.

## Delivery process

These rules bind every task in tasks.md and every subagent that implements one.

- **Research first.** Each task group opens with a state-of-the-art note in `docs/research/` naming what exists, what is reused, where this design differs, and the sources. No implementation task starts before its group's note exists.
- **Spikes before construction.** The seven spikes S1 to S7 in tasks.md group 2 de-risk the assumptions this design rests on: shell-user ownership of Android UI automation, Rust control of the Simulator lifecycle and frames, both Simulator input transports, the Rust accessibility bridge, the wire format, the ADB transport and the emulator control endpoint. Each spike is throwaway code under `spikes/`, delivers `docs/spikes/<id>.md` with measurements and a go or no-go verdict, and gates the groups that depend on it. A failed spike changes this design; it is not patched around.
- **Measured decisions.** D3, D4, D6 and D7 are candidates until their spike records numbers here. Any later choice with measurable consequences follows the same rule.
- **Tests first.** Every implementation task adds failing tests, records the failure, then implements until they pass. A task without red-then-green evidence is not complete, and the validation phase checks that changed code is covered.
- **Atomic commits.** Each task lands as one or more atomic commits made by the implementer, one logical change per commit, message in `type(scope): summary` form, no attribution trailers.
- **Validation and fix after every task.** Before the next task starts: `cargo build --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`, `cargo test --workspace`, the agent and fixture builds where touched, `openspec validate`, and a diff review against the spec section the task implements. Failures are fixed and committed first.
- **Installs through Socket.** Every dependency install is prefixed with Socket Firewall: `sfw cargo add`, `sfw cargo fetch`, `sfw npm install`, `sfw pnpm add`. Bun is not documented as supported; the fixture task verifies `sfw bun` and falls back to `sfw npm` if it is not wrapped. No bare installs.
- **Recap before dispatch.** Whoever dispatches a task to an implementer restates its goal, the spec and design sections it implements, the files it owns, the acceptance checks and the constraints above.

## Risks / Trade-offs

- [Simulator private APIs change again in an Xcode beta] → transport probing at attach, a CI matrix over installed Xcode versions, and the Objective-C escape hatch so a fix is one file.
- [An OEM Android build blocks the shell-user automation path] → detect at deploy and report; instrumentation APK fallback scheduled as a follow-up change rather than built speculatively.
- [The only fast Simulator tree reader depends on a private framework built for the iOS Simulator platform and must run inside the guest] → keep the host-only translator path as the fallback, record per-Xcode validation for the spawned reader, and report readiness time in target status.
- [Only one UI automation owner per Android device] → the daemon is the single multiplexer; the platform never names the other holder (the error carries only a binder proxy id), so the conflict report infers a likely holder from the process and instrumentation listings; the role is released when the owning process dies; a force flag exists for recovery.
- [Event mirror misses in-place swaps and 100 ms event batching narrows the quiet window] → periodic and on-demand full re-read with cache clearing; quiet window retuned after S1.
- [Reflection on hidden Android members breaks on a new API level] → the agent verifies each member at startup and reports which capability is lost instead of crashing; the capability set is what the daemon negotiates on.
- [Frame hashing misreads video content as never settled] → settle reports the reason so agents can choose the tree-only signal; a tree-only mode is exposed.
- [Ref identity collides for visually identical siblings] → sibling index is part of the identity; collisions are detected and reported as a degraded verdict rather than silently merged.
- [Benchmark fixtures drift between tools] → flows are defined once and adapters are tested against a golden result before timing runs.

## Migration Plan

Greenfield. No deployment beyond building the workspace; nothing to roll back. The Android agent is versioned with the protocol and replaced on the device whenever the version differs.

## Open Questions

- Whether this CLI eventually becomes an agent-browser provider or stays standalone. It does not change this change's crates: the daemon API is the integration point either way.
- Default JPEG quality and maximum frame dimension for agents. Set from benchmark results, not decided up front.
- Whether to keep a direct-USB ADB path. Decided by whether hosts without an adb server appear in practice.
