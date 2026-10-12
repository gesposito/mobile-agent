# Spec Delta

## ADDED Requirements

### Requirement: Text entry verification
`fill` SHALL read the field back after the entry and report whether the text was verified; when the field holds its value in descendants rather than in its own value, as a composite code field does, it SHALL verify through them, and when it holds it in its own accessible name, which changed with the entry, through the name; when no value can be read back it SHALL report the entry as unverified and succeed rather than fail with `text_mismatch`, which is reserved for a value read back that differs from the text entered; when `fill` or `type` resolves an identifier to a node that takes no text but holds exactly one editable descendant, that descendant SHALL be the target, named in the reply; the reply's `method` SHALL say `keys` for typed text and `direct` for text entered in one piece, which on iOS is a paste through the pasteboard, documented with its side effect on the pasteboard.

#### Scenario: Code field that names its digits
- **WHEN** `fill` enters a six-digit code into a field whose accessible name spells the digits it holds and shows no value
- **THEN** the reply reports the entry verified through the name, and a wrong code read back fails with `text_mismatch`

#### Scenario: Identifier on the field's wrapper
- **WHEN** `fill` is given an identifier that resolves to a generic node holding one editable field
- **THEN** the text goes into that field and the reply names it

#### Scenario: Composite code field
- **WHEN** `fill` enters a six-digit code into a field whose six boxes hold one digit each
- **THEN** the digits land, the reply reports the entry verified through the boxes, and no `text_mismatch` is raised

#### Scenario: Value that cannot be read back
- **WHEN** `fill` enters text into a field the tree reports without a value
- **THEN** the reply succeeds with `verified: false` and says why

#### Scenario: Mismatch
- **WHEN** the field reads back a value that differs from the text entered after the retry
- **THEN** the reply fails with `text_mismatch` naming both values
