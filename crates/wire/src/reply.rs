//! The reply line.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One reply line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply<T = Value> {
    pub id: Value,
    pub success: bool,
    pub data: Option<T>,
}

macro_rules! replies {
    ($($name:ident),* $(,)?) => {
        $(
            #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
            #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
            #[serde(deny_unknown_fields)]
            pub struct $name {}
        )*
    };
}

replies!(
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
    Details,
    DeltaSnapshot,
);
