# Maintaining the repository on GitHub

The settings the repository needs on GitHub, which no file in the tree can set: a checklist for the maintainer to apply by hand and tick, in this order, with GitHub's path to each setting as of 2026-10-15. The reasons are in `docs/research/open-sourcing.md`, which follows Astral's "Open source security at Astral"; the files the settings work with are `.github/workflows/`, `.github/dependabot.yml`, `SECURITY.md` and `.github/CODEOWNERS`. `<owner>` stands for the account that holds the repository.

Paths start from the repository's **Settings** tab; "Rules" and "Actions" sit under "Code, planning, and automation" in its sidebar, "Advanced Security" under "Security and quality". A `gh api` line checks the setting once it is applied.

## Before the repository is made public

- [ ] The history rewritten, scanned and force-pushed, the old branches deleted or left unpushed (`openspec/changes/open-sourcing/tasks.md`, task 5.2).
- [ ] GitHub Support asked to remove the cached views and the unreachable objects of the old history, with the repository's name, the number of affected pull requests from `.git/filter-repo/changed-refs` and the "First Changed Commit(s)" that `git filter-repo` printed; or, instead, the rewritten history pushed into a new repository under the same name, which holds no old object (`docs/research/open-sourcing.md`, Removing data). Until one of the two is done, old commits stay reachable by their ids.
- [ ] Dependabot's open pull request into the old history closed; Dependabot opens it again on the new one.

## The maintainer's account

- [ ] Two-factor authentication with a passkey or a security key, TOTP at the least, no SMS: your account's **Settings > Password and authentication > Two-factor authentication** and **Passkeys**. Astral's post requires nothing weaker than TOTP.
- [ ] Push protection for yourself left on, as it is by default: your account's **Settings > Code security > Push protection for yourself**.

## General

- [ ] A description and topics: the repository's front page, **About** (the gear beside it). The community standards count the description.
- [ ] **Settings > General > Features**: Issues on; Wikis and Projects off, since the documentation lives in the tree.
- [ ] **Settings > General > Pull Requests**: "Automatically delete head branches" on.

## Rules

- [ ] A branch ruleset for `main`: **Settings > Rules > Rulesets > New ruleset > New branch ruleset**, named `main`, enforcement **Active**, an empty bypass list (no bypass, the administrator's included), target **Include default branch**, and the rules:
  - [ ] Restrict deletions.
  - [ ] Require a pull request before merging, with 0 required approvals while there is one maintainer, and "Dismiss stale pull request approvals when new commits are pushed".
  - [ ] Require status checks to pass: `Audit`, `Linux`, `macOS (Xcode 26)` and `macOS (Xcode 27)`, with "Require branches to be up to date before merging".
  - [ ] Block force pushes.
- [ ] A branch ruleset that keeps disclosure branches from being pushed early, as Astral's does: target `advisory-*` and `internal-*`, rule **Restrict creations**, no bypass.
- [ ] A tag ruleset: **Settings > Rules > Rulesets > New ruleset > New tag ruleset**, target all tags (`**`), rules **Restrict updates**, **Restrict deletions** and **Block force pushes**, no bypass. **Restrict creations** joins them with the first release, its bypass the release workflow's deployment alone.
- `gh api repos/<owner>/mobile-agent/rulesets` lists the three.

## Actions

- [ ] **Settings > Actions > General > Actions permissions**: allow the actions the workflows use, and tick **Require actions to be pinned to a full-length commit SHA**, which fails a workflow that names an action by tag or branch, nested actions included. Every `uses:` is pinned already; zizmor's `unpinned-uses` and `impostor-commit` check them in CI.
  - `gh api repos/<owner>/mobile-agent/actions/permissions` answers `"sha_pinning_required": true`.
- [ ] **Settings > Actions > General > Workflow permissions**: "Read repository contents and packages permissions", and "Allow GitHub Actions to create and approve pull requests" off. Every workflow also starts with `permissions: {}`.
- [ ] **Settings > Actions > General > Approval for running fork pull request workflows from contributors**: "Require approval for all external contributors", so that no outside pull request runs a workflow before the maintainer has read it.
- [ ] **Settings > Actions > General > Artifact and log retention**: 90 days, which the benchmarks' trend relies on (`docs/research/ci-benchmarks.md`).

## Advanced Security

- [ ] **Settings > Advanced Security > Dependabot alerts**: Enable; and **Dependabot security updates**: Enable, which a public repository has on by default. Version updates come from `.github/dependabot.yml`, each after 7 days.
- [ ] **Settings > Advanced Security > Secret Protection**: Enable, free for a public repository; then **Push protection**: Enable.
- [ ] **Settings > Advanced Security > Private vulnerability reporting**: Enable, the channel `SECURITY.md` and `CODE_OF_CONDUCT.md` name.
  - `gh api repos/<owner>/mobile-agent/private-vulnerability-reporting` answers `{"enabled": true}`.
- [ ] Optional: **Settings > Advanced Security > Code scanning > CodeQL analysis > Set up > Default**, for Rust and GitHub Actions, free for a public repository; it is what OpenSSF Scorecard's SAST check counts.

## Publication

- [ ] **Settings > General > Danger Zone > Change repository visibility > Change to public**, once everything above is ticked.
- [ ] **Insights > Community Standards**: every item ticked but content reports, which only an organization's repositories have.
- [ ] The first runs of `ci.yml` green on `main`, and a scheduled or dispatched run of `benchmarks.yml` (its first run follows the visibility change, since Actions was blocked by the account's billing).

## At the first release

- [ ] **Settings > General > Releases > Enable release immutability**, before the first release: a release's tag and assets can then not change, and each release gets an attestation.
- [ ] **Settings > Environments > New environment** named `release`, with required reviewers and the release workflow's branch alone allowed; the publishing job runs in it, with no cache.
- [ ] Trusted publishing on crates.io for each crate published, after its first manual publish, and attestations for binaries (`docs/research/open-sourcing.md`, What a first release will need).
