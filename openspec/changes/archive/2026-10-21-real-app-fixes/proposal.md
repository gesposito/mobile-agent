# Proposal

## Why

The first pass of the MCP server on real apps, run on 2026-10-20 on the iOS Simulator's Settings, Calendar, Maps, Safari and Photos and on the Android emulator's Settings, Clock, Dialer and Chrome, completed its nine tasks but cost 116 turns and 9 failed calls on iOS against 58 turns and none on Android. The cost had causes in the tool, not in the server's protocol work. The largest: rows plainly on screen are marked out of view and left out of the listings. Settings' root rows (General, Accessibility) never appear, `get box text=General` reports `inView: false` at y 293 of an 874-point screen, Calendar's event block and Maps' search field are "out of view" while visible, and `fill` on Calendar's title said the field "left the screen" while the sheet showed it. The models fell back to screenshots and coordinates, which cost most of Settings' 47 turns. The acceptance on the fixture app had shown none of this; the README's quick start listed Settings' General row under Xcode 27.0, and this pass ran under macOS 27.0.1, Xcode 27.1 and the iOS 26.5 runtime. Next: the server raises every tool's default timeout to 53 s, so a `click` on a missing element waits 53 s before `element_not_found`, a design miss of the `mcp-server` change; `type` has no read-back, so dropped keys ("exampl") went unreported and Safari ran a search; refs printed in a wait's timeout summary are not usable; a Settings switch flips only with a 150 ms hold; `get value` on a switch returns nothing; `open` blocks 53 s behind a permission alert; the screenshot result does not state the screen's point size, so a model passes pixel coordinates; `wait --text` answers `✓ Done` alone. The pass's notes, transcripts and videos are kept privately with the maintainer.

## What Changes

- A spike on the Simulator's Settings, Calendar and Maps that finds why on-screen elements are marked out of view under this host's toolchain, and measures what press flips a UIKit switch; then the mirror's fix for the cause, held by a fake screen with the same geometry.
- The daemon and CLI: `type` reads the focused field back as `fill` does and says so; a wait's timeout summary issues the refs it prints; `get value` on a switch gives its state; `wait --text` names what it found; `open` returns when a system alert comes in front, naming it; the screenshot reply and the off-screen coordinate error name the screen's size in points; a press that flips a switch, if the spike finds the tap too short, gated on the Simulator's switches.
- The MCP server: the call bound caps a command's timeout and never raises it; the screenshot result's text states the image's size beside the screen's points; the instructions card says to `wait` for a change after a click that reports none.
- A verification pass through the server on the same Settings, Calendar and Safari tasks, counted against the first pass.

## Capabilities

### Modified Capabilities

- `ui-mirror`: in-view marking follows the screen for elements inside scroll containers.
- `agent-cli`: `type` reads back; replies that name what the screen holds (a wait's summary refs, switch values, the found element, an alert in front of `open`, the screen's point size).
- `mcp-server`: the bound caps and does not raise timeouts; the screenshot's text names the sizes.

## Impact

- `crates/mirror` (the visibility computation the spike points at), `crates/daemon` (`handlers/act.rs` for `type`'s read-back and the switch press, `handlers/view.rs` for the summary's refs, `get value` and `wait --text`'s line, `handlers/mod.rs` for `open` at an alert, `screenshot`'s reply), `crates/wire` (the reply structs that gain fields), `crates/cli` (`output.rs` lines), `crates/mcp` (`tools.rs` timeouts, `results.rs` screenshot text, `instructions.rs`), `docs/spikes/s14-settings-visibility.md`, `reports/12.2-real-apps.md`, the READMEs and the agent guide where the lines change.
- No change to the drivers unless the spike puts the switch press there.
