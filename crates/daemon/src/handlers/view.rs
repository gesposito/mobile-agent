//! What the screen shows: snapshots, diffs, element reads, waits and frames.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mirror::{
    Condition, DEFAULT_MAX_DIMENSION, DEFAULT_QUALITY, DEFAULT_QUIET_WINDOW, Diff, Element,
    FrameRequest, Selector, SettleOptions, Signal, Signals, Snapshot, SnapshotOptions,
};
use serde_json::{Map, Value, json};

use super::aim::{self, Aim};
use super::{millis, origin, settle_max, timeout, verdict_json, verdict_warning};
use crate::driver::CaptureRequest;
use crate::signals::{LastChanges, ended_by};
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
pub(crate) fn element_json(element: &Element) -> Value {
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
    let started = Instant::now();
    let read_before = target.mirror.timestamps().last_read;
    target.catch_up()?;
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
    // How long the target's worker took to answer, device reads included,
    // and whether it read the device at all.
    data.insert("readMs".into(), millis(started.elapsed()).into());
    data.insert(
        "servedFromReplica".into(),
        (target.mirror.timestamps().last_read == read_before).into(),
    );
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
    target.catch_up()?;
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
            // Any element the replica holds, in view or not (design D51).
            let element = aim::held(target, session, &aim, timeout(request)?)?;
            let in_view = aim::in_view(target, &element);
            let b = element.bounds;
            match what {
                "text" => json!({
                    "text": element.value.clone().filter(|v| !v.is_empty()).unwrap_or(element.name),
                    "inView": in_view,
                }),
                "value" => json!({"value": element.value, "inView": in_view}),
                _ => json!({
                    "x": b.left, "y": b.top, "width": b.width(), "height": b.height(),
                    "inView": in_view,
                }),
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

/// A signal as replies name it, with the names `endedBy` uses.
fn signal_name(signal: &Signal) -> &'static str {
    match signal {
        Signal::Tree => "tree",
        Signal::Frames => "frame",
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
    let (aimed, kind, given) = match request.str("text") {
        Some(text) => (Aim::Select(Selector::text(text)), "text", text.to_owned()),
        None => {
            let given = request.str("target").ok_or_else(|| {
                Failure::new(
                    code::INVALID_ARGUMENT,
                    "wait takes an element, a number of milliseconds, --text or --settle",
                )
            })?;
            let aimed = match Aim::parse(given)? {
                Aim::Ref(reference) => {
                    aim::check_seen(target, session, reference)?;
                    Aim::Select(Selector::reference(reference))
                }
                other => other,
            };
            (aimed, "selector", given.to_owned())
        }
    };
    // Attached and detached ask whether the replica holds the element, in
    // view or not (design D51); visible, hidden and enabled whether it is on
    // screen.
    let condition = match request.str("state") {
        None | Some("visible") => Condition::Appear,
        Some("attached") => Condition::Attached,
        Some("hidden") => Condition::Disappear,
        Some("detached") => Condition::Detached,
        Some("enabled") => Condition::Enabled,
        Some(other) => {
            return Err(Failure::new(
                code::INVALID_ARGUMENT,
                format!(
                    "--state takes visible, attached, hidden, detached or enabled, not {other:?}"
                ),
            ));
        }
    };
    let timeout = timeout(request)?;
    target.catch_up()?;
    let held = matches!(condition, Condition::Attached | Condition::Detached);
    let outcome = match &aimed {
        Aim::Exact(text) if held => target.mirror.wait(&aim::exact(text), condition, timeout)?,
        Aim::Exact(text) => aim::wait_exact(target, text, condition, timeout)?,
        Aim::Select(selector) => target.mirror.wait(selector, condition, timeout)?,
        Aim::Ref(reference) => {
            target
                .mirror
                .wait(&Selector::reference(*reference), condition, timeout)?
        }
    };
    if let Some(reference) = outcome.element.as_ref().and_then(|e| e.reference) {
        target.session(session).seen.insert(reference);
    }
    let mut data = json!({
        "waited": kind,
        "elapsedMs": millis(outcome.elapsed),
        "element": outcome.element.as_ref().map(element_json),
    });
    if let Some(element) = &outcome.element {
        data["inView"] = aim::in_view(target, element).into();
    }
    data[kind] = given.into();
    Ok(Success::new(data))
}

fn settle(target: &mut Attached, request: &Request) -> Outcome {
    let max = settle_max(request)?;
    let quiet = request
        .u64("quiet")?
        .map_or(DEFAULT_QUIET_WINDOW, Duration::from_millis);
    let signals = if request.flag("treeOnly") {
        Signals::TreeOnly
    } else {
        Signals::TreeAndFrames
    };
    let began = Instant::now();
    let settled = target.mirror.settle(&SettleOptions {
        quiet,
        max,
        signals,
    })?;
    // The changes as the settle predicate counted them.
    let last = LastChanges::of(
        &target.mirror.timestamps(),
        target.frames && signals != Signals::TreeOnly,
    );
    let ended = ended_by(settled.settled, last, began, quiet);
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
        "endedBy": ended.name(),
        "elapsedMs": millis(settled.elapsed),
        "stillChanging": settled.still_changing.iter().map(signal_name).collect::<Vec<_>>(),
        "treeChanges": settled.tree_changes,
        "frameChanges": settled.frame_changes,
    }))
    .warn(warning))
}

pub(super) fn screenshot(target: &mut Attached, _session: &str, request: &Request) -> Outcome {
    let max_dimension = request
        .u64("maxDimension")?
        .map(|px| u32::try_from(px).unwrap_or(u32::MAX));
    let quality = request
        .u64("quality")?
        .map(|q| u8::try_from(q).unwrap_or(u8::MAX));
    // The driver's own screenshot when it takes one, the screen as it is
    // at any size; else the mirror's latest frame.
    let shot = match target.capture.as_mut() {
        Some(capture) => {
            let quality = quality.unwrap_or(DEFAULT_QUALITY).clamp(1, 100);
            let image = capture.screenshot(&CaptureRequest {
                max_dimension: max_dimension.unwrap_or(DEFAULT_MAX_DIMENSION).max(1),
                quality,
            })?;
            Shot {
                data: image.data,
                format: image.format,
                width: image.width,
                height: image.height,
                quality,
                seq: None,
                age: Duration::ZERO,
                source: "capture",
            }
        }
        None => {
            let frame = target.mirror.frame(&FrameRequest {
                max_dimension,
                quality,
            })?;
            Shot {
                data: frame.data,
                format: "jpeg",
                width: frame.width,
                height: frame.height,
                quality: frame.quality,
                seq: Some(frame.seq),
                age: frame.age,
                source: "frame",
            }
        }
    };
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
            let extension = if shot.format == "png" { "png" } else { "jpg" };
            std::env::temp_dir().join(format!("mobile-agent-{name}-{stamp}.{extension}"))
        }
    };
    std::fs::write(&path, &shot.data).map_err(|e| {
        Failure::new(
            IO_ERROR,
            format!("cannot write the screenshot to {}: {e}", path.display()),
        )
    })?;
    Ok(Success::new(json!({
        "path": path.to_string_lossy(),
        "format": shot.format,
        "width": shot.width,
        "height": shot.height,
        "quality": shot.quality,
        "seq": shot.seq,
        "ageMs": millis(shot.age),
        "source": shot.source,
    })))
}

/// An encoded screenshot, from the driver or the latest frame.
struct Shot {
    data: Vec<u8>,
    /// `jpeg` or `png`.
    format: &'static str,
    width: u32,
    height: u32,
    quality: u8,
    /// The frame's number, for a frame.
    seq: Option<u64>,
    age: Duration,
    /// `capture` or `frame`.
    source: &'static str,
}
