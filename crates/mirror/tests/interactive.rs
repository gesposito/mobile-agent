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
/// Cancel search, and the tab bar lies in another branch.
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
        14,
        "4 in the header, 5 in the typeahead, 5 in the tab bar:\n{}",
        snapshot.text
    );
}

/// The Bluesky typeahead fixture with nothing marked modal.
fn unmarked(screen: &Screen) -> Screen {
    fn strip(node: &mut serde_json::Value) {
        if let Some(flags) = node
            .get_mut("flags")
            .and_then(serde_json::Value::as_array_mut)
        {
            flags.retain(|flag| flag != "modal");
        }
        if let Some(children) = node
            .get_mut("children")
            .and_then(serde_json::Value::as_array_mut)
        {
            children.iter_mut().for_each(strip);
        }
    }
    let mut value = screen.0.clone();
    for window in value["windows"].as_array_mut().expect("windows") {
        strip(&mut window["root"]);
    }
    Screen(value)
}

#[test]
fn acting_on_an_element_a_view_marked_modal_covers_fails_with_its_code() {
    let screen = Screen::load("bluesky-typeahead");
    let target = FakeTarget::new(unmarked(&screen).read());
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
/// bar drawn over the screen's bottom stay.
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
    for kept in [
        "Open drawer menu",
        "Cancel search",
        "Search for “nobel”",
        "Placeholder Account 4's avatar",
    ] {
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
        18,
        "4 in the header, 9 in the typeahead, 5 in the tab bar:\n{}",
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
    assert!(error.to_string().contains("overlay"), "{error}");
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
    assert_eq!(snapshot.refs().len(), 8, "keys get no refs");
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
