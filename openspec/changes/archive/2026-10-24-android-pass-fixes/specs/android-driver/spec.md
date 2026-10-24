# Spec Delta

## ADDED Requirements

### Requirement: A launch takes the command's deadline
The driver SHALL wait for an app's launch (`am start -W`) until the deadline of the command that launches it, `open`'s start plus its timeout, rather than the adb client's default read timeout, so that a cold launch slower than that read timeout but within the command's deadline completes; a launch that outlasts the deadline SHALL fail as a timeout that names the launch and the deadline, never as the socket's error.

#### Scenario: A cold launch slower than the read timeout
- **WHEN** `am start -W` answers after the adb client's default read timeout but before `open`'s deadline
- **THEN** the launch completes and the reply says the app opened

#### Scenario: A launch past the deadline
- **WHEN** `am start -W` has not answered by `open`'s deadline
- **THEN** `open` fails with `deadline_exceeded`, its message naming the launch and the deadline
