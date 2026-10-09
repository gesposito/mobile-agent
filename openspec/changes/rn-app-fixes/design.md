# Design

## Context

See proposal.md. The smoke test ran the maintainer's React Native app (Expo, a development client) on an iPhone 17 Simulator with iOS 26.x through the CLI built from main at 14:53 on 2026-10-09, driven by another agent session from `help agent` alone; its report lists every command with its outcome, the listings' sizes, the verdicts and the timing spans, and reproduces each defect. What the foundation change built that these fixes extend: the mirror's modal cut on iOS (task 10.22, `cover_by_modal_views`), the Android drawing-order cut (task 10.28, `cover_by_views_on_top` from the agent's `on_top` flag), `visibility` and `scroll_viewport` (10.15, 10.16), `fill`'s read-back with its hold after typed keys (10.23, 10.30) and its targeted re-read (10.29), the keyboard window the Simulator's reader bounds by what it draws (10.21), and the listing rules for React Native fields and labelled generic nodes (7.11, 7.12). The relevant spike numbers: S3 measured the Simulator keyboard's slide-in at about 1.2 s before its benchmark typed, and the Simulator's typed entry of 36 characters at 199 ms once the keyboard was up (10.31).

## Goals / Non-Goals

**Goals:**
- An agent cannot act on what it cannot see: a covered element is cut from the listing and refused, and an element out of view is refused, never tapped at stale bounds.
- `fill` tells the truth: verified through what the tree offers, unverified when it offers nothing, a mismatch only for a value that differs.
- Typed text lands whole on iOS from the first key.
- Every control an agent needs is listed once with a ref: tabs, checkboxes.
- Keep the core lean: each rule gated on the platform or shape that needs it (the drawing-order cover on iOS, the keyboard wait on iOS after a focusing tap, the descendant read-back for a field without a value of its own).

**Non-Goals:**
- Changing the device driver's tree (the physical-ios change); the fixes apply to the device through the shared mirror once its tree carries the attributes.
- Fixing the app's own accessibility (its code field names the digits).

## Decisions

### D31. Covers on iOS from the tree's drawing order
On iOS the reader hands the mirror the tree in XCTest's order, which is UIKit's subview order: a later sibling is drawn over an earlier one. The mirror applies the `on_top` rule of task 10.28 to iOS nodes without a flag: a node drawn after a sibling it overlaps, when it shows something (a node on screen with a ref or a name in it or below it), cuts what the sibling holds within its bounds; a node with no area is not judged, and only siblings are compared, as on Android. The shapes from the smoke test: a fixed footer's button over a heading (consent), a floating button over a list's last row (create), a sticky header over a scrolled card (feed). Alternative: an explicit flag from the Simulator's reader (rejected: XCTest's snapshot reports no occlusion and no drawing order beyond the children's order; the order is the information). Risk, as on Android: a transparent container drawn over content hides it; the Bluesky and the smoke test's screens showed none. `Mirror::covered_by` reports the cover, and `click` fails with `element_covered` naming the innermost control of the cover that a tap would hit, or `overlay` for a plain container, with advice to scroll the element out from under it rather than to act on the cover first, which on a consent screen would accept the consent. Two things task 1.1 found: the rule is gated on trees whose windows are rooted at `XCUIElementType` classes from a backend that is not a fallback (the translator flattens the order), and a tab bar or toolbar is never cut by a later sibling, since XCTest lists a navigation bar before the full-screen list its controller draws it over; a tab bar listed after a list does cover the rows wholly under it, as on Android. Overlapping siblings are found by a sweep over top edges, about one comparison per sibling. The refreshed microbenchmark baseline (a quiet run at abf42ed against the one at b724af0) puts the cost at 0.4 µs on a 51-node view (`visibility/row-list` 7.1 to 7.5 µs, `scroll_viewport/list` 7.1 to 7.5, `wait/holds-list` 7.3 to 7.7, 5 to 6% each), 1.4 µs on the Expo list's full listing (21.6 to 23.0), 5 µs on the SwiftUI 500-cell screen's (321.8 to 327.1, 1.6%) and 24 µs on the React Native one's (475.4 to 499.5, 5.1%), with one allocation and 1.6 to 4 KB more per listing of the SwiftUI and Bluesky screens; the interactive listings of the 500-cell screens stayed within 1%. Accepted as the price of not tapping through a footer; the small views pay it in the view's build, where the sweep runs for every window.

### D32. No tap at bounds that are not in view
`click` and the other ref actions compute the element's visibility first (task 10.15). Today `scroll --until` brings an element in with a stroke; `click` makes no stroke, and an element less than mostly in view is tapped with a warning. The smoke test's case was neither: the keyboard's appearance moved the submit button up, the replica still held its old bounds (no event, the frame-driven re-read not yet due), visibility saw 0% in view, and the tap went to the old center on the number pad. The rule: an element with 0% in view and no scroll container to bring it in gets one read of its window, cache cleared, then the action either taps the fresh bounds or fails with `element_offscreen`; the warning path stays for elements partly in view. The re-read is the targeted kind of 10.29, bounded by the read's own cost.

### D33. Read-back through descendants, unverified rather than wrong
`fill` reads the target back after the entry (10.23). A composite field, as a React Native code input built of six boxes, has no value of its own: its digits live in its descendants' texts or values. The read-back therefore takes the field's own value when it has one, else the concatenation of its descendants' values or texts in order, and compares that; when neither exists, the reply succeeds with `verified: false` and a reason (`no_value_to_read`), never `text_mismatch`, which stays for a value that differs after the retry. `type` is unchanged. The reply's `method` keeps `keys` and `direct`; the guide says `direct` is a paste on iOS and the set on Android, and that a paste shows iOS's banner and replaces the pasteboard.

### D34. Keyboard-ready typing on iOS
When `fill` taps the field to focus it and the keyboard window was not up before the tap, the driver waits until the mirror shows the keyboard window (the input-method window the reader presents, 10.21) or a bound passes (1 s, above S3's 1.2 s slide-in measured from the tap's first frame is not needed: the window appears in the tree before the animation ends; the task measures the bound) before the first key. A field already focused with the keyboard up pays nothing. Android is untouched: the agent's text set does not type. The cost is measured against the text-entry series on the Simulator (`reports/10.26-remeasure.md`'s 10.31 numbers) and recorded.

### D35. Listing corrections
A node whose role is `tab` (a Pressable with `accessibilityRole="tab"`, which XCTest reports as a generic with the `selected` trait) gets a ref as a labelled control, selected or not; a checkbox lists as `checkbox` with `[checked=…]` as a switch does; a text node whose only child is its own text is listed once; a scroll bar once; a heading not nested in itself. Each from a fixture of the smoke test's shape, counts only.

### D36. A wait that already holds answers at once
`wait` pumps the mirror before it evaluates, and a pump performs a read that is due; a wait whose condition holds in the replica therefore paid for the read (43 to 473 ms in the smoke test, 380 ms at the median). The predicate is evaluated against the replica first; only when it does not hold does the wait take the due read and go on as before. The replica is current within the poll interval by the mirror's own rules, so the answer is as fresh as a snapshot's.

## Delivery process

The archived design's Delivery process binds this change unchanged. Fixtures are synthetic shapes with placeholder names, never the app's content; the acceptance run is the other session's, on its Simulator and app, reported as counts and outcomes.

## Risks / Trade-offs

- [A transparent later sibling on iOS hides what it spans] → the rule needs the cover to show something, as on Android; a screen that hits it is recorded and the rule gated further.
- [The re-read before a refused tap costs a read on every click of a partly visible element] → only 0% in view triggers it; partly visible elements keep the warning path.
- [The keyboard wait adds up to its bound on fields whose keyboard never comes (a field with its own custom input)] → the bound is short and the wait skipped when the keyboard was up before the tap; measured in task 1.4.

## Migration Plan

Behavioural fixes behind the same commands and codes; `fill` replies gain `verified: false` with a reason where they failed before. Nothing to deploy.

## Open Questions

- `type` is not gated on the keyboard, since a second tap on a non-empty field selects a word on iOS; the Simulator driver taps the field before typing, and skipping that tap when its latest read shows the field focused would spare `fill`'s second tap too (task 1.4's finding).

- Whether the device's tree (physical-ios D21) carries the attributes D31 and D35 need; decided when task 4.4 of that change lands.
