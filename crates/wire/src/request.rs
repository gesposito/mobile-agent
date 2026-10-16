//! The request line.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One request line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub id: Value,
    #[serde(default = "default_session")]
    pub session: String,
    #[serde(flatten)]
    pub command: Command,
}

fn default_session() -> String {
    "default".to_owned()
}

/// The action and its parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    Status(Status),
    Devices(Devices),
    Close(Close),
    SetTimeout(SetTimeout),
    Doctor(Doctor),
    Open(Open),
    Snapshot(Snapshot),
    DiffSnapshot(DiffSnapshot),
    Get(Get),
    Wait(Wait),
    Screenshot(Screenshot),
    Click(Click),
    Fill(Fill),
    #[serde(rename = "type")]
    Type(TypeText),
    Press(Press),
    Swipe(Swipe),
    Scroll(Scroll),
    Shutdown(Shutdown),
}

macro_rules! actions {
    ($($name:ident),* $(,)?) => {
        $(
            #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
            #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
            pub struct $name {}
        )*
    };
}

actions!(
    Status,
    Devices,
    Close,
    SetTimeout,
    Doctor,
    Open,
    Snapshot,
    DiffSnapshot,
    Get,
    Wait,
    Screenshot,
    Click,
    Fill,
    TypeText,
    Press,
    Swipe,
    Scroll,
    Shutdown,
);
