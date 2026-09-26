# Spec Delta

## Purpose

Drives Android devices and emulators through a resident agent that runs as the shell user, so that tree reads, input and screenshots come from one persistent process with nothing installed on the device.

## ADDED Requirements

### Requirement: Target discovery
The driver SHALL list connected Android targets with serial, model, OS API level, whether the target is an emulator, and the agent state (absent, running, incompatible). Unauthorized or offline targets SHALL be listed with that state and an actionable message.

#### Scenario: Unauthorized device
- **WHEN** a device is connected but USB debugging has not been accepted on it
- **THEN** the target is listed as unauthorized with instructions to accept the prompt on the device

### Requirement: Agent deployment without installation
On first use of a target the driver SHALL transfer the agent and start it as a shell-user process without installing a package, running an instrumentation, or prompting the user. A later attach SHALL reuse a running agent of a compatible version and SHALL restart an incompatible one.

#### Scenario: First attach
- **WHEN** a target has no agent running
- **THEN** the driver starts one and the first snapshot succeeds without any package being installed on the device

#### Scenario: Compatible agent reused
- **WHEN** a compatible agent is already running from a previous session
- **THEN** the driver attaches to it and no new process is started

#### Scenario: Incompatible agent replaced
- **WHEN** the running agent reports an older protocol version than the daemon requires
- **THEN** the driver stops it and starts the current version

### Requirement: Single ownership of UI automation
The agent SHALL hold the device's UI test automation role exclusively. If another holder is present the driver SHALL report the conflict, naming the likely holder when it can be inferred from the device's process and instrumentation listings, and SHALL NOT take over unless the client passes an explicit force option.

#### Scenario: Another tool holds automation
- **WHEN** another automation tool is active on the device and a client attaches without force
- **THEN** the attach fails with a conflict error naming the likely holder when one can be inferred

### Requirement: Tree acquisition across windows
The driver SHALL return the accessibility tree of every interactive window, including the keyboard and system overlays, with text, content description, resource identifier, class, bounds, and actionability per node. Depth and node limits SHALL be configurable and any truncation SHALL be flagged.

#### Scenario: Keyboard is included
- **WHEN** a text field is focused and the soft keyboard is showing
- **THEN** the tree includes the keyboard window's keys alongside the application window

#### Scenario: Node limit reached
- **WHEN** a screen exceeds the configured node limit
- **THEN** the tree is returned with a truncation flag and the count of omitted nodes

### Requirement: Bounded idle waiting
Tree acquisition SHALL NOT wait for the device to become idle longer than a configurable bound whose default is under one second, and a client SHALL be able to request no wait at all.

#### Scenario: Continuous animation
- **WHEN** a screen contains an endless animation and a snapshot is requested with the default bound
- **THEN** the snapshot returns within the bound plus the read time and is not withheld until the animation stops

### Requirement: Change events
The agent SHALL stream window and content change events that identify the affected subtree, so that the mirror can refresh only what changed.

#### Scenario: Single field changes
- **WHEN** the text of one input field changes
- **THEN** the emitted event identifies that field's subtree and no full-tree refresh is triggered

### Requirement: Input injection
The driver SHALL inject taps, long presses, swipes with explicit duration, multi-step gestures, key presses and text. Each injected action SHALL be acknowledged within a bounded time and the acknowledgement SHALL state whether the platform accepted the event.

#### Scenario: Tap acknowledged
- **WHEN** a tap is injected at valid coordinates
- **THEN** an acknowledgement is returned within 50 ms on an emulator stating the event was accepted

### Requirement: Direct text setting with fallback
For a focused editable element the driver SHALL set its text directly when the platform allows it and SHALL otherwise fall back to key events. The response SHALL state which method was used.

#### Scenario: React Native text input
- **WHEN** a client fills a focused React Native text input with a 40 character string
- **THEN** the text is set in one operation and the response names the direct method

#### Scenario: Element rejects direct setting
- **WHEN** the focused element does not support direct text setting
- **THEN** the text is typed with key events and the response names the fallback

### Requirement: Screenshot capture
The driver SHALL capture the full display as a correctly oriented image within a bounded time.

#### Scenario: Landscape screenshot
- **WHEN** the device is in landscape orientation
- **THEN** the returned image has landscape dimensions and correct orientation

### Requirement: Emulator fast path
When the target is an emulator whose control endpoint is available, input and frame streaming SHALL use that endpoint and the driver SHALL report which path is active. When it is unavailable the driver SHALL fall back to the agent and emit a warning explaining how to enable the endpoint.

#### Scenario: Control endpoint enabled
- **WHEN** an emulator was started with its control endpoint enabled
- **THEN** taps and frames are served through it and the target status names the fast path

#### Scenario: Control endpoint disabled
- **WHEN** an emulator was started without its control endpoint
- **THEN** taps and screenshots are served through the agent and the target status carries a warning with the launch flag to enable it

### Requirement: Agent lifecycle
The driver SHALL stop the agent on explicit request and on daemon shutdown, leaving no process behind. The agent SHALL survive application restarts and screen rotation without being restarted.

#### Scenario: Daemon shutdown
- **WHEN** the daemon exits normally
- **THEN** no agent process remains on the device

#### Scenario: App restarted
- **WHEN** the application under automation is force-stopped and relaunched
- **THEN** the same agent continues to serve snapshots without reattachment
