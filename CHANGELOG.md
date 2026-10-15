# Changelog

The changes to mobile-agent, newest first. Nothing is released yet: each entry is an OpenSpec change, dated by its archive under `openspec/changes/archive/`, where its proposal, design, spec deltas and task records are. A first release will take the Unreleased section as its version, numbered by Semantic Versioning.

## Unreleased

- **open-sourcing** (`openspec/changes/open-sourcing/`): the repository prepared for publication under the MIT License. The reports describe their host by chip, cores, memory and macOS version and keep their samplers' loads and CPU shares without process names; `just check-public` refuses machine paths, device identifiers, serials, team ids, emails, host models and private terms in tracked files; the spikes' programs leave the repository, their reports staying in `docs/spikes/`; the workflows start without permissions, pin every action to a commit and keep no credentials, with an audit job running actionlint, zizmor and cargo-deny; Dependabot with a 7-day cooldown, `deny.toml`, `THIRD-PARTY-NOTICES.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, issue and pull request templates and `docs/maintaining.md`.
- **ci-benchmarks** (`openspec/changes/ci-benchmarks/`): `.github/workflows/benchmarks.yml` on a schedule or a dispatch, never per push: the microbenchmarks as an A/B of two refs on one runner (`micro --ab`), the harness's reduced plan judged by ratios against the baselines (`run --plan ci --gates none`), the tables in the job summary, the reports kept as artifacts and the trend against the previous scheduled run.

## 2026-10-15: rn-app-fixes

What a production React Native app showed on an iOS 26 Simulator, fixed for every React Native app: on iOS the mirror cuts what a later sibling drawn over an earlier one covers (fixed footers, floating buttons, sticky headers); an action on an element out of view fails with a code instead of tapping stale bounds; `fill` reads a value back through the field's descendants and reports an entry it cannot read as unverified; text typed into a field the action just focused waits for the keyboard; tabs get refs whether selected or not, checkboxes are listed as such, and the full listing loses its duplicates; a wait whose condition already holds answers at once. Checked over seven rounds of production-app runs (`openspec/changes/archive/2026-10-15-rn-app-fixes/`).

## 2026-10-14: physical-ios

Physical iPhones on USB: the `crates/ios` driver, with discovery and readiness checks, the CoreDevice tunnel in the daemon's own process without root, the app lifecycle, input through the developer disk image's `dtuhidd` from iOS 18, screenshots polled as frames, and an XCTest runner in Rust (`agent-ios/`) that serves the tree and text entry over the device protocol; `MOBILE_AGENT_DRIVERS=ios`, `devices` and `doctor` with the iPhone's readiness, and the harness's `ios-device` target class, measured on an iPhone SE (3rd generation) on iOS 26.6.2 (`reports/11.1-ios-device.md`; spikes S9 to S11).

## 2026-10-08: microbenchmarks

Host microbenchmarks of the codec and framing, the mirror, the daemon's request paths, the Simulator's frame pipeline and the Android paths, under criterion 0.8.2 as spike S8 chose; `just micro` measures only on a quiet host and compares with the committed `reports/micro/baseline.json` at a 5% threshold, allocations exactly.

## 2026-10-07: mobile-automation-foundation

The foundation: a Rust workspace with a persistent daemon and a thin CLI that follows agent-browser's grammar, refs and JSON envelope; the device protocol in CBOR with version and capability negotiation; the live UI mirror that answers snapshots, waits and settle checks from a replica kept current by events and frames; the Android driver with its single-dex agent started through `app_process` and the emulator's gRPC fast path; the iOS Simulator driver entirely from the host; and the benchmark harness against Maestro, agent-device and maestro-runner, with its gates (spikes S1 to S7; `reports/10.*`).
