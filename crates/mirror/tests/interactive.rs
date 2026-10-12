//! Task 7.3: what snapshots list. An interactive snapshot keeps only the
//! elements with refs, as agent-browser's `-i`; an actionable element with no
//! label of its own is named by its non-actionable descendants; content
//! scrolled out of view is summarized from off-screen nodes when the read has
//! them and from the protocol's scroll semantics when it does not (task
//! 3.6); a modal cuts what it covers. Lines follow agent-browser:
//! `- role "name" [attrs, ref=eN]`.

mod support;

use mirror::backend::Owner;
use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, Direction, HintSource, Mirror, MirrorConfig, MirrorError, SnapshotOptions, Target, code,
};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind, WindowChangeKind};
use serde_json::json;
use support::{DRAWN_OVER_ON_IOS, Screen, mirror_on, ms, timeline, window_event};

fn full() -> SnapshotOptions {
    SnapshotOptions::full()
}

fn interactive() -> SnapshotOptions {
    SnapshotOptions::interactive()
}

fn lines(text: &[&str]) -> String {
    text.join("\n")
}

fn rows_text(first: u32, last: u32, indent: &str) -> Vec<String> {
    (first..=last)
        .map(|n| format!("{indent}- button \"Row {n}\" [id=row-{n}, ref=e{n}]"))
        .collect()
}

#[test]
fn an_interactive_list_snapshot_lists_visible_rows_and_hints_three_below() {
    let (_, mut mirror) = mirror_on("list");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    let mut expected = rows_text(1, 16, "");
    expected.push("(3 more actionable items below, scroll down to reveal)".to_owned());
    expected.extend(
        [
            "- button \"Login\" [id=tab-login, ref=e17]",
            "- button \"List\" [selected, id=tab-list, ref=e18]",
            "- button \"Big\" [id=tab-big, ref=e19]",
            "- button \"Anim\" [id=tab-anim, ref=e20]",
            "- button \"System\" [id=tab-system, ref=e21]",
        ]
        .map(str::to_owned),
    );
    assert_eq!(snapshot.text, expected.join("\n"));
    assert_eq!(snapshot.hints.len(), 1);
    let hint = &snapshot.hints[0];
    assert_eq!(hint.direction, Direction::Below);
    assert_eq!(hint.count, Some(3));
    assert_eq!(hint.source, HintSource::OffscreenNodes);
    assert!(
        snapshot.by_identifier("row-17").is_none(),
        "rows below the fold are summarized, not listed"
    );
    assert!(snapshot.elements.iter().all(|e| e.reference.is_some()));

    let listing = mirror.snapshot(&full()).unwrap();
    let mut expected = vec![
        "- text \"List\"".to_owned(),
        "- generic [id=list-scroll]".to_owned(),
    ];
    expected.extend(rows_text(1, 16, "  "));
    expected.push("  (3 more actionable items below, scroll down to reveal)".to_owned());
    expected.extend(
        [
            "- button \"Login\" [id=tab-login, ref=e17]",
            "- button \"List\" [selected, id=tab-list, ref=e18]",
            "- button \"Big\" [id=tab-big, ref=e19]",
            "- button \"Anim\" [id=tab-anim, ref=e20]",
            "- button \"System\" [id=tab-system, ref=e21]",
            "- text \"12:30\"",
        ]
        .map(str::to_owned),
    );
    assert_eq!(listing.text, expected.join("\n"));
}

#[test]
fn scroll_semantics_give_the_hint_when_offscreen_rows_are_absent() {
    let screen = Screen::load("list-compose");
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    let mut expected = rows_text(1, 16, "");
    expected.push("(184 more items below, scroll down to reveal)".to_owned());
    assert_eq!(snapshot.text, expected.join("\n"));
    assert_eq!(snapshot.hints[0].source, HintSource::ScrollSemantics);
    assert_eq!(snapshot.hints[0].count, Some(184));

    let near_end = screen.patched(json!([{"find": "row-list", "scroll": {
        "axis": "vertical", "back": true, "forward": true,
        "range": ["items", 0, 199], "position": [181, 196]
    }}]));
    let target = FakeTarget::new(near_end.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    let mut expected = vec!["(181 more items above, scroll up to reveal)".to_owned()];
    expected.extend(rows_text(1, 16, ""));
    expected.push("(3 more items below, scroll down to reveal)".to_owned());
    assert_eq!(snapshot.text, expected.join("\n"));
    let directions: Vec<_> = snapshot
        .hints
        .iter()
        .map(|h| (h.direction, h.count))
        .collect();
    assert_eq!(
        directions,
        [(Direction::Above, Some(181)), (Direction::Below, Some(3))]
    );
}

/// Mid-list a Compose list names no row index: its range gives no count,
/// and the ways it can scroll say where more lies. A position the platform
/// reports as a percentage says it too.
#[test]
fn scroll_semantics_without_an_item_position_say_which_ways_more_lies() {
    let screen = Screen::load("list-compose");
    let mid = screen.patched(json!([{"find": "row-list", "scroll": {
        "axis": "vertical", "back": true, "forward": true, "range": ["items", 0, 199]
    }}]));
    let target = FakeTarget::new(mid.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    let mut expected = vec!["(more items above, scroll up to reveal)".to_owned()];
    expected.extend(rows_text(1, 16, ""));
    expected.push("(more items below, scroll down to reveal)".to_owned());
    assert_eq!(snapshot.text, expected.join("\n"));
    let hints: Vec<_> = snapshot
        .hints
        .iter()
        .map(|h| (h.direction, h.count, h.source))
        .collect();
    assert_eq!(
        hints,
        [
            (Direction::Above, None, HintSource::ScrollSemantics),
            (Direction::Below, None, HintSource::ScrollSemantics)
        ]
    );

    let at_end = screen.patched(json!([{"find": "row-list", "scroll": {
        "axis": "vertical", "range": ["percent", 0, 100], "position": [100, 100]
    }}]));
    let target = FakeTarget::new(at_end.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    let mut expected = vec!["(more items above, scroll up to reveal)".to_owned()];
    expected.extend(rows_text(1, 16, ""));
    assert_eq!(snapshot.text, expected.join("\n"));
}

#[test]
fn an_actionable_container_takes_its_name_from_non_actionable_descendants() {
    let (_, mut mirror) = mirror_on("login");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- textbox \"Username\" [id=login-username, ref=e1]",
            "- textbox \"Password\" [id=login-password, ref=e2]",
            "- button \"Sign in\" [id=login-submit, ref=e3]",
            "- button \"Login\" [selected, id=tab-login, ref=e4]",
            "- button \"List\" [id=tab-list, ref=e5]",
            "- button \"Big\" [id=tab-big, ref=e6]",
            "- button \"Anim\" [id=tab-anim, ref=e7]",
            "- button \"System\" [id=tab-system, ref=e8]",
        ])
    );
    let full_listing = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        full_listing
            .elements
            .iter()
            .filter(|e| e.name == "Sign in")
            .count(),
        1,
        "the label that names the button is not listed again"
    );

    let (_, mut compose) = mirror_on("compose-anim");
    let snapshot = compose.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- switch \"Animate\" [checked=true, id=anim-toggle, ref=e1]",
            "- button \"Login\" [id=tab-login, ref=e2]",
            "- button \"List\" [id=tab-list, ref=e3]",
            "- button \"Big\" [id=tab-big, ref=e4]",
            "- button \"Anim\" [selected, id=tab-anim, ref=e5]",
            "- button \"System\" [id=tab-system, ref=e6]",
        ]),
        "the role and label that Material keeps on child nodes name the tagged container"
    );
}

#[test]
fn a_stale_ref_names_the_element_it_was_issued_for() {
    let screen = Screen::load("login");
    let gone = screen.patched(json!([{"remove": "login-submit"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(gone.read()));
    target.at(
        ms(100),
        Step::Event(EventPayload::Tree(TreeChange {
            change: TreeChangeKind::CONTENT,
            window: 1,
            path: vec![0],
        })),
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&interactive()).unwrap();
    let submit = before
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();
    target.advance(ms(150));
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(submit),
            hold: None,
        })
        .unwrap_err();
    assert!(error.to_string().contains("button \"Sign in\""), "{error}");
    let after = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(after.removed_refs, [submit]);
    assert!(
        mirror
            .snapshot(&interactive())
            .unwrap()
            .removed_refs
            .is_empty()
    );
}

#[test]
fn an_alert_cuts_the_content_it_covers() {
    let (_, mut mirror) = mirror_on("alert");
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- alert \"In-app alert\"",
            "  - text \"In-app alert\"",
            "  - text \"This alert is presented by the app.\"",
            "  - button \"OK\" [id=inapp-alert-ok, ref=e1]",
        ]),
        "the covered screen and tab bar are cut; the nested OK button is folded into its parent"
    );
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        "- button \"OK\" [id=inapp-alert-ok, ref=e1]"
    );
    assert_eq!(snapshot.foreground, Owner::Target);
}

/// `value` without the identifiers that equal `id`, as React Native's alert
/// gives its OK button none.
fn without_identifier(value: &mut serde_json::Value, id: &str) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("id").is_some_and(|value| value == id) {
                map.remove("id");
            }
            map.values_mut().for_each(|v| without_identifier(v, id));
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| without_identifier(v, id)),
        _ => {}
    }
}

/// Physical-ios, task 9.5 (design D26): UIKit's alert controller holds its
/// title and message in a scroll view that its scroll indicators follow, as
/// XCTest lists them, each twice, their accessibility frames 30-point hit
/// areas along the view's edges, the horizontal one wholly over the
/// message. The alert lays out its own views and draws none over its text,
/// so the message is listed once under the alert, after its title: as the
/// Simulator's reader reads the SwiftUI fixture's alert on an iPhone 17
/// (`alert-message-ios`), as the runner reads it on the SE, where the
/// message fits (`alert-message-se`), and as React Native's alert, whose OK
/// button has no identifier, on either. The listings of task 6.1 had the
/// title and the button and not the message, on both platforms.
#[test]
fn an_alerts_message_is_listed_once_under_it_on_the_simulator_and_the_phone() {
    for (fixture, pages) in [
        ("alert-message-ios", "2 pages"),
        ("alert-message-se", "1 page"),
    ] {
        for identified in [true, false] {
            let mut screen = Screen::load(fixture);
            if !identified {
                without_identifier(&mut screen.0, "inapp-alert-ok");
            }
            let target = FakeTarget::new(screen.read());
            let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
            let ok = if identified {
                "  - button \"OK\" [id=inapp-alert-ok, ref=e1]"
            } else {
                "  - button \"OK\" [ref=e1]"
            };
            let vertical = format!("  - text \"Vertical scroll bar, {pages}\": 0%");
            assert_eq!(
                mirror.snapshot(&full()).unwrap().text,
                lines(&[
                    "- alert \"In-app alert\"",
                    "  - text \"In-app alert\"",
                    "  - text \"This alert is presented by the app.\"",
                    &vertical,
                    "  - text \"Horizontal scroll bar, 1 page\": 0%",
                    ok,
                    "  - text \"Vertical scroll bar, 1 page\": 0%",
                    "  - text \"Horizontal scroll bar, 1 page\": 0%",
                ]),
                "{fixture}, the OK button identified: {identified}"
            );
            assert_eq!(
                mirror.snapshot(&interactive()).unwrap().text,
                ok.trim_start(),
                "{fixture}: the screen under the alert is cut"
            );
        }
    }
}

/// A window the snapshot marks modal, an Android dialog here, cuts the
/// application windows below it, and names itself when an action aims at
/// what it covers.
#[test]
fn a_modal_window_cuts_the_application_windows_below_it() {
    let screen = Screen::load("login");
    let dialog = screen.patched(json!([{"add_window": {
        "id": 7, "kind": "application", "layer": 2, "package": "dev.mobileagent.fixture.expo",
        "title": "In-app alert", "bounds": [10, 300, 398, 560], "flags": ["active", "focused"],
        "modal": true,
        "root": {"class": "android.widget.FrameLayout", "bounds": [10, 300, 398, 560], "children": [
            {"class": "android.widget.TextView", "text": "This alert is presented by the app.", "bounds": [26, 320, 382, 360]},
            {"class": "android.widget.Button", "text": "OK", "bounds": [300, 500, 382, 548], "flags": ["clickable", "focusable"]}
        ]}
    }}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(dialog.read()));
    target.at(ms(100), window_event(WindowChangeKind::ADDED, 7));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&interactive()).unwrap();
    let submit = before
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();

    mirror.run_until(target.instant(ms(200))).unwrap();
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert!(snapshot.window(7).unwrap().modal);
    assert!(!snapshot.window(1).unwrap().modal);
    assert!(
        snapshot.text.starts_with("- button \"OK\" [ref="),
        "{}",
        snapshot.text
    );
    assert!(
        snapshot.by_identifier("login-submit").is_none(),
        "the dialog covers the login screen: {}",
        snapshot.text
    );
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(submit),
            hold: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED);
    assert!(
        error.to_string().contains("dialog \"In-app alert\""),
        "{error}"
    );
}

#[test]
fn acting_on_an_element_an_alert_covers_fails_with_its_code() {
    let screen = Screen::load("login-ios");
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(Screen::load("alert").read()));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(120), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&interactive()).unwrap();
    let submit = before
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();

    mirror.run_until(target.instant(ms(600))).unwrap();
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(submit),
            hold: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED);
    assert!(
        error.to_string().contains("alert \"In-app alert\""),
        "{error}"
    );
    assert_eq!(
        mirror.resolve(submit).unwrap().identifier.as_deref(),
        Some("login-submit"),
        "the covered element keeps its ref"
    );
    assert!(target.inputs().is_empty());
}

/// Task 10.22: a view the app marks modal inside its window, as Bluesky
/// marks its typeahead, hides what it covers: the Explore page's controls
/// under it get no refs, and the page's scroll hint goes with them. Its
/// siblings outside it stay: the header, a sibling only because React
/// Native flattens the views between them, keeps the search field and
/// Cancel search, and the tab bar lies in another branch, wholly under the
/// keyboard, which counts it (rn-app-fixes, design D38).
#[test]
fn a_view_marked_modal_hides_what_it_covers_and_keeps_what_lies_outside_it() {
    let (_, mut mirror) = mirror_on("bluesky-typeahead");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    for covered in [
        "Placeholder explore action",
        "Placeholder trending topic",
        "Placeholder section action",
        "Placeholder feed 1",
        "Placeholder feed 4",
    ] {
        assert!(
            snapshot.by_name(covered).is_none(),
            "{covered} lies under the typeahead:\n{}",
            snapshot.text
        );
    }
    assert!(snapshot.hints.is_empty(), "{:?}", snapshot.hints);
    for kept in ["Open drawer menu", "Cancel search", "Search for “nobel”"] {
        assert!(
            snapshot.by_name(kept).is_some(),
            "{kept}:\n{}",
            snapshot.text
        );
    }
    for kept in [
        "searchScreenInput",
        "searchTextInputClearBtn",
        "searchAutoCompleteResult-placeholder-1",
        "searchAutoCompleteResult-placeholder-4",
    ] {
        assert!(
            snapshot.by_identifier(kept).is_some(),
            "{kept}:\n{}",
            snapshot.text
        );
    }
    assert_eq!(
        snapshot.refs().len(),
        9,
        "4 in the header, 5 in the typeahead:\n{}",
        snapshot.text
    );
    assert!(
        snapshot
            .text
            .ends_with("\n(5 more actionable items under the keyboard, close it to reveal)"),
        "the tab bar under the keyboard:\n{}",
        snapshot.text
    );
}

/// The Bluesky typeahead fixture without the typeahead, the view it marks
/// modal. Unmarked, the typeahead would still cover the Explore page:
/// XCTest lists it after the page, which it is drawn over (rn-app-fixes,
/// design D31).
fn without_modal_views(screen: &Screen) -> Screen {
    fn prune(node: &mut serde_json::Value) {
        if let Some(children) = node
            .get_mut("children")
            .and_then(serde_json::Value::as_array_mut)
        {
            children.retain(|child| {
                !child["flags"]
                    .as_array()
                    .is_some_and(|flags| flags.iter().any(|flag| flag == "modal"))
            });
            children.iter_mut().for_each(prune);
        }
    }
    let mut value = screen.0.clone();
    for window in value["windows"].as_array_mut().expect("windows") {
        prune(&mut window["root"]);
    }
    Screen(value)
}

#[test]
fn acting_on_an_element_a_view_marked_modal_covers_fails_with_its_code() {
    let screen = Screen::load("bluesky-typeahead");
    let target = FakeTarget::new(without_modal_views(&screen).read());
    target.at(ms(100), Step::Show(screen.read()));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(120), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&interactive()).unwrap();
    let feed = before
        .by_identifier("placeholder-feed-1")
        .expect("listed while nothing covers it")
        .reference
        .unwrap();

    mirror.run_until(target.instant(ms(600))).unwrap();
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(feed),
            hold: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED);
    assert!(error.to_string().contains("modal view"), "{error}");
    assert_eq!(
        mirror.resolve(feed).unwrap().identifier.as_deref(),
        Some("placeholder-feed-1"),
        "the covered element keeps its ref"
    );
    assert!(target.inputs().is_empty());
}

/// Task 10.28: on Android a view drawn over its siblings hides what it
/// covers, as Bluesky draws its typeahead over the Explore page. The two
/// have the same bounds and the platform lists the typeahead first, so only
/// the order they are drawn in tells, which the agent passes on as the
/// `on_top` flag. The page's controls under the typeahead get no refs, and
/// the page's scroll hint goes with them; the header beside it and the tab
/// bar drawn over the screen's bottom stay. Gboard's floating toolbar lies
/// wholly over the fourth account's avatar, which it counts, and over part
/// of the rows beside it, which say how much of them shows (rn-app-fixes,
/// design D38).
#[test]
fn a_view_drawn_over_its_siblings_on_android_hides_what_it_covers() {
    let (_, mut mirror) = mirror_on("bluesky-typeahead-android");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    for covered in [
        "Placeholder explore action",
        "Placeholder interests action",
        "Placeholder section action",
        "Placeholder topic 1",
        "Placeholder topic 5",
    ] {
        assert!(
            snapshot.by_name(covered).is_none(),
            "{covered} lies under the typeahead:\n{}",
            snapshot.text
        );
    }
    assert!(snapshot.hints.is_empty(), "{:?}", snapshot.hints);
    for kept in ["Open drawer menu", "Cancel search", "Search for “nobel”"] {
        assert!(
            snapshot.by_name(kept).is_some(),
            "{kept}:\n{}",
            snapshot.text
        );
    }
    for kept in [
        "searchScreenInput",
        "searchTextInputClearBtn",
        "searchAutoCompleteResult-placeholder-1",
        "searchAutoCompleteResult-placeholder-4",
        "bottomBarHomeBtn",
        "bottomBarProfileBtn",
    ] {
        assert!(
            snapshot.by_identifier(kept).is_some(),
            "{kept}:\n{}",
            snapshot.text
        );
    }
    assert_eq!(
        snapshot.refs().len(),
        17,
        "4 in the header, 8 in the typeahead, 5 in the tab bar:\n{}",
        snapshot.text
    );
    let profile = snapshot
        .by_identifier("searchAutoCompleteResult-placeholder-4")
        .unwrap();
    assert_eq!(profile.beside_keyboard, Some(86));
    assert!(
        snapshot
            .text
            .ends_with("\n(1 more actionable item under the keyboard, close it to reveal)"),
        "the avatar under the toolbar:\n{}",
        snapshot.text
    );
}

/// The Bluesky typeahead fixture on Android without its typeahead.
fn without_typeahead(screen: &Screen) -> Screen {
    fn prune(node: &mut serde_json::Value) {
        if let Some(children) = node
            .get_mut("children")
            .and_then(serde_json::Value::as_array_mut)
        {
            children.retain(|child| child["class"] != "android.widget.AbsListView");
            children.iter_mut().for_each(prune);
        }
    }
    let mut value = screen.0.clone();
    for window in value["windows"].as_array_mut().expect("windows") {
        prune(&mut window["root"]);
    }
    Screen(value)
}

/// The refusal names what a tap at the topic row's centre would land on
/// (rn-app-fixes, design D37): the typeahead's row there, a button.
#[test]
fn acting_on_an_element_a_view_drawn_over_it_covers_fails_with_its_code() {
    let screen = Screen::load("bluesky-typeahead-android");
    let target = FakeTarget::new(without_typeahead(&screen).read());
    target.at(ms(100), Step::Show(screen.read()));
    target.at(
        ms(100),
        Step::Event(EventPayload::Tree(TreeChange {
            change: TreeChangeKind::CONTENT,
            window: 1,
            path: vec![0],
        })),
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&interactive()).unwrap();
    let topic = before
        .by_identifier("placeholder-topic-1")
        .expect("listed while nothing covers it")
        .reference
        .unwrap();

    mirror.run_until(target.instant(ms(200))).unwrap();
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(topic),
            hold: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED);
    assert!(
        error
            .to_string()
            .contains("lies under the button \"View placeholder-4's profile\""),
        "{error}"
    );
    assert_eq!(
        mirror.resolve(topic).unwrap().identifier.as_deref(),
        Some("placeholder-topic-1"),
        "the covered element keeps its ref"
    );
    assert!(target.inputs().is_empty());
}

/// With the typeahead gone the tab bar, drawn over the screen's bottom,
/// hides what lies wholly under it and nothing more: the topic row it
/// overlaps stays, and so do the rows scrolled out of the page, which
/// Android gives bounds clipped to nothing, in the page's hint.
#[test]
fn a_bar_drawn_over_a_page_hides_what_lies_wholly_under_it_and_leaves_the_page_its_hint() {
    let screen = without_typeahead(&Screen::load("bluesky-typeahead-android"));
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert!(
        snapshot
            .by_name("Placeholder action under the tab bar")
            .is_none(),
        "{}",
        snapshot.text
    );
    for kept in ["Placeholder explore action", "Placeholder topic 5"] {
        assert!(
            snapshot.by_name(kept).is_some(),
            "{kept}:\n{}",
            snapshot.text
        );
    }
    let hints: Vec<_> = snapshot
        .hints
        .iter()
        .map(|h| (h.direction, h.count))
        .collect();
    assert_eq!(hints, [(Direction::Below, Some(2))], "{}", snapshot.text);
}

/// A view drawn over its siblings that shows nothing, neither a ref nor a
/// name, as the decoration Bluesky draws over a video, hides nothing.
#[test]
fn a_view_drawn_over_its_siblings_that_shows_nothing_hides_nothing() {
    let (_, mut mirror) = mirror_on_json(json!([
        {"class": "android.view.ViewGroup", "bounds": [70, 460, 324, 912], "children": [
            {"class": "android.widget.Button", "desc": "Play video", "bounds": [70, 460, 324, 912], "flags": ["clickable"]},
            {"class": "android.view.ViewGroup", "bounds": [70, 460, 324, 912], "flags": ["on_top"]}
        ]}
    ]));
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        "- button \"Play video\" [ref=e1]"
    );
}

/// Rn-app-fixes D31, scenario "Heading under a fixed footer on iOS": XCTest
/// lists a view's subviews in the order UIKit draws them and reports no
/// occlusion, so on the consent screen only the order and the bounds say
/// that the footer, drawn after the scroll view, lies over the heading
/// scrolled inside Accept and continue's bounds. The heading is left out;
/// the link above the footer and the footer's buttons stay, and so do the
/// scroll view's hints.
#[test]
fn a_footer_drawn_after_a_heading_on_ios_hides_it() {
    let (_, mut mirror) = mirror_on("consent-ios");
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&[
            "- button \"Placeholder back\" [ref=e1]",
            "- heading \"Placeholder consent\" [ref=e2]",
            "(more items above, scroll up to reveal)",
            "- heading \"Placeholder section 2\" [ref=e3]",
            "- link \"Placeholder policy\" [id=placeholder-policy, ref=e4]",
            "(more items below, scroll down to reveal)",
            "- button \"Accept and continue\" [id=placeholder-accept, ref=e5]",
            "- button \"Placeholder decline\" [ref=e6]",
        ])
    );
}

/// D31: on the create screen a floating button drawn after the list lies
/// over its last row, which is left out; the row it only overlaps stays.
#[test]
fn a_floating_button_drawn_after_a_list_on_ios_hides_its_last_row() {
    let (_, mut mirror) = mirror_on("create-ios");
    let mut expected = vec![
        "- button \"Placeholder close\" [ref=e1]".to_owned(),
        "- heading \"Placeholder create\" [ref=e2]".to_owned(),
        "(more items above, scroll up to reveal)".to_owned(),
    ];
    expected.extend((1..=9).map(|n| {
        format!(
            "- button \"Placeholder option {n}\" [id=placeholder-option-{n}, ref=e{}]",
            n + 2
        )
    }));
    expected
        .push("- button \"Placeholder generate\" [id=placeholder-generate, ref=e12]".to_owned());
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        expected.join("\n")
    );
}

/// D31, scenario "Card under a sticky header": on the feed the header,
/// drawn after the list, lies over the post scrolled under it, which is
/// left out and not counted in the list's hints; the post that only
/// touches the header's edge stays.
#[test]
fn a_sticky_header_drawn_after_a_list_on_ios_hides_the_post_under_it() {
    let (_, mut mirror) = mirror_on("feed-ios");
    let mut expected = vec!["(1 more actionable item above, scroll up to reveal)".to_owned()];
    expected.extend((3..=8).map(|n| {
        format!(
            "- button \"Placeholder post {n}\" [id=placeholder-post-{n}, ref=e{}]",
            n - 2
        )
    }));
    expected.extend(
        [
            "(more items below, scroll down to reveal)",
            "- heading \"Placeholder feed\" [ref=e7]",
            "- button \"Placeholder compose\" [ref=e8]",
        ]
        .map(str::to_owned),
    );
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        expected.join("\n")
    );
}

/// D31: a ref taken while nothing covered its element is refused once a
/// view drawn after it covers it, naming the view, and no tap goes out: a
/// tap on the consent screen's heading would accept the consent. The
/// refusal does not advise acting on what covers it.
#[test]
fn acting_on_what_a_view_drawn_after_it_covers_on_ios_fails_naming_the_view() {
    for (fixture, covered, cover, named) in DRAWN_OVER_ON_IOS {
        let screen = Screen::load(fixture);
        let target = FakeTarget::new(screen.patched(json!([{"remove": cover}])).read());
        target.at(ms(100), Step::Show(screen.read()));
        target.at(ms(100), Step::Frame(1));
        target.at(ms(120), Step::Frame(2));
        let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
        let before = mirror.snapshot(&interactive()).unwrap();
        let reference = before
            .by_identifier(covered)
            .unwrap_or_else(|| panic!("{fixture}: {covered} is listed while nothing covers it"))
            .reference
            .unwrap();

        mirror.run_until(target.instant(ms(600))).unwrap();
        let error = mirror
            .act(&Action::Tap {
                target: Target::Ref(reference),
                hold: None,
            })
            .expect_err(fixture);
        assert_eq!(error.code(), code::ELEMENT_COVERED, "{fixture}: {error}");
        let message = error.to_string();
        assert!(message.contains(named), "{fixture}: {message}");
        assert!(
            !message.contains(&format!("act on the {named}")),
            "{fixture}: {message}"
        );
        assert_eq!(
            mirror.resolve(reference).unwrap().identifier.as_deref(),
            Some(covered),
            "{fixture}: the covered element keeps its ref"
        );
        assert!(target.inputs().is_empty(), "{fixture}");
    }
}

/// Rn-app-fixes D38, scenario "Control behind a header listed first" (task
/// 3.1; section 3 of the masked listings): XCTest lists a React Native
/// header before the list it is drawn over and reports whole frames, so
/// the rows the list scrolled up past its top lie in the header's frame,
/// out of the list's bounds: the acceptance run's 44-point row at 2 points
/// stayed listed, with the two headings below it, counted among the six
/// above the list. A plain view listed before a list, across its top and
/// as wide, is a header the app pins: it covers what lies in its frame.
/// The row and the headings behind it are left out and not counted;
/// the header's own controls stay.
#[test]
fn a_header_listed_before_its_list_on_ios_hides_the_rows_scrolled_up_behind_it() {
    let (_, mut mirror) = mirror_on("header-top-ios");
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&[
            "- button \"placeholder-button-1\" [ref=e1]",
            "- heading \"placeholder-heading-1\" [ref=e2]",
            "(3 more actionable items above, scroll up to reveal)",
            "- button \"placeholder-button-3\" [ref=e3]",
            "- button \"placeholder-button-4\" [ref=e4]",
            "- heading \"placeholder-heading-4\" [ref=e5]",
            "- generic \"placeholder-generic-1\" [ref=e6]",
            "- textbox \"placeholder-textbox-1\" [focused, id=placeholder-id-2, ref=e7]: placeholder-value-1",
            "- button \"placeholder-button-5\" [visible=70%, id=placeholder-id-3, ref=e8]",
        ])
    );
}

/// D38: a navigation bar listed before the content it is drawn over covers
/// what lies in its frame as a pinned header does, whether or not the
/// content scrolls: the header of the list with the keyboard up as a
/// navigation bar over content that does not scroll.
#[test]
fn a_navigation_bar_listed_before_the_content_on_ios_hides_what_lies_in_it() {
    let mut screen = Screen::load("header-top-ios");
    let content = &mut screen.0["windows"][1]["root"]["children"][0]["children"][0]["children"];
    content[0]["class"] = json!("XCUIElementTypeNavigationBar");
    content[1]["class"] = json!("RCTViewComponentView");
    content[1].as_object_mut().unwrap().remove("scroll");
    content[1]["flags"] = json!([]);
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    for behind in [
        "placeholder-button-2",
        "placeholder-heading-2",
        "placeholder-heading-3",
    ] {
        assert!(
            snapshot.by_name(behind).is_none(),
            "{behind}:\n{}",
            snapshot.text
        );
    }
    assert!(
        snapshot.by_name("placeholder-button-3").is_some(),
        "{}",
        snapshot.text
    );
}

/// D38: UIKit hit-tests a disabled control as an enabled one, and the touch
/// goes nowhere, so a disabled view drawn after a control covers it
/// whatever it shows: the creation tab's primary button (section 1b of the
/// masked listings) as an unlabelled, disabled view holding an unlabelled
/// image, which shows neither a ref nor a name, over the paste control
/// moved up wholly within it. The paste control is left out, and a tap on
/// it is refused naming the disabled control; the button beside it, only
/// partly under the view, stays.
#[test]
fn a_disabled_view_drawn_after_a_control_on_ios_hides_it_whatever_it_shows() {
    let screen = Screen::load("create-disabled-ios");
    let silent = screen.patched(json!([
        {"find": "placeholder-id-8", "bounds": [205, 712, 382, 760]},
        {"find": "placeholder-id-9", "class": "RCTViewComponentView", "desc": null, "off": ["clickable"],
         "children": [{"class": "XCUIElementTypeImage", "bounds": [181, 716, 221, 756]}]}
    ]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(silent.read()));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(120), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let paste = mirror
        .snapshot(&interactive())
        .unwrap()
        .by_identifier("placeholder-id-8")
        .expect("listed before")
        .reference
        .unwrap();

    mirror.run_until(target.instant(ms(600))).unwrap();
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert!(
        snapshot.by_identifier("placeholder-id-8").is_none(),
        "{}",
        snapshot.text
    );
    assert!(
        snapshot.by_name("placeholder-button-19").is_some(),
        "{}",
        snapshot.text
    );
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(paste),
            hold: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED, "{error}");
    assert!(error.to_string().contains("disabled control"), "{error}");
    assert!(target.inputs().is_empty());
}

/// A tap on `reference`: where it landed, or why it was refused.
fn tap_on(mirror: &mut Mirror, reference: mirror::Ref) -> Result<mirror::Point, MirrorError> {
    mirror
        .act(&Action::Tap {
            target: Target::Ref(reference),
            hold: None,
        })
        .map(|outcome| outcome.at.expect("a tap lands at a point"))
}

/// Rn-app-fixes D37, scenario "Heading half under a footer button" (task
/// 3.1; section 1a of the masked listings): the consent screen's ninth
/// heading, 22 points tall, lies with its lower 11 points under the
/// secondary footer button, which declines the consent. The acceptance
/// run's tap at its centre landed on that button's top edge and declined
/// the consent. A tap goes to the centre of what of the heading shows, its
/// upper half; with 6 points of it left above the button, fewer than 8, it
/// is refused naming the button, and nothing is tapped. The listing is the
/// screen's as the acceptance run read it, the half-covered heading and the
/// texts that straddle the buttons still in it.
#[test]
fn a_tap_on_a_heading_half_under_a_footer_button_lands_on_the_half_that_shows() {
    let screen = Screen::load("consent-footer-ios");
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let listing = mirror.snapshot(&full()).unwrap();
    assert_eq!(listing.text.lines().count(), 31, "{}", listing.text);
    for kept in ["placeholder-text-6", "placeholder-text-8", "\u{e900}"] {
        assert!(listing.by_name(kept).is_some(), "{kept}:\n{}", listing.text);
    }
    let heading = listing
        .by_name("placeholder-heading-9")
        .unwrap()
        .reference
        .unwrap();
    let at = tap_on(&mut mirror, heading).unwrap();
    // The heading spans 33 to 90.33 across and 709 to 731 down, the button
    // from 720 down.
    assert!(
        (at.x - (33.0 + 90.33) / 2.0).abs() < 0.01 && at.y == 714.5,
        "{at}"
    );

    let mut lower = screen.clone();
    lower.0["windows"][0]["root"]["children"][0]["children"][0]["children"][0]["children"][21]["bounds"] =
        json!([33, 714, 90.33, 736]);
    let target = FakeTarget::new(lower.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let heading = mirror
        .snapshot(&interactive())
        .unwrap()
        .by_name("placeholder-heading-9")
        .unwrap()
        .reference
        .unwrap();
    let error = tap_on(&mut mirror, heading).unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED, "{error}");
    assert!(
        error
            .to_string()
            .contains("button \"placeholder-button-1\""),
        "{error}"
    );
    assert!(target.inputs().is_empty());
}

/// D37, scenario "Card partly under a sticky header" (task 3.1; section 2
/// of the masked listings): on the feed the card at 41 points, 116 tall,
/// lies 81 points above its list's top, where the header listed before the
/// list is drawn, and the acceptance run's tap at its centre landed on the
/// header's heading. A tap goes to the centre of the 30% of the card in
/// view. So it does when the list runs under the header, drawn after it
/// over the list's top.
#[test]
fn a_tap_on_a_card_partly_under_a_sticky_header_lands_on_the_part_in_view() {
    let screen = Screen::load("feed-sticky-ios");
    let mut drawn_over = screen.clone();
    let content = &mut drawn_over.0["windows"][0]["root"]["children"][0]["children"][0]["children"];
    let header = content[0].clone();
    content[1]["bounds"] = json!([0, 0, 402, 791]);
    let list = content[1].clone();
    content[0] = list;
    content[1] = header;
    for screen in [screen, drawn_over] {
        let target = FakeTarget::new(screen.read());
        let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
        let card = mirror
            .snapshot(&interactive())
            .unwrap()
            .by_identifier("placeholder-id-4")
            .expect("the card is listed")
            .reference
            .unwrap();
        let at = tap_on(&mut mirror, card).unwrap();
        assert_eq!((at.x, at.y), (201.0, 139.5), "{at}");
    }
}

/// The creation tab as the second acceptance run read it (section 1b of the
/// masked listings): its list ends above the tab bar, which no footer
/// holds, so nothing but the disabled button lies over the paste control.
fn creation_open() -> Screen {
    let mut screen = Screen::load("create-disabled-ios");
    let content = &mut screen.0["windows"][0]["root"]["children"][0]["children"][0]["children"];
    content[1]["bounds"] = json!([0, 122, 402, 791]);
    content[3] = content[3]["children"][0].clone();
    screen
}

/// D37 and D38 (task 3.1; section 1b of the masked listings): on the
/// creation tab the paste control lies 38 of its 48 points under the
/// disabled primary button drawn after the tab's list, and the acceptance
/// run's taps at its centre landed on the disabled button, twice, doing
/// nothing. With the list ending above the tab bar, as that run read it,
/// the control stays listed, and a tap goes to the centre of its lower 10
/// points, below the button.
#[test]
fn a_tap_on_a_control_under_a_disabled_button_lands_on_the_part_below_it() {
    let target = FakeTarget::new(creation_open().read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    // The section's 23 lines, and the tab bar's first tab, a tab beside the
    // selected one (design D40).
    assert_eq!(snapshot.text.lines().count(), 24, "{}", snapshot.text);
    let paste = snapshot
        .by_identifier("placeholder-id-8")
        .expect("listed, partly under the button")
        .reference
        .unwrap();
    let at = tap_on(&mut mirror, paste).unwrap();
    assert_eq!((at.x, at.y), (293.5, 769.0), "{at}");
    assert_eq!(target.inputs().len(), 1);
}

/// D38, scenario "Control behind a header listed first": a ref taken while
/// the row lay below the header is refused once the list has scrolled it
/// up behind the header, naming the header as a tap there would land on
/// it, a plain view, and nothing is tapped.
#[test]
fn acting_on_a_row_behind_a_header_listed_before_its_list_fails_naming_the_header() {
    let screen = Screen::load("header-top-ios");
    let mut below = screen.clone();
    below.0["windows"][1]["root"]["children"][0]["children"][0]["children"][1]["children"][4]["bounds"] =
        json!([38, 126, 364, 170]);
    let target = FakeTarget::new(below.read());
    target.at(ms(100), Step::Show(screen.read()));
    target.at(ms(100), Step::Frame(1));
    target.at(ms(120), Step::Frame(2));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let row = mirror
        .snapshot(&interactive())
        .unwrap()
        .by_identifier("placeholder-id-1")
        .expect("listed below the header")
        .reference
        .unwrap();

    mirror.run_until(target.instant(ms(600))).unwrap();
    let error = tap_on(&mut mirror, row).unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED, "{error}");
    assert!(error.to_string().contains("overlay"), "{error}");
    assert!(target.inputs().is_empty());
}

/// Rn-app-fixes D45, scenario "Strip clipped by the scroll view" (task 4.3;
/// task 3.7, N6): on the creation tab as the third acceptance run met it,
/// the lower 10 points of the paste control and of the button beside it lie
/// below the disabled primary button, under the footer drawn over the
/// list's lower part, outside what the list shows. Neither keeps a part a
/// tap reaches, so the interactive listing leaves both out, the list's hint
/// counting them below, and a tap on the paste control by the ref the full
/// listing gives it is refused naming the disabled button, which a tap at
/// its centre would hit, rather than tapping the strip, which opened
/// nothing in the run.
#[test]
fn a_control_whose_strip_below_a_floating_button_lies_under_a_footer_is_left_out_and_refused() {
    let (target, mut mirror) = mirror_on("create-disabled-ios");
    let listing = mirror.snapshot(&interactive()).unwrap();
    for left in ["placeholder-button-19", "placeholder-button-20"] {
        assert!(listing.by_name(left).is_none(), "{left}:\n{}", listing.text);
    }
    assert!(
        listing
            .text
            .contains("- heading \"placeholder-heading-24\" [ref=e17]\n(2 more actionable items below, scroll down to reveal)\n- button \"placeholder-button-21\""),
        "{}",
        listing.text
    );
    let paste = mirror
        .snapshot(&full())
        .unwrap()
        .by_identifier("placeholder-id-8")
        .expect("in the full listing")
        .reference
        .unwrap();
    let error = tap_on(&mut mirror, paste).unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED, "{error}");
    assert!(
        error
            .to_string()
            .contains("button \"placeholder-button-21\""),
        "{error}"
    );
    assert!(target.inputs().is_empty());
}

/// The organizations list of section 3 of the masked listings as the third
/// acceptance run met it (task 3.7, N7): the header is listed after the
/// view that holds the list rather than before the list, so it pins
/// nothing, and the rows the list has scrolled up behind it lie outside the
/// list's bounds, out of view rather than covered.
fn header_after_its_list() -> Screen {
    let mut screen = Screen::load("header-top-ios");
    let content = &mut screen.0["windows"][1]["root"]["children"][0]["children"][0]["children"];
    let (header, list) = (content[0].clone(), content[1].clone());
    *content = json!([
        {"class": "RCTViewComponentView", "bounds": [0, 0, 402, 874], "children": [list]},
        header,
    ]);
    screen
}

/// Rn-app-fixes D45, scenario "Partly covered elements marked" (task 4.3;
/// task 3.7, N7): the third acceptance run's interactive listings gave no
/// sign of the consent screen's heading half under a footer button or of
/// the feed's card 70% under the header, and listed a card scrolled above
/// the list's top and the rows behind the organizations list's header,
/// which a tap reaches none of. The interactive listing marks each element
/// a tap reaches part of with the share it reaches, and leaves out each one
/// it reaches nothing of, which the list's hint counts, whatever covers it:
/// here the header, the status bar's place above it and a list's edge. The
/// keyboard's marks stay as they were (see the keyboard's tests).
#[test]
fn partly_covered_elements_say_how_much_shows_and_those_a_tap_cannot_reach_are_counted() {
    let (_, mut consent) = mirror_on("consent-footer-ios");
    let text = consent.snapshot(&interactive()).unwrap().text;
    assert!(
        text.contains("\n- heading \"placeholder-heading-9\" [visible=50%, ref=e9]\n"),
        "{text}"
    );

    let (_, mut feed) = mirror_on("feed-sticky-ios");
    assert_eq!(
        feed.snapshot(&interactive()).unwrap().text,
        lines(&[
            "- button \"placeholder-button-1\" [id=placeholder-id-1, ref=e1]",
            "- heading \"placeholder-heading-1\" [ref=e2]",
            "- button \"placeholder-button-2\" [id=placeholder-id-2, ref=e3]: placeholder-value-1",
            "(13 more actionable items above, scroll up to reveal)",
            "- button \"placeholder-button-4\" [visible=30%, id=placeholder-id-4, ref=e4]",
            "- button \"placeholder-button-5\" [id=placeholder-id-5, ref=e5]",
            "- button \"placeholder-button-6\" [id=placeholder-id-6, ref=e6]",
            "- button \"placeholder-button-7\" [id=placeholder-id-7, ref=e7]",
            "(more items below, scroll down to reveal)",
            "- tab \"placeholder-heading-1\" [selected, ref=e8]",
            "- tab \"placeholder-generic-1\" [ref=e9]",
            "- tab \"placeholder-generic-2\" [ref=e10]",
        ])
    );

    let target = FakeTarget::new(header_after_its_list().read());
    let mut organizations = Mirror::new(target.backend(), MirrorConfig::default());
    let listing = organizations.snapshot(&interactive()).unwrap();
    for behind in [
        "placeholder-button-2",
        "placeholder-heading-2",
        "placeholder-heading-3",
    ] {
        assert!(
            listing.by_name(behind).is_none(),
            "{behind}:\n{}",
            listing.text
        );
    }
    assert!(
        listing
            .text
            .starts_with("(6 more actionable items above, scroll up to reveal)\n"),
        "{}",
        listing.text
    );
}

/// Rn-app-fixes D38, scenario "Elements under the keyboard" (task 3.2;
/// section 5 of the masked listings): the acceptance run's interactive
/// listings kept what lay under the keyboard, and its clicks went to the
/// centres of submit buttons 1 to 35% in view. With the keyboard up an
/// element wholly under it is left out of the interactive listing, the
/// hint counting it under the keyboard, and one partly under it is listed
/// with the share of it in view; the full listing keeps both.
#[test]
fn what_lies_under_the_keyboard_is_left_out_and_what_lies_partly_under_it_says_how_much_shows() {
    let screen = Screen::load("form-keyboard-ios");
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        lines(&[
            "(more items above, scroll up to reveal)",
            "- heading \"placeholder-heading-1\" [ref=e1]",
            "- generic \"placeholder-generic-1\" [ref=e2]",
            "- textbox \"placeholder-textbox-1\" [focused, id=placeholder-id-1, ref=e3]: placeholder-value-1",
            "- button \"placeholder-button-1\" [visible=8%, id=placeholder-id-2, ref=e4]",
            "(2 more actionable items under the keyboard, close it to reveal)",
        ])
    );
    let listing = mirror.snapshot(&full()).unwrap();
    for kept in ["placeholder-link-1", "placeholder-link-2"] {
        assert!(listing.by_name(kept).is_some(), "{kept}:\n{}", listing.text);
    }
    assert!(listing.text.ends_with("\n- keyboard"), "{}", listing.text);

    let under =
        screen.patched(json!([{"find": "placeholder-id-2", "bounds": [20, 560, 382, 608]}]));
    let target = FakeTarget::new(under.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let text = mirror.snapshot(&interactive()).unwrap().text;
    assert!(
        !text.contains("placeholder-button-1")
            && text.ends_with("(3 more actionable items under the keyboard, close it to reveal)"),
        "{text}"
    );
}

/// D37 and D38 (task 3.2; section 5 of the masked listings): a tap on a
/// submit button partly under the keyboard goes to the part of it in view:
/// 17 of its 48 points above the keyboard's top, 35%, take the tap at their
/// centre, where the acceptance run's tap at the button's centre hit the
/// keyboard; 3.67 points, 8%, are too few, and the tap is refused naming
/// the keyboard, which a tap at the button's centre would land on; a
/// button wholly under the keyboard is out of view.
#[test]
fn a_tap_on_a_button_partly_under_the_keyboard_lands_on_what_shows_or_is_refused_naming_it() {
    let screen = Screen::load("form-keyboard-ios");
    let submit_in = |screen: &Screen| {
        let target = FakeTarget::new(screen.read());
        let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
        let submit = mirror
            .snapshot(&full())
            .unwrap()
            .by_identifier("placeholder-id-2")
            .expect("the submit button")
            .reference
            .unwrap();
        (target, mirror, submit)
    };
    let third =
        screen.patched(json!([{"find": "placeholder-id-2", "bounds": [20, 522, 382, 570]}]));
    let (_, mut mirror, submit) = submit_in(&third);
    let at = tap_on(&mut mirror, submit).unwrap();
    assert_eq!((at.x, at.y), (201.0, 530.5), "{at}");

    let (target, mut mirror, submit) = submit_in(&screen);
    let error = tap_on(&mut mirror, submit).unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_COVERED, "{error}");
    assert!(error.to_string().contains("under the keyboard"), "{error}");
    assert!(target.inputs().is_empty());

    let under =
        screen.patched(json!([{"find": "placeholder-id-2", "bounds": [20, 560, 382, 608]}]));
    let (target, mut mirror, submit) = submit_in(&under);
    let error = tap_on(&mut mirror, submit).unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_OFFSCREEN, "{error}");
    assert!(target.inputs().is_empty());
}

/// Rn-app-fixes D35: XCTest reports a React Native text as a text whose
/// only child is its own text, a heading likewise without its identifier,
/// and UIKit's scroll bars as elements whose only child repeats them. The
/// full listing shows each once, and the heading's repeat gets no ref.
#[test]
fn a_text_a_scroll_bar_and_a_heading_that_repeat_themselves_are_listed_once() {
    let (_, mut mirror) = mirror_on("members-ios");
    assert_eq!(
        mirror.snapshot(&full()).unwrap().text,
        lines(&[
            "- button \"Placeholder back\" [ref=e1]",
            "- heading \"Placeholder members\" [id=placeholder-members-title, ref=e2]",
            "- text \"Placeholder 2 selected\"",
            "- generic [id=placeholder-members-list]",
            "  - checkbox \"Placeholder member 1\" [checked=true, ref=e3]",
            "    - text \"Placeholder member 1\"",
            "  - checkbox \"Placeholder member 2\" [checked=true, ref=e4]",
            "    - text \"Placeholder member 2\"",
            "  - checkbox \"Placeholder member 3\" [checked=false, ref=e5]",
            "    - text \"Placeholder member 3\"",
            "  - checkbox \"Placeholder member 4\" [checked=false, ref=e6]",
            "  - text \"Vertical scroll bar, 1 page\": 0%",
            "  - text \"Horizontal scroll bar, 1 page\": 0%",
            "- button \"Placeholder add members\" [ref=e7]",
        ])
    );
    let (_, mut tabs) = mirror_on("tabbar-ios");
    let listing = tabs.snapshot(&full()).unwrap().text;
    assert_eq!(
        listing
            .lines()
            .filter(|line| line.trim_start() == "- text \"Placeholder tab 2\"")
            .count(),
        1,
        "{listing}"
    );
}

#[test]
fn a_system_alert_reports_system_ui_as_the_foreground() {
    let (_, mut mirror) = mirror_on("system-alert");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        snapshot.foreground,
        Owner::System("com.apple.springboard".to_owned())
    );
    assert_eq!(
        snapshot.text,
        lines(&[
            "- button \"Allow Once\" [ref=e1]",
            "- button \"Allow While Using App\" [ref=e2]",
            "- button \"Don’t Allow\" [ref=e3]",
        ])
    );
    let listing = mirror.snapshot(&full()).unwrap();
    assert!(
        !listing.text.contains("12:30"),
        "the alert covers SpringBoard's own content"
    );
}

#[test]
fn the_full_listing_follows_agent_browser_lines() {
    let tl = timeline("events-typing");
    let target = tl.target();
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let snapshot = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        snapshot.text,
        lines(&[
            "- text \"Login\"",
            "- textbox \"Username\" [id=login-username, ref=e1]",
            "- textbox \"Password\" [id=login-password, ref=e2]",
            "- button \"Sign in\" [id=login-submit, ref=e3]",
            "- text \"Signed out\" [id=login-status]",
            "- button \"Login\" [selected, id=tab-login, ref=e4]",
            "- button \"List\" [id=tab-list, ref=e5]",
            "- button \"Big\" [id=tab-big, ref=e6]",
            "- button \"Anim\" [id=tab-anim, ref=e7]",
            "- button \"System\" [id=tab-system, ref=e8]",
            "- text \"12:30\"",
        ])
    );
    mirror.run_until(target.instant(ms(530))).unwrap();
    let typing = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(
        typing.text.lines().take(2).collect::<Vec<_>>(),
        [
            "- textbox \"Username\" [id=login-username, ref=e1]: alice",
            "- textbox \"Password\" [focused, id=login-password, ref=e2]: •••••••",
        ]
    );

    let (_, mut odd) = mirror_on_json(json!([
        {"class": "android.widget.Button", "text": "Say \"hi\"\nnow", "bounds": [16, 16, 392, 64], "flags": ["clickable"]},
        {"class": "android.widget.CheckBox", "text": "Agree\u{00a0}to\u{200b} terms", "bounds": [16, 80, 392, 128], "flags": ["clickable", "checkable"]},
        {"class": "android.widget.Button", "text": "Pay", "bounds": [16, 144, 392, 192], "flags": ["clickable"], "off": ["enabled"]},
        {"class": "android.widget.EditText", "id": "a b", "desc": "Note", "text": "x", "bounds": [16, 208, 392, 256], "flags": ["clickable", "editable"]}
    ]));
    assert_eq!(
        odd.snapshot(&interactive()).unwrap().text,
        lines(&[
            "- button \"Say \\\"hi\\\"\\nnow\" [ref=e1]",
            "- checkbox \"Agree to terms\" [checked=false, ref=e2]",
            "- button \"Pay\" [disabled, ref=e3]",
            "- textbox \"Note\" [id=\"a b\", ref=e4]: x",
        ])
    );
}

/// A mirror over one application window holding `children` under its root.
fn mirror_on_json(children: serde_json::Value) -> (FakeTarget, Mirror) {
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

#[test]
fn an_empty_listing_says_so_as_agent_browser_does() {
    let (_, mut mirror) = mirror_on_json(json!([
        {"class": "android.widget.TextView", "text": "Nothing to do", "bounds": [16, 16, 392, 64]}
    ]));
    assert_eq!(
        mirror.snapshot(&interactive()).unwrap().text,
        "(no interactive elements)"
    );
    let (_, mut empty) = mirror_on_json(json!([]));
    assert_eq!(empty.snapshot(&full()).unwrap().text, "(empty screen)");
}

#[test]
fn the_keyboard_window_is_listed_as_one_line() {
    let screen = Screen::load("login").patched(json!([{"add_window": {
        "id": 9, "kind": "input_method", "layer": 2, "package": "com.google.android.inputmethod.latin",
        "title": "Keyboard", "bounds": [0, 600, 408, 912],
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 600, 408, 912], "children": [
            {"class": "android.widget.Button", "desc": "q", "bounds": [0, 640, 40, 690], "flags": ["clickable"]},
            {"class": "android.widget.Button", "desc": "w", "bounds": [40, 640, 80, 690], "flags": ["clickable"]}
        ]}
    }}]));
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let listing = mirror.snapshot(&full()).unwrap();
    assert_eq!(
        listing.text.lines().filter(|l| *l == "- keyboard").count(),
        1
    );
    assert!(!listing.text.contains("\"q\""), "{}", listing.text);
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    // Keys get no refs, and the tab bar wholly under the keyboard is
    // counted under it rather than listed (rn-app-fixes, design D38).
    assert_eq!(snapshot.refs().len(), 3, "{}", snapshot.text);
    assert!(
        snapshot
            .text
            .ends_with("\n(5 more actionable items under the keyboard, close it to reveal)"),
        "{}",
        snapshot.text
    );
}

#[test]
fn every_interactive_line_carries_a_ref() {
    let (_, mut mirror) = mirror_on("grid");
    let snapshot = mirror.snapshot(&interactive()).unwrap();
    assert_eq!(snapshot.elements.len(), 505, "500 cells and 5 tabs");
    for line in snapshot.text.lines() {
        assert!(line.contains("ref=e"), "{line}");
    }
    assert!(
        snapshot
            .text
            .starts_with("- button \"Cell 1\" [id=cell-1, ref=e1]\n"),
        "{}",
        &snapshot.text[..80]
    );
    let listing = mirror.snapshot(&full()).unwrap();
    assert!(
        listing
            .text
            .starts_with("- text \"500 elements\" [id=big-count]\n")
    );
}

#[test]
fn acting_on_a_row_scrolled_out_of_view_fails_with_its_code() {
    let list = |scrolled: i64| {
        Screen::from_json(json!({
            "backend": {"name": "android-agent"},
            "scale": 2.625,
            "windows": [{
                "id": 1, "kind": "application", "layer": 1, "bounds": [0, 0, 408, 912], "flags": ["active"],
                "root": {"class": "android.widget.FrameLayout", "bounds": [0, 0, 408, 912], "children": [
                    {"class": "android.widget.ScrollView", "bounds": [0, 72, 408, 840], "flags": ["scrollable"], "children": [
                        {"repeat": [1, 19], "viewport": [0, 72, 408, 840],
                         "layout": {"rows": {"left": 0, "top": 72, "right": 408, "height": 48}, "offset": [0, -scrolled]},
                         "node": {"class": "android.view.ViewGroup", "id": "row-{n}", "desc": "Row {n}", "flags": ["clickable"]}}
                    ]}
                ]}
            }]
        }))
    };
    let target = FakeTarget::new(list(0).read());
    target.at(ms(100), Step::Show(list(48).read()));
    target.at(
        ms(100),
        Step::Event(EventPayload::Tree(TreeChange {
            change: TreeChangeKind::SCROLL,
            window: 1,
            path: vec![0],
        })),
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let before = mirror.snapshot(&interactive()).unwrap();
    let first = before.by_identifier("row-1").unwrap().reference.unwrap();
    mirror.run_until(target.instant(ms(200))).unwrap();
    let after = mirror.snapshot(&interactive()).unwrap();
    assert!(
        after
            .text
            .starts_with("(1 more actionable item above, scroll up to reveal)\n")
    );
    let error = mirror
        .act(&Action::Tap {
            target: Target::Ref(first),
            hold: None,
        })
        .unwrap_err();
    assert_eq!(error.code(), code::ELEMENT_OFFSCREEN);
    assert!(matches!(error, MirrorError::Offscreen { .. }));
    assert!(target.inputs().is_empty());
}
