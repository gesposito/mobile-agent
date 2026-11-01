# Spec Delta

## MODIFIED Requirements

### Requirement: Bars, headers and system windows as covers
A bar or a pinned header SHALL cover what lies within its bounds whatever its place in the order, except the controls it holds itself, which stay reachable; every other system window presented over the application, the edit menu among them, SHALL cover what it draws and nothing more.

#### Scenario: Card under a sticky header
- **WHEN** a list scrolls a card under a header drawn after the list
- **THEN** the card is listed as covered, not in view

#### Scenario: Control behind a header listed first
- **WHEN** a navigation bar or a pinned header is listed before the content it is drawn over and a control lies within its bounds
- **THEN** the control is cut from the interactive listing and an action on it fails with `element_covered` naming the bar

#### Scenario: A button the header holds
- **WHEN** a header holds a search field whose clear button lies within the header's bounds
- **THEN** the button is listed in view and a click on it answers with the button hit

## ADDED Requirements

### Requirement: A listing after a system window closes
When a tap answers a dialog of system UI's and the replica then holds system UI's windows alone, without the dialog, the mirror SHALL read the application's windows again before the next listing, until they are back or a bound passes, so that the interactive listing lists what the screen shows rather than what system UI left, and a settle right after SHALL count the change.

#### Scenario: An alert dismissed
- **WHEN** a permission alert is dismissed over an app's first screen
- **THEN** the next interactive listing lists the screen's controls and a settle right after names the change
