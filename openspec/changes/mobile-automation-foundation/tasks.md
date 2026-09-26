# Tasks

Rules that bind every task below (see design.md, Delivery process): a research note precedes each group; implementation tasks add failing tests first and record red-then-green; every install is prefixed with `sfw`; each task lands as atomic commits by its implementer; a validation-and-fix phase (build, clippy with warnings denied, fmt check, full tests, `openspec validate`, diff review against the spec) runs before the next task starts. Groups 4 through 9 map to separate crates or source trees so they can be assigned to separate implementers with disjoint files. Group 1 first, group 2 spikes before anything they gate, group 3 before 4 through 8, group 10 last.

## 1. Process and workspace

- [ ] 1.1 Install Socket Firewall with `npm i -g sfw` and verify `sfw --version` runs and `sfw cargo fetch` completes in an empty crate
- [ ] 1.2 Create the Cargo workspace with empty crates protocol, mirror, android, simulator, daemon, cli and bench, plus `spikes/` and `docs/research/` and `docs/spikes/` directories; verify `cargo build --workspace` and `cargo clippy --workspace -- -D warnings` pass
- [ ] 1.3 Add a task runner file with build, test, agent-build, fixtures-build, bench and validate recipes, where validate runs the full validation-and-fix checklist; verify every recipe runs locally and `just validate` exits zero on the empty workspace
- [ ] 1.4 Add a CI workflow matrix (macOS per installed Xcode, Linux for Android) that runs the validate recipe; verify the workflow file passes `actionlint`
- [ ] 1.5 Write the Android agent build script using `kotlinc` and `d8` from the SDK to produce a single dex jar in `agent-android/dist`; verify the jar starts under `app_process` on an emulator and prints its version banner
- [ ] 1.6 Scaffold the reference fixtures through Socket: an Expo React Native app via the official init (verify `sfw bun` wraps it, else use `sfw npm`), a Compose app and a SwiftUI app, each with a login screen, a scrolling list and a deterministic 500-node screen; verify each builds and the 500-node screen renders that count in the platform inspector

## 2. Research and de-risking spikes

- [ ] 2.1 Write `docs/research/android-agent.md` covering scrcpy's server, the UIAutomator2 server, agent-device's snapshot and IME helpers and AOSP's `uiautomator` command: what each does, what to reuse, where this design differs, with sources; verify the note is reviewed and linked from design.md
- [ ] 2.2 Write `docs/research/simulator.md` covering idb and FBSimulatorControl, baguette, iosef, stim, AXe and agent-device's snapshot bridge for lifecycle, HID, accessibility and frames; verify the note is reviewed and linked from design.md
- [ ] 2.3 Write `docs/research/protocol.md` covering agent-browser's daemon protocol, scrcpy's control protocol, WebDriverAgent's API and CDP's framing and event model; verify the note is reviewed and linked from design.md
- [ ] 2.4 Spike S1: a throwaway shell-user `app_process` program that obtains UI automation ownership, reads the 500-node fixture tree, injects a tap and takes a screenshot on an API 36 emulator and on a physical device; deliver `docs/spikes/s1-android-ownership.md` with tree read and tap latencies and a go or no-go
- [ ] 2.5 Spike S2: a throwaway Rust program that boots a Simulator, launches the SwiftUI fixture and captures display surface frames, encoding JPEG with two candidate encoders; deliver `docs/spikes/s2-simulator-frames.md` with fps, CPU and encode time per frame and a go or no-go
- [ ] 2.6 Spike S3: a throwaway Rust program that delivers a tap through the Mach message transport on an iOS 26 runtime and through the XPC transport on an Xcode 27 runtime; deliver `docs/spikes/s3-simulator-hid.md` with tap latency per transport and a go or no-go
- [ ] 2.7 Spike S4: a throwaway Rust accessibility bridge reading the foreground tree of the SwiftUI and React Native fixtures, compared with an XCTest-based read of the same screens; deliver `docs/spikes/s4-simulator-ax.md` with p50 and p95 per screen and a go or no-go
- [ ] 2.8 Spike S5: benchmark CBOR, MessagePack, protobuf-lite, FlatBuffers and JSON on the 500-node tree and 10,000 small events in Rust and Kotlin, measuring bytes, encode and decode time and dex size delta; deliver `docs/spikes/s5-wire-format.md` and rewrite design.md D4 with the numbers and the chosen format
- [ ] 2.9 Spike S6: benchmark shell round trip and push throughput for the adb-server client crate, direct USB and the `adb` binary; deliver `docs/spikes/s6-adb-transport.md` and rewrite design.md D3 with the numbers
- [ ] 2.10 Spike S7: verify the emulator control endpoint with token discovery, measure `StreamScreenshot` fps and `SendTouch` latency; deliver `docs/spikes/s7-emulator-grpc.md` with a go or no-go
- [ ] 2.11 Spike review: consolidate verdicts, update design.md decisions D3, D4, D6 and D7 with measurements, adjust groups 3 to 9 for any no-go; verify `openspec validate` passes and every measured decision cites its spike

## 3. Protocol crate and agent message set

- [ ] 3.1 Add failing tests that round-trip every message kind (request, response, event, error with retryable flag, capability set, version list) in the format chosen by S5, then implement the schema in `crates/protocol`; verify red-then-green
- [ ] 3.2 Add failing tests for the length-prefixed framing codec, including an oversized frame producing an explicit error and partial frames being buffered, then implement it; verify red-then-green
- [ ] 3.3 Add failing tests for the handshake and capability negotiation covering common version, no common version and refusal of an unadvertised capability, then implement the state machine; verify red-then-green
- [ ] 3.4 Add a failing test that an event sequence gap raises a resync signal, then implement per-stream sequence tracking; verify red-then-green
- [ ] 3.5 Add golden byte fixtures produced by the Rust crate and failing Kotlin tests that decode them, then hand-write the Kotlin encoder and decoder for the same message set in `agent-android`; verify Kotlin output also decodes in Rust and both directions are red-then-green

## 4. Android agent (Kotlin, `agent-android`)

- [ ] 4.1 Add a failing Rust test client that expects a handshake from the agent, then implement the `app_process` entry point, argument parsing and abstract-socket server; verify the handshake completes on an emulator
- [ ] 4.2 Add failing tests for ownership attach and for a conflict error while `uiautomator dump` runs, then implement UI automation ownership with conflict detection and a force option; verify red-then-green on an emulator
- [ ] 4.3 Add failing tests that the 500-node fixture returns its full count with truncation flags at lower caps and that the keyboard window appears when a field is focused, then implement tree serialization across interactive windows; verify red-then-green
- [ ] 4.4 Add a failing test that a snapshot on the animation fixture returns within the idle bound plus read time, then implement the bounded idle wait with a default under one second and zero allowed; verify red-then-green
- [ ] 4.5 Add a failing test that editing one field emits an event naming that subtree only, then implement change-event forwarding with subtree paths and sequence numbers; verify red-then-green
- [ ] 4.6 Add failing tests for tap, long press, swipe, gesture and key injection acknowledgements under 50 ms on an emulator, then implement injection; verify red-then-green
- [ ] 4.7 Add failing tests for direct text setting on the React Native input fixture and for the key-event fallback with method reporting, then implement both; verify red-then-green
- [ ] 4.8 Add a failing landscape screenshot test, then implement screenshot capture with orientation; verify red-then-green
- [ ] 4.9 Add failing tests that no agent process remains after daemon shutdown and that snapshots work after the app is force-stopped and relaunched, then implement stop, clean exit and survival; verify red-then-green

## 5. Android host driver (`crates/android`)

- [ ] 5.1 Add failing integration tests for device listing with unauthorized and offline states, push, shell and forward against an emulator, then implement the ADB transport chosen by S6; verify red-then-green
- [ ] 5.2 Add failing tests for first attach, compatible reuse and incompatible replacement, then implement agent deploy, reuse and replace with version checks; verify red-then-green
- [ ] 5.3 Add a failing full-stack snapshot test, then implement the agent session driving handshake, requests and events into the mirror; verify red-then-green
- [ ] 5.4 Add failing tests that frames arrive and taps land with the control flag and that a warning is emitted without it, then implement the emulator gRPC client from the controller proto with token discovery; verify red-then-green
- [ ] 5.5 Add failing tests that `status` names the active path in both emulator configurations, then implement path selection and status reporting; verify red-then-green

## 6. Simulator driver (`crates/simulator`)

- [ ] 6.1 Add a failing integration test that boots a Simulator and launches the SwiftUI fixture with a custom environment variable visible, then implement CoreSimulator lifecycle bindings from S2; verify red-then-green
- [ ] 6.2 Add failing tests that frames arrive during a transition, stop when static and sustain 30 fps under 20 percent of one core, then implement frame capture with the encoder chosen in S2; verify red-then-green
- [ ] 6.3 Add failing tests that a tap lands on the fixture button and a swipe scrolls the list on an iOS 26 runtime, then implement the Mach message input transport from S3; verify red-then-green
- [ ] 6.4 Add a failing test that a tap lands on an Xcode 27 runtime, then implement the XPC input transport from S3; verify red-then-green
- [ ] 6.5 Add failing tests for a mixed-runtime session and for the unsupported-combination diagnostic, then implement transport probing and negotiation; verify red-then-green
- [ ] 6.6 Add failing tests that the SwiftUI fixture read returns identifiers and frames and that a permission alert is reported as system-owned, then port the accessibility bridge from S4 with degraded verdicts; verify red-then-green
- [ ] 6.7 Add a failing test that a 500 character string appears in the focused field with the method reported, then implement keyboard and paste text entry; verify red-then-green
- [ ] 6.8 Add a failing test that an injected framework failure marks the target unavailable while another Simulator keeps working, then wrap every private call with exception containment; verify red-then-green
- [ ] 6.9 Add a failing test for the refusal message outside the supported Xcode range, then implement the version gate and the `doctor` compatibility table; verify red-then-green

## 7. Mirror (`crates/mirror`)

- [ ] 7.1 Add failing tests replaying recorded event fixtures to expected trees, then implement the replica with full acquisition and delta application; verify red-then-green
- [ ] 7.2 Add failing tests for a surviving element, a stale ref and a sibling collision marked degraded, then implement ref identity and stability; verify red-then-green
- [ ] 7.3 Add a failing test on the scrolling-list fixture expecting visible buttons and a hint of three below the fold, then implement interactive filtering and off-screen hints; verify red-then-green
- [ ] 7.4 Add failing tests for a settling transition and for a continuous animation returning the frame-change reason, then implement the settle predicate over tree deltas and frame hashes; verify red-then-green
- [ ] 7.5 Add failing tests that appear, disappear and enabled waits resolve within one processing cycle and time out with the last snapshot summary, then implement them from the event stream; verify red-then-green
- [ ] 7.6 Add a failing diff test on the form-submit fixture, then implement the structural diff; verify red-then-green
- [ ] 7.7 Add failing tests that refs from before a resync are stale and the verdict names the fallback backend, then implement quality verdicts and resync; verify red-then-green
- [ ] 7.8 Add a failing scaled-frame test with age reporting, then implement frame access; verify red-then-green

## 8. Daemon and CLI (`crates/daemon`, `crates/cli`)

- [ ] 8.1 Add failing integration tests for a claimed target refusing another session and for idle exit, then implement the daemon with Unix socket API, target registry and session claims with force; verify red-then-green
- [ ] 8.2 Add failing tests for interactive snapshot output and for a JSON error with code, then implement the CLI commands from the agent-cli spec; verify red-then-green
- [ ] 8.3 Add a failing `hyperfine` gate that `status` overhead against a running daemon is under 20 ms, then implement daemon auto-start and trim CLI startup; verify red-then-green
- [ ] 8.4 Add a failing test where the second of three batch commands fails and the third does not run, then implement batch execution with stop-on-error and aggregate JSON; verify red-then-green
- [ ] 8.5 Add a failing test with two fake targets expecting the ambiguity error listing both, then implement target selection flags; verify red-then-green
- [ ] 8.6 Add failing tests that a 2 s wait fails at 2 s and that the agent guide is under 120 lines, then implement timeouts, per-command help and the guide; verify red-then-green
- [ ] 8.7 Add failing tests for each `doctor` check, then implement `doctor` covering toolchains, ADB server, emulator flag hints and the Xcode table; verify red-then-green and the output on this machine lists every check with a status

## 9. Benchmark harness (`crates/bench`)

- [ ] 9.1 Add failing tests that a run writes JSON and markdown agreeing on every number with provenance recorded, then implement primitive measurement with warm-up exclusion; verify red-then-green
- [ ] 9.2 Add a failing test forcing one gate to fail and expecting the table and non-zero exit, then implement gate configuration with the spec defaults; verify red-then-green
- [ ] 9.3 Add a failing test that an absent tool is noted as skipped, then implement baseline adapters for Maestro, maestro-runner and agent-device; verify red-then-green and one baseline run
- [ ] 9.4 Add failing golden end-state tests for the login, list scroll and 500-node flows across adapters, then write the flow descriptions and adapter execution; verify red-then-green
- [ ] 9.5 Add a failing test with two injected failures expecting the reported rate, then implement failure and flake rates; verify red-then-green

## 10. Integration verification

- [ ] 10.1 Run the full harness on an Android emulator with the control flag on and off; verify all Android gates pass and commit the reports under `reports/`
- [ ] 10.2 Run the full harness on iOS Simulators for every installed runtime; verify all Simulator gates pass and commit the reports under `reports/`
- [ ] 10.3 Run the harness on a physical Android device; verify tap acknowledgement and warm snapshot gates and commit the report
- [ ] 10.4 Smoke-test a Bluesky-class React Native app on Simulator and emulator; verify snapshots are not degraded on screens where an XCTest-based tool reports sparse roots, and record the finding in `reports/`
- [ ] 10.5 Write the README with the supported Xcode and Android ranges, the results tables and the agent guide; verify a new user can run `doctor`, `devices`, `open` and `snapshot` from the README alone
