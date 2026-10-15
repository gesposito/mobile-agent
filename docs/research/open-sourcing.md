# Open-sourcing a Rust project: state of the art

Research note for task 1.1 of `open-sourcing`. It feeds the scrub and the publish scan (design D2), the history rewrite (D3), the hardened workflows and the dependency checks (D4) and the community files (D5), and records what a first release will need. Every claim was read from the sources at the end on 2026-10-15, and versions are the latest released that day. Platform facts come from current documentation: Context7 for GitHub's documentation (`/github/docs`), OpenSSF Scorecard, cargo-deny and cargo-about; Astral's post, the best-practices badge's criteria, crates.io's announcement and the tools' releases are read on their own sites, and each section says which.

Out of scope: publishing crates or binaries (a first release's needs are recorded at the end, not built), squashing the history (rejected in the proposal), and choosing the license (MIT, the maintainer's decision).

## Findings in brief

- **Astral's post is about CI and releases more than code.** For a repository without releases it comes down to: no `pull_request_target` or `workflow_run`; every action pinned to a full commit SHA, cross-checked against impostor commits with zizmor's `unpinned-uses` and `impostor-commit`, and GitHub's policy that refuses unpinned actions turned on; `permissions: {}` at the top of every workflow and the least per job; no secrets outside deployment environments; actions read for "mutable decisions", such as downloading a latest binary, which a pin does not stop; branch rules that keep `main` from force-pushes and changes off pull requests, tag rules that keep tags from moving or going away, no admin bypass; 2FA no weaker than TOTP; Dependabot with a cooldown. Trusted publishing, Sigstore attestations, immutable releases, a release environment approved by a second person and no caching in release jobs wait for a first release.
- **This repository's workflows miss three of those rules today.** No action is pinned (zizmor 1.31.0 counts 19 `unpinned-uses`, every `uses:`); the top-level permissions grant `contents: read`, and `actions: read` in `benchmarks.yml`, instead of nothing; and two steps make mutable decisions: Socket Firewall comes from the `latest` release without a checksum, and Bun is `bun-version: latest`. zizmor also counts 7 `artipacked` (every checkout keeps its credentials) and 2 `adhoc-packages` (OpenSpec installed outside a lockfile), which the post does not cover. Neither workflow uses a dangerous trigger, a secret or a write permission.
- **OpenSSF Scorecard's 18 checks** weigh Critical 10, High 7.5, Medium 5 and Low 2.5. Pinned-Dependencies, Token-Permissions, Dangerous-Workflow, Dependency-Update-Tool, License, Security-Policy, CI-Tests and Binary-Artifacts are in reach before publication; Branch-Protection is a setting; Code-Review and Contributors stay low with one maintainer; Signed-Releases and Packaging need releases; Fuzzing detects OSS-Fuzz, ClusterFuzzLite and Go's native fuzzing, not cargo-fuzz.
- **The best-practices badge's passing level** lists 67 criteria in six groups. The ones a public repository with this README, CI and the files below does not meet yet are a release with notes and a unique version (`version_unique`, `release_notes`) and documented response times for reports; the rest is either met by the tree or answered in the badge's form.
- **GitHub's community standards** check a description, README, code of conduct, contributing guide, license, security policy, issue templates (with `name` and `about` or `description` keys under `.github/ISSUE_TEMPLATE/`), a pull request template and content reports, the last of which an organization's repositories have and a personal account's do not.
- **A force-push is not the end of a history rewrite on GitHub.** GitHub's procedure is `git filter-repo --sensitive-data-removal` (2.47 or later), a `--mirror` force-push, then a support request with the repository, the number of affected pull requests and the "First Changed Commit(s)" filter-repo prints; support then dereferences the pull requests, collects the garbage and removes cached views. It helps only with data whose risk "can't be mitigated by rotating affected credentials", and it removes nothing in forks or other clones. Until then the old commits stay reachable by their ids: Truffle Security's "Cross Fork Object Reference" work showed commits of deleted and private forks served through a public repository's network, which GitHub calls working as designed.
- **cargo-deny** checks advisories (the RustSec database), licenses (SPDX expressions against an allow list, with a confidence threshold for license files), bans (duplicates, wildcards, denied crates) and sources (registries and git); it is 0.20.2. **cargo-about** 0.9.2 renders every dependency's license text through a Handlebars template, from an `about.toml` of accepted licenses, skipping unpublished workspace members with `private = { ignore = true }`. The workspace's graph holds 17 license identifiers besides its own crates, which declare none yet.
- **Dependabot's `cooldown`** delays version updates, never security updates, by `default-days` (3 when unset; 1 to 90) or by SemVer level; Cargo supports both forms, GitHub Actions only `default-days`.
- **A first release** will need: trusted publishing to crates.io (GitHub Actions only, after a first manual publish of each crate, through `rust-lang/crates-io-auth-action` v1.0.5 with `id-token: write`), artifact attestations (`actions/attest-build-provenance` v4.2.2, `gh attestation verify`), immutable releases (tag and assets locked, a release attestation generated), a release environment, tag rules, and release jobs without caches.

## Astral's practices

From William Woodruff's "Open source security at Astral" (2026-04-08), read on astral.sh.

| Practice in the post | Here, now | Here, at a first release | Not here |
|---|---|---|---|
| No `pull_request_target` or `workflow_run`, forbidden organization-wide | already: `ci.yml` runs on `push`, `pull_request` and `workflow_dispatch`, `benchmarks.yml` on `schedule` and `workflow_dispatch` | | |
| "All actions to be pinned to specific commits", "cross-check[ed]" against the released state; GitHub's policy to "require actions to be pinned to a full-length commit SHA" as a hard gate; pinact to pin, zizmor to check | pins with the version as a comment, Dependabot keeping them current, zizmor in `just validate` and CI, the policy in the settings checklist | | |
| Actions reviewed for "mutable decisions", such as fetching a latest binary | Socket Firewall pinned to a release with its SHA-256, as Maestro is; Bun pinned to a version | | |
| `permissions: {}` at the top, broadened per job | both workflows | | |
| Secrets in deployment environments only | no secrets: the token the benchmarks job uses is `GITHUB_TOKEN` with `actions: read` | the publishing job's environment | |
| Few privileged accounts; 2FA no weaker than TOTP | the maintainer's account, with a passkey or TOTP | | organization roles |
| `main` without force-pushes and by pull request; `advisory-*` and `internal-*` branches blocked; no admin bypass | a branch ruleset (settings checklist) | | |
| Release tags created only by a successful release deployment, never updated or deleted | tags neither updated nor deleted (settings checklist) | creation restricted to the release deployment | |
| Automations in a minimal GitHub App instead of Actions with write tokens | | | nothing comments or labels |
| Trusted Publishing; Sigstore attestations; immutable releases | | all three | |
| A dedicated release environment, approved by a second privileged member; no caching in releases | | the environment and an uncached job; a second approver needs a second maintainer | |
| Dependabot or Renovate with cooldowns; new dependencies added cautiously, binary blobs avoided, unused features disabled | Dependabot for actions and cargo with a 7-day cooldown; installs through Socket Firewall | | |

The post does not mention `persist-credentials`, lockfiles for tools or a security policy; zizmor's `artipacked` and `adhoc-packages` audits and Scorecard's Security-Policy cover them.

**For this project.** Everything in the first column that a repository without releases can have, now; the release column is recorded below for the change that makes a first release. GitHub's newer workflow dependency lockfile (`.github/workflows/actions.lock`, written by the `gh actions-lock` extension, pinning transitive actions and checking each source repository's id at run time) is in public preview and "subject to change", so the pins stay in the workflow files, where zizmor and Dependabot read them.

## OpenSSF Scorecard

Context7 (`/ossf/scorecard`), the README and `docs/checks.md` at v5.6.0; scorecard-action is v2.5.0.

| Check | Risk | This repository after the change |
|---|---|---|
| Dangerous-Workflow | Critical | no `pull_request_target`, no `workflow_run`, no untrusted input in a `run:` (zizmor's `template-injection`) |
| Binary-Artifacts | High | no executables once `spikes/` leaves; the protocol's golden frames are data |
| Branch-Protection | High | a setting: the branch ruleset |
| Code-Review | High | low with one maintainer: changes land through pull requests nobody else reviews |
| Dependency-Update-Tool | High | `.github/dependabot.yml` |
| Maintained | High | met by activity, once the repository is 90 days old |
| Signed-Releases | High | needs releases |
| Token-Permissions | High | `permissions: {}` at the top, read-only per job |
| Vulnerabilities | High | `cargo deny check advisories`; OSV for the score |
| Fuzzing | Medium | none: the check detects OSS-Fuzz, ClusterFuzzLite and Go's native fuzzing, not cargo-fuzz |
| Packaging | Medium | needs packages published from CI |
| Pinned-Dependencies | Medium | actions by SHA, tool installs from lockfiles (`npm ci`, `cargo install --locked` with a version), downloads checked against a SHA-256 |
| SAST | Medium | clippy does not count; CodeQL's default setup does, and covers Rust since 2025-10-14 (settings checklist, optional) |
| Security-Policy | Medium | `SECURITY.md` |
| CI-Tests | Low | CI on every pull request |
| CII-Best-Practices | Low | not applied for |
| Contributors | Low | low with one maintainer |
| License | Low | `LICENSE` |

Webhooks, experimental and Critical, needs a maintainer's token and applies to configured webhooks only; there are none.

**For this project.** The workflow and file checks are met by the change; Branch-Protection by the settings checklist; the rest are facts of a one-maintainer project without releases, and none is worth gaming. Running Scorecard's action would need `id-token: write` and `security-events: write` on a job of its own, so it waits until the repository is public.

## The OpenSSF Best Practices badge

The criteria page for the passing level, read on bestpractices.dev.

The passing level's 67 criteria fall into Basics (a description, how to interact and contribute, an OSI license in a standard location, documentation, HTTPS sites, a discussion venue, English, maintained), Change Control (a public repository with interim versions, unique version numbers, release notes naming fixed vulnerabilities), Reporting (a bug process with a tracker and responses, an archive, a vulnerability process with private reports and a response within 14 days), Quality (a working build, an automated test suite run before changes are accepted, tests added with new functionality, warnings fixed), Security (secure design and common errors known, cryptography left to published, reviewed libraries, delivery over HTTPS, vulnerabilities fixed within 60 days, no leaked credentials) and Analysis (static analysis with its findings fixed, dynamic analysis findings fixed). MUST, SHOULD and SUGGESTED mark each.

**For this project.** The README, `CONTRIBUTING.md`, `SECURITY.md`, `LICENSE`, CI and clippy with warnings denied meet most of the MUSTs. Until a release, `version_unique` and `release_notes` are unmet, so the badge waits for the first release; `SECURITY.md` states the response windows the Reporting criteria ask about.

## GitHub's community standards

Context7 (`/github/docs`, the community profile pages) and the Community Standards page of public repositories.

The checklist under Insights > Community Standards has nine items: Description, README, Code of conduct, Contributing, License, Security policy, Issue templates, Pull request template and "Repository admins accept content reports". An issue template counts only from `.github/ISSUE_TEMPLATE/`, with `name:` and `about:` front matter in a Markdown template or `name:` and `description:` in an issue form; a pull request template may live in the root, `docs/` or `.github/`. A `LICENSE` cannot come from an account's default community files: "License files must be added to individual repositories". Content reports are a setting of repositories owned by an organization ("You can enable or disable reported content for public repositories owned by an organization").

**For this project.** Every file of the list in the tree; the description is a setting; content reports do not apply to a personal account's repository, so `CODE_OF_CONDUCT.md` names the private reporting channel and GitHub's own abuse reports instead.

## Removing data from a repository and from GitHub

GitHub's "Removing sensitive data from a repository", through Context7 and docs.github.com; `git filter-repo`'s help at 2.47.0.

**Does.** Four steps: rewrite the history locally with `git filter-repo`, push the rewritten history, clean up other clones, and prevent repeats. The rewrite is `git-filter-repo --sensitive-data-removal --invert-paths --path <file>` for files and `--replace-text <file>` for text, in a fresh clone; `--sensitive-data-removal` (2.47 and later) fetches every ref from `origin` first, records the changed refs in `.git/filter-repo/changed-refs` and prints the "First Changed Commit(s)" and any orphaned LFS objects. `grep -c '^refs/pull/.*/head$' .git/filter-repo/changed-refs` counts the affected pull requests, and `git push --force --mirror origin` pushes, the `refs/pull/` refs failing as read-only, as expected. Then support, given the owner and name, the count and the first changed commits, will "dereference or delete any affected PRs", run a garbage collection and remove cached views, but "won't remove non-sensitive data" and helps only "in cases where we determine that the risk can't be mitigated by rotating affected credentials". Side effects: every later commit gets a new id, signatures are dropped, closed pull requests lose their diffs, and anyone with an old clone can push the data back.

filter-repo also prunes the commits a removal empties (`--prune-empty auto`, the default), rewrites abbreviated commit ids in commit messages to the new ones (unless `--preserve-commit-hashes`), turns a fresh clone's `refs/remotes/origin/*` into branches and removes the `origin` remote, and writes `.git/filter-repo/commit-map`. `--replace-text` takes literals, `regex:` and `glob:` lines with an optional `==>replacement` (`***REMOVED***` without one), applies the literals in order and then the regular expressions, skips a blob with a zero byte in its first 8 KiB as binary, and reads every line as an expression, so a comment line would be replaced wherever its text occurs; `--replace-message` does the same to commit messages.

**For this project.** The history holds no credentials, but its leaks are of the kind rotation cannot fix: a phone's identifier prefix, a home directory's account name, the host's model and its security product, process names. The rewrite is rehearsed on a clone first (task 5.1), with `--sensitive-data-removal` for the first changed commits support asks for, `--prune-empty never` so that the commits that touched only `spikes/` stay and every cited commit id maps to one new id, and the replacement list kept outside the repository. The repository has never been public and has no forks; its one pull request, Dependabot's toolchain bump, holds a ref into the old history. The old objects are therefore reachable only on GitHub's side, and support's cleanup, or a new repository under the same name, which starts with no old objects at all, removes them before the visibility changes. A new repository keeps none of the old one's issues, pull requests, settings or workflow runs, of which this one has the one pull request and 142 runs.

## Scanning the tree and the history

gitleaks 8.30.1 from its help; GitHub's secret scanning through Context7.

**Does.** `gitleaks dir` scans files and `gitleaks git` a repository's history (`--log-opts=--all` for every ref), with built-in rules for credentials, a `.gitleaks.toml` that extends them (`[extend] useDefault = true`) and allowlists, `--redact` to keep matches out of the output, and exit status 1 on a finding. GitHub's push protection for users, on by default, blocks pushes of supported secrets to public repositories; secret scanning and push protection for a repository are free for public repositories and enabled from Settings > Advanced Security.

**For this project.** gitleaks finds two matches today, both a placeholder token in the Android agent's record tests (`0123456789abcdef`), which a `.gitleaks.toml` allows by value and path. No credential scanner looks for what this repository leaks, so the publish scan adds its own patterns: machine paths, iOS device identifiers, Android serials in the contexts the tree writes them (`adb -s`, `--target`, `serial`), Apple team ids beside a team's name, emails, Mac model identifiers, and the values of a private list kept outside the repository.

## Dependencies: advisories, licenses and notices

cargo-deny and cargo-about through Context7 (`/embarkstudios/cargo-deny`, `/embarkstudios/cargo-about`); Dependabot's options reference through Context7 and docs.github.com.

**cargo-deny 0.20.2.** `cargo deny check advisories licenses bans sources` reads `deny.toml`: `[advisories]` against the RustSec database, with `ignore` entries carrying a reason and `unmaintained` scoped to `workspace` or all; `[licenses]` with an `allow` list of SPDX identifiers, per-crate `exceptions`, a `confidence-threshold` (0.8 by default) for license files, `private = { ignore = true }` for unpublished workspace crates, and a warning for an allowed license nothing uses; `[bans]` for multiple versions, wildcards and denied crates; `[sources]` denying unknown registries and git sources. `[graph] targets` limits the graph to the platforms built. `cargo deny list` prints every license with its crates.

**cargo-about 0.9.2.** `cargo about init` writes `about.toml` and `about.hbs`; `cargo about generate about.hbs` renders the license texts of the dependency graph, grouped by license, from `accepted = [...]`, with `targets`, `ignore-build-dependencies`, `ignore-dev-dependencies` and `private = { ignore = true }`. It installs with `cargo install --locked cargo-about --features cli`.

**The licenses in the graph.** `cargo deny list` names 17 identifiers besides the workspace's own crates: MIT (207 crates) and Apache-2.0 (159), mostly as `MIT OR Apache-2.0`; Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, BSL-1.0 (xxhash-rust), BlueOak-1.0.0 (minicbor), CC0-1.0, CDLA-Permissive-2.0 (webpki-roots' data), IJG (jpeg-encoder), ISC, MIT-0, Unicode-3.0, Zlib, 0BSD and Unlicense, each in some expression; LGPL-2.1-or-later only as one alternative of r-efi's `MIT OR Apache-2.0 OR LGPL-2.1-or-later`. Which of them an allow list needs depends on the expressions, which cargo-deny evaluates.

**Dependabot.** `cooldown` delays version updates by `default-days`, or by `semver-major-days`, `semver-minor-days` and `semver-patch-days`; "only available for *version* updates, not *security* updates"; 3 days when `default-days` is unset, 1 to 90 days. Cargo supports the SemVer forms, GitHub Actions only `default-days`. A Dependabot pull request for Cargo changes `Cargo.lock`, and the crates it names reach a local checkout through `cargo`'s own fetch, outside Socket Firewall, unless `sfw cargo fetch` runs first; the cooldown, `cargo deny` in CI and that fetch are what this project puts around it.

## What a first release will need

- **Trusted publishing to crates.io**, from the crates.io announcement of 2025-07-11: GitHub Actions only, a trusted publisher configured per crate on crates.io, and "you'll need to publish your first release manually"; the job needs `id-token: write` and exchanges the token through `rust-lang/crates-io-auth-action` (v1.0.5, 2026-06-16) for a short-lived one that `cargo publish` reads as `CARGO_REGISTRY_TOKEN`, preferably in a `release` environment.
- **Artifact attestations** for binaries: `actions/attest-build-provenance` (v4.2.2) with `id-token: write` and `attestations: write`, checked with `gh attestation verify <file> -R <owner>/<repo>`.
- **Immutable releases**, a repository setting: a published release's tag "cannot be changed, and cannot be deleted while the release exists", its assets cannot be modified or deleted, a deleted release's tag name cannot be reused, and each release gets a release attestation (`gh release verify <tag>`).
- **A release environment and tag rules**: the publishing job alone in an environment with required reviewers, tag creation restricted to it, release jobs without caches, as Astral's post prescribes; with one maintainer, the approval is the maintainer's own.
- **Then** `publish = false` comes off the crates that ship, the metadata gains `repository`, the badge's release criteria become reachable, and `CHANGELOG.md`'s Unreleased section becomes the first version.

## What we take

- **D1.** `spikes/` leaves the tree for the private folder, its reports in `docs/spikes/` staying; the maintainer kept the archived changes' records in the repository.
- **D2.** The host described by its chip, cores, memory and macOS version; the samplers keep the loads and the CPU shares. The publish scan's built-in patterns are shapes, the values in a private list outside the repository, read when present; gitleaks over the tree in `just validate` when installed, with an allowlist for the placeholder token.
- **D3.** `git filter-repo` 2.47.0 with `--sensitive-data-removal`, `--replace-text` from the private folder, `--path spikes --invert-paths` and `--prune-empty never`, rehearsed on a clone and followed by a full-history scan with the same patterns and gitleaks; after the force-push, support's cleanup or a new repository before the visibility changes.
- **D4.** Astral's CI rules as tabled above, plus `persist-credentials: false`, OpenSpec from a lockfile, `just` at a pinned version with `--locked`, Socket Firewall pinned with its SHA-256 and Bun pinned to a version; zizmor and actionlint in `just validate` and CI; Dependabot for `github-actions` and `cargo`, weekly, 7 days of cooldown; `cargo deny check advisories licenses bans sources` against an allow list of the licenses the graph needs.
- **D5.** The nine community-standards items, `THIRD-PARTY-NOTICES.md` from cargo-about with the notes for what it cannot see, and the settings checklist in `docs/maintaining.md`.

## Where this design differs

- From Astral: one maintainer, so no second approver, no organization rules and no GitHub App; pins checked by zizmor online when a token is at hand, offline otherwise.
- From GitHub's procedure: the rewrite replaces text in many files rather than removing a file, keeps the emptied commits, and runs once on a clone that is then pushed by the maintainer.
- From Scorecard's ideal: no fuzzing, no code review by a second person, no signed releases yet.

## Risks

- **Old objects on GitHub.** Without support's cleanup or a new repository, commits of the old history stay reachable by id after the force-push; the visibility changes only after one of the two.
- **A pattern misses a value.** The private list grows with each finding, the full-history scan runs before the push, and gitleaks runs with it.
- **Dependabot's Cargo updates and Socket Firewall.** Its pull requests fetch nothing locally, but merging one moves `Cargo.lock`; `sfw cargo fetch` before the next build, the cooldown and `cargo deny` in CI keep that path checked.
- **Pins going stale.** Dependabot proposes each new action release after the cooldown; zizmor refuses an unpinned or impostor reference.

## Sources

- Astral: W. Woodruff, [Open source security at Astral](https://astral.sh/blog/open-source-security-at-astral) (2026-04-08), and the linked [example rulesets](https://gist.github.com/woodruffw/643a6cf70ad72d404ce6f9f333181cf8)
- zizmor 1.31.0 (2026-10-15): [audits](https://docs.zizmor.sh/audits/) (`unpinned-uses`, `impostor-commit`, `artipacked`, `adhoc-packages`, `template-injection`); pinact 5.0.0 (2026-09-12): [README](https://github.com/suzuki-shunsuke/pinact); actionlint 1.7.12
- GitHub Actions, through Context7 (`/github/docs`) and docs.github.com: [secure use reference](https://docs.github.com/en/actions/reference/security/secure-use), [workflow dependency lockfiles](https://docs.github.com/en/actions/concepts/security/workflow-dependency-lockfiles) (public preview), [a repository's Actions settings](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository); [changelog of 2025-08-15](https://github.blog/changelog/2025-08-15-github-actions-policy-now-supports-blocking-and-sha-pinning-actions/) (blocking and SHA pinning in the allowed actions policy)
- OpenSSF Scorecard v5.6.0 (2026-10-12): [README](https://github.com/ossf/scorecard/blob/main/README.md), [checks](https://github.com/ossf/scorecard/blob/main/docs/checks.md), through Context7 (`/ossf/scorecard`); [scorecard-action](https://github.com/ossf/scorecard-action) v2.5.0
- OpenSSF Best Practices badge: [passing criteria](https://www.bestpractices.dev/en/criteria/0)
- GitHub community standards: [about community profiles](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/about-community-profiles-for-public-repositories), [default community health files](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/creating-a-default-community-health-file), [reported content](https://docs.github.com/en/communities/moderating-comments-and-conversations/managing-how-contributors-report-abuse-in-your-organizations-repository), through Context7 (`/github/docs`)
- GitHub, removing data: [Removing sensitive data from a repository](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/removing-sensitive-data-from-a-repository), through Context7 (`/github/docs`); `git filter-repo` 2.47.0 (2024-12-04), [manual](https://htmlpreview.github.io/?https://github.com/newren/git-filter-repo/blob/docs/html/git-filter-repo.html); Truffle Security's Cross Fork Object Reference, as reported by [The Register](https://www.theregister.com/2024/07/25/data_from_deleted_github_repos/) (2024-07-25)
- GitHub security settings, through Context7 (`/github/docs`): [security and analysis settings](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-security-and-analysis-settings-for-your-repository), [push protection](https://docs.github.com/en/code-security/concepts/secret-security/push-protection), [private vulnerability reporting](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/configure-for-a-repository), [rulesets](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/creating-rulesets-for-a-repository); CodeQL: [changelog of 2025-10-14](https://github.blog/changelog/2025-10-14-codeql-scanning-rust-and-c-c-without-builds-is-now-generally-available/) (Rust generally available)
- gitleaks 8.30.1 (2026-03-21): [README](https://github.com/gitleaks/gitleaks)
- cargo-deny 0.20.2 (2026-07-09): [book](https://embarkstudios.github.io/cargo-deny/), through Context7 (`/embarkstudios/cargo-deny`); cargo-about 0.9.2 (2026-08-18): [book](https://embarkstudios.github.io/cargo-about/), through Context7 (`/embarkstudios/cargo-about`)
- Dependabot: [options reference](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference) (`cooldown`), [optimizing pull request creation](https://docs.github.com/en/code-security/tutorials/secure-your-dependencies/optimizing-pr-creation-version-updates), through Context7 (`/github/docs`)
- Releases: crates.io, [development update of 2025-07-11](https://blog.rust-lang.org/2025/07/11/crates-io-development-update-2025-07) (trusted publishing), [RFC 3691](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html), [crates-io-auth-action](https://github.com/rust-lang/crates-io-auth-action) v1.0.5; [attest-build-provenance](https://github.com/actions/attest-build-provenance) v4.2.2; GitHub's [immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases), through Context7 (`/github/docs`)
