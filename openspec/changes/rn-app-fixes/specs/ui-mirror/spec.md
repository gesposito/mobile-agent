# Spec Delta

## MODIFIED Requirements

### Requirement: Snapshot content
A snapshot SHALL list elements with role, accessible name, test identifier when present, value, bounds, enabled, focused and selected state, and a ref. An interactive snapshot SHALL include only actionable elements and SHALL summarize actionable content that is scrolled out of view. On a platform whose tree order is its drawing order, a node drawn after a sibling it overlaps SHALL cut what that sibling holds within its bounds from the interactive listing when it shows something, and an action on a covered element SHALL fail with `element_covered` naming the cover; a tab control SHALL get a ref whether it is selected or not; a checkbox SHALL be listed with its role and checked state as a switch is; a text node and a scroll bar SHALL appear once in the full listing and a heading SHALL be nested once.

#### Scenario: Interactive snapshot of a scrolling list
- **WHEN** an interactive snapshot is requested on a list whose last three buttons are below the fold
- **THEN** visible actionable elements are listed with refs and a hint states that three actionable items are off-screen below

#### Scenario: Heading under a fixed footer on iOS
- **WHEN** a consent screen's heading lies within the bounds of an "Accept and continue" button drawn after it in a fixed footer
- **THEN** the interactive listing leaves the heading out, `visibility` reports it covered, and a click on its ref fails with `element_covered` naming the button rather than accepting the consent

#### Scenario: Card under a sticky header
- **WHEN** a list scrolls a card under a header drawn after the list
- **THEN** the card is listed as covered, not in view

#### Scenario: Selected tab keeps its ref
- **WHEN** a tab bar holds three tabs and the middle one is selected
- **THEN** all three are listed with refs and the selected one carries its selected state

### Requirement: Single coordinate contract
All bounds and coordinates SHALL be expressed in device points with the scale factor reported alongside, and the same space SHALL be used for element bounds and for coordinate-based actions. An action on an element none of whose bounds are in view SHALL read the element's window again once and, if it is still out of view, fail with `element_offscreen`, never tapping bounds the replica held before a layout change.

#### Scenario: Tapping reported bounds hits the element
- **WHEN** a client taps the center of an element's reported bounds
- **THEN** the element receives the tap

#### Scenario: Submit button moved by the keyboard
- **WHEN** the keyboard's appearance moved a submit button and the replica still holds its old bounds, none of them in view
- **THEN** the click reads the window again and taps the button where it is, or fails with `element_offscreen` if it is not in view, and never taps the old bounds

### Requirement: Wait predicates from the event stream
`wait` SHALL evaluate its predicate against the replica after every item the device reports and every re-read, sleeping on the event stream in between, never on a fixed sleep; a wait whose condition already holds in the replica SHALL answer at once without waiting for a read that is due; a timeout SHALL fail with `wait_timeout` and a summary of the screen last seen.

#### Scenario: Element appears
- **WHEN** a wait for text begins before the element exists and the element then appears
- **THEN** the wait resolves within one processing cycle of its appearance

#### Scenario: Wait times out
- **WHEN** the condition never holds
- **THEN** the wait fails at its timeout with the last snapshot summary

#### Scenario: Condition already holds
- **WHEN** a wait is requested for text the replica already lists while a poll re-read is due
- **THEN** the wait answers from the replica at once and the read happens afterwards
