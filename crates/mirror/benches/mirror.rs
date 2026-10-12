//! Microbenchmarks of the replica, the listings, visibility, deltas, waits
//! and settle (design D14), run by `mobile-agent-bench micro`. Each drives a
//! `Mirror` over the scripted `FakeTarget` through the crate's public API, on
//! the fixtures the tests read: `grid` (the SwiftUI 500-cell screen, 510
//! nodes), `list` (the Expo list, 51), `bluesky-typeahead-android` (46) and
//! `grid-rn` (the Expo 500-cell screen as the emulator reads it, 1,049). Ids
//! are `mirror/<function>/<parameter>`, the parameter naming the case and the
//! fixture.
//!
//! Before its first sample, each benchmark calls its routine
//! [`COUNTED_CALLS`] times more, outside the timed region, each call on a
//! state of its own that the benchmark's setup builds: it verifies every
//! call's result, checks that the calls allocated the same and prints what a
//! call allocated, and how many bytes, on a line of its own,
//! `allocations <id> <allocations> <bytes>`, for `micro` to read. A routine
//! that uses up its state, as a pump uses up the read that was due, runs on
//! a target and mirror that its setup builds afresh for every iteration
//! outside the timed region (`iter_batched_ref`); a routine that leaves its
//! state as it found it runs on one target and mirror (`iter`). Either way
//! the routine's output is dropped inside the timed region, as spike S8
//! timed the protocol's decode, so that a first snapshot and the same
//! snapshot again count the same work.
//!
//! Besides the mirror's own work, the timed region holds the fake target's:
//! a read hands over a clone of the screen the target shows, standing in for
//! the read a driver decodes (the protocol's benches time the decode), and
//! the event stream and the clock take the target's lock.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::measurement::WallTime;
use criterion::{
    BatchSize, BenchmarkGroup, BenchmarkId, Criterion, criterion_group, criterion_main,
};
use mirror::backend::{ReadReason, ReadRequest, StreamItem, TreeRead};
use mirror::testing::{FakeTarget, Step};
use mirror::{
    Condition, DEFAULT_QUIET_WINDOW, DeltaSnapshot, ElementId, Field, Mirror, MirrorConfig,
    MirrorError, Point, Rect, Ref, RefChange, RefField, Selector, Settle, SettleOptions,
    SnapshotOptions, TreeSplice, Uncovered, Visibility,
};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind, WindowChangeKind};
use serde_json::{Value, json};

#[path = "../../protocol/benches/support/alloc.rs"]
mod alloc;

#[path = "../tests/support/mod.rs"]
mod support;

use support::{Screen, timeline, window_event};

#[global_allocator]
static ALLOCATOR: alloc::Counting = alloc::Counting;

/// The application window of every fixture here, which scripted changes name.
const APP: u32 = 1;

/// The id of the navigation bar the window benchmarks add and remove.
const NAVIGATION_BAR: u32 = 3;

/// The longest one wait or settle on the fake clock may take in real time
/// (design D14, Risks): its waiting moves the fake clock, so it costs real
/// time only if something blocks.
const REAL_TIME_BOUND: Duration = Duration::from_millis(50);

/// How many calls a benchmark's allocations are counted over, each on a state
/// of its own. They must all allocate the same, or the benchmark fails:
/// `micro` compares the count exactly (design D14), so a count that varied
/// from call to call would show as a change no change made. Calls on equal
/// states do the same work: the replica takes the children of matched nodes
/// in the order of the reads, whatever the seeds of its hash maps, so the
/// stack of pairs it walks grows to the same peak. Calls on one shared state
/// would not: the call that grows the fake target's record of reads
/// allocates more than the others.
const COUNTED_CALLS: usize = 3;

/// What the benchmarks use: criterion's group of wall-clock measurements.
type Group<'a> = BenchmarkGroup<'a, WallTime>;

/// A fixture's screen and the read a driver returns for it.
struct Fixture {
    name: &'static str,
    screen: Screen,
    read: TreeRead,
}

impl Fixture {
    fn load(name: &'static str) -> Fixture {
        let screen = Screen::load(name);
        let read = screen.read();
        Fixture { name, screen, read }
    }

    /// Whether a window of the fixture sends no change events, which a
    /// settle without frames re-reads every minimum re-read interval.
    fn rereads_without_frames(&self) -> bool {
        self.read
            .snapshot
            .windows
            .iter()
            .any(|window| window.emits_events == Some(false))
    }
}

/// Elements and refs in a fixture's listing right after its first read.
fn listed(fixture: &str, interactive: bool) -> (usize, usize) {
    match (fixture, interactive) {
        ("grid", true) => (505, 505),
        ("grid", false) => (507, 505),
        ("list", true) => (21, 21),
        ("list", false) => (24, 21),
        ("bluesky-typeahead-android", true) => (18, 18),
        ("bluesky-typeahead-android", false) => (21, 18),
        ("grid-rn", true) => (505, 505),
        ("grid-rn", false) => (1_017, 505),
        _ => panic!("no listing recorded for {fixture}"),
    }
}

/// `screen` with the text of the text view that node `id` holds replaced, as
/// a row or a cell shows new text.
fn held_text(screen: &Screen, id: &str, text: &str) -> Screen {
    fn find<'a>(node: &'a mut Value, id: &str) -> Option<&'a mut Value> {
        if node["id"].as_str() == Some(id) {
            return Some(node);
        }
        node.get_mut("children")?
            .as_array_mut()?
            .iter_mut()
            .find_map(|child| find(child, id))
    }
    let mut value = screen.0.clone();
    let node = value["windows"]
        .as_array_mut()
        .expect("windows")
        .iter_mut()
        .find_map(|window| find(&mut window["root"], id))
        .unwrap_or_else(|| panic!("no node {id}"));
    node["children"][0]["text"] = json!(text);
    Screen(value)
}

/// A three-button navigation bar 48 points tall across the bottom of
/// `screen`'s application window, the system window the window benchmarks
/// add to a screen and remove from it: four nodes. On `list` it spans
/// `[0, 864, 408, 912]`.
fn navigation_bar(screen: &Screen) -> Value {
    let app = screen.0["windows"]
        .as_array()
        .expect("windows")
        .iter()
        .find(|window| window["id"] == APP)
        .expect("the application window");
    let edge = |i: usize| app["bounds"][i].as_f64().expect("the window's bounds");
    let (width, bottom) = (edge(2), edge(3));
    let top = bottom - 48.0;
    let button = |desc: &str, middle: f64| {
        json!({
            "class": "android.widget.ImageView", "desc": desc,
            "bounds": [middle - 32.0, top, middle + 32.0, bottom], "flags": ["clickable"]
        })
    };
    let middle = width / 2.0;
    json!({
        "id": NAVIGATION_BAR, "kind": "system", "layer": 3, "package": "com.android.systemui",
        "title": "Navigation bar", "bounds": [0, top, width, bottom], "owner": "com.android.systemui",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, top, width, bottom], "children": [
            button("Back", middle - 132.0), button("Home", middle), button("Overview", middle + 132.0)
        ]}
    })
}

/// The Android keyboard's window over the screen from 600 points down, as
/// `tests/visibility.rs` builds it.
fn keyboard() -> Value {
    json!({
        "id": 5, "kind": "input_method", "layer": 4, "package": "com.google.android.inputmethod.latin",
        "title": "Keyboard", "bounds": [0, 600, 408, 912], "owner": "com.google.android.inputmethod.latin",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 600, 408, 912]}
    })
}

/// A target showing `read` and a mirror over it that has read nothing yet,
/// with a frame source as on a Simulator or, as on an Android device, none.
fn attach(read: &TreeRead, frames: bool) -> (FakeTarget, Mirror) {
    let target = FakeTarget::new(read.clone());
    let backend = if frames {
        target.backend()
    } else {
        target.backend_without_frames()
    };
    let mirror = Mirror::new(backend, MirrorConfig::default());
    (target, mirror)
}

/// The same after the mirror's first read.
fn warm(read: &TreeRead, frames: bool) -> (FakeTarget, Mirror) {
    let (target, mut mirror) = attach(read, frames);
    mirror.pump().expect("the first read");
    (target, mirror)
}

/// Schedules `steps` at the target's current time and plays them: the
/// screen a `Show` replaces is dropped here, outside the timed region, and
/// the events wait in the stream for the next pump.
fn play(target: &FakeTarget, steps: Vec<Step>) {
    let now = target.elapsed();
    for step in steps {
        target.at(now, step);
    }
    target.advance(Duration::ZERO);
}

/// A window event for the application window, which makes a read of every
/// window due.
fn window_changed() -> Step {
    window_event(WindowChangeKind::STATE, APP)
}

/// The element with `identifier`, found as a wait finds it.
fn on_screen(mirror: &mut Mirror, identifier: &str) -> ElementId {
    mirror
        .wait(
            &Selector::identifier(identifier),
            Condition::Appear,
            Duration::ZERO,
        )
        .expect("on screen")
        .element
        .expect("the element")
        .id
}

/// Checks that the call made one read of every window, for `reason`, after
/// the first read.
fn assert_one_full_read(target: &FakeTarget, reason: ReadReason) {
    let reads = target.reads();
    assert_eq!(reads.len(), 2, "{reads:?}");
    assert_eq!(
        reads[1].request,
        ReadRequest {
            windows: None,
            reason
        }
    );
}

/// Calls `routine` once on `state`, verifies the result with `verify`, which
/// also gets the call's real time, and returns what the call allocated.
fn counted_call<S, O>(
    state: &mut S,
    routine: &impl Fn(&mut S) -> O,
    verify: &impl Fn(&mut S, O, Duration),
) -> alloc::Counts {
    let started = Instant::now();
    let (output, counts) = alloc::counted(|| routine(state));
    let real = started.elapsed();
    verify(state, output, real);
    counts
}

/// Checks that `calls` allocated the same, and prints for `micro` what one
/// of them allocated.
fn report_agreed(id: &str, calls: impl Iterator<Item = alloc::Counts>) {
    let counts: Vec<alloc::Counts> = calls.collect();
    let first = *counts.first().expect("at least one call");
    assert!(
        counts.iter().all(|&c| c == first),
        "the calls of {id} allocated differently: {counts:?}"
    );
    alloc::report(id, first);
}

/// Calls `routine` [`COUNTED_CALLS`] times, each on a state `setup` builds,
/// verifies each result and prints for `micro` what a call allocated.
fn count<S, O>(
    id: &str,
    setup: &impl Fn() -> S,
    routine: &impl Fn(&mut S) -> O,
    verify: &impl Fn(&mut S, O, Duration),
) {
    let calls = (0..COUNTED_CALLS).map(|_| counted_call(&mut setup(), routine, verify));
    report_agreed(id, calls);
}

/// Benchmarks `routine` on a state `setup` builds afresh for every
/// iteration, outside the timed region; the routine's output is dropped
/// inside it, rather than kept for criterion to drop outside. The inputs
/// hold a target and a whole replica, so they are built a few at a time
/// (`LargeInput`). Before the first sample, [`COUNTED_CALLS`] calls, each on
/// a state of its own, are verified and their allocations counted.
fn batched<S, O>(
    group: &mut Group<'_>,
    function: &str,
    parameter: &str,
    setup: impl Fn() -> S,
    routine: impl Fn(&mut S) -> O,
    verify: impl Fn(&mut S, O, Duration),
) {
    let id = format!("mirror/{function}/{parameter}");
    let mut checked = false;
    group.bench_function(BenchmarkId::new(function, parameter), |b| {
        if !checked {
            checked = true;
            count(&id, &setup, &routine, &verify);
        }
        b.iter_batched_ref(
            &setup,
            |state| {
                black_box(routine(state));
            },
            BatchSize::LargeInput,
        );
    });
}

/// Benchmarks `routine` on one state, built by `setup` before the first
/// sample, which every iteration leaves as it found it; the routine's
/// output is dropped inside the timed region. Before the first sample,
/// [`COUNTED_CALLS`] calls, each on a state of its own that `setup` builds
/// as it builds the timed one, are verified and their allocations counted.
fn reused<S, O>(
    group: &mut Group<'_>,
    function: &str,
    parameter: &str,
    setup: impl Fn() -> S,
    routine: impl Fn(&mut S) -> O,
    verify: impl Fn(&mut S, O, Duration),
) {
    let id = format!("mirror/{function}/{parameter}");
    let mut state = None;
    group.bench_function(BenchmarkId::new(function, parameter), |b| {
        let state = state.get_or_insert_with(|| {
            count(&id, &setup, &routine, &verify);
            setup()
        });
        b.iter(|| routine(state));
    });
}

/// Reads taken into the replica: the first, again unchanged, with one node
/// changed, with a window added or removed, and started over.
fn pumps(group: &mut Group<'_>, grid: &Fixture, list: &Fixture, grid_rn: &Fixture) {
    for f in [grid, list, grid_rn] {
        let taken = f.read.snapshot.windows.len() + f.read.snapshot.node_count as usize;
        // The cold acquisition: every node converted, hashed and given an
        // identity.
        batched(
            group,
            "pump",
            &format!("first-read-{}", f.name),
            || attach(&f.read, true),
            |(_, mirror)| mirror.pump(),
            |(target, mirror), result, _| {
                result.expect("the first read");
                assert_eq!(mirror.revision(), 1);
                let reads = target.reads();
                assert_eq!(reads.len(), 1);
                assert_eq!(reads[0].request.reason, ReadReason::Attach);
                assert_eq!(
                    mirror.dump().lines().count(),
                    taken,
                    "every window and node"
                );
            },
        );
    }
    for f in [grid, list, grid_rn] {
        // The screen shown again, unchanged: every node matched, no delta.
        batched(
            group,
            "pump",
            &format!("unchanged-{}", f.name),
            || {
                let (target, mirror) = warm(&f.read, true);
                play(&target, vec![Step::Show(f.read.clone()), window_changed()]);
                (target, mirror)
            },
            |(_, mirror)| mirror.pump(),
            |(target, mirror), result, _| {
                result.expect("the read");
                assert_eq!(mirror.revision(), 1, "nothing changed");
                assert!(mirror.deltas().is_empty(), "{:?}", mirror.deltas());
                assert_one_full_read(target, ReadReason::Events);
            },
        );
    }
    let changed = [
        (
            grid,
            grid.screen
                .patched(json!([{"find": "cell-250", "desc": "Cell 250 (done)"}])),
        ),
        (
            grid_rn,
            held_text(&grid_rn.screen, "cell-250", "250 (done)"),
        ),
    ];
    for (f, screen) in &changed {
        // One cell's text changed: one node of the application window.
        let read = screen.read();
        batched(
            group,
            "pump",
            &format!("one-change-{}", f.name),
            || {
                let (target, mirror) = warm(&f.read, true);
                play(&target, vec![Step::Show(read.clone()), window_changed()]);
                (target, mirror)
            },
            |(_, mirror)| mirror.pump(),
            |(target, mirror), result, _| {
                result.expect("the read");
                assert_eq!(mirror.revision(), 2);
                let [delta] = mirror.deltas() else {
                    panic!("one delta: {:?}", mirror.deltas());
                };
                let found = (delta.added.len(), delta.removed.len(), delta.changed.len());
                assert_eq!((delta.window, found), (APP, (0, 0, 1)));
                assert_one_full_read(target, ReadReason::Events);
            },
        );
    }
    for (f, added, removed) in [
        (list, "window-added-list", "window-removed-list"),
        (grid_rn, "window-added-grid-rn", "window-removed-grid-rn"),
    ] {
        // A system window added in front of the application, then removed:
        // its four nodes come or go, every other window matched unchanged.
        let with_bar = f
            .screen
            .patched(json!([{"add_window": navigation_bar(&f.screen)}]))
            .read();
        batched(
            group,
            "pump",
            added,
            || {
                let (target, mirror) = warm(&f.read, true);
                let event = window_event(WindowChangeKind::ADDED, NAVIGATION_BAR);
                play(&target, vec![Step::Show(with_bar.clone()), event]);
                (target, mirror)
            },
            |(_, mirror)| mirror.pump(),
            |(target, mirror), result, _| {
                result.expect("the read");
                assert_eq!(mirror.revision(), 2);
                let [delta] = mirror.deltas() else {
                    panic!("one delta: {:?}", mirror.deltas());
                };
                let found = (delta.added.len(), delta.removed.len(), delta.changed.len());
                assert_eq!((delta.window, found), (NAVIGATION_BAR, (4, 0, 0)));
                assert_one_full_read(target, ReadReason::Events);
            },
        );
        batched(
            group,
            "pump",
            removed,
            || {
                let (target, mirror) = warm(&with_bar, true);
                let event = window_event(WindowChangeKind::REMOVED, NAVIGATION_BAR);
                play(&target, vec![Step::Show(f.read.clone()), event]);
                (target, mirror)
            },
            |(_, mirror)| mirror.pump(),
            |(target, mirror), result, _| {
                result.expect("the read");
                assert_eq!(mirror.revision(), 2);
                let [delta] = mirror.deltas() else {
                    panic!("one delta: {:?}", mirror.deltas());
                };
                let found = (delta.added.len(), delta.removed.len(), delta.changed.len());
                assert_eq!((delta.window, found), (NAVIGATION_BAR, (0, 4, 0)));
                assert_one_full_read(target, ReadReason::Events);
            },
        );
    }
    for f in [grid, grid_rn] {
        // A new agent connection: the replica starts over from a full read
        // that carries no identity.
        batched(
            group,
            "pump",
            &format!("resync-{}", f.name),
            || {
                let (target, mirror) = warm(&f.read, true);
                play(&target, vec![Step::Item(StreamItem::Reconnected)]);
                (target, mirror)
            },
            |(_, mirror)| mirror.pump(),
            |(target, mirror), result, _| {
                result.expect("the read");
                assert_eq!(mirror.revision(), 2, "the replica started over");
                assert!(mirror.deltas().is_empty(), "a fresh read records none");
                assert_one_full_read(target, ReadReason::Resync);
            },
        );
    }
}

/// The interactive and full listings right after a read, which build the
/// view, the hints, the listing, the refs and the verdict, and the
/// interactive one again with nothing changed, every ref a hit in the table.
fn snapshots(group: &mut Group<'_>, fixtures: [&Fixture; 4]) {
    for interactive in [true, false] {
        let mode = if interactive { "interactive" } else { "full" };
        let options = SnapshotOptions {
            interactive,
            max_depth: None,
        };
        for f in fixtures {
            let expected = listed(f.name, interactive);
            batched(
                group,
                "snapshot",
                &format!("{mode}-{}", f.name),
                || warm(&f.read, true),
                |(_, mirror)| mirror.snapshot(&options),
                |(target, _), snapshot, _| {
                    let snapshot = snapshot.expect("the snapshot");
                    let found = (snapshot.elements.len(), snapshot.refs().len());
                    assert_eq!(found, expected, "{}", snapshot.text);
                    assert_eq!(snapshot.revision, 1);
                    assert_eq!(target.reads().len(), 1, "served from the replica");
                },
            );
        }
    }
    let interactive = SnapshotOptions::interactive();
    for f in fixtures {
        reused(
            group,
            "snapshot",
            &format!("interactive-again-{}", f.name),
            || {
                let (target, mut mirror) = warm(&f.read, true);
                let first = mirror.snapshot(&interactive).expect("the first snapshot");
                (target, mirror, first)
            },
            |(_, mirror, _)| mirror.snapshot(&interactive),
            |(target, _, first), again, _| {
                let again = again.expect("the snapshot");
                assert_eq!(again.text, first.text);
                assert_eq!(again.refs(), first.refs());
                assert!(again.removed_refs.is_empty());
                assert_eq!(target.reads().len(), 1, "served from the replica");
            },
        );
    }
}

/// What the replica tells without reading the target: how much of a row is
/// in view, the scroll container under a point and what of the screen the
/// windows above the app leave.
fn views(group: &mut Group<'_>, list: &Fixture) {
    reused(
        group,
        "visibility",
        "row-list",
        || {
            let (target, mut mirror) = warm(&list.read, true);
            let row = on_screen(&mut mirror, "row-3");
            (target, mirror, row)
        },
        |(_, mirror, row)| mirror.visibility(*row),
        |_, visibility, _| {
            let expected = Visibility {
                bounds: Rect::new(0.0, 168.0, 408.0, 216.0),
                viewport: Rect::new(0.0, 72.0, 408.0, 840.0),
                fraction: 1.0,
                cut: None,
                reachable: Some(Rect::new(0.0, 168.0, 408.0, 216.0)),
            };
            assert_eq!(visibility, Some(expected));
        },
    );
    reused(
        group,
        "scroll_viewport",
        "list",
        || warm(&list.read, true),
        |(_, mirror)| mirror.scroll_viewport(Point::new(204.0, 456.0), true),
        |_, viewport, _| assert_eq!(viewport, Some(Rect::new(0.0, 72.0, 408.0, 840.0))),
    );
    let typing = list
        .screen
        .patched(json!([{"add_window": keyboard()}]))
        .read();
    reused(
        group,
        "uncovered",
        "under-keyboard",
        || warm(&typing, true),
        |(_, mirror)| mirror.uncovered(Rect::new(0.0, 0.0, 408.0, 912.0), None),
        |_, uncovered, _| {
            let expected = Uncovered {
                area: Rect::new(0.0, 24.0, 408.0, 600.0),
                keyboard: true,
            };
            assert_eq!(uncovered, expected);
        },
    );
}

/// A delta snapshot and a structural diff after one row's text changed.
fn changes(group: &mut Group<'_>, list: &Fixture) {
    let changed = held_text(&list.screen, "row-5", "Row 5 (done)").read();
    let interactive = SnapshotOptions::interactive();
    batched(
        group,
        "delta_snapshot",
        "one-change-list",
        || {
            let (target, mut mirror) = attach(&list.read, true);
            mirror
                .delta_snapshot(&interactive, false)
                .expect("the first delta snapshot");
            play(&target, vec![Step::Show(changed.clone()), window_changed()]);
            (target, mirror)
        },
        |(_, mirror)| mirror.delta_snapshot(&interactive, false),
        |_, delta, _| {
            let expected = DeltaSnapshot::Delta {
                revision: 2,
                base_revision: 1,
                changes: vec![RefChange::Replace {
                    reference: Ref(5),
                    field: RefField::Name,
                    value: "Row 5 (done)".to_owned(),
                }],
                tree_change: TreeSplice {
                    start_line: 4,
                    delete_count: 1,
                    lines: vec!["- button \"Row 5 (done)\" [id=row-5, ref=e5]".to_owned()],
                },
            };
            assert_eq!(delta.expect("the delta snapshot"), expected);
        },
    );
    batched(
        group,
        "diff",
        "one-change-list",
        || {
            let (target, mut mirror) = attach(&list.read, true);
            let baseline = mirror.snapshot(&interactive).expect("the baseline");
            play(&target, vec![Step::Show(changed.clone()), window_changed()]);
            (target, mirror, baseline)
        },
        |(_, mirror, baseline)| mirror.diff(baseline),
        |_, diff, _| {
            let diff = diff.expect("the diff");
            assert!(
                diff.added.is_empty() && diff.removed.is_empty(),
                "{}",
                diff.text()
            );
            let [change] = diff.changed.as_slice() else {
                panic!("one change: {}", diff.text());
            };
            assert_eq!(change.after.identifier.as_deref(), Some("row-5"));
            assert_eq!(change.fields, [Field::Name]);
            assert_eq!((diff.baseline_revision, diff.revision), (1, 2));
        },
    );
}

/// A wait that holds at once, one evaluation, and one over the typing
/// timeline that resolves at its last event, one evaluation per item.
fn waits(group: &mut Group<'_>, list: &Fixture) {
    let timeout = Duration::from_secs(5);
    reused(
        group,
        "wait",
        "holds-list",
        || (warm(&list.read, true), Selector::text("Row 16")),
        |((_, mirror), selector)| mirror.wait(selector, Condition::Appear, timeout),
        |((target, _), _), outcome, real| {
            let outcome = outcome.expect("the wait holds");
            assert_eq!(outcome.elapsed, Duration::ZERO);
            let row = outcome.element.expect("the row");
            assert_eq!(row.identifier.as_deref(), Some("row-16"));
            assert_eq!(target.reads().len(), 1, "nothing read");
            assert!(real < REAL_TIME_BOUND, "{real:?} of real time");
        },
    );
    let typing = timeline("events-typing");
    let welcome = Selector::text("Welcome");
    batched(
        group,
        "wait",
        "events-typing",
        || {
            let target = typing.target();
            let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
            mirror.pump().expect("the first read");
            (target, mirror)
        },
        |(_, mirror)| mirror.wait(&welcome, Condition::Appear, timeout),
        |(target, _), outcome, real| {
            let outcome = outcome.expect("the wait resolves");
            let status = outcome.element.expect("the status");
            assert_eq!(status.identifier.as_deref(), Some("login-status"));
            assert_eq!(status.name, "Welcome, alice");
            // From the end of the first read, 3 ms in, to the end of the
            // read the last event brings, 643 ms in.
            assert_eq!(outcome.elapsed, Duration::from_millis(640));
            assert_eq!(target.reads().len(), 8, "the first read and one per event");
            assert!(real < REAL_TIME_BOUND, "{real:?} of real time");
        },
    );
}

/// A change arrives and the screen stays still: the settle takes the change
/// in and waits one quiet window on the fake clock before it settles. The
/// change is a tree event, the cheapest to script; the settle evaluates the
/// same whichever signal brought it.
fn settle_after_a_change(
    (target, mirror): &mut (FakeTarget, Mirror),
) -> Result<Settle, MirrorError> {
    let change = TreeChange {
        change: TreeChangeKind::CONTENT,
        window: APP,
        path: Vec::new(),
    };
    target.after(Duration::ZERO, Step::Event(EventPayload::Tree(change)));
    mirror.settle(&SettleOptions::default())
}

/// Settles of a quiet screen with frames, as on a Simulator, and without, as
/// on an Android device, where the windows that send no change events, such
/// as `grid`'s, are re-read every minimum re-read interval while waiting.
/// The fake target answers no bounds probes, so none is made.
fn settles(group: &mut Group<'_>, fixtures: [&Fixture; 2]) {
    for f in fixtures {
        for frames in [true, false] {
            let kind = if frames { "frames" } else { "no-frames" };
            let rereads = usize::from(!frames && f.rereads_without_frames());
            reused(
                group,
                "settle",
                &format!("{kind}-{}", f.name),
                || {
                    // One settle before those counted or timed, so that the
                    // target's queues hold what every later one finds.
                    let mut state = warm(&f.read, frames);
                    settle_after_a_change(&mut state).expect("the first settle");
                    let reads = state.0.reads().len();
                    (state, reads)
                },
                |(state, _)| settle_after_a_change(state),
                |((target, _), reads), settle, real| {
                    let settle = settle.expect("the settle");
                    assert!(settle.settled, "{settle:?}");
                    assert_eq!(settle.elapsed, DEFAULT_QUIET_WINDOW);
                    assert_eq!((settle.tree_changes, settle.frame_changes), (1, 0));
                    assert_eq!(target.reads().len(), *reads + rereads);
                    *reads = target.reads().len();
                    assert!(real < REAL_TIME_BOUND, "{real:?} of real time");
                },
            );
        }
    }
}

fn benches(c: &mut Criterion) {
    let grid = Fixture::load("grid");
    let list = Fixture::load("list");
    let bluesky = Fixture::load("bluesky-typeahead-android");
    let grid_rn = Fixture::load("grid-rn");
    let mut group = c.benchmark_group("mirror");
    pumps(&mut group, &grid, &list, &grid_rn);
    snapshots(&mut group, [&grid, &list, &bluesky, &grid_rn]);
    views(&mut group, &list);
    changes(&mut group, &list);
    waits(&mut group, &list);
    settles(&mut group, [&grid, &grid_rn]);
    group.finish();
}

criterion_group!(mirror, benches);
criterion_main!(mirror);
