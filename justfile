# Workspace tasks. `just --list` describes each recipe; `just validate` runs the
# checklist that must pass before the next task starts.
#
# On Linux, build, lint and test cover the Simulator driver only as a stub that
# cannot open, without its reader or its integration tests (see
# crates/simulator/README.md). agent-android's scripts and the Compose fixture
# default ANDROID_HOME to $HOME/Library/Android/sdk, the macOS location, so set
# it there. Only the Compose fixture builds on Linux: the SwiftUI fixture and
# the Expo fixture's default iOS build need Xcode. CI's Linux job runs
# `just validate` with FIXTURES=compose, and its macOS job with
# FIXTURES=expo,swiftui and SKIP_AGENT_BUILD=1.

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

# Build the Android agent, skipping when agent-android/build.sh is absent
agent-build:
    if [ "${SKIP_AGENT_BUILD:-}" = "1" ]; then echo "SKIP_AGENT_BUILD=1, skipping the Android agent build"; elif [ -f agent-android/build.sh ]; then agent-android/build.sh; else echo "agent-android/build.sh not present, skipping"; fi

# Cross-check the Kotlin codec against the Rust golden frames and run the agent's host-side unit tests on the host JVM, skipping without kotlinc
codec-test:
    if [ "${SKIP_AGENT_BUILD:-}" = "1" ]; then echo "SKIP_AGENT_BUILD=1, skipping the codec cross-check"; elif ! command -v kotlinc >/dev/null 2>&1; then echo "kotlinc not present, skipping the codec cross-check"; elif [ -f agent-android/codec-test.sh ]; then agent-android/codec-test.sh --host-only; else echo "agent-android/codec-test.sh not present, skipping"; fi

# Build the reference fixture apps, or those FIXTURES names, skipping when fixtures/build.sh is absent
fixtures-build:
    if [ -f fixtures/build.sh ]; then fixtures/build.sh; else echo "fixtures/build.sh not present, skipping"; fi

# Run the benchmark harness in release mode, passing arguments through
[positional-arguments]
bench *args:
    cargo run -p bench --release -- "$@"

# Validate the OpenSpec change in strict mode
openspec-validate:
    openspec validate mobile-automation-foundation --strict

# Run build, lint, fmt-check, check-paths, test, agent-build, codec-test, fixtures-build and openspec-validate, stopping at the first failure
validate: build lint fmt-check check-paths test agent-build codec-test fixtures-build openspec-validate
