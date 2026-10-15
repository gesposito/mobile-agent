## What and why

<!-- The OpenSpec change (openspec/changes/<name>/) and its task, or the issue a fix closes. -->

## Checks

- [ ] A failing test came first and was seen red, in a commit of its own.
- [ ] `just validate` passes (say which parts were skipped, such as `SKIP_AGENT_BUILD=1 FIXTURES=none`).
- [ ] `just check-public` passes: no machine path, device identifier, serial, team id, email or host model in a tracked file.
- [ ] Numbers, if any, come from a quiet host and their report says which (chip, cores, memory, macOS).
- [ ] The documentation says what the code does now, the README and the crate's README included.
- [ ] Atomic commits with conventional subjects and no trailers; dependencies added through Socket Firewall.
