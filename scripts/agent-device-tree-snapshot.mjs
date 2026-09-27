#!/usr/bin/env node
// Takes one agent-device snapshot of an iOS Simulator app through its XCTest
// runner's recursive tree (the `tree` backend) rather than the Simulator AX
// bridge that agent-device 0.21 uses for local Simulators. The CLI and the
// typed client do not expose the backend choice; the daemon accepts it as the
// `snapshotPreferredBackend` flag. This script builds the request with
// agent-device's client and adds the flag in a custom transport, which writes
// the request to the daemon's socket as one JSON line, as the CLI does.
//
// Usage: node scripts/agent-device-tree-snapshot.mjs <udid> <session> [i|raw|regular] [tree|private-ax]
//
// The session must already hold the app (`agent-device open <bundle> --udid
// <udid> --platform ios --session <session>`), so the daemon is running.
// AGENT_DEVICE_STATE_DIR selects that daemon, as for the CLI; the package is
// found with `npm root -g` unless AGENT_DEVICE_ROOT names the directory that
// holds it. Prints one JSON line: ok, wallMs (the request's round trip),
// nodeCount, the capture's snapshotQuality and the nodes, or the error.
// Validated with agent-device 0.21.14; the flag is internal to agent-device.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [udid, session, mode = 'i', backend = 'tree'] = process.argv.slice(2);
if (!udid || !session || !['i', 'raw', 'regular'].includes(mode)) {
  console.error('usage: agent-device-tree-snapshot.mjs <udid> <session> [i|raw|regular] [tree|private-ax]');
  process.exit(2);
}

const root = process.env.AGENT_DEVICE_ROOT ?? execFileSync('npm', ['root', '-g'], { encoding: 'utf8' }).trim();
const { createAgentDeviceClient } = await import(
  pathToFileURL(path.join(root, 'agent-device', 'dist', 'src', 'index.js')).href
);
const stateDir = process.env.AGENT_DEVICE_STATE_DIR ?? path.join(os.homedir(), '.agent-device');
const daemon = JSON.parse(fs.readFileSync(path.join(stateDir, 'daemon.json'), 'utf8'));

function send(request) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: '127.0.0.1', port: daemon.port }, () => {
      socket.write(`${JSON.stringify(request)}\n`);
    });
    let buffer = '';
    socket.on('data', (chunk) => {
      buffer += chunk;
      for (let end = buffer.indexOf('\n'); end >= 0; end = buffer.indexOf('\n')) {
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + 1);
        if (!line.trim()) continue;
        const message = JSON.parse(line);
        // The reply is the first line with an `ok` field; other lines report progress.
        if ('ok' in message) {
          socket.end();
          resolve(message);
        }
      }
    });
    socket.on('error', reject);
  });
}

const transport = async (request) => {
  const flags =
    request.command === 'snapshot' ? { ...request.flags, snapshotPreferredBackend: backend } : request.flags;
  return await send({ ...request, flags, token: daemon.token });
};

const client = createAgentDeviceClient({ session, cost: true }, { transport });
const started = performance.now();
try {
  const result = await client.capture.snapshot({
    udid,
    platform: 'ios',
    session,
    interactiveOnly: mode === 'i',
    raw: mode === 'raw',
  });
  const { nodes, ...rest } = result;
  const wallMs = Math.round(performance.now() - started);
  console.log(JSON.stringify({ ok: true, wallMs, nodeCount: nodes.length, ...rest, nodes }));
} catch (error) {
  const wallMs = Math.round(performance.now() - started);
  const { code, message, details } = error;
  console.log(JSON.stringify({ ok: false, wallMs, error: { code, message, details } }));
  process.exitCode = 1;
}
