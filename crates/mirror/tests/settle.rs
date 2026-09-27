//! Task 7.4: the settle predicate. Settled means no tree change and no frame
//! hash change for the quiet window, 200 ms by default (spike S1), bounded by
//! a maximum, 3 s by default, after which the result names what kept
//! changing. Frame callbacks that change no pixel do not count (spike S2).
//! Windows that send no change events get their tree signal from re-reads
//! when there are no frames to watch.

mod support;

use std::time::Duration;

use mirror::backend::ReadReason;
use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, DEFAULT_QUIET_WINDOW, DEFAULT_SETTLE_MAX, Mirror, MirrorConfig, SettleOptions, Signal,
    Signals, SnapshotOptions, Target,
};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind};
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
