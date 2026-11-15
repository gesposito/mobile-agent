'use strict';
// Task 3.4 of `alpha-release` (design D7): the manifests of the four npm
// packages. @mobileagent/cli holds the launcher and its three commands, with
// no dependency but the three platform packages, optional and at its own
// version; each platform package holds one target's binaries, for npm to
// install where its os, cpu and libc match. None has a script, so nothing
// runs at install, and each carries the workspace's version and the
// repository that npm's provenance names.
//
//   node --test npm/test/

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');

const NPM = path.join(__dirname, '..');
const ROOT = path.join(NPM, '..');
const COMMANDS = ['mobile-agent', 'mobile-agent-daemon', 'mobile-agent-mcp'];
// Each platform package, its os, cpu and libc, and the release's target whose tarball fills it.
const PLATFORMS = [
  ['darwin-arm64', 'darwin', 'arm64', undefined, 'aarch64-apple-darwin'],
  ['linux-x64', 'linux', 'x64', ['glibc'], 'x86_64-unknown-linux-gnu'],
  ['linux-arm64', 'linux', 'arm64', ['glibc'], 'aarch64-unknown-linux-gnu'],
];

function manifest(dir) {
  return JSON.parse(fs.readFileSync(path.join(NPM, dir, 'package.json'), 'utf8'));
}

/** `[workspace.package]`'s version in the root Cargo.toml. */
function workspaceVersion() {
  const cargo = fs.readFileSync(path.join(ROOT, 'Cargo.toml'), 'utf8');
  const section = cargo.split('\n[workspace.package]\n')[1].split('\n[')[0];
  return section.match(/^version = "([^"]+)"$/m)[1];
}

function repository(dir) {
  return {
    type: 'git',
    url: 'git+https://github.com/gesposito/mobile-agent.git',
    directory: `npm/${dir}`,
  };
}

test('the npm directory holds the four packages and their tests alone', () => {
  const dirs = fs
    .readdirSync(NPM, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
  assert.deepEqual(dirs, ['cli', 'cli-darwin-arm64', 'cli-linux-arm64', 'cli-linux-x64', 'test']);
});

test('@mobileagent/cli runs its three commands through the launcher and installs nothing but a platform package', () => {
  const version = workspaceVersion();
  const cli = manifest('cli');
  assert.equal(cli.name, '@mobileagent/cli');
  assert.equal(cli.version, version);
  assert.equal(cli.license, 'MIT');
  assert.deepEqual(cli.repository, repository('cli'));
  assert.match(cli.description, /Experimental, pre-release/);
  assert.deepEqual(
    cli.bin,
    Object.fromEntries(COMMANDS.map((command) => [command, `bin/${command}.js`])),
  );
  assert.deepEqual(cli.files, ['bin/', 'shim.js']);
  assert.deepEqual(cli.engines, { node: '>=22' });
  assert.deepEqual(
    cli.optionalDependencies,
    Object.fromEntries(PLATFORMS.map(([platform]) => [`@mobileagent/cli-${platform}`, version])),
  );
  for (const field of ['dependencies', 'peerDependencies', 'devDependencies', 'bundleDependencies', 'main', 'type']) {
    assert.equal(cli[field], undefined, field);
  }
  for (const command of COMMANDS) {
    const entry = path.join(NPM, 'cli', 'bin', `${command}.js`);
    const text = fs.readFileSync(entry, 'utf8');
    assert.ok(text.startsWith('#!/usr/bin/env node\n'), entry);
    assert.ok(text.includes(`.run('${command}')`), entry);
    assert.notEqual(fs.statSync(entry).mode & 0o111, 0, `${entry} is executable`);
  }
});

test('each platform package holds one target binaries for npm to install where its os, cpu and libc match', () => {
  const version = workspaceVersion();
  const shim = require(path.join(NPM, 'cli', 'shim.js'));
  for (const [platform, os, cpu, libc, target] of PLATFORMS) {
    const dir = `cli-${platform}`;
    const pkg = manifest(dir);
    assert.equal(pkg.name, `@mobileagent/cli-${platform}`);
    assert.equal(pkg.version, version, dir);
    assert.equal(pkg.license, 'MIT', dir);
    assert.deepEqual(pkg.repository, repository(dir));
    assert.deepEqual(pkg.os, [os], dir);
    assert.deepEqual(pkg.cpu, [cpu], dir);
    assert.deepEqual(pkg.libc, libc, dir);
    assert.deepEqual(pkg.files, [...COMMANDS.map((command) => `bin/${command}`), 'THIRD-PARTY-NOTICES.md'], dir);
    assert.equal(pkg.preferUnplugged, true, dir);
    assert.ok(pkg.description.includes(target), pkg.description);
    for (const field of ['bin', 'dependencies', 'optionalDependencies', 'main']) {
      assert.equal(pkg[field], undefined, `${dir} ${field}`);
    }
    assert.equal(shim.PLATFORMS[platform], target, platform);
    // The binaries come from the release's tarball when the workflow packs it, never from the checkout.
    assert.ok(!fs.existsSync(path.join(NPM, dir, 'bin')), `${dir}/bin is not tracked`);
  }
});

test('no package has a script, so nothing runs when one is installed or packed', () => {
  for (const dir of ['cli', ...PLATFORMS.map(([platform]) => `cli-${platform}`)]) {
    const pkg = manifest(dir);
    assert.equal(pkg.scripts, undefined, dir);
    assert.equal(pkg.gypfile, undefined, dir);
    assert.ok(!fs.existsSync(path.join(NPM, dir, 'binding.gyp')), dir);
    assert.ok(fs.existsSync(path.join(NPM, dir, 'README.md')), `${dir} has a README`);
  }
});
