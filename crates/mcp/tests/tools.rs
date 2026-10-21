//! The tools (task 5.1 of the `mcp-server` change, design D4 and D5): one
//! tool per daemon action an agent drives, listed in both protocol eras,
//! each input schema the schema of its action's request struct in
//! `crates/wire` with the tool table's changes, and each answer the text the
//! CLI prints for the reply.

mod support;

use std::sync::Arc;

use rmcp::handler::server::common::schema_for_input;
use rmcp::model::JsonObject;
use rmcp::schemars::JsonSchema;
use serde_json::{Value, json};
use support::{APP, Client, ERAS, Raw, TestEnv, ends_with_ref, is_error, ok, ref_of, text};

/// The tools, sorted by name.
const TOOLS: [&str; 17] = [
    "click",
    "close",
    "devices",
    "diff_snapshot",
    "doctor",
    "fill",
    "get",
    "help",
    "open",
    "press",
    "screenshot",
    "scroll",
    "snapshot",
    "status",
    "swipe",
    "type",
    "wait",
];

/// The tools that change nothing on the device (design D4).
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

fn schema<T: JsonSchema + 'static>() -> Arc<JsonObject> {
    schema_for_input::<T>().expect("a request schema is an object")
}

/// Each tool and the schema of its action's request struct.
fn actions() -> Vec<(&'static str, Arc<JsonObject>)> {
    vec![
        ("click", schema::<wire::Click>()),
        ("close", schema::<wire::Close>()),
        ("devices", schema::<wire::Devices>()),
        ("diff_snapshot", schema::<wire::DiffSnapshot>()),
        ("doctor", schema::<wire::Doctor>()),
        ("fill", schema::<wire::Fill>()),
        ("get", schema::<wire::Get>()),
        ("open", schema::<wire::Open>()),
        ("press", schema::<wire::Press>()),
        ("screenshot", schema::<wire::Screenshot>()),
        ("scroll", schema::<wire::Scroll>()),
        ("snapshot", schema::<wire::Snapshot>()),
        ("status", schema::<wire::Status>()),
        ("swipe", schema::<wire::Swipe>()),
        ("type", schema::<wire::TypeText>()),
        ("wait", schema::<wire::Wait>()),
    ]
}

fn properties(schema: &Value) -> serde_json::Map<String, Value> {
    schema["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default()
}

fn required(schema: &Value) -> Vec<String> {
    let mut names: Vec<String> = schema["required"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn types(property: &Value) -> Vec<String> {
    match &property["type"] {
        Value::String(one) => vec![one.clone()],
        Value::Array(many) => many
            .iter()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

#[track_caller]
fn tool<'a>(tools: &'a [Value], name: &str) -> &'a Value {
    tools
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("no tool {name}"))
}

#[test]
fn the_tools_list_in_both_eras_as_flat_objects_with_selector_session_and_device() {
    for era in ERAS {
        let env = TestEnv::start("android");
        let mut client = Client::spawn(&env, era);
        let tools = client.list_tools();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, TOOLS, "{era}: the tools, sorted by name");
        for tool in &tools {
            let name = tool["name"].as_str().unwrap_or_default();
            let description = tool["description"].as_str().unwrap_or_default();
            assert!(
                !description.is_empty() && !description.contains('\n'),
                "{era}: {name} has a one-line description: {description:?}"
            );
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object", "{era}: {name}: {schema:#}");
            for combinator in ["anyOf", "oneOf", "allOf"] {
                assert!(
                    schema.get(combinator).is_none(),
                    "{era}: {name}: {combinator} at the root"
                );
            }
            let properties = properties(schema);
            for (parameter, property) in &properties {
                assert!(
                    !types(property).is_empty(),
                    "{era}: {name}.{parameter} has no type: {property:#}"
                );
                assert!(
                    property["description"]
                        .as_str()
                        .is_some_and(|text| !text.is_empty()),
                    "{era}: {name}.{parameter} has no description: {property:#}"
                );
            }
            for envelope in ["session", "device"] {
                assert!(
                    properties
                        .get(envelope)
                        .is_some_and(|p| types(p).contains(&"string".to_owned())),
                    "{era}: {name} takes {envelope} as a string: {schema:#}"
                );
            }
            assert_eq!(
                properties.contains_key("force"),
                name == "open",
                "{era}: force on open alone, {name}: {schema:#}"
            );
            assert!(
                !properties.contains_key("target"),
                "{era}: {name} names the element selector: {schema:#}"
            );
            for name in required(schema) {
                assert!(properties.contains_key(&name), "{era}: required {name}");
            }
            let annotations = &tool["annotations"];
            let read_only = READ_ONLY.contains(&name);
            assert_eq!(
                annotations["readOnlyHint"] == true,
                read_only,
                "{era}: {name}: {annotations}"
            );
            if !read_only {
                assert_eq!(
                    annotations["destructiveHint"],
                    name == "close",
                    "{era}: {name} is destructive if it is close: {annotations}"
                );
            }
        }
        for name in ["click", "fill", "get", "scroll", "type", "wait"] {
            assert!(
                properties(&tool(&tools, name)["inputSchema"]).contains_key("selector"),
                "{era}: {name} takes selector"
            );
        }
        for name in ["fill", "get"] {
            assert!(
                required(&tool(&tools, name)["inputSchema"]).contains(&"selector".to_owned()),
                "{era}: {name} requires selector"
            );
        }
        let snapshot = properties(&tool(&tools, "snapshot")["inputSchema"]);
        for flag in ["interactive", "delta"] {
            assert_eq!(snapshot[flag]["default"], true, "{era}: snapshot {flag}");
        }
        client.close();
    }
}

/// The tool table's changes to an action's parameters, the test's own
/// copy: `target` named `selector`, `snapshot`'s `interactive` and `delta`
/// on by default, `screenshot`'s `inline` set by the server, which answers
/// with the image, and its `path` described for the tool.
fn expected(tool: &str, parameter: &str, property: &Value) -> Option<(String, Value)> {
    if (tool, parameter) == ("screenshot", "inline") {
        return None;
    }
    let mut property = property.clone();
    if let Some(description) = property["description"].as_str() {
        property["description"] = description.replace("`target`", "`selector`").into();
    }
    if tool == "snapshot" && ["interactive", "delta"].contains(&parameter) {
        property["default"] = true.into();
    }
    let name = if parameter == "target" {
        "selector"
    } else {
        parameter
    };
    Some((name.to_owned(), property))
}

#[test]
fn every_parameter_of_each_action_reaches_its_tool() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    let tools = client.list_tools();
    for (name, action) in actions() {
        let action = Value::Object(action.as_ref().clone());
        let listed = &tool(&tools, name)["inputSchema"];
        let mut tool_properties = properties(listed);
        for (parameter, property) in properties(&action) {
            let Some((renamed, mut property)) = expected(name, &parameter, &property) else {
                assert!(
                    !tool_properties.contains_key(&parameter),
                    "{name}: {parameter} is the server's"
                );
                continue;
            };
            let mut published = tool_properties
                .remove(&renamed)
                .unwrap_or_else(|| panic!("{name} lacks {parameter} as {renamed}: {listed:#}"));
            if (name, parameter.as_str()) == ("screenshot", "path") {
                property["description"] = Value::Null;
                published["description"] = Value::Null;
            }
            assert_eq!(published, property, "{name}.{renamed}");
        }
        let mut own: Vec<String> = tool_properties.keys().cloned().collect();
        own.sort();
        let mut server = vec!["device", "session"];
        if name == "open" {
            server.push("force");
        }
        if name == "screenshot" {
            server.push("ifChanged");
        }
        server.sort_unstable();
        assert_eq!(own, server, "{name}: the server's own parameters");
        let renamed: Vec<String> = required(&action)
            .into_iter()
            .map(|name| {
                if name == "target" {
                    "selector".to_owned()
                } else {
                    name
                }
            })
            .collect();
        assert_eq!(required(listed), renamed, "{name}: the required parameters");
    }
    client.close();
}

#[test]
fn a_snapshot_answers_with_the_listing_and_no_refs_map_as_a_delta_by_default() {
    for era in ERAS {
        let env = TestEnv::start("android");
        let mut client = Client::spawn(&env, era);
        let listing = ok(&client.call("snapshot", json!({})));
        let lines: Vec<&str> = listing.lines().collect();
        assert_eq!(lines.len(), 3, "{era}: the launcher's listing: {listing}");
        assert!(
            lines[1].starts_with("- button \"Fixture\" ["),
            "{era}: {listing}"
        );
        assert!(
            lines.iter().all(|line| ends_with_ref(line)),
            "{era}: {listing}"
        );
        assert!(
            !listing.contains("\"refs\"") && !listing.contains('{'),
            "{era}: no refs map: {listing}"
        );
        // The listing as the daemon gives the same session.
        let session = client.session();
        let reply = env.request(json!({
            "action": "snapshot", "session": session, "interactive": true,
        }));
        assert_eq!(reply["data"]["snapshot"], listing.as_str(), "{era}");
        // The next delta of a still screen says so in a line.
        let unchanged = ok(&client.call("snapshot", json!({})));
        assert!(
            unchanged.starts_with("unchanged (revision ") && !unchanged.contains('\n'),
            "{era}: {unchanged}"
        );
        let full = ok(&client.call("snapshot", json!({"delta": false})));
        assert_eq!(full, listing, "{era}: without delta, the whole listing");
        client.close();
    }
}

#[test]
fn a_click_answers_with_the_cli_line_and_keeps_its_warning() {
    for era in ERAS {
        let env = TestEnv::start("android");
        let mut client = Client::spawn(&env, era);
        let opened = ok(&client.call("open", json!({"app": APP})));
        assert_eq!(opened, format!("✓ Opened {APP} on emulator-5554"), "{era}");
        let clicked = ok(&client.call("click", json!({"selector": "#tab-list"})));
        assert!(
            clicked.starts_with("✓ Done (dispatch ") && !clicked.contains('\n'),
            "{era}: {clicked}"
        );
        ok(&client.call("scroll", json!({"direction": "down", "amount": 98})));
        let result = client.call("click", json!({"selector": "#row-20"}));
        let answer = ok(&result);
        let lines: Vec<&str> = answer.lines().collect();
        assert_eq!(lines.len(), 2, "{era}: the line and the warning: {answer}");
        assert!(lines[0].starts_with("✓ Done ("), "{era}: {answer}");
        assert!(
            lines[1].starts_with("⚠ ")
                && lines[1].contains("#row-20 was not mostly in view")
                && lines[1].contains("scroll --until #row-20"),
            "{era}: {answer}"
        );
        client.close();
    }
}

#[test]
fn fill_and_get_answer_with_the_lines_the_cli_prints() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    ok(&client.call("open", json!({"app": APP})));
    let filled = ok(&client.call(
        "fill",
        json!({"selector": "#login-username", "text": "agent"}),
    ));
    assert_eq!(filled, "✓ Done (read back \"agent\", entered in one piece)");
    let masked = ok(&client.call(
        "fill",
        json!({"selector": "#login-password", "text": "hunter2"}),
    ));
    assert_eq!(
        masked,
        "✓ Done (read back 7 masked characters, entered in one piece)"
    );
    let got = ok(&client.call(
        "get",
        json!({"what": "text", "selector": "#login-username"}),
    ));
    assert_eq!(got, "agent");
    // Design D5 of real-app-fixes: an element without a value says so.
    let none = ok(&client.call("get", json!({"what": "value", "selector": "#login-title"})));
    assert_eq!(none, "no value");
    let session = client.session();
    let boxed = ok(&client.call("get", json!({"what": "box", "selector": "#login-submit"})));
    let (printed, _) = env.cli(&["--session", &session, "get", "box", "#login-submit"]);
    assert_eq!(format!("{boxed}\n"), printed, "the CLI's lines");
    assert_eq!(boxed.lines().count(), 4, "{boxed}");
    client.close();
}

/// Task 6.5 of the `mcp-server` change: scroll answers with the CLI's
/// line, its strokes and, with `until`, how much of the element is in
/// view, so a model need not take a snapshot to see where the row ended.
#[test]
fn scroll_answers_with_its_strokes_and_where_until_s_element_ended() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    ok(&client.call("open", json!({"app": APP})));
    ok(&client.call("click", json!({"selector": "#tab-list"})));
    let moved = ok(&client.call("scroll", json!({"direction": "down", "amount": 98})));
    assert_eq!(moved, "✓ Done (1 stroke)");
    let brought = ok(&client.call("scroll", json!({"direction": "down", "until": "#row-57"})));
    assert!(
        brought.starts_with("✓ Done (")
            && brought.contains(" strokes; \"Row 57\" is ")
            && brought.ends_with("% in view)"),
        "{brought}"
    );
    // Already in view: no stroke, in the line the CLI prints.
    let again = ok(&client.call("scroll", json!({"direction": "down", "until": "#row-57"})));
    assert!(
        again.starts_with("✓ Done (0 strokes; \"Row 57\" is "),
        "{again}"
    );
    let session = client.session();
    let (printed, _) = env.cli(&[
        "--session",
        &session,
        "scroll",
        "down",
        "--until",
        "#row-57",
    ]);
    assert_eq!(format!("{again}\n"), printed, "the CLI's line");
    client.close();
}

#[test]
fn devices_and_status_answer_with_the_lines_the_cli_prints() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    let devices = ok(&client.call("devices", json!({})));
    assert!(devices.starts_with("emulator-5554 "), "{devices}");
    let (printed, _) = env.cli(&["devices"]);
    assert_eq!(format!("{devices}\n"), printed, "the CLI's lines");
    let status = ok(&client.call("status", json!({})));
    assert!(status.starts_with("daemon: running, pid "), "{status}");
    client.close();
}

#[test]
fn a_failure_is_an_execution_error_naming_its_code_and_what_to_do() {
    for era in ERAS {
        let env = TestEnv::start("android");
        let mut client = Client::spawn(&env, era);
        let unknown = client.call("click", json!({"selector": "@e999"}));
        assert!(is_error(&unknown), "{era}: {unknown}");
        let message = text(&unknown);
        assert!(
            message.starts_with("✗ unknown_ref: ") && message.contains("snapshot"),
            "{era}: {message}"
        );
        // A ref from before the screen changed is stale.
        let listing = ok(&client.call("snapshot", json!({})));
        let fixture = ref_of(&listing, "launcher-fixture");
        ok(&client.call("click", json!({"selector": fixture})));
        let stale = client.call("click", json!({"selector": fixture}));
        assert!(is_error(&stale), "{era}: {stale}");
        let message = text(&stale);
        assert!(
            message.starts_with("✗ stale_ref: ") && message.contains("Take a new snapshot"),
            "{era}: {message}"
        );
        // A malformed value goes to the daemon, which refuses it with its
        // own code and message.
        let refused = client.call("scroll", json!({"amount": "a lot"}));
        assert!(is_error(&refused), "{era}: {refused}");
        assert_eq!(
            text(&refused),
            "✗ invalid_argument: \"amount\" must be a number, not \"a lot\"",
            "{era}"
        );
        client.close();
    }
}
