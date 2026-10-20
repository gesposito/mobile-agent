# Spec Delta

## ADDED Requirements

### Requirement: Change waits, claim release and cancel
`wait --change` SHALL return when the session's listing, in the options of its last snapshot, differs from the one that snapshot showed, at once when it changed before the call, with the delta `snapshot --delta` gives and what moved, or `wait_timeout` at its bound, a change the listing does not show leaving it waiting; with `--frames` a frame change SHALL end it too; `close --keep` SHALL release the session's claim without stopping the app; `cancel` SHALL end the session's command in flight, which then fails with `cancelled`, so a target is not held by a wait a client gave up on, a device call in flight running to its end first. Requests and replies SHALL be built from the shared types whose schemas the CLI, the harness and the MCP server use.

#### Scenario: Wait for the next change
- **WHEN** `wait --change` is called while a tap's effect is still arriving
- **THEN** it returns at the first change with the delta and what moved, and on a still screen it returns `wait_timeout` at its bound

#### Scenario: Change before the wait
- **WHEN** the screen changed after the session's last snapshot and before `wait --change`
- **THEN** it returns at once with the delta

#### Scenario: A change outside the listing
- **WHEN** the replica changes outside the session's interactive listing, or only the frames move, during `wait --change`
- **THEN** it keeps waiting and returns `wait_timeout` at its bound

#### Scenario: Frames on request
- **WHEN** only the frames move during `wait --change --frames`
- **THEN** it returns with `endedBy` `frame` and the delta `unchanged`

#### Scenario: Close and keep
- **WHEN** a session closes with `--keep` while its app runs
- **THEN** the claim is released, the app keeps running, and another session can attach

#### Scenario: Cancel a wait
- **WHEN** a session's wait runs and the session's `cancel` arrives on another connection
- **THEN** the wait fails with `cancelled` within about 100 ms, `cancel` replies that it ended it, and the target serves the next command at once

#### Scenario: Cancel during a tap
- **WHEN** `cancel` arrives while a click's tap is being delivered
- **THEN** the tap goes in and the click completes, its watch cut short with a warning
