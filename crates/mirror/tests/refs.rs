//! Task 7.2: refs. An element keeps its ref while it survives from read to
//! read, identity coming from its window, class, identifier and sibling path;
//! numbers only grow; a ref whose element is gone is refused as stale with
//! its own code; siblings that only their position tells apart make the
//! snapshot degraded. Coordinates are device points both ways.

mod support;

use mirror::backend::{Input, NodeHandle};
use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, Degradation, Mirror, MirrorConfig, MirrorError, Ref, SnapshotOptions, StaleCause,
    Target, code,
};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind};
use protocol::message::TextMethod;
use serde_json::{Value, json};
use support::{Screen, mirror_on, ms, timeline};

fn full() -> SnapshotOptions {
    SnapshotOptions::full()
}

fn tap(reference: Ref) -> Action {
    Action::Tap {
        target: Target::Ref(reference),
        hold: None,
    }
}

/// A content change somewhere in window 1.
fn content_changed() -> Step {
    Step::Event(EventPayload::Tree(TreeChange {
        change: TreeChangeKind::CONTENT,
        window: 1,
        path: vec![0],
    }))
}

/// A recycler list whose rows carry no identifier: rows `first` to `last`,
/// 48 points high, moved up by `scrolled` points.
fn rows(first: u64, last: u64, scrolled: i64) -> Screen {
    Screen::from_json(json!({
        "backend": {"name": "android-agent"},
        "scale": 2.625,
        "windows": [{
            "id": 1, "kind": "application", "layer": 1, "bounds": [0, 0, 408, 912], "flags": ["active"],
            "root": {"class": "android.widget.FrameLayout", "bounds": [0, 0, 408, 912], "children": [
                {"class": "androidx.recyclerview.widget.RecyclerView", "bounds": [0, 0, 408, 480], "flags": ["scrollable"], "children": [
                    {"repeat": [first, last],
                     "layout": {"rows": {"left": 0, "top": 0, "right": 408, "height": 48}, "offset": [0, -scrolled]},
                     "node": {"class": "android.view.ViewGroup", "desc": "Row {n}", "flags": ["clickable", "focusable"]}}
                ]}
            ]}
        }]
    }))
}

#[test]
fn a_surviving_button_keeps_its_ref_when_the_screen_changes_elsewhere() {
    let screen = Screen::load("login");
    let changed = screen.patched(json!([
        {"find": "login-status", "text": "Invalid credentials"},
        {"insert_before": "login-submit", "node":
            {"class": "android.widget.TextView", "text": "Wrong password", "bounds": [16, 216, 392, 232]}}
    ]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(changed.read()));
    target.at(ms(100), content_changed());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());

    let before = mirror.snapshot(&full()).unwrap();
    let submit = before.by_identifier("login-submit").unwrap().reference;
    assert!(submit.is_some());
    mirror.run_until(target.instant(ms(200))).unwrap();
    let after = mirror.snapshot(&full()).unwrap();

    assert_eq!(
        after.by_identifier("login-submit").unwrap().reference,
        submit
    );
    for id in [
        "login-username",
        "login-password",
        "tab-login",
        "tab-system",
    ] {
        assert_eq!(
            after.by_identifier(id).unwrap().reference,
            before.by_identifier(id).unwrap().reference,
            "{id} keeps its ref"
        );
    }
    assert_eq!(before.refs(), after.refs(), "no ref appears or disappears");
    assert_eq!(
        mirror
            .resolve(submit.unwrap())
            .unwrap()
            .identifier
            .as_deref(),
        Some("login-submit")
    );
}

#[test]
fn acting_on_a_removed_element_fails_with_a_stale_ref_error_and_guidance() {
    let screen = Screen::load("login");
    let gone = screen.patched(json!([{"remove": "login-submit"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(gone.read()));
    target.at(ms(100), content_changed());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    let submit = before
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();

    target.advance(ms(150));
    let error = mirror.act(&tap(submit)).unwrap_err();
    assert_eq!(error.code(), code::STALE_REF);
    assert!(!error.retryable());
    assert!(matches!(
        &error,
        MirrorError::StaleRef { reference, cause: StaleCause::Removed, .. } if *reference == submit
    ));
    let message = error.to_string();
    assert!(
        message.starts_with(&format!("{submit} is stale")),
        "{message}"
    );
    assert!(message.contains("Sign in"), "{message}");
    assert!(message.contains("new snapshot"), "{message}");
    assert!(target.inputs().is_empty(), "nothing was tapped");
    assert!(matches!(
        mirror.resolve(submit),
        Err(MirrorError::StaleRef { .. })
    ));
}

#[test]
fn a_stale_ref_never_resolves_to_another_element() {
    let target = FakeTarget::new(rows(1, 10, 0).read());
    target.at(ms(100), Step::Show(rows(3, 12, 96).read()));
    target.at(
        ms(100),
        Step::Event(EventPayload::Tree(TreeChange {
            change: TreeChangeKind::SCROLL,
            window: 1,
            path: vec![0],
        })),
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    let row = |n: u32| {
        before
            .by_name(&format!("Row {n}"))
            .unwrap()
            .reference
            .unwrap()
    };
    let (row1, row2, row3, row10) = (row(1), row(2), row(3), row(10));

    mirror.run_until(target.instant(ms(200))).unwrap();
    let after = mirror.snapshot(&full()).unwrap();
    assert_eq!(mirror.resolve(row3).unwrap().name, "Row 3");
    assert_eq!(mirror.resolve(row10).unwrap().name, "Row 10");
    for gone in [row1, row2] {
        assert!(
            matches!(mirror.resolve(gone), Err(MirrorError::StaleRef { .. })),
            "{gone} scrolled away and must not name another row"
        );
    }
    let row11 = after.by_name("Row 11").unwrap().reference.unwrap();
    assert!(row11 > row10, "rows scrolled in get new refs");
}

#[test]
fn refs_only_increase_and_are_never_reused() {
    let screen = Screen::load("login");
    let swapped = screen.patched(json!([
        {"remove": "login-submit"},
        {"insert_before": "login-status", "node":
            {"class": "android.view.ViewGroup", "id": "login-forgot", "desc": "Forgot password?",
             "bounds": [16, 232, 392, 280], "flags": ["clickable", "focusable"]}}
    ]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(swapped.read()));
    target.at(ms(100), content_changed());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    let highest = before.refs().into_iter().max().unwrap();
    let submit = before
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();

    mirror.run_until(target.instant(ms(200))).unwrap();
    let after = mirror.snapshot(&full()).unwrap();
    let forgot = after
        .by_identifier("login-forgot")
        .unwrap()
        .reference
        .unwrap();
    assert!(forgot > highest, "{forgot} comes after {highest}");
    assert!(!after.refs().contains(&submit), "{submit} is not reused");
    assert!(matches!(
        mirror.resolve(submit),
        Err(MirrorError::StaleRef { .. })
    ));
}

#[test]
fn typing_into_a_field_keeps_its_ref() {
    let tl = timeline("events-typing");
    let target = tl.target();
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&full()).unwrap();
    mirror.run_until(target.instant(tl.end)).unwrap();
    let after = mirror.snapshot(&full()).unwrap();
    let username = after.by_identifier("login-username").unwrap();
    assert_eq!(username.value.as_deref(), Some("alice"));
    assert!(username.reference.is_some());
    assert_eq!(
        username.reference,
        before.by_identifier("login-username").unwrap().reference
    );
    assert_eq!(before.refs(), after.refs());
}

fn delete_buttons() -> Value {
    json!({
        "backend": {"name": "android-agent"},
        "scale": 2.625,
        "windows": [{
            "id": 1, "kind": "application", "layer": 1, "bounds": [0, 0, 408, 912], "flags": ["active"],
            "root": {"class": "android.widget.LinearLayout", "bounds": [0, 0, 408, 912], "children": [
                {"class": "android.widget.Button", "text": "Delete", "bounds": [16, 16, 392, 64], "flags": ["clickable"]},
                {"class": "android.widget.Button", "text": "Delete", "bounds": [16, 80, 392, 128], "flags": ["clickable"]},
                {"class": "android.widget.Button", "text": "Delete", "bounds": [16, 144, 392, 192], "flags": ["clickable"]},
                {"class": "android.widget.Button", "text": "Add", "bounds": [16, 208, 392, 256], "flags": ["clickable"]}
            ]}
        }]
    })
}

#[test]
fn identical_siblings_are_reported_as_a_degraded_sibling_collision() {
    let target = FakeTarget::new(Screen::from_json(delete_buttons()).read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&full()).unwrap();
    let deletes: Vec<Ref> = snapshot
        .elements
        .iter()
        .filter(|e| e.name == "Delete")
        .map(|e| e.reference.unwrap())
        .collect();
    assert_eq!(deletes.len(), 3);
    let verdict = &snapshot.verdict;
    assert!(verdict.degraded);
    assert_eq!(
        verdict.reasons,
        [Degradation::SiblingCollision {
            refs: deletes.clone()
        }]
    );
    assert_eq!(verdict.reasons[0].code(), "sibling_collision");
    assert!(verdict.advice()[0].contains(&deletes[0].to_string()));

    let (_, mut clean) = mirror_on("login");
    let snapshot = clean.snapshot(&full()).unwrap();
    assert!(!snapshot.verdict.degraded);
    assert!(snapshot.verdict.reasons.is_empty());
    assert_eq!(snapshot.verdict.backend.name, "android-agent");
}

#[test]
fn tapping_the_center_of_reported_bounds_hits_the_element() {
    let (target, mut mirror) = mirror_on("login");
    let snapshot = mirror.snapshot(&full()).unwrap();
    for (id, actionable) in [
        ("login-username", true),
        ("login-password", true),
        ("login-submit", true),
        ("login-status", false),
        ("tab-big", true),
    ] {
        let element = snapshot.by_identifier(id).unwrap();
        mirror
            .act(&Action::Tap {
                target: Target::Point(element.bounds.center()),
                hold: None,
            })
            .unwrap();
        assert_eq!(target.inputs().last().unwrap().hit.as_deref(), Some(id));
        assert_eq!(element.reference.is_some(), actionable, "{id}");
        if let Some(reference) = element.reference {
            let outcome = mirror.act(&tap(reference)).unwrap();
            assert_eq!(outcome.at, Some(element.bounds.center()));
            assert_eq!(target.inputs().last().unwrap().hit.as_deref(), Some(id));
        }
    }
}

#[test]
fn refs_parse_in_every_form_agent_browser_accepts() {
    for input in ["@e5", "ref=e5", "e5", "  @e5 "] {
        assert_eq!(Ref::parse(input), Some(Ref(5)), "{input}");
    }
    assert_eq!(Ref::parse("e123"), Some(Ref(123)));
    for input in [
        "", "e", "@e", "ref=", "e0", "E5", "x5", "e5a", "@@e5", "e-1",
    ] {
        assert_eq!(Ref::parse(input), None, "{input:?}");
    }
    assert_eq!(Ref(42).to_string(), "e42");
}

#[test]
fn a_ref_that_was_never_issued_is_unknown() {
    let (target, mut mirror) = mirror_on("login");
    let snapshot = mirror.snapshot(&full()).unwrap();
    let beyond = Ref(snapshot.refs().into_iter().max().unwrap().0 + 1);
    let error = mirror.act(&tap(beyond)).unwrap_err();
    assert_eq!(error.code(), code::UNKNOWN_REF);
    assert!(error.to_string().contains("take a snapshot"), "{error}");
    assert!(matches!(
        mirror.resolve(Ref(0)),
        Err(MirrorError::UnknownRef(_))
    ));
    assert!(target.inputs().is_empty());
}

#[test]
fn text_into_a_ref_names_the_node_of_the_latest_read() {
    let (target, mut mirror) = mirror_on("login");
    let snapshot = mirror.snapshot(&full()).unwrap();
    let username = snapshot
        .by_identifier("login-username")
        .unwrap()
        .reference
        .unwrap();
    let outcome = mirror
        .act(&Action::Text {
            text: "alice".to_owned(),
            into: Some(username),
            method: TextMethod::AUTO,
        })
        .unwrap();
    assert_eq!(outcome.method, Some(TextMethod::DIRECT));
    let read = target.screen();
    let window = read.snapshot.windows.iter().find(|w| w.id == 1).unwrap();
    let node = find_id(window.root.as_ref().unwrap(), "login-username").unwrap();
    assert_eq!(
        target.inputs().last().unwrap().input,
        Input::Text {
            text: "alice".to_owned(),
            node: Some(NodeHandle { window: 1, node }),
            method: TextMethod::AUTO,
        }
    );
}

fn find_id(node: &protocol::snapshot::Node, id: &str) -> Option<u32> {
    if node.res_id.as_deref() == Some(id) {
        return Some(node.id);
    }
    node.children.iter().find_map(|c| find_id(c, id))
}
