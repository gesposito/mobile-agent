//! Task 7.5: waits from the event stream. A wait for an element to appear,
//! disappear or become enabled re-evaluates when the target reports a change
//! or a re-read of a window without change events falls due, never on a
//! fixed sleep, and fails at its timeout with the last screen it saw.

mod support;

use std::time::Duration;

use mirror::backend::ReadReason;
use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, Condition, DEFAULT_POLL_INTERVAL, Mirror, MirrorConfig, MirrorError, Ref, Role,
    Selector, SnapshotOptions, Target, code,
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

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn a_wait_for_text_resolves_within_one_processing_cycle_of_its_appearance() {
    let screen = Screen::load("login");
    let welcome = screen.patched(json!([{"find": "login-status", "text": "Welcome, alice"}]));
    let target = FakeTarget::new(screen.read());
    target.read_cost(ms(3));
    target.at(ms(800), Step::Show(welcome.read()));
    target.at(ms(800), content_changed(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    let started = target.elapsed();

    let outcome = mirror
        .wait(&Selector::text("Welcome"), Condition::Appear, secs(5))
        .unwrap();
    assert_eq!(
        outcome.elapsed,
        ms(800) + ms(3) - started,
        "the read the appearance's event triggered is the only work before resolving"
    );
    let element = outcome.element.unwrap();
    assert_eq!(element.identifier.as_deref(), Some("login-status"));
    assert_eq!(element.name, "Welcome, alice");
    let reads: Vec<_> = target
        .reads()
        .iter()
        .map(|r| (r.at, r.request.reason))
        .collect();
    assert_eq!(
        reads,
        [(ms(0), ReadReason::Attach), (ms(800), ReadReason::Events)],
        "nothing polls while the screen is quiet"
    );
}

#[test]
fn a_wait_that_never_matches_fails_at_the_timeout_with_the_last_snapshot_summary() {
    let target = FakeTarget::new(Screen::load("login").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let error = mirror
        .wait(&Selector::text("Welcome"), Condition::Appear, secs(2))
        .unwrap_err();
    assert_eq!(error.code(), code::WAIT_TIMEOUT);
    assert_eq!(target.elapsed(), secs(2), "it fails at the timeout");
    let MirrorError::WaitTimeout {
        waited, summary, ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(*waited, secs(2));
    assert_eq!(summary.elements, 8);
    assert_eq!(
        summary.lines[2],
        "- button \"Sign in\" [id=login-submit, ref=e3]"
    );
    assert_eq!(summary.verdict.backend.name, "android-agent");
    let message = error.to_string();
    assert!(
        message.starts_with("waited 2000 ms for text \"Welcome\" to appear;"),
        "{message}"
    );
    assert!(message.contains("- button \"Sign in\""), "{message}");
}

#[test]
fn a_disappear_wait_resolves_when_the_element_leaves() {
    let screen = Screen::load("login");
    let gone = screen.patched(json!([{"remove": "login-submit"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(300), Step::Show(gone.read()));
    target.at(ms(300), content_changed(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let outcome = mirror
        .wait(
            &Selector::identifier("login-submit"),
            Condition::Disappear,
            secs(5),
        )
        .unwrap();
    assert_eq!(outcome.elapsed, ms(300));
    assert!(outcome.element.is_none());
}

#[test]
fn an_enabled_wait_resolves_when_the_element_becomes_enabled() {
    let screen = Screen::load("login");
    let disabled = screen.patched(json!([{"find": "login-submit", "off": ["enabled"]}]));
    let target = FakeTarget::new(disabled.read());
    target.at(ms(400), Step::Show(screen.read()));
    target.at(ms(400), content_changed(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let selector = Selector::identifier("login-submit");
    let appeared = mirror.wait(&selector, Condition::Appear, secs(1)).unwrap();
    assert_eq!(appeared.elapsed, ms(0));
    assert!(!appeared.element.unwrap().enabled);

    let enabled = mirror.wait(&selector, Condition::Enabled, secs(5)).unwrap();
    assert_eq!(enabled.elapsed, ms(400));
    assert!(enabled.element.unwrap().enabled);
}

#[test]
fn a_wait_on_a_window_without_change_events_resolves_from_its_re_reads() {
    let screen = Screen::load("login-ios");
    let welcome = screen.patched(json!([{"find": "login-status", "desc": "Welcome, alice"}]));
    let script = |target: &FakeTarget, frames: bool| {
        target.read_cost(ms(12));
        target.at(ms(600), Step::Show(welcome.read()));
        if frames {
            target.at(ms(0), Step::Frame(0));
            target.at(ms(600), Step::Frame(1));
            target.at(ms(616), Step::Frame(2));
        }
    };

    let target = FakeTarget::new(screen.read());
    script(&target, true);
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let outcome = mirror
        .wait(&Selector::text("Welcome"), Condition::Appear, secs(5))
        .unwrap();
    assert_eq!(
        outcome.elapsed,
        ms(600) + ms(12),
        "the first frame change re-reads the window"
    );
    assert_eq!(
        target.reads().last().unwrap().request.reason,
        ReadReason::Frames
    );

    let target = FakeTarget::new(screen.read());
    script(&target, false);
    let mut blind = Mirror::new(target.backend_without_frames(), MirrorConfig::default());
    let outcome = blind
        .wait(&Selector::text("Welcome"), Condition::Appear, secs(5))
        .unwrap();
    assert_eq!(
        outcome.elapsed,
        ms(1_000) + ms(12),
        "without frames, the next poll finds it"
    );
    assert_eq!(
        target.reads().last().unwrap().request.reason,
        ReadReason::Poll
    );
}

#[test]
fn a_wait_that_already_holds_returns_at_once() {
    let target = FakeTarget::new(Screen::load("login").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let outcome = mirror
        .wait(
            &Selector::identifier("login-username"),
            Condition::Appear,
            secs(5),
        )
        .unwrap();
    assert_eq!(outcome.elapsed, ms(0));
    assert_eq!(outcome.element.unwrap().role, Role::Textbox);
}

/// Rn-app-fixes D36, scenario "Condition already holds": a wait for text
/// the replica already lists answers from it at once, without the read
/// that is due, here a poll of a window that sends no change events, which
/// a wait paid 43 to 473 ms for in the smoke test; the next pump takes the
/// read.
#[test]
fn a_wait_that_holds_in_the_replica_answers_before_a_poll_that_is_due() {
    let target = FakeTarget::new(Screen::load("login-ios").read());
    target.read_cost(ms(380));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(DEFAULT_POLL_INTERVAL + ms(200));
    let now = target.instant(target.elapsed());
    assert!(
        mirror.next_wakeup().is_some_and(|due| due <= now),
        "a poll is due"
    );
    let reads = target.reads().len();

    let outcome = mirror
        .wait(&Selector::text("Sign in"), Condition::Appear, secs(5))
        .unwrap();
    assert_eq!(outcome.elapsed, ms(0), "answered from the replica");
    assert_eq!(
        outcome.element.unwrap().identifier.as_deref(),
        Some("login-submit")
    );
    assert_eq!(target.reads().len(), reads, "no read before the answer");

    mirror.pump().unwrap();
    let poll = target.reads().last().cloned().unwrap();
    assert_eq!(
        (poll.at, poll.request.reason),
        (ms(380) + DEFAULT_POLL_INTERVAL + ms(200), ReadReason::Poll),
        "the next pump takes the poll"
    );
}

/// D36 answers from the replica while it is current by the mirror's own
/// rules. Right after a tap, which the mirror reads a window without
/// change events again for when the tree is next needed, and once an event
/// names a window, a wait first takes the read: it does not answer from
/// the screen before either.
#[test]
fn a_wait_does_not_answer_from_a_replica_behind_a_tap_or_an_event() {
    let screen = Screen::load("login-ios");
    let welcome = screen.patched(json!([{"find": "login-status", "desc": "Welcome, alice"}]));
    let target = FakeTarget::new(screen.read());
    target.on_tap("login-submit", vec![(ms(0), Step::Show(welcome.read()))]);
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let submit = mirror
        .snapshot(&SnapshotOptions::interactive())
        .unwrap()
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();
    mirror
        .act(&Action::Tap {
            target: Target::Ref(submit),
            hold: None,
        })
        .unwrap();
    let error = mirror
        .wait(&Selector::text("Signed out"), Condition::Appear, ms(0))
        .unwrap_err();
    assert_eq!(error.code(), code::WAIT_TIMEOUT, "{error}");

    let screen = Screen::load("login");
    let welcome = screen.patched(json!([{"find": "login-status", "text": "Welcome, alice"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(welcome.read()));
    target.at(ms(100), content_changed(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(200));
    let error = mirror
        .wait(&Selector::text("Signed out"), Condition::Appear, ms(0))
        .unwrap_err();
    assert_eq!(error.code(), code::WAIT_TIMEOUT, "{error}");
}

#[test]
fn selectors_match_text_identifier_role_and_ref_on_screen_only() {
    let target = FakeTarget::new(Screen::load("login").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&SnapshotOptions::interactive()).unwrap();
    assert_eq!(
        snapshot.by_ref(Ref(3)).unwrap().identifier.as_deref(),
        Some("login-submit")
    );
    let quick = ms(10);
    let found = |mirror: &mut Mirror, selector: Selector| {
        mirror.wait(&selector, Condition::Appear, quick).is_ok()
    };
    assert!(found(
        &mut mirror,
        Selector::text("Sign").with_role(Role::Button)
    ));
    assert!(!found(
        &mut mirror,
        Selector::text("Sign").with_role(Role::Textbox)
    ));
    assert!(found(&mut mirror, Selector::identifier("tab-big")));
    assert!(!found(&mut mirror, Selector::identifier("tab")));
    assert!(found(
        &mut mirror,
        Selector::reference(Ref(3)).with_role(Role::Button)
    ));
    assert_eq!(Selector::text("Welcome").to_string(), "text \"Welcome\"");
    assert_eq!(
        Selector::identifier("login-submit")
            .with_role(Role::Button)
            .to_string(),
        "button with id login-submit"
    );
    assert_eq!(Selector::reference(Ref(4)).to_string(), "@e4");

    let (_, mut covered) = support::mirror_on("alert");
    assert!(
        !found(&mut covered, Selector::identifier("login-submit")),
        "an element the alert covers is not on screen"
    );
    assert!(found(&mut covered, Selector::identifier("inapp-alert-ok")));
}

/// Rn-app-fixes D53 (task 6.2): a wait for an element attached, as an
/// action's selector resolves one, gives the match on screen and not
/// covered before a match the replica holds that a view covers, whatever
/// their order: on the consent screen the heading the footer covers comes
/// first in the tree, and the footer's button named alike is what a tap
/// reaches; the heading is held still.
#[test]
fn an_attached_wait_gives_the_match_on_screen_before_one_a_view_covers() {
    let screen = Screen::load("consent-ios").patched(json!([
        {"find": "placeholder-accept", "desc": "Placeholder section 3"}
    ]));
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let held = |mirror: &mut Mirror, selector: Selector| {
        mirror
            .wait(&selector, Condition::Attached, ms(10))
            .expect("the replica holds a match")
            .element
            .expect("the match")
    };
    let found = held(&mut mirror, Selector::text("Placeholder section 3"));
    assert_eq!(found.identifier.as_deref(), Some("placeholder-accept"));
    let covered = held(&mut mirror, Selector::identifier("placeholder-section-3"));
    assert_eq!(covered.name, "Placeholder section 3");
}
