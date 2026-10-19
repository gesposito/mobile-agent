//! The instructions card and the `help` tool's text (design D10 of the
//! `mcp-server` change). The card goes to the client with `initialize` and
//! `server/discover`: its first paragraph stands alone within the 512
//! characters Codex keeps of it, and the whole stays under the 4,096
//! Claude Code keeps, so the long guide is the `help` tool's: the CLI's
//! agent guide (`mobile-agent help agent`), preceded by a table that maps
//! the guide's commands to the tools and their arguments, so that a model
//! reads one guide for both surfaces.

use std::time::Duration;

use crate::tools::Profile;

/// The card's first paragraph, which stands alone.
const HEAD: &str = "Drives Android devices, emulators, iOS Simulators and iPhones through the mobile-agent daemon. Call snapshot for the screen's elements, each with a ref such as @e3, then act by ref or selector with click, fill, type, press, swipe and scroll: every reply verifies what the action did, and a failure names its code and what to do next. The help tool holds the full guide.";

/// The card's paragraphs after the first.
const BODY: &str = "Selectors and refs: a ref names one element while it stays on screen and fails with stale_ref once it is gone, so snapshot again before acting on older refs; a selector is an id (#login), text=Name or text==Exact name, or role=switch, and waits up to the call's timeout for a match. snapshot lists the interactive elements as a delta by default: a session's first call is the whole listing, later ones what changed, or unchanged (revision N); full: true gives the whole listing again, interactive: false every element with the texts and system bars. A plain text, such as a status line, has no ref and is in neither the interactive listing nor a change wait's delta taken in it: read it with get {what: \"text\", selector: \"#status\"}, wait for it with text, or list it with interactive: false.

Waiting: click waits for the tap's first effect, so a snapshot right after it sees the change. wait with change: true answers once the listing changed from your last snapshot, with the delta (frames: true ends it at a frame change too, for a canvas or a map); settle: true once nothing changed for 200 ms, at most 3 s; text: \"Welcome\" once an element shows the text; selector with state (visible, hidden, enabled, attached, detached) once the element reaches it; ms for a fixed pause.

Sessions: this server acts in a session of its own, which holds the target it drives; name session on a call to resume another session's refs. close stops the app and releases the target; close with keep: true releases the target and leaves the app running, which this server also does for its own session when it exits, so the next session finds the app as it was.

Screenshots: screenshot answers with the JPEG inline, its long edge at most 1000 px; with ifChanged: true it answers unchanged since the last screenshot instead of a second image of a still screen; with path it writes the file and answers the path alone. A screenshot right after a click can show the transition the tap set off, since click answers at its first effect: wait with settle: true before one.";

/// The table before the guide: each command the guide uses, and the tool
/// call it is through this server.
const TABLE: &str = "The guide below is the mobile-agent CLI's; through this server each of its commands is a tool call, with these arguments:
  snapshot -i                      snapshot (interactive and delta are the defaults)
  snapshot                         snapshot {interactive: false}
  snapshot -d 2                    snapshot {maxDepth: 2}
  snapshot --delta                 snapshot (the default); {full: true} for the whole listing again
  click @e4 | tap 200 480          click {selector: \"@e4\"} | click {x: 200, y: 480}
  click @e4 --hold 800             click {selector: \"@e4\", hold: 800}
  fill @e2 agent                   fill {selector: \"@e2\", text: \"agent\"}
  type @e2 ' smith'                type {selector: \"@e2\", text: \" smith\"}; without selector, the focused field
  press Enter                      press {key: \"Enter\"}
  scroll down 600                  scroll {direction: \"down\", amount: 600}; -s @e7 is {selector: \"@e7\"}
  scroll down --until '#row-150'   scroll {direction: \"down\", until: \"#row-150\"}
  swipe up | swipe X1 Y1 X2 Y2     swipe {direction: \"up\"} | swipe {from: [X1, Y1], to: [X2, Y2]}
  open com.example.app             open {app: \"com.example.app\"}; --force is {force: true}
  close | close --keep             close | close {keep: true}
  wait --change [--frames]         wait {change: true}, with {frames: true} for --frames
  wait --settle                    wait {settle: true}
  wait --text Welcome              wait {text: \"Welcome\"}
  wait '#spinner' --state hidden   wait {selector: \"#spinner\", state: \"hidden\"}
  wait 500                         wait {ms: 500}
  --timeout MS                     timeout: MS on any waiting call, at most this server's bound
  get text @e5 | get value | box   get {what: \"text\", selector: \"@e5\"}; what: \"value\" or \"box\"
  diff snapshot                    diff_snapshot
  screenshot shot.jpg              screenshot {path: \"shot.jpg\"}; without path the image comes inline, and {ifChanged: true} skips a still screen
  devices                          devices
  doctor                           doctor
  --device ID | --platform P       device: \"ID\" on any call; name the device instead of a platform
  --session NAME                   session: \"NAME\" on any call; without it, this server's own session
  set timeout 5000                 no tool: pass timeout on each waiting call
  cancel                           no tool: this server cancels a call its client abandons, and one past its bound
  batch, --json                    no tool: calls go one by one, and a reply is the CLI's text
  help agent                       help (this text)";

/// The instructions card for a server that serves `profile`'s tools with
/// calls bounded at `bound`.
pub fn card(profile: Profile, bound: Duration) -> String {
    let tools = match profile {
        Profile::All => format!(
            "Tools: every tool is served; a call ends within {} s, a wait that names no timeout with wait_timeout and the screen's summary, and a call the client abandons is cancelled.",
            bound.as_secs()
        ),
        Profile::ReadOnly => format!(
            "Tools: this server runs with --tools read-only, serving devices, status, doctor, snapshot, diff_snapshot, get, wait, screenshot and help alone: it observes the device and acts on nothing; a call ends within {} s.",
            bound.as_secs()
        ),
    };
    format!("{HEAD}\n\n{BODY}\n\n{tools}")
}

/// The `help` tool's text: the table, then the CLI's agent guide.
pub fn help() -> String {
    format!("{TABLE}\n\n{}", cli::AGENT_GUIDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_s_first_paragraph_stands_alone_and_the_whole_fits_the_clients() {
        for profile in [Profile::All, Profile::ReadOnly] {
            let card = card(profile, Duration::from_secs(55));
            let first = card.split("\n\n").next().unwrap_or_default();
            assert_eq!(first, HEAD);
            let head = first.chars().count();
            assert!(head < 512, "the first paragraph is {head} characters");
            assert!(first.ends_with('.'));
            let whole = card.chars().count();
            assert!(whole < 4096, "the card is {whole} characters");
            assert_eq!(
                card.contains("read-only"),
                profile == Profile::ReadOnly,
                "{profile:?}:\n{card}"
            );
            assert!(card.contains("55 s"), "{profile:?}: the bound:\n{card}");
        }
    }

    #[test]
    fn help_is_the_table_then_the_guide() {
        let help = help();
        assert!(help.ends_with(cli::AGENT_GUIDE));
        assert!(help.starts_with(TABLE));
        let rows = TABLE.lines().count();
        assert!(rows < 60, "{rows} lines");
    }
}
