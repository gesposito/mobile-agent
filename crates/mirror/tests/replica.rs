//! Task 7.1: the replica. The first use acquires every window; change events
//! are applied by re-reading the windows they name; windows whose toolkit
//! sends no change events are re-read in full after each injected action,
//! when frames change and on a poll, their deltas derived by diffing
//! consecutive reads and the read's cost and age reported in the snapshot.
//! Time is the fake target's clock: nothing sleeps.

mod support;

use mirror::backend::{ReadReason, ReadRequest};
use mirror::testing::{FakeTarget, Step};
use mirror::{Action, Mirror, MirrorConfig, Rect, RefreshMode, SnapshotOptions, Target};
use protocol::event::{TreeChangeKind, WindowChangeKind};
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
