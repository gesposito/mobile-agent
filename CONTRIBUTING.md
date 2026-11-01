# Contributing to mobile-agent

mobile-agent is built in OpenSpec changes, each researched, specified and measured before its code: `openspec/changes/` holds the ones in progress and `openspec/changes/archive/` the finished ones, each with its proposal, design, tasks and spec deltas, and `openspec/specs/` the requirements as they stand. This file says how a change is made and checked; `README.md` says what the project does and `docs/overview.md` how it compares with the tools it measures against.

By contributing, you agree that your contribution is licensed under the repository's MIT License (`LICENSE`). Conduct follows `CODE_OF_CONDUCT.md`; a vulnerability goes through `SECURITY.md`, never a public issue.

## What to open

- **A bug**: an issue from the bug template, with `mobile-agent doctor --json`, then a pull request whose first commit is a failing test.
- **A device, OS or Xcode version tried**: an issue from the device report template, whether it worked or not.
- **Anything larger**, a feature, a change of behaviour, of the protocol or of a spec: an OpenSpec change under `openspec/changes/<name>/` (proposal, design, tasks, and spec deltas when a requirement changes), proposed and discussed before the code. The OpenSpec CLI is the one `.github/tools/package.json` pins; `openspec validate <name> --strict` checks a change.

## How a change is made

1. **Research first.** A component starts with a note in `docs/research/`: what exists, what this project reuses, where it differs, and the sources, read on the day (`docs/research/README.md`).
2. **Spikes before building on an assumption.** Throwaway code with a go or no-go gate settles what the design rests on, and its report goes into `docs/spikes/`, the code itself kept out of the repository.
3. **Measured decisions.** A choice a benchmark can make, a wire format, a transport, a codec, is made by one, with the numbers in the design.
4. **Tests first.** A failing test, observed red, comes before the code that makes it pass, in a commit of its own (`test(scope): …`), then the change (`fix(scope): …` or `feat(scope): …`), then the task's record (`docs(change): record task …`).
5. **Atomic commits** with conventional subjects, `feat`, `fix`, `test`, `docs`, `chore` or `ci` with the crate or area as the scope, and no trailers such as `Co-Authored-By`. Comments and documentation describe the code as it is, not the change that made it so.
6. **Installs through Socket Firewall.** `npm i -g sfw`, then every install through it: `sfw cargo add`, `sfw cargo fetch`, `sfw cargo install`, `sfw npm install`. After merging Dependabot's update of `Cargo.lock`, run `sfw cargo fetch` before building, and `just notices` to write `THIRD-PARTY-NOTICES.md` again.
7. **Numbers from a quiet host only.** A measurement starts at a 1-minute load under 4 with no build, emulator or Simulator above 5% of a core, which `just micro` checks before it runs and the harness records in each report's provenance; a report states its host by chip, cores, memory and macOS version.
8. **Nothing personal or machine-local in a tracked file.** `just check-public` refuses home and scratch paths, device identifiers, Android serials, Apple team ids, emails and Mac model identifiers in a tracked file or its name; write placeholders instead (`IPHONE-SE-UDID`, `NEXUS5X-SERIAL`, `TEAMPLACE1`, `$HOME`, `$TMPDIR`, `<repo>`). A maintainer can keep values no pattern catches in a private terms file outside the repository, which the scan reads from `MOBILE_AGENT_PRIVATE_TERMS` (`scripts/public-patterns.sh`).

## Checks

`just validate` runs what a change must pass: the build, clippy with warnings denied, the format check, the publish scan, the workflows' lint and audit, the dependencies' check, the tests, the scripts' own checks, the Android agent's and the iPhone runner's builds, the Kotlin codec cross-check, the fixture scripts' checks and builds, and `openspec validate --all --strict`. It needs the toolchain `rust-toolchain.toml` pins (`rustup toolchain install`), `just`, the OpenSpec CLI, and for the agent and the fixtures the Android SDK, a JDK, `kotlinc` and, on macOS, Xcode (`README.md`, Build). Without them:

```sh
SKIP_AGENT_BUILD=1 FIXTURES=none just validate   # no agent build, no codec cross-check, no fixture build
```

gitleaks, actionlint, zizmor and cargo-deny run when they are installed (`brew install gitleaks actionlint zizmor`, `sfw cargo install cargo-deny --locked`), and CI runs the last three in its audit job, with the publish scan and the OpenSpec validation. The tests that need a device skip unless their environment names one: an emulator or an Android device through `MOBILE_AGENT_ANDROID_SERIAL`, a booted Simulator through `MOBILE_AGENT_SIM_UDID`, an iPhone through `MOBILE_AGENT_IOS_UDID` (`crates/daemon/README.md`, `crates/simulator/README.md`, `crates/ios/README.md`). `just micro` runs the host microbenchmarks against `reports/micro/baseline.json`, and `just bench` the device harness (`crates/bench/README.md`).

## Pull requests

Fill in the template: the OpenSpec change or the issue, the test seen red, the checks run and the numbers' host. CI's audit, Linux and macOS jobs must pass, a pull request that changes documentation alone skipping Linux and macOS, whose checks then pass as skipped; workflows from forks run once the maintainer has read them. The maintainer reviews every pull request and merges it; `docs/maintaining.md` lists the repository's settings.
