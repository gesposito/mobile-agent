# Tasks

Rules that bind every task below: research first; atomic commits without trailers; every scrub and scan checked on the tree before the history is touched; the history rewrite only after every other task of groups 2 to 4 has landed, with a bundle first; installs through `sfw`; no value that must not be tracked is written into a tracked file, not even in a scan's list.

## 1. Research

- [ ] 1.1 Write `docs/research/open-sourcing.md`: what Astral's "Open source security at Astral" prescribes and what applies to a small Rust project without releases yet, OpenSSF Scorecard and the best-practices badge criteria, GitHub's community standards and its documented procedure for removing sensitive data from a repository and its caches, cargo-deny and cargo-about for advisories, licenses and notices, Dependabot cooldowns, what a first release will need (trusted publishing, attestations, immutable releases) recorded for later; add its row to `docs/research/README.md`

## 2. Scrub and move

- [ ] 2.1 Provenance and samplers (`crates/bench/src/provenance.rs`, the harness's load sampler and the reports' Setup tables, `reports/**`, the tests' fixtures of provenance): failing tests that the provenance's host line names the chip, cores, memory and macOS version and no model or product, and that a sampler line keeps loads and CPU shares without process names; then implement, and rewrite the tracked reports and raw files the same way with a script kept beside the reports (so the rewrite is reproducible); verify red-then-green, every report's numbers unchanged, and the audit's terms gone from the tree
- [ ] 2.2 The publish scan (`scripts/check-local-paths.sh` grown into `scripts/check-public.sh`, kept under the old name as an alias or renamed in the justfile): patterns for machine paths, iOS identifiers, Android serials, Apple team ids, emails, host models, product names and personal handles, the values themselves never in the tracked script (an untracked `scripts/private-terms.txt` the script reads when present, documented); gitleaks over the tree when installed; a test with a staged file per pattern; verify the scan passes on the tree after 2.1 and fails on each planted value
- [ ] 2.3 Move the internal material out: the archived changes' `tasks.md` files and `spikes/` to `~/code/mobile-agent-private/<date>/` with a README there saying what each is, removed from the tree in one commit, `docs/spikes/README.md` and the archived designs' references adjusted to say the programs and the records are kept privately, `spikes/README.md` gone; verify `just validate` passes without `spikes/` (the workspace excludes it already) and no tracked file points at a removed path

## 3. Repository security

- [ ] 3.1 Workflows hardened (`.github/workflows/{ci,benchmarks}.yml`): `permissions: {}` at the top and the least per job, every action pinned to a full SHA with its version in a comment, `persist-credentials: false` on every checkout, the tool installs from a pinned lockfile (`.github/tools/` with `package.json` and its lockfile for OpenSpec, `cargo install just@<version> --locked`), checksums kept for downloaded binaries; zizmor and actionlint in `just validate` when installed and in a CI step; verify zizmor reports nothing and the workflow tests pass
- [ ] 3.2 Dependencies (`.github/dependabot.yml` for `github-actions` and `cargo` weekly with a 7-day cooldown; `deny.toml` and `cargo deny check` in `just validate` when installed and in CI): verify `cargo deny check advisories licenses bans sources` passes on the tree, every dependency's license listed as allowed on purpose
- [ ] 3.3 `docs/maintaining.md`, the maintainer's checklist for the repository's settings (rulesets for `main` and tags, 2FA, secret scanning with push protection, private vulnerability reporting, Dependabot alerts, the Actions policy requiring full-SHA pins), each with GitHub's current path to it, for the maintainer to apply and tick after publication

## 4. Community files and metadata

- [ ] 4.1 `LICENSE` (MIT), `THIRD-PARTY-NOTICES.md` generated with `cargo about` plus libjpeg-turbo's, the Kotlin standard library's and the fixtures' notes, `Cargo.toml` and the crates' manifests with license, repository, readme and description; verify `cargo deny check licenses` agrees with the notices
- [ ] 4.2 `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), `SECURITY.md` (private vulnerability reporting, a response window, what is in scope), `.github/ISSUE_TEMPLATE/` (bug, device report) and `PULL_REQUEST_TEMPLATE.md`, `CHANGELOG.md` with the four archived changes as its first entries, `.github/CODEOWNERS`; the README's opening paragraph and status for a first-time reader, its note on private Apple interfaces kept, every link in the tree checked
- [ ] 4.3 A duplication scan of the tree (jscpd or a Rust equivalent) recorded in the task with what it found, duplicates above a threshold folded or justified

## 5. History and publication

- [ ] 5.1 The history rewrite prepared and rehearsed on a clone: the replacement list (the identifier prefix line, the machine paths, the host model, the product name; the values in an untracked file), the path removals (`spikes/`, the archived `tasks.md`), a full-history scan script that greps every blob of every commit for the publish scan's patterns and runs gitleaks over the history; verify on the rehearsal clone that the scan is clean after the rewrite, the tree at `main` identical to the real one's, and the commit count equal
- [ ] 5.2 The rewrite on this clone by the maintainer's go: the bundle of every ref into the private folder, `git filter-repo` with the rehearsed options, the cited commit ids in tracked files remapped from the commit map, the scan of 5.1 run on the result, the branches and tags listed before and after; then the maintainer force-pushes
- [ ] 5.3 Publication, the maintainer's steps recorded here: GitHub support asked to remove the cached objects, the repository made public, `docs/maintaining.md`'s checklist applied and ticked, the Actions billing or the public plan settling CI's first run
- [ ] 5.4 Update the project memory and archive the change; verify `openspec validate open-sourcing --strict` passes and every task above is ticked
