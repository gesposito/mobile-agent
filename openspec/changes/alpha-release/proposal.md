# Proposal

## Why

Nothing of mobile-agent is published: the README says to build from source, and the four passes on real apps, the benchmarks on CI and the site all ran binaries built here. The repository has been public since 2026-10-30 with the licence, the notices, the security policy, the hardened workflows and a known-issues section, so what a first release lacks is the mechanics: a workflow that builds and attests the three binaries and publishes an immutable pre-release, the settings the maintaining checklist reserves for the first release, a version and the words around it, and, for `cargo install`, crates whose names can be published. The user decided on both: a tagged alpha with binaries on GitHub Releases first, then the crates on crates.io.

## What Changes

- A release workflow: on a dispatch naming the version, with a dry-run mode that only uploads artifacts, the three binaries built on the macOS and Linux runners, attested with build provenance, and published as an immutable pre-release with the third-party notices beside them; the publishing job alone in a `release` environment with a required reviewer; the tag created by the workflow, which the tag ruleset lets through and nothing else.
- The version `0.1.0-alpha.1`: the changelog's unreleased section becomes it, the README gains Install beside Build, `--version` prints the commit, the alpha's scope and the experimental label are stated where a user reads first.
- The alpha published and verified: the release attested, a clean-machine install on a Mac with Xcode, the record.
- The crates renamed with a `mobile-agent-` prefix as packages while their library names stay, publishable in dependency order; the first publish by hand, trusted publishing configured per crate after it, the publish job added to the release workflow; `cargo install mobile-agent` verified.
- The archive, with the memory updated.

## Capabilities

### Modified Capabilities

None: the change adds workflows, settings, metadata and documentation; no reply, listing or code changes. `skip_specs` is set.

## Impact

- `.github/workflows/release.yml` (new), `docs/maintaining.md` (the release items ticked), `docs/research/alpha-release.md` (new: the prior art), `README.md` (Install, the alpha's words), `CHANGELOG.md`, every `Cargo.toml` (`package.name` with the prefix, `[lib] name` kept, `version`), `crates/cli` (`--version` with the commit), `Cargo.lock`, `THIRD-PARTY-NOTICES.md` if a dependency moves.
- Repository settings (the maintainer's, through the API where it allows and the web form where it does not): release immutability, the `release` environment, the tag ruleset's bypass. crates.io: the trusted publisher per crate (the maintainer's account).
