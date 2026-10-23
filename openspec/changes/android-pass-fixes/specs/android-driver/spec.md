# Spec Delta

## ADDED Requirements

### Requirement: Transient adb errors are retried
The driver SHALL read and write again on a would-block or an interrupt from the adb socket, bounded by the command's deadline, so that a shell command such as an app launch does not fail on a socket that is not ready; a would-block that outlasts the deadline SHALL be the command's timeout.

#### Scenario: A launch meets a would-block
- **WHEN** the adb socket answers would-block once while `am start` is written or its result read
- **THEN** the launch completes and the reply says the app opened
