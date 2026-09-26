# Spec Delta

## Purpose

Keeps an always-current, locally queryable replica of a device's user interface so that agents get snapshots, waits and diffs from host state instead of paying a device round trip and an idle wait for every question.

## ADDED Requirements

### Requirement: Replica maintained from device events
For each attached target the mirror SHALL maintain the current set of windows, the accessibility tree and the latest frame, updated from change events pushed by the device driver. Once warm, serving a snapshot SHALL NOT require a device round trip.

#### Scenario: Warm snapshot is served locally
- **WHEN** the replica is warm and no change events are pending
- **THEN** a snapshot request is answered from the replica without contacting the device

#### Scenario: First snapshot after attach
- **WHEN** a target is attached and no replica exists yet
- **THEN** the mirror performs a full acquisition, then serves the snapshot and begins applying events

### Requirement: Windows without change events
For a window whose toolkit sends no change events to the driver, the mirror SHALL re-read that window in full after every action it injected and on a periodic poll, SHALL derive the window's deltas by diffing consecutive reads, and SHALL report the read's cost and age in the snapshot's metadata so consumers can tell a fresh tree from a polled one.

#### Scenario: Action on an event-less window
- **WHEN** a client taps a control in a window that emits no change events and then requests a snapshot
- **THEN** the snapshot reflects the tap's effect, its metadata names a full re-read as the source, and the elapsed read time is included

#### Scenario: Idle event-less window
- **WHEN** no action has been injected and the poll interval elapses
- **THEN** the window is re-read once, and an unchanged tree produces no delta events

### Requirement: Snapshot content
A snapshot SHALL list elements with role, accessible name, test identifier when present, value, bounds, enabled, focused and selected state, and a ref. An interactive snapshot SHALL include only actionable elements and SHALL summarize actionable content that is scrolled out of view.

#### Scenario: Interactive snapshot of a scrolling list
- **WHEN** an interactive snapshot is requested on a list whose last three buttons are below the fold
- **THEN** visible actionable elements are listed with refs and a hint states that three actionable items are off-screen below

### Requirement: Stable refs
A ref SHALL stay attached to the same element across consecutive snapshots while that element survives. A ref whose element no longer exists SHALL be rejected with a stale-ref error and SHALL NOT resolve to a different element.

#### Scenario: Element survives between snapshots
- **WHEN** a button received ref e4 and the screen changes elsewhere
- **THEN** the next snapshot lists the same button as e4

#### Scenario: Stale ref is refused
- **WHEN** a client acts on a ref whose element was removed
- **THEN** the action fails with a stale-ref error and guidance to re-snapshot

### Requirement: Single coordinate contract
All bounds and coordinates SHALL be expressed in device points with the scale factor reported alongside, and the same space SHALL be used for element bounds and for coordinate-based actions.

#### Scenario: Tapping reported bounds hits the element
- **WHEN** a client taps the center of an element's reported bounds
- **THEN** the element receives the tap

### Requirement: Settle predicate
The mirror SHALL provide a settle wait that resolves when no tree deltas and no frame changes have occurred for a configurable quiet window, bounded by a configurable maximum. When the maximum elapses the result SHALL state that the screen is still changing and why, rather than reporting an error.

#### Scenario: Screen settles
- **WHEN** a tap triggers a transition that completes within 400 ms and the quiet window is 150 ms
- **THEN** the settle wait resolves shortly after the transition ends and reports the elapsed time

#### Scenario: Continuous animation
- **WHEN** a spinner animates indefinitely and the maximum wait is 3 s
- **THEN** the settle wait returns after 3 s with a not-settled result naming frame changes as the reason

### Requirement: Wait predicates from the event stream
The mirror SHALL support waiting for an element matching a selector to appear, disappear or become enabled, resolved from change events without fixed sleeps, with a timeout.

#### Scenario: Element appears
- **WHEN** a client waits for text "Welcome" with a 5 s timeout and the text appears after 800 ms
- **THEN** the wait resolves within one event-processing cycle of the appearance

#### Scenario: Wait times out
- **WHEN** the awaited element never appears
- **THEN** the wait fails at the timeout with the last known snapshot summary

### Requirement: Structural diff
The mirror SHALL produce a diff between the current snapshot and a chosen baseline listing added, removed and changed elements.

#### Scenario: Diff after a form submit
- **WHEN** a baseline was captured before submitting a form and a diff is requested after
- **THEN** the removed form fields and the added confirmation text are listed as such

### Requirement: Quality verdict on every snapshot
Every snapshot SHALL carry a verdict naming the backend that produced it and whether the data is degraded, including truncation by depth or node limits and sparse roots, so that a consumer can distinguish an empty screen from a failed read.

#### Scenario: Degraded read is flagged
- **WHEN** the driver could only return the application root for a screen that is visibly populated
- **THEN** the snapshot is marked degraded with the reason and the consumer is advised to retry or use the frame

### Requirement: Frame access
The mirror SHALL return the latest frame as an image at a requested maximum dimension and quality, and SHALL report the frame timestamp and its age relative to the last tree update.

#### Scenario: Scaled frame
- **WHEN** a client requests the frame at a maximum of 1000 pixels on the long edge
- **THEN** a correctly oriented image no larger than that is returned with its timestamp

### Requirement: Resynchronization
On an event sequence gap or a driver backend fallback the mirror SHALL perform a full re-acquisition and SHALL mark refs issued before the resync as stale.

#### Scenario: Backend fallback
- **WHEN** the driver switches from its fast tree backend to its fallback for the current app
- **THEN** the replica is rebuilt, previously issued refs are stale, and the next snapshot's verdict names the fallback backend
