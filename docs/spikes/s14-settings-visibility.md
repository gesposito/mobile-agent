# S14: on-screen elements marked out of view on the Simulator's Settings, Calendar and Maps

Spike S14 (task 1.1 of `real-app-fixes`) reproduces, with the CLI on the iOS Simulator, the fault the demo pass met on 2026-10-20: Settings' root rows absent from every listing while on screen, `get box text=General` answering `inView: false` at y 293 of an 874-point screen, Calendar's event block and Maps' search field "out of view" while visible. It instruments the mirror's visibility computation to name the cause for design D1, against the fixture app's List tab on the same host, which lists; and it measures what press flips a UIKit switch, for design D6. The programs, a throwaway trace in the mirror and two shell and Python drivers, are kept privately with the other spike programs (`docs/spikes/README.md`); every number below comes from their raw outputs. Snapshot dates: 2026-10-20 and 2026-10-21.

**In brief.** Go on D1: the cause is in the mirror, not in what the accessibility bridge reports.

- **A later sibling that spans the window while drawing a bar.** In Settings and Calendar every element of the screen's content is hidden as covered whole by `_UIFloatingBarContainerView`, the view iOS 26 hosts a navigation controller's floating bottom toolbar in: it is listed after the `UINavigationTransitionView` that holds the content, its frame is the whole window, 0,0 402x874 points, and it draws only its toolbar, 0,788 402x86, whose search field or buttons have refs. The later-sibling rule of `rn-app-fixes` (design D31, `cover_by_later_siblings`) takes a sibling that shows something as drawn over everything within its frame, so the rows, the day grid and the event are `Covered` with the cover named `overlay`; the scroll containers, the window, the bars and the 8-point floor all come out right, the row's bounds lying whole inside the visible bounds 0,0 402x788.
- **Maps is the same rule with a list's frame.** The search field lies in a view listed before the sheet's content view, whose list "Locations" has the sheet's whole frame, 8,488.33 386x377.67, and draws its first cell 73 points lower, at y 561.33, under the search bar's band; the rule covers the field with the list, which the refusal and the hints name.
- **The bridge reports what it did.** XCTest's frames are whole frames, as the mirror's documentation says; the runtime is the same iOS 26.5 build (23F77) the README's quick start ran on, and that listing, which showed General as `@e11`, was recorded on 2026-09-27, before the later-sibling rule landed on 2026-10-09. The fixture's List tab lists because its tab bar is a tablist, which the rule spares, and nothing else spans its content.
- **The New Event sheet lists.** Calendar's title field is listed, focused and `inView: true` at 36,208.67 330x22 with the keyboard up; the demo's `fill` warning did not come from the listing.
- **The switch.** A plain tap flips no switch; a 40 ms hold flips every one. The driver's plain tap submits its touch down and up back to back, 0.05 to 0.23 ms apart as the daemon timed the dispatch: it flipped UIKit's Bold Text switch 0 times in 10 and the fixture's SwiftUI toggle 0 in 10 (1 in 10 on its labelled element), while holds of 40, 80 and 150 ms flipped both 10 of 10 each. Settings' labelled switch, the row's content 330 points wide that `#id` and `role=switch` resolve to, flipped 0 of 40 at any hold: its centre is the label, 135 points left of the control.

D1 gets the verdict and D2 the geometry; D6 gets the duration.

## Setup

**Host.** Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 27.0.1 (26A434), Xcode 27.1 (27A9275), CoreSimulator 1174.9.2 with dtuhidd as the input transport, the iOS 26.5 runtime (23F77), the only one installed; the older runtime the quick start ran on was not downloaded. The Simulator, an iPhone 17 (402x874 points at 3x), was booted headless with `simctl boot` and its boot status awaited; Simulator.app never ran. The 1-minute load average was checked before each build and each run and waited for under 6 (60 s polls): the runs started at 5.94, 5.96, 3.55 and 5.46.

**The repository.** The worktree of `real-app-fixes` at `54532228` (the change's proposal on `main`), `cargo build --release -p daemon -p cli` in 1 min 09 s from a clean target directory, the CLI 0.1.0; the daemon on a private socket directory, `mobile-agent-daemon --socket-dir <dir> --drivers simulator --idle-timeout 4h`, the Simulator driver alone, its reader spawned into the Simulator (`simulator-reader` 0.1.0, ready 440 ms after its spawn). The apps: Settings (`com.apple.Preferences`), Calendar (`com.apple.mobilecal`, holding the demo pass's "Demo standup" event on 2026-10-21, 10:00 to 11:00), Maps (`com.apple.Maps`, whose location alert, back at this launch, was declined from the listing), and the SwiftUI fixture (`dev.mobileagent.fixture.swiftui`, `fixtures/swiftui`).

**The programs.**

- `repro.sh <tag> <selector...>`: `snapshot -i`, `snapshot`, both with `--json`, `get box` of each selector in both forms, and a screenshot, into `results/`.
- The trace, a 133-line diff over `crates/mirror`: `visibility::s14_trace(replica, view, w, i)`, called from `Mirror::visibility`, the path `get box` takes for `inView`, when `S14_TRACE` is set. For the element it prints the replica's windows (id, kind, title, bounds, flags, modal), the element's facts and ancestors with their rects, roles, hidden states and scroll semantics, the scroll containers on its path with their frames and what each shows (`Sight::shows`), the window's bars, the windows above, the clip and the visible bounds before the covers (`inside`), the views over part of it and the cover of all of it, `Sight::reach`'s viewport, fraction, cut and reachable rectangle with the floor, which test marked it, and, for a view drawn over it, that view with its parent's children in order and every node in it that shows something; `S14_DUMP` adds every node of the window. The diff was reverted before the switch runs and is not committed.
- `switch.py <settings|fixture>`: for each target, the unnamed switch inside the labelled one (the UISwitch control) and the labelled switch holding it (what `#id` and `role=switch` resolve to), and for each press, a plain `click @ref` and `--hold 40`, `80` and `150`, ten trials: a full `snapshot --json` before, whose listing gives the refs and `checked=` of both switches, the press with its `--json` reply, `wait --settle` (Settings) or `wait 500` (the fixture, whose spinner never settles), a full `snapshot --json` after, and, when the state changed, a revert by `--hold 150` presses on the control, read back until it holds. Settings' Bold Text is reached by a tap at the centre of the covered Accessibility row, 201,371, then `click 'text=Display & Text Size'`; the fixture's Animate by `click '#tab-anim'`. `get value` was not used, since on iOS it returns nothing for a switch (the proposal's gap 6). The daemon was the clean build, the trace reverted.

**Runs.** Run 1 on 2026-10-20 at 10:40 recorded Settings' root and Calendar's day view, then the session was cut; run 2 on 2026-10-21 from 08:48 to 08:52 recorded all four screens through one daemon session, the traced runs followed from 08:54 to 08:58 with the instrumented daemon, and the switch runs from 09:00:42 to 09:05:10 with the clean build. Run 1's listings and bounds equal run 2's.

**Rerun**, with the repository built and a daemon on a private socket directory `$DIR`:

```sh
export MOBILE_AGENT_SOCKET_DIR=$DIR MOBILE_AGENT_DEVICE=<udid>
mobile-agent open com.apple.Preferences && mobile-agent snapshot -i && mobile-agent get box text=General
mobile-agent open com.apple.mobilecal && mobile-agent click 'text=Wednesday, October 21' && mobile-agent get box 'text=Demo standup'
mobile-agent open com.apple.Maps && mobile-agent get box 'text=Apple Maps' && mobile-agent click 'text=Apple Maps'
mobile-agent open dev.mobileagent.fixture.swiftui && mobile-agent click '#tab-list' && mobile-agent get box '#row-5'
```

The trace needs the private diff applied and `S14_TRACE=1` (`S14_DUMP=1` for the whole window) in the daemon's environment; its lines go to the daemon's standard error. The switch runs are `switch.py settings` and `switch.py fixture` in the private copy.

## Measurements

### The listings

The screen is 402x874 points at scale 3 in every case; `-i` is the interactive listing. Bounds are `get box`'s, in points, `x, y width x height`.

| Screen | `-i` lines | Full lines | The element | Its bounds | `inView` | In the listings | Hint lines |
|---|---:|---:|---|---|---|---|---|
| Settings' root | 4 | 8 | button "General" (`com.apple.settings.general`) | 16, 293.33 370x52 | false | in neither; the full listing holds the two toolbars alone, no application root, no list | `-i`: `(1 more actionable item under the button "Dictate")`; full: none |
| the same | | | button "Accessibility" | 16, 345.33 370x52 | false | in neither | |
| Calendar's day view, 2026-10-21 | 15 | 19 | button "Demo standup, from 10:00 to 11:00, Calendar" (`event-shown:Demo standup`) | 69.33, 518 330x50 | false | in neither; the full listing holds the navigation bar with the week strip and the bottom toolbar, no day grid | `(more items to the left, scroll left to reveal)` and `right` for the week strip; none for the day |
| Calendar's New Event sheet | 22 | 46 | textbox "Title" (`title-field`), focused | 36, 208.67 330x22 | true | in both, the list at `visible=59%` under the keyboard | `(2 more actionable items below, 1 of them under the keyboard, ...)`, `(6 more actionable items under the keyboard, ...)` |
| Maps' sheet at launch | 15 | 42 | searchbox "Apple Maps" (`MapsSearchTextField`) | 23.33, 503.67 305.33x42.33 | false | full: `[visible=0%]` under the application root with its two Dictate buttons; `-i`: left out | `-i`: `(1 more actionable item below, scroll down to reveal)`, `(4 more actionable items under the list "Locations")`, `(3 more actionable items under the overlay)` |
| the fixture's List tab | 21 | 76 | button "Row 5" (`row-5`) | 0, 270 402x52 | true | in both | `-i`: 15 rows, Row 15 at `visible=2%`, then `(1 more actionable item below, 1 of them under the footer, scroll down to reveal)` |

- `click text=Apple Maps` was refused at once: `element_covered`, "e87 lies under the list "Locations", which a tap there would land on; scroll it out from under the list "Locations", or close what covers it, then take a new snapshot". Settings' and Calendar's elements name `overlay` the same way (`covered_by` in the trace).
- The full listings of Settings and Calendar begin at a toolbar, with no `generic` application root line: the root and the window hold nothing listable once the content is hidden. Maps' begins at `generic [id=Maps.Application]`.
- Settings' two `Dictate` buttons, identical siblings in the search field, make every snapshot of the root `degraded (sibling_collision)`; Maps' search field has the same pair.

### The computation's inputs

From the trace, for the missing elements and the fixture's row; rects are `left,top right,bottom` in points. "Shows" is what the container shows (`Sight::shows`): its frame clipped by the window and its own containers, less the bars.

| Input | Settings, General | Calendar, the event | Maps, the search field | Fixture, Row 5 |
|---|---|---|---|---|
| The window | application, 0,0 402,874, active and focused, not modal, 168 nodes | the same shape, 133 nodes, plus an empty second `XCUIElementTypeWindow` | the same shape, 156 nodes | the same shape, 244 nodes |
| The top-most modal | none | none | none | none |
| Scroll containers on the path: frame; semantics; shows | collection view 0,0 402,874; vertical, forward, 0 to 56% shown; shows 0,0 402,788 | scroll view 0,0 402,874; vertical, back and forward, 15 to 64%; shows 0,0 402,788; inside a scroll view 0,0 402,874 along both axes, shows the same | none | collection view 0,0 402,874; vertical, forward, 0 to 7%; shows 0,0 402,791 |
| Bars of the window | navigation bar `Settings` 0,62 402,168 (role toolbar); toolbar `Toolbar` 0,788 402,874 | navigation bar `DayViewContainerView` 0,62 402,183.33; toolbar `Toolbar` 0,788 402,874 | none | tab bar 0,791 402,874 (role tablist) |
| System or keyboard windows above | none | none | none | none |
| Clip (window and containers' frames); before (less bars); viewport | 0,0 402,874; 0,0 402,788; 0,0 402,788 | 0,0 402,874; 0,0 402,788; 0,0 402,788 | 0,0 402,874 for all three | 0,0 402,874; 0,0 402,791; 0,0 402,791 |
| Bounds inside the viewport | all of the row, 16,293.33 386,345.33 | all of the block, 69.33,518 399.33,568 | all of the field, 23.33,503.67 328.67,546 | all of the row, 0,270 402,322 |
| Views over part of it | none | none | none | none |
| View covering all of it | `DrawnOver` node 153, named `overlay` | `DrawnOver` node 106, named `overlay` | `DrawnOver` node 75, named `list "Locations"` | none |
| Floor (8 points, or the element's side) | 8 x 8 | 8 x 8 | 8 x 8 | 8 x 8 |
| Reachable rectangle; fraction | none; 0 | none; 0 | none; 0 | the whole row; 1.0 |
| The test that marked it | `hidden = Covered` by `cover_by_later_siblings` (design D31) | the same | the same | reachable |

Every geometric input of the three missing elements is right: the element lies whole inside what its window and its scroll containers show, no bar or keyboard cuts it, and the floor is not reached. The verdict comes from the cover alone.

### The cover

The view drawn over each element, with its parent's children in the order XCTest lists them (UIKit's drawing order), and what in it shows something (a node on screen with a ref or a name, or a disabled control, which is what makes a sibling a cover):

| Screen | The parent | Its children in order | The cover's frame | What shows in the cover |
|---|---|---|---|---|
| Settings | `UILayoutContainerView` 0,0 402,874 | `XCUIElementTypeNavigationBar` `Settings` 0,62 402,168; `UINavigationTransitionView` 0,0 402,874, holding the collection view and its rows; `_UIFloatingBarContainerView` 0,0 402,874 | 0,0 402,874 | its `FloatingBarHostingView` 0,0 402,874 holds `XCUIElementTypeToolbar` `Toolbar` 0,788 402,874, in it the searchbox "Search" 33,803 369,841, the image `magnifyingglass` 46,812.33 66.33,831 and two `Dictate` buttons 335.67,811 353,833 |
| Calendar | `UILayoutContainerView` 0,0 402,874 | `XCUIElementTypeNavigationBar` `DayViewContainerView` 0,62 402,183.33; `UINavigationTransitionView` 0,0 402,874, holding the scroll views and the day grid (`EKDayGridView` 63.33,0 402,1237 with an hour row every 50 points and the event at 518); `_UIFloatingBarContainerView` 0,0 402,874 | 0,0 402,874 | the same host and toolbar 0,788 402,874, in it the buttons Today 33,803 102,841, Calendars 251.67,803 308.33,841 and Inbox 322.33,803 369,841 |
| Maps | `ContaineeViewControllerView` 8,488.33 394,866 | `UIView` 8,488.33 394,561.33, holding `PassthruSearchBar` `MapsSearchBar`, the search field 23.33,503.67 328.67,546 and the `profile` button 336.33,503.67 378.67,546; `UIView` 8,488.33 394,866 | 8,488.33 394,866 | `HomeView` and the collection view "Locations", both 8,488.33 394,866; the list's first cell 27.33,561.33 374.67,603.67 with the "Places" header button at 27.33,571; `PinnedItemSection` 8,603.67 394,724.67 with three tiles; the "Recents" header at 753.33; `RecentsSection` 8,786 394,851.67 with the "Lisbon, Lisboa" cell; two scroll indicators |
| Fixture | `UILayoutContainerView` 0,0 402,874 | `UINavigationTransitionView` 0,0 402,874, holding the collection view; `_UITabBarContainerWrapperView` 0,791 402,874 | the wrapper is not a cover: its frame is the bar's, and `XCUIElementTypeTabBar` in it, 0,791 402,874, is a tablist the rule spares | the five tab buttons |

- In Settings and Calendar the floating bar container's only content is its toolbar, 86 points at the bottom of the screen, which the visibility clip already takes out of what the containers show (788 in the table above); the rest of its 874-point frame draws nothing. The rule hides the earlier sibling's 141 nodes within that frame: the collection view, every cell and every row's button, as `hide_under` walks them.
- In Maps the list's frame holds the 73-point band under the search bar as a content inset: nothing of the list draws there, its first cell starting at 561.33. The reader reads the list's visible frame (`XC_kAXXCAttributeVisibleFrame`) for the scroll semantics, which leave the insets out, and gives the mirror the frame as the node's bounds, as it gives every node.
- The fixture's tab bar wrapper spans only the bar, and the bar is a tablist, spared by role since `rn-app-fixes` task 1.1 found XCTest listing a navigation bar before the list it is drawn over. Nothing in the fixture's screens spans the content from a later position; the acceptance run of `mcp-server` could not have met this.

### What the bridge reports and what the computation assumes

- **The bridge.** The reader loads XCTAutomationSupport from the runtime root, so the tree is the iOS 26.5 runtime's, build 23F77 under Xcode 27.1 as it was under Xcode 27.0, which the README names for the quick start; Xcode's version changes the host side (CoreSimulator, dtuhidd) and not the runtime's bridge. The frames are whole frames, every one on this screen plausible, and `_UIFloatingBarContainerView` is a UIKit class of the runtime, the host of iOS 26's floating bars. Whether the quick start's tree held it cannot be checked without the older runtime; what can be is that the quick start's listing predates the rule: `README.md`'s first commit, with General at `@e11`, is `d1381b76` of 2026-09-27, and `cover_by_later_siblings` is `087170d8` of 2026-10-09, with the bars and pinned headers of D38 in `56a6fe29` of 2026-10-12.
- **The computation.** `cover_by_later_siblings` judges a later sibling by its frame and by whether anything in it shows something; it assumes a sibling draws over its whole frame, which a view laid out over part of the screen does (the React Native footers, floating buttons and sticky headers the rule was built from) and a host spanning its parent while drawing a bar at one edge does not, nor a list whose frame holds an inset. The archived design named this risk, "a transparent later sibling on iOS hides what it spans", and asked for the screen that hits it to be recorded and the rule gated further; these are the screens.

### The switch

Flips in ten presses, by target and press; the box is `get box`'s and the tap point the reply's `x, y`, the centre of what a tap reaches of the element:

| Target | Box | Tap point | Plain tap | 40 ms | 80 ms | 150 ms |
|---|---|---|---:|---:|---:|---:|
| Settings, Bold Text, the control: the unnamed `switch` inside the labelled one | 305, 146.33 63x28 | 336.5, 160.33 | 0/10 | 10/10 | 10/10 | 10/10 |
| Settings, Bold Text, the labelled `switch` (`ENHANCE_TEXT_LEGIBILITY`), the row's content | 36, 146.33 330x28 | 201, 160.33 | 0/10 | 0/10 | 0/10 | 0/10 |
| Fixture, Animate, the control | 208, 270 63x28 | 239.5, 284 | 0/10 | 10/10 | 10/10 | 10/10 |
| Fixture, Animate, the labelled `switch` (`anim-toggle`) | 208, 270 61x28 | 238.5, 284 | 1/10 | 10/10 | 10/10 | 10/10 |

What the click replied, Settings' control (dispatch and first frame change in ms, p50 with the range):

| Press | Reply | `dispatchMs` | `frameChangeMs` |
|---|---|---:|---:|
| plain tap | `no change in 205 ms`, 10 of 10 | 0.13 | none |
| 40 ms | first frame change, 10 of 10 | 50.1 | 102.0 (96.1 to 112.4) |
| 80 ms | first frame change, 10 of 10 | 85.4 | 142.7 (133.6 to 177.2) |
| 150 ms | first frame change, 8 of 10; `no change`, 2 of 10, the switch flipped all the same | 154.4 | 204.8 (169.5 to 237.5) |

- Settings' labelled switch answered `no change` 40 times in 40: a tap at its centre lands on the label "Bold Text", which UIKit's switch cell does not take as a toggle, at any hold. The unnamed switch inside it is the control; on the fixture the labelled toggle is 61 points wide and its centre 1 point from the control's, so the two behave alike there.
- Every reply on the fixture carried a frame change (the Anim tab's spinner never rests), so the reply tells nothing of the flip there; the states were read from the listings. A hold's `dispatchMs` is the hold plus the two submits: 44 to 50, 84 to 85 and 154 to 155 ms.
- The 150 ms replies that said `no change` while the switch flipped: the first frame change came after the daemon's 200 ms watch, which starts at the down and holds 150 of its 200 ms.
- Bold Text on re-lays out the screen; every revert held (0 failed in 60 flips), and no trial erred or warned.

**The tap's down-to-up time.** `SimulatorDriver::tap` is `press(point, Duration::ZERO)` (`crates/simulator/src/macos/driver.rs`): `gesture::press` makes two steps, down at 0 and up at the hold, and `Input::touch` sleeps until each step's time and submits it, so a plain tap's up follows its down with no wait between two XPC submits. The daemon's `dispatchMs`, the time to submit both, was 0.05 to 0.23 ms over the 40 plain taps here (1.5 ms twice), and `crates/simulator/README.md` measured the whole tap at 98 µs at p50 and 8.5 ms at p95. Buttons take it: every other click in this spike landed with it. A UIKit switch does not, nor SwiftUI's toggle, which is a UISwitch underneath.

## Verdict

**Go on D1: the cause is in the mirror's computation.** The three missing elements are hidden by one rule, `cover_by_later_siblings` (design D31 of `rn-app-fixes`), on correct geometry: a later sibling whose frame spans the content while it draws only a bar at the bottom (`_UIFloatingBarContainerView`, 0,0 402x874 drawing 0,788 402x86) or a list whose frame holds a 73-point inset (Maps' "Locations"). Nothing in what the bridge reports is misread: the windows, the containers' frames and semantics, the bars and the floor all come out as on the fixture. The no-go path, a reading gated on the bridge, does not apply.

| Candidate cause (D1) | Found | Verdict |
|---|---|---|
| A container's frame or content size misread | the collection view 0,0 402x874 showing 0 to 56% of its content, shows 0,0 402,788; Calendar's two scroll views the same; the row whole inside | no |
| A window misplaced | one application window 0,0 402x874, active, not modal, no modal cut | no |
| A cover that is not one | `_UIFloatingBarContainerView` in Settings and Calendar; the sheet's content view with the list "Locations" in Maps | **yes** |
| The floor | 8 x 8 points against a 370x52 row; the reachable rectangle is `None` only because the cover leaves nothing | no |
| The bridge under Xcode 27.1 and iOS 26.5 | whole frames as before, the runtime build unchanged; the quick start's listing predates the rule | no |

**D2, what to change and the geometry.** The fake screen, or the mirror's fixture, holds two shapes, with placeholder names:

1. A window 0,0 402x874 whose layout container lists, in order: a navigation bar 0,62 402x106 (a toolbar role); a transition view 0,0 402x874 holding a collection view 0,0 402x874 with vertical scroll semantics (forward, 0 to 56% shown) and rows of 370x52 points at x 16 from y 293.33 (the first two at 293.33 and 345.33), each a cell holding a button with an identifier; and a plain container 0,0 402x874 (no name, no identifier, enabled) holding a plain host 0,0 402x874 holding a toolbar 0,788 402x86 with a search field 33,803 336x38 and a button in it. Expected: the rows listed in view with refs and clickable at their centres, the toolbar's field listed, no `overlay` in the hints.
2. A sheet's content view 8,488.33 386x377.67 listing, in order: a plain view 8,488.33 386x73 holding a search field 23.33,503.67 305.33x42.33 and a button 336.33,503.67 42.33x42.33; and a plain view 8,488.33 386x377.67 holding a named list of the same frame whose first cell is at 27.33,561.33 347.33x42.33 (a header button at 27.33,571 75.33x23) and whose next cells start at y 603.67. Expected: the search field listed and clickable; a tap on a cell still reaches the cell.

The change the geometry asks for: a later sibling covers what lies within the bounds of what shows in it, not within its frame, when it shows nothing itself (a plain container that holds the showing nodes: the floating bar host); and a scroll container among those contributes the bounds of its content on screen, its cells, not its frame, since on iOS the frame holds the insets under bars (equivalently, the visible frame the reader already reads for the semantics, which the payload could carry). A sibling that shows itself (a named view, a control, a disabled one) keeps its frame, so the React Native footers, floating buttons and sticky headers of the archived design, and the fixture tests built on them, keep their covers; a view that holds nothing keeps design D48's rule. The 8-point floor, `Sight::shows` and the bars need no change. Task 2.1 writes the listing tests red on both shapes before touching `cover_by_later_siblings` and `hide_under`'s area.

**D6.** A plain tap does not flip a UIKit switch, nor the fixture's SwiftUI toggle: 0 of 10 each (1 of 10 on the toggle's labelled element), against 10 of 10 at 40 ms, the shortest hold tried, and at 80 and 150. The press on a switch holds 40 ms, gated on the switch role on the Simulator, as D6 has it; the gate must take in SwiftUI's toggle, which D6 assumed flips on a tap, while Android's switches, not measured here, keep the tap until measured. And the press goes to the control: Settings' labelled switch, the row content `#id` and `role=switch` resolve to, flipped 0 of 40 at any hold with its centre on the label, while the unnamed switch inside it flipped 30 of 30 with a hold; a press on a labelled switch that holds a switch aims at the inner one. The demo's 150 ms is not needed, and the tap did not land off the switch: it landed on the control's centre and was too short.

**For the tasks:** 2.1 takes the two shapes above; 3.3 holds the press on a switch 40 ms (10 of 10 on both at 40, 80 and 150; 0 of 10 with the tap), aims it at the control inside a labelled switch, with the fake's switch class holding one, and gates it on the role on the Simulator, SwiftUI's toggle included; 3.2's `get value` on a switch has `checked` to read, which the listing already carries; 5.1 verifies on the same three screens, where `click text=General`, the event's ref and `click text=Apple Maps` are expected to land.
