//! What the screen shows: snapshots, diffs, element reads, waits and frames.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mirror::{
    Condition, DEFAULT_QUIET_WINDOW, DEFAULT_SETTLE_MAX, Diff, Element, FrameRequest, Selector,
    SettleOptions, Signal, Signals, Snapshot, SnapshotOptions,
};
use serde_json::{Map, Value, json};

use super::aim::{self, Aim};
use super::{millis, origin, timeout, verdict_json, verdict_warning};
use crate::wire::code::{self, IO_ERROR};
use crate::wire::{Failure, Request, Success};
use crate::worker::{Attached, Outcome};

/// The snapshot options a request asks for.
pub(super) fn snapshot_options(request: &Request) -> Result<SnapshotOptions, Failure> {
    let max_depth = request
        .u64("maxDepth")?
        .map(|depth| usize::try_from(depth).unwrap_or(usize::MAX));
    Ok(SnapshotOptions {
        interactive: request.flag("interactive"),
        max_depth,
    })
}

/// An element as replies describe it.
pub(super) fn element_json(element: &Element) -> Value {
    let b = element.bounds;
    json!({
        "ref": element.reference.map(|r| format!("@{r}")),
        "role": element.role.as_str(),
        "name": element.name,
        "id": element.identifier,
        "value": element.value,
        "enabled": element.enabled,
        "bounds": [b.left, b.top, b.right, b.bottom],
    })
}

pub(super) fn snapshot(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let options = snapshot_options(request)?;
    let snapshot = target.mirror.snapshot(&options)?;
    let mut data = Map::new();
    if request.flag("delta") {
        // Another session's delta state is no base for this one.
        let full = request.flag("full") || target.delta_session.as_deref() != Some(session);
        let delta = target.mirror.delta_snapshot(&options, full)?;
        target.delta_session = Some(session.to_owned());
        let delta: Value = serde_json::from_str(&delta.to_json())
            .map_err(|e| Failure::internal(format!("the delta snapshot is not JSON: {e}")))?;
        data.insert("snapshot".into(), delta);
    } else {
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
    }
    data.insert("origin".into(), origin(&snapshot).into());
    // The mirror's replica revision, apart from the delta's own revision.
    data.insert("replicaRevision".into(), snapshot.revision.into());
    data.insert("target".into(), target.info.id.clone().into());
    data.insert("verdict".into(), verdict_json(&snapshot.verdict));
    let warning = verdict_warning(&snapshot.verdict);
    remember(target, session, snapshot);
    Ok(Success::new(Value::Object(data)).warn(warning))
}

/// Keeps `snapshot` as the session's last and its refs as the session's.
fn remember(target: &mut Attached, session: &str, snapshot: Snapshot) {
    let entry = target.session(session);
    entry.seen.extend(snapshot.refs());
    entry.last = Some(snapshot);
}

pub(super) fn diff_snapshot(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let baseline = target.session(session).last.clone();
    let options = match &baseline {
        Some(baseline) => baseline.options,
        None => snapshot_options(request)?,
    };
    let current = target.mirror.snapshot(&options)?;
    // Without a snapshot to compare with, every element is new.
    let baseline = baseline.unwrap_or_else(|| Snapshot {
        elements: Vec::new(),
        ..current.clone()
    });
    let diff = Diff::between(&baseline, &current);
    let unchanged = current
        .elements
        .len()
        .saturating_sub(diff.added.len() + diff.changed.len());
    let modified: Vec<Value> = diff
        .changed
        .iter()
        .map(|change| {
            let mut entry = element_json(&change.after);
            entry["fields"] = change
                .fields
                .iter()
                .map(|f| f.name())
                .collect::<Vec<_>>()
                .into();
            entry
        })
        .collect();
    let data = json!({
        "changed": !diff.is_empty(),
        "diff": diff.text(),
        "additions": diff.added.len(),
        "removals": diff.removed.len(),
        "modifications": diff.changed.len(),
        "unchanged": unchanged,
        "added": diff.added.iter().map(element_json).collect::<Vec<_>>(),
        "removed": diff.removed.iter().map(element_json).collect::<Vec<_>>(),
        "modified": modified,
        "baseReplicaRevision": baseline.revision,
        "replicaRevision": current.revision,
        "target": target.info.id,
    });
    remember(target, session, current);
    Ok(Success::new(data))
}

pub(super) fn get(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    let what = request.required_str("what")?;
    let given = request.required_str("target")?;
    let aim = Aim::parse(given)?;
    let data = match what {
        "text" | "value" | "box" => {
            let element = aim::element(target, session, &aim, timeout(request)?)?;
            let b = element.bounds;
            match what {
                "text" => json!({
                    "text": element.value.clone().filter(|v| !v.is_empty()).unwrap_or(element.name),
                }),
                "value" => json!({"value": element.value}),
                _ => json!({"x": b.left, "y": b.top, "width": b.width(), "height": b.height()}),
            }
        }
        other => {
            return Err(Failure::new(
                code::INVALID_ARGUMENT,
                format!("get takes text, value or box, not {other:?}"),
            ));
        }
    };
    Ok(Success::new(data))
}

fn signal_name(signal: &Signal) -> &'static str {
    match signal {
        Signal::Tree => "tree",
        Signal::Frames => "frames",
    }
}

pub(super) fn wait(target: &mut Attached, session: &str, request: &Request) -> Outcome {
    if let Some(ms) = request.u64("ms")? {
        target
            .mirror
            .run_until(Instant::now() + Duration::from_millis(ms))?;
        return Ok(Success::new(json!({"waited": "time", "ms": ms})));
    }
    if request.flag("settle") {
        return settle(target, request);
    }
    let (selector, kind, given) = match request.str("text") {
        Some(text) => (Selector::text(text), "text", text.to_owned()),
        None => {
            let given = request.str("target").ok_or_else(|| {
                Failure::new(
                    code::INVALID_ARGUMENT,
                    "wait takes an element, a number of milliseconds, --text or --settle",
                )
            })?;
            let selector = match Aim::parse(given)? {
                Aim::Ref(reference) => {
                    aim::check_seen(target, session, reference)?;
                    Selector::reference(reference)
                }
                Aim::Select(selector) => selector,
            };
            (selector, "selector", given.to_owned())
        }
    };
    let condition = match request.str("state") {
        None | Some("visible" | "attached") => Condition::Appear,
        Some("hidden" | "detached") => Condition::Disappear,
        Some("enabled") => Condition::Enabled,
        Some(other) => {
            return Err(Failure::new(
                code::INVALID_ARGUMENT,
                format!("--state takes visible, hidden, detached or enabled, not {other:?}"),
            ));
        }
    };
    let outcome = target
        .mirror
        .wait(&selector, condition, timeout(request)?)?;
    if let Some(reference) = outcome.element.as_ref().and_then(|e| e.reference) {
        target.session(session).seen.insert(reference);
    }
    let mut data = json!({
        "waited": kind,
        "elapsedMs": millis(outcome.elapsed),
        "element": outcome.element.as_ref().map(element_json),
    });
    data[kind] = given.into();
    Ok(Success::new(data))
}

fn settle(target: &mut Attached, request: &Request) -> Outcome {
    let max = request
        .u64("timeout")?
        .map_or(DEFAULT_SETTLE_MAX, Duration::from_millis);
    let quiet = request
        .u64("quiet")?
        .map_or(DEFAULT_QUIET_WINDOW, Duration::from_millis);
    let signals = if request.flag("treeOnly") {
        Signals::TreeOnly
    } else {
        Signals::TreeAndFrames
    };
    let settled = target.mirror.settle(&SettleOptions {
        quiet,
        max,
        signals,
    })?;
    let warning = (!settled.settled).then(|| {
        format!(
            "the screen did not settle within {} ms: {} kept arriving",
            max.as_millis(),
            settled.reason().unwrap_or_else(|| "changes".to_owned())
        )
    });
    Ok(Success::new(json!({
        "waited": "settle",
        "settled": settled.settled,
        "elapsedMs": millis(settled.elapsed),
        "stillChanging": settled.still_changing.iter().map(signal_name).collect::<Vec<_>>(),
        "treeChanges": settled.tree_changes,
        "frameChanges": settled.frame_changes,
    }))
    .warn(warning))
}

pub(super) fn screenshot(target: &mut Attached, _session: &str, request: &Request) -> Outcome {
    let frame = target.mirror.frame(&FrameRequest {
        max_dimension: request
            .u64("maxDimension")?
            .map(|px| u32::try_from(px).unwrap_or(u32::MAX)),
        quality: request
            .u64("quality")?
            .map(|q| u8::try_from(q).unwrap_or(u8::MAX)),
    })?;
    let path = match request.str("path") {
        Some(path) => PathBuf::from(path),
        None => {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_millis());
            let name: String = target
                .info
                .id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            std::env::temp_dir().join(format!("mobile-agent-{name}-{stamp}.jpg"))
        }
    };
    std::fs::write(&path, &frame.data).map_err(|e| {
        Failure::new(
            IO_ERROR,
            format!("cannot write the screenshot to {}: {e}", path.display()),
        )
    })?;
    Ok(Success::new(json!({
        "path": path.to_string_lossy(),
        "format": "jpeg",
        "width": frame.width,
        "height": frame.height,
        "quality": frame.quality,
        "seq": frame.seq,
        "ageMs": millis(frame.age),
    })))
}
