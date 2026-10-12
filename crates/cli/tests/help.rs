//! Agent-facing help (spec `agent-cli`: Agent-facing help): help with
//! examples for every command, defaults in help output, and a compact guide.

mod support;

use support::TestEnv;

const COMMANDS: [&str; 20] = [
    "devices",
    "open",
    "close",
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
