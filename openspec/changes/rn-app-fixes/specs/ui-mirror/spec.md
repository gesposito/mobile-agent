# Spec Delta

## MODIFIED Requirements

### Requirement: Snapshot content
A snapshot SHALL list elements with role, accessible name, test identifier when present, value, bounds, enabled, focused and selected state, and a ref. An interactive snapshot SHALL include only actionable elements and SHALL summarize actionable content that is scrolled out of view. On a platform whose tree order is its drawing order, a node drawn after a sibling it overlaps SHALL cut what that sibling holds within its bounds from the interactive listing when it shows something or is a control, enabled or not, or when it holds and names nothing and lies over part of a scroll container the sibling holds, as a view the platform lists for the background it draws; a bar or a pinned header SHALL cover what lies within its bounds whatever its place in the order; the keyboard window SHALL cover what lies under it, an element wholly under it left out of the interactive listing with the keyboard named, an element partly under it listed with its visible fraction; an action on a covered element SHALL fail with `element_covered` naming the cover; a tab control SHALL get a ref whether it is selected or not, the tabs told by their row when only the selected one carries the trait; a checkbox SHALL be listed with its role and checked state as a switch is; a text node and a scroll bar SHALL appear once in the full listing and a heading SHALL be nested once; a node SHALL keep its ref across a change of its name when its role, identifier, parent and place among its siblings are unchanged.

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

#### Scenario: Control under a disabled button
- **WHEN** a control lies within the bounds of a disabled button drawn after it
- **THEN** the control is cut from the interactive listing, and an action on it fails with `element_covered` naming the button

#### Scenario: Control behind a header listed first
- **WHEN** a navigation bar or a pinned header is listed before the content it is drawn over and a control lies within its bounds
- **THEN** the control is cut from the interactive listing and an action on it fails with `element_covered` naming the bar

#### Scenario: Elements under the keyboard
- **WHEN** the keyboard window covers a submit button wholly and a field partly
- **THEN** the button is left out of the interactive listing with the keyboard named in the hint, and the field is listed with its visible fraction

#### Scenario: Ref across a label change
- **WHEN** a field's name changes once it holds text and nothing else about it changes
- **THEN** its ref is the same before and after, and a `fill` by that ref reads it back

### Requirement: Single coordinate contract
All bounds and coordinates SHALL be expressed in device points with the scale factor reported alongside, and the same space SHALL be used for element bounds and for coordinate-based actions. An action on an element none of whose bounds are in view SHALL read the element's window again once and, if it is still out of view, fail with `element_offscreen`, never tapping bounds the replica held before a layout change. An action on an element SHALL tap the centre of the largest part of it that a tap can reach, its bounds within the viewport minus every cover, and SHALL fail with `element_covered` naming what a tap at the element's centre would hit when no reachable part is at least 8 points on each side; actions by ref, by identifier and by text selector SHALL take the same path, and `scroll --until` SHALL judge arrival by the same reachable part. The reachable part SHALL be clipped by the visible bounds of every scroll container above the element before the covers are taken away; the interactive listing SHALL mark every partly reachable element with its visible fraction and leave out every element with no reachable part, counted in the hint, whatever covers it; and a `scroll` that fails because the content stopped moving SHALL name its stroke's start and end. A scroll container SHALL be told by the platform's element type and scroll traits, never by a class name alone, so that every scrolling view subclass gives its semantics and visible bounds. The full listing SHALL mark partly reachable elements as the interactive listing does, and the interactive listing's hint lines SHALL count what it leaves out for a cover beside what is scrolled away, naming the cover.

#### Scenario: Strip clipped by the scroll view
- **WHEN** a button's lower 10 points lie below a floating button but outside its scroll view's visible bounds
- **THEN** the button has no reachable part, is left out of the interactive listing and a click on it fails with `element_covered` naming the floating button

#### Scenario: Partly covered elements marked
- **WHEN** a heading shows 50% below a footer and a card 30% below a sticky header
- **THEN** the interactive listing marks each with its visible fraction, and a control behind a header is left out and counted in the hint

#### Scenario: Card partly under a sticky header
- **WHEN** a card lies 70% under a sticky header and 30% of it shows below the header
- **THEN** a click on the card taps the centre of the part that shows, with a warning naming the fraction in view, and never the header

#### Scenario: Heading half under a footer button
- **WHEN** a heading's lower half lies under a fixed footer's button and its upper half shows
- **THEN** a click taps the upper half when it is at least 8 points tall, and otherwise fails with `element_covered` naming the button

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

#### Scenario: React Native scroll view
- **WHEN** a React Native screen scrolls a list inside a scroll view that runs on under a footer to the bottom of the screen, the footer's view listed holding nothing and its tabs after it, and a floating button lies over the list's last row
- **THEN** the mirror knows the scroll view's bounds and semantics by its element type, what the scroll view shows ends at the footer, the last row's strip under the footer leaves it no reachable part and a click on it fails with `element_covered` naming the floating button, and `scroll --until` strokes inside what the scroll view shows

#### Scenario: Hints agree with the listing
- **WHEN** the interactive listing leaves out two controls behind a header and six rows scrolled above
- **THEN** its hint lines count the six rows above and the two behind the header, and the full listing shows the two controls with their visible fraction
