# Spec Delta

## MODIFIED Requirements

### Requirement: A node drawn over its sibling covers it
On a platform whose tree order is its drawing order, a node drawn after a sibling it overlaps SHALL cut what that sibling holds within its bounds from the interactive listing when it shows something or is a control, enabled or not, or when it holds and names nothing and lies over part of a scroll container the sibling holds, the container taken as far as it lies within that sibling, as a view the platform lists for the background it draws. On Android, where a touch that no view takes goes to the views under it, a node drawn over its siblings that takes no touch, nothing in it clickable, long-clickable or scrollable, SHALL cut nothing.

#### Scenario: Heading under a fixed footer on iOS
- **WHEN** a consent screen's heading lies within the bounds of an "Accept and continue" button drawn after it in a fixed footer
- **THEN** the interactive listing leaves the heading out, `visibility` reports it covered, and a click on its ref fails with `element_covered` naming the button rather than accepting the consent

#### Scenario: Control under a disabled button
- **WHEN** a control lies within the bounds of a disabled button drawn after it
- **THEN** the control is cut from the interactive listing, and an action on it fails with `element_covered` naming the button

#### Scenario: A menu read while it comes in
- **WHEN** a context menu is read while its glass view, listed after the view that holds the menu's list, holds nothing, the list's frame running far past the menu
- **THEN** the menu's items are listed in view and a click on one lands at its centre

#### Scenario: A chip under its input's layout on Android
- **WHEN** a time picker's minute chip lies under the minute input's layout, drawn on top, which holds the input's label and its hidden field and nothing that takes a touch
- **THEN** the chip is listed in view and a click on its ref lands at its centre

### Requirement: Bars, headers and system windows as covers
A bar or a pinned header SHALL cover what lies within its bounds whatever its place in the order, except the controls it holds itself, which stay reachable, a header pinned across a sibling that scrolls, or holds a scroll view of its own frame through views of that frame, being drawn over that sibling, which covers none of it; every other system window presented over the application, the edit menu among them, SHALL cover what it draws and nothing more.

#### Scenario: Card under a sticky header
- **WHEN** a list scrolls a card under a header drawn after the list
- **THEN** the card is listed as covered, not in view

#### Scenario: Control behind a header listed first
- **WHEN** a navigation bar or a pinned header is listed before the content it is drawn over and a control lies within its bounds
- **THEN** the control is cut from the interactive listing and an action on it fails with `element_covered` naming the bar

#### Scenario: A button the header holds
- **WHEN** a card's fixed header holding a close button is listed before the view of the card's content, whose scroll view starts with a header of its own spanning the card's width
- **THEN** the close button is listed in view and a click on it answers with the button hit

## ADDED Requirements

### Requirement: A listing after a system window closes
When a tap answers a dialog of system UI's and the replica then holds system UI's windows alone, without the dialog, the mirror SHALL read the application's windows again before the next listing, until they are back or a bound passes, so that the interactive listing lists what the screen shows rather than what system UI left, and a settle right after SHALL count the change.

#### Scenario: An alert dismissed
- **WHEN** a permission alert is dismissed over an app's first screen
- **THEN** the next interactive listing lists the screen's controls and a settle right after names the change
