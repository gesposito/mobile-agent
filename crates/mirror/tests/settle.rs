//! Task 7.4: the settle predicate. Settled means no tree change and no frame
//! hash change for the quiet window, 200 ms by default (spike S1), bounded by
//! a maximum, 3 s by default, after which the result names what kept
//! changing. Frame callbacks that change no pixel do not count (spike S2).
//! Windows that send no change events get their tree signal from re-reads
//! when there are no frames to watch, and nodes that move without a change
//! event get theirs from bounds probes (task 10.11), while a window the
//! platform animates in holds the wait as long as it is flagged
//! transformed.

mod support;

use std::time::Duration;

use mirror::backend::{NodeHandle, ReadReason, TreeRead};
use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, DEFAULT_PROBE_ANCHORS, DEFAULT_PROBE_INTERVAL, DEFAULT_QUIET_WINDOW,
    DEFAULT_SETTLE_MAX, Mirror, MirrorConfig, Rect, SettleOptions, Signal, Signals,
    SnapshotOptions, Target,
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
