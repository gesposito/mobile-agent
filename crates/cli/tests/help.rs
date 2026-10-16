//! Agent-facing help (spec `agent-cli`: Agent-facing help): help with
//! examples for every command, defaults in help output, and a compact guide.

mod support;

use support::TestEnv;

const COMMANDS: [&str; 21] = [
    "devices",
    "open",
    "close",
    "cancel",
    "snapshot",
    "click",
    "tap",
    "fill",
    "type",
    "press",
    "key",
    "swipe",
    "scroll",
    "screenshot",
    "wait",
    "get",
    "diff",
    "batch",
    "status",
    "set",
    "shutdown",
];

#[test]
fn the_agent_guide_fits_in_120_lines_and_covers_the_loop_refs_and_settle() {
    let env = TestEnv::new("android");
    let run = env.cli(&["help", "agent"]).ok();
    let lines = run.stdout.lines().count();
    assert!(lines < 120, "the guide has {lines} lines");
    assert!(lines > 20, "the guide has only {lines} lines");
    for needle in [
        "snapshot -i",
        "@e",
        "ref=e",
        "snapshot again",
        "stale_ref",
        "wait --settle",
        "--json",
        "--session",
        "batch",
    ] {
        assert!(
            run.stdout.contains(needle),
            "the guide lacks {needle:?}:\n{}",
            run.stdout
        );
    }
    assert!(!env.socket().exists(), "help starts no daemon");
}

/// Rn-app-fixes (design D31 to D33): the guide says what an action on an
/// element out of sight fails with and what `fill` reads back, `direct`
/// text with its side effect on the pasteboard included.
#[test]
fn the_agent_guide_says_what_a_click_refuses_and_what_fill_reads_back() {
    let env = TestEnv::new("android");
    let guide = env.cli(&["help", "agent"]).ok().stdout;
    for needle in [
        "element_covered",
        "element_offscreen",
        "out from under",
        "readFrom: descendants",
        "verified: false",
        "reason: no_value_to_read",
        "method: keys",
        "pasteboard",
    ] {
        assert!(
            guide.contains(needle),
            "the guide lacks {needle:?}:\n{guide}"
        );
    }
}

/// Rn-app-fixes (design D37 to D40): the guide says where a tap goes on an
/// element partly covered and what refuses it, what the keyboard hides
/// from a snapshot, that every tab has a ref, what `fill` reads back from a
/// name, and that an identifier on a field's wrapper fills the field.
#[test]
fn the_agent_guide_says_where_a_tap_goes_and_what_the_keyboard_hides() {
    let env = TestEnv::new("android");
    let guide = env.cli(&["help", "agent"]).ok().stdout;
    for needle in [
        "the part of an element you can see",
        "8 points",
        "disabled",
        "under the keyboard",
        "visible=",
        "Every tab has a ref",
        "readFrom: name",
        "wrapper",
    ] {
        assert!(
            guide.contains(needle),
            "the guide lacks {needle:?}:\n{guide}"
        );
    }
}

/// Rn-app-fixes (design D43 to D45): the guide says that `readFrom: name`
/// compares the part of a name the text changed, that an identifier on a
/// view holding no field types where the keyboard a tap on it brings up
/// goes, and that an interactive snapshot leaves out what a tap reaches
/// none of, whatever covers it, and marks what it reaches part of.
#[test]
fn the_agent_guide_says_what_fill_reads_from_a_name_and_what_a_snapshot_leaves_out() {
    let env = TestEnv::new("android");
    let guide = env.cli(&["help", "agent"]).ok().stdout;
    for needle in [
        "part of its name the text changed",
        "keyboard it brings up",
        "reaches none of",
        "reaches part of",
        "a header or a footer",
    ] {
        assert!(
            guide.contains(needle),
            "the guide lacks {needle:?}:\n{guide}"
        );
    }
}

/// Rn-app-fixes (design D50 and D51): the guide says that a hint names what
/// covers what it counts, that the full listing keeps what a tap reaches
/// none of at `visible=0%`, that `get` reads an element out of view and says
/// whether it is in view, and that a wait for `attached` asks the tree.
#[test]
fn the_agent_guide_says_what_hints_count_what_snapshot_keeps_and_what_get_reads_out_of_view() {
    let env = TestEnv::new("android");
    let guide = env.cli(&["help", "agent"]).ok().stdout;
    for needle in [
        "2 of them behind the header",
        "visible=0%",
        "inView",
        "attached",
    ] {
        assert!(
            guide.contains(needle),
            "the guide lacks {needle:?}:\n{guide}"
        );
    }
}

/// Rn-app-fixes (design D52 and D53): the guide says that a hardware
/// keyboard shows no keys and covers nothing, and that a selector on an
/// element that is covered or out of view is refused at once, as its ref
/// is, where it waited out its timeout.
#[test]
fn the_agent_guide_says_a_hardware_keyboard_covers_nothing_and_a_selector_is_refused_as_its_ref() {
    let env = TestEnv::new("android");
    let guide = env.cli(&["help", "agent"]).ok().stdout;
    for needle in [
        "hardware keyboard shows no keys",
        "in view or not",
        "refused at once, as its ref would be",
    ] {
        assert!(
            guide.contains(needle),
            "the guide lacks {needle:?}:\n{guide}"
        );
    }
}

/// Rn-app-fixes (designs D55 to D57): the guide says that iOS's edit menu
/// covers only the strip it draws on and that fill closes the one a lost
/// paste leaves up, that a code field's value counts only when the text
/// changed it, and that a refusal names what covers an element as a hint
/// does.
#[test]
fn the_agent_guide_says_what_the_edit_menu_covers_and_when_a_code_fields_value_counts() {
    let env = TestEnv::new("android");
    let guide = env.cli(&["help", "agent"]).ok().stdout;
    for needle in [
        "edit menu only its strip",
        "closes the edit menu",
        "only when the text changed it",
        "named as hints name it",
    ] {
        assert!(
            guide.contains(needle),
            "the guide lacks {needle:?}:\n{guide}"
        );
    }
}

/// Physical-ios, task 5.3: the guide says where a physical iPhone differs:
/// the readiness states a command on one can fail with and where the step
/// is named, the slower tier that makes settle and waits take longer, and
/// how text goes in, typed or set through the runner rather than pasted.
#[test]
fn the_agent_guide_says_where_a_physical_iphone_differs() {
    let env = TestEnv::new("android");
    let guide = env.cli(&["help", "agent"]).ok().stdout;
    for needle in [
        "MOBILE_AGENT_DRIVERS",
        "device_locked",
        "developer_mode_off",
        "automation_not_enabled",
        "doctor",
        "0.5 s",
        "19 a second",
        "through the runner",
        "not a paste",
    ] {
        assert!(
            guide.contains(needle),
            "the guide lacks {needle:?}:\n{guide}"
        );
    }
    assert!(
        guide.contains("pasted on a Simulator"),
        "the guide says the paste is a Simulator's:\n{guide}"
    );
}

/// The `mcp-server` change (designs D7 to D9): the guide says when to wait
/// for a change, what `--frames` adds to it, that `close --keep` leaves the
/// app running for the next session, and that `cancel` ends a command left
/// in flight.
#[test]
fn the_agent_guide_says_when_to_wait_for_a_change_and_what_close_keep_leaves() {
    let env = TestEnv::new("android");
    let guide = env.cli(&["help", "agent"]).ok().stdout;
    for needle in [
        "wait --change",
        "may still be arriving",
        "--frames",
        "close --keep",
        "app running",
        "cancel",
    ] {
        assert!(
            guide.contains(needle),
            "the guide lacks {needle:?}:\n{guide}"
        );
    }
}

#[test]
fn every_command_has_help_with_examples() {
    let env = TestEnv::new("android");
    for command in COMMANDS {
        for args in [vec!["help", command], vec![command, "--help"]] {
            let run = env.cli(&args).ok();
            assert!(
                run.stdout.contains("Examples:"),
                "{args:?} has no examples:\n{}",
                run.stdout
            );
            assert!(
                run.stdout.contains(&format!("mobile-agent {command}")),
                "{args:?} shows no example of {command}:\n{}",
                run.stdout
            );
        }
    }
    let nested = env.cli(&["help", "get", "text"]).ok();
    assert!(
        nested.stdout.contains("mobile-agent get text"),
        "{}",
        nested.stdout
    );
}

#[test]
fn waiting_commands_document_their_default_timeouts() {
    let env = TestEnv::new("android");
    let wait = env.cli(&["wait", "--help"]).ok().stdout;
    for needle in [
        "10000",
        "set timeout",
        "MOBILE_AGENT_DEFAULT_TIMEOUT",
        "3000",
    ] {
        assert!(
            wait.contains(needle),
            "wait --help lacks {needle:?}:\n{wait}"
        );
    }
    for command in ["click", "fill", "type", "get text", "scroll"] {
        let mut args: Vec<&str> = command.split(' ').collect();
        args.push("--help");
        let help = env.cli(&args).ok().stdout;
        assert!(
            help.contains("10000"),
            "{command} --help lacks its default:\n{help}"
        );
    }
    let open = env.cli(&["open", "--help"]).ok().stdout;
    assert!(open.contains("3000"), "{open}");
}

/// `wait --help` shows `--frames` beside `--change` (design D8 of the
/// `mcp-server` change): what it adds and that it goes with `--change`.
#[test]
fn wait_help_shows_frames_for_a_change_wait() {
    let env = TestEnv::new("android");
    let wait = env.cli(&["wait", "--help"]).ok().stdout;
    let line = wait
        .lines()
        .find(|line| line.trim_start().starts_with("--frames"))
        .unwrap_or_else(|| panic!("wait --help lacks --frames:\n{wait}"));
    assert!(line.contains("--change"), "{line}");
    assert!(
        wait.contains("mobile-agent wait --change --frames"),
        "wait --help shows no example of --frames:\n{wait}"
    );
}

#[test]
fn the_top_level_help_points_to_the_agent_guide() {
    let env = TestEnv::new("android");
    let help = env.cli(&["--help"]).ok().stdout;
    assert!(help.contains("help agent"), "{help}");
    let bare = env.cli(&["help"]).ok().stdout;
    assert!(bare.contains("Commands:"), "{bare}");
}

#[test]
fn help_for_an_unknown_topic_fails_with_a_code() {
    let env = TestEnv::new("android");
    let reply = env.cli(&["--json", "help", "frobnicate"]).failed().json();
    assert_eq!(reply["code"], "unknown_command", "{reply}");
}
