# Spec Delta

## ADDED Requirements

### Requirement: A refused launch is retried once
When the Simulator refuses a launch with `NSPOSIXErrorDomain 3`, the driver SHALL retry the launch once after a bounded wait and say so in the reply, and SHALL fail as before when the retry is refused too; no other launch failure is retried.

#### Scenario: A launch right after a termination
- **WHEN** `open --relaunch` follows the app's termination and the Simulator refuses the first launch
- **THEN** the driver retries once after its wait, the app opens, and the reply says it retried
