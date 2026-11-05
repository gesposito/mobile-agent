# Tasks

Rules that bind every task below (the archived foundation design's Delivery process): research first; measured decisions recorded in design.md; fail-first where a test applies; atomic commits without trailers; comments and docs describe the state; every change lands through a pull request with the required checks (squash for one commit, rebase for several); nothing published, tagged or renamed on crates.io without the maintainer's own go at that step; device identifiers and tokens never in tracked files; one agent at a time; `scripts/check-public.sh` and `openspec validate --all --strict` before each commit.

## 1. Research and the dry run

- [ ] 1.1 `docs/research/alpha-release.md` (D1): cargo-dist against a hand-written workflow on attestations, immutability, the environment gate and tag creation; what a tag ruleset lets bypass and how Astral's release does it; crates.io trusted publishing step by step and the workspace's publish order; the binaries' targets and what each needs at build time; a verdict; its row in `docs/research/README.md` if one lists the notes.
- [ ] 1.2 The spike (D1): `release.yml` in dry-run mode on the real runners, the artifacts downloaded, `gh attestation verify` green on each, `--version` checked; the workflow's minutes and the artifacts' sizes in the design; the workflow tests (`crates/bench/tests/workflow.rs`, `hardening.rs`) extended for the new workflow; `just lint-workflows`.

## 2. The alpha on GitHub Releases

- [ ] 2.1 The workflow's publishing path (D2): the `publish` job in the `release` environment, the tag and the pre-release with checksums and notices, the notes from the changelog; the repository settings: release immutability, the environment with the maintainer as reviewer, the tag ruleset's creation rule with the workflow's bypass (through the API where it allows; the web-form steps written for the maintainer where it does not); `docs/maintaining.md`'s release items ticked with what was set.
- [ ] 2.2 The version and the words (D3): `0.1.0-alpha.1`, the changelog's section, the README's Install and the alpha's scope, `--version` with the commit, SECURITY.md's table.
- [ ] 2.3 The alpha (D4): the maintainer's approval, the published pre-release, `gh release verify` and `gh attestation verify` from a download, the clean-account install and the fixture's login flow; the record.

## 3. The crates on crates.io

- [ ] 3.1 The names (D5): every package renamed with the prefix, lib names kept, dependencies by the new names, `publish = false` where decided, `cargo publish --dry-run` green per crate in order, the lockfile and the notices regenerated, the README's crate list; the workspace's tests, clippy and the benches' names untouched.
- [ ] 3.2 The first publish (D6): the exact commands in order written for the maintainer, run by the maintainer, each crate's page checked; the record.
- [ ] 3.3 Trusted publishing (D6): a trusted publisher per crate configured by the maintainer, the `publish-crates` job added to `release.yml` and proven on `0.1.0-alpha.2` or a dry run that `cargo publish --dry-run`s from the job; `cargo install mobile-agent` on a clean account; the record.

## 4. The record

- [ ] 4.1 Update the project memory and archive the change; verify `openspec validate alpha-release --strict` passes and every task above is ticked.
