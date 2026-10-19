//! `--structured` (task 5.6 of the `mcp-server` change, design D5): with
//! the flag, each tool whose answer is a daemon reply carries an
//! `outputSchema`, the schema of its reply-data struct in `crates/wire`,
//! and each successful result a `structuredContent`, the reply's data with
//! a snapshot's refs map and a screenshot's image left to the content that
//! carries them; without the flag, neither, since Codex shows a model only
//! the structured content when a result has it, dropping images and text.

mod support;

use serde_json::{Value, json};
use support::{APP, Client, ERAS, Raw, TestEnv, is_error, ok, text};

/// The tools the server answers itself, from no daemon reply: no schema.
const UNTYPED: [&str; 2] = ["doctor", "help"];

#[track_caller]
fn tool<'a>(tools: &'a [Value], name: &str) -> &'a Value {
    tools
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("no tool {name}"))
}

/// The keys of an object.
fn keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

/// Checks `content` against `schema`, the structure schemars gives a
/// reply-data struct: an object whose keys are the schema's properties,
/// with every required one present.
#[track_caller]
fn conforms(content: &Value, schema: &Value) {
    assert_eq!(schema["type"], "object", "{schema:#}");
    let properties = keys(&schema["properties"]);
    for key in keys(content) {
        assert!(
            properties.contains(&key),
            "{key} is no property of the schema: {content:#}"
        );
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        let name = required.as_str().unwrap_or_default();
        assert!(
            content.get(name).is_some(),
            "{name} is required by the schema: {content:#}"
        );
    }
}

#[test]
fn with_structured_each_typed_tool_carries_its_reply_s_schema_and_each_result_the_reply_s_data() {
    for era in ERAS {
        let env = TestEnv::start("android");
        let mut client = Client::spawn_with(&env, era, &["--structured"]);
        let tools = client.list_tools();
        for tool in &tools {
            let name = tool["name"].as_str().unwrap_or_default();
            let schema = &tool["outputSchema"];
            if UNTYPED.contains(&name) {
                assert!(schema.is_null(), "{era}: {name} has no reply struct");
                continue;
            }
            assert_eq!(schema["type"], "object", "{era}: {name}: {schema:#}");
            assert!(
                !keys(&schema["properties"]).is_empty(),
                "{era}: {name}: {schema:#}"
            );
        }
        let snapshot = &tool(&tools, "snapshot")["outputSchema"];
        for property in [
            "snapshot",
            "refs",
            "origin",
            "replicaRevision",
            "target",
            "verdict",
        ] {
            assert!(
                snapshot["properties"].get(property).is_some(),
                "{era}: the snapshot reply's {property}: {snapshot:#}"
            );
        }
        // A snapshot's data, without the refs map the listing carries.
        let result = client.call("snapshot", json!({}));
        let listing = ok(&result);
        assert!(listing.contains("id=launcher-fixture"), "{era}: {listing}");
        let content = &result["structuredContent"];
        conforms(content, snapshot);
        let parsed: wire::reply::Snapshot = serde_json::from_value(content.clone())
            .unwrap_or_else(|e| panic!("{era}: the reply struct reads it ({e}): {content:#}"));
        assert_eq!(parsed.target, "emulator-5554", "{era}");
        assert!(parsed.refs.is_none(), "{era}: no refs map: {content:#}");
        assert!(
            content["snapshot"].get("refs").is_none(),
            "{era}: no refs map in the delta: {content:#}"
        );
        assert!(
            content["snapshot"]["tree"].as_str() == Some(listing.as_str()),
            "{era}: the delta's whole listing: {content:#}"
        );
        // A click's data, as its reply struct reads it.
        let result = client.call("click", json!({"selector": "#launcher-fixture"}));
        ok(&result);
        let content = &result["structuredContent"];
        conforms(content, &tool(&tools, "click")["outputSchema"]);
        let clicked: wire::reply::Click = serde_json::from_value(content.clone())
            .unwrap_or_else(|e| panic!("{era}: {e}: {content:#}"));
        assert!(clicked.accepted, "{era}");
        client.close();
    }
}

#[test]
fn with_structured_a_screenshot_s_image_a_failure_and_the_untyped_tools_keep_their_content() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn_with(&env, &["--structured"]);
    ok(&client.call("open", json!({"app": APP})));
    let shot = client.call("screenshot", json!({}));
    assert!(
        shot["content"]
            .as_array()
            .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "image")),
        "the image content: {shot}"
    );
    let content = &shot["structuredContent"];
    let parsed: wire::reply::Screenshot =
        serde_json::from_value(content.clone()).unwrap_or_else(|e| panic!("{e}: {content:#}"));
    assert_eq!(parsed.format, "jpeg");
    assert!(
        parsed.image.is_none() && content.get("image").is_none(),
        "the image is the content's: {content:#}"
    );
    // A failure is its text alone, so a client that shows structured
    // content alone still sees the code.
    let failed = client.call("click", json!({"selector": "@e999"}));
    assert!(is_error(&failed), "{failed}");
    assert!(text(&failed).starts_with("✗ unknown_ref: "), "{failed}");
    assert!(
        failed.get("structuredContent").is_none(),
        "a failure carries no structured content: {failed}"
    );
    // The tools the server answers itself carry their text as data.
    let help = client.call("help", json!({}));
    assert_eq!(help["structuredContent"]["help"], Value::String(ok(&help)));
    let doctor = client.call("doctor", json!({}));
    assert!(
        doctor["structuredContent"].is_object(),
        "the report's JSON: {doctor}"
    );
    client.close();
}

#[test]
fn without_structured_no_tool_carries_a_schema_and_no_result_structured_content() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    for tool in client.list_tools() {
        assert!(
            tool.get("outputSchema").is_none(),
            "{}: no outputSchema without the flag: {tool}",
            tool["name"]
        );
    }
    ok(&client.call("open", json!({"app": APP})));
    for (name, arguments) in [
        ("snapshot", json!({})),
        ("screenshot", json!({})),
        ("help", json!({})),
        ("click", json!({"selector": "#tab-list"})),
    ] {
        let result = client.call(name, arguments);
        assert!(!is_error(&result), "{name}: {result}");
        assert!(
            result.get("structuredContent").is_none(),
            "{name}: no structuredContent without the flag: {result}"
        );
    }
    let shot = client.call("screenshot", json!({}));
    assert!(
        shot["content"]
            .as_array()
            .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "image")),
        "the image arrives without the flag: {shot}"
    );
    client.close();
}
