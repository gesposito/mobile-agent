'use strict';
// The launcher of @mobileagent/cli's commands, mobile-agent, mobile-agent-daemon
// and mobile-agent-mcp (design D7 of the alpha-release change). The binaries
// are in one platform package per target, @mobileagent/cli-<platform>-<arch>,
// which npm installs as an optional dependency where its os, cpu and libc
// match the host, so that nothing runs or downloads at install. A command
// finds its binary in the platform package of Node's platform and
// architecture and runs it from there, with the command's arguments and
// standard streams, forwards SIGINT, SIGTERM and SIGHUP to it, and exits
// with its status, or by the signal that killed it. The CLI and the MCP
// server start the daemon they find beside their own executable, which is
// the platform package's. Without a platform package for the host it prints
// one line naming the supported platforms and exits 1.

const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

/** Each platform package's `<platform>-<arch>`, as Node names them, and the
 * release's target whose tarball its binaries come from. */
const PLATFORMS = {
  'darwin-arm64': 'aarch64-apple-darwin',
  'linux-x64': 'x86_64-unknown-linux-gnu',
  'linux-arm64': 'aarch64-unknown-linux-gnu',
};

const SUPPORTED = 'darwin-arm64 (macOS on Apple silicon), linux-x64 and linux-arm64 (glibc 2.35 or newer)';

const FORWARDED = ['SIGINT', 'SIGTERM', 'SIGHUP'];

/** A host @mobileagent/cli has no binary for, or whose binary is not installed. */
class Unavailable extends Error {}

/** The platform package for Node's `platform` and `arch`, or null when there is none. */
function platformPackage(platform, arch) {
  const key = `${platform}-${arch}`;
  return Object.hasOwn(PLATFORMS, key) ? `@mobileagent/cli-${key}` : null;
}

/** The path of the binary `name` in the platform package of `platform` and
 * `arch`, the package resolved from the directory `from` as `require` would. */
function binaryPath(name, { platform = process.platform, arch = process.arch, from = __dirname } = {}) {
  const host = `${platform}-${arch}`;
  const pkg = platformPackage(platform, arch);
  if (pkg === null) {
    throw new Unavailable(`${name}: no binary for ${host}; @mobileagent/cli supports ${SUPPORTED}`);
  }
  let manifest;
  try {
    manifest = require.resolve(`${pkg}/package.json`, { paths: [from] });
  } catch {
    throw new Unavailable(
      `${name}: ${pkg}, the binaries for ${host}, is not installed; npm installs it with @mobileagent/cli unless optional dependencies are omitted, and @mobileagent/cli supports ${SUPPORTED}`,
    );
  }
  const binary = path.join(path.dirname(manifest), 'bin', name);
  if (!fs.existsSync(binary)) {
    throw new Unavailable(`${name}: ${pkg} has no bin/${name}; install @mobileagent/cli again (it supports ${SUPPORTED})`);
  }
  return binary;
}

/** Runs the binary `name` with this process's arguments and ends this process as it ends. */
function run(name) {
  let binary;
  try {
    binary = binaryPath(name);
  } catch (error) {
    if (!(error instanceof Unavailable)) throw error;
    process.stderr.write(`${error.message}\n`);
    process.exit(1);
  }
  const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });
  const forward = (signal) => child.kill(signal);
  for (const signal of FORWARDED) process.on(signal, forward);
  child.on('error', (error) => {
    process.stderr.write(`${name}: cannot run ${binary}: ${error.message}\n`);
    process.exit(1);
  });
  child.on('exit', (code, signal) => {
    if (signal === null) {
      process.exit(code);
    }
    // Die of the same signal, with the default action back, else exit as a shell reports it.
    for (const forwarded of FORWARDED) process.removeListener(forwarded, forward);
    process.kill(process.pid, signal);
    process.exit(128 + (os.constants.signals[signal] ?? 0));
  });
}

module.exports = { PLATFORMS, Unavailable, binaryPath, platformPackage, run };
