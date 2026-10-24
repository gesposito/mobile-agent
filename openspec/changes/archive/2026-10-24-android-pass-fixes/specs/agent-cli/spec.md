# Spec Delta

## ADDED Requirements

### Requirement: Hits are named without a ref
A click's reply SHALL name the element the tap landed on whether or not the session holds a ref for it, the ref null and the line saying so when none, so that a tap by coordinates or on a plain text says what it landed on.

#### Scenario: A tap on a plain text
- **WHEN** a click by coordinates lands on a text the interactive listing does not give a ref
- **THEN** the reply names the text with `ref` null and the line says `(no ref)`
