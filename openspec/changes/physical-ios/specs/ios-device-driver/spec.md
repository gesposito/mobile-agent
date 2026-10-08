# Spec Delta

## Purpose

Drives a physical iOS device over USB from the host: discovers and readies it, keeps one resident runner on it for a session, and serves the device protocol's tree, input, frames, screenshots and app lifecycle to the daemon, so that a phone is a target like a Simulator or an Android device.

## ADDED Requirements

### Requirement: Device discovery and readiness
The driver SHALL list every iOS device attached over USB with its identifier, name, model and iOS version, and SHALL report for each what stands between it and a session: not paired or not trusted, Developer Mode off, the developer disk image not mounted, the device locked, no signing identity for the runner, or ready. `doctor` SHALL name the step the user takes for each unready state, and a command on an unready device SHALL fail with a code naming that state rather than waiting.

#### Scenario: Ready device listed
- **WHEN** a trusted iPhone with Developer Mode on is plugged in and the signing identity is available
- **THEN** `devices` lists it as ready with its identifier, name, model and iOS version

#### Scenario: Device not ready
- **WHEN** the iPhone is plugged in with Developer Mode off
- **THEN** `devices` lists it as not ready naming Developer Mode, `doctor` says how to turn it on, and a `snapshot` on it fails with a code naming the state

### Requirement: Transport without root or helpers
The driver SHALL reach the device's developer services from the daemon's own process, through the pairing the host holds and the device's tunnel, without root, a network interface of its own, or a helper process in another language, and SHALL mount the developer disk image itself when the device lacks one.

#### Scenario: Services reached from the daemon
- **WHEN** the daemon attaches to a ready device as an unprivileged user
- **THEN** it launches an app and takes a screenshot through the device's developer services with no other process involved

### Requirement: One resident runner per session
The driver SHALL deploy a runner bundle signed for the host's team, start it through the device's test manager once per session, keep it running across every command of the session, and stop it when the target is released or the daemon stops; a runner of another version SHALL be replaced before use, and a runner that dies SHALL be restarted once and the restart reported in the target's status. The runner SHALL speak the device protocol over a socket the host reaches through the device's connection, with the capabilities it has.

#### Scenario: Runner stays up
- **WHEN** fifty commands run against the device in one session
- **THEN** the same runner process serves all of them and the target's status reports no restart

#### Scenario: Runner replaced on version change
- **WHEN** the installed runner's version differs from the one built beside the daemon
- **THEN** the driver installs the built one before starting the session

### Requirement: Foreground tree read
The runner SHALL read the foreground application's accessibility tree in one request, with the same windows, identifiers, labels, values, traits, bounds in device pixels and scale, keyboard window and modal views as the Simulator's reader gives, and SHALL mark every window as sending no change events, so the mirror re-reads it on frame changes and polls; a read of a system screen SHALL be served from the system process's tree with its owner named.

#### Scenario: Reference screen read
- **WHEN** the fixture's 500-cell screen is in front
- **THEN** one request returns every cell with its identifier and bounds, and the mirror lists the same refs it lists on the Simulator

### Requirement: Input actions without idle waits
The runner SHALL synthesize taps, long presses, swipes that rest before lifting, key presses and typed text at device coordinates without waiting for the application to go idle, SHALL report each as accepted once the event is synthesized, and SHALL enter text into the focused field by typing or by setting its value where typing cannot, reporting which.

#### Scenario: Tap dispatched
- **WHEN** a tap is requested at a point
- **THEN** the runner acknowledges it once the event is synthesized, without an idle wait, and the frame stream shows the screen change afterwards

### Requirement: Frames and screenshots from the host
The driver SHALL take a frame stream of the device's display from the host while a target is attached, hash it as the Simulator driver hashes its frames so that settle and the mirror's re-reads work from it, and SHALL serve screenshots on demand at full resolution and at a requested long edge and quality.

#### Scenario: Frame change after a tap
- **WHEN** a tap changes the screen
- **THEN** the mirror sees a frame hash change and reports its time, and `settle` ends a quiet window after the last change

### Requirement: App lifecycle
The driver SHALL install an app from a signed bundle, launch it by bundle identifier with arguments and environment, terminate it, and report whether it is running, through the device's developer services.

#### Scenario: Fixture launched
- **WHEN** `open` names the fixture's bundle identifier
- **THEN** the app is in front within the attach budget and the tree read that follows is the app's

### Requirement: Failure containment
A device that is unplugged, locked or rebooted SHALL leave the target disconnected with a code and message saying so, every other target unaffected, and a session that reaches it again SHALL start over with fresh refs; a command on a device whose runner is restarting SHALL wait for the restart up to its timeout rather than fail at once.

#### Scenario: Cable pulled
- **WHEN** the device is unplugged during a session
- **THEN** the next command fails with `disconnected`, the daemon keeps serving its other targets, and plugging the device back in lets a new attach succeed

### Requirement: Compatibility statement
The driver SHALL record, for every private or undocumented interface it uses on the device or in the runner, the iOS and Xcode versions it was validated on, in the compatibility table `doctor` prints, and SHALL refuse with a message naming the version rather than fail silently where a version it does not know changes an interface.

#### Scenario: Table lists the device paths
- **WHEN** `doctor` runs on the host
- **THEN** the compatibility table lists each device-side interface with the iOS and Xcode versions it was validated on
