//! Task 7.12: which control a ref names, from the Bluesky run of the React
//! Native smoke test (`reports/10.4-rn-smoke.md`, second run). A ref's
//! identity includes the accessible name of an element without an
//! identifier, so a control that a screen transition renames gets a fresh
//! ref, and its old ref is refused as stale rather than pressing the control
//! that took its place (finding 6).

mod support;

use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, Condition, Mirror, MirrorConfig, MirrorError, Ref, Selector, SnapshotOptions,
    StaleCause, Target, code,
};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind};
use serde_json::json;
use support::{Screen, ms};

fn lines(text: &[&str]) -> String {
    text.join("\n")
}

fn interactive() -> SnapshotOptions {
    SnapshotOptions::interactive()
}

fn tap(reference: Ref) -> Action {
    Action::Tap {
        target: Target::Ref(reference),
        hold: None,
    }
}

/// A content change in window 1.
fn content_changed() -> Step {
    Step::Event(EventPayload::Tree(TreeChange {
        change: TreeChangeKind::CONTENT,
        window: 1,
        path: vec![0],
    }))
}

const SIGN_IN: [&str; 4] = [
    "- button \"Bluesky Social\" [id=selectServiceButton, ref=e1]",
    "- textbox \"Username or email address\" [id=loginUsernameInput, ref=e2]",
    "- textbox \"Password\" [id=loginPasswordInput, ref=e3]",
    "- button \"Forgot password?\" [ref=e4]",
];

/// A mirror on Bluesky's sign-in form after a tap on Forgot password?
/// (`e4`) that opened the forgot-password form 50 ms later, 300 ms before
/// the clock stands now.
fn after_forgot_password() -> (FakeTarget, Mirror) {
    let target = FakeTarget::new(Screen::load("bluesky-signin").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&SIGN_IN)
    );
    mirror.act(&tap(Ref(4))).unwrap();
    target.after(ms(50), Step::Show(Screen::load("bluesky-forgot").read()));
    target.advance(ms(300));
    (target, mirror)
}

#[test]
fn a_stale_ref_to_a_control_a_transition_renamed_is_refused_on_ios() {
    let (target, mut mirror) = after_forgot_password();
    let error = mirror.act(&tap(Ref(4))).unwrap_err();
    assert_eq!(error.code(), code::STALE_REF);
    assert!(matches!(
        &error,
        MirrorError::StaleRef {
            reference: Ref(4),
            cause: StaleCause::Removed,
            ..
        }
    ));
    assert!(
        error
            .to_string()
            .contains("the button \"Forgot password?\""),
        "{error}"
    );
    assert_eq!(target.inputs().len(), 1, "Next was not pressed");
    assert!(matches!(
        mirror.resolve(Ref(4)),
        Err(MirrorError::StaleRef { .. })
    ));
}

#[test]
fn the_control_that_took_its_place_gets_a_fresh_ref_and_the_old_one_stays_stale() {
    let (_, mut mirror) = after_forgot_password();
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- button \"Bluesky Social\" [id=selectServiceButton, ref=e1]",
            "- textbox \"Enter your email address\" [id=forgotPasswordEmail, ref=e5]",
            "- button \"Next\" [ref=e6]",
        ])
    );
    assert_eq!(snapshot.removed_refs, [Ref(2), Ref(3), Ref(4)]);
    let stale = mirror.resolve(Ref(4)).unwrap_err();
    assert!(
        stale.to_string().contains("\"Forgot password?\""),
        "{stale}"
    );
    assert_eq!(mirror.resolve(Ref(6)).unwrap().name, "Next");
}

#[test]
fn a_wait_on_the_old_ref_sees_the_renamed_control_gone() {
    let (_, mut mirror) = after_forgot_password();
    let gone = mirror
        .wait(&Selector::reference(Ref(4)), Condition::Disappear, ms(1000))
        .unwrap();
    assert_eq!(gone.elapsed, ms(0));
    let next = mirror
        .wait(&Selector::text("Next"), Condition::Appear, ms(1000))
        .unwrap()
        .element
        .unwrap();
    assert_ne!(next.reference, Some(Ref(4)), "Next does not inherit e4");
}

/// Android's form of the same transition: a pressable named by the text
/// inside it, with no identifier.
fn android_form(title: &str, control: &str) -> Screen {
    Screen::from_json(json!({
        "backend": {"name": "android-agent"},
        "scale": 2.625,
        "windows": [{
            "id": 1, "kind": "application", "layer": 1, "bounds": [0, 0, 408, 912], "flags": ["active"],
            "root": {"class": "android.widget.FrameLayout", "bounds": [0, 0, 408, 912], "children": [
                {"class": "android.view.ViewGroup", "bounds": [0, 24, 408, 912], "children": [
                    {"class": "android.widget.TextView", "text": title, "bounds": [24, 64, 384, 104]},
                    {"class": "android.view.ViewGroup", "bounds": [24, 336, 384, 384], "flags": ["clickable", "focusable"],
                     "children": [{"class": "android.widget.TextView", "text": control, "bounds": "fill"}]}
                ]}
            ]}
        }]
    }))
}

#[test]
fn a_control_named_by_its_text_gets_a_fresh_ref_when_the_text_changes_on_android() {
    let target = FakeTarget::new(android_form("Sign in", "Forgot password?").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        "- button \"Forgot password?\" [ref=e1]"
    );
    mirror.act(&tap(Ref(1))).unwrap();
    target.after(
        ms(50),
        Step::Show(android_form("Reset password", "Next").read()),
    );
    target.after(ms(50), content_changed());
    target.advance(ms(100));

    let error = mirror.act(&tap(Ref(1))).unwrap_err();
    assert_eq!(error.code(), code::STALE_REF);
    assert!(
        error
            .to_string()
            .contains("the button \"Forgot password?\""),
        "{error}"
    );
    assert_eq!(target.inputs().len(), 1);
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        "- button \"Next\" [ref=e2]"
    );
}

#[test]
fn an_identified_control_keeps_its_ref_when_its_name_changes() {
    let screen = Screen::load("bluesky-signin");
    let custom = screen.patched(json!([{"find": "selectServiceButton", "desc": "example.com"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(custom.read()));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(120), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&SIGN_IN)
    );
    mirror.run_until(target.instant(ms(600))).unwrap();
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text.lines().next(),
        Some("- button \"example.com\" [id=selectServiceButton, ref=e1]"),
        "the identifier is the identity"
    );
    assert!(snapshot.removed_refs.is_empty());
    assert_eq!(mirror.resolve(Ref(1)).unwrap().name, "example.com");
}

#[test]
fn typing_into_a_field_without_an_identifier_keeps_its_ref() {
    let screen = Screen::load("signin-android");
    let mut typed = screen.clone();
    typed.0["windows"][0]["root"]["children"][0]["children"][2]["children"][0]["text"] =
        json!("alice@example.com");
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(typed.read()));
    target.at(ms(100), content_changed());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&interactive()).unwrap();
    let email = before.by_name("Email").unwrap().reference;
    assert!(email.is_some());

    mirror.run_until(target.instant(ms(200))).unwrap();
    let after = mirror.snapshot(&interactive()).unwrap();
    let field = after.by_name("Email").unwrap();
    assert_eq!(field.value.as_deref(), Some("alice@example.com"));
    assert_eq!(field.reference, email, "a value is not a name");
    assert_eq!(before.refs(), after.refs());
}
