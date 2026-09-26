# Workspace tasks. `just --list` describes each recipe; `just validate` runs the
# checklist that must pass before the next task starts.

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

# Build the Android agent, skipping when agent-android/build.sh is absent
agent-build:
    if [ -f agent-android/build.sh ]; then agent-android/build.sh; else echo "agent-android/build.sh not present, skipping"; fi

# Build the reference fixture apps, skipping when fixtures/build.sh is absent
fixtures-build:
    if [ -f fixtures/build.sh ]; then fixtures/build.sh; else echo "fixtures/build.sh not present, skipping"; fi

# Run the benchmark harness in release mode, passing arguments through
[positional-arguments]
bench *args:
    cargo run -p bench --release -- "$@"

# Validate the OpenSpec change in strict mode
openspec-validate:
    openspec validate mobile-automation-foundation --strict

# Run build, lint, fmt-check, test, agent-build, fixtures-build and openspec-validate, stopping at the first failure
validate: build lint fmt-check test agent-build fixtures-build openspec-validate
