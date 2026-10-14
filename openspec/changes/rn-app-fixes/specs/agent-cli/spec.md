# Spec Delta

## ADDED Requirements

### Requirement: Text entry verification
`fill` SHALL read the field back after the entry and report whether the text was verified; when the field holds its value in descendants rather than in its own value, as a composite code field does, it SHALL verify through them, and when it holds it in its own accessible name, through the part of the name that changed with the entry, separators left out, never the whole name; when no value can be read back it SHALL report the entry as unverified and succeed rather than fail with `text_mismatch`, which is reserved for a value read back that differs from the text entered; when `fill` or `type` resolves an identifier to a node that takes no text but holds exactly one editable descendant, that descendant SHALL be the target, named in the reply; when the node holds none and a tap on it raises the keyboard within a bound, the text SHALL go to the focused input, the reply naming the node, and otherwise the action SHALL fail with `invalid_argument`; the reading the read-back compares with SHALL be taken after the field is emptied, when `fill` empties a field that held text before typing; the reply's `method` SHALL say `keys` for typed text and `direct` for text entered in one piece, which on iOS is a paste through the pasteboard, documented with its side effect on the pasteboard.

#### Scenario: Code field that names its digits
- **WHEN** `fill` enters a six-digit code into a field whose accessible name spells the digits it holds and shows no value
- **THEN** the reply reports the entry verified through the name, and a wrong code read back fails with `text_mismatch`

#### Scenario: Identifier on the field's wrapper
- **WHEN** `fill` is given an identifier that resolves to a generic node holding one editable field
- **THEN** the text goes into that field and the reply names it

#### Scenario: Name with a fixed suffix
- **WHEN** `fill` enters a six-digit code into a field whose name reads "–, –, –, –, –, –, Verification code, required" before and "1, 2, 3, 4, 5, 6, Verification code, required" after
- **THEN** the reply reports the entry verified through the name, comparing the digits alone

#### Scenario: Node without a field that raises the keyboard
- **WHEN** `fill` is given an identifier that resolves to a generic node with no children and a tap on it raises the keyboard
- **THEN** the text goes to the focused input, the reply names the node and says the keyboard carried the text, and the read-back runs through the node's name

#### Scenario: Composite code field
- **WHEN** `fill` enters a six-digit code into a field whose six boxes hold one digit each
- **THEN** the digits land, the reply reports the entry verified through the boxes, and no `text_mismatch` is raised

#### Scenario: Value that cannot be read back
- **WHEN** `fill` enters text into a field the tree reports without a value
- **THEN** the reply succeeds with `verified: false` and says why

#### Scenario: Mismatch
- **WHEN** the field reads back a value that differs from the text entered after the retry
- **THEN** the reply fails with `text_mismatch` naming both values

#### Scenario: Refill of the same text
- **WHEN** `fill` types into a field the same six-digit code it already holds, the field's name spelling the digits
- **THEN** the field is emptied first, the reply reports the entry verified through the name, never `no_value_to_read`

### Requirement: Queries reach elements out of view
`get` and a `wait` for an element's presence or absence in the tree (`--state attached` and `detached`) SHALL resolve any element the replica holds, in view or not, within one read, and SHALL say in the reply whether the element is in view; only an element the tree does not hold SHALL fail with `element_not_found`, naming what is listed.

#### Scenario: Box of a row scrolled above
- **WHEN** `get box` names a row the list has scrolled above the screen and the tree still holds
- **THEN** the reply gives its bounds and says it is out of view, within one read

### Requirement: Selectors resolve covered elements
An action by text selector or identifier SHALL resolve any element the replica holds within one read and then take the same path as an action by ref, failing with `element_covered` naming the cover or `element_offscreen` as a ref would, within the time a ref takes; the selector wait SHALL apply only to an element the tree does not hold yet.

#### Scenario: Click by text on a covered element
- **WHEN** a click names by text an element one view covers whole
- **THEN** it fails with `element_covered` naming the view within one read, never `element_not_found` after the selector timeout
