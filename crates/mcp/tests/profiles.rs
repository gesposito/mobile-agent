//! Profiles and instructions (task 5.5 of the `mcp-server` change, design
//! D10): `--tools read-only` serves the tools that change nothing on the
//! device alone and refuses the others naming the profile; the
//! instructions card, which `initialize` and `server/discover` carry, has a
//! first paragraph that stands alone under 512 characters and a whole under
//! 4,096, and says when the profile is read-only; and the `help` tool
//! answers with the CLI's agent guide, preceded by a table that maps the
//! guide's commands to the tools and their arguments.

mod support;

use serde_json::json;
use support::{Client, ERAS, Raw, TestEnv, ok};

/// The tools that change nothing on the device, which the read-only
/// profile serves alone, sorted by name.
const READ_ONLY: [&str; 9] = [
    "devices",
    "diff_snapshot",
    "doctor",
    "get",
    "help",
    "screenshot",
    "snapshot",
    "status",
    "wait",
];

#[test]
fn the_read_only_profile_lists_the_read_only_tools_alone_and_refuses_the_rest() {
    for era in ERAS {
        let env = TestEnv::start("android");
        let mut client = Client::spawn_with(&env, era, &["--tools", "read-only"]);
        let names: Vec<String> = client
            .list_tools()
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(names, READ_ONLY, "{era}");
        let refused = client
            .try_call("click", json!({"selector": "#launcher-fixture"}))
            .expect_err("a tool outside the profile is refused");
        assert!(
            refused.starts_with("-32602: ")
                && refused.contains("click")
                && refused.contains("read-only"),
            "{era}: invalid params naming the profile: {refused}"
        );
        // The read-only tools answer, and the refused click tapped nothing.
        let listing = ok(&client.call("snapshot", json!({})));
        assert!(listing.contains("id=launcher-fixture"), "{era}: {listing}");
        client.close();
    }
}

#[test]
fn the_instructions_card_s_first_paragraph_stands_alone_and_the_whole_fits_the_clients() {
    for era in ERAS {
        let env = TestEnv::start("android");
        let client = Client::spawn(&env, era);
        let card = client.instructions();
        let first = card.split("\n\n").next().unwrap_or_default();
        let first_len = first.chars().count();
        assert!(
            (200..512).contains(&first_len),
            "{era}: the first paragraph is {first_len} characters:\n{first}"
        );
        assert!(
            first.ends_with('.') && !first.contains('\n'),
            "{era}: the first paragraph ends at a sentence:\n{first}"
        );
        for needle in ["snapshot", "@e", "code", "help"] {
            assert!(
                first.contains(needle),
                "{era}: the first paragraph lacks {needle:?}:\n{first}"
            );
        }
        let whole = card.chars().count();
        assert!(
            whole < 4096 && whole > first_len,
            "{era}: the card is {whole} characters"
        );
        for needle in [
            "selector",
            "stale_ref",
            "change",
            "settle",
            "text",
            "session",
            "close",
            "keep",
            "screenshot",
            "ifChanged",
        ] {
            assert!(
                card.contains(needle),
                "{era}: the card lacks {needle:?}:\n{card}"
            );
        }
        assert!(
            !card.contains("read-only"),
            "{era}: every tool served:\n{card}"
        );
        client.close();
        // With the read-only profile, the card says so.
        let client = Client::spawn_with(&env, era, &["--tools", "read-only"]);
        let card = client.instructions();
        assert!(
            card.contains("read-only") && card.chars().count() < 4096,
            "{era}: the profile in force:\n{card}"
        );
        client.close();
    }
}

#[test]
fn help_answers_with_the_agent_guide_after_a_table_mapping_its_commands_to_the_tools() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    let help = ok(&client.call("help", json!({})));
    let (guide, _) = env.cli(&["help", "agent"]);
    let guide = guide.trim_end();
    let table = help
        .strip_suffix(guide)
        .unwrap_or_else(|| panic!("the guide, as the CLI prints it, ends the text:\n{help}"));
    for (command, needles) in [
        ("snapshot -i", &["snapshot", "interactive"][..]),
        ("snapshot -d 2", &["maxDepth"]),
        ("snapshot --delta", &["delta"]),
        ("click @e4", &["click", "selector"]),
        ("tap 200 480", &["x", "y"]),
        ("--hold 800", &["hold"]),
        ("fill @e2 agent", &["fill", "selector", "text"]),
        ("type @e2", &["type", "text"]),
        ("press Enter", &["press", "key"]),
        ("scroll down 600", &["scroll", "direction", "amount"]),
        ("-s @e7", &["selector"]),
        ("--until", &["until"]),
        ("swipe up", &["swipe", "direction"]),
        ("open com.example.app", &["open", "app"]),
        ("close --keep", &["close", "keep"]),
        ("wait --change", &["wait", "change"]),
        ("--frames", &["frames"]),
        ("wait --settle", &["settle"]),
        ("wait --text Welcome", &["text"]),
        ("--state hidden", &["selector", "state"]),
        ("wait 500", &["ms"]),
        ("get text @e5", &["get", "what", "selector"]),
        ("diff snapshot", &["diff_snapshot"]),
        ("screenshot shot.jpg", &["screenshot", "path"]),
        ("devices", &["devices"]),
        ("--device ID", &["device"]),
        ("--session NAME", &["session"]),
        ("--force", &["force"]),
        ("set timeout", &["timeout"]),
        ("cancel", &["cancel"]),
        ("help agent", &["help"]),
    ] {
        let row = table
            .lines()
            .find(|line| line.contains(command))
            .unwrap_or_else(|| panic!("no row for {command:?} in:\n{table}"));
        for needle in needles {
            assert!(
                row.contains(needle),
                "the row for {command:?} lacks {needle:?}: {row}"
            );
        }
    }
    let rows = table.lines().count();
    assert!(rows < 60, "a short table: {rows} lines:\n{table}");
    client.close();
}
