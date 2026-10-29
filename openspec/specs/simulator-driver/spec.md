# simulator-driver Specification

## Purpose
Drives iOS Simulators entirely from the host so that input lands in milliseconds, trees are read without a test runner, and frames stream live, with nothing built or installed inside the Simulator.

## Requirements

### Requirement: Simulator discovery and lifecycle
The driver SHALL list Simulators with identifier, name, runtime version and state, and SHALL boot, shut down, install an application bundle, launch an application by bundle identifier with environment and arguments, and terminate it.

#### Scenario: Launch with environment
- **WHEN** a client launches an app with a custom environment variable
- **THEN** the app starts and the variable is visible to the process

### Requirement: No test runner and no per-app build
Tree reads, input and frame capture SHALL work on a freshly created Simulator without building or installing any application, test runner or helper into it and without invoking build tools per app. A helper process spawned into the Simulator from the host is permitted only if it needs no installation, no code signing and no per-app build, and is ready to serve within 500 ms of being spawned; its presence and readiness time SHALL be reported in the target status.

#### Scenario: Fresh Simulator
- **WHEN** a Simulator is created, booted, and an app launched, with no build tools invoked
- **THEN** a snapshot, a tap and a frame all succeed

#### Scenario: Spawned helper is reported
- **WHEN** the driver spawns a helper into the Simulator to read the tree
- **THEN** the target status names the helper and its readiness time, and no application or runner was installed

### Requirement: Foreground tree read
The driver SHALL read the foreground application's accessibility tree with identifiers, labels, values, roles and frames, SHALL mark the read as degraded when the tree is sparse or truncated, and SHALL report when the foreground is owned by system UI rather than the application.

#### Scenario: System alert in front
- **WHEN** a permission alert covers the application
- **THEN** the response states that system UI owns the foreground and lists the alert's buttons when they can be read

#### Scenario: Sparse tree flagged
- **WHEN** the read returns only a root for a visibly populated screen
- **THEN** the snapshot is marked degraded with the reason

### Requirement: Input transport selection
The driver SHALL select the input transport from the host's CoreSimulator and Xcode versions, because the host injects the same input service into every booted Simulator, and SHALL NOT probe a transport in a way that disables another for the rest of the boot. When no transport works the driver SHALL fail with a diagnostic naming the CoreSimulator, Xcode and runtime versions and the transports tried.

#### Scenario: Two runtimes on one host
- **WHEN** two Simulators with different runtime versions are booted on one host
- **THEN** both receive input through the transport selected for that host and each target status names it

#### Scenario: Unsupported combination
- **WHEN** no transport works for the host and runtime
- **THEN** input commands fail with a diagnostic naming the versions and the transports tried

### Requirement: Input waits for the transport's activation
The driver SHALL open the input session and wait for the transport's activation reply before the first action, queueing input meanwhile, so that no event is dropped.

#### Scenario: Input before activation
- **WHEN** a tap is requested immediately after the input session is opened
- **THEN** the tap is delivered once activation completes and the response reports the activation wait

### Requirement: Input actions
The driver SHALL support taps, long presses, swipes with velocity control, multi-finger gestures, hardware buttons including home and lock, key presses and text typing. On a warm session a tap SHALL be dispatched within 20 ms of the request.

#### Scenario: Warm tap latency
- **WHEN** a tap is requested on a session that has already dispatched input
- **THEN** the input is delivered to the Simulator within 20 ms

### Requirement: Frames and screenshots
The driver SHALL provide frames from the display surface up to the display refresh rate and screenshots on demand, each with a timestamp.

#### Scenario: Frame stream during animation
- **WHEN** a client subscribes to frames while a transition plays
- **THEN** new frames arrive as the display updates and stop arriving when the display is static

### Requirement: Text entry
The driver SHALL type text through keyboard events and SHALL support paste-based entry for long strings, reporting which method was used. When the field was focused by the same action, the driver SHALL wait, bounded, until the keyboard window is up before the first key, so that keys sent while the keyboard slides in are not dropped.

#### Scenario: Long string
- **WHEN** a client enters a 500 character string
- **THEN** the text appears in the focused field and the response names the method used

#### Scenario: First keystroke lands
- **WHEN** `fill` focuses a field and types a 20-character string starting with a lower-case letter
- **THEN** the field reads back the whole string, first letter included, in every one of ten runs

### Requirement: Typing cadence per app technology
The cadence of typed keys SHALL be one a field of each app technology takes without a loss, decided by measurement: a React Native field, which takes keys through its JavaScript thread, MAY be typed more slowly than a native one.

#### Scenario: React Native field typed whole
- **WHEN** `fill` types the 36-character entry into a React Native field twenty times at the cadence chosen for it
- **THEN** every run reads back whole

### Requirement: A fill over a selection waits for the emptied field
When `fill` types an entry into a field whose text it selected to replace, it SHALL delete the selection and wait, bounded, for a read that shows the field empty before the first key, since autocorrection rewrites the first word of keys typed over the selection, its capital lost, and a refill of the same text could not otherwise be read back.

#### Scenario: Capital first letter over what the field held
- **WHEN** `fill` types "Smoke Org" ten times into a field with autocorrection on that capitalizes words, each time over the entry the field held and after the focus was in another field
- **THEN** every run reads back whole, typed, without the paste retry

### Requirement: Parallel Simulators
One daemon SHALL drive several Simulators at once with fully isolated sessions.

#### Scenario: Two sessions
- **WHEN** two clients drive two different Simulators concurrently
- **THEN** neither session's input, frames or refs leak into the other

### Requirement: Failure containment
A failure inside the private framework layer SHALL not terminate the daemon. The affected target SHALL be marked unavailable with diagnostics and other targets SHALL continue to work.

#### Scenario: Framework call throws
- **WHEN** a tree read raises an exception in the framework layer
- **THEN** the request fails with diagnostics and the daemon keeps serving other targets

### Requirement: Xcode compatibility statement
The driver SHALL report the Xcode version range it supports and SHALL refuse to operate outside it with a message naming the detected and supported versions.

#### Scenario: Unsupported Xcode
- **WHEN** the installed Xcode is newer than the supported range
- **THEN** commands fail with a message naming both versions and pointing to the compatibility notes
