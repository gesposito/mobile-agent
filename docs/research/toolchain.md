# Rust toolchain: the pin and its cadence

Research note for task 10.8 of `mobile-automation-foundation`. It records what changed between the pinned Rust 1.95.0 and the current stable, 1.98.1, that matters to this workspace, and how the pin keeps up with new releases. Sources were read on 2026-09-27.

## Verdict

- **Pin 1.98.1**, not 1.98.0, with clippy, rustfmt and the `aarch64-apple-ios-sim` target as before: 1.98.0 could build a trait-object vtable with a null function pointer, which 1.98.1 fixes [rel-1981]. An exact pin keeps local builds and CI on one compiler and makes new lints arrive in a change of their own. While CI floated on `stable`, the 1.98 release failed its lint step with no code change (run 36303704593).
- **Keep `rust-version = "1.95"`.** It is the MSRV consumers see, resolver 3 prefers dependency versions that support it, and clippy takes it as its own MSRV, so lints that would suggest newer APIs stay quiet. `as_chunks`, the only newer API the fixes use, is stable since 1.88.
- **Dependabot keeps the pin current.** Its `rust-toolchain` ecosystem, checked weekly on Fridays (`.github/dependabot.yml`), opens a pull request when a new stable ships [dep-cl], and its jobs cost no Actions minutes [dep-actions]. 1.99.0 is due on 2026-10-01.

## What changed from 1.95.0 to 1.98.1

Only what touches this workspace; the rest is in the release notes [relnotes] and the clippy changelog [clippy-cl].

- **Clippy 1.98, `chunks_exact_to_as_chunks`** (style, warn): `chunks_exact` or `chunks_exact_mut` with a constant width on a slice should be `as_chunks::<N>()`, which yields arrays and returns the remainder as the second half of a pair; the lint is gated on an MSRV of 1.88 [lint][lint-src]. The run above stopped at the first crates it failed, the `simulator` library (seven sites in `pipeline.rs`) and the `android` library (one in `session/controller.rs`), so test targets and dependent crates were never linted. On every target, 1.98.1 flags 13 sites: those eight, three in `pipeline.rs`'s tests and two in `crates/android/tests/emulator_controller.rs`. Only `pipeline::pixels_hash` uses a remainder, which a test now pins; everywhere else the lengths are whole pixels.
- **Clippy 1.97, `byte_char_slices` on arrays** [clippy-16770]: one test helper's `[b'A', ..., b'F']` became `b"ABCDEF"`.
- **Other new default-on clippy lints**, none of which fire here: `manual_noop_waker`, `manual_option_zip`, `manual_pop_if` (1.96); `manual_clear`, `useless_borrows_in_formatting` (1.97); `unnecessary_unwrap_unchecked`, `manual_isolate_lowest_one`, `by_ref_peekable_peek`, `for_unbounded_range` (1.98). 1.97 moved `nonminimal_bool` and `overly_complex_bool_expr` to pedantic and 1.98 moved `empty_enums` to nursery, so those warn less. The reader linted for `aarch64-apple-ios-sim` and `agent-android/host-tests` have no findings.
- **rustc lints**: from 1.97 `linker_messages` warns by default, filtering the known macOS `ld` noise and ignoring `-D warnings` [rust-153968]; clippy does not link, so `just lint` cannot fail on it. 1.98 adds `c_void_returns`, `suspicious_runtime_symbol_definitions` and the deny-by-default `invalid_runtime_symbol_definitions`; nothing here returns `c_void` or defines C runtime symbols. 1.97 deprecates the `std::char` module's constants and functions, which nothing here uses.
- **Codegen**: 1.97 mangles symbols with the v0 scheme by default [rust-151994], which changes names in profiles and crash reports, not behaviour.
- **Language**: 1.98's stricter `repr(transparent)` concerns additional zero-sized fields; `AXUIElementRef` and `IndigoMessage` have one field each.
- **Cargo**: nothing changes `rust-version` semantics on stable [cargo-cl]. 1.97 stabilizes `build.warnings`, an alternative to `-- -D warnings` that `just lint` does not need.
- **Platforms**: no Apple target changes, so the macOS host and the `aarch64-apple-ios-sim` reader build as before.

## The rewrite, measured

The old `chunks_exact` code built by 1.95.0 against the new `as_chunks` code built by 1.98.1, both release builds of the ignored measurements named under [A bump](#a-bump): 200 runs after 10 warm-up runs, two alternating rounds, on an M1 Max under macOS 26.6.2 at a one-minute load average of 3.4, then 3.3. The frame pipeline is the Simulator driver's work per frame: the downscale to 1000 pixels, the pixels hash and the content hash. Times are p50 / p95 in ms, round 1 then round 2.

| Work | 1.95.0, `chunks_exact` | 1.98.1, `as_chunks` |
|---|---|---|
| Frame pipeline, 1206x2622 to 460x1000 | 1.338 / 1.462, 1.325 / 1.389 | 1.316 / 1.412, 1.316 / 1.391 |
| Frame pipeline, 1179x2556 to 461x1000 | 1.304 / 1.399, 1.300 / 1.347 | 1.287 / 1.358, 1.289 / 1.370 |
| `resize_rgb`, 1080x2400 to 450x1000 | 4.986 / 5.389, 4.907 / 5.083 | 4.845 / 5.412, 4.795 / 6.030 |

The new code is 0.7 to 2.8 % faster at p50, and every checksum the measurements print matched between the builds, so they computed the same frames.

## Keeping the pin current

| Mechanism | Runs on | Cost here | Verdict |
|---|---|---|---|
| Dependabot, `rust-toolchain` ecosystem | GitHub, on Actions runners outside the minutes | one config file | chosen |
| Renovate, `rust-toolchain` manager | Mend's hosted app, or self-hosted on Actions | a third-party app with write access, or minutes | rejected |
| Scheduled workflow comparing the pin with the stable manifest | Actions, Linux | a script, and a billed minute a run | rejected |
| `just` recipe around `rustup check` | a developer's machine | nothing, and no schedule | rejected |
| Floating `stable` | everywhere | CI fails on a release with no code change | rejected in task 10.6 |

- Dependabot has updated `rust-toolchain.toml` and `rust-toolchain` files since 2025-08-19, `channel = "1.xx.yy"` pins included [dep-cl][dep-eco]; livekit/rust-sdks took 1.97.1 to 1.98.1 that way on 2026-09-08 [dep-livekit]. Its jobs run on Actions runners, bypass repository-level disablement, and "do not count towards your included GitHub Actions minutes" [dep-actions]. Whether they run while the account's Actions spending limit is exhausted is not documented, so the first bump pull request is the test. `cooldown` is documented for the ecosystem [dep-opts], but its pull requests report "Cooldown could not be applied because no publication date was available from the registry" [dep-cooldown], so waiting for a point release is the reviewer's call. The pull request runs CI like any other.
- Dependabot watches the toolchain only: crate updates would install dependencies around Socket Firewall (design.md, Delivery process). The toolchain itself comes from static.rust-lang.org, which sfw does not wrap, so rustup installs it unwrapped either way.
- Renovate has an equivalent `rust-toolchain` manager with a `rust-version` datasource [ren-mgr], but it needs Mend's GitHub App, a third party with write access to this private repository, or a self-hosted run on Actions, and configuration to keep it to one file.
- A scheduled workflow would read the pin, fetch `channel-rust-stable.toml` and compare: a script to maintain, and a billed minute a run because GitHub rounds each job up [gh-billing], and no run at all while Actions is blocked. Its failures notify whoever last edited the cron [gh-schedule].
- `rustup check` reports whether installed channels have updates, not whether the pin is behind, and a recipe runs only when someone remembers it.

## A bump

1. `rustup toolchain install <version> --profile minimal --component clippy --component rustfmt --target aarch64-apple-ios-sim --no-self-update`.
2. Change `channel` in `rust-toolchain.toml` and fix what the new lints find, preferring the fix to an `#[allow]`, in three places: `just lint`; the reader for its own target, which `just lint` checks only for the host, with `cargo clippy --release -p simulator-reader --target aarch64-apple-ios-sim --all-targets -- -D warnings`; and `agent-android/host-tests`, its own workspace, with `cargo clippy --manifest-path agent-android/host-tests/Cargo.toml --all-targets -- -D warnings`.
3. When pixel code changes, time it under the old and the new toolchain with the ignored measurements `pipeline::tests::measure_the_frame_pipeline` (`-p simulator --no-default-features --features jpeg-encoder`) and `session::controller::tests::measure_the_rgb_resize` (`-p android`), each run as `cargo +<version> test --release --lib <test> -- --ignored --nocapture` on a quiet machine.
4. `just validate`, then CI.

## Sources

- [rel-1981] https://blog.rust-lang.org/2026/09/03/Rust-1.98.1/ and https://github.com/rust-lang/rust/issues/161441
- [relnotes] https://github.com/rust-lang/rust/blob/1.98.1/RELEASES.md (1.96.0 to 1.98.1)
- [clippy-cl] https://github.com/rust-lang/rust-clippy/blob/rust-1.98.0/CHANGELOG.md (Rust 1.96 to 1.98)
- [lint] https://rust-lang.github.io/rust-clippy/rust-1.98.0/index.html#chunks_exact_to_as_chunks and https://github.com/rust-lang/rust-clippy/pull/16931
- [lint-src] https://github.com/rust-lang/rust-clippy/blob/rust-1.98.0/clippy_lints/src/methods/chunks_exact_to_as_chunks.rs (`msrvs::AS_CHUNKS`, 1.88.0)
- [clippy-16770] https://github.com/rust-lang/rust-clippy/pull/16770
- [rust-153968] https://github.com/rust-lang/rust/pull/153968
- [rust-151994] https://github.com/rust-lang/rust/pull/151994
- [cargo-cl] https://doc.rust-lang.org/nightly/cargo/CHANGELOG.html (Cargo 1.96 to 1.98)
- [dep-cl] https://github.blog/changelog/2025-08-19-dependabot-now-supports-rust-toolchain-updates/
- [dep-eco] https://docs.github.com/en/code-security/reference/supply-chain-security/supported-ecosystems-and-repositories
- [dep-opts] https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference (`cooldown`)
- [dep-actions] https://docs.github.com/en/code-security/concepts/supply-chain-security/dependabot-on-actions
- [dep-livekit] https://github.com/livekit/rust-sdks/pull/1412
- [dep-cooldown] https://github.com/leynos/statelet/pull/77
- [ren-mgr] https://docs.renovatebot.com/modules/manager/rust-toolchain/
- [gh-billing] https://docs.github.com/en/billing/reference/actions-runner-pricing
- [gh-schedule] https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule
