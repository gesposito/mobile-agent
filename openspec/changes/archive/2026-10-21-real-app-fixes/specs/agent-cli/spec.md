# Spec Delta

## ADDED Requirements

### Requirement: Typed text is read back
`type` SHALL read the focused field back after entering the text, as `fill` does, SHALL say what it read in its line, and SHALL warn naming what the field holds when it differs, without entering the text again; with no focused field it SHALL say the text was not read back.

#### Scenario: Keys dropped
- **WHEN** `type example.com` reaches the field as `exampl`
- **THEN** the reply says it read back `exampl` and warns that the field differs from the text, advising `fill`

### Requirement: Replies name what the screen holds
A wait's timeout summary SHALL issue the refs it prints to the session; `get value` on a switch SHALL return its state and SHALL say `no value` for an element without one; `wait --text` SHALL name the element it found; `open` SHALL return when a system alert comes in front of the app, naming the alert; the screenshot reply SHALL carry the screen's size in points beside the image's pixels, and a click on coordinates off the screen SHALL name the screen's size in points and say that a screenshot's pixels are not points.

#### Scenario: An alert in front of the opened app
- **WHEN** `open` launches an app whose first screen is covered by a permission alert
- **THEN** `open` returns at once with the app opened and a warning naming the alert, and the next snapshot lists the alert

#### Scenario: Pixels taken for points
- **WHEN** a click names coordinates beyond the screen's points
- **THEN** the failure says the screen's size in points and that the screenshot's pixels are scaled
