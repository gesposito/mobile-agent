//! Task 7.1: the replica. The first use acquires every window; change events
//! are applied by re-reading the windows they name; windows whose toolkit
//! sends no change events are re-read in full after each injected action,
//! when frames change and on a poll, their deltas derived by diffing
//! consecutive reads and the read's cost and age reported in the snapshot.
//! Which windows send no change events comes from the snapshot's window
//! metadata (task 3.6), inferred from Compose's classes only without it.
//! Time is the fake target's clock: nothing sleeps.

mod support;

use std::time::Duration;

use mirror::backend::{ReadReason, ReadRequest};
use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, Mirror, MirrorConfig, Point, Rect, Ref, RefreshMode, SnapshotOptions, Target,
};
use protocol::event::{TreeChangeKind, WindowChangeKind};
use protocol::message::TextMethod;
use protocol::snapshot::WindowType;
use serde_json::json;
use support::{Screen, cold_dump, mirror_on, ms, timeline, tree_event, window_event};

fn full() -> SnapshotOptions {
    SnapshotOptions::full()
}

fn scoped(windows: &[u32], reason: ReadReason) -> ReadRequest {
    ReadRequest {
        windows: Some(windows.to_vec()),
        reason,
    }
}

#[test]
fn the_first_snapshot_after_attach_acquires_the_whole_tree_then_applies_events() {
    let screen = Screen::load("login");
    let typed = screen.patched(json!([{"find": "login-username", "text": "alice"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(typed.read()));
    target.at(
        ms(100),
        tree_event(&typed, 1, TreeChangeKind::TEXT, "login-username"),
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    assert!(target.reads().is_empty(), "attaching reads nothing");

    let first = mirror.snapshot(&full()).unwrap();
    let reads = target.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(
        reads[0].request,
        ReadRequest {
            windows: None,
            reason: ReadReason::Attach
        }
    );
    assert_eq!(first.by_identifier("login-username").unwrap().value, None);
    assert_eq!(first.windows.len(), 2);

    mirror.run_until(target.instant(ms(150))).unwrap();
    let reads = target.reads();
    assert_eq!(reads.len(), 2, "the event re-reads the window it names");
    assert_eq!(reads[1].at, ms(100));
    assert_eq!(reads[1].request, scoped(&[1], ReadReason::Events));

    let second = mirror.snapshot(&full()).unwrap();
    assert_eq!(target.reads().len(), 2, "served from the replica");
    assert_eq!(
        second
            .by_identifier("login-username")
            .unwrap()
            .value
            .as_deref(),
        Some("alice")
    );
    assert!(
        second.window(2).is_some(),
        "a scoped read keeps the windows it did not ask for"
    );
}

#[test]
fn a_warm_snapshot_is_served_without_reading_the_device() {
    let (target, mut mirror) = mirror_on("login");
    let first = mirror.snapshot(&full()).unwrap();
    target.advance(ms(5_000));
    let second = mirror.snapshot(&full()).unwrap();
    assert_eq!(target.reads().len(), 1, "no round trip once warm");
    assert_eq!(first.elements, second.elements);
    assert_eq!(first.revision, second.revision);
}

#[test]
fn recorded_event_sequences_replay_to_the_trees_a_cold_read_returns() {
    let tl = timeline("events-typing");
    for whole_reads in [false, true] {
        let target = tl.target();
        target.whole_reads(whole_reads);
        let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
        mirror.pump().unwrap();
        assert_eq!(mirror.dump(), cold_dump(&tl.start.read()));

        mirror.run_until(target.instant(tl.end)).unwrap();
        assert_eq!(
            mirror.dump(),
            cold_dump(&tl.finish.read()),
            "whole reads: {whole_reads}"
        );

        let reads = target.reads();
        assert_eq!(reads.len(), 8, "one acquisition and one read per event");
        for read in &reads[1..] {
            assert_eq!(read.request, scoped(&[1], ReadReason::Events));
        }
        let changed: Vec<usize> = mirror.deltas().iter().map(|d| d.changed.len()).collect();
        assert_eq!(changed, [1, 1, 1, 1, 1, 2, 2]);
        assert!(
            mirror
                .deltas()
                .iter()
                .all(|d| d.added.is_empty() && d.removed.is_empty() && d.window == 1)
        );
        assert_eq!(mirror.revision(), 8, "the acquisition and seven deltas");
        let at: Vec<_> = mirror
            .deltas()
            .iter()
            .map(|d| d.at - target.start())
            .collect();
        assert_eq!(
            at,
            [103, 183, 263, 343, 423, 523, 643].map(ms),
            "each delta when its read finished"
        );
    }
}

#[test]
fn a_tap_on_an_event_less_window_is_reflected_by_a_full_re_read_with_its_cost() {
    let screen = Screen::load("compose-anim");
    let off = screen.patched(json!([{"find": "anim-toggle", "off": ["checked"]}]));
    let target = FakeTarget::new(screen.read());
    target.read_cost(ms(21));
    target.on_tap("anim-toggle", vec![(ms(0), Step::Show(off.read()))]);
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());

    let before = mirror.snapshot(&full()).unwrap();
    let toggle = before.by_identifier("anim-toggle").unwrap();
    assert_eq!(toggle.checked, Some(true));
    assert_eq!(before.window(5).unwrap().source.mode, RefreshMode::Reread);

    let outcome = mirror
        .act(&Action::Tap {
            target: Target::Point(toggle.bounds.center()),
            hold: None,
        })
        .unwrap();
    assert!(outcome.accepted);
    assert_eq!(target.inputs()[0].hit.as_deref(), Some("anim-toggle"));
    assert_eq!(
        target.reads().len(),
        1,
        "the re-read waits until it is needed"
    );

    target.advance(ms(30));
    let after = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        after.by_identifier("anim-toggle").unwrap().checked,
        Some(false)
    );
    let source = after.window(5).unwrap().source;
    assert_eq!(source.mode, RefreshMode::Reread);
    assert_eq!(source.reason, ReadReason::Action);
    assert_eq!(source.read_cost, ms(21));
    assert_eq!(source.age, ms(0));
    let reads = target.reads();
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[1].request, scoped(&[5], ReadReason::Action));
    let delta = mirror.deltas().last().unwrap();
    assert_eq!(delta.reason, ReadReason::Action);
    assert_eq!(delta.changed.len(), 1);
}

#[test]
fn an_idle_event_less_window_is_re_read_once_per_poll_without_deltas_when_unchanged() {
    let (target, mut mirror) = mirror_on("compose-anim");
    target.read_cost(ms(21));
    mirror.snapshot(&full()).unwrap();
    let revision = mirror.revision();
    assert_eq!(
        mirror.next_wakeup(),
        Some(target.instant(ms(1_000))),
        "a poll interval after the read started"
    );

    target.advance(ms(999) - target.elapsed());
    mirror.pump().unwrap();
    assert_eq!(target.reads().len(), 1, "not due before the poll interval");

    target.advance(ms(1));
    mirror.pump().unwrap();
    mirror.pump().unwrap();
    let reads = target.reads();
    assert_eq!(reads.len(), 2, "one re-read per poll");
    assert_eq!(reads[1].at, ms(1_000));
    assert_eq!(reads[1].request, scoped(&[5], ReadReason::Poll));
    assert!(
        mirror.deltas().is_empty(),
        "an unchanged tree yields no delta"
    );
    assert_eq!(mirror.revision(), revision);
    assert_eq!(mirror.next_wakeup(), Some(target.instant(ms(2_000))));

    target.advance(ms(100));
    let snapshot = mirror.snapshot(&full()).unwrap();
    let source = snapshot.window(5).unwrap().source;
    assert_eq!(source.reason, ReadReason::Poll);
    assert_eq!(source.read_cost, ms(21));
    assert_eq!(source.age, ms(100), "since the poll's read finished");
}

#[test]
fn an_event_less_window_that_changed_on_its_own_yields_a_delta_at_the_next_poll() {
    let screen = Screen::load("compose-anim");
    let off = screen.patched(json!([{"find": "anim-toggle", "off": ["checked"]}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(500), Step::Show(off.read()));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();

    mirror.run_until(target.instant(ms(1_100))).unwrap();
    let deltas = mirror.deltas();
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].reason, ReadReason::Poll);
    assert_eq!(deltas[0].at, target.instant(ms(1_000)));
    assert_eq!(deltas[0].changed.len(), 1);
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        snapshot.by_identifier("anim-toggle").unwrap().checked,
        Some(false)
    );
}

#[test]
fn frame_changes_re_read_event_less_windows_at_most_every_minimum_interval_and_once_after_they_stop()
 {
    let screen = Screen::load("login-ios");
    let welcome = screen.patched(json!([{"find": "login-status", "desc": "Welcome, alice"}]));
    let target = FakeTarget::new(screen.read());
    target.read_cost(ms(10));
    for k in 0..=15 {
        target.at(ms(100 + 20 * k), Step::Frame(k));
    }
    target.at(ms(390), Step::Show(welcome.read()));
    for k in 0..5 {
        target.at(ms(500 + 20 * k), Step::Frame(15));
    }
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();

    mirror.run_until(target.instant(ms(1_000))).unwrap();
    let reads: Vec<_> = target
        .reads()
        .iter()
        .map(|r| (r.at, r.request.reason))
        .collect();
    assert_eq!(
        reads,
        [
            (ms(0), ReadReason::Attach),
            (ms(210), ReadReason::Frames),
            (ms(420), ReadReason::Frames),
        ],
        "frame-driven re-reads 200 ms apart, the last after the frames stop; unchanged hashes read nothing"
    );
    assert_eq!(mirror.dump(), cold_dump(&welcome.read()));
}

/// The Login tab as the Simulator's reader shows it with the keyboard up:
/// the application's window, 1, and the keyboard's, 9, neither sending
/// change events.
fn login_with_keyboard() -> Screen {
    Screen::load("login-ios").patched(json!([{"add_window": {
        "id": 9, "kind": "input_method", "layer": 1, "bounds": [0, 539, 402, 874], "events": false,
        "root": {"class": "XCUIElementTypeKeyboard", "bounds": [0, 583, 402, 874]}
    }}]))
}

/// `screen` with `text` in its username field.
fn typed(screen: &Screen, text: &str) -> Screen {
    screen.patched(json!([{"find": "login-username", "text": text}]))
}

/// The reads `target` saw from `from` on: when each began, from the start
/// of the script, and what it asked for.
fn reads_from(target: &FakeTarget, from: Duration) -> Vec<(Duration, ReadRequest)> {
    target
        .reads()
        .into_iter()
        .filter(|read| read.at >= from)
        .map(|read| (read.at, read.request))
        .collect()
}

/// A mirror over `screen` whose reads cost 30 ms, with frames unless
/// `blind`, holding a ref to the username field from a snapshot at the
/// start, its clock moved on to 500 ms.
fn login_at_500_ms(screen: &Screen, blind: bool) -> (FakeTarget, Mirror, Ref) {
    let target = FakeTarget::new(screen.read());
    target.read_cost(ms(30));
    let backend = if blind {
        target.backend_without_frames()
    } else {
        target.at(ms(0), Step::Frame(0));
        target.backend()
    };
    let mut mirror = Mirror::new(backend, MirrorConfig::default());
    let field = mirror
        .snapshot(&full())
        .unwrap()
        .by_identifier("login-username")
        .unwrap()
        .reference
        .unwrap();
    target.advance(ms(500) - target.elapsed());
    (target, mirror, field)
}

/// Text for the field `field` names, pasted.
fn paste(field: Ref, text: &str) -> Action {
    Action::Text {
        text: text.to_owned(),
        into: Some(field),
        method: TextMethod::DIRECT,
    }
}

/// Task 10.29: on the iOS 26.5 Simulator a paste of 36 characters showed in
/// the fixtures' username fields 43 to 86 ms after Command-V, never in the
/// read the mirror made at once after the paste, and the next read of the
/// field's window, which sends no change events, came 200 ms after that
/// read ended, the minimum re-read interval: the fill took the text 252 to
/// 315 ms after the paste. After text goes into a field, a frame that
/// changes once the field's window was read calls for a re-read of that
/// window at once; the keyboard's window keeps the interval.
#[test]
fn text_into_a_field_re_reads_its_window_as_soon_as_a_frame_changes_and_others_at_the_interval() {
    let screen = login_with_keyboard();
    let (target, mut mirror, field) = login_at_500_ms(&screen, false);
    target.at(ms(560), Step::Show(typed(&screen, "agent").read()));
    target.at(ms(560), Step::Frame(1));

    mirror.act(&paste(field, "agent")).unwrap();
    mirror.run_until(target.instant(ms(1_000))).unwrap();
    assert_eq!(
        reads_from(&target, ms(500)),
        [
            (ms(500), scoped(&[9, 1], ReadReason::Action)),
            (ms(560), scoped(&[1], ReadReason::Frames)),
            (ms(730), scoped(&[9], ReadReason::Frames)),
        ],
        "the field's window as the frame that shows the text arrives, the keyboard's \
         200 ms after the read that followed the paste"
    );
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        snapshot
            .by_identifier("login-username")
            .unwrap()
            .value
            .as_deref(),
        Some("agent")
    );
}

/// Task 10.29: on a Simulator text for a field taps the field first, so the
/// read after the text can show the field focused before it shows the
/// text, as the first fill after a launch did on the iOS 26.5 Simulator;
/// the field's window is still read again as the frame that shows the text
/// arrives.
#[test]
fn a_field_that_shows_its_focus_before_its_text_has_its_window_re_read_as_the_text_shows() {
    let screen = Screen::load("login-ios");
    let (target, mut mirror, field) = login_at_500_ms(&screen, false);
    let focused = screen.patched(json!([{"find": "login-username", "flags": ["focused"]}]));
    target.input_cost(ms(20));
    target.at(ms(510), Step::Show(focused.read()));
    target.at(ms(510), Step::Frame(1));
    target.at(ms(580), Step::Show(typed(&focused, "agent").read()));
    target.at(ms(580), Step::Frame(2));

    mirror.act(&paste(field, "agent")).unwrap();
    mirror.run_until(target.instant(ms(1_000))).unwrap();
    assert_eq!(
        reads_from(&target, ms(500)),
        [
            (ms(520), scoped(&[1], ReadReason::Action)),
            (ms(580), scoped(&[1], ReadReason::Frames)),
        ],
        "the read at 520 ms shows the field focused, the one at 580 ms its text"
    );
}

/// Task 10.29: the sooner re-read follows text alone, and only until a
/// read shows the field's text changed: frames after a tap, after the
/// field showed the text, or after a tap that followed the text re-read the
/// field's window no sooner than the minimum interval after its last read.
#[test]
fn frames_after_a_tap_or_once_the_field_changed_re_read_at_the_minimum_interval() {
    let screen = Screen::load("login-ios");
    let tap = Action::Tap {
        target: Target::Point(Point::new(200.0, 40.0)),
        hold: None,
    };

    let (target, mut mirror, _) = login_at_500_ms(&screen, false);
    target.at(ms(560), Step::Frame(1));
    mirror.act(&tap).unwrap();
    mirror.run_until(target.instant(ms(1_000))).unwrap();
    assert_eq!(
        reads_from(&target, ms(500)),
        [
            (ms(500), scoped(&[1], ReadReason::Action)),
            (ms(730), scoped(&[1], ReadReason::Frames)),
        ],
        "after a tap"
    );

    let (target, mut mirror, field) = login_at_500_ms(&screen, false);
    target.at(ms(560), Step::Show(typed(&screen, "agent").read()));
    target.at(ms(560), Step::Frame(1));
    target.at(ms(650), Step::Frame(2));
    mirror.act(&paste(field, "agent")).unwrap();
    mirror.run_until(target.instant(ms(1_000))).unwrap();
    assert_eq!(
        reads_from(&target, ms(500)),
        [
            (ms(500), scoped(&[1], ReadReason::Action)),
            (ms(560), scoped(&[1], ReadReason::Frames)),
            (ms(790), scoped(&[1], ReadReason::Frames)),
        ],
        "once the read at 560 ms showed the text"
    );

    let (target, mut mirror, field) = login_at_500_ms(&screen, false);
    target.at(ms(600), Step::Show(typed(&screen, "agent").read()));
    target.at(ms(600), Step::Frame(1));
    mirror.act(&paste(field, "agent")).unwrap();
    mirror.pump().unwrap();
    target.advance(ms(10));
    mirror.act(&tap).unwrap();
    mirror.run_until(target.instant(ms(1_000))).unwrap();
    assert_eq!(
        reads_from(&target, ms(500)),
        [
            (ms(500), scoped(&[1], ReadReason::Action)),
            (ms(540), scoped(&[1], ReadReason::Action)),
            (ms(770), scoped(&[1], ReadReason::Frames)),
        ],
        "after a tap that followed the text"
    );
}

/// Task 10.29: text typed as keys keeps the interval. A `fill` holds typed
/// text 200 ms after the entry, which a read at the first frame cannot end,
/// and that read would put the next one 200 ms after it: on the iOS 26.5
/// Simulator the read-back of Command-A and the delete key took 307.7 ms at
/// p50 with it, against 279.6 ms without.
#[test]
fn text_typed_as_keys_re_reads_its_fields_window_at_the_minimum_interval() {
    let screen = Screen::load("login-ios");
    let (target, mut mirror, field) = login_at_500_ms(&screen, false);
    target.at(ms(560), Step::Show(typed(&screen, "agent").read()));
    target.at(ms(560), Step::Frame(1));
    target.at(ms(600), Step::Frame(2));

    mirror
        .act(&Action::Text {
            text: "agent".to_owned(),
            into: Some(field),
            method: TextMethod::KEYS,
        })
        .unwrap();
    mirror.run_until(target.instant(ms(1_000))).unwrap();
    assert_eq!(
        reads_from(&target, ms(500)),
        [
            (ms(500), scoped(&[1], ReadReason::Action)),
            (ms(730), scoped(&[1], ReadReason::Frames)),
        ],
        "200 ms after the read that followed the keys"
    );
}

/// Task 10.29: without frames nothing says when the field changes, so its
/// window is read again as soon as each read ends until one shows the
/// field changed; the keyboard's window waits for its poll.
#[test]
fn without_frames_text_into_a_field_re_reads_its_window_until_the_field_changes() {
    let screen = login_with_keyboard();
    let (target, mut mirror, field) = login_at_500_ms(&screen, true);
    target.at(ms(580), Step::Show(typed(&screen, "agent").read()));

    mirror.act(&paste(field, "agent")).unwrap();
    mirror.run_until(target.instant(ms(1_550))).unwrap();
    assert_eq!(
        reads_from(&target, ms(500)),
        [
            (ms(500), scoped(&[9, 1], ReadReason::Action)),
            (ms(530), scoped(&[1], ReadReason::Action)),
            (ms(560), scoped(&[1], ReadReason::Action)),
            (ms(590), scoped(&[1], ReadReason::Action)),
            (ms(1_500), scoped(&[9], ReadReason::Poll)),
        ],
        "the field's window until the read at 590 ms shows the text"
    );
}

/// Task 10.29: text that leaves its field as it was, as text the field
/// already shows, never shows a change, so the sooner re-read lasts one
/// poll interval after the text at most: then frames re-read the field's
/// window no sooner than the minimum interval again.
#[test]
fn text_that_leaves_its_field_unchanged_re_reads_its_window_sooner_for_one_poll_interval() {
    let screen = Screen::load("login-ios");
    let (target, mut mirror, field) = login_at_500_ms(&screen, false);
    for k in 0..15 {
        target.at(ms(550 + 100 * k), Step::Frame(k + 1));
    }

    mirror.act(&paste(field, "")).unwrap();
    mirror.run_until(target.instant(ms(2_000))).unwrap();
    let mut expected = vec![(ms(500), scoped(&[1], ReadReason::Action))];
    expected.extend((0..10).map(|k| (ms(550 + 100 * k), scoped(&[1], ReadReason::Frames))));
    expected.extend([
        (ms(1_680), scoped(&[1], ReadReason::Frames)),
        (ms(1_910), scoped(&[1], ReadReason::Frames)),
    ]);
    assert_eq!(
        reads_from(&target, ms(500)),
        expected,
        "each frame until 1,500 ms, then 200 ms after a read ends"
    );
}

/// A window the protocol marks as sending no change events is re-read after
/// an injected action and on a poll, whatever toolkit its classes suggest.
#[test]
fn window_metadata_that_a_window_sends_no_events_puts_it_on_the_re_read_path() {
    let mut screen = Screen::load("login");
    for window in screen.0["windows"].as_array_mut().unwrap() {
        if window["id"] == 1 {
            window["events"] = json!(false);
        }
    }
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    assert_eq!(before.window(1).unwrap().source.mode, RefreshMode::Reread);
    assert_eq!(before.window(2).unwrap().source.mode, RefreshMode::Events);
    assert_eq!(
        mirror.next_wakeup(),
        Some(target.instant(ms(1_000))),
        "the window is polled"
    );

    let submit = before.by_identifier("login-submit").unwrap();
    mirror
        .act(&Action::Tap {
            target: Target::Point(submit.bounds.center()),
            hold: None,
        })
        .unwrap();
    target.advance(ms(30));
    let after = mirror.snapshot(&full()).unwrap();
    assert_eq!(after.window(1).unwrap().source.reason, ReadReason::Action);
    assert_eq!(
        target.reads().last().unwrap().request,
        scoped(&[1], ReadReason::Action)
    );
}

/// A window the protocol marks as sending change events is kept current
/// from them, even when it hosts Compose's view class.
#[test]
fn window_metadata_that_a_window_sends_events_keeps_it_off_the_re_read_path() {
    let mut screen = Screen::load("compose-anim");
    screen.0["windows"][0]["events"] = json!(true);
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert_eq!(snapshot.window(5).unwrap().source.mode, RefreshMode::Events);
    assert_eq!(mirror.next_wakeup(), None, "nothing to poll");
    let toggle = snapshot.by_identifier("anim-toggle").unwrap();
    mirror
        .act(&Action::Tap {
            target: Target::Point(toggle.bounds.center()),
            hold: None,
        })
        .unwrap();
    target.advance(ms(30));
    mirror.snapshot(&full()).unwrap();
    assert_eq!(target.reads().len(), 1, "no re-read after the tap");
}

/// The Compose Anim screen with its window's root and the node below it,
/// the Compose host in the fixture, given the classes `root` and `host`, and
/// the window metadata `events` when given. No other node's class is under
/// `androidx.compose.`.
fn compose_anim_with(root: &str, host: &str, events: Option<bool>) -> Screen {
    let mut screen = Screen::load("compose-anim");
    let window = &mut screen.0["windows"][0];
    window["root"]["class"] = json!(root);
    window["root"]["children"][0]["class"] = json!(host);
    if let Some(events) = events {
        window["events"] = json!(events);
    }
    screen
}

/// How the mirror keeps window 5 of `screen` current.
fn refresh_mode(screen: &Screen) -> RefreshMode {
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&full()).unwrap();
    snapshot.window(5).unwrap().source.mode
}

/// Without window metadata, a window holding a node of any class under
/// `androidx.compose.` is taken to send no change events: on the API 36
/// emulator a Compose window shows `androidx.compose.ui.platform.ComposeView`
/// rather than the `AndroidComposeView` of spike S1. The metadata, when
/// present, decides whatever the classes.
#[test]
fn without_window_metadata_any_androidx_compose_class_puts_a_window_on_the_re_read_path() {
    const FRAME: &str = "android.widget.FrameLayout";
    const VIEW: &str = "android.view.View";
    for (root, host) in [
        ("androidx.compose.ui.platform.ComposeView", VIEW),
        (FRAME, "androidx.compose.ui.platform.ComposeView"),
        (FRAME, "androidx.compose.ui.window.DialogLayout"),
        (FRAME, "androidx.compose.ui.platform.AndroidComposeView"),
    ] {
        assert_eq!(
            refresh_mode(&compose_anim_with(root, host, None)),
            RefreshMode::Reread,
            "{root} > {host} without window metadata"
        );
        assert_eq!(
            refresh_mode(&compose_anim_with(root, host, Some(true))),
            RefreshMode::Events,
            "{root} > {host} marked as sending events"
        );
    }
    for host in [VIEW, "com.example.ui.ComposeView"] {
        assert_eq!(
            refresh_mode(&compose_anim_with(FRAME, host, None)),
            RefreshMode::Events,
            "{FRAME} > {host}: no class under androidx.compose."
        );
        assert_eq!(
            refresh_mode(&compose_anim_with(FRAME, host, Some(false))),
            RefreshMode::Reread,
            "{FRAME} > {host} marked as sending no events"
        );
    }
}

#[test]
fn bounds_are_in_device_points_with_the_scale_factor_beside_them() {
    let (_, mut mirror) = mirror_on("login");
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert_eq!(snapshot.scale, 2.625);
    assert_eq!(
        snapshot.by_identifier("login-username").unwrap().bounds,
        Rect::new(16.0, 96.0, 392.0, 144.0)
    );
    assert_eq!(
        snapshot.window(1).unwrap().bounds,
        Rect::new(0.0, 0.0, 408.0, 912.0)
    );

    let (_, mut mirror) = mirror_on("login-ios");
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert_eq!(snapshot.scale, 3.0);
    assert_eq!(
        snapshot.by_identifier("login-submit").unwrap().bounds,
        Rect::new(16.0, 236.0, 386.0, 280.0)
    );
}

#[test]
fn a_window_change_reads_every_window() {
    let screen = Screen::load("login");
    let keyboard = screen.patched(json!([{"add_window": {
        "id": 9, "kind": "input_method", "layer": 2, "package": "com.google.android.inputmethod.latin",
        "title": "Keyboard", "bounds": [0, 600, 408, 912],
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 600, 408, 912]}
    }}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(keyboard.read()));
    target.at(ms(100), window_event(WindowChangeKind::ADDED, 9));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();

    mirror.run_until(target.instant(ms(200))).unwrap();
    let reads = target.reads();
    assert_eq!(reads.len(), 2);
    assert_eq!(
        reads[1].request,
        ReadRequest {
            windows: None,
            reason: ReadReason::Events
        }
    );
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        snapshot.window(9).unwrap().kind,
        WindowType::INPUT_METHOD,
        "the new window is in the replica"
    );
    assert_eq!(mirror.deltas().last().unwrap().window, 9);
}
