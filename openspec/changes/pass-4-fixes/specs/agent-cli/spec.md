# Spec Delta

## MODIFIED Requirements

### Requirement: Typed text is read back
`type` SHALL read the focused field back after entering the text, as `fill` does, reading until the field holds the text or two consecutive reads agree, within a bound, SHALL say what it read in its line, SHALL warn `shorter than typed` when the read-back is a prefix of the text, and SHALL warn naming what the field holds when it differs otherwise, without entering the text again; with no focused field it SHALL say the text was not read back.

#### Scenario: Keys dropped
- **WHEN** `type example.com` reaches the field as `exampl`
- **THEN** the reply says it read back `exampl` and warns that the field differs from the text, advising `fill`

#### Scenario: The last key lands late
- **WHEN** the field holds all but the last character when the first read-back is taken
- **THEN** the read-back is taken again and the reply says it read back the whole text

### Requirement: A settle waits for the input's effect
A settle requested within a second of the session's last input SHALL wait for that input's first effect before it counts quiet, and SHALL say when none came; a settle on a target whose windows send no change events SHALL read the tree its frames changed before it ends; a settle that stops at its timeout SHALL say `settled: false` with how long it waited and why, exiting as a success unless `--strict` makes it fail `settle_timeout`.

#### Scenario: A settle right after a press
- **WHEN** `wait --settle` follows a press whose effect comes 300 ms later
- **THEN** it settles after the effect and names what changed last, and with no effect within a second it says so

#### Scenario: A menu's items after a pause
- **WHEN** `wait --settle` follows a tap whose menu takes its items with a frame after a pause longer than the quiet window, on a target whose windows send no change events
- **THEN** the settle ends once the tree holds the items, and the listing right after names them

#### Scenario: A settle that gives up
- **WHEN** `wait --settle` stops at its timeout because frames kept changing
- **THEN** the reply says `settled: false` with the time waited, the line warns, the exit is a success, and with `--strict` the call fails `settle_timeout`

### Requirement: A stale ref resolves to its twin
A stale ref whose element has a twin in the replica, by identifier and role, else by name, role and bounds, SHALL resolve to the twin and say so, and SHALL fail `stale_ref` without one, naming the system dialog in front when one holds the screen.

#### Scenario: A rebuilt field
- **WHEN** the app rebuilds a field's node after the snapshot that listed it
- **THEN** a fill on its old ref acts on the rebuilt field and the reply names the new ref

#### Scenario: A dialog in front
- **WHEN** a stale ref has no twin while a system dialog is in front
- **THEN** the failure names the dialog and says to dismiss it

### Requirement: Hits are named without a ref
A click's reply SHALL name the element the tap landed on whether or not the session holds a ref for it, the ref null and the line saying so when none: the deepest node under the point with a name, an identifier or a role other than generic, else the nearest labeled ancestor, else the text under the point, `generic` only when nothing under the point is labeled.

#### Scenario: A tap on a plain text
- **WHEN** a click by coordinates lands on a text the interactive listing does not give a ref
- **THEN** the reply names the text with `ref` null and the line says `(no ref)`

#### Scenario: A tap through a generic container
- **WHEN** a click by coordinates lands on a generic container drawn over a labeled button
- **THEN** the reply names the button

## ADDED Requirements

### Requirement: A text wait prefers the whole text
`wait --text` SHALL prefer an element whose text is the sought text, case and surrounding whitespace aside, over one whose text merely contains it, keeping the listing's order among equals, and SHALL still resolve on a containing text when no element holds the text alone.

#### Scenario: A city beside its airport
- **WHEN** a wait for `Lisbon` runs while the screen lists `Lisbon Airport (LIS)` above `Lisbon`
- **THEN** the wait resolves on `Lisbon`
