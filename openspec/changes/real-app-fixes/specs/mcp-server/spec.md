# Spec Delta

## MODIFIED Requirements

### Requirement: Long waits bounded and cancellable
A tool that waits SHALL send progress notifications while it waits, SHALL end at a per-call bound of 55 s by default (configurable), SHALL pass the bound to the daemon only as a cap on a command's timeout and never as a default above the daemon's own, and a cancelled call SHALL cancel the command in flight in the daemon so the target is free for the next call.

#### Scenario: Wait past the bound
- **WHEN** a client waits for text that never appears with a `timeout` above the bound
- **THEN** progress arrives while it waits, the call ends at the bound with `wait_timeout` and the screen's summary, and the next call runs at once

#### Scenario: A click on a missing element
- **WHEN** a client clicks a selector that matches nothing and gives no `timeout`
- **THEN** the call fails with `element_not_found` at the daemon's default timeout, not at the bound
