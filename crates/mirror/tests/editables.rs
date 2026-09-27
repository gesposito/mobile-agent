//! Task 7.11: editable fields nested in labelled containers, the shape of
//! Ignite's sign-in fields in the React Native smoke test
//! (`reports/10.4-rn-smoke.md`), where the interactive listing showed one
//! element against agent-device's seven. A field without a label of its own
//! takes the label of the container it sits in, and the container is not
//! listed again; the covered-content, off-screen and collision rules hold
//! for such fields as for any other element.

mod support;

use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, Degradation, Direction, HintSource, Mirror, MirrorConfig, MirrorError, SnapshotOptions,
    Target, code,
};
use serde_json::{Value, json};
use support::{Screen, mirror_on, ms};

fn lines(text: &[&str]) -> String {
    text.join("\n")
}

fn interactive() -> SnapshotOptions {
    SnapshotOptions::interactive()
}

fn full() -> SnapshotOptions {
    SnapshotOptions::full()
}

const SIGN_IN: [&str; 4] = [
    "- textbox \"Email\" [ref=e1]",
    "- textbox \"Password\" [ref=e2]",
    "- button \"Show password\" [ref=e3]",
    "- button \"Tap to log in!\" [ref=e4]",
];

const SIGN_IN_FULL: [&str; 6] = [
    "- text \"Log In\"",
    "- text \"Enter your details below to unlock top secret info.\"",
    "- textbox \"Email\" [ref=e1]",
    "- textbox \"Password\" [ref=e2]",
    "- button \"Show password\" [ref=e3]",
    "- button \"Tap to log in!\" [ref=e4]",
];

#[test]
fn a_field_in_a_labelled_container_takes_the_containers_label_on_android() {
    let (_, mut mirror) = mirror_on("signin-android");
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&SIGN_IN)
    );
    assert_eq!(
        mirror.snapshot(&full()).unwrap().text,
        lines(&SIGN_IN_FULL),
        "the containers that name the fields are not listed again"
    );
}

#[test]
fn a_field_in_a_labelled_container_takes_the_containers_label_on_ios() {
    let (_, mut mirror) = mirror_on("signin-ios");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(snapshot.text, lines(&SIGN_IN));
    assert!(!snapshot.verdict.degraded, "{:?}", snapshot.verdict);
    assert_eq!(mirror.snapshot(&full()).unwrap().text, lines(&SIGN_IN_FULL));
}

/// A mirror over one Android application window holding `children`.
fn android(children: Value) -> (FakeTarget, Mirror) {
    let screen = Screen::from_json(json!({
        "backend": {"name": "android-agent"},
        "scale": 2.625,
        "windows": [{
            "id": 1, "kind": "application", "layer": 1, "bounds": [0, 0, 408, 912], "flags": ["active"],
            "root": {"class": "android.widget.FrameLayout", "bounds": [0, 0, 408, 912], "children": children}
        }]
    }));
    let target = FakeTarget::new(screen.read());
    let mirror = Mirror::new(target.backend(), MirrorConfig::default());
    (target, mirror)
}

fn field(bounds: [i32; 4]) -> Value {
    json!({"class": "android.widget.EditText", "bounds": bounds, "flags": ["clickable", "focusable", "editable"]})
}

#[test]
fn a_container_names_only_its_single_unlabelled_field() {
    let (_, mut mirror) = android(json!([
        {"class": "android.view.ViewGroup", "desc": "Phone", "bounds": [16, 344, 392, 424], "children": [
            field([16, 376, 392, 424])
        ]},
        {"class": "android.view.ViewGroup", "desc": "Account", "bounds": [16, 16, 392, 96], "children": [
            {"class": "android.widget.EditText", "desc": "Username", "bounds": [16, 48, 392, 96],
             "flags": ["clickable", "focusable", "editable"]}
        ]},
        {"class": "android.view.ViewGroup", "desc": "Sign in", "bounds": [16, 112, 392, 256], "children": [
            field([16, 144, 392, 192]),
            field([16, 208, 392, 256])
        ]},
        {"class": "android.view.ViewGroup", "desc": "Search", "bounds": [16, 272, 392, 336],
         "flags": ["clickable"], "children": [field([16, 288, 392, 336])]}
    ]));
    assert_eq!(
        mirror.snapshot(&full()).unwrap().text,
        lines(&[
            "- textbox \"Phone\" [ref=e1]",
            "- generic \"Account\"",
            "  - textbox \"Username\" [ref=e2]",
            "- generic \"Sign in\"",
            "  - textbox [ref=e3]",
            "  - textbox [ref=e4]",
            "- button \"Search\" [ref=e5]",
            "  - textbox [ref=e6]",
        ]),
        "a container names its one field; a field keeps its own label; a container \
         of two fields names neither; an actionable container is an element of its own"
    );
}

#[test]
fn a_named_field_under_an_alert_is_covered_and_its_ref_says_which_field() {
    let screen = Screen::load("signin-ios");
    let mut alerted = screen.clone();
    alerted.0["windows"][0]["root"]["children"][0]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!(
            {"class": "_UIAlertControllerPhoneTVMacView", "desc": "Signed out", "bounds": [61, 360, 341, 514],
             "children": [
                {"class": "XCUIElementTypeStaticText", "desc": "Signed out", "bounds": [77, 380, 325, 402]},
                {"class": "XCUIElementTypeButton", "desc": "OK", "bounds": [61, 470, 341, 514]}
             ]}
        ));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(alerted.read()));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(120), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&interactive()).unwrap();
    let email = before.by_name("Email").unwrap().reference.unwrap();

    mirror.run_until(target.instant(ms(600))).unwrap();
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        "- button \"OK\" [ref=e5]",
        "the alert covers the fields"
    );
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(email),
            hold: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED);
    assert!(target.inputs().is_empty());

    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(Screen::load("login-ios").read()));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(120), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let email = mirror
        .snapshot(&interactive())
        .unwrap()
        .by_name("Email")
        .unwrap()
        .reference
        .unwrap();
    mirror.run_until(target.instant(ms(600))).unwrap();
    let stale = mirror.resolve(email).unwrap_err();
    assert!(matches!(stale, MirrorError::StaleRef { .. }));
    assert!(stale.to_string().contains("textbox \"Email\""), "{stale}");
}

#[test]
fn fields_below_the_fold_are_hinted_once_each() {
    let (_, mut mirror) = android(json!([
        {"class": "android.widget.ScrollView", "bounds": [0, 72, 408, 840], "flags": ["scrollable"], "children": [
            {"repeat": [1, 6], "viewport": [0, 72, 408, 840],
             "layout": {"rows": {"left": 0, "top": 72, "right": 408, "height": 192}},
             "node": {"class": "android.view.ViewGroup", "desc": "Field {n}", "flags": ["focusable"], "children": [
                 {"class": "android.widget.EditText", "bounds": "fill", "flags": ["clickable", "focusable", "editable"]}
             ]}}
        ]}
    ]));
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- textbox \"Field 1\" [ref=e1]",
            "- textbox \"Field 2\" [ref=e2]",
            "- textbox \"Field 3\" [ref=e3]",
            "- textbox \"Field 4\" [ref=e4]",
            "(2 more actionable items below, scroll down to reveal)",
        ])
    );
    assert_eq!(snapshot.hints.len(), 1);
    assert_eq!(snapshot.hints[0].direction, Direction::Below);
    assert_eq!(snapshot.hints[0].source, HintSource::OffscreenNodes);
}

#[test]
fn fields_in_identically_labelled_containers_collide() {
    let (_, mut mirror) = android(json!([
        {"class": "android.view.ViewGroup", "desc": "Email", "bounds": [16, 16, 392, 96], "children": [
            field([16, 48, 392, 96])
        ]},
        {"class": "android.view.ViewGroup", "desc": "Email", "bounds": [16, 112, 392, 192], "children": [
            field([16, 144, 392, 192])
        ]}
    ]));
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- textbox \"Email\" [ref=e1]",
            "- textbox \"Email\" [ref=e2]"
        ])
    );
    assert_eq!(
        snapshot.verdict.reasons,
        [Degradation::SiblingCollision {
            refs: snapshot.refs()
        }],
        "nothing but their position tells the two fields apart"
    );
}

#[test]
fn a_labelled_container_whose_field_the_platform_hides_gets_a_ref() {
    let (target, mut mirror) = mirror_on("signin-ios-hidden");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- generic \"Email\" [ref=e1]",
            "- generic \"Password\" [ref=e2]",
            "- button \"Tap to log in!\" [ref=e3]",
        ]),
        "the containers agent-device lists as cells get refs"
    );
    assert_eq!(
        mirror.snapshot(&full()).unwrap().text,
        lines(&[
            "- text \"Log In\"",
            "- text \"Enter your details below to unlock top secret info.\"",
            "- generic \"Email\" [ref=e1]",
            "- generic \"Password\" [ref=e2]",
            "- button \"Tap to log in!\" [ref=e3]",
        ])
    );
    let email = snapshot.by_name("Email").unwrap();
    let outcome = mirror
        .act(&Action::Tap {
            target: Target::Ref(email.reference.unwrap()),
            hold: None,
        })
        .unwrap();
    assert_eq!(
        outcome.at,
        Some(email.bounds.center()),
        "a tap lands on the container, over the hidden field"
    );
    assert_eq!(target.inputs().len(), 1);
}

#[test]
fn only_labelled_containers_with_nothing_but_static_text_below_them_get_such_refs() {
    let (_, mut mirror) = android(json!([
        {"class": "android.view.View", "desc": "Promo", "bounds": [16, 16, 392, 64]},
        {"class": "android.view.View", "bounds": [16, 80, 392, 128]},
        {"class": "android.widget.TextView", "text": "Hello", "bounds": [16, 144, 392, 192]},
        {"class": "android.widget.ImageView", "desc": "Logo", "bounds": [16, 208, 392, 256]},
        {"class": "android.view.ViewGroup", "desc": "Section", "bounds": [16, 272, 392, 400], "children": [
            {"class": "android.widget.TextView", "text": "Inside", "bounds": [16, 288, 392, 336]}
        ]},
        {"class": "android.view.ViewGroup", "desc": "Banner", "bounds": [16, 416, 392, 528], "children": [
            {"class": "android.widget.TextView", "text": "Sale", "bounds": [16, 416, 392, 464]},
            {"class": "android.widget.ImageView", "desc": "Sale banner", "bounds": [16, 464, 392, 528]}
        ]}
    ]));
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&[
            "- generic \"Promo\" [ref=e1]",
            "- generic \"Section\" [ref=e2]",
        ]),
        "not an unlabelled leaf, a text, an image or a labelled container with more than static text"
    );

    let (_, mut root) = mirror_on("sparse");
    assert_eq!(
        root.snapshot(&full()).unwrap().text,
        "(empty screen)",
        "a window's root is no such container"
    );
}
