//! The listing's reply: the data of `snapshot` without `delta`, built from
//! the mirror's snapshot of the screen: the listing's text, its refs map,
//! the refs that left the screen, the snapshot's origin, revision and
//! verdict, and what the worker's answer cost (`README.md`, Protocol).

use mirror::Snapshot;
use serde_json::{Map, Value, json};

use crate::handlers::{origin, verdict_json, verdict_warning};
use crate::wire::Success;

/// What the listing's reply says of the worker's answer beside the
/// snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Answer<'a> {
    /// The target's worker's time to answer, device reads included, in
    /// milliseconds.
    pub read_ms: f64,
    /// Whether the worker answered without reading the device.
    pub served_from_replica: bool,
    /// The target's id.
    pub target: &'a str,
}

/// The reply to `snapshot` without `delta` for `snapshot`, which the
/// target's worker answered as `answer` says, with the warning a degraded
/// snapshot carries.
pub fn reply(snapshot: &Snapshot, answer: &Answer<'_>) -> Success {
    let mut data = Map::new();
    let refs: Map<String, Value> = snapshot
        .elements
        .iter()
        .filter_map(|element| {
            element.reference.map(|reference| {
                (
                    reference.to_string(),
                    json!({"name": element.name, "role": element.role.as_str()}),
                )
            })
        })
        .collect();
    let removed: Vec<String> = snapshot
        .removed_refs
        .iter()
        .map(|r| format!("@{r}"))
        .collect();
    data.insert("snapshot".into(), snapshot.text.clone().into());
    data.insert("refs".into(), Value::Object(refs));
    data.insert("removedRefs".into(), removed.into());
    data.insert("origin".into(), origin(snapshot).into());
    data.insert("readMs".into(), answer.read_ms.into());
    data.insert(
        "servedFromReplica".into(),
        answer.served_from_replica.into(),
    );
    // The mirror's replica revision.
    data.insert("replicaRevision".into(), snapshot.revision.into());
    data.insert("target".into(), answer.target.into());
    data.insert("verdict".into(), verdict_json(&snapshot.verdict));
    Success::new(Value::Object(data)).warn(verdict_warning(&snapshot.verdict))
}
