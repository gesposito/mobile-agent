# Spec Delta

## Purpose
The Model Context Protocol surface over the daemon: a stdio server an agent client starts, exposing the daemon's actions as tools with the same refs, replies, codes and sessions the CLI has, sized for a model's context.

## ADDED Requirements

### Requirement: Tools over the daemon
The server SHALL expose one tool per daemon action an agent drives (every action but `set_timeout`, `shutdown` and `cancel`), under agent-browser's stems where the concepts match, with flat input schemas generated from the shared request types, SHALL send each call as one request on the daemon's socket in the session the server owns, and SHALL return the reply's text (a listing without the refs map, the CLI's lines for the rest, any `warning` kept) as text content and a failure as an execution error carrying the daemon's code and message, so that a model sees what the CLI's user sees.

#### Scenario: Snapshot through the server
- **WHEN** a client calls the snapshot tool with `interactive` on a target
- **THEN** the result's text is the interactive listing with its refs, and the refs map is not sent

#### Scenario: Failure carries the code
- **WHEN** a click names a ref the screen no longer holds
- **THEN** the result is an execution error whose text names `stale_ref` and says to take a new snapshot

### Requirement: Sessions and claims
The server SHALL mint one session per process, SHALL accept an optional `session` argument on every tool for a client that resumes a transcript, and SHALL release its claim on exit without stopping the app, so that the next session does not meet `target_claimed`.

#### Scenario: Server exits
- **WHEN** the client ends the server's process while an app it opened runs
- **THEN** the claim is released, the app keeps running, and a new server attaches to the target at once

### Requirement: Screenshots inline
The screenshot tool SHALL return the image as image content by default, bounded to a long edge of 1000 px, SHALL skip the image and say so when `ifChanged` is set and the frames have not changed since the last screenshot of the session, and SHALL offer a path instead on request.

#### Scenario: Unchanged screen
- **WHEN** a client asks for a screenshot with `ifChanged` twice on a still screen
- **THEN** the first result carries the image and the second a text saying the screen is unchanged

### Requirement: Long waits bounded and cancellable
A tool that waits SHALL send progress notifications while it waits, SHALL end at a per-call bound of 55 s by default (configurable), and a cancelled call SHALL cancel the command in flight in the daemon so the target is free for the next call.

#### Scenario: Wait past the bound
- **WHEN** a client waits for text that never appears with the default bound
- **THEN** progress arrives while it waits, the call ends at 55 s with `wait_timeout` and the screen's summary, and the next call runs at once

### Requirement: Profiles and instructions
The server SHALL offer a `--tools` option with a read-only profile (devices, status, doctor, snapshot, diff_snapshot, get, wait, screenshot, help) and the full set, SHALL give the client an instructions card whose first 512 characters stand alone and whose whole stays under 4,096, and SHALL serve the agent guide through a `help` tool.

#### Scenario: Read-only profile
- **WHEN** the server starts with the read-only profile
- **THEN** the client lists no tool that acts on the device, and the instructions say so
