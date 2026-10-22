# Spec Delta

## ADDED Requirements

### Requirement: Replies verify the action
A click SHALL name the element the tap landed on as the replica held it at dispatch, and SHALL warn when it is not the target named; a settle requested within a second of the session's last input SHALL wait for that input's first effect before it counts quiet, and SHALL say when none came; a stale ref whose element has a twin in the replica, by identifier and role, else by name, role and bounds, SHALL resolve to the twin and say so, and SHALL fail `stale_ref` without one; `open` SHALL leave the app in front, activating a running one that stayed behind, and SHALL fail `not_in_front` naming what is in front when it cannot; a delta snapshot's changes SHALL print as listing lines in the human output while `--json` keeps the delta object.

#### Scenario: A tap that lands elsewhere
- **WHEN** the layout moves between the snapshot and the tap so that another element lies under the point
- **THEN** the click's reply names the element hit and warns that it is not the target

#### Scenario: A settle right after a press
- **WHEN** `wait --settle` follows a press whose effect comes 300 ms later
- **THEN** it settles after the effect and names what changed last, and with no effect within a second it says so

#### Scenario: A rebuilt field
- **WHEN** the app rebuilds a field's node after the snapshot that listed it
- **THEN** a fill on its old ref acts on the rebuilt field and the reply names the new ref

#### Scenario: Open behind the home screen
- **WHEN** `open` names an app that runs behind the home screen
- **THEN** the app comes to the front, or `open` fails naming what is in front
