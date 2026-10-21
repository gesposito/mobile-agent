//! The tool table (design D4 of the `mcp-server` change): one tool per
//! daemon action an agent drives, under agent-browser's stems where the
//! concepts match, each with a one-line description, its annotations and
//! its input schema, the schema of its action's request struct in
//! `crates/wire` generated at run time with the table's changes:
//!
//! - the element argument, `target` to the daemon, is `selector`, as
//!   agent-browser names it, in the schema's properties, its `required` and
//!   the descriptions that name it;
//! - [`Change::On`]: a flag on unless a call turns it off, `snapshot`'s
//!   `interactive` and `delta` (design D5);
//! - every tool takes the envelope's `session` and `device`, and `open` its
//!   `force` ([`Change::Force`]);
//! - [`Change::Set`]: a parameter the server sets itself and the tool does
//!   not take, `screenshot`'s `inline` (design D6);
//! - [`Change::Own`]: a flag of the tool's own, which the server reads and
//!   does not send, `screenshot`'s `ifChanged`;
//! - [`Change::Describe`]: a parameter the tool describes otherwise than
//!   its action, `screenshot`'s `path`.
//!
//! Every other parameter of a struct is its tool's as the struct's schema
//! gives it, so a parameter a struct gains reaches its tool. A call's
//! arguments go back through the same changes into the request line with
//! their values as the call gave them, so the daemon refuses a malformed
//! one with its own code and message.
//!
//! The server's bound of a call (design D7) reaches the line the same way
//! for every tool, as a cap (design D7 of `real-app-fixes`): a `timeout` or
//! `ms` above the bound less [`MARGIN`] is capped to it, so the daemon's
//! own wait ends first, which the call records for the result's text; a
//! call that names no `timeout` sends none, so the daemon's own default
//! applies: the session's, else 10 s, and 3 s for a settle.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::common::{schema_for_input, schema_for_output};
use rmcp::model::{JsonObject, ToolAnnotations};
use rmcp::schemars::JsonSchema;
use serde_json::{Map, Value, json};

/// The element argument as the daemon's requests name it.
const TARGET: &str = "target";

/// The element argument as the tools name it, agent-browser's name.
const SELECTOR: &str = "selector";

/// How far under the server's bound a call's timeout is capped, so that
/// the daemon's own wait ends first.
pub const MARGIN: Duration = Duration::from_secs(2);

/// The duration arguments the bound caps.
const DURATIONS: [&str; 2] = ["timeout", "ms"];

/// The tools the server serves (design D10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Profile {
    /// The tools that change nothing on the device.
    #[value(name = "read-only")]
    ReadOnly,
    /// Every tool.
    All,
}

impl Profile {
    /// Whether the profile serves `tool`.
    pub fn serves(self, tool: &Tool) -> bool {
        match self {
            Profile::All => true,
            Profile::ReadOnly => tool.effect == Effect::Reads,
        }
    }
}

/// What a tool's annotations say it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// It changes nothing on the device: `readOnlyHint`.
    Reads,
    /// It acts on the device without undoing anything: `destructiveHint`
    /// false, since a client takes a tool that is not read-only as
    /// destructive unless the tool says otherwise.
    Acts,
    /// It stops what runs: `destructiveHint`.
    Stops,
}

/// A change of the table to a tool's parameters, beyond the selector and
/// the envelope every tool has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The flag defaults to true, which a call that gives none sends.
    On(&'static str),
    /// The server sends the flag set, and the tool does not take it.
    Set(&'static str),
    /// The tool's own flag, which the server reads and does not send, with
    /// its description.
    Own(&'static str, &'static str),
    /// The envelope's `force`.
    Force,
    /// The parameter with the tool's description instead of the action's.
    Describe(&'static str, &'static str),
}

/// One tool.
pub struct Tool {
    /// The tool's name: agent-browser's stem where it has one.
    pub name: &'static str,
    /// The daemon action a call sends.
    pub action: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    /// The schema of the action's request struct.
    parameters: fn() -> Arc<JsonObject>,
    /// The schema of the action's reply-data struct, which `--structured`
    /// serves as the tool's `outputSchema` (design D5); none for a tool the
    /// server answers itself.
    reply: Option<fn() -> Arc<JsonObject>>,
    pub changes: &'static [Change],
}

/// `screenshot`'s flag that answers a still screen with a line.
pub const IF_CHANGED: &str = "ifChanged";

/// The tools, sorted by name, the order `tools/list` gives them in.
pub const TOOLS: [Tool; 17] = [
    Tool {
        name: "click",
        action: "click",
        description: "Tap an element (`selector`: a ref such as `@e3` from a snapshot, `#id` or `text=...`) or a point (`x`, `y`), and report what the tap set off: the first frame change and tree delta, or no change.",
        effect: Effect::Acts,
        parameters: schema::<wire::Click>,
        reply: Some(reply_schema::<wire::reply::Click>),
        changes: &[],
    },
    Tool {
        name: "close",
        action: "close",
        description: "Stop the app this session opened and release its target; with `keep`, release the target and leave the app running.",
        effect: Effect::Stops,
        parameters: schema::<wire::Close>,
        reply: Some(reply_schema::<wire::reply::Close>),
        changes: &[],
    },
    Tool {
        name: "devices",
        action: "devices",
        description: "List the connected Android devices, emulators, iOS Simulators and iPhones, with the id `device` takes and the session holding each.",
        effect: Effect::Reads,
        parameters: schema::<wire::Devices>,
        reply: Some(reply_schema::<wire::reply::Devices>),
        changes: &[],
    },
    Tool {
        name: "diff_snapshot",
        action: "diff_snapshot",
        description: "Compare the screen with this session's last snapshot: the elements added, removed and changed.",
        effect: Effect::Reads,
        parameters: schema::<wire::DiffSnapshot>,
        reply: Some(reply_schema::<wire::reply::DiffSnapshot>),
        changes: &[],
    },
    Tool {
        name: "doctor",
        action: "doctor",
        description: "Check this host's toolchains and the daemon, each check with a status and, where something can be done, a hint.",
        effect: Effect::Reads,
        parameters: schema::<wire::Doctor>,
        reply: None,
        changes: &[],
    },
    Tool {
        name: "fill",
        action: "fill",
        description: "Replace a text field's content with `text` and read the field back; fails with `text_mismatch` when the field shows other text.",
        effect: Effect::Acts,
        parameters: schema::<wire::Fill>,
        reply: Some(reply_schema::<wire::reply::Fill>),
        changes: &[],
    },
    Tool {
        name: "get",
        action: "get",
        description: "Read an element's text, value or bounds (`what`: `text`, `value` or `box`), in view or not.",
        effect: Effect::Reads,
        parameters: schema::<wire::Get>,
        reply: Some(reply_schema::<wire::reply::Get>),
        changes: &[],
    },
    Tool {
        name: "help",
        action: "help",
        description: "Read the agent guide: the loop, snapshots, refs and selectors, acting, waiting, reading, errors, targets and sessions, after a table mapping the guide's commands to these tools and their arguments.",
        effect: Effect::Reads,
        parameters: no_parameters,
        reply: None,
        changes: &[],
    },
    Tool {
        name: "open",
        action: "open",
        description: "Launch an app, an Android package or an iOS bundle id, claim its target for this session and wait for its first screen to settle.",
        effect: Effect::Acts,
        parameters: schema::<wire::Open>,
        reply: Some(reply_schema::<wire::reply::Open>),
        changes: &[Change::Force],
    },
    Tool {
        name: "press",
        action: "press",
        description: "Press a key: `Enter`, `Back`, `Home`, a letter or digit, or modifiers joined with `+` as in `Control+a`.",
        effect: Effect::Acts,
        parameters: schema::<wire::Press>,
        reply: Some(reply_schema::<wire::reply::Press>),
        changes: &[],
    },
    Tool {
        name: "screenshot",
        action: "screenshot",
        description: "Capture the screen as a JPEG image, its long edge at most 1000 px by default; with `ifChanged`, a line instead when it is this session's last screenshot; with `path`, the file's path instead of the image.",
        effect: Effect::Reads,
        parameters: schema::<wire::Screenshot>,
        reply: Some(reply_schema::<wire::reply::Screenshot>),
        changes: &[
            Change::Set("inline"),
            Change::Own(
                IF_CHANGED,
                "Answer `unchanged since the last screenshot` instead of the image when the image is this session's last screenshot's on the target.",
            ),
            Change::Describe(
                "path",
                "Save the image to this file, relative to the server's working directory, and answer with the path instead of the image, for a client that takes no images.",
            ),
        ],
    },
    Tool {
        name: "scroll",
        action: "scroll",
        description: "Scroll the content in a direction, inside an element (`selector`) or the screen, or until an element (`until`) is mostly in view.",
        effect: Effect::Acts,
        parameters: schema::<wire::Scroll>,
        reply: Some(reply_schema::<wire::reply::Scroll>),
        changes: &[],
    },
    Tool {
        name: "snapshot",
        action: "snapshot",
        description: "List the screen's elements, one per line with role, name and a ref (`@e3`) to act on; by default the interactive elements as a delta: a session's first is the whole listing, later ones what changed or `unchanged (revision N)`.",
        effect: Effect::Reads,
        parameters: schema::<wire::Snapshot>,
        reply: Some(reply_schema::<wire::reply::Snapshot>),
        changes: &[Change::On("interactive"), Change::On("delta")],
    },
    Tool {
        name: "status",
        action: "status",
        description: "Show the daemon, its targets with their state, app and holding session, and its sessions.",
        effect: Effect::Reads,
        parameters: schema::<wire::Status>,
        reply: Some(reply_schema::<wire::reply::Status>),
        changes: &[],
    },
    Tool {
        name: "swipe",
        action: "swipe",
        description: "Move a finger in a direction from the screen's center, or from one point to another (`from`, `to`).",
        effect: Effect::Acts,
        parameters: schema::<wire::Swipe>,
        reply: Some(reply_schema::<wire::reply::Swipe>),
        changes: &[],
    },
    Tool {
        name: "type",
        action: "type",
        description: "Type `text` as key events into an element (`selector`), or into the focused field, keeping what the field holds.",
        effect: Effect::Acts,
        parameters: schema::<wire::TypeText>,
        reply: Some(reply_schema::<wire::reply::TypeText>),
        changes: &[],
    },
    Tool {
        name: "wait",
        action: "wait",
        description: "Wait some milliseconds (`ms`), for the screen to settle (`settle`) or to change from the last snapshot (`change`), or for an element (`selector`) or a text (`text`) to reach a `state`; pass `timeout` for a wait past the default of 10 s.",
        effect: Effect::Reads,
        parameters: schema::<wire::Wait>,
        reply: Some(reply_schema::<wire::reply::Wait>),
        changes: &[],
    },
];

/// The schema of `T`, a request struct, as rmcp generates a tool's input:
/// JSON Schema 2020-12 with the root's title and description left out.
fn schema<T: JsonSchema + 'static>() -> Arc<JsonObject> {
    schema_for_input::<T>()
        .unwrap_or_else(|error| panic!("a request struct's schema is an object: {error}"))
}

/// The schema of `T`, a reply-data struct, as rmcp generates a tool's
/// output: the struct's with the root's title and description left out.
fn reply_schema<T: JsonSchema + 'static>() -> Arc<JsonObject> {
    schema_for_output::<T>()
}

/// The schema of a tool that sends no request, `help`: an object without
/// properties of its own.
fn no_parameters() -> Arc<JsonObject> {
    let mut schema = JsonObject::new();
    schema.insert("type".to_owned(), "object".into());
    schema.insert("properties".to_owned(), Value::Object(Map::new()));
    Arc::new(schema)
}

/// The envelope's `session`, which every tool takes.
fn session_schema() -> Value {
    json!({
        "default": null,
        "description": "The session to act in, whose refs, target claims and snapshot baselines a call uses; the server's own session without it. Name a session to resume a transcript whose refs another session issued.",
        "type": ["string", "null"],
    })
}

/// The envelope's `device`, which every tool takes.
fn device_schema() -> Value {
    json!({
        "default": null,
        "description": "The target: an id as `devices` lists it (an adb serial or a Simulator UDID), or a target's name; without it, the only target connected, else the one this session holds.",
        "type": ["string", "null"],
    })
}

/// The envelope's `force`, which `open` takes.
fn force_schema() -> Value {
    json!({
        "default": false,
        "description": "Take the target over from the session that holds it.",
        "type": "boolean",
    })
}

/// A call as the server sends it.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// The session the request goes out in; none when the call named one
    /// that is no string, which the daemon refuses.
    pub session: Option<String>,
    /// The `device` the call named.
    pub device: Option<String>,
    /// The request line.
    pub line: Map<String, Value>,
    /// The tool's own flags the call set.
    pub flags: Vec<&'static str>,
    /// What the bound capped, one sentence each, for the result's text.
    pub capped: Vec<String>,
}

impl Call {
    /// Whether the call set the tool's own flag `name`.
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(&name)
    }

    /// What the call waits for, as its arguments say it, for a progress
    /// message: the element `until` brings into view, the element, the
    /// text, the app, a change, the screen settling or the milliseconds.
    pub fn waits_for(&self) -> String {
        let string = |key: &str| self.line.get(key).and_then(Value::as_str);
        let flag = |key: &str| self.line.get(key) == Some(&Value::Bool(true));
        if let Some(until) = string("until") {
            return format!("{until:?} to come into view");
        }
        if let Some(target) = string(TARGET) {
            return format!("the element {target:?}");
        }
        if let Some(text) = string("text") {
            return format!("the text {text:?}");
        }
        if let Some(app) = string("app") {
            return format!("{app} to show its first screen");
        }
        if flag("change") {
            return "a change since the session's last snapshot".to_owned();
        }
        if flag("settle") {
            return "the screen to settle".to_owned();
        }
        if let Some(ms) = self.line.get("ms").and_then(Value::as_u64) {
            return format!("{ms} ms to pass");
        }
        "the daemon's reply".to_owned()
    }
}

impl Tool {
    /// The tool's definition, as `tools/list` gives it, with its reply's
    /// schema as `outputSchema` when `structured`.
    pub fn definition(&self, structured: bool) -> rmcp::model::Tool {
        let mut schema = (self.parameters)().as_ref().clone();
        let mut properties = match schema.remove("properties") {
            Some(Value::Object(properties)) => properties,
            _ => Map::new(),
        };
        if let Some(target) = properties.remove(TARGET) {
            properties.insert(SELECTOR.to_owned(), target);
        }
        for property in properties.values_mut() {
            if let Some(Value::String(description)) = property.get_mut("description") {
                *description = description.replace("`target`", "`selector`");
            }
        }
        if let Some(Value::Array(required)) = schema.get_mut("required") {
            for name in required.iter_mut() {
                if name == TARGET {
                    *name = SELECTOR.into();
                }
            }
        }
        for change in self.changes {
            match *change {
                Change::On(name) => {
                    if let Some(property) = properties.get_mut(name) {
                        property["default"] = true.into();
                    }
                }
                Change::Set(name) => {
                    properties.remove(name);
                }
                Change::Own(name, description) => {
                    properties.insert(
                        name.to_owned(),
                        json!({"default": false, "description": description, "type": "boolean"}),
                    );
                }
                Change::Force => {
                    properties.insert("force".to_owned(), force_schema());
                }
                Change::Describe(name, description) => {
                    if let Some(property) = properties.get_mut(name) {
                        property["description"] = description.into();
                    }
                }
            }
        }
        properties.insert("session".to_owned(), session_schema());
        properties.insert("device".to_owned(), device_schema());
        schema.insert("properties".to_owned(), Value::Object(properties));
        let annotations = match self.effect {
            Effect::Reads => ToolAnnotations::new().read_only(true),
            Effect::Acts => ToolAnnotations::new().destructive(false),
            Effect::Stops => ToolAnnotations::new().destructive(true),
        };
        let mut definition = rmcp::model::Tool::new(self.name, self.description, schema)
            .with_annotations(annotations);
        if structured {
            definition.output_schema = self.reply.map(|schema| schema());
        }
        definition
    }

    /// The tool's own flag `name`, if it has one.
    fn own(&self, name: &str) -> Option<&'static str> {
        self.changes.iter().find_map(|change| match *change {
            Change::Own(own, _) if own == name => Some(own),
            _ => None,
        })
    }

    /// The request a call with `arguments` sends, in `session` unless the
    /// call names another, with relative paths taken from `cwd` and the
    /// durations it names capped by `bound`. Only the parameters
    /// `definition` lists go out, mapped back through the table's changes,
    /// each value as the call gave it; null is a parameter not given.
    pub fn call(
        &self,
        definition: &rmcp::model::Tool,
        arguments: &JsonObject,
        session: &str,
        cwd: &Path,
        bound: Duration,
    ) -> Call {
        let listed = definition.input_schema.get("properties");
        let takes = |name: &str| listed.and_then(|p| p.get(name)).is_some();
        let mut line = Map::new();
        let mut flags = Vec::new();
        line.insert("id".to_owned(), cli::client::request_id().into());
        line.insert("action".to_owned(), self.action.into());
        let session = match arguments.get("session") {
            None | Some(Value::Null) => Some(session.to_owned()),
            Some(Value::String(named)) if !named.is_empty() => Some(named.clone()),
            // The daemon refuses a session that is no string, or empty.
            Some(other) => {
                line.insert("session".to_owned(), other.clone());
                None
            }
        };
        if let Some(session) = &session {
            line.insert("session".to_owned(), session.clone().into());
        }
        for (name, value) in arguments {
            if name == "session" || !takes(name) || value.is_null() {
                continue;
            }
            if let Some(flag) = self.own(name) {
                if *value == Value::Bool(true) {
                    flags.push(flag);
                }
                continue;
            }
            let key = if name == SELECTOR { TARGET } else { name };
            line.insert(key.to_owned(), value.clone());
        }
        for change in self.changes {
            match *change {
                Change::On(name) if !line.contains_key(name) => {
                    line.insert(name.to_owned(), true.into());
                }
                Change::Set(name) => {
                    line.insert(name.to_owned(), true.into());
                }
                _ => {}
            }
        }
        // A relative path is the server's working directory's, as a
        // relative path the CLI takes is the CLI's: the daemon runs from
        // the root.
        if let Some(Value::String(path)) = line.get_mut("path")
            && Path::new(path.as_str()).is_relative()
        {
            *path = cwd.join(path.as_str()).to_string_lossy().into_owned();
        }
        // The bound caps the durations the call names; a timeout the call
        // does not name stays the daemon's.
        let limit = u64::try_from(bound.saturating_sub(MARGIN).as_millis()).unwrap_or(u64::MAX);
        let mut capped = Vec::new();
        for key in DURATIONS {
            if let Some(given) = line.get(key).and_then(Value::as_u64)
                && given > limit
            {
                line.insert(key.to_owned(), limit.into());
                capped.push(format!(
                    "{key} {given} ms was capped to {limit} ms, the server's bound of {} s less its margin of {} s",
                    bound.as_secs(),
                    MARGIN.as_secs()
                ));
            }
        }
        let device = line
            .get("device")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Call {
            session,
            device,
            line,
            flags,
            capped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(value: Value) -> JsonObject {
        match value {
            Value::Object(arguments) => arguments,
            _ => unreachable!("arguments are an object"),
        }
    }

    /// A call of `tool` with `value` as its arguments, within a bound of
    /// 12 s.
    fn call(tool: &str, value: Value) -> Call {
        let tool = TOOLS
            .iter()
            .find(|candidate| candidate.name == tool)
            .expect("a tool");
        let mut call = tool.call(
            &tool.definition(false),
            &arguments(value),
            "mcp-1",
            Path::new("/work"),
            Duration::from_secs(12),
        );
        call.line.remove("id");
        call
    }

    /// The line of [`call`].
    fn line(tool: &str, value: Value) -> Map<String, Value> {
        call(tool, value).line
    }

    #[test]
    fn the_table_is_sorted_by_name_with_one_tool_per_action() {
        let names: Vec<&str> = TOOLS.iter().map(|tool| tool.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        for tool in &TOOLS {
            assert!(
                tool.action == "help" || wire::Command::ACTIONS.contains(&tool.action),
                "{}",
                tool.action
            );
        }
    }

    #[test]
    fn a_call_goes_out_as_the_daemon_s_line_through_the_table() {
        assert_eq!(
            Value::Object(line("click", json!({"selector": "@e3", "hold": 600}))),
            json!({"action": "click", "session": "mcp-1", "target": "@e3", "hold": 600})
        );
        assert_eq!(
            Value::Object(line("snapshot", json!({}))),
            json!({"action": "snapshot", "session": "mcp-1", "interactive": true, "delta": true}),
            "snapshot is interactive and a delta by default"
        );
        assert_eq!(
            Value::Object(line(
                "snapshot",
                json!({"delta": false, "maxDepth": "deep"})
            )),
            json!({"action": "snapshot", "session": "mcp-1", "interactive": true, "delta": false, "maxDepth": "deep"}),
            "a value goes out as the call gave it"
        );
        assert_eq!(
            Value::Object(line(
                "open",
                json!({"app": "com.example", "force": true, "device": "emulator-5556", "session": "resumed"})
            )),
            json!({"action": "open", "session": "resumed", "app": "com.example", "force": true, "device": "emulator-5556"})
        );
        assert_eq!(
            Value::Object(line(
                "screenshot",
                json!({"path": "shot.jpg", "ifChanged": true, "inline": false, "action": "shutdown"})
            )),
            json!({"action": "screenshot", "session": "mcp-1", "path": "/work/shot.jpg", "inline": true}),
            "the server's own flags and parameters the tool does not take stay out"
        );
        assert_eq!(
            Value::Object(line("click", json!({"force": true, "platform": "ios"}))),
            json!({"action": "click", "session": "mcp-1"}),
            "force is open's alone, and platform no tool's"
        );
    }

    #[test]
    fn a_definition_carries_its_reply_s_schema_when_structured() {
        for tool in &TOOLS {
            assert!(
                tool.definition(false).output_schema.is_none(),
                "{}: no outputSchema without the flag",
                tool.name
            );
            let typed = tool.definition(true).output_schema;
            let untyped = ["doctor", "help"].contains(&tool.name);
            assert_eq!(typed.is_none(), untyped, "{}", tool.name);
            if let Some(schema) = typed {
                assert_eq!(
                    schema.get("type"),
                    Some(&Value::from("object")),
                    "{}",
                    tool.name
                );
                assert!(schema.get("title").is_none(), "{}", tool.name);
            }
        }
    }

    /// Design D7 of real-app-fixes: the bound caps a timeout a call names
    /// and gives none to a call that names none, so that the daemon's own
    /// default rules, the session's, else 10 s.
    #[test]
    fn the_bound_caps_a_call_s_timeout_and_gives_none_to_a_call_without_one() {
        for (tool, arguments) in [
            ("wait", json!({"text": "Welcome"})),
            ("wait", json!({"selector": "#spinner", "state": "hidden"})),
            ("wait", json!({"change": true})),
            ("wait", json!({"settle": true})),
            ("click", json!({"selector": "#submit"})),
            ("fill", json!({"selector": "#name", "text": "agent"})),
            ("type", json!({"text": "agent"})),
            ("get", json!({"what": "text", "selector": "#name"})),
            ("open", json!({"app": "com.example"})),
            ("scroll", json!({"until": "#row-150"})),
            ("snapshot", json!({})),
            ("screenshot", json!({})),
            ("press", json!({"key": "Enter"})),
        ] {
            let sent = call(tool, arguments);
            assert!(
                !sent.line.contains_key("timeout") && sent.capped.is_empty(),
                "{tool} sends the daemon no timeout: {:?}",
                sent.line
            );
        }
        let given = call("wait", json!({"text": "Welcome", "timeout": 5000}));
        assert_eq!(
            given.line["timeout"], 5000,
            "a timeout under the bound as given"
        );
        assert!(given.capped.is_empty());
        let at = call("click", json!({"selector": "#submit", "timeout": 10_000}));
        assert_eq!(
            at.line["timeout"], 10_000,
            "the bound of 12 s less 2 s, as given"
        );
        assert!(at.capped.is_empty());
        let above = call("wait", json!({"text": "Welcome", "timeout": 60_000}));
        assert_eq!(above.line["timeout"], 10_000);
        assert_eq!(
            above.capped,
            vec![
                "timeout 60000 ms was capped to 10000 ms, the server's bound of 12 s less its margin of 2 s"
            ]
        );
        let click = call("click", json!({"selector": "#submit", "timeout": 30_000}));
        assert_eq!(click.line["timeout"], 10_000, "a click's timeout is capped");
        assert_eq!(click.capped.len(), 1);
        let time = call("wait", json!({"ms": 30_000}));
        assert_eq!(time.line["ms"], 10_000, "a time wait is capped too");
        assert!(!time.line.contains_key("timeout"), "{:?}", time.line);
        assert_eq!(time.capped.len(), 1);
        let settle = call("wait", json!({"settle": true, "timeout": 20_000}));
        assert_eq!(
            settle.line["timeout"], 10_000,
            "a settle's timeout is capped"
        );
    }

    #[test]
    fn a_call_says_what_it_waits_for() {
        assert_eq!(
            call("wait", json!({"text": "Welcome"})).waits_for(),
            "the text \"Welcome\""
        );
        assert_eq!(
            call("wait", json!({"selector": "#spinner", "state": "hidden"})).waits_for(),
            "the element \"#spinner\""
        );
        assert_eq!(
            call("scroll", json!({"until": "#row-150"})).waits_for(),
            "\"#row-150\" to come into view"
        );
        assert_eq!(
            call("open", json!({"app": "com.example"})).waits_for(),
            "com.example to show its first screen"
        );
        assert_eq!(
            call("wait", json!({"change": true})).waits_for(),
            "a change since the session's last snapshot"
        );
        assert_eq!(
            call("wait", json!({"settle": true})).waits_for(),
            "the screen to settle"
        );
        assert_eq!(
            call("wait", json!({"ms": 500})).waits_for(),
            "500 ms to pass"
        );
        assert_eq!(
            call("snapshot", json!({})).waits_for(),
            "the daemon's reply"
        );
    }
}
