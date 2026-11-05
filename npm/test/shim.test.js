'use strict';
// Task 3.4 of `alpha-release` (design D7): the launcher behind the commands
// of @mobileagent/cli. It picks the platform package for Node's platform and
// architecture, finds the binary in it and runs it with the arguments and
// the standard streams, and exits with its status; with no platform package
// for the host it prints one line that names the supported platforms. The
// platform packages are faked in a temporary node_modules, where the
// launcher is copied as npm installs it.
//
//   node --test npm/test/

const assert = require('node:assert/strict');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');

const CLI = path.join(__dirname, '..', 'cli');
const shim = require(path.join(CLI, 'shim.js'));

const COMMANDS = ['mobile-agent', 'mobile-agent-daemon', 'mobile-agent-mcp'];
const SUPPORTED = ['darwin-arm64', 'linux-x64', 'linux-arm64'];
const HOST = `${process.platform}-${process.arch}`;

// A binary that prints its own path and its arguments, then does what its
// first argument says: exit with a status, echo a line of its input, kill
// itself, or wait for SIGTERM and exit with 7 once it comes.
const FAKE = `#!/bin/sh
printf '%s\\n' "$0" "$@"
case "$1" in
  exit) exit "$2" ;;
  stdin) read -r line; printf 'read %s\\n' "$line" ;;
  signal) kill -TERM $$ ;;
  trap)
    sleep 30 &
    pid=$!
    trap 'kill $pid; echo terminated; exit 7' TERM
    echo ready
    wait $pid
    ;;
esac
`;

/** A temporary directory with node_modules holding @mobileagent/cli as npm
 * installs it and a fake platform package for each of `platforms`. */
function install(platforms) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'mobileagent-shim-'));
  const scope = path.join(root, 'node_modules', '@mobileagent');
  fs.cpSync(CLI, path.join(scope, 'cli'), { recursive: true });
  for (const platform of platforms) {
    const dir = path.join(scope, `cli-${platform}`);
    fs.mkdirSync(path.join(dir, 'bin'), { recursive: true });
    fs.writeFileSync(
      path.join(dir, 'package.json'),
      JSON.stringify({ name: `@mobileagent/cli-${platform}`, version: '0.0.0-test' }),
    );
    for (const command of COMMANDS) {
      fs.writeFileSync(path.join(dir, 'bin', command), FAKE, { mode: 0o755 });
    }
  }
  return { root, scope };
}

/** The command line that runs `command`'s launcher in the installed tree. */
function launcher(scope, command) {
  return path.join(scope, 'cli', 'bin', `${command}.js`);
}

test('each supported platform and architecture names its platform package', () => {
  assert.equal(shim.platformPackage('darwin', 'arm64'), '@mobileagent/cli-darwin-arm64');
  assert.equal(shim.platformPackage('linux', 'x64'), '@mobileagent/cli-linux-x64');
  assert.equal(shim.platformPackage('linux', 'arm64'), '@mobileagent/cli-linux-arm64');
  for (const [platform, arch] of [
    ['darwin', 'x64'],
    ['win32', 'x64'],
    ['linux', 'ia32'],
    ['freebsd', 'x64'],
  ]) {
    assert.equal(shim.platformPackage(platform, arch), null, `${platform}-${arch}`);
  }
  assert.deepEqual(Object.keys(shim.PLATFORMS), SUPPORTED);
});

test('the binary is the one in the platform package of the platform and architecture', () => {
  const { scope } = install(SUPPORTED);
  const from = path.join(scope, 'cli');
  for (const platform of SUPPORTED) {
    const [os_, arch] = platform.split('-');
    for (const command of COMMANDS) {
      const found = shim.binaryPath(command, { platform: os_, arch, from });
      assert.equal(
        fs.realpathSync(found),
        fs.realpathSync(path.join(scope, `cli-${platform}`, 'bin', command)),
      );
    }
  }
});

test('an unsupported platform or a missing platform package is one line naming the supported platforms', () => {
  const { scope } = install([]);
  const from = path.join(scope, 'cli');
  const cases = [
    ['win32', 'x64'],
    ['darwin', 'x64'],
    ['linux', 'arm64'],
  ];
  for (const [platform, arch] of cases) {
    assert.throws(
      () => shim.binaryPath('mobile-agent-mcp', { platform, arch, from }),
      (error) => {
        assert.ok(error instanceof shim.Unavailable, error.stack);
        assert.ok(!error.message.includes('\n'), error.message);
        assert.ok(error.message.startsWith('mobile-agent-mcp: '), error.message);
        assert.ok(error.message.includes(`${platform}-${arch}`), error.message);
        for (const supported of SUPPORTED) {
          assert.ok(error.message.includes(supported), error.message);
        }
        return true;
      },
      `${platform}-${arch}`,
    );
  }
});

test('with no platform package installed, a command prints the line and exits 1', () => {
  const { scope } = install([]);
  const run = spawnSync(process.execPath, [launcher(scope, 'mobile-agent'), '--version'], {
    encoding: 'utf8',
  });
  assert.equal(run.status, 1, run.stderr);
  assert.equal(run.stdout, '');
  const lines = run.stderr.split('\n').filter(Boolean);
  assert.equal(lines.length, 1, run.stderr);
  for (const supported of SUPPORTED) {
    assert.ok(lines[0].includes(supported), lines[0]);
  }
});

const onHost = { skip: !SUPPORTED.includes(HOST) && `no platform package for ${HOST}` };

test('each command runs its binary from the platform package with the arguments, the input and the status', onHost, () => {
  const { scope } = install([HOST]);
  for (const command of COMMANDS) {
    const run = spawnSync(
      process.execPath,
      [launcher(scope, command), 'exit', '3', 'two words', '--flag'],
      { encoding: 'utf8' },
    );
    assert.equal(run.status, 3, run.stderr);
    const [self, ...args] = run.stdout.trimEnd().split('\n');
    // Its own path is the platform package's, where the CLI and the server look for the daemon.
    assert.equal(
      fs.realpathSync(self),
      fs.realpathSync(path.join(scope, `cli-${HOST}`, 'bin', command)),
    );
    assert.deepEqual(args, ['exit', '3', 'two words', '--flag']);
  }
  const echoed = spawnSync(process.execPath, [launcher(scope, 'mobile-agent-mcp'), 'stdin'], {
    encoding: 'utf8',
    input: 'a line on standard input\n',
  });
  assert.equal(echoed.status, 0, echoed.stderr);
  assert.ok(echoed.stdout.endsWith('read a line on standard input\n'), echoed.stdout);
});

test('a binary killed by a signal ends the command by the same signal', onHost, () => {
  const { scope } = install([HOST]);
  const run = spawnSync(process.execPath, [launcher(scope, 'mobile-agent'), 'signal'], {
    encoding: 'utf8',
  });
  // Killed by it, or, if the signal came back to a launcher still running, 128 plus its number.
  assert.ok(
    run.signal === 'SIGTERM' || run.status === 128 + os.constants.signals.SIGTERM,
    `${run.signal} ${run.status}`,
  );
});

test('SIGTERM sent to the command reaches the binary, whose status the command exits with', onHost, async () => {
  const { scope } = install([HOST]);
  const child = spawn(process.execPath, [launcher(scope, 'mobile-agent-mcp'), 'trap'], {
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  let stderr = '';
  child.stderr.on('data', (chunk) => {
    stderr += chunk;
  });
  const ended = new Promise((resolve) => child.on('exit', (code, signal) => resolve({ code, signal })));
  await new Promise((resolve, reject) => {
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
      if (stdout.includes('ready\n')) resolve();
    });
    child.on('exit', () => reject(new Error(`ended before it was ready: ${stdout} ${stderr}`)));
  });
  child.kill('SIGTERM');
  const { code, signal } = await ended;
  assert.equal(signal, null, stderr);
  assert.equal(code, 7, stderr);
  assert.ok(stdout.endsWith('terminated\n'), stdout);
});
