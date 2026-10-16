//! The schemas of the request and reply types, built with the `schema`
//! feature (task 3.1 of the `mcp-server` change). A tool's input schema must
//! be a flat object, since Claude's API refuses `anyOf`, `oneOf` and `allOf`
//! at a schema's root (docs/research/agent-surfaces.md), with every property
//! typed and described; an MCP output schema must be an object too.

use schemars::{JsonSchema, Schema, schema_for};
use serde_json::Value;

/// Each action's parameters and those of them a request must give.
struct Expected {
    action: &'static str,
    schema: fn() -> Schema,
    parameters: &'static [&'static str],
    required: &'static [&'static str],
}

fn of<T: JsonSchema>() -> Schema {
    schema_for!(T)
}

const REQUESTS: [Expected; 19] = [
    Expected {
        action: "status",
        schema: of::<wire::Status>,
        parameters: &[],
        required: &[],
    },
    Expected {
        action: "devices",
        schema: of::<wire::Devices>,
        parameters: &[],
        required: &[],
    },
    Expected {
        action: "close",
        schema: of::<wire::Close>,
        parameters: &["keep"],
        required: &[],
    },
    Expected {
        action: "cancel",
        schema: of::<wire::Cancel>,
        parameters: &[],
        required: &[],
    },
    Expected {
        action: "set_timeout",
        schema: of::<wire::SetTimeout>,
        parameters: &["ms"],
        required: &["ms"],
    },
    Expected {
        action: "doctor",
        schema: of::<wire::Doctor>,
        parameters: &[],
        required: &[],
    },
    Expected {
        action: "open",
        schema: of::<wire::Open>,
        parameters: &["app", "relaunch", "env", "timeout"],
        required: &["app"],
    },
    Expected {
        action: "snapshot",
        schema: of::<wire::Snapshot>,
        parameters: &["interactive", "maxDepth", "delta", "full"],
        required: &[],
    },
    Expected {
        action: "diff_snapshot",
        schema: of::<wire::DiffSnapshot>,
        parameters: &["interactive", "maxDepth"],
        required: &[],
    },
    Expected {
        action: "get",
        schema: of::<wire::Get>,
        parameters: &["what", "target", "timeout"],
        required: &["what", "target"],
    },
    Expected {
        action: "wait",
        schema: of::<wire::Wait>,
        parameters: &[
            "ms", "settle", "quiet", "treeOnly", "change", "text", "target", "state", "timeout",
        ],
        required: &[],
    },
    Expected {
        action: "screenshot",
        schema: of::<wire::Screenshot>,
        parameters: &["path", "quality", "maxDimension", "inline"],
        required: &[],
    },
    Expected {
        action: "click",
        schema: of::<wire::Click>,
        parameters: &["target", "x", "y", "hold", "timeout"],
        required: &[],
    },
    Expected {
        action: "fill",
        schema: of::<wire::Fill>,
        parameters: &["target", "text", "timeout"],
        required: &["target", "text"],
    },
    Expected {
        action: "type",
        schema: of::<wire::TypeText>,
        parameters: &["text", "target", "timeout"],
        required: &["text"],
    },
    Expected {
        action: "press",
        schema: of::<wire::Press>,
        parameters: &["key"],
        required: &["key"],
    },
    Expected {
        action: "swipe",
        schema: of::<wire::Swipe>,
        parameters: &["direction", "distance", "from", "to", "duration"],
        required: &[],
    },
    Expected {
        action: "scroll",
        schema: of::<wire::Scroll>,
        parameters: &["direction", "amount", "target", "until", "timeout"],
        required: &[],
    },
    Expected {
        action: "shutdown",
        schema: of::<wire::Shutdown>,
        parameters: &[],
        required: &[],
    },
];

fn sorted(names: impl IntoIterator<Item = impl Into<String>>) -> Vec<String> {
    let mut names: Vec<String> = names.into_iter().map(Into::into).collect();
    names.sort();
    names
}

#[test]
fn every_request_schema_is_a_flat_object_of_typed_and_described_parameters() {
    for expected in &REQUESTS {
        let schema = (expected.schema)();
        let schema = schema.as_value();
        let action = expected.action;
        assert_eq!(schema["type"], "object", "{action}: {schema:#}");
        for combinator in ["anyOf", "oneOf", "allOf"] {
            assert!(
                schema.get(combinator).is_none(),
                "{action}: {combinator} at the root: {schema:#}"
            );
        }
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            sorted(properties.keys().cloned()),
            sorted(expected.parameters.iter().copied()),
            "{action}: the parameters"
        );
        for (name, property) in &properties {
            assert!(
                property.get("type").is_some(),
                "{action}.{name} has no type: {property:#}"
            );
            assert!(
                property
                    .get("description")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty()),
                "{action}.{name} has no description: {property:#}"
            );
        }
        let required: Vec<String> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| name.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            sorted(required),
            sorted(expected.required.iter().copied()),
            "{action}: the required parameters"
        );
    }
    // One schema in full, as a tool's input schema would publish it.
    println!("{:#}", schema_for!(wire::Scroll).as_value());
}

#[test]
fn every_reply_schema_is_an_object() {
    let replies: [(&str, Schema); 19] = [
        ("status", of::<wire::reply::Status>()),
        ("devices", of::<wire::reply::Devices>()),
        ("close", of::<wire::reply::Close>()),
        ("cancel", of::<wire::reply::Cancel>()),
        ("set_timeout", of::<wire::reply::SetTimeout>()),
        ("doctor", of::<wire::reply::Doctor>()),
        ("open", of::<wire::reply::Open>()),
        ("snapshot", of::<wire::reply::Snapshot>()),
        ("diff_snapshot", of::<wire::reply::DiffSnapshot>()),
        ("get", of::<wire::reply::Get>()),
        ("wait", of::<wire::reply::Wait>()),
        ("screenshot", of::<wire::reply::Screenshot>()),
        ("click", of::<wire::reply::Click>()),
        ("fill", of::<wire::reply::Fill>()),
        ("type", of::<wire::reply::TypeText>()),
        ("press", of::<wire::reply::Press>()),
        ("swipe", of::<wire::reply::Swipe>()),
        ("scroll", of::<wire::reply::Scroll>()),
        ("shutdown", of::<wire::reply::Shutdown>()),
    ];
    for (action, schema) in &replies {
        let schema = schema.as_value();
        assert_eq!(schema["type"], "object", "{action}: {schema:#}");
        assert_eq!(
            schema["additionalProperties"], false,
            "{action}: a reply's data holds the fields its type names"
        );
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .map_or(0, serde_json::Map::len);
        assert!(properties > 0, "{action}: no fields: {schema:#}");
    }
}
