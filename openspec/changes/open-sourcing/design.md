# Design

## Context

The tree has been checked for machine-local paths on every commit since 2026-09-27 (`scripts/check-local-paths.sh`, later extended to iOS identifiers and emails), the Nexus serial and two personal lines were rewritten out of the history on 2026-10-08 with `git filter-repo --replace-text` (on this clone's `main`, force-pushed), and the device identifier and the team id of the physical-iOS work never entered a tracked file. An audit on 2026-10-15 found: no name, email, team id, device identifier, account handle or other product's name in the tree; the host's model identifier, its endpoint-security product and the host's process names (including the tools the maintainer works with) in the reports' provenance, two tests and the device run's raw `loads.txt` samplers; in the history, two commits of 2026-10-08 with another phone's identifier prefix and three of 2026-09-27 with `/Users/<name>` paths; all 1,061 commits authored with the maintainer's name and email. zizmor reported 19 actions pinned to tags, 7 checkouts keeping credentials and 2 ad-hoc package installs; GitHub keeps unreachable objects after a force-push until its support removes them.

## Goals / Non-Goals

**Goals:**
- Nothing personal, private or internal in the tree or in any commit of the published history; a scan that keeps it so.
- The notebook's internal records out of the public tree and kept locally for reference.
- CI and the repository hardened to the practices the maintainer chose (Astral's), with the gaps that post leaves filled (a security policy, dependency advisories and licenses, `persist-credentials`).
- The files and metadata a public Rust project is expected to carry, under MIT.

**Non-Goals:**
- Publishing crates or binaries (trusted publishing, attestations and a release workflow come with a first release, recorded in the research note).
- A squashed history; the maintainer keeps the history and their authorship.
- Removing the reports' raw data: the measured evidence stays, scrubbed.

## Decisions

### D1. What stays internal, and where it goes
The archived changes' `tasks.md` (the red-then-green counts, phone visits and session hand-offs: records for the notebook, not a reader) and `spikes/` (throwaway programs whose findings `docs/spikes/` reports) leave the tree. Proposals, designs and specs stay: they are the "why" a reader needs. `reports/*.md` and the raw folders beside them stay as the evidence the README cites, scrubbed by D2, with `reports/micro/baseline.json` which `just micro` reads. The material moves to `~/code/mobile-agent-private/<date>/` outside any repository, with the pre-rewrite bundle beside it; the archived designs' references to task numbers stay as references to the private records. Alternative: a second private repository (rejected by the maintainer: a local folder is enough for references).

### D2. The scrub
Provenance describes the host by class ("Apple M1 Max, 10 cores, 64 GB, macOS 26.6.2") and says "endpoint security software" where it named a product; the samplers' `top=[…]` fields keep the CPU shares and drop the process names, since a reader needs the load, not what the maintainer runs; tests' fixtures of provenance follow. `scripts/check-local-paths.sh` grows into the publish scan: machine paths, iOS identifiers and emails as today, plus Android serials, Apple team ids, the host's model and product names, the maintainer's name and handles, and the other product's name, all as patterns, never as a list of the values themselves in the tracked script (the values live in an untracked file the script reads when present, or in the pattern shapes). gitleaks runs over the tree in `just validate` when installed, over the whole history once before the push.

### D3. The history rewrite
One `git filter-repo` run on this clone: `--replace-text` for the identifier prefix line, the `/Users/<name>` paths, the host model and the product name; `--path` removals for `spikes/` and the archived `tasks.md` files from every commit; the author untouched. Before it, a backup bundle of every ref; after it, a full-history scan (every blob of every commit grepped for the patterns, and gitleaks), the commit map kept beside the bundle, and the cited commit ids in tracked files remapped as on 2026-10-08. The force-push is the maintainer's step, as is asking GitHub support to remove the cached unreachable objects (GitHub's documented procedure), before the repository is made public. Alternative: squashing (rejected by the maintainer).

### D4. CI hardening
Both workflows start with `permissions: {}` and each job declares the least it needs (`contents: read`; `actions: read` for the artifact download); every `uses:` is pinned to a full commit SHA with the version as a comment, kept current by Dependabot and checked by zizmor's `unpinned-uses` and `impostor-commit` in `just validate` when zizmor is installed and in CI; `actions/checkout` gets `persist-credentials: false`; `just` and OpenSpec install from a pinned lockfile (`cargo install --locked` with a version, the npm tool through a `package.json` and lockfile under `.github/tools/`); downloaded binaries keep their checksums as Maestro's does; no `pull_request_target`, no `workflow_run`, no secrets; Dependabot for `github-actions` and `cargo` weekly with a 7-day cooldown; `cargo deny check advisories licenses bans sources` with a `deny.toml` allowing MIT, Apache-2.0, BSD and the licenses the tree holds, run by `just validate` when installed and by CI.

### D5. Community files and metadata
MIT `LICENSE` in the maintainer's name and year; `THIRD-PARTY-NOTICES.md` from `cargo about` for the crates, with libjpeg-turbo's notice (linked statically), the Kotlin standard library's (Apache-2.0, in the Android agent's dex) and the fixtures' dependencies noted as installed, not shipped; `CONTRIBUTING.md` states the process (an OpenSpec change for anything beyond a fix, research first, fail-first tests, measured decisions, atomic commits without trailers, installs through Socket, the quiet-host rule for numbers) and how to run the suites; `CODE_OF_CONDUCT.md` the Contributor Covenant 2.1; `SECURITY.md` a disclosure path through GitHub's private vulnerability reporting with a response window; issue templates for a bug and a device report, a pull-request template with the checks; `CHANGELOG.md` with an Unreleased section and the four archived changes as its first entries; `Cargo.toml` metadata (`license = "MIT"`, `repository`, `readme`, `description` per crate), `publish = false` kept until a release; the README opens with what the project is and its status, and keeps its note that the drivers use private Apple interfaces; `docs/maintaining.md` holds the repository-settings checklist (rulesets: `main` by pull request, no force-push, tag protection; 2FA; secret scanning and push protection; private vulnerability reporting; Dependabot alerts) for the maintainer to apply by hand and tick.

## Risks / Trade-offs

- [The rewrite drops something a reader needs] → the designs and specs stay; the private folder keeps everything; the bundle restores the old history if the maintainer wants it back.
- [GitHub serves the old objects after the force-push] → the support request before going public; the research note records the procedure.
- [A scrub pattern misses a value] → the publish scan is pattern-based and extended by every finding; gitleaks over the history before the push.
- [Pinned SHAs go stale] → Dependabot keeps them current; the cooldown keeps a compromised release out for a week.
