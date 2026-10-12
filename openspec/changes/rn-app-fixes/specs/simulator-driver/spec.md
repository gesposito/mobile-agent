# Spec Delta

## MODIFIED Requirements

### Requirement: Text entry
The driver SHALL type text through keyboard events and SHALL support paste-based entry for long strings, reporting which method was used. When the field was focused by the same action, the driver SHALL wait, bounded, until the keyboard window is up before the first key, so that keys sent while the keyboard slides in are not dropped. The cadence of typed keys SHALL be one a field of each app technology takes without a loss, decided by measurement: a React Native field, which takes keys through its JavaScript thread, MAY be typed more slowly than a native one.

#### Scenario: React Native field typed whole
- **WHEN** `fill` types the 36-character entry into a React Native field twenty times at the cadence chosen for it
- **THEN** every run reads back whole

#### Scenario: Long string
- **WHEN** a client enters a 500 character string
- **THEN** the text appears in the focused field and the response names the method used

#### Scenario: First keystroke lands
- **WHEN** `fill` focuses a field and types a 20-character string starting with a lower-case letter
- **THEN** the field reads back the whole string, first letter included, in every one of ten runs
