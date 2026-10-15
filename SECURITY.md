# Security policy

## Reporting a vulnerability

Report a vulnerability privately, through GitHub's private vulnerability reporting: the repository's **Security** tab, **Report a vulnerability** ([the form](../../security/advisories/new)). Do not open a public issue, pull request or discussion for it.

Say what is affected (the daemon, the CLI, the Android agent, the iPhone runner, the Simulator reader, a script or a workflow), the version or commit, how to reproduce it and what an attacker gains. Leave out device identifiers, account names and anything else personal; the maintainer asks for them privately when a report needs them.

## What happens next

- An acknowledgement within 7 days.
- An assessment within 14 days: whether the report is confirmed, its severity, and what a fix needs.
- A fix or a mitigation for a confirmed vulnerability of medium or higher severity within 60 days, published with a GitHub security advisory that credits the reporter unless they ask otherwise. The advisory goes out with the fix, and the details stay private until then.

## Supported versions

Nothing is released yet: the `main` branch is the one supported version, and a fix lands there.

## Scope

In scope, the code of this repository and how it uses its dependencies:

- the daemon, its Unix socket and its socket directory (mode 0700), and the commands it accepts (`crates/daemon`);
- the CLI (`crates/cli`) and the benchmark harness (`crates/bench`);
- the Android agent, its abstract socket reached through adb and the token that guards it (`agent-android`, `crates/android`);
- the iPhone runner and the port it serves over usbmuxd (`agent-ios`, `crates/ios`), and the Simulator reader (`crates/simulator/reader`);
- the scripts and the workflows (`scripts/`, `.github/workflows/`);
- a dependency's vulnerability as this project is exposed to it (`cargo deny check advisories` checks the known ones, and Dependabot proposes fixes).

Out of scope: Apple's and Google's tools and private interfaces themselves, which the drivers use as development tooling (`README.md`, Status and limitations); the apps under test; and an attacker who already runs code as the same user as the daemon, which serves that user's own processes.
