# @mobileagent/cli-linux-x64

The `mobile-agent`, `mobile-agent-daemon` and `mobile-agent-mcp` binaries of [mobile-agent](https://github.com/gesposito/mobile-agent) for x86_64 Linux with glibc 2.35 or newer (`x86_64-unknown-linux-gnu`), where they drive Android alone, with their `THIRD-PARTY-NOTICES.md`, as the release's tarball `mobile-agent-<version>-x86_64-unknown-linux-gnu.tar.gz` holds them. Install [`@mobileagent/cli`](https://www.npmjs.com/package/@mobileagent/cli), whose commands run them: this package has no command of its own and no install script, and npm installs it with `@mobileagent/cli` on such a host alone.

Each binary carries the attestation of the release workflow's run that built it, which `gh attestation verify bin/mobile-agent -R gesposito/mobile-agent` checks in the package's directory.

Experimental, pre-release: the drivers use private Apple Simulator frameworks, Apple's developer services on an iPhone and hidden Android APIs; the command-line and MCP interfaces may change between alphas. MIT License.
