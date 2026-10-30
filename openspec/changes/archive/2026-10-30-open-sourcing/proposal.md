# Proposal

## Why

The repository is private and was written as a lab notebook: task records that log red-then-green counts and phone visits, throwaway spike programs, raw measurement data with the host's model, its security software and its process names, and a history that for five commits carried a device identifier prefix and machine-local paths before they were scrubbed from the tree (`scripts/check-local-paths.sh` was written that day). Nothing in the tree names a person, an email, a team, a device or another product, but the history does, every commit is authored with the maintainer's email, and the earlier history rewrite lives only on this clone. CI is validation only and, measured by zizmor, pins no action to a commit, keeps credentials in every checkout and installs one tool outside a lockfile. There is no license, no contribution guide, no security policy and no crate metadata.

The maintainer wants the repository public without leaking personal, private or internal data, without publishing details that mean nothing outside this notebook, with CI and the repository hardened as Astral's "Open source security at Astral" (2026-04-08) prescribes, and with the usual open-source practices in place. Decisions taken: MIT; the full history rewritten rather than squashed, the author kept; this repository kept and made public; the material that leaves the tree moved to a local folder outside any repository for reference.

## What Changes

- A research note on open-sourcing a Rust project: Astral's practices, OpenSSF Scorecard and the best-practices badge, GitHub's community standards and its guidance on removing sensitive data, cargo-deny and third-party notices, Dependabot cooldowns.
- The tree scrubbed of internal detail: the reports' and tests' provenance say the host's class, not its model or its security product; raw samplers keep loads and CPU shares without process names; a scan script refuses the terms and patterns that must never be tracked, run by `just validate` with the path check.
- Material that is not for the public moved to a local folder outside the repository: `spikes/` (its reports in `docs/spikes/` stay, as does everything under `openspec/`, the archived changes' `tasks.md` records included, which the maintainer chose to keep).
- The history rewritten once with `git filter-repo`: the five known leaks replaced, the moved material removed from every commit, the host and product names replaced, the author kept; a backup bundle before, a full-history scan after; the force-push and GitHub's cleanup of cached objects are the maintainer's steps.
- CI hardened: every action pinned to a full commit SHA, `permissions: {}` at the top of each workflow and the least per job, `persist-credentials: false`, tool installs from lockfiles with checksums, zizmor in validation, Dependabot for actions and cargo with a cooldown, `cargo deny` for advisories and licenses.
- Community and metadata: MIT `LICENSE`, third-party notices, `CONTRIBUTING.md` (the project's process), `CODE_OF_CONDUCT.md`, `SECURITY.md` with a disclosure path, issue and pull-request templates, `CHANGELOG.md`, crate metadata, the README's opening and its note on private APIs; a maintainer's checklist for the repository settings (rulesets, 2FA, secret scanning, private vulnerability reporting, tag protection).
- Publication: the rewritten history force-pushed by the maintainer, the repository made public, the settings applied.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None: tooling, documentation, history and settings; no requirement of the product changes.

## Impact

- `.github/` (workflows, Dependabot, templates, CODEOWNERS), `deny.toml`, `justfile`, `scripts/check-local-paths.sh` (extended), `Cargo.toml` and the crates' manifests (metadata), `LICENSE`, `THIRD-PARTY-NOTICES.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `CHANGELOG.md`, `README.md`, `docs/research/open-sourcing.md`, `docs/maintaining.md`.
- `reports/` and `crates/bench` (provenance wording and the raw samplers' format), `spikes/` (moved out), the git history (rewritten).
- No change to the drivers, the mirror, the daemon or the CLI's behaviour.
