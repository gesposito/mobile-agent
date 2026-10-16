//! What the screen shows: snapshots, diffs, element reads, waits and frames.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mirror::{
    Condition, Diff, Element, FrameRequest, Selector, SettleOptions, Signal, Signals, Snapshot,
    SnapshotOptions,
};
use serde_json::{Map, Value, json};

use super::aim::{self, Aim};
use super::{Call, millis, origin, settle_max, timeout, verdict_json, verdict_warning};
use crate::driver::CaptureRequest;
use crate::signals::{LastChanges, ended_by};
use crate::wire::code::{self, IO_ERROR};
use crate::wire::{Failure, Param, Success, arg, needed};
use crate::worker::{Attached, Outcome};

/// The snapshot options a request asks for.
pub(super) fn snapshot_options(
    interactive: bool,
    max_depth: &Param<Option<u64>>,
) -> Result<SnapshotOptions, Failure> {
    let max_depth =
        arg("maxDepth", max_depth)?.map(|depth| usize::try_from(depth).unwrap_or(usize::MAX));
    Ok(SnapshotOptions {
        interactive,
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

pub(super) fn snapshot(target: &mut Attached, call: &Call, args: &::wire::Snapshot) -> Outcome {
    let session = call.session.as_str();
    let options = snapshot_options(args.interactive, &args.max_depth)?;
    let started = Instant::now();
    let read_before = target.mirror.timestamps().last_read;
    target.catch_up()?;
    let snapshot = target.mirror.snapshot(&options)?;
    let mut data = Map::new();
    if args.delta {
        let delta = delta_of(target, session, &options, args.full)?;
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

/// The listing with `options` as `snapshot` with `delta` gives it: the
/// delta against the mirror's last delta snapshot, or the whole listing
/// when `full` or when another session took that snapshot, whose delta
/// state is no base for this one.
fn delta_of(
    target: &mut Attached,
    session: &str,
    options: &SnapshotOptions,
    full: bool,
) -> Result<Value, Failure> {
    let full = full || target.delta_session.as_deref() != Some(session);
    let delta = target.mirror.delta_snapshot(options, full)?;
    target.delta_session = Some(session.to_owned());
    serde_json::from_str(&delta.to_json())
        .map_err(|e| Failure::internal(format!("the delta snapshot is not JSON: {e}")))
}

/// Keeps `snapshot` as the session's last, with the latest frame change it
/// saw, and its refs as the session's.
fn remember(target: &mut Attached, session: &str, snapshot: Snapshot) {
    let frames = target.mirror.timestamps().last_frame_change;
    let entry = target.session(session);
    entry.seen.extend(snapshot.refs());
    entry.frames = frames;
    entry.last = Some(snapshot);
}

pub(super) fn diff_snapshot(
    target: &mut Attached,
    call: &Call,
    args: &::wire::DiffSnapshot,
) -> Outcome {
    let session = call.session.as_str();
    let baseline = target.session(session).last.clone();
    let options = match &baseline {
        Some(baseline) => baseline.options,
        None => snapshot_options(args.interactive, &args.max_depth)?,
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

pub(super) fn get(target: &mut Attached, call: &Call, args: &::wire::Get) -> Outcome {
    let what = needed("get", "what", &args.what)?;
    let given = needed("get", "target", &args.target)?;
    let aim = Aim::parse(given)?;
    let data = match what {
        "text" | "value" | "box" => {
            // Any element the replica holds, in view or not (design D51).
            let element = aim::held(target, &call.session, &aim, timeout(&args.timeout, call)?)?;
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

pub(super) fn wait(target: &mut Attached, call: &Call, args: &::wire::Wait) -> Outcome {
    let session = call.session.as_str();
    if let Some(ms) = arg("ms", &args.ms)? {
        target
            .mirror
            .run_until(Instant::now() + Duration::from_millis(ms))?;
        return Ok(Success::new(json!({"waited": "time", "ms": ms})));
    }
    if args.settle {
        return settle(target, args);
    }
    if args.change {
        return change(target, call, args);
    }
    let (aimed, kind, given) = match args.text.as_deref() {
        Some(text) => (Aim::Select(Selector::text(text)), "text", text.to_owned()),
        None => {
            let given = args.target.as_deref().ok_or_else(|| {
                Failure::new(
                    code::INVALID_ARGUMENT,
                    "wait takes an element, a number of milliseconds, --text, --settle or --change",
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
    let condition = match args.state.as_str() {
        "visible" => Condition::Appear,
        "attached" => Condition::Attached,
        "hidden" => Condition::Disappear,
        "detached" => Condition::Detached,
        "enabled" => Condition::Enabled,
        other => {
            return Err(Failure::new(
                code::INVALID_ARGUMENT,
                format!(
                    "--state takes visible, attached, hidden, detached or enabled, not {other:?}"
                ),
            ));
        }
    };
    let timeout = timeout(&args.timeout, call)?;
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

fn settle(target: &mut Attached, args: &::wire::Wait) -> Outcome {
    let max = settle_max(&args.timeout)?;
    let quiet = Duration::from_millis(arg("quiet", &args.quiet)?);
    let signals = if args.tree_only {
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

/// How often a change wait looks whether the replica or the frames moved;
/// the mirror keeps the replica current in between.
const CHANGE_SLICE: Duration = Duration::from_millis(5);

/// What a session's last snapshot saw: the replica's revision and the
/// latest frame change.
struct Seen {
    revision: u64,
    frames: Option<Instant>,
}

/// What moved since a session's last snapshot.
#[derive(Clone, Copy)]
struct Moved {
    tree: bool,
    frames: bool,
}

impl Moved {
    /// What moved on `target` since `seen`; for a session that took no
    /// snapshot, the tree, which it has not seen.
    fn since(target: &Attached, seen: Option<&Seen>) -> Moved {
        match seen {
            Some(seen) => Moved {
                tree: target.mirror.revision() > seen.revision,
                frames: target.mirror.timestamps().last_frame_change > seen.frames,
            },
            None => Moved {
                tree: true,
                frames: false,
            },
        }
    }

    fn any(self) -> bool {
        self.tree || self.frames
    }

    /// As `endedBy` names it, with the names a settle's reply uses.
    fn name(self) -> &'static str {
        match (self.tree, self.frames) {
            (true, true) => "both",
            (true, false) => "tree",
            _ => "frame",
        }
    }
}

/// `wait` with `change` (design D8 of mcp-server): waits until the
/// replica's revision or the frames have moved since the session's last
/// snapshot, a change that came before the wait ending it at once, then
/// answers with the delta `snapshot` with `delta` gives, in the options of
/// that snapshot (interactive for a session that took none), which becomes
/// the session's last snapshot as a snapshot does.
fn change(target: &mut Attached, call: &Call, args: &::wire::Wait) -> Outcome {
    let session = call.session.as_str();
    let timeout = timeout(&args.timeout, call)?;
    let started = Instant::now();
    let deadline = started + timeout;
    let (seen, options) = {
        let entry = target.session(session);
        let seen = entry.last.as_ref().map(|last| Seen {
            revision: last.revision,
            frames: entry.frames,
        });
        let options = entry
            .last
            .as_ref()
            .map_or_else(SnapshotOptions::interactive, |last| last.options);
        (seen, options)
    };
    loop {
        target.mirror.drain();
        if Moved::since(target, seen.as_ref()).any() {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(Failure::new(
                mirror::code::WAIT_TIMEOUT,
                format!(
                    "waited {} ms for a change since the session's last snapshot; neither the tree nor the frames changed, so that snapshot still shows the screen",
                    timeout.as_millis()
                ),
            ));
        }
        target
            .mirror
            .run_until((now + CHANGE_SLICE).min(deadline))?;
    }
    // Frames that moved call for the read that shows what they show, and an
    // event that arrived for the read it calls for.
    target.catch_up()?;
    target.mirror.pump()?;
    let moved = Moved::since(target, seen.as_ref());
    let elapsed = started.elapsed();
    let snapshot = target.mirror.snapshot(&options)?;
    let delta = delta_of(target, session, &options, false)?;
    let warning = verdict_warning(&snapshot.verdict);
    remember(target, session, snapshot);
    Ok(Success::new(json!({
        "waited": "change",
        "endedBy": moved.name(),
        "elapsedMs": millis(elapsed),
        "snapshot": delta,
    }))
    .warn(warning))
}

pub(super) fn screenshot(
    target: &mut Attached,
    _call: &Call,
    args: &::wire::Screenshot,
) -> Outcome {
    let max_dimension =
        u32::try_from(arg("maxDimension", &args.max_dimension)?).unwrap_or(u32::MAX);
    let quality = u8::try_from(arg("quality", &args.quality)?).unwrap_or(u8::MAX);
    // The driver's own screenshot when it takes one, the screen as it is
    // at any size; else the mirror's latest frame.
    let shot = match target.capture.as_mut() {
        Some(capture) => {
            let quality = quality.clamp(1, 100);
            let image = capture.screenshot(&CaptureRequest {
                max_dimension: max_dimension.max(1),
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
                max_dimension: Some(max_dimension),
                quality: Some(quality),
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
    let path = match args.path.as_deref() {
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
