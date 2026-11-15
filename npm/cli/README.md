# @mobileagent/cli

[mobile-agent](https://github.com/gesposito/mobile-agent) drives Android devices and emulators, iOS Simulators and physical iPhones for AI agents, from a daemon that keeps a live mirror of each screen. This package gives its three commands: `mobile-agent`, the command line, `mobile-agent-mcp`, the Model Context Protocol server an agent client starts, and `mobile-agent-daemon`, which both start on their first call.

Experimental, pre-release: the drivers use private Apple Simulator frameworks, Apple's developer services on an iPhone and hidden Android APIs; the command-line and MCP interfaces may change between alphas.

## Use

The package has three commands, so `npx` takes the package with `--package` and the command by its name; `npx mobile-agent-mcp` alone would fetch an unrelated package of that name. `@alpha` is the newest alpha; a version, `@mobileagent/cli@<version>`, keeps one.

```sh
claude mcp add --transport stdio mobile-agent -- npx -y --package=@mobileagent/cli@alpha mobile-agent-mcp
```

```json
{"mcpServers": {"mobile-agent": {"command": "npx", "args": ["-y", "--package=@mobileagent/cli@alpha", "mobile-agent-mcp"]}}}
```

```toml
# ~/.codex/config.toml
[mcp_servers.mobile-agent]
command = "npx"
args = ["-y", "--package=@mobileagent/cli@alpha", "mobile-agent-mcp"]
default_tools_approval_mode = "approve"
```

```sh
npx -y --package=@mobileagent/cli@alpha mobile-agent --version
npm install --global @mobileagent/cli@alpha    # the three commands on PATH
```

The repository's README says what each host needs (Install, Quick start): Xcode with an iOS Simulator runtime on the Mac for the Simulator and the iPhone, the Android SDK's `platform-tools` for Android.

## Platforms

| Host | Platform package | The release's target |
|---|---|---|
| macOS on Apple silicon | `@mobileagent/cli-darwin-arm64` | `aarch64-apple-darwin` |
| x86_64 Linux, glibc 2.35 or newer, Android alone | `@mobileagent/cli-linux-x64` | `x86_64-unknown-linux-gnu` |
| aarch64 Linux, glibc 2.35 or newer, Android alone | `@mobileagent/cli-linux-arm64` | `aarch64-unknown-linux-gnu` |

npm installs the platform package that matches the host's os, cpu and libc as an optional dependency, the way esbuild ships its binaries, and each command runs the binary in it; nothing runs or downloads at install, since no package has a script. On another host, or when optional dependencies are omitted, a command prints the supported platforms and exits 1. The binaries are those of the release's tarball for the target, which carries the release workflow's attestation (`gh attestation verify <binary> -R gesposito/mobile-agent`), and the packages are published from that workflow with npm's provenance, the first version on npm aside, which the maintainer published by hand from the release's tarballs.

MIT License.
