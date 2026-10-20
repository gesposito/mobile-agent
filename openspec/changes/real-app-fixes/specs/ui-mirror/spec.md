# Spec Delta

## ADDED Requirements

### Requirement: In-view marking follows the screen
An element whose bounds lie within its window's and the screen's bounds and that no view covers SHALL be marked in view and listed by the interactive listing, whatever scroll container holds it and whatever that container's content size, so that the rows of a UIKit collection view, the content of a sheet and the fields of a search bar are listed as a SwiftUI list's rows are.

#### Scenario: A collection view's rows
- **WHEN** a screen holds a scroll container whose content is taller than its frame and whose first rows lie within the frame
- **THEN** those rows are listed with their refs and marked in view, and the rows below the frame are counted as scrolled away
