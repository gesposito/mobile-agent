# Spec Delta

## MODIFIED Requirements

### Requirement: Windows without change events
For a window whose toolkit sends no change events to the driver, the mirror SHALL re-read that window in full after every action it injected and on a periodic poll, SHALL derive the window's deltas by diffing consecutive reads, and SHALL report the read's cost and age in the snapshot's metadata so consumers can tell a fresh tree from a polled one. On a target whose frames are a change signal and whose reads cost more than a frame interval, the poll SHALL re-read only once the frames have changed since the read before, or once a long bound has passed, so that an idle target is not kept reading.

#### Scenario: Action on an event-less window
- **WHEN** a client taps a control in a window that emits no change events and then requests a snapshot
- **THEN** the snapshot reflects the tap's effect, its metadata names a full re-read as the source, and the elapsed read time is included

#### Scenario: Idle event-less window
- **WHEN** no action has been injected and the poll interval elapses
- **THEN** the window is re-read once, and an unchanged tree produces no delta events

#### Scenario: Idle event-less window with still frames
- **WHEN** a physical iPhone sits idle for ten seconds with its frames unchanged
- **THEN** the mirror makes no poll re-read in that time, and a command arriving then is not kept waiting behind a read
