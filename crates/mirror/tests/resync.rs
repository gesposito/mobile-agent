//! Task 7.7: quality verdicts and resynchronization. Every snapshot names
//! the backend that produced it and whether, and why, it is degraded. A
//! sequence gap, a driver's out-of-sync report, a reconnection or a switch of
//! tree backend rebuilds the replica, and refs issued before are stale.

mod support;

use mirror::backend::{BackendInfo, ReadReason, StreamItem};
use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, Degradation, Mirror, MirrorConfig, MirrorError, Ref, ResyncCause, SnapshotOptions,
    StaleCause, Target, code,
};
use protocol::event::{EventPayload, Lifecycle, LifecycleKind, Stream, TreeChange, TreeChangeKind};
use serde_json::json;
use support::{Screen, cold_dump, fixture, ms, timeline};

fn full() -> SnapshotOptions {
    SnapshotOptions::full()
}

fn text_changed(screen: &Screen, id: &str) -> Step {
    Step::Event(EventPayload::Tree(TreeChange {
        change: TreeChangeKind::TEXT,
        window: 1,
        path: screen.path(1, id),
    }))
}

fn stale_cause(mirror: &mut Mirror, reference: Ref) -> StaleCause {
    match mirror.resolve(reference) {
        Err(MirrorError::StaleRef { cause, .. }) => cause,
        other => panic!("{reference} should be stale: {other:?}"),
    }
}

#[test]
fn a_backend_fallback_rebuilds_the_replica_stales_old_refs_and_names_the_fallback() {
    let target = FakeTarget::new(fixture("login-ios"));
    let translator = BackendInfo::new("simulator-translator", true);
    target.at(ms(100), Step::Show(fixture("login-translator")));
    target.at(
        ms(100),
        Step::Item(StreamItem::BackendChanged(translator.clone())),
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    assert_eq!(before.verdict.backend.name, "simulator-reader");
    assert!(!before.verdict.degraded);
    let submit = before
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();
    let highest = before.refs().into_iter().max().unwrap();

    mirror.run_until(target.instant(ms(200))).unwrap();
    assert_eq!(
        target.reads().last().unwrap().request.reason,
        ReadReason::Resync,
        "the replica is rebuilt from a full read"
    );
    assert_eq!(
        stale_cause(&mut mirror, submit),
        StaleCause::Resync(ResyncCause::Backend(translator.clone()))
    );
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(submit),
            hold: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), code::STALE_REF);
    assert!(
        error
            .to_string()
            .contains("after a switch to the simulator-translator tree backend"),
        "{error}"
    );

    let after = mirror.snapshot(&full()).unwrap();
    assert_eq!(after.verdict.backend, translator);
    assert!(after.verdict.degraded);
    assert_eq!(
        after.verdict.reasons,
        [Degradation::FallbackBackend {
            name: "simulator-translator".to_owned()
        }]
    );
    assert!(after.verdict.advice()[0].contains("fallback backend simulator-translator"));
    let rebuilt = after
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();
    assert!(rebuilt > highest, "{rebuilt} is issued after {highest}");
}

#[test]
fn a_read_from_another_backend_rebuilds_the_replica_even_unannounced() {
    let target = FakeTarget::new(fixture("login-ios"));
    target.at(ms(100), Step::Show(fixture("login-translator")));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(120), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    let submit = before
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();
    mirror.run_until(target.instant(ms(600))).unwrap();
    let reasons: Vec<ReadReason> = target.reads().iter().map(|r| r.request.reason).collect();
    assert_eq!(
        reasons,
        [ReadReason::Attach, ReadReason::Frames, ReadReason::Resync]
    );
    assert!(matches!(
        stale_cause(&mut mirror, submit),
        StaleCause::Resync(ResyncCause::Backend(_))
    ));
}

#[test]
fn a_sequence_gap_rebuilds_the_replica_and_stales_earlier_refs() {
    let tl = timeline("events-gap");
    let target = tl.target();
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    let username = before
        .by_identifier("login-username")
        .unwrap()
        .reference
        .unwrap();

    mirror.run_until(target.instant(tl.end)).unwrap();
    let reads: Vec<_> = target
        .reads()
        .iter()
        .map(|r| (r.at, r.request.reason, r.request.windows.is_none()))
        .collect();
    assert_eq!(
        reads,
        [
            (ms(0), ReadReason::Attach, true),
            (ms(100), ReadReason::Events, false),
            (ms(300), ReadReason::Resync, true),
            (ms(400), ReadReason::Events, false),
        ],
        "event 3 after event 1 is a gap: a full read, then events apply again"
    );
    assert_eq!(
        stale_cause(&mut mirror, username),
        StaleCause::Resync(ResyncCause::Gap {
            stream: Stream::TREE,
            expected: 2,
            received: 3
        })
    );
    assert_eq!(mirror.dump(), cold_dump(&tl.finish.read()));
    let after = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        after
            .by_identifier("login-username")
            .unwrap()
            .value
            .as_deref(),
        Some("abcd")
    );
    assert!(!after.verdict.degraded);
}

#[test]
fn an_out_of_sync_report_from_the_driver_also_rebuilds_the_replica() {
    let tl = timeline("events-resync");
    let target = tl.target();
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    let username = before
        .by_identifier("login-username")
        .unwrap()
        .reference
        .unwrap();

    mirror.run_until(target.instant(tl.end)).unwrap();
    let reasons: Vec<ReadReason> = target.reads().iter().map(|r| r.request.reason).collect();
    assert_eq!(
        reasons,
        [
            ReadReason::Attach,
            ReadReason::Events,
            ReadReason::Resync,
            ReadReason::Events,
            ReadReason::Events,
        ],
        "after the driver's resync at 3, events 4 and 5 apply without another rebuild"
    );
    assert!(matches!(
        stale_cause(&mut mirror, username),
        StaleCause::Resync(ResyncCause::Gap { .. })
    ));
    assert_eq!(mirror.dump(), cold_dump(&tl.finish.read()));
}

#[test]
fn events_a_read_already_reflects_are_skipped_but_a_gap_is_not() {
    let screen = Screen::load("login");
    let typed = screen.patched(json!([{"find": "login-username", "text": "a"}]));
    let target = FakeTarget::new(screen.read());
    target.delivery_delay(ms(20));
    target.at(ms(100), Step::Show(typed.read()));
    target.at(ms(100), text_changed(&typed, "login-username"));
    target.at(ms(300), text_changed(&typed, "login-username"));
    target.at(ms(350), Step::Drop(Stream::TREE));
    target.at(ms(400), text_changed(&typed, "login-username"));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    target.advance(ms(110));
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        snapshot
            .by_identifier("login-username")
            .unwrap()
            .value
            .as_deref(),
        Some("a"),
        "the read at 110 ms reflects event 1, which arrives at 120 ms"
    );
    mirror.run_until(target.instant(ms(500))).unwrap();
    let reads: Vec<_> = target
        .reads()
        .iter()
        .map(|r| (r.at, r.request.reason))
        .collect();
    assert_eq!(
        reads,
        [
            (ms(110), ReadReason::Attach),
            (ms(320), ReadReason::Events),
            (ms(420), ReadReason::Resync),
        ],
        "event 1 is skipped, event 2 is read, event 4 after 2 rebuilds the replica"
    );
}

#[test]
fn a_reconnection_rebuilds_the_replica_and_restarts_sequence_numbers() {
    let screen = Screen::load("login");
    let typed = screen.patched(json!([{"find": "login-username", "text": "a"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), text_changed(&screen, "login-username"));
    target.at(ms(200), Step::Item(StreamItem::Reconnected));
    target.at(ms(300), Step::Show(typed.read()));
    target.at(
        ms(300),
        Step::EventSeq(
            1,
            EventPayload::Tree(TreeChange {
                change: TreeChangeKind::TEXT,
                window: 1,
                path: typed.path(1, "login-username"),
            }),
        ),
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    let submit = before
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();
    mirror.run_until(target.instant(ms(400))).unwrap();
    let reasons: Vec<ReadReason> = target.reads().iter().map(|r| r.request.reason).collect();
    assert_eq!(
        reasons,
        [
            ReadReason::Attach,
            ReadReason::Events,
            ReadReason::Resync,
            ReadReason::Events,
        ],
        "number 1 on the new connection is applied"
    );
    assert_eq!(
        stale_cause(&mut mirror, submit),
        StaleCause::Resync(ResyncCause::Reconnected)
    );
    let after = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        after
            .by_identifier("login-username")
            .unwrap()
            .value
            .as_deref(),
        Some("a")
    );
}

#[test]
fn a_root_only_read_of_a_populated_screen_is_degraded_with_advice() {
    let target = FakeTarget::new(fixture("sparse"));
    target.at(ms(0), Step::Frame(42));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert!(snapshot.verdict.degraded);
    assert_eq!(snapshot.verdict.backend.name, "simulator-reader");
    assert_eq!(
        snapshot.verdict.reasons,
        [Degradation::SparseTree { window: 1 }]
    );
    let advice = &snapshot.verdict.advice()[0];
    assert!(
        advice.contains("retry") && advice.contains("frame"),
        "{advice}"
    );
    assert_eq!(snapshot.text, "(empty screen)");

    let sparse = Screen::load("sparse").patched(json!([
        {"backend": {"name": "simulator-translator", "fallback": true}}
    ]));
    let target = FakeTarget::new(sparse.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let codes: Vec<&str> = mirror
        .snapshot(&full())
        .unwrap()
        .verdict
        .reasons
        .iter()
        .map(Degradation::code)
        .collect();
    assert_eq!(codes, ["fallback_backend", "sparse_tree"]);
}

#[test]
fn truncated_reads_are_degraded_naming_the_limit() {
    let mut screen = Screen::load("grid");
    screen.0["truncated"] = json!(["nodes"]);
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    assert_eq!(
        snapshot.verdict.reasons,
        [Degradation::Truncated {
            depth: false,
            nodes: true,
            windows: false
        }]
    );
    assert!(snapshot.verdict.advice()[0].contains("node count limit"));
}

#[test]
fn agent_warnings_and_disconnection_reach_the_verdict() {
    let target = FakeTarget::new(fixture("login"));
    target.at(
        ms(100),
        Step::Event(EventPayload::Lifecycle(Lifecycle {
            change: LifecycleKind::WARNING,
            message: "input injection unavailable".to_owned(),
        })),
    );
    target.at(
        ms(200),
        Step::Item(StreamItem::Disconnected("the agent exited".to_owned())),
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    mirror.run_until(target.instant(ms(150))).unwrap();
    let warned = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        warned.verdict.reasons,
        [Degradation::AgentWarning(
            "input injection unavailable".to_owned()
        )]
    );

    mirror.run_until(target.instant(ms(250))).unwrap();
    let reads = target.reads().len();
    let last = mirror.snapshot(&full()).unwrap();
    assert!(
        last.verdict
            .reasons
            .contains(&Degradation::Disconnected("the agent exited".to_owned()))
    );
    assert_eq!(
        target.reads().len(),
        reads,
        "nothing is read once disconnected"
    );
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(Ref(1)),
            hold: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), code::DISCONNECTED);
    assert!(target.inputs().is_empty());
}

#[test]
fn events_on_streams_this_version_does_not_know_are_ignored_even_with_gaps() {
    let target = FakeTarget::new(fixture("login"));
    target.at(ms(100), Step::EventSeq(5, EventPayload::Unknown(Stream(9))));
    target.at(ms(200), Step::EventSeq(9, EventPayload::Unknown(Stream(9))));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    mirror.run_until(target.instant(ms(300))).unwrap();
    let reasons: Vec<ReadReason> = target.reads().iter().map(|r| r.request.reason).collect();
    assert_eq!(reasons, [ReadReason::Attach], "nothing to re-read");
    let submit = before
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();
    assert!(mirror.resolve(submit).is_ok(), "refs stay valid");
}
