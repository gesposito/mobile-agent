//! Task 7.10: what the mirror tells a daemon about when things happened,
//! so the daemon needs no bookkeeping of its own beside it: the latest
//! frame and frame changes, tree changes, reads and input, and a drain that
//! takes in what the target reported without reading the tree.

mod support;

use std::time::Duration;

use mirror::backend::ReadReason;
use mirror::testing::{FakeTarget, Step};
use mirror::{Action, Mirror, MirrorConfig, Point, ReadSpan, SnapshotOptions, Target, Timestamps};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind};
use serde_json::json;
use support::{Screen, fixture, ms};

fn content_changed(window: u32) -> Step {
    Step::Event(EventPayload::Tree(TreeChange {
        change: TreeChangeKind::CONTENT,
        window,
        path: vec![0],
    }))
}

#[test]
fn nothing_has_happened_before_the_first_read_and_the_first_read_gives_its_span() {
    let target = FakeTarget::new(fixture("login-ios"));
    target.read_cost(ms(12));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    assert_eq!(mirror.timestamps(), Timestamps::default());

    target.advance(ms(40));
    mirror.pump().unwrap();
    let stamps = mirror.timestamps();
    assert_eq!(
        stamps.last_read,
        Some(ReadSpan {
            started: target.instant(ms(40)),
            finished: target.instant(ms(52)),
            reason: ReadReason::Attach,
        })
    );
    assert_eq!(
        stamps.last_tree_change, None,
        "the first acquisition is no change"
    );
    assert_eq!(stamps.last_frame, None);
    assert_eq!(stamps.last_action, None);
}

#[test]
fn the_last_frame_and_frame_change_come_from_the_frames_the_mirror_took_in() {
    let target = FakeTarget::new(fixture("login-ios"));
    for (at, hash) in [(100, 1), (150, 2), (170, 2), (200, 3)] {
        target.at(ms(at), Step::Frame(hash));
    }
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();

    mirror.run_until(target.instant(ms(160))).unwrap();
    let stamps = mirror.timestamps();
    let frame = stamps.last_frame.unwrap();
    assert_eq!((frame.seq, frame.hash), (2, 2));
    assert_eq!(frame.at, target.instant(ms(150)));
    assert_eq!(stamps.last_frame_change, Some(target.instant(ms(150))));

    mirror.run_until(target.instant(ms(180))).unwrap();
    let stamps = mirror.timestamps();
    assert_eq!(
        stamps.last_frame.unwrap().at,
        target.instant(ms(170)),
        "the latest frame"
    );
    assert_eq!(
        stamps.last_frame_change,
        Some(target.instant(ms(150))),
        "a frame with the same hash is no change"
    );

    mirror.run_until(target.instant(ms(300))).unwrap();
    assert_eq!(
        mirror.timestamps().last_frame_change,
        Some(target.instant(ms(200)))
    );
    assert_eq!(
        mirror.frame_changes_since(target.instant(ms(0))),
        [target.instant(ms(150)), target.instant(ms(200))],
        "the first frame is no change"
    );
    assert_eq!(
        mirror.frame_changes_since(target.instant(ms(151))),
        [target.instant(ms(200))]
    );
    assert!(
        mirror
            .frame_changes_since(target.instant(ms(201)))
            .is_empty()
    );
}

#[test]
fn only_the_latest_1024_frame_changes_are_kept() {
    let target = FakeTarget::new(fixture("login-ios"));
    for k in 0..=1030 {
        target.at(ms(k), Step::Frame(k));
    }
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.run_until(target.instant(ms(1_100))).unwrap();
    let changes = mirror.frame_changes_since(target.instant(ms(0)));
    assert_eq!(changes.len(), 1024);
    assert_eq!(changes[0], target.instant(ms(7)));
    assert_eq!(changes[1023], target.instant(ms(1_030)));
}

#[test]
fn the_last_tree_change_is_an_event_arrival_or_the_end_of_a_read_that_found_a_delta() {
    let screen = Screen::load("login");
    let later = screen.patched(json!([{"find": "login-status", "text": "Invalid credentials"}]));
    let target = FakeTarget::new(screen.read());
    target.read_cost(ms(5));
    target.delivery_delay(ms(20));
    target.at(ms(100), Step::Show(later.read()));
    target.at(ms(100), content_changed(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();

    target.advance(ms(120) - target.elapsed());
    assert_eq!(mirror.drain(), 1);
    assert_eq!(
        mirror.timestamps().last_tree_change,
        Some(target.instant(ms(120))),
        "the event's arrival"
    );
    mirror.pump().unwrap();
    let stamps = mirror.timestamps();
    assert_eq!(
        stamps.last_tree_change,
        Some(target.instant(ms(125))),
        "the end of the read that found the delta"
    );
    assert_eq!(
        stamps.last_read,
        Some(ReadSpan {
            started: target.instant(ms(120)),
            finished: target.instant(ms(125)),
            reason: ReadReason::Events,
        })
    );

    target.advance(ms(100));
    target.delivery_delay(ms(0));
    target.at(target.elapsed(), content_changed(1));
    mirror.pump().unwrap();
    let stamps = mirror.timestamps();
    assert_eq!(
        stamps.last_tree_change,
        Some(target.instant(ms(225))),
        "an event counts on arrival even when its read finds nothing new"
    );
    assert_eq!(stamps.last_read.unwrap().finished, target.instant(ms(230)));
}

#[test]
fn the_last_action_is_when_the_mirror_injected_input() {
    let target = FakeTarget::new(fixture("login"));
    target.input_cost(ms(3));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    target.advance(ms(70));
    mirror
        .act(&Action::Tap {
            target: Target::Point(Point::new(200.0, 256.0)),
            hold: None,
        })
        .unwrap();
    assert_eq!(
        mirror.timestamps().last_action,
        Some(target.instant(ms(73))),
        "once the input was accepted"
    );
}

#[test]
fn drain_takes_in_what_arrived_without_reading_the_tree() {
    let screen = Screen::load("login");
    let later = screen.patched(json!([{"find": "login-status", "text": "Invalid credentials"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(later.read()));
    target.at(ms(100), content_changed(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(150));
    assert_eq!(mirror.drain(), 1, "the event");
    assert_eq!(mirror.drain(), 0, "nothing more arrived");
    assert_eq!(target.reads().len(), 1, "the event's read waits");
    mirror.pump().unwrap();
    let reasons: Vec<ReadReason> = target.reads().iter().map(|r| r.request.reason).collect();
    assert_eq!(reasons, [ReadReason::Attach, ReadReason::Events]);

    let target = FakeTarget::new(fixture("login-ios"));
    target.at(ms(0), Step::Frame(0));
    target.at(ms(300), Step::Frame(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(Duration::from_secs(2));
    assert_eq!(mirror.drain(), 1, "the frame");
    assert_eq!(
        mirror.timestamps().last_frame_change,
        Some(target.instant(ms(300)))
    );
    assert_eq!(
        target.reads().len(),
        1,
        "neither the poll nor the frame-driven re-read runs"
    );
    assert!(
        mirror
            .next_wakeup()
            .is_some_and(|due| due <= target.instant(Duration::from_secs(2)))
    );
    mirror.pump().unwrap();
    assert_eq!(target.reads().len(), 2);
}
