# Workspace tasks. `just --list` describes each recipe; `just validate` runs the
# checklist that must pass before the next task starts.
#
# On Linux, build, lint and test cover the Simulator and iPhone drivers only as
# stubs that cannot open, without the Simulator's reader or either driver's
# integration tests (see crates/simulator/README.md and crates/ios/README.md).
# On macOS the iPhone driver's tests run on its fake device, and its device
# tests skip unless MOBILE_AGENT_IOS_UDID names a phone, so nothing here runs
# against one. agent-android's scripts and the Compose fixture default
# ANDROID_HOME to $HOME/Library/Android/sdk, the macOS location, so set it
# there. Only the Compose fixture builds on Linux: the SwiftUI fixture and the
# Expo fixture's default iOS build need Xcode, and the iOS runner and the
# fixtures' device builds Xcode and MOBILE_AGENT_IOS_TEAM, without which they
# are skipped. CI's Linux job runs `just validate` with FIXTURES=compose, and
# its macOS job with FIXTURES=expo,swiftui and SKIP_AGENT_BUILD=1, which skips
# the iOS runner's build as it skips the Android agent's.

[private]
default:
    @just --list

# Build every crate in the workspace
build:
    cargo build --workspace

# Run every test in the workspace
test:
    cargo test --workspace

# Run clippy on all targets with warnings denied
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format all Rust sources in place
fmt:
    cargo fmt --all

# Check Rust formatting without changing files
fmt-check:
    cargo fmt --all -- --check

# Check that no tracked file names a machine-local path, such as a home or scratch directory
check-paths:
    scripts/check-local-paths.sh

# Check the repository's own scripts, such as the reports' scrub, in a scratch directory without the network
scripts-test:
    scripts/test.sh

# Build the Android agent, skipping with SKIP_AGENT_BUILD=1 or when agent-android/build.sh is absent
agent-build:
    if [ "${SKIP_AGENT_BUILD:-}" = "1" ]; then echo "SKIP_AGENT_BUILD=1, skipping the Android agent build"; elif [ -f agent-android/build.sh ]; then agent-android/build.sh; else echo "agent-android/build.sh not present, skipping"; fi

# Build and sign the iOS runner into agent-ios/dist, skipping with SKIP_AGENT_BUILD=1, without MOBILE_AGENT_IOS_TEAM or without the iPhoneOS SDK
agent-ios-build:
    if [ "${SKIP_AGENT_BUILD:-}" = "1" ]; then echo "SKIP_AGENT_BUILD=1, skipping the iOS runner build"; elif [ -z "${MOBILE_AGENT_IOS_TEAM:-}" ]; then echo "MOBILE_AGENT_IOS_TEAM is not set, skipping the iOS runner build"; elif ! xcrun --sdk iphoneos --show-sdk-path >/dev/null 2>&1; then echo "the iPhoneOS SDK is not present, skipping the iOS runner build"; else agent-ios/build.sh; fi

# Cross-check the Kotlin codec against the Rust golden frames and run the agent's host-side unit tests on the host JVM, skipping without kotlinc
codec-test:
    if [ "${SKIP_AGENT_BUILD:-}" = "1" ]; then echo "SKIP_AGENT_BUILD=1, skipping the codec cross-check"; elif ! command -v kotlinc >/dev/null 2>&1; then echo "kotlinc not present, skipping the codec cross-check"; elif [ -f agent-android/codec-test.sh ]; then agent-android/codec-test.sh --host-only; else echo "agent-android/codec-test.sh not present, skipping"; fi

# Check the fixtures' build scripts with fake build tools, without Xcode, a device or the network
fixtures-test:
    fixtures/test.sh

# Build the reference fixture apps, or those FIXTURES names, and the iOS ones for a physical iPhone when MOBILE_AGENT_IOS_TEAM is set, skipping when fixtures/build.sh is absent
fixtures-build:
    if [ -f fixtures/build.sh ]; then fixtures/build.sh; else echo "fixtures/build.sh not present, skipping"; fi

# Run the benchmark harness in release mode, passing arguments through
[positional-arguments]
bench *args:
    cargo run -p bench --release -- "$@"

# Run the host microbenchmarks on a quiet host and compare them with reports/micro/baseline.json, passing arguments through
[positional-arguments]
micro *args:
    cargo run -p bench --release -- micro "$@"

# Compare the host microbenchmarks of the checkout with those of another ref, both built and run here in alternating rounds, passing further arguments through
[positional-arguments]
bench-ab base *args:
    cargo run -p bench --release -- micro --ab "$@"

# Validate every active OpenSpec change and the main specs in strict mode
openspec-validate:
    openspec validate --all --strict

# Run build, lint, fmt-check, check-paths, test, scripts-test, agent-build, agent-ios-build, codec-test, fixtures-test, fixtures-build and openspec-validate, stopping at the first failure
validate: build lint fmt-check check-paths test scripts-test agent-build agent-ios-build codec-test fixtures-test fixtures-build openspec-validate
