//! Task 7.4: the settle predicate. Settled means no tree change and no frame
//! hash change for the quiet window, 200 ms by default (spike S1), bounded by
//! a maximum, 3 s by default, after which the result names what kept
//! changing. Frame callbacks that change no pixel do not count (spike S2).
//! Windows that send no change events get their tree signal from re-reads
//! when there are no frames to watch, and nodes that move without a change
//! event get theirs from bounds probes (task 10.11), while a window the
//! platform animates in holds the wait as long as it is flagged
//! transformed, and a stroke that leaves a list at an end of its range
//! holds it for the overscroll effect the platform draws there (task
//! 10.14).

mod support;

use std::time::Duration;

use mirror::backend::{Input, NodeHandle, ReadReason, Release, TreeRead};
use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, DEFAULT_FIRST_FRAME_WAIT, DEFAULT_OVERSCROLL, DEFAULT_PROBE_ANCHORS,
    DEFAULT_PROBE_INTERVAL, DEFAULT_QUIET_WINDOW, DEFAULT_SETTLE_MAX, Mirror, MirrorConfig, Point,
    Rect, SettleOptions, Signal, Signals, SlowFrames, SnapshotOptions, Target,
};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind, WindowChange, WindowChangeKind};
use protocol::snapshot::{Bounds, Node, WindowFlags};
use serde_json::json;
use support::{Screen, ms};

fn content_changed(window: u32) -> Step {
    Step::Event(EventPayload::Tree(TreeChange {
        change: TreeChangeKind::CONTENT,
        window,
        path: vec![0],
    }))
}

fn tap_on(mirror: &mut Mirror, identifier: &str) {
    let snapshot = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    let reference = snapshot
        .by_identifier(identifier)
        .unwrap()
        .reference
        .unwrap();
    mirror
        .act(&Action::Tap {
            target: Target::Ref(reference),
            hold: None,
        })
        .unwrap();
}

fn quiet(ms_: u64) -> SettleOptions {
    SettleOptions {
        quiet: ms(ms_),
        ..SettleOptions::default()
    }
}

#[test]
fn the_quiet_window_defaults_to_200_ms_and_the_maximum_to_3_s() {
    assert_eq!(DEFAULT_QUIET_WINDOW, ms(200));
    assert_eq!(DEFAULT_SETTLE_MAX, Duration::from_secs(3));
    let defaults = SettleOptions::default();
    assert_eq!(defaults.quiet, ms(200));
    assert_eq!(defaults.max, Duration::from_secs(3));
    assert_eq!(defaults.signals, Signals::TreeAndFrames);

    let target = FakeTarget::new(Screen::load("login").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    tap_on(&mut mirror, "tab-list");
    let settle = mirror.settle(&defaults).unwrap();
    assert!(settle.settled);
    assert_eq!(
        settle.elapsed,
        ms(200),
        "a tap that changes nothing settles one default window later"
    );
}

#[test]
fn a_transition_settles_shortly_after_it_ends_and_reports_the_elapsed_time() {
    let screen = Screen::load("login");
    let welcome = screen.patched(json!([{"find": "login-status", "text": "Welcome, alice"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(0), Step::Frame(0));
    let mut steps: Vec<(Duration, Step)> = (1..=25).map(|k| (ms(16 * k), Step::Frame(k))).collect();
    for t in [100, 200, 300, 400] {
        steps.push((ms(t), content_changed(1)));
    }
    steps.push((ms(400), Step::Show(welcome.read())));
    target.on_tap("login-submit", steps);
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    tap_on(&mut mirror, "login-submit");

    let settle = mirror.settle(&quiet(150)).unwrap();
    assert!(settle.settled);
    assert_eq!(
        settle.elapsed,
        ms(550),
        "the transition ends at 400 ms, the quiet window closes 150 ms later"
    );
    assert!(settle.still_changing.is_empty());
    assert_eq!(settle.reason(), None);
    assert_eq!(settle.frame_changes, 25);
    assert_eq!(settle.tree_changes, 4);
    let snapshot = mirror.snapshot(&SnapshotOptions::full()).unwrap();
    assert_eq!(
        snapshot.by_identifier("login-status").unwrap().name,
        "Welcome, alice"
    );
}

#[test]
fn a_continuous_animation_is_not_settled_after_the_maximum_and_names_frame_changes() {
    let target = FakeTarget::new(Screen::load("compose-anim").read());
    for k in 0..=250 {
        target.at(ms(16 * k), Step::Frame(k % 2));
    }
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(100));
    mirror.pump().unwrap();

    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(!settle.settled, "not an error: a result that says why");
    assert_eq!(settle.elapsed, Duration::from_secs(3));
    assert_eq!(settle.still_changing, [Signal::Frames]);
    assert_eq!(settle.reason().as_deref(), Some("frame changes"));
    assert_eq!(
        settle.tree_changes, 0,
        "Compose's spinner sends no tree events"
    );
    assert_eq!(settle.frame_changes, 187);
}

#[test]
fn an_animation_that_also_sends_tree_events_names_both_signals() {
    let target = FakeTarget::new(Screen::load("login").read());
    for k in 0..=250 {
        target.at(ms(16 * k), Step::Frame(k % 2));
    }
    for k in 1..=40 {
        target.at(ms(101 * k), content_changed(1));
    }
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(100));
    mirror.pump().unwrap();

    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(!settle.settled);
    assert_eq!(settle.elapsed, Duration::from_secs(3));
    assert_eq!(settle.still_changing, [Signal::Tree, Signal::Frames]);
    assert_eq!(
        settle.reason().as_deref(),
        Some("tree changes and frame changes")
    );
}

#[test]
fn frame_callbacks_without_pixel_changes_do_not_hold_the_wait() {
    let target = FakeTarget::new(Screen::load("login").read());
    for k in 0..=40 {
        target.at(ms(16 * k), Step::Frame(7));
    }
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    tap_on(&mut mirror, "tab-list");

    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(settle.elapsed, ms(200), "one quiet window after the tap");
    assert_eq!(settle.frame_changes, 0);
}

#[test]
fn a_screen_that_has_been_quiet_settles_at_once() {
    let target = FakeTarget::new(Screen::load("login").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(1_000));
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(settle.elapsed, ms(0));
}

#[test]
fn the_default_window_does_not_end_inside_a_scroll_while_150_ms_would() {
    let events = [0, 110, 226, 413, 523, 633];
    let scroll = || {
        let target = FakeTarget::new(Screen::load("list").read());
        for t in events {
            target.at(ms(t), content_changed(1));
        }
        target
    };
    let target = scroll();
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(settle.elapsed, ms(833), "200 ms after the last event");
    assert_eq!(settle.tree_changes, 6);

    let target = scroll();
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let early = mirror.settle(&quiet(150)).unwrap();
    assert_eq!(
        early.elapsed,
        ms(376),
        "the 187 ms gap inside the scroll outlasts a 150 ms window"
    );
}

#[test]
fn tree_only_settling_ignores_frames() {
    let target = FakeTarget::new(Screen::load("compose-anim").read());
    for k in 0..=250 {
        target.at(ms(16 * k), Step::Frame(k % 2));
    }
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(100));
    mirror.pump().unwrap();
    let settle = mirror
        .settle(&SettleOptions {
            signals: Signals::TreeOnly,
            ..SettleOptions::default()
        })
        .unwrap();
    assert!(settle.settled, "frames keep changing, the tree does not");
    assert_eq!(settle.elapsed, ms(0));
}

#[test]
fn without_frames_an_event_less_window_is_re_read_while_waiting() {
    let screen = Screen::load("compose-anim");
    let off = screen.patched(json!([{"find": "anim-toggle", "off": ["checked"]}]));
    let target = FakeTarget::new(screen.read());
    target.read_cost(ms(10));
    target.on_tap("anim-toggle", vec![(ms(150), Step::Show(off.read()))]);
    let mut mirror = Mirror::new(target.backend_without_frames(), MirrorConfig::default());
    tap_on(&mut mirror, "anim-toggle");
    let tapped = target.elapsed();

    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(
        settle.tree_changes, 1,
        "the re-read found the toggle's change"
    );
    let reads: Vec<_> = target
        .reads()
        .iter()
        .skip(1)
        .map(|r| (r.at - tapped, r.request.reason))
        .collect();
    assert_eq!(
        reads,
        [
            (ms(0), ReadReason::Action),
            (ms(200), ReadReason::Poll),
            (ms(400), ReadReason::Poll),
        ],
        "re-read every minimum interval while waiting"
    );
    assert_eq!(
        settle.elapsed,
        ms(410),
        "a quiet window after the read that found the change"
    );
}

#[test]
fn a_launch_noted_to_the_mirror_holds_settle_until_the_app_has_drawn() {
    let target = FakeTarget::new(Screen::load("login-ios").read());
    target.at(ms(0), Step::Frame(0));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(Duration::from_secs(2));

    // The app is launched outside the mirror's input; its first frames come
    // after the wait begins.
    mirror.note_window_change();
    for k in 1..=10 {
        target.after(ms(60 + 30 * k), Step::Frame(k));
    }
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(
        settle.elapsed,
        ms(560),
        "the last frame 360 ms after the launch, then one quiet window"
    );
    assert_eq!(settle.frame_changes, 10);
}

#[test]
fn a_launch_counts_as_a_change_whether_noted_or_reported_as_a_window_event() {
    let target = FakeTarget::new(Screen::load("login").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(Duration::from_secs(1));
    mirror.note_window_change();
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(
        settle.elapsed,
        ms(200),
        "a launch that changes nothing else holds one quiet window"
    );
    mirror.snapshot(&SnapshotOptions::full()).unwrap();
    let read = target.reads().last().unwrap().request.clone();
    assert_eq!(
        (read.windows, read.reason),
        (None, ReadReason::Events),
        "the next read covers every window, as after a window event"
    );

    // The Simulator driver reports a launch as a window event it queues
    // before the wait begins.
    let target = FakeTarget::new(Screen::load("login-ios").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(Duration::from_secs(1));
    target.after(
        ms(0),
        Step::Event(EventPayload::Window(WindowChange {
            change: WindowChangeKind::STATE,
            window: 1,
        })),
    );
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(settle.elapsed, ms(200));
    assert_eq!(settle.tree_changes, 1);
}

/// `read` with the nodes of window `window` moved `points` device points
/// to the right, as a screen sliding in from the right shows them midway.
fn shifted(read: &TreeRead, window: u32, points: f64) -> TreeRead {
    fn shift(node: &mut Node, dx: i32) {
        node.bounds.left += dx;
        node.bounds.right += dx;
        for child in &mut node.children {
            shift(child, dx);
        }
    }
    let dx = (points * read.scale).round() as i32;
    let mut read = read.clone();
    for w in &mut read.snapshot.windows {
        if w.id == window
            && let Some(root) = &mut w.root
        {
            shift(root, dx);
        }
    }
    read
}

/// Where the replica has the element `identifier`.
fn rect_of(mirror: &mut Mirror, identifier: &str) -> Rect {
    mirror
        .snapshot(&SnapshotOptions::full())
        .unwrap()
        .by_identifier(identifier)
        .unwrap_or_else(|| panic!("no {identifier}"))
        .bounds
}

/// The Expo login screen, 300 points right of where it comes to rest, on a
/// target without frames that answers probes: the replica read it there as
/// the app began to slide it in by moving its views, and from 10 ms on it
/// moves 37.5 points every 50 ms for 400 ms with no change event, while
/// reads keep returning what they returned before, as the platform's node
/// cache does.
fn sliding_in() -> (FakeTarget, Mirror, TreeRead) {
    let rest = Screen::load("login").read();
    let target = FakeTarget::new(shifted(&rest, 1, 300.0));
    target.read_cost(ms(10));
    target.probe_cost(ms(2));
    target.answer_probes(true);
    let mut mirror = Mirror::new(target.backend_without_frames(), MirrorConfig::default());
    mirror.pump().unwrap();
    for k in 1..=8u32 {
        let points = 300.0 * f64::from(8 - k) / 8.0;
        target.at(
            ms(10 + 50 * u64::from(k)),
            Step::Move(shifted(&rest, 1, points)),
        );
    }
    (target, mirror, rest)
}

#[test]
fn the_probe_defaults_are_four_anchors_every_50_ms() {
    assert_eq!(DEFAULT_PROBE_INTERVAL, ms(50));
    assert_eq!(DEFAULT_PROBE_ANCHORS, 4);
    let defaults = MirrorConfig::default();
    assert_eq!(defaults.probe_interval, ms(50));
    assert_eq!(defaults.probe_anchors, 4);
}

/// Task 10.11: without frames, a screen an app slides in by moving its
/// views without a change event changes no node the tree reports, so the
/// quiet window could pass 200 ms after the slide began with the replica
/// holding the positions of mid-slide. Probes of anchor nodes see the
/// slide; once the anchors hold still where the replica does not have them,
/// their window is read with its cache cleared, and the wait settles a
/// quiet window after that read, the replica holding where the screen came
/// to rest.
#[test]
fn without_frames_settle_waits_until_probed_bounds_hold_still_and_the_replica_has_them() {
    let (target, mut mirror, rest) = sliding_in();
    let launched = target.elapsed();
    mirror.note_window_change();

    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    let slide_ends = ms(410) - launched;
    assert!(
        settle.elapsed >= slide_ends + DEFAULT_QUIET_WINDOW,
        "settled {:?} after the launch, before the slide ended {slide_ends:?} after it and \
         a quiet window passed",
        settle.elapsed
    );
    assert!(
        settle.elapsed <= slide_ends + DEFAULT_QUIET_WINDOW + ms(150),
        "a probe interval to see it hold still, a read and a quiet window: {:?}",
        settle.elapsed
    );
    assert!(
        settle.tree_changes >= 8,
        "each probe that saw the screen move is a tree change: {settle:?}"
    );

    let moved: Vec<_> = target
        .reads()
        .into_iter()
        .filter(|r| r.request.reason == ReadReason::Moved)
        .collect();
    assert_eq!(
        moved.len(),
        1,
        "one read once the slide held still: {moved:?}"
    );
    assert_eq!(moved[0].request.windows, Some(vec![1]), "the app's window");
    assert!(moved[0].at >= ms(410), "{moved:?}");

    let probes = target.probes();
    assert!(probes.len() >= 12, "a probe every 50 ms: {probes:?}");
    for pair in probes.windows(2) {
        assert!(
            pair[1].at - pair[0].at >= DEFAULT_PROBE_INTERVAL,
            "no more often than the interval: {probes:?}"
        );
    }
    assert!(
        probes
            .iter()
            .all(|p| !p.nodes.is_empty() && p.nodes.len() <= DEFAULT_PROBE_ANCHORS),
        "{probes:?}"
    );
    assert!(
        probes
            .iter()
            .all(|p| p.nodes.iter().all(|n: &NodeHandle| n.window == 1)),
        "anchors in the app's window, not the status bar: {probes:?}"
    );
    assert!(
        settle.elapsed < ms(1_000),
        "not the maximum: {:?}",
        settle.elapsed
    );

    let mut at_rest = Mirror::new(FakeTarget::new(rest).backend(), MirrorConfig::default());
    assert_eq!(
        rect_of(&mut mirror, "tab-big"),
        rect_of(&mut at_rest, "tab-big"),
        "the replica holds the tab where the screen came to rest"
    );
}

/// Task 10.11: a still screen whose replica is current settles as it did,
/// after one probe that finds every anchor where the replica has it.
#[test]
fn without_frames_a_still_screen_settles_after_one_agreeing_probe() {
    let target = FakeTarget::new(Screen::load("login").read());
    target.probe_cost(ms(2));
    target.answer_probes(true);
    let mut mirror = Mirror::new(target.backend_without_frames(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(1_000));
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(settle.elapsed, ms(2), "the probe's own cost");
    assert_eq!(target.probes().len(), 1);
    assert_eq!(settle.tree_changes, 0);
    assert!(
        target
            .reads()
            .iter()
            .all(|r| r.request.reason != ReadReason::Moved),
        "{:?}",
        target.reads()
    );
}

/// Task 10.11: a replica that went stale before the wait began, the screen
/// having come to rest meanwhile without an event, is read again: the first
/// probe finds the anchors elsewhere, the next finds them there still.
#[test]
fn without_frames_a_replica_stale_since_before_the_wait_is_read_again() {
    let rest = Screen::load("login").read();
    let target = FakeTarget::new(shifted(&rest, 1, 120.0));
    target.read_cost(ms(10));
    target.probe_cost(ms(2));
    target.answer_probes(true);
    let mut mirror = Mirror::new(target.backend_without_frames(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.at(ms(20), Step::Move(rest.clone()));
    target.advance(ms(1_000));

    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    let reads: Vec<ReadReason> = target
        .reads()
        .iter()
        .skip(1)
        .map(|r| r.request.reason)
        .collect();
    assert_eq!(reads, [ReadReason::Moved], "one read, of the moved window");
    assert!(
        settle.elapsed >= DEFAULT_PROBE_INTERVAL + DEFAULT_QUIET_WINDOW,
        "{:?}",
        settle.elapsed
    );
    let mut at_rest = Mirror::new(FakeTarget::new(rest).backend(), MirrorConfig::default());
    assert_eq!(
        rect_of(&mut mirror, "tab-big"),
        rect_of(&mut at_rest, "tab-big")
    );
}

/// The frame-source path is unchanged: with frames the frames carry the
/// settle and nothing is probed, even from a source that could.
#[test]
fn with_frames_settle_probes_nothing() {
    let rest = Screen::load("login").read();
    let target = FakeTarget::new(shifted(&rest, 1, 300.0));
    target.answer_probes(true);
    target.at(ms(0), Step::Frame(0));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    mirror.note_window_change();
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(settle.elapsed, ms(200));
    assert!(target.probes().is_empty(), "{:?}", target.probes());
}

/// A tree source that cannot read bounds again leaves the wait as it was:
/// the quiet window after the last change the tree reports.
#[test]
fn without_frames_a_source_that_cannot_probe_settles_on_the_tree_alone() {
    let (target, mut mirror, _) = sliding_in();
    target.answer_probes(false);
    mirror.note_window_change();
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(settle.elapsed, ms(200));
    assert!(target.probes().is_empty());
}

/// `read` with window `window` drawn at `factor` of its size about its
/// center and flagged transformed, as the platform reports a window it
/// scales in on Android 14 and later: the window's bounds and its nodes'
/// are those of one frame of the animation.
fn scaled(read: &TreeRead, window: u32, factor: f64) -> TreeRead {
    fn scale(bounds: &mut Bounds, center: (f64, f64), factor: f64) {
        let map = |v: i32, c: f64| (c + (f64::from(v) - c) * factor).round() as i32;
        *bounds = Bounds::new(
            map(bounds.left, center.0),
            map(bounds.top, center.1),
            map(bounds.right, center.0),
            map(bounds.bottom, center.1),
        );
    }
    fn walk(node: &mut Node, center: (f64, f64), factor: f64) {
        scale(&mut node.bounds, center, factor);
        for child in &mut node.children {
            walk(child, center, factor);
        }
    }
    let mut read = read.clone();
    for w in &mut read.snapshot.windows {
        if w.id == window {
            let b = w.bounds;
            let center = (
                f64::from(b.left + b.right) / 2.0,
                f64::from(b.top + b.bottom) / 2.0,
            );
            scale(&mut w.bounds, center, factor);
            w.flags |= WindowFlags::TRANSFORMED;
            if let Some(root) = &mut w.root {
                walk(root, center, factor);
            }
        }
    }
    read
}

/// A target without frames that answers probes, showing `read` since the
/// launch.
fn launched_showing(read: TreeRead) -> (FakeTarget, Mirror) {
    let target = FakeTarget::new(read);
    target.read_cost(ms(10));
    target.probe_cost(ms(2));
    target.answer_probes(true);
    let mut mirror = Mirror::new(target.backend_without_frames(), MirrorConfig::default());
    mirror.pump().unwrap();
    (target, mirror)
}

/// Task 10.11: on Android 14 and later the platform reports a window it
/// animates in at one frame of that animation, the same to every read and
/// probe, until the animation ends and the window's bounds change at once:
/// on the API 36 emulator, about 300 ms of the Expo app's launch scaled to
/// 95% over the launcher, or 500 ms slid partly off the display over
/// another app, with neither an event nor a move, longer than the quiet
/// window. The window's transformed flag holds the wait until a read has
/// the window at rest, and the wait settles a quiet window after that read.
#[test]
fn without_frames_settle_waits_while_a_window_is_drawn_transformed() {
    let rest = Screen::load("login").read();
    let (target, mut mirror) = launched_showing(scaled(&rest, 1, 0.95));
    target.at(ms(300), Step::Show(rest.clone()));
    target.at(
        ms(300),
        Step::Event(EventPayload::Window(WindowChange {
            change: WindowChangeKind::BOUNDS,
            window: 1,
        })),
    );
    let launched = target.elapsed();
    mirror.note_window_change();

    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    let at_rest_after = ms(300) - launched;
    assert!(
        settle.elapsed >= at_rest_after + DEFAULT_QUIET_WINDOW,
        "settled {:?} after the launch, while the window was drawn transformed or less than \
         a quiet window after it came to rest {at_rest_after:?} after the launch",
        settle.elapsed
    );
    assert!(
        settle.elapsed <= at_rest_after + DEFAULT_QUIET_WINDOW + ms(100),
        "a read and a quiet window: {:?}",
        settle.elapsed
    );
    let mut at_rest = Mirror::new(FakeTarget::new(rest).backend(), MirrorConfig::default());
    assert_eq!(
        rect_of(&mut mirror, "tab-big"),
        rect_of(&mut at_rest, "tab-big"),
        "the replica holds the tab where the window came to rest"
    );
}

/// A window that stays drawn transformed, as an app the platform scales to
/// fit the screen is, holds the wait to the maximum, which then names the
/// tree.
#[test]
fn without_frames_a_window_that_stays_transformed_holds_the_wait_to_the_maximum() {
    let (target, mut mirror) = launched_showing(scaled(&Screen::load("login").read(), 1, 0.95));
    target.advance(ms(1_000));
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(!settle.settled, "{settle:?}");
    assert!(settle.elapsed >= DEFAULT_SETTLE_MAX, "{settle:?}");
    assert_eq!(settle.still_changing, vec![Signal::Tree], "{settle:?}");
}

/// A picture-in-picture window is drawn scaled at rest, so flagged
/// transformed; it does not hold the wait.
#[test]
fn without_frames_a_picture_in_picture_window_does_not_hold_the_wait() {
    let mut read = scaled(&Screen::load("login").read(), 1, 0.4);
    for w in &mut read.snapshot.windows {
        if w.id == 1 {
            w.flags |= WindowFlags::PICTURE_IN_PICTURE;
        }
    }
    let (target, mut mirror) = launched_showing(read);
    target.advance(ms(1_000));
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    assert_eq!(settle.elapsed, ms(2), "the probe's own cost");
}

/// The frame-source path is unchanged: with frames, a window drawn
/// transformed leaves the wait to the frames.
#[test]
fn with_frames_a_window_drawn_transformed_does_not_hold_the_wait() {
    let target = FakeTarget::new(scaled(&Screen::load("login").read(), 1, 0.95));
    target.at(ms(0), Step::Frame(0));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(1_000));
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    assert!(settle.elapsed < DEFAULT_QUIET_WINDOW, "{settle:?}");
}

/// The Expo fixture's list on Android, its scroll view able to scroll back
/// (up) when `back` and forward (down) when `forward`, with everything
/// inside it moved up `points` device points, as scrolling the list that
/// far down shows it.
fn list(back: bool, forward: bool, points: f64) -> TreeRead {
    fn lift(node: &mut Node, dy: i32) {
        node.bounds.top -= dy;
        node.bounds.bottom -= dy;
        for child in &mut node.children {
            lift(child, dy);
        }
    }
    fn scroll_view(node: &mut Node) -> Option<&mut Node> {
        if node.res_id.as_deref() == Some("list-scroll") {
            return Some(node);
        }
        node.children.iter_mut().find_map(scroll_view)
    }
    let screen = Screen::load("list").patched(json!([{
        "find": "list-scroll",
        "scroll": {"axis": "vertical", "back": back, "forward": forward}
    }]));
    let mut read = screen.read();
    let dy = (points * read.scale).round() as i32;
    for window in &mut read.snapshot.windows {
        if let Some(view) = window.root.as_mut().and_then(scroll_view) {
            for child in &mut view.children {
                lift(child, dy);
            }
        }
    }
    read
}

/// The scroll event the list's scroll view sends after it scrolled.
fn list_scrolled() -> Step {
    support::tree_event(
        &Screen::load("list"),
        1,
        TreeChangeKind::SCROLL,
        "list-scroll",
    )
}

/// A target without frames that answers probes, showing `read`, and a
/// mirror that has read it a second ago, as the agent path has a list
/// between strokes.
fn list_on_the_agent_path(read: TreeRead) -> (FakeTarget, Mirror) {
    let target = FakeTarget::new(read);
    target.read_cost(ms(10));
    target.probe_cost(ms(2));
    target.answer_probes(true);
    let mut mirror = Mirror::new(target.backend_without_frames(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(1_000));
    (target, mirror)
}

/// A 300 ms stroke from `from` to `to`, acknowledged once the finger has
/// lifted, as the Android agent's swipes are; returns when the platform
/// acknowledged it, from the start of the script.
fn stroke(target: &FakeTarget, mirror: &mut Mirror, from: Point, to: Point) -> Duration {
    target.input_cost(ms(300));
    mirror
        .act(&Action::Swipe {
            from,
            to,
            duration: ms(300),
            release: Release::Moving,
        })
        .unwrap();
    target.input_cost(Duration::ZERO);
    target.elapsed()
}

/// Task 10.16: a stroke reaches the target with the release the action
/// gives it, so a driver can let its finger come to rest before it lifts.
#[test]
fn a_stroke_reaches_the_target_released_as_the_action_says() {
    let (target, mut mirror) = list_on_the_agent_path(list(false, true, 0.0));
    for release in [Release::Moving, Release::Resting] {
        mirror
            .act(&Action::Swipe {
                from: Point::new(204.0, 700.0),
                to: Point::new(204.0, 100.0),
                duration: ms(300),
                release,
            })
            .unwrap();
        let last = target.inputs().pop().expect("the stroke");
        assert!(
            matches!(last.input, Input::Swipe { release: r, .. } if r == release),
            "{last:?}"
        );
    }
}

/// A stroke up the middle of the list, which scrolls it down, as the
/// daemon's `scroll down 600` makes one.
fn scroll_down(target: &FakeTarget, mirror: &mut Mirror) -> Duration {
    stroke(
        target,
        mirror,
        Point::new(204.0, 700.0),
        Point::new(204.0, 100.0),
    )
}

/// Task 10.14: from Android 12 a list that a stroke pushes against its end
/// draws a stretch that springs back after the finger lifts, for 333 to
/// 383 ms on the API 36 emulator's Expo fixture. The stretch moves no view
/// and sends no event, and until it rests
/// the list takes a touch that comes down as the start of a drag that
/// catches it, so a tap on a row opens nothing: the harness's list-scroll
/// flow on the agent path clicked `row-200` right after the settle that
/// followed its last stroke, and the row never opened
/// (`reports/10.11-agent-path/report.md`). A stroke that pulls on a list
/// already at its end moves nothing, and without frames nothing shows the
/// stretch; the wait holds for the overscroll hold after the stroke.
#[test]
fn without_frames_a_stroke_that_pulls_a_list_at_its_end_holds_the_wait_for_the_overscroll() {
    assert_eq!(DEFAULT_OVERSCROLL, ms(500));
    assert_eq!(MirrorConfig::default().overscroll, DEFAULT_OVERSCROLL);
    let (target, mut mirror) = list_on_the_agent_path(list(true, false, 0.0));
    scroll_down(&target, &mut mirror);
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    assert!(
        settle.elapsed >= DEFAULT_OVERSCROLL,
        "settled {:?} after the stroke lifted, while the stretch it pulled on the list's end \
         could still be springing back",
        settle.elapsed
    );
    assert!(
        settle.elapsed <= DEFAULT_OVERSCROLL + DEFAULT_PROBE_INTERVAL,
        "the hold, then no longer: {settle:?}"
    );
    assert_eq!(
        settle.tree_changes, 0,
        "nothing the tree reports: {settle:?}"
    );
}

/// Task 10.14: the last stroke of a scroll down a long list carries it the
/// last points to its end and pulls on the end for the rest: the probes see
/// the rows move and read the list again, at its end, and the wait holds
/// for the overscroll hold after that read.
#[test]
fn without_frames_a_stroke_that_leaves_a_list_at_its_end_holds_the_wait_for_the_overscroll() {
    let (target, mut mirror) = list_on_the_agent_path(list(true, true, 0.0));
    // The stroke carries the list the last 50 points to its end in its
    // first 30 ms; the scroll view reports the scroll 100 ms later.
    target.after(ms(30), Step::Show(list(true, false, 50.0)));
    target.after(ms(130), list_scrolled());
    let released = scroll_down(&target, &mut mirror);
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    let moved: Vec<Duration> = target
        .reads()
        .into_iter()
        .filter(|r| r.request.reason == ReadReason::Moved)
        .map(|r| r.at)
        .collect();
    assert_eq!(moved.len(), 1, "one read of the list at its end: {moved:?}");
    let read_end = moved[0] + ms(10) - released;
    assert!(
        settle.elapsed >= read_end + DEFAULT_OVERSCROLL,
        "settled {:?} after the stroke lifted, the list read at its end {read_end:?} after it, \
         while its stretch could still be springing back",
        settle.elapsed
    );
    assert!(
        settle.elapsed <= read_end + DEFAULT_OVERSCROLL + DEFAULT_PROBE_INTERVAL,
        "the hold after that read, then no longer: {settle:?}"
    );
    let rows = mirror.snapshot(&SnapshotOptions::full()).unwrap();
    assert!(
        rows.by_identifier("row-1")
            .is_some_and(|row| row.bounds.top < 72.0),
        "the replica holds the list where it came to rest:\n{}",
        rows.text
    );
}

/// Task 10.14: a fling that carries the list into its end after the finger
/// lifted starts the stretch when it gets there: the hold counts from the
/// tree's last change, the read that found the list at its end.
#[test]
fn without_frames_a_fling_into_a_list_s_end_holds_the_wait_from_its_last_change() {
    let (target, mut mirror) = list_on_the_agent_path(list(true, true, 0.0));
    target.after(ms(150), Step::Show(list(true, true, 20.0)));
    target.after(ms(150), list_scrolled());
    // The fling moves the list 10 more points every 100 ms after the finger
    // lifts, as its end nears, the sixth move reaching the end 600 ms after
    // the finger lifted.
    for k in 1..=6u32 {
        let at = ms(300 + 100 * u64::from(k));
        let points = 20.0 + 10.0 * f64::from(k);
        target.after(at, Step::Show(list(true, k < 6, points)));
        target.after(at, list_scrolled());
    }
    let released = scroll_down(&target, &mut mirror);
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    let last_move = ms(600);
    assert!(
        settle.elapsed >= last_move + DEFAULT_OVERSCROLL,
        "settled {:?} after the stroke lifted, the fling having reached the list's end \
         {last_move:?} after it, while its stretch could still be springing back",
        settle.elapsed
    );
    let reads = target.reads();
    let last_read = reads.last().map(|r| r.at + ms(10) - released).unwrap();
    assert!(
        settle.elapsed <= last_read + DEFAULT_OVERSCROLL + DEFAULT_PROBE_INTERVAL,
        "the hold after the read that found the list at its end ({last_read:?}), then no \
         longer: {settle:?}"
    );
}

/// Every stroke but the last of a scroll down a long list leaves it between
/// its ends, where the platform draws no stretch: the wait settles a quiet
/// window after the read that found the list there, as before.
#[test]
fn without_frames_a_stroke_that_leaves_a_list_between_its_ends_settles_after_the_quiet_window() {
    let (target, mut mirror) = list_on_the_agent_path(list(true, true, 0.0));
    target.after(ms(30), Step::Show(list(true, true, 50.0)));
    target.after(ms(130), list_scrolled());
    scroll_down(&target, &mut mirror);
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    assert!(settle.elapsed >= DEFAULT_QUIET_WINDOW, "{settle:?}");
    assert!(
        settle.elapsed < DEFAULT_OVERSCROLL,
        "no hold for a list between its ends: {settle:?}"
    );
}

/// Only what lies under the point where a stroke comes down, and scrolls
/// along the stroke, scrolls: a stroke from the tab bar, or across a list
/// that scrolls up and down, leaves the wait as it was, wherever the list
/// sits, and so does a mirror whose overscroll hold is zero.
#[test]
fn without_frames_a_stroke_that_scrolls_nothing_at_an_end_settles_after_the_quiet_window() {
    for (from, to) in [
        (Point::new(204.0, 880.0), Point::new(204.0, 280.0)),
        (Point::new(380.0, 400.0), Point::new(80.0, 380.0)),
    ] {
        let (target, mut mirror) = list_on_the_agent_path(list(true, false, 0.0));
        stroke(&target, &mut mirror, from, to);
        let settle = mirror.settle(&SettleOptions::default()).unwrap();
        assert!(settle.settled, "{settle:?}");
        assert!(
            settle.elapsed < DEFAULT_OVERSCROLL,
            "a stroke from {from} to {to}: {settle:?}"
        );
    }

    let target = FakeTarget::new(list(true, false, 0.0));
    target.answer_probes(true);
    let config = MirrorConfig {
        overscroll: Duration::ZERO,
        ..MirrorConfig::default()
    };
    let mut mirror = Mirror::new(target.backend_without_frames(), config);
    mirror.pump().unwrap();
    target.advance(ms(1_000));
    scroll_down(&target, &mut mirror);
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    assert_eq!(
        settle.elapsed, DEFAULT_QUIET_WINDOW,
        "a hold of zero: {settle:?}"
    );
}

/// The hold follows the latest input only while it is a stroke on the app
/// still in front: a tap after it, or a launch, ends the hold.
#[test]
fn without_frames_a_tap_or_a_launch_after_the_stroke_ends_the_overscroll_hold() {
    for launch in [false, true] {
        let (target, mut mirror) = list_on_the_agent_path(list(true, false, 0.0));
        scroll_down(&target, &mut mirror);
        if launch {
            mirror.note_window_change();
        } else {
            tap_on(&mut mirror, "tab-list");
        }
        let settle = mirror.settle(&SettleOptions::default()).unwrap();
        assert!(settle.settled, "launch {launch}: {settle:?}");
        assert!(
            settle.elapsed < DEFAULT_OVERSCROLL,
            "launch {launch}: no hold after the stroke gave way: {settle:?}"
        );
    }
}

/// A maximum that passes during the hold names the tree, as the platform's
/// stretch is drawn from the tree's content.
#[test]
fn without_frames_a_maximum_that_passes_during_the_overscroll_hold_names_the_tree() {
    let (target, mut mirror) = list_on_the_agent_path(list(true, false, 0.0));
    scroll_down(&target, &mut mirror);
    let settle = mirror
        .settle(&SettleOptions {
            max: ms(300),
            ..SettleOptions::default()
        })
        .unwrap();
    assert!(!settle.settled, "{settle:?}");
    assert_eq!(settle.still_changing, vec![Signal::Tree], "{settle:?}");
}

/// The frame-source path is unchanged: with frames, which show the
/// stretch, a stroke that leaves a list at its end leaves the wait to them.
#[test]
fn with_frames_a_stroke_to_a_list_s_end_leaves_the_wait_to_the_frames() {
    let target = FakeTarget::new(list(true, true, 0.0));
    target.read_cost(ms(10));
    target.at(ms(0), Step::Frame(0));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(1_000));
    target.after(ms(30), Step::Show(list(true, false, 50.0)));
    target.after(ms(130), list_scrolled());
    scroll_down(&target, &mut mirror);
    // The replica has the list at its end.
    mirror.pump().unwrap();
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    assert_eq!(
        settle.elapsed, DEFAULT_QUIET_WINDOW,
        "a quiet window after the read, no frame having changed: {settle:?}"
    );
}

/// The mirror's settings for frames captured every `interval`, as a
/// physical iPhone's screenshots come (design D23).
fn slow(interval: u64) -> MirrorConfig {
    MirrorConfig {
        slow_frames: Some(SlowFrames::every(ms(interval))),
        ..MirrorConfig::default()
    }
}

/// Physical-ios, task 5.2 (design D23): on a target whose frames come
/// slowly, settle waits at least two capture intervals without a change,
/// whatever shorter quiet window it is given, since a change can take a
/// capture interval to show; the defaults keep the window given.
#[test]
fn with_slow_frames_settle_waits_at_least_two_capture_intervals_without_a_change() {
    // A frame changes at 40 ms, when the wait begins.
    let changed_then = |config: MirrorConfig| {
        let target = FakeTarget::new(Screen::load("login-ios").read());
        target.at(ms(0), Step::Frame(1));
        target.at(ms(40), Step::Frame(2));
        target.advance(ms(40));
        Mirror::new(target.backend(), config)
    };
    for (config, settled_after) in [(slow(150), 300), (MirrorConfig::default(), 100)] {
        let mut mirror = changed_then(config.clone());
        let settle = mirror.settle(&quiet(100)).unwrap();
        assert!(settle.settled, "{config:?}: {settle:?}");
        assert_eq!(settle.frame_changes, 1, "{config:?}: {settle:?}");
        assert_eq!(settle.elapsed, ms(settled_after), "{config:?}: {settle:?}");
    }
    // A window longer than two intervals stands.
    let mut mirror = changed_then(slow(50));
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert_eq!(settle.elapsed, DEFAULT_QUIET_WINDOW, "{settle:?}");
}

/// Taps the middle of the element `identifier` names, as a full snapshot
/// lists it, whether or not it has a ref.
fn tap_middle_of(mirror: &mut Mirror, identifier: &str) {
    let snapshot = mirror.snapshot(&SnapshotOptions::full()).unwrap();
    let at = snapshot.by_identifier(identifier).unwrap().bounds.center();
    mirror
        .act(&Action::Tap {
            target: Target::Point(at),
            hold: None,
        })
        .unwrap();
}

/// Physical-ios, task 5.2 (design D23): on a target whose frames come
/// slowly, a tap's first changed frame can come after the quiet window
/// (S11: 274 and 439 ms at p50 and p95 for a row that opens a page), so
/// settle after input holds until that frame, up to the first-frame wait,
/// then waits its quiet window after it; a tap that changes no frame
/// settles once the wait is over. Tree-only settling, which ignores frames,
/// does not hold for one.
#[test]
fn with_slow_frames_settle_after_input_holds_for_its_first_changed_frame() {
    let screen = Screen::load("login-ios");
    let target = FakeTarget::new(screen.read());
    target.at(ms(0), Step::Frame(1));
    target.on_tap("tab-list", vec![(ms(350), Step::Frame(2))]);
    let mut mirror = Mirror::new(target.backend(), slow(50));
    tap_on(&mut mirror, "tab-list");
    let tapped = target.elapsed();
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    assert_eq!(
        settle.frame_changes, 1,
        "the frame the tap changed: {settle:?}"
    );
    assert_eq!(
        target.elapsed() - tapped,
        ms(550),
        "a quiet window after the frame at 350 ms: {settle:?}"
    );

    let target = FakeTarget::new(screen.read());
    target.at(ms(0), Step::Frame(1));
    let mut mirror = Mirror::new(target.backend(), slow(50));
    tap_middle_of(&mut mirror, "login-status");
    let tapped = target.elapsed();
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled, "{settle:?}");
    assert_eq!(settle.frame_changes, 0, "{settle:?}");
    assert_eq!(target.elapsed() - tapped, DEFAULT_FIRST_FRAME_WAIT);

    let target = FakeTarget::new(screen.read());
    target.at(ms(0), Step::Frame(1));
    let mut mirror = Mirror::new(target.backend(), slow(50));
    tap_middle_of(&mut mirror, "login-status");
    let tapped = target.elapsed();
    let settle = mirror
        .settle(&SettleOptions {
            signals: Signals::TreeOnly,
            ..SettleOptions::default()
        })
        .unwrap();
    assert!(settle.settled, "{settle:?}");
    assert_eq!(target.elapsed() - tapped, DEFAULT_QUIET_WINDOW);
}

/// Physical-ios, task 5.2 (design D23, "without any frames"): a target
/// configured for slow frames that has no frame source settles on the
/// probe-less path for windows without events, re-read every minimum
/// interval, exactly as a target without frames does.
#[test]
fn without_frames_a_slow_frame_target_settles_on_rereads_every_minimum_interval() {
    let screen = Screen::load("compose-anim");
    let off = screen.patched(json!([{"find": "anim-toggle", "off": ["checked"]}]));
    let target = FakeTarget::new(screen.read());
    target.read_cost(ms(10));
    target.on_tap("anim-toggle", vec![(ms(150), Step::Show(off.read()))]);
    let mut mirror = Mirror::new(target.backend_without_frames(), slow(50));
    tap_on(&mut mirror, "anim-toggle");
    let tapped = target.elapsed();
    let settle = mirror.settle(&SettleOptions::default()).unwrap();
    assert!(settle.settled);
    assert_eq!(settle.tree_changes, 1, "{settle:?}");
    let reads: Vec<_> = target
        .reads()
        .iter()
        .skip(1)
        .map(|r| (r.at - tapped, r.request.reason))
        .collect();
    assert_eq!(
        reads,
        [
            (ms(0), ReadReason::Action),
            (ms(200), ReadReason::Poll),
            (ms(400), ReadReason::Poll),
        ]
    );
    assert_eq!(settle.elapsed, ms(410));
}
