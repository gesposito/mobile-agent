//! Task 7.12: which control a ref names, and which controls get refs, from
//! the Bluesky run of the React Native smoke test
//! (`reports/10.4-rn-smoke.md`, second run). A ref's identity includes the
//! accessible name of an element without an identifier, so a control that a
//! screen transition renames and moves gets a fresh ref, and its old ref is
//! refused as stale rather than pressing the control that took its place
//! (finding 6); one renamed in place keeps its ref (rn-app-fixes, D39).
//! A labelled node without traits whose children, if any, are static text is
//! a control and gets a ref, unless a clickable ancestor holds it, whose name
//! it then forms when the ancestor has none of its own (finding 4); the
//! covered-content, off-screen and collision rules hold for such controls as
//! for any other element.

mod support;

use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, Condition, Degradation, Direction, HintSource, Mirror, MirrorConfig, MirrorError, Ref,
    Selector, SnapshotOptions, StaleCause, Target, code,
};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind};
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

/// Rn-app-fixes D39: a control renamed in place, its role, parent and place
/// among its siblings unchanged, keeps its ref under its new name, as a
/// field whose label changes keeps it; Bluesky's transition, which also
/// moves the control, still refuses the old ref.
#[test]
fn a_control_named_by_its_text_keeps_its_ref_when_the_text_changes_in_place_on_android() {
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

    assert_eq!(mirror.resolve(Ref(1)).unwrap().name, "Next");
    mirror.act(&tap(Ref(1))).unwrap();
    assert_eq!(target.inputs().len(), 2);
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        "- button \"Next\" [ref=e1]"
    );
}

/// The paste sheet's nodes below its root view (section 7 of the masked
/// listings): the sheet's children.
fn sheet_children(screen: &mut Screen) -> &mut Value {
    &mut screen.0["windows"][0]["root"]["children"][0]["children"][0]["children"][0]["children"]
}

/// Rn-app-fixes D39, scenario "Ref across a label change" (task 3.3;
/// section 7 of the masked listings): on the paste sheet the title field,
/// without an identifier, took a new ref when it took the focus, its view
/// one of four unidentified views of a class whose content gained a
/// counter, and again once it held text and its name changed, so a fill by
/// its ref failed stale before anything was typed. A node keeps its ref,
/// and its element, across a change of its name or of what it holds while
/// its role, identifier, parent and place among its siblings are
/// unchanged: the field's ref is the same before the focus, after it, and
/// once it holds text, and resolves to the field under its new name.
#[test]
fn a_field_keeps_its_ref_when_its_name_and_its_view_change_in_place() {
    let sheet = Screen::load("sheet-ios");
    let mut focused = sheet.clone();
    let view = &mut sheet_children(&mut focused)[3];
    view["children"][0]["flags"] = json!(["editable", "focusable", "focused"]);
    view["children"].as_array_mut().unwrap().push(json!(
        {"class": "XCUIElementTypeStaticText", "desc": "placeholder-text-1", "bounds": [300, 262, 374, 268]}
    ));
    let mut typed = focused.clone();
    let field = &mut sheet_children(&mut typed)[3]["children"][0];
    field["desc"] = json!("placeholder-textbox-3");
    field["text"] = json!("placeholder-value-intended");
    let target = FakeTarget::new(sheet.read());
    target.at(ms(100), Step::Show(focused.read()));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(300), Step::Show(typed.read()));
    target.at(ms(300), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        before.text,
        lines(&[
            "- heading \"placeholder-heading-1\" [ref=e1]",
            "- button \"placeholder-button-1\" [ref=e2]",
            "- generic \"placeholder-generic-1\" [ref=e3]",
            "- textbox \"placeholder-textbox-1\" [ref=e4]",
            "- generic \"placeholder-generic-2\" [ref=e5]",
            "- textbox \"placeholder-textbox-2\" [id=placeholder-id-1, ref=e6]",
            "- button \"placeholder-button-2\" [disabled, id=placeholder-id-2, ref=e7]",
            "- button \"placeholder-button-1\" [ref=e8]",
        ])
    );
    let field = before.by_name("placeholder-textbox-1").unwrap().reference;

    mirror.run_until(target.instant(ms(250))).unwrap();
    let after_focus = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        after_focus
            .by_name("placeholder-textbox-1")
            .unwrap()
            .reference,
        field,
        "{}",
        after_focus.text
    );
    mirror.run_until(target.instant(ms(600))).unwrap();
    let after_typing = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        after_typing
            .by_name("placeholder-textbox-3")
            .unwrap()
            .reference,
        field,
        "{}",
        after_typing.text
    );
    assert!(after_typing.removed_refs.is_empty(), "{after_typing:?}");
    let resolved = mirror.resolve(field.unwrap()).unwrap();
    assert_eq!(
        resolved.value.as_deref(),
        Some("placeholder-value-intended")
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

const HOSTING: [&str; 4] = [
    "- generic \"Automatic\" [ref=e1]",
    "- generic \"Manual\" [id=manualSelectBtn, ref=e2]",
    "- button \"Done\" [ref=e3]",
    "- generic \"Dismiss\" [ref=e4]",
];

#[test]
fn labelled_controls_without_traits_on_the_hosting_provider_sheet_get_refs() {
    let (target, mut mirror) = mirror_on("bluesky-hosting");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(snapshot.text, lines(&HOSTING));
    assert_eq!(
        mirror.snapshot(&full()).unwrap().text,
        lines(&[
            "- text \"Hosting provider\"",
            "- generic \"Automatic\" [ref=e1]",
            "  - text \"Automatic\"",
            "- generic \"Manual\" [id=manualSelectBtn, ref=e2]",
            "  - text \"Manual\"",
            "- button \"Done\" [ref=e3]",
            "- generic \"Dismiss\" [ref=e4]",
        ])
    );
    let manual = snapshot.by_identifier("manualSelectBtn").unwrap();
    let outcome = mirror.act(&tap(manual.reference.unwrap())).unwrap();
    assert_eq!(outcome.at, Some(manual.bounds.center()));
    assert_eq!(target.inputs()[0].hit.as_deref(), Some("manualSelectBtn"));
}

#[test]
fn language_rows_get_refs_and_those_below_the_fold_are_hinted() {
    let (_, mut mirror) = mirror_on("bluesky-language");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    let mut expected: Vec<String> = (1..=10)
        .map(|n| format!("- generic \"Language {n}\" [ref=e{n}]"))
        .collect();
    expected.push("(6 more actionable items below, scroll down to reveal)".to_owned());
    assert_eq!(snapshot.text, expected.join("\n"));
    assert_eq!(snapshot.hints.len(), 1);
    assert_eq!(snapshot.hints[0].direction, Direction::Below);
    assert_eq!(snapshot.hints[0].source, HintSource::OffscreenNodes);
}

#[test]
fn a_labelled_control_under_an_alert_is_covered() {
    let screen = Screen::load("bluesky-hosting");
    let mut alerted = screen.clone();
    alerted.0["windows"][0]["root"]["children"][0]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!(
            {"class": "_UIAlertControllerPhoneTVMacView", "desc": "Change provider?", "bounds": [61, 360, 341, 514],
             "children": [
                {"class": "XCUIElementTypeStaticText", "desc": "Change provider?", "bounds": [77, 380, 325, 402]},
                {"class": "XCUIElementTypeButton", "desc": "OK", "bounds": [61, 470, 341, 514]}
             ]}
        ));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(alerted.read()));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(120), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&HOSTING)
    );

    mirror.run_until(target.instant(ms(600))).unwrap();
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        "- button \"OK\" [ref=e5]"
    );
    let error = mirror.act(&tap(Ref(1))).unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED);
    assert!(target.inputs().is_empty());
}

/// Rn-app-fixes D35, scenario "Selected tab keeps its ref", and D40 (task
/// 3.4; section 6 of the masked listings): React Native exposes a tab as a
/// labelled node without traits, the selected one with the `selected`
/// trait, and the production-app run found an unselected tab holding its icon
/// glyph, a text whose only child is its own text, without a ref (0 of 2).
/// Labelled nodes without traits laid out in one row with a selected one
/// are tabs: all three get refs as tabs, the selected one with its state,
/// as UIKit's tab bar buttons do, selected or not.
#[test]
fn a_tab_gets_a_ref_whether_selected_or_not_and_the_selected_one_says_so() {
    let (_, mut mirror) = mirror_on("tabbar-ios");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- heading \"Placeholder home\" [ref=e1]",
            "- button \"Placeholder action\" [ref=e2]",
            "- tab \"Placeholder tab 1\" [id=placeholder-tab-1, ref=e3]",
            "- tab \"Placeholder tab 2\" [selected, id=placeholder-tab-2, ref=e4]",
            "- tab \"Placeholder tab 3\" [id=placeholder-tab-3, ref=e5]",
        ])
    );
    assert!(
        snapshot
            .by_identifier("placeholder-tab-2")
            .unwrap()
            .selected
    );

    let (_, mut uikit) = mirror_on("login-ios");
    let snapshot = uikit.snapshot(&interactive()).unwrap();
    for tab in ["tab-login", "tab-list", "tab-big", "tab-anim", "tab-system"] {
        let element = snapshot.by_identifier(tab).unwrap();
        assert!(element.reference.is_some(), "{tab}");
        assert_eq!(element.selected, tab == "tab-login", "{tab}");
    }
}

/// D40: a row of labelled nodes without traits none of which is selected is
/// no tab bar, and the rule changes nothing: without the middle tab's
/// `selected` trait the tabs holding their icon's text get no ref, as
/// before, and none lists as a tab.
#[test]
fn a_row_without_a_selected_node_lists_no_tabs() {
    let screen = Screen::load("tabbar-ios").patched(json!([
        {"find": "placeholder-tab-2", "off": ["selected"]}
    ]));
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&[
            "- heading \"Placeholder home\" [ref=e1]",
            "- button \"Placeholder action\" [ref=e2]",
        ])
    );
    let listing = mirror.snapshot(&full()).unwrap();
    assert!(!listing.text.contains("- tab "), "{}", listing.text);
}

/// D35: React Native says a view is a checkbox, and whether it is checked,
/// only in its value on iOS (`checkbox, checked`), where no trait says so.
/// Such a view lists as a checkbox with its state, as a switch does, and
/// gets a ref whatever it holds; the value is no longer shown.
#[test]
fn a_checkbox_react_native_announces_in_its_value_lists_as_a_checkbox_with_its_state() {
    let (_, mut mirror) = mirror_on("members-ios");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- button \"Placeholder back\" [ref=e1]",
            "- heading \"Placeholder members\" [id=placeholder-members-title, ref=e2]",
            "- checkbox \"Placeholder member 1\" [checked=true, ref=e3]",
            "- checkbox \"Placeholder member 2\" [checked=true, ref=e4]",
            "- checkbox \"Placeholder member 3\" [checked=false, ref=e5]",
            "- checkbox \"Placeholder member 4\" [checked=false, ref=e6]",
            "- button \"Placeholder add members\" [ref=e7]",
        ])
    );
    let member = snapshot.by_name("Placeholder member 1").unwrap();
    assert_eq!(member.checked, Some(true));
    assert_eq!(member.value, None);
    assert!(member.actionable);
}

/// A mirror over one Simulator application window holding `children`.
fn ios(children: Value) -> (FakeTarget, Mirror) {
    let screen = Screen::from_json(json!({
        "backend": {"name": "simulator-reader"},
        "scale": 3,
        "windows": [{
            "id": 1, "kind": "application", "layer": 0, "bounds": [0, 0, 402, 874], "flags": ["active"],
            "events": false,
            "root": {"class": "XCUIElementTypeApplication", "desc": "Bluesky", "bounds": [0, 0, 402, 874], "children": [
                {"class": "XCUIElementTypeWindow", "bounds": [0, 0, 402, 874], "children": children}
            ]}
        }]
    }));
    let target = FakeTarget::new(screen.read());
    let mirror = Mirror::new(target.backend(), MirrorConfig::default());
    (target, mirror)
}

#[test]
fn inside_a_clickable_ancestor_such_controls_name_it_and_get_no_ref() {
    let (_, mut mirror) = ios(json!([
        {"class": "XCUIElementTypeButton", "bounds": [24, 100, 378, 148], "children": [
            {"class": "XCUIElementTypeOther", "desc": "Automatic", "bounds": "fill", "children": [
                {"class": "XCUIElementTypeStaticText", "desc": "Automatic", "bounds": "fill"}
            ]}
        ]},
        {"class": "XCUIElementTypeButton", "desc": "Provider", "bounds": [24, 164, 378, 212], "children": [
            {"class": "XCUIElementTypeOther", "desc": "Manual", "bounds": [24, 164, 330, 212], "children": [
                {"class": "XCUIElementTypeStaticText", "desc": "Manual", "bounds": "fill"}
            ]},
            {"class": "XCUIElementTypeOther", "desc": "Selected", "bounds": [330, 164, 378, 212]}
        ]}
    ]));
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&[
            "- button \"Automatic\" [ref=e1]",
            "- button \"Provider\" [ref=e2]",
        ])
    );
    assert_eq!(
        mirror.snapshot(&full()).unwrap().text,
        lines(&[
            "- button \"Automatic\" [ref=e1]",
            "- button \"Provider\" [ref=e2]",
            "  - generic \"Manual\"",
            "    - text \"Manual\"",
            "  - generic \"Selected\"",
        ]),
        "an unnamed button takes its name from them; a named one keeps them as content"
    );
}

#[test]
fn identical_labelled_controls_collide() {
    let (_, mut mirror) = ios(json!([
        {"class": "XCUIElementTypeOther", "desc": "Remove", "bounds": [24, 100, 378, 148], "children": [
            {"class": "XCUIElementTypeStaticText", "desc": "Remove", "bounds": "fill"}
        ]},
        {"class": "XCUIElementTypeOther", "desc": "Remove", "bounds": [24, 164, 378, 212], "children": [
            {"class": "XCUIElementTypeStaticText", "desc": "Remove", "bounds": "fill"}
        ]}
    ]));
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- generic \"Remove\" [ref=e1]",
            "- generic \"Remove\" [ref=e2]"
        ])
    );
    assert_eq!(
        snapshot.verdict.reasons,
        [Degradation::SiblingCollision {
            refs: snapshot.refs()
        }]
    );
}

#[test]
fn nodes_with_more_than_static_text_below_them_or_no_label_are_no_such_controls() {
    let (_, mut mirror) = ios(json!([
        {"class": "XCUIElementTypeOther", "desc": "Card", "bounds": [24, 100, 378, 196], "children": [
            {"class": "XCUIElementTypeStaticText", "desc": "Title", "bounds": [24, 100, 378, 148]},
            {"class": "XCUIElementTypeImage", "desc": "Avatar", "bounds": [24, 148, 72, 196]}
        ]},
        {"class": "XCUIElementTypeOther", "desc": "Section", "bounds": [24, 212, 378, 260], "children": [
            {"class": "XCUIElementTypeOther", "bounds": "fill", "children": [
                {"class": "XCUIElementTypeStaticText", "desc": "Nested", "bounds": "fill"}
            ]}
        ]},
        {"class": "XCUIElementTypeOther", "bounds": [24, 276, 378, 324], "children": [
            {"class": "XCUIElementTypeStaticText", "desc": "Caption", "bounds": "fill"}
        ]},
        {"class": "XCUIElementTypeStaticText", "desc": "Plain text", "bounds": [24, 340, 378, 388]}
    ]));
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        "(no interactive elements)"
    );
}
