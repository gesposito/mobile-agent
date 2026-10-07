# agent-cli Specification

## Purpose
Gives agents and humans one command surface for devices whose grammar, ref syntax and JSON output follow agent-browser, so that skills and prompts written for the web tool transfer to devices unchanged where the concepts overlap.

## Requirements

### Requirement: Command grammar
The CLI SHALL provide at least these commands: `devices`, `open`, `close`, `snapshot` with an interactive flag, `click` by ref or coordinates, `fill`, `type`, `press`, `swipe`, `scroll`, `screenshot`, `wait` by selector or settle, `get text`, `diff snapshot`, `batch`, `status`, and `doctor`. Snapshot output SHALL list one element per line with role, name and ref.

#### Scenario: Interactive snapshot output
- **WHEN** a client runs an interactive snapshot on a screen with a heading and two buttons
- **THEN** the output lists a heading line and two button lines, each ending with a ref

### Requirement: Ref syntax
Elements SHALL be addressed as `@e` followed by a number as printed by snapshot. Acting on a stale ref SHALL fail with an error that says to re-snapshot.

#### Scenario: Stale ref
- **WHEN** a client clicks a ref from a snapshot taken before navigation
- **THEN** the command fails with a stale-ref error and guidance

### Requirement: Output formats
The CLI SHALL print human-readable output by default and structured JSON with a stable schema when the JSON flag is given. Errors SHALL go to standard error with a non-zero exit code and SHALL include a machine-readable code in JSON mode.

#### Scenario: JSON error
- **WHEN** a command fails in JSON mode
- **THEN** standard output carries a JSON object with an error code and message and the exit status is non-zero

### Requirement: Daemon lifecycle
The first command SHALL start the daemon when none is running. CLI process overhead, measured as time from process start to daemon request, SHALL stay under 20 ms. The daemon SHALL exit after a configurable idle period and `status` SHALL report the daemon and its targets.

#### Scenario: Cold first command
- **WHEN** no daemon is running and a client runs `devices`
- **THEN** a daemon starts, the command succeeds, and a second command reuses the daemon

### Requirement: Target selection
The CLI SHALL accept platform and device flags. With one connected target it SHALL be selected by default; with several and no flag the command SHALL fail listing the candidates.

#### Scenario: Ambiguous target
- **WHEN** two targets are connected and no device flag is given
- **THEN** the command fails and lists both targets with their identifiers

### Requirement: Sessions
A session flag SHALL isolate target claims and refs between concurrent clients. A target claimed by one session SHALL refuse commands from another session unless a force flag is given.

#### Scenario: Claimed target
- **WHEN** session A holds a target and session B sends a command to it without force
- **THEN** session B's command fails naming session A as the holder

### Requirement: Batch execution
The CLI SHALL run several commands from one invocation in order, SHALL support stopping at the first error, and SHALL return one aggregate result in JSON mode.

#### Scenario: Batch with failure
- **WHEN** a batch of three commands is run with stop-on-error and the second fails
- **THEN** the third is not executed and the aggregate result marks the first as succeeded and the second as failed

### Requirement: Configurable timeouts
Every waiting command SHALL accept a timeout, defaults SHALL be documented in help output, and a session-level default SHALL be settable.

#### Scenario: Per-command timeout
- **WHEN** a wait is run with a 2 s timeout on an element that never appears
- **THEN** the command fails after 2 s, not after the session default

### Requirement: Agent-facing help
Each command SHALL provide help with examples, and a dedicated help command SHALL print a compact guide suitable for inclusion in an agent prompt.

#### Scenario: Prompt guide
- **WHEN** a client asks for the agent guide
- **THEN** the output fits in under 120 lines and covers the snapshot-act-resnapshot loop, refs and settle
