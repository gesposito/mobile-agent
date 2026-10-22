//! What the screen shows: snapshots, diffs, element reads, waits and frames.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use mirror::{
    Condition, Diff, Element, FrameRequest, Selector, Settle, SettleOptions, Signal, Signals,
    Snapshot, SnapshotOptions,
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
pub(super) fn remember(target: &mut Attached, session: &str, snapshot: Snapshot) {
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
    let (data, resolved) = match what {
        "text" | "value" | "box" => {
            // Any element the replica holds, in view or not (design D51),
            // a stale ref's twin for it (action-verdicts, design D3).
            let (element, resolved) =
                aim::held(target, &call.session, &aim, timeout(&args.timeout, call)?)?;
            // An element that holds a switch sharing its identifier, or that
            // such a switch holds, reads as that switch, the one a click on
            // it presses: on the second pass on real apps `get value` by the
            // identifier Settings' Bold Text switch shares with the table
            // cell around it read the cell, `no value` with the switch on,
            // and `get text` its empty name (real-app-fixes, task 5.4). Its
            // box stays the element's own.
            let element = match what {
                "box" => element,
                _ => aim::switch_of(target, &element)?
                    .map_or(element, |switch| switch.element().clone()),
            };
            let in_view = aim::in_view(target, &element);
            let b = element.bounds;
            let data = match what {
                "text" => json!({
                    "text": element.value.clone().filter(|v| !v.is_empty()).unwrap_or(element.name),
                    "inView": in_view,
                }),
                // A switch's state, or a checkbox's, as the listing has it:
                // on the demo pass `get value` on a switch returned nothing
                // (real-app-fixes, design D5).
                "value" => {
                    let mut data = json!({"value": element.value, "inView": in_view});
                    if let Some(checked) = element.checked {
                        data["checked"] = checked.into();
                    }
                    data
                }
                _ => json!({
                    "x": b.left, "y": b.top, "width": b.width(), "height": b.height(),
                    "inView": in_view,
                }),
            };
            (data, resolved)
        }
        other => {
            return Err(Failure::new(
                code::INVALID_ARGUMENT,
                format!("get takes text, value or box, not {other:?}"),
            ));
        }
    };
    Ok(aim::with_resolved(Success::new(data), resolved))
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
        return settle(target, call, args);
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
        Aim::Exact(text) if held => {
            aim::wait(target, session, (&aim::exact(text), condition), timeout)?
        }
        Aim::Exact(text) => aim::wait_exact(target, session, (text, condition), timeout)?,
        Aim::Select(selector) => aim::wait(target, session, (selector, condition), timeout)?,
        Aim::Ref(reference) => aim::wait(
            target,
            session,
            (&Selector::reference(*reference), condition),
            timeout,
        )?,
    };
    let element = outcome
        .element
        .map(|element| with_ref(target, element))
        .transpose()?;
    if let Some(reference) = element.as_ref().and_then(|e| e.reference) {
        target.session(session).seen.insert(reference);
    }
    let mut data = json!({
        "waited": kind,
        "elapsedMs": millis(outcome.elapsed),
        "element": element.as_ref().map(element_json),
    });
    if let Some(element) = &element {
        data["inView"] = aim::in_view(target, element).into();
    }
    data[kind] = given.into();
    Ok(Success::new(data))
}

/// `element`, which a wait found, with the ref the full listing gives it
/// when no snapshot has issued it one yet, so that the wait names it by a
/// ref the session can act on, as `✓ Found button "Sign in" @e4`
/// (real-app-fixes, design D5): an actionable element, or content with a
/// name, gets one. Any other element, as a plain text, is left without one,
/// and without the listing.
fn with_ref(target: &mut Attached, element: Element) -> Result<Element, Failure> {
    let bears = element.actionable || (element.role.is_content() && !element.name.is_empty());
    if element.reference.is_some() || !bears {
        return Ok(element);
    }
    let listing = target.mirror.snapshot(&SnapshotOptions::full())?;
    let reference = listing
        .elements
        .iter()
        .find(|listed| listed.id == element.id)
        .and_then(|listed| listed.reference);
    Ok(Element {
        reference,
        ..element
    })
}

/// How long after an input a settle waits for the input's first effect, a
/// change of the tree or of the frames since it, before it counts the
/// quiet window (design D2 of action-verdicts): on the second demo pass a
/// settle right after the Enter that loads a page settled at once, nothing
/// having changed yet, and one after the tap that opened a menu ended
/// before the menu's tree came.
pub(super) const EFFECT_WAIT: Duration = Duration::from_secs(1);

/// How often the wait for an input's first effect looks at what arrived;
/// the mirror keeps the replica current in between.
const EFFECT_SLICE: Duration = Duration::from_millis(5);

/// Waits until the tree, or the frames when `frames` says so, changed after
/// `since`, at most until `deadline`, a change that came before the wait
/// ending it at once; says whether one came.
fn first_change(
    target: &mut Attached,
    since: Instant,
    frames: bool,
    deadline: Instant,
) -> Result<bool, Failure> {
    loop {
        target.mirror.drain();
        let stamps = target.mirror.timestamps();
        let after = |at: Option<Instant>| at.is_some_and(|at| at > since);
        if after(stamps.last_tree_change) || (frames && after(stamps.last_frame_change)) {
            return Ok(true);
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        target
            .mirror
            .run_until((now + EFFECT_SLICE).min(deadline))?;
    }
}

/// Settles as `options` say after an input that went in at `input` (design
/// D2 of action-verdicts): within [`EFFECT_WAIT`] of it, the wait first
/// waits for the input's first effect, a change of the tree, or of the
/// frames unless `options` watch the tree alone, since the input, until
/// `EFFECT_WAIT` after it at most, or until `by` when that comes first;
/// then the mirror's settle counts the quiet window as it does after any
/// change, its predicate unchanged. The settle's elapsed time and counts of
/// changes cover both waits. Says when the input went in, if the settle
/// waited for its effect.
pub(super) fn settle_after(
    target: &mut Attached,
    input: Option<Instant>,
    (options, by): (&SettleOptions, Option<Instant>),
) -> Result<(Settle, Option<Instant>), Failure> {
    let began = Instant::now();
    let (tree, frames) = target.mirror.changes();
    let input = input.filter(|&at| began.saturating_duration_since(at) < EFFECT_WAIT);
    if let Some(at) = input {
        let deadline = by.map_or(at + EFFECT_WAIT, |by| by.min(at + EFFECT_WAIT));
        let watched = options.signals != Signals::TreeOnly;
        first_change(target, at, watched, deadline)?;
    }
    let waited = began.elapsed();
    let mut settled = target.mirror.settle(options)?;
    let (tree_now, frames_now) = target.mirror.changes();
    settled.elapsed += waited;
    settled.tree_changes = tree_now - tree;
    settled.frame_changes = frames_now - frames;
    Ok((settled, input))
}

/// `wait` with `settle`, after the session's last input when it came less
/// than [`EFFECT_WAIT`] ago, whose first effect the wait waits for
/// ([`settle_after`]): the reply names the input's action in `input`, and
/// in `endedBy` the signal that changed last since the input, `none` when
/// nothing changed within a second of it (design D2 of action-verdicts);
/// without such an input, the signal the settle ended on, as the quiet
/// window counted it.
fn settle(target: &mut Attached, call: &Call, args: &::wire::Wait) -> Outcome {
    let max = settle_max(&args.timeout)?;
    let quiet = Duration::from_millis(arg("quiet", &args.quiet)?);
    let signals = if args.tree_only {
        Signals::TreeOnly
    } else {
        Signals::TreeAndFrames
    };
    let began = Instant::now();
    let input = target.session(&call.session).input;
    let options = SettleOptions {
        quiet,
        max,
        signals,
    };
    let (settled, after) = settle_after(target, input.map(|input| input.at), (&options, None))?;
    let input = input.filter(|_| after.is_some());
    // The changes as the settle predicate counted them.
    let last = LastChanges::of(
        &target.mirror.timestamps(),
        target.frames && signals != Signals::TreeOnly,
    );
    let ended = ended_by(settled.settled, last, after.unwrap_or(began), quiet);
    let warning = (!settled.settled).then(|| {
        format!(
            "the screen did not settle within {} ms: {} kept arriving",
            max.as_millis(),
            settled.reason().unwrap_or_else(|| "changes".to_owned())
        )
    });
    let mut data = json!({
        "waited": "settle",
        "settled": settled.settled,
        "endedBy": ended.name(),
        "elapsedMs": millis(settled.elapsed),
        "stillChanging": settled.still_changing.iter().map(signal_name).collect::<Vec<_>>(),
        "treeChanges": settled.tree_changes,
        "frameChanges": settled.frame_changes,
    });
    if let Some(input) = input {
        data["input"] = input.action.into();
    }
    Ok(Success::new(data).warn(warning))
}

/// How often a change wait looks whether the replica's revision or the
/// frames moved; the mirror keeps the replica current in between.
const CHANGE_SLICE: Duration = Duration::from_millis(5);

/// What a session's last snapshot saw: the replica's revision it listed and
/// the latest frame change.
struct Seen {
    revision: u64,
    frames: Option<Instant>,
}

impl Seen {
    /// Whether the frames of `target` changed since.
    fn frames_moved(&self, target: &Attached) -> bool {
        target.mirror.timestamps().last_frame_change > self.frames
    }
}

/// What moved since a session's last snapshot: its listing, and the frames.
#[derive(Clone, Copy)]
struct Moved {
    listing: bool,
    frames: bool,
}

impl Moved {
    /// As `endedBy` names it, with the names a settle's reply uses, a
    /// change of the listing being the tree's.
    fn name(self) -> &'static str {
        match (self.listing, self.frames) {
            (true, true) => "both",
            (true, false) => "tree",
            _ => "frame",
        }
    }
}

/// Whether `current` lists other lines than the last snapshot of
/// `session`, which lists none for a session that took no snapshot.
fn listing_moved(target: &mut Attached, session: &str, current: &Snapshot) -> bool {
    target
        .session(session)
        .last
        .as_ref()
        .is_none_or(|last| last.text != current.text)
}

/// `wait` with `change` (design D8 of mcp-server): waits until the listing
/// in the options of the session's last snapshot (interactive for a session
/// that took none) differs from the one that snapshot showed, and with
/// `frames` until the frames moved too, a change that came before the wait
/// ending it at once. The listing is taken again only when the replica's
/// revision moved since the wait last took it, so a change that the listing
/// does not show leaves the wait running. It answers with the delta
/// `snapshot` with `delta` gives in those options, which becomes the
/// session's last snapshot as a snapshot does.
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
    // A session that took no snapshot has seen no listing.
    let mut listing = seen.is_none();
    if let Some(seen) = &seen {
        // The revision at which the wait last took the listing.
        let mut look = seen.revision;
        loop {
            target.mirror.drain();
            if args.frames && seen.frames_moved(target) {
                break;
            }
            if target.mirror.revision() > look {
                let current = target.mirror.snapshot(&options)?;
                look = current.revision;
                if listing_moved(target, session, &current) {
                    listing = true;
                    break;
                }
            }
            let now = Instant::now();
            if now >= deadline {
                let unchanged = if args.frames {
                    "neither its listing nor the frames changed"
                } else {
                    "its listing did not change"
                };
                return Err(Failure::new(
                    mirror::code::WAIT_TIMEOUT,
                    format!(
                        "waited {} ms for a change since the session's last snapshot; {unchanged}, so that snapshot still shows the screen",
                        timeout.as_millis()
                    ),
                ));
            }
            target
                .mirror
                .run_until((now + CHANGE_SLICE).min(deadline))?;
        }
    }
    // Frames that moved call for the read that shows what they show, and an
    // event that arrived for the read it calls for.
    target.catch_up()?;
    target.mirror.pump()?;
    let elapsed = started.elapsed();
    let snapshot = target.mirror.snapshot(&options)?;
    let moved = Moved {
        listing: listing || listing_moved(target, session, &snapshot),
        frames: seen.as_ref().is_some_and(|seen| seen.frames_moved(target)),
    };
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
    // The path names the file; without one, a new file in the temporary
    // directory, unless the image goes in the reply alone (design D6 of
    // mcp-server).
    let path = match args.path.as_deref() {
        Some(path) => Some(PathBuf::from(path)),
        None if args.inline => None,
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
            Some(std::env::temp_dir().join(format!("mobile-agent-{name}-{stamp}.{extension}")))
        }
    };
    if let Some(path) = &path {
        std::fs::write(path, &shot.data).map_err(|e| {
            Failure::new(
                IO_ERROR,
                format!("cannot write the screenshot to {}: {e}", path.display()),
            )
        })?;
    }
    // The screen's size in points beside the image's pixels: on the demo
    // pass a model took a 460x1000 screenshot's pixels for the 402x874
    // points a click takes (real-app-fixes, design D5).
    let points = target.display.size()?;
    let mut data = json!({
        "path": path.as_ref().map(|path| path.to_string_lossy()),
        "format": shot.format,
        "width": shot.width,
        "height": shot.height,
        "points": {"width": points.width, "height": points.height},
        "quality": shot.quality,
        "seq": shot.seq,
        "ageMs": millis(shot.age),
        "source": shot.source,
    });
    if args.inline {
        data["image"] = BASE64.encode(&shot.data).into();
        data["mimeType"] = if shot.format == "png" {
            "image/png"
        } else {
            "image/jpeg"
        }
        .into();
    }
    Ok(Success::new(data))
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
