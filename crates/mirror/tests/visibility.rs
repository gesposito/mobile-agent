//! Task 10.15: how much of an element is in view. A scroll that looks for
//! an element stops once the element is mostly in view, so the mirror tells
//! the share of an element's bounds inside the visible bounds of what holds
//! it (its window, clipped by its scroll containers, less the bars drawn
//! over them) and whether its bounds end at an edge where a container can
//! still scroll: Android reports bounds clipped to their ancestors, so a
//! row a list cuts off reads as the sliver that shows.
//!
//! Task 10.16: how far a stroke may carry the content before an element
//! passes through the view unseen. The mirror tells the visible bounds of
//! the scroll container a stroke through a point moves, as an element
//! inside it sees them.
//!
//! Task 10.32: where a stroke may come down. The mirror tells what of an
//! area the system and keyboard windows above it leave uncovered.

mod support;

use std::time::Duration;

use mirror::testing::{FakeTarget, Step};
use mirror::{
    Condition, Direction, ElementId, Mirror, MirrorConfig, Point, Rect, Selector, SnapshotOptions,
    Uncovered, Visibility,
};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind};
use serde_json::{Value, json};
use support::{Screen, ms};

/// The rows of a list at `[left, top, right, bottom]`, as wide as the list
/// and 48 points tall, the first at `first_top`, cut to the list as Android
/// reports them when `clipped`, and whole as iOS reports them otherwise;
/// rows wholly outside it are left out, as a virtualized list leaves them.
fn rows(class: &str, first_top: f64, count: u32, list: [f64; 4], clipped: bool) -> Vec<Value> {
    let [left, top, right, bottom] = list;
    (1..=count)
        .filter_map(|n| {
            let row_top = first_top + 48.0 * f64::from(n - 1);
            let row_bottom = row_top + 48.0;
            if row_bottom <= top || row_top >= bottom {
                return None;
            }
            let (t, b) = if clipped {
                (row_top.max(top), row_bottom.min(bottom))
            } else {
                (row_top, row_bottom)
            };
            Some(json!({
                "class": class, "id": format!("row-{n}"), "desc": format!("Row {n}"),
                "flags": ["clickable", "focusable"], "bounds": [left, t, right, b]
            }))
        })
        .collect()
}

/// The status bar window of an Android screen 408 points wide.
fn status_bar() -> Value {
    json!({
        "id": 2, "kind": "system", "layer": 3, "package": "com.android.systemui",
        "title": "Status bar", "bounds": [0, 0, 408, 24], "owner": "com.android.systemui",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 0, 408, 24]}
    })
}

/// An Android app window holding `content` below its root.
fn app_window(content: Vec<Value>) -> Value {
    json!({
        "id": 1, "kind": "application", "layer": 1, "package": "dev.mobileagent.fixture.expo",
        "title": "FixtureExpo", "bounds": [0, 0, 408, 912], "flags": ["active", "focused"],
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 0, 408, 912], "children": content}
    })
}

fn android(windows: Vec<Value>) -> Screen {
    Screen::from_json(json!({
        "backend": {"name": "android-agent", "fallback": false},
        "scale": 2.625,
        "windows": windows
    }))
}

/// The Expo fixture's list on Android as its agent reads it: a scroll view
/// at [0, 72, 408, 840] whose rows start at `first_top`, clipped to it, and
/// whose scroll semantics say it can scroll back and forward as `back` and
/// `forward` say; `None` for a scroll view that reports no semantics.
fn android_list(first_top: f64, semantics: Option<(bool, bool)>) -> Screen {
    let mut list = json!({
        "class": "android.widget.ScrollView", "id": "list", "bounds": [0, 72, 408, 840],
        "flags": ["scrollable", "focusable"],
        "children": [{
            "class": "android.view.ViewGroup", "bounds": [0, 72, 408, 840],
            "children": rows("android.view.ViewGroup", first_top, 200, [0.0, 72.0, 408.0, 840.0], true)
        }]
    });
    if let Some((back, forward)) = semantics {
        list["scroll"] = json!({"axis": "vertical", "back": back, "forward": forward});
    }
    android(vec![status_bar(), app_window(vec![list])])
}

/// A mirror that has read `screen`.
fn mirror_showing(screen: &Screen) -> Mirror {
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().expect("the first read");
    mirror
}

/// The element with `identifier`, on screen.
fn element(mirror: &mut Mirror, identifier: &str) -> ElementId {
    mirror
        .wait(
            &Selector::identifier(identifier),
            Condition::Appear,
            Duration::ZERO,
        )
        .unwrap_or_else(|e| panic!("{identifier} is on screen: {e}"))
        .element
        .expect("the element")
        .id
}

fn visibility(mirror: &mut Mirror, identifier: &str) -> Visibility {
    let id = element(mirror, identifier);
    mirror
        .visibility(id)
        .expect("the replica holds the element")
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 0.01
}

#[test]
fn a_row_inside_its_list_is_wholly_in_view_within_the_list_s_bounds() {
    let mut mirror = mirror_showing(&android_list(72.0, Some((false, true))));
    let row = visibility(&mut mirror, "row-3");
    assert_eq!(row.bounds, Rect::new(0.0, 168.0, 408.0, 216.0), "{row:?}");
    assert_eq!(row.viewport, Rect::new(0.0, 72.0, 408.0, 840.0), "{row:?}");
    assert_eq!(row.fraction, 1.0, "{row:?}");
    assert_eq!(row.cut, None, "{row:?}");
}

/// The frame-path run of the harness's list-scroll flow stopped with
/// `row-200` 1.2 points tall at the Expo list's bottom edge: Android clips
/// a row's bounds to the list, so its bounds are all in view, and only the
/// edge they end at, where the list can still scroll, tells that more of
/// the row may lie beyond.
#[test]
fn a_row_android_clips_at_the_bottom_edge_of_a_list_that_scrolls_on_is_cut_there() {
    // Row 17 starts at 838.8, 1.2 points above the list's bottom edge.
    let mut mirror = mirror_showing(&android_list(838.8 - 768.0, Some((true, true))));
    let row = visibility(&mut mirror, "row-17");
    assert!(row.bounds.height() < 1.5, "a sliver: {row:?}");
    assert_eq!(row.bounds.bottom, 840.0, "{row:?}");
    assert_eq!(
        row.fraction, 1.0,
        "its bounds, as reported, are in view: {row:?}"
    );
    assert_eq!(row.cut, Some(Direction::Below), "{row:?}");

    let whole = visibility(&mut mirror, "row-10");
    assert_eq!(whole.cut, None, "a row short of the edges: {whole:?}");
}

#[test]
fn a_row_android_clips_at_the_top_edge_of_a_list_that_scrolls_back_is_cut_there() {
    let mut mirror = mirror_showing(&android_list(72.0 - 40.0, Some((true, true))));
    let row = visibility(&mut mirror, "row-1");
    assert_eq!(row.bounds, Rect::new(0.0, 72.0, 408.0, 80.0), "{row:?}");
    assert_eq!(row.cut, Some(Direction::Above), "{row:?}");
}

/// At the end of its range a list shows no more below its last row, so a
/// row that ends at the bottom edge there is whole.
#[test]
fn a_row_at_the_edge_of_a_list_that_cannot_scroll_that_way_is_whole() {
    // Rows 185 to 200 fill the list exactly; it cannot scroll forward.
    let mut mirror = mirror_showing(&android_list(72.0 - 48.0 * 184.0, Some((true, false))));
    let row = visibility(&mut mirror, "row-200");
    assert_eq!(row.bounds, Rect::new(0.0, 792.0, 408.0, 840.0), "{row:?}");
    assert_eq!(row.fraction, 1.0, "{row:?}");
    assert_eq!(row.cut, None, "{row:?}");
}

/// A container that reports no scroll semantics cannot tell whether it
/// scrolls further, and an edge its rows end at is no cut.
#[test]
fn without_scroll_semantics_an_edge_is_no_cut() {
    let mut mirror = mirror_showing(&android_list(838.8 - 768.0, None));
    let row = visibility(&mut mirror, "row-17");
    assert_eq!(row.cut, None, "{row:?}");
}

/// iOS reports whole frames, and a UIKit list runs under the navigation bar
/// and the tab bar: the bars cut its visible bounds, and a row under the
/// tab bar is in view only as far as it shows above it. A row wholly under
/// the tab bar, which XCTest lists after the list, is covered by it
/// (rn-app-fixes, design D31), the navigation bar listed before the list
/// is no less a bar for it.
#[test]
fn bars_over_a_list_cut_its_visible_bounds_and_a_whole_frame_shows_its_share() {
    let list_rows = rows(
        "XCUIElementTypeCell",
        62.0 + 16.0,
        20,
        [0.0, 0.0, 402.0, 874.0],
        false,
    );
    let screen = Screen::from_json(json!({
        "backend": {"name": "simulator-reader", "fallback": false},
        "scale": 3.0,
        "windows": [{
            "id": 7, "kind": "application", "package": "dev.mobileagent.fixture.swiftui",
            "title": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "flags": ["active", "focused"], "events": false,
            "root": {"class": "XCUIElementTypeApplication", "desc": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "children": [
                {"class": "XCUIElementTypeWindow", "bounds": [0, 0, 402, 874], "children": [
                    {"class": "XCUIElementTypeNavigationBar", "id": "nav", "desc": "List", "bounds": [0, 0, 402, 62]},
                    {"class": "XCUIElementTypeCollectionView", "id": "list", "bounds": [0, 0, 402, 874],
                     "scroll": {"axis": "vertical", "back": false, "forward": true},
                     "children": list_rows},
                    {"class": "XCUIElementTypeTabBar", "id": "tabs", "desc": "Tab Bar", "bounds": [0, 791, 402, 874], "children": [
                        {"class": "XCUIElementTypeButton", "id": "tab-list", "desc": "List", "bounds": [0, 791, 134, 874]}
                    ]}
                ]}
            ]}
        }]
    }));
    let mut mirror = mirror_showing(&screen);
    // Row 15 spans [750, 798]: 41 of its 48 points show above the tab bar.
    let row = visibility(&mut mirror, "row-15");
    assert_eq!(row.viewport, Rect::new(0.0, 62.0, 402.0, 791.0), "{row:?}");
    assert!(close(row.fraction, 41.0 / 48.0), "{row:?}");
    assert_eq!(row.cut, None, "a whole frame is no cut: {row:?}");
    // Row 16 spans [798, 846], wholly under the tab bar: covered, and none
    // of it in view.
    let before = screen.patched(json!([{"remove": "tabs"}]));
    let (_, covered, row_16) = read_then_covered(&before, &screen, 7, "row-16");
    assert_eq!(
        covered.covered_by(row_16).as_deref(),
        Some("tablist \"Tab Bar\"")
    );
    let hidden = covered.visibility(row_16).expect("the replica holds it");
    assert_eq!(hidden.fraction, 0.0, "{hidden:?}");

    // A tab of the tab bar is not under its own bar.
    let tab = visibility(&mut mirror, "tab-list");
    assert_eq!(tab.fraction, 1.0, "{tab:?}");
}

/// System bars and the keyboard are windows above the app's, and an app
/// drawn edge to edge runs under them.
#[test]
fn system_bars_and_the_keyboard_above_the_app_cut_its_visible_bounds() {
    let list = |first_top: f64| {
        json!({
            "class": "android.widget.ScrollView", "id": "list", "bounds": [0, 0, 408, 912],
            "flags": ["scrollable"], "scroll": {"axis": "vertical", "back": true, "forward": true},
            "children": rows("android.view.ViewGroup", first_top, 30, [0.0, 0.0, 408.0, 912.0], true)
        })
    };
    let navigation_bar = json!({
        "id": 3, "kind": "system", "layer": 3, "package": "com.android.systemui",
        "title": "Navigation bar", "bounds": [0, 864, 408, 912], "owner": "com.android.systemui",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 864, 408, 912]}
    });
    let screen = android(vec![
        status_bar(),
        navigation_bar.clone(),
        app_window(vec![list(0.0)]),
    ]);
    let mut mirror = mirror_showing(&screen);
    // Row 18 spans [816, 864], just above the navigation bar; row 19, [864,
    // 912], is under it.
    let above = visibility(&mut mirror, "row-18");
    assert_eq!(
        above.viewport,
        Rect::new(0.0, 24.0, 408.0, 864.0),
        "{above:?}"
    );
    assert_eq!(above.fraction, 1.0, "{above:?}");
    let under = visibility(&mut mirror, "row-19");
    assert_eq!(under.fraction, 0.0, "{under:?}");
    // Row 1 spans [0, 48]: half of it is under the status bar.
    let first = visibility(&mut mirror, "row-1");
    assert_eq!(first.fraction, 0.5, "{first:?}");

    let keyboard = json!({
        "id": 5, "kind": "input_method", "layer": 4, "package": "com.google.android.inputmethod.latin",
        "title": "Keyboard", "bounds": [0, 600, 408, 912], "owner": "com.google.android.inputmethod.latin",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 600, 408, 912]}
    });
    let typing = android(vec![
        keyboard,
        status_bar(),
        navigation_bar,
        app_window(vec![list(0.0)]),
    ]);
    let mut mirror = mirror_showing(&typing);
    // Row 13 spans [576, 624], half under the keyboard; row 14 is under it.
    let row = visibility(&mut mirror, "row-13");
    assert_eq!(row.viewport, Rect::new(0.0, 24.0, 408.0, 600.0), "{row:?}");
    assert_eq!(row.fraction, 0.5, "{row:?}");
    let under = visibility(&mut mirror, "row-14");
    assert_eq!(under.fraction, 0.0, "{under:?}");
}

/// Task 10.21: the Simulator's reader gives the keyboard's window the bounds
/// of what it draws, from the prediction bar down to the bottom of the
/// screen, as it read on Bluesky's search screen; what lies above stays in
/// view, the Cancel search button the React Native smoke test's click warned about
/// included, and the tab bar under the keyboard does not.
#[test]
fn the_ios_keyboard_cuts_from_its_prediction_bar_down() {
    let screen = Screen::from_json(json!({
        "backend": {"name": "simulator-reader", "fallback": false},
        "scale": 3.0,
        "windows": [
            {"id": 9, "kind": "input_method", "layer": 1, "package": "xyz.blueskyweb.app",
             "bounds": [0, 539, 402, 874], "events": false,
             "root": {"class": "XCUIElementTypeWindow", "bounds": [0, 0, 402, 874], "children": [
                 {"class": "XCUIElementTypeKeyboard", "bounds": [0, 583, 402, 816], "children": [
                     {"class": "XCUIElementTypeKey", "desc": "q", "bounds": [4.67, 590, 44, 644], "flags": ["clickable"]}
                 ]}
             ]}},
            {"id": 7, "kind": "application", "layer": 0, "package": "xyz.blueskyweb.app",
             "title": "Bluesky", "bounds": [0, 0, 402, 874], "events": false,
             "root": {"class": "XCUIElementTypeApplication", "desc": "Bluesky", "bounds": [0, 0, 402, 874], "children": [
                 {"class": "XCUIElementTypeWindow", "bounds": [0, 0, 402, 874], "children": [
                     {"class": "XCUIElementTypeButton", "id": "cancel", "desc": "Cancel search", "bounds": [16, 119.67, 49, 152.67]},
                     {"class": "XCUIElementTypeScrollView", "id": "typeahead", "bounds": [0, 166.33, 402, 874],
                      "flags": ["scrollable"], "children": rows("XCUIElementTypeButton", 166.33, 15, [0.0, 166.33, 402.0, 874.0], false)},
                     {"class": "XCUIElementTypeTabBar", "id": "tabs", "bounds": [0, 791, 402, 874], "children": [
                         {"class": "XCUIElementTypeButton", "id": "bottomBarHomeBtn", "desc": "Home", "bounds": [8, 793, 85.33, 840]}
                     ]}
                 ]}
             ]}}
        ]
    }));
    let mut mirror = mirror_showing(&screen);
    let cancel = visibility(&mut mirror, "cancel");
    assert_eq!(cancel.fraction, 1.0, "{cancel:?}");
    // Row 1 spans [166.33, 214.33], row 8 [502.33, 550.33], 11 of its 48
    // points under the prediction bar.
    let first = visibility(&mut mirror, "row-1");
    assert_eq!(first.fraction, 1.0, "{first:?}");
    assert!(close(first.viewport.bottom, 539.0), "{first:?}");
    let eighth = visibility(&mut mirror, "row-8");
    assert!(close(eighth.fraction, 37.0 / 48.0), "{eighth:?}");
    let home = visibility(&mut mirror, "bottomBarHomeBtn");
    assert_eq!(home.fraction, 0.0, "{home:?}");
    let list = mirror
        .scroll_viewport(Point::new(201.0, 300.0), true)
        .expect("the typeahead's list");
    assert!(
        close(list.top, 166.33) && close(list.bottom, 539.0),
        "{list:?}"
    );
}

/// Task 10.25: the Android agent gives the keyboard's window the bounds of
/// what it draws, the pieces of its touchable region that hold its nodes. On
/// the API 36 emulator, whose touchscreen also reports a stylus source,
/// Gboard shows its stylus toolbar for a text field, a pill floating at the
/// left from 342 to 605 points down, whose touchable region also holds the
/// navigation bar it draws across the bottom edge; the box around both, from
/// 342 points down across the screen, is the window's bounds as the platform
/// reports them, which cut every row of Bluesky's typeahead below the pill's
/// top. The pill spans no list, so it cuts none: the rows beside and below
/// it are in view, and a stroke through the list moves all of it.
#[test]
fn gboard_s_floating_toolbar_leaves_the_rows_beside_and_below_it_in_view() {
    let toolbar = json!({
        "id": 14, "kind": "input_method", "layer": 4, "package": "com.google.android.inputmethod.latin",
        "title": "Gboard", "bounds": [8, 342.1, 64.38, 604.95], "owner": "com.google.android.inputmethod.latin",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 50.3, 408, 912], "children": [
            {"class": "android.widget.FrameLayout", "bounds": [8, 342.1, 64.38, 604.95], "children": [
                {"class": "android.widget.FrameLayout", "desc": "Use voice typing", "flags": ["clickable"],
                 "bounds": [12.2, 345.9, 60.2, 393.9]}
            ]}
        ]}
    });
    let typeahead = json!({
        "class": "android.widget.ScrollView", "id": "typeahead", "bounds": [0, 152, 408, 832],
        "flags": ["scrollable"], "scroll": {"axis": "vertical", "back": false, "forward": true},
        "children": rows("android.view.ViewGroup", 200.0, 13, [0.0, 152.0, 408.0, 832.0], true)
    });
    let mut mirror = mirror_showing(&android(vec![
        toolbar,
        status_bar(),
        app_window(vec![typeahead]),
    ]));
    // Row 3 spans [296, 344], across the toolbar's top; row 7, [488, 536],
    // beside the toolbar; row 12, [728, 776], below it.
    for row in ["row-3", "row-7", "row-12"] {
        let seen = visibility(&mut mirror, row);
        assert_eq!(seen.fraction, 1.0, "{row}: {seen:?}");
        assert_eq!(
            seen.viewport,
            Rect::new(0.0, 152.0, 408.0, 832.0),
            "{row}: {seen:?}"
        );
    }
    assert_eq!(
        mirror.scroll_viewport(Point::new(204.0, 500.0), true),
        Some(Rect::new(0.0, 152.0, 408.0, 832.0))
    );
}

/// An element no scroll container holds is in view as far as its window
/// shows it.
#[test]
fn an_element_outside_any_scroll_container_is_measured_against_its_window() {
    let screen = android(vec![
        status_bar(),
        app_window(vec![json!({
            "class": "android.widget.Button", "id": "submit", "text": "Sign in",
            "flags": ["clickable", "focusable"], "bounds": [16, 200, 392, 248]
        })]),
    ]);
    let mut mirror = mirror_showing(&screen);
    let button = visibility(&mut mirror, "submit");
    assert_eq!(
        button.viewport,
        Rect::new(0.0, 24.0, 408.0, 912.0),
        "{button:?}"
    );
    assert_eq!(button.fraction, 1.0, "{button:?}");
    assert_eq!(button.cut, None, "{button:?}");
    assert_eq!(mirror.visibility(ElementId(u64::MAX)), None);
}

/// A mirror that read `before`, where the element `identifier` names is
/// listed, and then `after`, which a content change in window `window`
/// brought in 100 ms later, with that element's id: a ref issued on
/// `before` still names it once something covers it.
fn read_then_covered(
    before: &Screen,
    after: &Screen,
    window: u32,
    identifier: &str,
) -> (FakeTarget, Mirror, ElementId) {
    let target = FakeTarget::new(before.read());
    target.at(ms(100), Step::Show(after.read()));
    target.at(
        ms(100),
        Step::Event(EventPayload::Tree(TreeChange {
            change: TreeChangeKind::CONTENT,
            window,
            path: vec![0],
        })),
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let reference = mirror
        .snapshot(&SnapshotOptions::interactive())
        .unwrap()
        .by_identifier(identifier)
        .unwrap_or_else(|| panic!("{identifier} is listed before anything covers it"))
        .reference
        .expect("a ref");
    mirror.run_until(target.instant(ms(200))).unwrap();
    let id = mirror
        .resolve(reference)
        .expect("the ref still names it")
        .id;
    (target, mirror, id)
}

/// What covers an element is named as an action on its ref names it when
/// it fails with `element_covered`: an alert by its role and name, a modal
/// window as a dialog with its title. An element that nothing covers has
/// no cover, and neither has one the replica no longer holds.
#[test]
fn what_covers_an_element_is_named_as_an_action_on_its_ref_names_it() {
    let (_, mirror, submit) = read_then_covered(
        &Screen::load("login-ios"),
        &Screen::load("alert"),
        1,
        "login-submit",
    );
    assert_eq!(
        mirror.covered_by(submit).as_deref(),
        Some("alert \"In-app alert\"")
    );

    let login = Screen::load("login");
    let dialog = login.patched(json!([{"add_window": {
        "id": 7, "kind": "application", "layer": 2, "package": "dev.mobileagent.fixture.expo",
        "title": "In-app alert", "bounds": [10, 300, 398, 560], "flags": ["active", "focused"],
        "modal": true,
        "root": {"class": "android.widget.FrameLayout", "bounds": [10, 300, 398, 560], "children": [
            {"class": "android.widget.Button", "text": "OK", "bounds": [300, 500, 382, 548], "flags": ["clickable", "focusable"]}
        ]}
    }}]));
    let (_, mut mirror, submit) = read_then_covered(&login, &dialog, 7, "login-submit");
    assert_eq!(
        mirror.covered_by(submit).as_deref(),
        Some("dialog \"In-app alert\"")
    );

    let (_, mut uncovered) = support::mirror_on("login");
    let username = element(&mut uncovered, "login-username");
    assert_eq!(uncovered.covered_by(username), None);
    assert_eq!(mirror.covered_by(ElementId(u64::MAX)), None);
    let ok = mirror
        .wait(&Selector::text("OK"), Condition::Appear, Duration::ZERO)
        .expect("the dialog's button is on screen")
        .element
        .expect("the element")
        .id;
    assert_eq!(mirror.covered_by(ok), None);
}

/// Rn-app-fixes D31, scenario "Card under a sticky header": on iOS what a
/// view drawn after it covers is covered, not out of view: a tap reaches
/// none of it (design D37), `visibility` finds none of it in view and no
/// part to tap, and `covered_by` names the view over it as a refused action
/// does, though its visible bounds, as its scroll view's, leave out what
/// the view hides (design D45). What lies
/// beside a cover, as the link above the consent screen's footer and the
/// post that touches the feed header's edge, is not covered.
#[test]
fn what_a_view_drawn_after_it_covers_on_ios_is_covered_not_out_of_view() {
    for (fixture, covered, cover, named) in support::DRAWN_OVER_ON_IOS {
        let screen = Screen::load(fixture);
        let before = screen.patched(json!([{"remove": cover}]));
        let (_, mirror, id) = read_then_covered(&before, &screen, 1, covered);
        assert_eq!(
            mirror.covered_by(id).as_deref(),
            Some(named),
            "{fixture}: {covered}"
        );
        let seen = mirror.visibility(id).expect("the replica holds it");
        assert_eq!(seen.fraction, 0.0, "{fixture}: {seen:?}");
        assert_eq!(seen.reachable, None, "{fixture}: {seen:?}");
    }

    let (_, mut consent) = support::mirror_on("consent-ios");
    let policy = element(&mut consent, "placeholder-policy");
    assert_eq!(consent.covered_by(policy), None);
    let (_, mut feed) = support::mirror_on("feed-ios");
    let below = element(&mut feed, "placeholder-post-3");
    assert_eq!(feed.covered_by(below), None);
}

/// The element named `name` on screen.
fn named(mirror: &mut Mirror, name: &str) -> ElementId {
    mirror
        .wait(&Selector::text(name), Condition::Appear, Duration::ZERO)
        .unwrap_or_else(|e| panic!("{name} is on screen: {e}"))
        .element
        .expect("the element")
        .id
}

/// Whether two rectangles agree to within a hundredth of a point, as bounds
/// read in thirds of a point do.
fn same(actual: Option<Rect>, expected: Rect) -> bool {
    actual.is_some_and(|actual| {
        close(actual.left, expected.left)
            && close(actual.top, expected.top)
            && close(actual.right, expected.right)
            && close(actual.bottom, expected.bottom)
    })
}

/// Rn-app-fixes D37 (task 3.1; sections 1a and 2 of the masked listings):
/// what of an element a tap reaches is its bounds inside its visible bounds
/// less every view drawn over it, and the tap goes to the centre of the
/// largest rectangle of that, at least 8 points on each side. The consent
/// screen's heading keeps its upper 11 points above the secondary footer
/// button, half of it; the feed's card the 35 of its 116 points below its
/// list's top, where the header is drawn. The 12-point text under the
/// primary button keeps 3 points below it, too few, and what covers it is
/// the button, which a tap at its centre would hit.
#[test]
fn visibility_gives_the_part_of_an_element_a_tap_reaches_beside_what_covers_it() {
    let (_, mut consent) = support::mirror_on("consent-footer-ios");
    let heading = named(&mut consent, "placeholder-heading-9");
    let seen = consent.visibility(heading).expect("the heading");
    assert!(
        same(seen.reachable, Rect::new(33.0, 709.0, 90.33, 720.0)),
        "{seen:?}"
    );
    assert!(close(seen.fraction, 0.5), "{seen:?}");
    assert_eq!(consent.covered_by(heading), None);
    let text = named(&mut consent, "placeholder-text-8");
    let seen = consent.visibility(text).expect("the text");
    assert_eq!(seen.reachable, None, "{seen:?}");
    assert!(close(seen.fraction, 3.0 / 12.0), "{seen:?}");
    assert_eq!(
        consent.covered_by(text).as_deref(),
        Some("button \"placeholder-button-2\"")
    );

    let (_, mut feed) = support::mirror_on("feed-sticky-ios");
    let card = element(&mut feed, "placeholder-id-4");
    let seen = feed.visibility(card).expect("the card");
    assert!(
        same(seen.reachable, Rect::new(37.0, 122.0, 365.0, 157.0)),
        "{seen:?}"
    );
    assert!(close(seen.fraction, 35.0 / 116.0), "{seen:?}");
}

/// Rn-app-fixes D45, scenario "Strip clipped by the scroll view" (task 4.3;
/// task 3.7, N6): on the creation tab as the third production-app run met it,
/// the paste control's lower 10 points lie below the disabled primary
/// button, under the footer drawn over the list's lower part, outside what
/// the list shows; the control sits in a row whose frame ends at the
/// footer's top, so nothing in the tree puts the footer over the control,
/// and the run counted those points reachable, tapped them and opened
/// nothing. What a tap reaches is clipped by the visible bounds of every
/// scroll container above the element, as a stroke through the list knows
/// them, before the views over it are taken away: the control keeps no part
/// to tap, and what covers it is the disabled button, which a tap at its
/// centre would hit. With the list ending above the footer, as the second
/// production-app run read the tab, the 10 points take the tap (design D38).
#[test]
fn what_a_tap_reaches_is_clipped_by_what_its_scroll_view_shows() {
    let (_, mut create) = support::mirror_on("create-disabled-ios");
    let paste = element(&mut create, "placeholder-id-8");
    assert_eq!(
        create.scroll_viewport(Point::new(201.0, 400.0), true),
        Some(Rect::new(0.0, 122.0, 402.0, 764.0))
    );
    let seen = create.visibility(paste).expect("the paste control");
    assert_eq!(seen.reachable, None, "{seen:?}");
    assert_eq!(seen.fraction, 0.0, "{seen:?}");
    assert_eq!(
        create.covered_by(paste).as_deref(),
        Some("button \"placeholder-button-21\"")
    );

    let mut open = Screen::load("create-disabled-ios");
    let content = &mut open.0["windows"][0]["root"]["children"][0]["children"][0]["children"];
    content[1]["bounds"] = json!([0, 122, 402, 791]);
    content[3] = content[3]["children"][0].clone();
    let mut open = mirror_showing(&open);
    let paste = element(&mut open, "placeholder-id-8");
    let seen = open.visibility(paste).expect("the paste control");
    assert!(
        same(seen.reachable, Rect::new(205.0, 764.0, 382.0, 774.0)),
        "{seen:?}"
    );
    assert!(close(seen.fraction, 10.0 / 48.0), "{seen:?}");
    assert_eq!(open.covered_by(paste), None);
}

/// Rn-app-fixes D48, scenario "React Native scroll view" (task 5.1; task
/// 4.7, N6): on the creation tab as a throwaway screen read on the
/// Simulator gives it, the fourth production-app run's boxes to the point, the
/// scroll view runs on to the bottom of the screen under the footer, and
/// React Native's accessibility lists the footer's view as a node holding
/// nothing, its tabs after it as its siblings. The footer draws over the
/// scroll view's lower part, where the paste control's lower 10 points lie
/// below the disabled button, and a tap there lands on the footer, as the
/// run's did, opening nothing. What the scroll view shows ends at the
/// footer's top, so a stroke through it goes no lower, and the control keeps
/// no part a tap reaches: what covers it is the disabled button, which a tap
/// at its centre would hit.
#[test]
fn a_footer_react_native_lists_holding_nothing_cuts_what_its_scroll_view_shows() {
    let (_, mut create) = support::mirror_on("create-rn-ios");
    let paste = element(&mut create, "placeholder-paste");
    assert_eq!(
        create.scroll_viewport(Point::new(201.0, 400.0), true),
        Some(Rect::new(0.0, 122.0, 402.0, 764.0))
    );
    let seen = create.visibility(paste).expect("the paste control");
    assert_eq!(seen.reachable, None, "{seen:?}");
    assert_eq!(seen.fraction, 0.0, "{seen:?}");
    assert_eq!(
        create.covered_by(paste).as_deref(),
        Some("button \"placeholder-generate\"")
    );
}

/// The creation tab of `create-rn-ios` without its footer's view: nothing
/// is drawn over the paste control below the disabled button.
fn creation_rn_open() -> Screen {
    let mut screen = Screen::load("create-rn-ios");
    let views = react_native_views(&mut screen);
    let footer = views
        .iter()
        .position(|view| view["bounds"] == json!([0.0, 764.0, 402.0, 874.0]))
        .expect("the footer's view");
    views.remove(footer);
    screen
}

/// The children of the view React Native lists the creation tab's views in.
fn react_native_views(screen: &mut Screen) -> &mut Vec<Value> {
    let mut node = &mut screen.0["windows"][0]["root"];
    while node["class"] != "RNCSafeAreaProviderComponentView" {
        node = &mut node["children"][0];
    }
    node["children"].as_array_mut().expect("its views")
}

/// Rn-app-fixes D48 (task 5.1): a view React Native lists holding nothing
/// covers what a scroll view listed before it holds only where it lies over
/// part of the scroll view, as a footer does. One that holds all of it, as
/// a background listed after the list would, covers none of it, and one
/// listed after a sibling that does not scroll, as the header's view after
/// the header's controls, covers none of those.
#[test]
fn a_view_react_native_lists_holding_nothing_covers_only_part_of_a_scroll_view() {
    let open = creation_rn_open();
    let mut alone = mirror_showing(&open);
    let paste = element(&mut alone, "placeholder-paste");
    let strip = Rect::new(205.0, 764.0, 382.0, 774.0);
    let seen = alone.visibility(paste).expect("the paste control");
    assert!(same(seen.reachable, strip), "{seen:?}");

    let mut behind = open.clone();
    let views = react_native_views(&mut behind);
    let root = views.remove(0);
    assert_eq!(
        root["bounds"],
        json!([0.0, 0.0, 402.0, 874.0]),
        "the root's view"
    );
    let list = views
        .iter()
        .position(|view| view["class"] == "RCTScrollViewComponentView")
        .expect("the scroll view's wrapper");
    views.insert(list + 1, root);
    let header = views.remove(0);
    assert_eq!(
        header["bounds"],
        json!([0.0, 62.0, 402.0, 122.0]),
        "the header's view"
    );
    views.insert(2, header);
    let mut behind = mirror_showing(&behind);
    let paste = element(&mut behind, "placeholder-paste");
    let seen = behind.visibility(paste).expect("the paste control");
    assert!(same(seen.reachable, strip), "{seen:?}");
    let back = element(&mut behind, "placeholder-back");
    assert_eq!(behind.covered_by(back), None);
    assert_eq!(behind.visibility(back).map(|seen| seen.fraction), Some(1.0));
}

/// Rn-app-fixes D46 (task 4.4; task 3.7's N5): with nothing drawn over the
/// creation tab's list below the disabled button, the paste control 38 of
/// whose 48 points the button covers fits below it with the least movement,
/// 38 points down, but the list stands at its top and cannot carry its
/// content down, so a stroke aiming there moves nothing, as the third
/// production-app run's three strokes did. The visible bounds keep the side
/// above the button, 74 points up, where a stroke can carry the control; a
/// list that can scroll back keeps the side of least movement.
#[test]
fn a_view_across_an_element_keeps_the_side_its_list_can_carry_it_to() {
    let mut open = Screen::load("create-disabled-ios");
    let content = &mut open.0["windows"][0]["root"]["children"][0]["children"][0]["children"];
    content.as_array_mut().unwrap().truncate(3);
    let mut at_top = mirror_showing(&open);
    let seen = visibility(&mut at_top, "placeholder-id-8");
    assert_eq!(
        seen.viewport,
        Rect::new(0.0, 122.0, 402.0, 708.0),
        "{seen:?}"
    );

    let mut scrolled = open.clone();
    scrolled.0["windows"][0]["root"]["children"][0]["children"][0]["children"][1]["scroll"] =
        json!({"axis": "vertical", "back": true, "forward": true});
    let mut scrolled = mirror_showing(&scrolled);
    let seen = visibility(&mut scrolled, "placeholder-id-8");
    assert_eq!(
        seen.viewport,
        Rect::new(0.0, 764.0, 402.0, 874.0),
        "{seen:?}"
    );
}

/// Task 10.16: a stroke through the middle of the Expo list moves the list,
/// whose visible bounds are its own inside its window.
#[test]
fn a_stroke_through_a_list_moves_the_list_and_its_visible_bounds_are_its_own() {
    let mirror = mirror_showing(&android_list(72.0, Some((false, true))));
    assert_eq!(
        mirror.scroll_viewport(Point::new(204.0, 456.0), true),
        Some(Rect::new(0.0, 72.0, 408.0, 840.0))
    );
    // Without scroll semantics the scroll view still scrolls.
    let bare = mirror_showing(&android_list(72.0, None));
    assert_eq!(
        bare.scroll_viewport(Point::new(204.0, 456.0), true),
        Some(Rect::new(0.0, 72.0, 408.0, 840.0))
    );
}

/// Task 10.16: the bars over a list are left out of what a stroke through
/// it can bring into view, whether they are the list's own window's
/// navigation and tab bars, as a UIKit list runs under them, or windows
/// above it, as an edge-to-edge Android list runs under the system bars and
/// the keyboard.
#[test]
fn the_bars_over_a_list_are_left_out_of_its_visible_bounds() {
    let screen = Screen::from_json(json!({
        "backend": {"name": "simulator-reader", "fallback": false},
        "scale": 3.0,
        "windows": [{
            "id": 7, "kind": "application", "package": "dev.mobileagent.fixture.swiftui",
            "title": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "flags": ["active", "focused"], "events": false,
            "root": {"class": "XCUIElementTypeApplication", "desc": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "children": [
                {"class": "XCUIElementTypeWindow", "bounds": [0, 0, 402, 874], "children": [
                    {"class": "XCUIElementTypeNavigationBar", "id": "nav", "desc": "List", "bounds": [0, 0, 402, 62]},
                    {"class": "XCUIElementTypeCollectionView", "id": "list", "bounds": [0, 0, 402, 874],
                     "children": rows("XCUIElementTypeCell", 78.0, 20, [0.0, 0.0, 402.0, 874.0], false)},
                    {"class": "XCUIElementTypeTabBar", "id": "tabs", "desc": "Tab Bar", "bounds": [0, 791, 402, 874], "children": [
                        {"class": "XCUIElementTypeButton", "id": "tab-list", "desc": "List", "bounds": [0, 791, 134, 874]}
                    ]}
                ]}
            ]}
        }]
    }));
    let uikit = mirror_showing(&screen);
    assert_eq!(
        uikit.scroll_viewport(Point::new(201.0, 437.0), true),
        Some(Rect::new(0.0, 62.0, 402.0, 791.0))
    );

    let list = json!({
        "class": "android.widget.ScrollView", "id": "list", "bounds": [0, 0, 408, 912],
        "flags": ["scrollable"], "scroll": {"axis": "vertical", "back": true, "forward": true},
        "children": rows("android.view.ViewGroup", 0.0, 30, [0.0, 0.0, 408.0, 912.0], true)
    });
    let navigation_bar = json!({
        "id": 3, "kind": "system", "layer": 3, "package": "com.android.systemui",
        "title": "Navigation bar", "bounds": [0, 864, 408, 912], "owner": "com.android.systemui",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 864, 408, 912]}
    });
    let keyboard = json!({
        "id": 5, "kind": "input_method", "layer": 4, "package": "com.google.android.inputmethod.latin",
        "title": "Keyboard", "bounds": [0, 600, 408, 912], "owner": "com.google.android.inputmethod.latin",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 600, 408, 912]}
    });
    let edge_to_edge = mirror_showing(&android(vec![
        status_bar(),
        navigation_bar.clone(),
        app_window(vec![list.clone()]),
    ]));
    assert_eq!(
        edge_to_edge.scroll_viewport(Point::new(204.0, 456.0), true),
        Some(Rect::new(0.0, 24.0, 408.0, 864.0))
    );
    let typing = mirror_showing(&android(vec![
        keyboard,
        status_bar(),
        navigation_bar,
        app_window(vec![list]),
    ]));
    assert_eq!(
        typing.scroll_viewport(Point::new(204.0, 300.0), true),
        Some(Rect::new(0.0, 24.0, 408.0, 600.0))
    );
}

/// Task 10.16: of the containers under a point, a stroke moves the
/// innermost one that scrolls along its axis: a carousel for a stroke
/// across it, the list that holds the carousel for one along the list.
#[test]
fn a_stroke_moves_the_innermost_container_that_scrolls_along_its_axis() {
    let carousel = json!({
        "class": "androidx.recyclerview.widget.RecyclerView", "id": "carousel",
        "bounds": [0, 200, 408, 400], "flags": ["scrollable"],
        "scroll": {"axis": "horizontal", "back": false, "forward": true},
        "children": [{
            "class": "android.view.ViewGroup", "id": "card-1", "desc": "Card 1",
            "flags": ["clickable"], "bounds": [16, 216, 216, 384]
        }]
    });
    let list = json!({
        "class": "android.widget.ScrollView", "id": "list", "bounds": [0, 72, 408, 840],
        "flags": ["scrollable"], "scroll": {"axis": "vertical", "back": false, "forward": true},
        "children": [{"class": "android.view.ViewGroup", "bounds": [0, 72, 408, 840], "children": [carousel]}]
    });
    let mirror = mirror_showing(&android(vec![status_bar(), app_window(vec![list])]));
    let inside = Point::new(204.0, 300.0);
    assert_eq!(
        mirror.scroll_viewport(inside, false),
        Some(Rect::new(0.0, 200.0, 408.0, 400.0)),
        "across the carousel"
    );
    assert_eq!(
        mirror.scroll_viewport(inside, true),
        Some(Rect::new(0.0, 72.0, 408.0, 840.0)),
        "along the list"
    );
}

/// The Android keyboard's window over the screen from 600 points down,
/// flagged as `flags` say.
fn android_keyboard(flags: Value) -> Value {
    json!({
        "id": 5, "kind": "input_method", "layer": 4, "package": "com.google.android.inputmethod.latin",
        "title": "Keyboard", "bounds": [0, 600, 408, 912], "owner": "com.google.android.inputmethod.latin",
        "flags": flags,
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 600, 408, 912]}
    })
}

/// Task 10.32: a stroke that comes down on a system bar or the keyboard goes
/// to that window, and Gboard took the strokes of a scroll across its keys
/// for glide typing. What of an area a stroke may go through is the area
/// less the system and input-method windows above the window it lies in,
/// each cutting it as a bar cuts visible bounds: the screen of an app less
/// the status bar, the navigation bar and the keyboard above its window,
/// even while the keyboard's window is the active one, as Android flags the
/// window a finger touches; a row half under the keyboard less that half,
/// nothing of a row under it, and all of a row above it.
#[test]
fn a_stroke_goes_through_what_the_system_bars_and_the_keyboard_leave_of_its_area() {
    let list = json!({
        "class": "android.widget.ScrollView", "id": "list", "bounds": [0, 0, 408, 912],
        "flags": ["scrollable"], "scroll": {"axis": "vertical", "back": true, "forward": true},
        "children": rows("android.view.ViewGroup", 0.0, 30, [0.0, 0.0, 408.0, 912.0], true)
    });
    let navigation_bar = json!({
        "id": 3, "kind": "system", "layer": 3, "package": "com.android.systemui",
        "title": "Navigation bar", "bounds": [0, 864, 408, 912], "owner": "com.android.systemui",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 864, 408, 912]}
    });
    let screen = Rect::new(0.0, 0.0, 408.0, 912.0);
    let bare = mirror_showing(&android(vec![
        status_bar(),
        navigation_bar.clone(),
        app_window(vec![list.clone()]),
    ]));
    assert_eq!(
        bare.uncovered(screen, None),
        Uncovered {
            area: Rect::new(0.0, 24.0, 408.0, 864.0),
            keyboard: false
        }
    );

    let mut typing = mirror_showing(&android(vec![
        android_keyboard(json!([])),
        status_bar(),
        navigation_bar.clone(),
        app_window(vec![list.clone()]),
    ]));
    let above_the_keyboard = Uncovered {
        area: Rect::new(0.0, 24.0, 408.0, 600.0),
        keyboard: true,
    };
    assert_eq!(typing.uncovered(screen, None), above_the_keyboard);
    // Row 13 spans [576, 624], half under the keyboard; row 14, [624, 672],
    // under it; row 3, [96, 144], above it.
    let row = |mirror: &mut Mirror, identifier: &str| {
        let id = element(mirror, identifier);
        let bounds = mirror.visibility(id).expect("the row").bounds;
        mirror.uncovered(bounds, Some(id))
    };
    assert_eq!(
        row(&mut typing, "row-13"),
        Uncovered {
            area: Rect::new(0.0, 576.0, 408.0, 600.0),
            keyboard: true
        }
    );
    let under = row(&mut typing, "row-14");
    assert!(under.area.is_empty() && under.keyboard, "{under:?}");
    assert_eq!(
        row(&mut typing, "row-3"),
        Uncovered {
            area: Rect::new(0.0, 96.0, 408.0, 144.0),
            keyboard: false
        }
    );

    let mut touched = app_window(vec![list]);
    touched["flags"] = json!([]);
    let touching = mirror_showing(&android(vec![
        android_keyboard(json!(["active"])),
        status_bar(),
        navigation_bar,
        touched,
    ]));
    assert_eq!(touching.uncovered(screen, None), above_the_keyboard);
}

/// Task 10.32: the windows above the screen are those above the window in
/// front. On a Simulator the keyboard's window, which the reader bounds by
/// what it draws (task 10.21), leaves the screen above its prediction bar;
/// Gboard's floating toolbar spans no part of the screen and cuts nothing
/// (task 10.25); and a system window in front, as SpringBoard's home screen,
/// is the screen's own window, which nothing above cuts.
#[test]
fn the_screen_loses_what_the_windows_above_the_one_in_front_cover() {
    let swiftui = Screen::from_json(json!({
        "backend": {"name": "simulator-reader", "fallback": false},
        "scale": 3.0,
        "windows": [
            {"id": 9, "kind": "input_method", "layer": 1, "package": "dev.mobileagent.fixture.swiftui",
             "bounds": [0, 539, 402, 874], "events": false,
             "root": {"class": "XCUIElementTypeWindow", "bounds": [0, 0, 402, 874], "children": [
                 {"class": "XCUIElementTypeKeyboard", "bounds": [0, 583, 402, 816]}
             ]}},
            {"id": 7, "kind": "application", "layer": 0, "package": "dev.mobileagent.fixture.swiftui",
             "title": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "events": false,
             "root": {"class": "XCUIElementTypeApplication", "desc": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "children": [
                 {"class": "XCUIElementTypeWindow", "bounds": [0, 0, 402, 874], "children": [
                     {"class": "XCUIElementTypeTextField", "id": "login-username", "desc": "Username",
                      "bounds": [16, 78, 386, 112], "flags": ["focused"]}
                 ]}
             ]}}
        ]
    }));
    let iphone = Rect::new(0.0, 0.0, 402.0, 874.0);
    let uncovered = mirror_showing(&swiftui).uncovered(iphone, None);
    assert!(
        uncovered.keyboard && close(uncovered.area.bottom, 539.0),
        "{uncovered:?}"
    );
    assert_eq!(uncovered.area.top, 0.0, "{uncovered:?}");

    let toolbar = json!({
        "id": 14, "kind": "input_method", "layer": 4, "package": "com.google.android.inputmethod.latin",
        "title": "Gboard", "bounds": [8, 342.1, 64.38, 604.95], "owner": "com.google.android.inputmethod.latin",
        "root": {"class": "android.widget.FrameLayout", "bounds": [8, 342.1, 64.38, 604.95]}
    });
    let floating = mirror_showing(&android(vec![toolbar, status_bar(), app_window(vec![])]));
    assert_eq!(
        floating.uncovered(Rect::new(0.0, 0.0, 408.0, 912.0), None),
        Uncovered {
            area: Rect::new(0.0, 24.0, 408.0, 912.0),
            keyboard: false
        }
    );

    let home = Screen::from_json(json!({
        "backend": {"name": "simulator-reader", "fallback": false},
        "scale": 3.0,
        "windows": [{
            "id": 900, "kind": "system", "layer": 0, "package": "com.apple.springboard",
            "title": "SpringBoard", "bounds": [0, 0, 402, 874], "flags": ["active", "focused"],
            "owner": "SpringBoard", "events": false,
            "root": {"class": "XCUIElementTypeApplication", "desc": "SpringBoard", "bounds": [0, 0, 402, 874]}
        }]
    }));
    assert_eq!(
        mirror_showing(&home).uncovered(iphone, None),
        Uncovered {
            area: iphone,
            keyboard: false
        }
    );
}

/// Task 10.16: a point that no scroll container holds has no container a
/// stroke through it moves.
#[test]
fn a_point_outside_every_scroll_container_has_none() {
    let mirror = mirror_showing(&android_list(72.0, Some((false, true))));
    assert_eq!(
        mirror.scroll_viewport(Point::new(204.0, 880.0), true),
        None,
        "below the list"
    );
    let form = mirror_showing(&android(vec![
        status_bar(),
        app_window(vec![json!({
            "class": "android.widget.Button", "id": "submit", "text": "Sign in",
            "flags": ["clickable", "focusable"], "bounds": [16, 200, 392, 248]
        })]),
    ]));
    assert_eq!(form.scroll_viewport(Point::new(204.0, 224.0), true), None);
}
