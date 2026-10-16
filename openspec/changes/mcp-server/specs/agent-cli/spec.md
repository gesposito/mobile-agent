# Spec Delta

## ADDED Requirements

### Requirement: Change waits, claim release and cancel
`wait --change` SHALL return when the replica's revision or the frames of the target move, with the delta since the session's last snapshot, or `wait_timeout` at its bound; `close --keep` SHALL release the session's claim without stopping the app; `cancel` SHALL end the session's command in flight, which then fails with `cancelled`, so a target is not held by a wait a client gave up on. Requests and replies SHALL be built from the shared types whose schemas the CLI, the harness and the MCP server use.

#### Scenario: Wait for the next change
- **WHEN** `wait --change` is called while a tap's effect is still arriving
- **THEN** it returns at the first change with the delta, and on a still screen it returns `wait_timeout` at its bound

#### Scenario: Close and keep
- **WHEN** a session closes with `--keep` while its app runs
- **THEN** the claim is released, the app keeps running, and another session can attach
