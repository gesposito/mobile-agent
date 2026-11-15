'use strict';
// Task 3.4 of `alpha-release` (design D7): npm/pack.sh, which makes the four
// npm packages of a version from the release's three tarballs, as the
// release workflow's npm-package job runs it. Each platform package holds
// its target's binaries, executable, and the notices from the tarball, with
// the license and the README; @mobileagent/cli holds the launcher; all four
// carry the version, the launcher's optional dependencies too, and list
// exactly those files. A tarball that is missing or whose sums do not match
// packs nothing. The tarballs are faked here, and the checkout's manifests
// stay as they are.
//
//   node --test npm/test/

const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');

const NPM = path.join(__dirname, '..');
const PACK = path.join(NPM, 'pack.sh');
const VERSION = '0.0.0-test.1';
const COMMANDS = ['mobile-agent', 'mobile-agent-daemon', 'mobile-agent-mcp'];
const TARGETS = {
  'darwin-arm64': 'aarch64-apple-darwin',
  'linux-x64': 'x86_64-unknown-linux-gnu',
  'linux-arm64': 'aarch64-unknown-linux-gnu',
};
const npm = { skip: spawnSync('npm', ['--version']).status !== 0 && 'npm is not installed' };

function sha256(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

/** A directory with the release's three tarballs for VERSION, each with
 * its sum beside it, as the build jobs make them; `change` may alter a
 * tarball's directory before it is packed. */
function release(change = () => {}) {
  const dist = fs.mkdtempSync(path.join(os.tmpdir(), 'mobileagent-dist-'));
  for (const target of Object.values(TARGETS)) {
    const name = `mobile-agent-${VERSION}-${target}`;
    const dir = path.join(dist, name);
    fs.mkdirSync(dir);
    for (const command of COMMANDS) {
      fs.writeFileSync(path.join(dir, command), `#!/bin/sh\necho ${command} ${target}\n`, { mode: 0o755 });
    }
    fs.writeFileSync(path.join(dir, 'THIRD-PARTY-NOTICES.md'), `# Notices for ${target}\n`);
    const files = [...COMMANDS, 'THIRD-PARTY-NOTICES.md'];
    fs.writeFileSync(
      path.join(dir, 'SHA256SUMS'),
      files.map((file) => `${sha256(path.join(dir, file))}  ${file}\n`).join(''),
    );
    change(target, dir);
    const tar = spawnSync('tar', ['-czf', `${name}.tar.gz`, name], { cwd: dist });
    assert.equal(tar.status, 0, String(tar.stderr));
    fs.rmSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dist, `${name}.tar.gz.sha256`), `${sha256(path.join(dist, `${name}.tar.gz`))}  ${name}.tar.gz\n`);
  }
  return dist;
}

function pack(dist) {
  const out = fs.mkdtempSync(path.join(os.tmpdir(), 'mobileagent-npm-'));
  const run = spawnSync('bash', [PACK, VERSION, dist, out], { encoding: 'utf8' });
  return { out, run };
}

/** The files of a packed package, each with whether it is executable, and its manifest. */
function contents(tgz) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mobileagent-tgz-'));
  const tar = spawnSync('tar', ['-xzf', tgz, '-C', dir]);
  assert.equal(tar.status, 0, String(tar.stderr));
  const files = [];
  const walk = (at, prefix) => {
    for (const entry of fs.readdirSync(at, { withFileTypes: true })) {
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) {
        walk(path.join(at, entry.name), relative);
      } else {
        const executable = (fs.statSync(path.join(at, entry.name)).mode & 0o111) !== 0;
        files.push(`${relative}${executable ? ' *' : ''}`);
      }
    }
  };
  walk(path.join(dir, 'package'), '');
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'package', 'package.json'), 'utf8'));
  return { dir: path.join(dir, 'package'), files: files.sort(), manifest };
}

test('the four packages of a version, from the release tarballs, each listing exactly its files', npm, () => {
  const templates = Object.fromEntries(
    ['cli', ...Object.keys(TARGETS).map((platform) => `cli-${platform}`)].map((dir) => [
      dir,
      fs.readFileSync(path.join(NPM, dir, 'package.json'), 'utf8'),
    ]),
  );
  const { out, run } = pack(release());
  assert.equal(run.status, 0, `${run.stdout}\n${run.stderr}`);
  const tarballs = [
    `mobileagent-cli-${VERSION}.tgz`,
    ...Object.keys(TARGETS).map((platform) => `mobileagent-cli-${platform}-${VERSION}.tgz`),
  ];
  assert.deepEqual(fs.readdirSync(out).sort(), ['SHA256SUMS', ...tarballs].sort());
  const sums = fs.readFileSync(path.join(out, 'SHA256SUMS'), 'utf8');
  for (const tarball of tarballs) {
    assert.ok(sums.includes(`${sha256(path.join(out, tarball))}  ${tarball}\n`), sums);
    // Each package's list is in the output, for the job's log and summary.
    assert.ok(run.stdout.includes(tarball), run.stdout);
  }

  for (const [platform, target] of Object.entries(TARGETS)) {
    const { dir, files, manifest } = contents(path.join(out, `mobileagent-cli-${platform}-${VERSION}.tgz`));
    assert.deepEqual(files, [
      'LICENSE',
      'README.md',
      'THIRD-PARTY-NOTICES.md',
      'bin/mobile-agent *',
      'bin/mobile-agent-daemon *',
      'bin/mobile-agent-mcp *',
      'package.json',
    ]);
    assert.equal(manifest.name, `@mobileagent/cli-${platform}`);
    assert.equal(manifest.version, VERSION);
    assert.equal(fs.readFileSync(path.join(dir, 'bin', 'mobile-agent-mcp'), 'utf8'), `#!/bin/sh\necho mobile-agent-mcp ${target}\n`);
    assert.equal(fs.readFileSync(path.join(dir, 'THIRD-PARTY-NOTICES.md'), 'utf8'), `# Notices for ${target}\n`);
    assert.equal(
      fs.readFileSync(path.join(dir, 'LICENSE'), 'utf8'),
      fs.readFileSync(path.join(NPM, '..', 'LICENSE'), 'utf8'),
    );
  }

  const { files, manifest } = contents(path.join(out, `mobileagent-cli-${VERSION}.tgz`));
  assert.deepEqual(files, [
    'LICENSE',
    'README.md',
    'bin/mobile-agent-daemon.js *',
    'bin/mobile-agent-mcp.js *',
    'bin/mobile-agent.js *',
    'package.json',
    'shim.js',
  ]);
  assert.equal(manifest.name, '@mobileagent/cli');
  assert.equal(manifest.version, VERSION);
  assert.deepEqual(
    manifest.optionalDependencies,
    Object.fromEntries(Object.keys(TARGETS).map((platform) => [`@mobileagent/cli-${platform}`, VERSION])),
  );

  // The checkout's manifests are left as they are.
  for (const [dir, text] of Object.entries(templates)) {
    assert.equal(fs.readFileSync(path.join(NPM, dir, 'package.json'), 'utf8'), text, dir);
  }
});

test('a missing tarball packs nothing and names it', npm, () => {
  const dist = release();
  const missing = `mobile-agent-${VERSION}-x86_64-unknown-linux-gnu.tar.gz`;
  fs.rmSync(path.join(dist, missing));
  const { out, run } = pack(dist);
  assert.notEqual(run.status, 0, run.stdout);
  assert.ok(run.stderr.includes(missing), run.stderr);
  assert.deepEqual(fs.readdirSync(out), []);
});

test('a tarball whose sum or whose binaries sums do not match packs nothing', npm, () => {
  const tampered = release((target, dir) => {
    if (target === 'aarch64-unknown-linux-gnu') {
      fs.appendFileSync(path.join(dir, 'mobile-agent-daemon'), 'echo changed\n');
    }
  });
  const inside = pack(tampered);
  assert.notEqual(inside.run.status, 0, inside.run.stdout);
  assert.ok(inside.run.stderr.includes('SHA256SUMS'), inside.run.stderr);
  assert.deepEqual(fs.readdirSync(inside.out), []);

  const dist = release();
  const name = `mobile-agent-${VERSION}-aarch64-apple-darwin.tar.gz`;
  fs.writeFileSync(path.join(dist, `${name}.sha256`), `${'0'.repeat(64)}  ${name}\n`);
  const outside = pack(dist);
  assert.notEqual(outside.run.status, 0, outside.run.stdout);
  assert.ok(outside.run.stderr.includes(name), outside.run.stderr);
  assert.deepEqual(fs.readdirSync(outside.out), []);
});

test('the version must be a semantic version without the v', npm, () => {
  const dist = release();
  const out = fs.mkdtempSync(path.join(os.tmpdir(), 'mobileagent-npm-'));
  const run = spawnSync('bash', [PACK, `v${VERSION}`, dist, out], { encoding: 'utf8' });
  assert.notEqual(run.status, 0, run.stdout);
  assert.ok(run.stderr.includes('semantic version'), run.stderr);
  assert.deepEqual(fs.readdirSync(out), []);
});
